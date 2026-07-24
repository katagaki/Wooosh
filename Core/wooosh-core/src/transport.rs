//! QUIC transport (PROTOCOL.md §4.1 + §6 tuning).
//!
//! Self-signed X.509 certs wrapping the Ed25519 identity key. Custom rustls
//! verifiers on both sides accept any well-formed cert (the cert is an
//! envelope; CA chains are never evaluated) while still verifying the TLS 1.3
//! handshake signature against the cert's key. Pinning/trust decisions happen
//! above the TLS layer, using the Ed25519 SPKI extracted from the peer cert.

use crate::error::WoooshError;
use crate::identity::Identity;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

pub const ALPN: &[u8] = b"wooosh/1";

/// Extract the raw Ed25519 public key (32 bytes) from a certificate's
/// SubjectPublicKeyInfo. Errors if the SPKI is not Ed25519.
pub fn ed25519_spki_from_cert(cert: &CertificateDer<'_>) -> Result<[u8; 32], WoooshError> {
    let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref())
        .map_err(|e| WoooshError::Crypto(format!("x509 parse: {e}")))?;
    let spki = parsed.public_key();
    // Ed25519 OID = 1.3.101.112
    let oid = spki.algorithm.algorithm.to_id_string();
    if oid != "1.3.101.112" {
        return Err(WoooshError::Crypto(format!("peer key is not Ed25519 (oid {oid})")));
    }
    let key = spki.subject_public_key.data.as_ref();
    key.try_into().map_err(|_| WoooshError::Crypto("bad Ed25519 SPKI length".into()))
}

/// Extract the peer's Ed25519 key from an established QUIC connection.
pub fn peer_pubkey(conn: &quinn::Connection) -> Result<[u8; 32], WoooshError> {
    let identity = conn
        .peer_identity()
        .ok_or_else(|| WoooshError::Crypto("peer presented no certificate".into()))?;
    let certs = identity
        .downcast::<Vec<CertificateDer<'static>>>()
        .map_err(|_| WoooshError::Crypto("unexpected peer identity type".into()))?;
    let cert = certs.first().ok_or_else(|| WoooshError::Crypto("empty cert chain".into()))?;
    ed25519_spki_from_cert(cert)
}

/// Generate the self-signed cert (rcgen PKCS_ED25519) for our identity key.
pub fn make_cert(identity: &Identity) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), WoooshError> {
    let pkcs8 = identity.pkcs8_der()?;
    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(pkcs8.clone()),
        &rcgen::PKCS_ED25519,
    )
    .map_err(|e| WoooshError::Crypto(format!("rcgen keypair: {e}")))?;
    let params = rcgen::CertificateParams::new(vec!["wooosh".to_string()])
        .map_err(|e| WoooshError::Crypto(format!("rcgen params: {e}")))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| WoooshError::Crypto(format!("rcgen sign: {e}")))?;
    Ok((cert.der().clone(), PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8))))
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Server side: require a client cert, accept any well-formed one.
#[derive(Debug)]
struct AcceptAnyClientCert {
    provider: Arc<CryptoProvider>,
    no_subjects: Vec<DistinguishedName>,
}

