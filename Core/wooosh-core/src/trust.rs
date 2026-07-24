//! Trust store: pinned peer public keys (PROTOCOL.md §4.5).
//! JSON file: { pubkey_b64 -> { device_id, dn, dt?, paired_at, last_seen, last_addr? } }.
//!
//! Beyond "is this key pinned?", the store remembers *where* a pinned peer was
//! last reached (`last_addr`, exact `ip:port`). That memory is what lets
//! `connect_to` enforce a pin when the shell passes no expected key — see
//! `pinned_key_for_addr` and DESIGN.md §4.

use crate::error::WoooshError;
use crate::identity;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustEntry {
    pub device_id: String,
    pub dn: String,
    /// Device type from the peer's HELLO, recorded at pairing time so a
    /// trust-list row can show the right icon.
    #[serde(default)]
    pub dt: Option<String>,
    pub paired_at: u64,
    pub last_seen: u64,
    /// Exact `ip:port` at which this pinned peer was last successfully
    /// authenticated. Advisory: used to re-apply the pin on a later
    /// `connect_peer(addr, None)`, never to establish trust by itself.
    #[serde(default)]
    pub last_addr: Option<String>,
}

pub struct TrustStore {
    path: PathBuf,
    entries: Mutex<HashMap<String, TrustEntry>>,
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn key_b64(pubkey: &[u8; 32]) -> String {
    base64::engine::general_purpose::STANDARD.encode(pubkey)
}

fn key_from_b64(s: &str) -> Option<[u8; 32]> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
}

impl TrustStore {
    pub fn open(path: PathBuf) -> Result<Self, WoooshError> {
        let entries = if path.exists() {
            let data = std::fs::read(&path)?;
            serde_json::from_slice(&data)
                .map_err(|e| WoooshError::Io(format!("trust store parse: {e}")))?
        } else {
            HashMap::new()
        };
        Ok(Self { path, entries: Mutex::new(entries) })
    }

    fn save_locked(&self, entries: &HashMap<String, TrustEntry>) -> Result<(), WoooshError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("tmp");
        let data = serde_json::to_vec_pretty(entries)
            .map_err(|e| WoooshError::Io(format!("trust store encode: {e}")))?;
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn contains(&self, pubkey: &[u8; 32]) -> bool {
        self.entries.lock().unwrap().contains_key(&key_b64(pubkey))
    }

    pub fn get(&self, pubkey: &[u8; 32]) -> Option<TrustEntry> {
        self.entries.lock().unwrap().get(&key_b64(pubkey)).cloned()
    }

    /// Pin a peer. `dt` and `addr` are UI/routing hints only; neither
    /// participates in the trust decision.
    pub fn insert(
        &self,
        pubkey: &[u8; 32],
        dn: &str,
        dt: Option<&str>,
        addr: Option<&str>,
    ) -> Result<(), WoooshError> {
        let mut entries = self.entries.lock().unwrap();
        let now = now_unix();
        let me = key_b64(pubkey);
        // One address maps to at most one pinned identity: otherwise a lookup
        // resolves to a stale key and hard-fails a peer that legitimately
        // took the address over.
        if let Some(a) = addr {
            for (k, e) in entries.iter_mut() {
                if k != &me && e.last_addr.as_deref() == Some(a) {
                    e.last_addr = None;
                }
            }
        }
        let paired_at = entries.get(&me).map(|e| e.paired_at).unwrap_or(now);
        entries.insert(
            me,
            TrustEntry {
                device_id: identity::device_id_string_for(pubkey),
                dn: dn.to_string(),
                dt: dt.map(|s| s.to_string()),
                paired_at,
                last_seen: now,
                last_addr: addr.map(|s| s.to_string()),
            },
        );
        self.save_locked(&entries)
    }

    /// Record where a pinned peer just authenticated (§4.5 pin memory) and
    /// bump `last_seen`. No-op for unpinned keys, so an untrusted peer can
    /// never write here.
    pub fn note_addr(&self, pubkey: &[u8; 32], addr: &str) {
        let mut entries = self.entries.lock().unwrap();
        let me = key_b64(pubkey);
        if !entries.contains_key(&me) {
            return;
        }
        for (k, e) in entries.iter_mut() {
            if k == &me {
                e.last_addr = Some(addr.to_string());
                e.last_seen = now_unix();
            } else if e.last_addr.as_deref() == Some(addr) {
                e.last_addr = None;
            }
        }
        let _ = self.save_locked(&entries);
    }

