//! Client half of rendezvous admission (PROTOCOL.md §9.2). The platform proof
//! comes from the shell's `Attestor`; this module decides which proof to ask
//! for and caches what the Worker hands back, so a device attests about once a
//! day rather than once per transfer (Play Integrity has a daily quota).

use crate::api::{AttestationKind, Attestor};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// The Worker issues tokens for 24 h; refresh a little early so a token never
/// expires between the check here and the Worker's.
const TOKEN_LIFETIME_SECS: u64 = 24 * 3600 - 300;

pub enum Evidence {
    Token(String),
    AppleAttest { key_id: Vec<u8>, attestation: Vec<u8> },
    AppleAssert { key_credential: String, assertion: Vec<u8> },
    Android { token: String, install_id: Vec<u8> },
    Tpm { ek_cert: Vec<u8>, ek_chain: Vec<Vec<u8>>, ak_public: Vec<u8> },
    Anonymous,
}

impl Evidence {
    pub fn from_cache(&self) -> bool {
        matches!(self, Evidence::Token(_) | Evidence::AppleAssert { .. })
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    token: Option<String>,
    #[serde(default)]
    token_exp: u64,
    key_credential: Option<String>,
    /// Android: random per-install ID bound into the Play Integrity request
    /// hash; the quota identity, since Play Integrity exposes none.
    install_id: Option<String>,
}

pub struct Admission {
    attestor: Mutex<Option<Arc<dyn Attestor>>>,
    path: Option<PathBuf>,
    cache: Mutex<Cache>,
    /// After a fresh attestation was refused, the retry goes in anonymously
    /// rather than repeating a proof that just failed.
    fallback_anonymous: Mutex<bool>,
    last_was_cached: Mutex<bool>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn sha256(parts: &[&[u8]]) -> Vec<u8> {
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    for p in parts {
        ctx.update(p);
    }
    ctx.finish().as_ref().to_vec()
}

fn b64(b: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn unb64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).ok()
}

impl Admission {
    /// `path` is where the cache persists; `None` keeps it in memory only.
    pub fn new(path: Option<PathBuf>) -> Self {
        let cache = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Admission {
            attestor: Mutex::new(None),
            path,
            cache: Mutex::new(cache),
            fallback_anonymous: Mutex::new(false),
            last_was_cached: Mutex::new(false),
        }
    }

    pub fn set_attestor(&self, a: Option<Arc<dyn Attestor>>) {
        *self.attestor.lock().unwrap() = a;
        // A token minted under another attestor's tier must not outlive it.
        let mut c = self.cache.lock().unwrap();
        c.token = None;
        c.token_exp = 0;
        drop(c);
        self.persist();
    }

    fn persist(&self) {
        let Some(path) = &self.path else { return };
        let bytes = serde_json::to_vec(&*self.cache.lock().unwrap()).unwrap_or_default();
        let tmp = path.with_extension(format!("tmp{}", rand::random::<u32>()));
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    pub async fn evidence(&self, nonce: &[u8]) -> Evidence {
        let ev = self.evidence_inner(nonce).await;
        *self.last_was_cached.lock().unwrap() = ev.from_cache();
        ev
    }

    async fn evidence_inner(&self, nonce: &[u8]) -> Evidence {
        if std::mem::take(&mut *self.fallback_anonymous.lock().unwrap()) {
            return Evidence::Anonymous;
        }
        {
            let c = self.cache.lock().unwrap();
            if let Some(t) = &c.token {
                if c.token_exp > now() {
                    return Evidence::Token(t.clone());
                }
            }
        }
        let Some(att) = self.attestor.lock().unwrap().clone() else { return Evidence::Anonymous };
        // Host callbacks may block (network calls to Apple/Google, TPM I/O), so
        // they never run on a runtime worker.
        let kind = {
            let att = att.clone();
            tokio::task::spawn_blocking(move || att.kind()).await.unwrap_or(AttestationKind::None)
        };
        match kind {
            AttestationKind::AppAttest => {
                let cdh = sha256(&[nonce]);
                let cred = self.cache.lock().unwrap().key_credential.clone();
                let fresh = cred.is_none();
                let r = tokio::task::spawn_blocking(move || att.app_attest(cdh, fresh)).await.ok().flatten();
                match (r, cred) {
                    (Some(ev), Some(key_credential)) if ev.assertion.is_some() => {
                        Evidence::AppleAssert { key_credential, assertion: ev.assertion.unwrap() }
                    }
                    (Some(ev), _) if ev.attestation.is_some() => {
                        Evidence::AppleAttest { key_id: ev.key_id, attestation: ev.attestation.unwrap() }
                    }
                    _ => {
                        // The key may be gone (reinstall, restore): attest afresh next time.
                        self.cache.lock().unwrap().key_credential = None;
                        self.persist();
                        Evidence::Anonymous
                    }
                }
            }
            AttestationKind::PlayIntegrity => {
                let install_id = self.install_id();
                let hash = hex::encode(sha256(&[nonce, &install_id]));
                match tokio::task::spawn_blocking(move || att.play_integrity(hash)).await.ok().flatten() {
                    Some(token) => Evidence::Android { token, install_id },
                    None => Evidence::Anonymous,
                }
            }
            AttestationKind::Tpm => match tokio::task::spawn_blocking(move || att.tpm_evidence()).await.ok().flatten() {
                Some(ev) => Evidence::Tpm { ek_cert: ev.ek_cert, ek_chain: ev.ek_chain, ak_public: ev.ak_public },
                None => Evidence::Anonymous,
            },
            AttestationKind::None => Evidence::Anonymous,
        }
    }

    pub async fn tpm_activate(&self, blob: Vec<u8>, secret: Vec<u8>) -> Option<Vec<u8>> {
        let att = self.attestor.lock().unwrap().clone()?;
        tokio::task::spawn_blocking(move || att.tpm_activate(blob, secret)).await.ok().flatten()
    }

    fn install_id(&self) -> Vec<u8> {
        let mut c = self.cache.lock().unwrap();
        if let Some(id) = c.install_id.as_deref().and_then(unb64).filter(|v| v.len() == 16) {
            return id;
        }
        let id: [u8; 16] = rand::random();
        c.install_id = Some(b64(&id));
        drop(c);
        self.persist();
        id.to_vec()
    }

    pub fn remember(&self, token: Option<&str>, key_credential: Option<&str>) {
        let mut c = self.cache.lock().unwrap();
        if let Some(t) = token {
            c.token = Some(t.to_string());
            c.token_exp = now() + TOKEN_LIFETIME_SECS;
        }
        if let Some(k) = key_credential {
            c.key_credential = Some(k.to_string());
        }
        drop(c);
        self.persist();
    }

    /// The Worker refused what was sent. Cached material is dropped so the
    /// retry attests afresh; a fresh proof that was refused is not repeated.
    pub fn forget(&self) {
        let mut c = self.cache.lock().unwrap();
        c.token = None;
        c.token_exp = 0;
        c.key_credential = None;
        drop(c);
        self.persist();
        if !*self.last_was_cached.lock().unwrap() {
            *self.fallback_anonymous.lock().unwrap() = true;
        }
    }
}
