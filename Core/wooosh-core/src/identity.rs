//! Device identity: Ed25519 keypair + DeviceID derivation (PROTOCOL.md §2).

use crate::error::WoooshError;
use crate::wordlist;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::{SigningKey, VerifyingKey};

#[derive(Clone)]
pub struct Identity {
    signing_key: SigningKey,
}

impl Identity {
    /// Load an identity from 32 secret-key bytes, or generate a new one.
    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, WoooshError> {
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| WoooshError::Crypto("identity key must be 32 bytes".into()))?;
        Ok(Self { signing_key: SigningKey::from_bytes(&arr) })
    }

    pub fn generate() -> Self {
        let mut rng = rand::rngs::OsRng;
        Self { signing_key: SigningKey::generate(&mut rng) }
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// PKCS#8 v2 DER of the private key (for rcgen / rustls).
    pub fn pkcs8_der(&self) -> Result<Vec<u8>, WoooshError> {
        let doc = self
            .signing_key
            .to_pkcs8_der()
            .map_err(|e| WoooshError::Crypto(format!("pkcs8 encode: {e}")))?;
        Ok(doc.as_bytes().to_vec())
    }

    /// DeviceID = BLAKE3(pubkey)[0..16].
    pub fn device_id(&self) -> [u8; 16] {
        device_id_for(&self.public_key_bytes())
    }

    pub fn device_id_string(&self) -> String {
        device_id_string_for(&self.public_key_bytes())
    }

    /// 6-word fingerprint phrase from BLAKE3(pubkey)[0..6].
    pub fn fingerprint_phrase(&self) -> String {
        fingerprint_phrase_for(&self.public_key_bytes())
    }
}

pub fn device_id_for(pubkey: &[u8; 32]) -> [u8; 16] {
    let hash = blake3::hash(pubkey);
    let mut id = [0u8; 16];
    id.copy_from_slice(&hash.as_bytes()[0..16]);
    id
}

/// Base32 (RFC 4648, no padding, upper-case) rendering, grouped by 4 with dashes.
pub fn render_device_id(id: &[u8; 16]) -> String {
    let raw = base32::encode(base32::Alphabet::Rfc4648 { padding: false }, id);
    raw.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn device_id_string_for(pubkey: &[u8; 32]) -> String {
    render_device_id(&device_id_for(pubkey))
}

pub fn fingerprint_phrase_for(pubkey: &[u8; 32]) -> String {
    let hash = blake3::hash(pubkey);
    wordlist::phrase(&hash.as_bytes()[0..6])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_stable_and_16_bytes() {
        let id = Identity::generate();
        let a = id.device_id();
        let b = device_id_for(&id.public_key_bytes());
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        let s = id.device_id_string();
        // 16 bytes -> 26 base32 chars -> 7 groups
        assert_eq!(s.replace('-', "").len(), 26);
        assert!(s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-'));
    }

    #[test]
    fn fingerprint_is_six_words() {
        let id = Identity::generate();
        assert_eq!(id.fingerprint_phrase().split(' ').count(), 6);
    }

    #[test]
    fn roundtrip_secret_bytes() {
        let id = Identity::generate();
        let id2 = Identity::from_secret_bytes(&id.secret_bytes()).unwrap();
        assert_eq!(id.public_key_bytes(), id2.public_key_bytes());
    }
}