    /// The pinned key we expect at `addr` (exact `ip:port`), if any.
    /// Ambiguous matches (should not happen — `insert`/`note_addr` keep the
    /// mapping unique) resolve to `None` rather than to an arbitrary key.
    pub fn pinned_key_for_addr(&self, addr: &str) -> Option<[u8; 32]> {
        let entries = self.entries.lock().unwrap();
        let mut found = None;
        for (k, e) in entries.iter() {
            if e.last_addr.as_deref() == Some(addr) {
                if found.is_some() {
                    return None;
                }
                found = key_from_b64(k);
            }
        }
        found
    }

    /// The pinned key registered under a rendered device-id string, if any.
    /// Used to catch a peer that *claims* a pinned identity in HELLO while
    /// presenting a different certificate key (PROTOCOL.md §4.1.1).
    pub fn pinned_key_for_device_id(&self, device_id: &str) -> Option<[u8; 32]> {
        let entries = self.entries.lock().unwrap();
        entries
            .iter()
            .find(|(_, e)| e.device_id == device_id)
            .and_then(|(k, _)| key_from_b64(k))
    }

    pub fn revoke(&self, pubkey: &[u8; 32]) -> Result<bool, WoooshError> {
        let mut entries = self.entries.lock().unwrap();
        let removed = entries.remove(&key_b64(pubkey)).is_some();
        if removed {
            self.save_locked(&entries)?;
        }
        Ok(removed)
    }

    pub fn list(&self) -> Vec<(String, TrustEntry)> {
        self.entries.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Pinned set with decoded keys, ordered by `paired_at` then device id so
    /// the shell's trust list is stable across calls.
    pub fn entries(&self) -> Vec<([u8; 32], TrustEntry)> {
        let mut out: Vec<([u8; 32], TrustEntry)> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(k, v)| key_from_b64(k).map(|pk| (pk, v.clone())))
            .collect();
        out.sort_by(|a, b| a.1.paired_at.cmp(&b.1.paired_at).then(a.1.device_id.cmp(&b.1.device_id)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join(format!("wooosh-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("trust.json");
        let ts = TrustStore::open(path.clone()).unwrap();
        let pk = [7u8; 32];
        assert!(!ts.contains(&pk));
        ts.insert(&pk, "Test Device", Some("laptop"), Some("10.0.0.4:5000")).unwrap();
        assert!(ts.contains(&pk));
        drop(ts);
        let ts2 = TrustStore::open(path).unwrap();
        assert!(ts2.contains(&pk));
        assert_eq!(ts2.get(&pk).unwrap().dn, "Test Device");
        assert_eq!(ts2.get(&pk).unwrap().dt.as_deref(), Some("laptop"));
        assert_eq!(ts2.pinned_key_for_addr("10.0.0.4:5000"), Some(pk));
        assert_eq!(ts2.pinned_key_for_addr("10.0.0.4:5001"), None);
        assert_eq!(ts2.entries().len(), 1);
        assert!(ts2.revoke(&pk).unwrap());
        assert!(!ts2.contains(&pk));
        assert_eq!(ts2.pinned_key_for_addr("10.0.0.4:5000"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_address_maps_to_at_most_one_pinned_key() {
        let dir = std::env::temp_dir().join(format!("wooosh-trust-addr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ts = TrustStore::open(dir.join("trust.json")).unwrap();
        let old = [1u8; 32];
        let new = [2u8; 32];
        ts.insert(&old, "Old", None, Some("192.168.1.9:44777")).unwrap();
        // The same address is taken over by a different pinned identity
        // (device reinstalled and re-paired): the stale association is dropped
        // so the lookup never resolves to the retired key.
        ts.insert(&new, "New", None, Some("192.168.1.9:44777")).unwrap();
        assert_eq!(ts.pinned_key_for_addr("192.168.1.9:44777"), Some(new));
        assert_eq!(ts.get(&old).unwrap().last_addr, None);
        ts.note_addr(&old, "192.168.1.9:44777");
        assert_eq!(ts.pinned_key_for_addr("192.168.1.9:44777"), Some(old));
        assert_eq!(ts.get(&new).unwrap().last_addr, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn device_id_lookup_finds_the_pinned_key() {
        let dir = std::env::temp_dir().join(format!("wooosh-trust-did-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ts = TrustStore::open(dir.join("trust.json")).unwrap();
        let pk = [3u8; 32];
        ts.insert(&pk, "Peer", None, None).unwrap();
        let did = identity::device_id_string_for(&pk);
        assert_eq!(ts.pinned_key_for_device_id(&did), Some(pk));
        assert_eq!(ts.pinned_key_for_device_id("AAAA-BBBB"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
