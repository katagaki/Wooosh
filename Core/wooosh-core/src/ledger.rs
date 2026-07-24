//! Persistent resume ledger (PROTOCOL.md §6).
//!
//! Lives at `staging/<tid_hex>/ledger.json` next to the `.part` files.
//! `verified_off` = bytes written + flushed for that file. The BLAKE3 hasher
//! state is NOT serialized (the crate does not expose a stable checkpoint
//! format); on resume the `.part` prefix is re-hashed, which PROTOCOL.md
//! explicitly allows ("recomputed over the .part prefix on cold resume").

use crate::error::WoooshError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Disambiguates concurrent atomic-write temp files (see `Ledger::save`).
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerFile {
    pub fid: u32,
    pub name: String,
    pub rel_path: Option<String>,
    pub size: u64,
    pub b3_hex: String,
    pub mime: String,
    pub verified_off: u64,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ledger {
    pub tid_hex: String,
    pub sender_pubkey_b64: String,
    pub files: BTreeMap<u32, LedgerFile>,
}

impl Ledger {
    pub fn path_for(staging_tid_dir: &Path) -> PathBuf {
        staging_tid_dir.join("ledger.json")
    }

    pub fn load(staging_tid_dir: &Path) -> Result<Option<Ledger>, WoooshError> {
        let p = Self::path_for(staging_tid_dir);
        if !p.exists() {
            return Ok(None);
        }
        let data = std::fs::read(&p)?;
        let ledger = serde_json::from_slice(&data)
            .map_err(|e| WoooshError::Io(format!("ledger parse: {e}")))?;
        Ok(Some(ledger))
    }

    /// Atomic write + fsync.
    ///
    /// The temp file name is unique per call: several slot streams persist
    /// concurrently for one transfer, and a shared `ledger.tmp` would have
    /// them truncate each other's partial writes (producing a corrupt
    /// `ledger.json`) and race on the rename (the loser gets ENOENT).
    pub fn save(&self, staging_tid_dir: &Path) -> Result<(), WoooshError> {
        std::fs::create_dir_all(staging_tid_dir)?;
        let p = Self::path_for(staging_tid_dir);
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = staging_tid_dir.join(format!("ledger.{}.{seq}.tmp", std::process::id()));
        let data = serde_json::to_vec_pretty(self)
            .map_err(|e| WoooshError::Io(format!("ledger encode: {e}")))?;
        let write = (|| -> Result<(), WoooshError> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&data)?;
            f.sync_all()?;
            Ok(())
        })();
        if let Err(e) = write {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&tmp, &p) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }
}

/// Re-hash the first `len` bytes of a `.part` file, returning the primed
/// hasher (used to trust the ledger's `verified_off` on cold resume).
pub fn rehash_prefix(part_path: &Path, len: u64) -> Result<blake3::Hasher, WoooshError> {
    let mut hasher = blake3::Hasher::new();
    if len == 0 {
        return Ok(hasher);
    }
    let f = std::fs::File::open(part_path)?;
    let mut limited = f.take(len);
    let mut buf = vec![0u8; 1024 * 1024];
    let mut hashed: u64 = 0;
    loop {
        let n = limited.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        hashed += n as u64;
    }
    if hashed != len {
        return Err(WoooshError::Io(format!(
            ".part shorter than ledger verified_off ({hashed} < {len})"
        )));
    }
    Ok(hasher)
}
