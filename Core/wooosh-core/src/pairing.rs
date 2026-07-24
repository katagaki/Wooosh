//! Pairing: QR payload + one-time tokens (PROTOCOL.md §4.2) and SAS state.
//!
//! QR payload format note: PROTOCOL.md §4.2 shows `?` between every parameter
//! (`...?pk=..?tok=..`), which is not valid URI query syntax. We emit `&`
//! separators (`wooosh-pair:1?pk=..&tok=..&hints=..&exp=..`) and accept both
//! on parse.

use crate::error::WoooshError;
use base64::Engine as _;
use rand::RngCore;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

pub const QR_TOKEN_TTL: Duration = Duration::from_secs(120);
pub const SAS_TIMEOUT: Duration = Duration::from_secs(60);

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
}

#[derive(Debug, Clone, PartialEq)]
pub struct QrPayload {
    pub version: u64,
    pub pubkey: [u8; 32],
    pub token: [u8; 32],
    /// Display-name hint (unauthenticated, UI label only).
    pub dn: Option<String>,
    pub hints: Vec<String>,
    pub expires_unix: u64,
}

fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn pct_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() + 1 {
                return None;
            }
            let hexpair = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hexpair, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl QrPayload {
    pub fn encode(&self) -> String {
        let dn_part = self
            .dn
            .as_ref()
            .map(|d| format!("&dn={}", pct_encode(d)))
            .unwrap_or_default();
        format!(
            "wooosh-pair:{}?pk={}&tok={}{}&hints={}&exp={}",
            self.version,
            b64().encode(self.pubkey),
            b64().encode(self.token),
            dn_part,
            self.hints.join(","),
            self.expires_unix
        )
    }

    pub fn parse(s: &str) -> Result<Self, WoooshError> {
        let bad = |m: &str| WoooshError::InvalidQrPayload(m.to_string());
        let rest = s.strip_prefix("wooosh-pair:").ok_or_else(|| bad("missing scheme"))?;
        let (ver_str, params_str) = rest.split_once('?').ok_or_else(|| bad("missing params"))?;
        let version: u64 = ver_str.parse().map_err(|_| bad("bad version"))?;
        if version != 1 {
            return Err(bad("unsupported version"));
        }
        let mut pk = None;
        let mut tok = None;
        let mut dn = None;
        let mut hints = Vec::new();
        let mut exp = None;
        // Accept both '&' and '?' as separators (spec text shows '?').
        for kv in params_str.split(['&', '?']) {
            let Some((k, v)) = kv.split_once('=') else { continue };
            match k {
                "pk" => {
                    let bytes = b64().decode(v).map_err(|_| bad("pk not base64"))?;
                    pk = Some(<[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| bad("pk length"))?);
                }
                "tok" => {
                    let bytes = b64().decode(v).map_err(|_| bad("tok not base64"))?;
                    tok = Some(<[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| bad("tok length"))?);
                }
                "dn" => dn = pct_decode(v),
                "hints" => {
                    hints = v.split(',').filter(|h| !h.is_empty()).map(|h| h.to_string()).collect();
                }
                "exp" => exp = Some(v.parse::<u64>().map_err(|_| bad("bad exp"))?),
                _ => {} // forward compat
            }
        }
        Ok(QrPayload {
            version,
            pubkey: pk.ok_or_else(|| bad("missing pk"))?,
            token: tok.ok_or_else(|| bad("missing tok"))?,
            dn,
            hints,
            expires_unix: exp.ok_or_else(|| bad("missing exp"))?,
        })
    }

    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        now > self.expires_unix
    }
}

/// Receiver-side state for an outstanding QR pairing offer.
pub struct QrPending {
    token: [u8; 32],
    issued_at: Instant,
    used: bool,
}

impl QrPending {
    pub fn new() -> (Self, [u8; 32]) {
        let mut token = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut token);
        (Self { token, issued_at: Instant::now(), used: false }, token)
    }

    /// Single-use, TTL-bound, constant-time token check.
    pub fn redeem(&mut self, presented: &[u8]) -> bool {
        if self.used || self.issued_at.elapsed() > QR_TOKEN_TTL {
            return false;
        }
        if presented.len() != 32 {
            return false;
        }
        let ok = presented.ct_eq(&self.token).unwrap_u8() == 1;
        if ok {
            self.used = true;
        }
        ok
    }
}

pub fn new_expiry_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + QR_TOKEN_TTL.as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_roundtrip() {
        let p = QrPayload {
            version: 1,
            pubkey: [1; 32],
            token: [2; 32],
            dn: Some("Kata's MacBook & Co".into()),
            hints: vec!["192.168.1.5:5000".into(), "127.0.0.1:5000".into()],
            expires_unix: new_expiry_unix(),
        };
        let s = p.encode();
        assert!(s.starts_with("wooosh-pair:1?pk="));
        assert!(!s.contains('\'') && !s.contains(' ') && !s.contains("&dn=Kata'"));
        let q = QrPayload::parse(&s).unwrap();
        assert_eq!(p, q);
        assert!(!q.is_expired());
    }

    #[test]
    fn qr_parse_question_mark_separators() {
        let p = QrPayload {
            version: 1,
            pubkey: [1; 32],
            token: [2; 32],
            dn: None,
            hints: vec!["10.0.0.1:1234".into()],
            expires_unix: 99,
        };
        let s = p.encode().replace('&', "?");
        let q = QrPayload::parse(&s).unwrap();
        assert_eq!(q.pubkey, p.pubkey);
        assert!(q.is_expired());
    }

    #[test]
    fn token_single_use_and_wrong_rejected() {
        let (mut pending, token) = QrPending::new();
        assert!(!pending.redeem(&[0u8; 32]));
        assert!(pending.redeem(&token));
        assert!(!pending.redeem(&token)); // single use
    }
}
