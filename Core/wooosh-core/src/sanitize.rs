//! Receiver-side filename / rel_path sanitization (PROTOCOL.md §5).
//!
//! - `name`: path separators stripped, `..` rejected, control chars rejected,
//!   reserved Windows names rejected.
//! - `rel_path`: only honored beneath a single transfer root; every component
//!   is validated; absolute paths, drive letters and `..` are rejected.

use std::path::{Component, Path, PathBuf};

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug, PartialEq, Eq)]
pub enum SanitizeError {
    Empty,
    DotDot,
    ControlChars,
    ReservedName,
    AbsolutePath,
    BadComponent,
}

impl std::fmt::Display for SanitizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SanitizeError::Empty => "empty name",
            SanitizeError::DotDot => "'..' rejected",
            SanitizeError::ControlChars => "control characters rejected",
            SanitizeError::ReservedName => "reserved name rejected",
            SanitizeError::AbsolutePath => "absolute path rejected",
            SanitizeError::BadComponent => "bad path component",
        };
        f.write_str(s)
    }
}

fn is_reserved_windows(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    WINDOWS_RESERVED.iter().any(|r| stem.eq_ignore_ascii_case(r))
}

/// Sanitize a bare file name. Strips path separators, then validates.
pub fn sanitize_name(name: &str) -> Result<String, SanitizeError> {
    // Strip path separators per spec ("strip path separators from name").
    let stripped: String = name.chars().filter(|c| *c != '/' && *c != '\\').collect();
    let stripped = stripped.trim().to_string();
    if stripped.is_empty() {
        return Err(SanitizeError::Empty);
    }
    if stripped == ".." || stripped == "." {
        return Err(SanitizeError::DotDot);
    }
    if stripped.chars().any(|c| c.is_control()) {
        return Err(SanitizeError::ControlChars);
    }
    if is_reserved_windows(&stripped) {
        return Err(SanitizeError::ReservedName);
    }
    // Also reject names that are all dots.
    if stripped.chars().all(|c| c == '.') {
        return Err(SanitizeError::DotDot);
    }
    Ok(stripped)
}

/// Validate a relative path (forward-slash separated on the wire) and return
/// a safe relative `PathBuf`. Never contains `..`, absolute roots or drive
/// letters; each component passes the same rules as `sanitize_name`.
pub fn sanitize_rel_path(rel: &str) -> Result<PathBuf, SanitizeError> {
    if rel.is_empty() {
        return Err(SanitizeError::Empty);
    }
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(SanitizeError::AbsolutePath);
    }
    // Windows drive letter or UNC.
    if rel.len() >= 2 && rel.as_bytes()[1] == b':' {
        return Err(SanitizeError::AbsolutePath);
    }
    let normalized = rel.replace('\\', "/");
    let mut out = PathBuf::new();
    for comp in normalized.split('/') {
        if comp.is_empty() {
            continue;
        }
        if comp == ".." {
            return Err(SanitizeError::DotDot);
        }
        if comp == "." {
            continue;
        }
        if comp.chars().any(|c| c.is_control()) {
            return Err(SanitizeError::ControlChars);
        }
        if is_reserved_windows(comp) {
            return Err(SanitizeError::ReservedName);
        }
        out.push(comp);
    }
    if out.as_os_str().is_empty() {
        return Err(SanitizeError::Empty);
    }
    Ok(out)
}

/// Join a sanitized rel_path under `root`, with a lexical containment check.
pub fn join_under_root(root: &Path, rel: &Path) -> Result<PathBuf, SanitizeError> {
    let joined = root.join(rel);
    for c in joined.components() {
        if matches!(c, Component::ParentDir) {
            return Err(SanitizeError::DotDot);
        }
    }
    if !joined.starts_with(root) {
        return Err(SanitizeError::BadComponent);
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_strips_separators() {
        assert_eq!(sanitize_name("a/b\\c.txt").unwrap(), "abc.txt");
    }

    #[test]
    fn name_rejects_dotdot_and_controls() {
        assert_eq!(sanitize_name(".."), Err(SanitizeError::DotDot));
        assert_eq!(sanitize_name("..."), Err(SanitizeError::DotDot));
        assert_eq!(sanitize_name("a\x00b"), Err(SanitizeError::ControlChars));
        assert_eq!(sanitize_name("a\nb"), Err(SanitizeError::ControlChars));
        assert_eq!(sanitize_name(""), Err(SanitizeError::Empty));
        assert_eq!(sanitize_name("///"), Err(SanitizeError::Empty));
    }

    #[test]
    fn name_rejects_windows_reserved() {
        assert_eq!(sanitize_name("CON"), Err(SanitizeError::ReservedName));
        assert_eq!(sanitize_name("con.txt"), Err(SanitizeError::ReservedName));
        assert_eq!(sanitize_name("Com1.tar.gz"), Err(SanitizeError::ReservedName));
        assert!(sanitize_name("console.txt").is_ok());
    }

    #[test]
    fn relpath_rules() {
        assert_eq!(sanitize_rel_path("a/b/c.txt").unwrap(), PathBuf::from("a/b/c.txt"));
        assert_eq!(sanitize_rel_path("a\\b\\c.txt").unwrap(), PathBuf::from("a/b/c.txt"));
        assert_eq!(sanitize_rel_path("../x"), Err(SanitizeError::DotDot));
        assert_eq!(sanitize_rel_path("a/../x"), Err(SanitizeError::DotDot));
        assert_eq!(sanitize_rel_path("/etc/passwd"), Err(SanitizeError::AbsolutePath));
        assert_eq!(sanitize_rel_path("C:\\x"), Err(SanitizeError::AbsolutePath));
        assert_eq!(sanitize_rel_path("a/NUL/b"), Err(SanitizeError::ReservedName));
        assert_eq!(sanitize_rel_path("./a/./b").unwrap(), PathBuf::from("a/b"));
    }

    #[test]
    fn join_containment() {
        let root = Path::new("/tmp/stage/t1");
        let rel = sanitize_rel_path("sub/f.bin").unwrap();
        let j = join_under_root(root, &rel).unwrap();
        assert!(j.starts_with(root));
    }
}