impl ClientCertVerifier for AcceptAnyClientCert {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &self.no_subjects
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        // Well-formedness: must parse and carry an Ed25519 key.
        ed25519_spki_from_cert(end_entity)
            .map_err(|e| rustls::Error::General(e.to_string()))?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Client side: accept any well-formed server cert; pinning happens above TLS.
#[derive(Debug)]
struct AcceptAnyServerCert {
    provider: Arc<CryptoProvider>,
    /// When set, the handshake itself fails unless the server presents
    /// exactly this Ed25519 key (used for QR pairing and pinned reconnects).
    expected_key: Option<[u8; 32]>,
    /// The key the server actually presented, recorded even when the pin
    /// rejects it — a failed handshake yields no `peer_identity()`, and the
    /// KEY_CHANGED event is far more useful with the offending key attached.
    seen_key: Arc<std::sync::Mutex<Option<[u8; 32]>>>,
}

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let key = ed25519_spki_from_cert(end_entity)
            .map_err(|e| rustls::Error::General(e.to_string()))?;
        *self.seen_key.lock().unwrap_or_else(|e| e.into_inner()) = Some(key);
        if let Some(expected) = &self.expected_key {
            use subtle::ConstantTimeEq;
            if key.ct_eq(expected).unwrap_u8() != 1 {
                return Err(rustls::Error::General("WOOOSH_KEY_MISMATCH".into()));
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOffered,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Transport tuning per PROTOCOL.md §6.
pub fn transport_config() -> Arc<quinn::TransportConfig> {
    let mut tc = quinn::TransportConfig::default();
    tc.stream_receive_window(quinn::VarInt::from_u32(8 * 1024 * 1024));
    tc.receive_window(quinn::VarInt::from_u32(32 * 1024 * 1024));
    tc.send_window(32 * 1024 * 1024);
    tc.max_concurrent_uni_streams(quinn::VarInt::from_u32(8));
    tc.max_concurrent_bidi_streams(quinn::VarInt::from_u32(4));
    tc.keep_alive_interval(Some(Duration::from_secs(15)));
    tc.max_idle_timeout(Some(quinn::IdleTimeout::try_from(Duration::from_secs(60)).unwrap()));
    Arc::new(tc)
}

/// Build the QUIC endpoint: server config (mutual TLS required) listening on
/// `bind`, usable for outgoing connections too.
pub fn make_endpoint(identity: &Identity, bind: SocketAddr) -> Result<quinn::Endpoint, WoooshError> {
    let (cert, key) = make_cert(identity)?;
    let provider = provider();

    let mut server_crypto = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| WoooshError::Crypto(format!("tls versions: {e}")))?
        .with_client_cert_verifier(Arc::new(AcceptAnyClientCert {
            provider: provider.clone(),
            no_subjects: Vec::new(),
        }))
        .with_single_cert(vec![cert], key)
        .map_err(|e| WoooshError::Crypto(format!("server tls: {e}")))?;
    server_crypto.alpn_protocols = vec![ALPN.to_vec()];

    let quic_server = QuicServerConfig::try_from(server_crypto)
        .map_err(|e| WoooshError::Crypto(format!("quic server cfg: {e}")))?;
    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server));
    server_config.transport_config(transport_config());

    let endpoint = quinn::Endpoint::server(server_config, bind)
        .map_err(|e| WoooshError::Io(format!("bind {bind}: {e}")))?;
    Ok(endpoint)
}

/// Handle onto the key a client-side handshake observed on the peer's
/// certificate. Populated even when the pin rejected it.
pub type SeenKey = Arc<std::sync::Mutex<Option<[u8; 32]>>>;

/// Per-connection client config; `expected_key` enforces pinning inside the
/// TLS handshake itself. The returned [`SeenKey`] carries the key the peer
/// presented (available whether or not the pin matched).
pub fn client_config(
    identity: &Identity,
    expected_key: Option<[u8; 32]>,
) -> Result<(quinn::ClientConfig, SeenKey), WoooshError> {
    let (cert, key) = make_cert(identity)?;
    let provider = provider();
    let seen_key: SeenKey = Arc::new(std::sync::Mutex::new(None));
    let mut client_crypto = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| WoooshError::Crypto(format!("tls versions: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert {
            provider,
            expected_key,
            seen_key: seen_key.clone(),
        }))
        .with_client_auth_cert(vec![cert], key)
        .map_err(|e| WoooshError::Crypto(format!("client tls: {e}")))?;
    client_crypto.alpn_protocols = vec![ALPN.to_vec()];

    let quic_client = QuicClientConfig::try_from(client_crypto)
        .map_err(|e| WoooshError::Crypto(format!("quic client cfg: {e}")))?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(quic_client));
    cfg.transport_config(transport_config());
    Ok((cfg, seen_key))
}

/// SAS derivation (PROTOCOL.md §4.3): 6-digit code from the TLS exporter.
/// export_keying_material(label="EXPORTER-wooosh-sas", context=empty, 32 bytes),
/// first 4 bytes as big-endian u32, mod 1_000_000.
pub fn derive_sas(conn: &quinn::Connection) -> Result<u32, WoooshError> {
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, b"EXPORTER-wooosh-sas", b"")
        .map_err(|_| WoooshError::Crypto("exporter unavailable".into()))?;
    let n = u32::from_be_bytes([out[0], out[1], out[2], out[3]]);
    Ok(n % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_wraps_identity_key() {
        let id = Identity::generate();
        let (cert, _key) = make_cert(&id).unwrap();
        let extracted = ed25519_spki_from_cert(&cert).unwrap();
        assert_eq!(extracted, id.public_key_bytes());
    }
}
