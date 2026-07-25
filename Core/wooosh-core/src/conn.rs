//! One QUIC connection, two stacks.
//!
//! Wooosh speaks the same protocol (PROTOCOL.md §4–§6) over two transports:
//!
//! - **LAN**: our own `quinn` endpoint with self-signed Ed25519 certificates
//!   and above-TLS key pinning (`transport.rs`).
//! - **Internet**: an [`iroh`] endpoint keyed by the *same* Ed25519 identity,
//!   which hole-punches and falls back to relaying (PROTOCOL.md §9).
//!
//! iroh 1.x does not use upstream `quinn`; it ships n0's fork (`noq`), so
//! `iroh::endpoint::Connection`, `SendStream` and `RecvStream` are distinct
//! types from ours and cannot be fed into the engine directly. This module is
//! the entire adaptation layer: three enums with the ~12 operations the engine
//! actually performs. **Everything above it — HELLO, OFFER/DECISION, the file
//! streams, the resume ledger, the trust logic — exists exactly once** and is
//! transport-blind. Do not fork the protocol code to add a transport; extend
//! these enums.

use crate::error::WoooshError;
use std::net::SocketAddr;

/// A closed/failed connection, normalized across the two stacks.
///
/// `app_code` is the QUIC *application* close code, which PROTOCOL.md §4.1.2
/// makes the only reliable rejection signal; transport-level failures leave it
/// `None`.
#[derive(Debug, Clone)]
pub struct ConnErr {
    pub app_code: Option<u32>,
    pub msg: String,
}

impl std::fmt::Display for ConnErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl From<quinn::ConnectionError> for ConnErr {
    fn from(e: quinn::ConnectionError) -> Self {
        let app_code = match &e {
            quinn::ConnectionError::ApplicationClosed(a) => u32::try_from(a.error_code.into_inner()).ok(),
            _ => None,
        };
        ConnErr { app_code, msg: e.to_string() }
    }
}

impl From<iroh::endpoint::ConnectionError> for ConnErr {
    fn from(e: iroh::endpoint::ConnectionError) -> Self {
        let app_code = match &e {
            iroh::endpoint::ConnectionError::ApplicationClosed(a) => {
                u32::try_from(a.error_code.into_inner()).ok()
            }
            _ => None,
        };
        ConnErr { app_code, msg: e.to_string() }
    }
}

impl From<ConnErr> for WoooshError {
    fn from(e: ConnErr) -> Self {
        WoooshError::Connect(e.msg)
    }
}

/// Outcome of a `read_exact` that must distinguish "clean end of stream" from
/// a real error: a slot stream FINs at a file boundary (PROTOCOL.md §6).
#[derive(Debug)]
pub enum ReadErr {
    /// The stream finished after `0` further bytes — a clean boundary.
    FinishedEarly(usize),
    Other(String),
}

impl std::fmt::Display for ReadErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadErr::FinishedEarly(n) => write!(f, "stream finished early ({n} bytes read)"),
            ReadErr::Other(m) => f.write_str(m),
        }
    }
}

impl From<quinn::ReadExactError> for ReadErr {
    fn from(e: quinn::ReadExactError) -> Self {
        match e {
            quinn::ReadExactError::FinishedEarly(n) => ReadErr::FinishedEarly(n),
            other => ReadErr::Other(other.to_string()),
        }
    }
}

impl From<iroh::endpoint::ReadExactError> for ReadErr {
    fn from(e: iroh::endpoint::ReadExactError) -> Self {
        match e {
            iroh::endpoint::ReadExactError::FinishedEarly(n) => ReadErr::FinishedEarly(n),
            other => ReadErr::Other(other.to_string()),
        }
    }
}

// ---------- streams ----------

pub enum SendStream {
    Lan(quinn::SendStream),
    Net(iroh::endpoint::SendStream),
}

impl SendStream {
    pub async fn write_all(&mut self, buf: &[u8]) -> Result<(), String> {
        match self {
            SendStream::Lan(s) => s.write_all(buf).await.map_err(|e| e.to_string()),
            SendStream::Net(s) => s.write_all(buf).await.map_err(|e| e.to_string()),
        }
    }

    pub fn finish(&mut self) {
        match self {
            SendStream::Lan(s) => {
                let _ = s.finish();
            }
            SendStream::Net(s) => {
                let _ = s.finish();
            }
        }
    }
}

pub enum RecvStream {
    Lan(quinn::RecvStream),
    Net(iroh::endpoint::RecvStream),
}

impl RecvStream {
    pub async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), ReadErr> {
        match self {
            RecvStream::Lan(s) => s.read_exact(buf).await.map_err(ReadErr::from),
            RecvStream::Net(s) => s.read_exact(buf).await.map_err(ReadErr::from),
        }
    }

    /// Ask the peer to stop sending, with a wooosh close code.
    pub fn stop(&mut self, code: u32) {
        match self {
            RecvStream::Lan(s) => {
                let _ = s.stop(quinn::VarInt::from_u32(code));
            }
            RecvStream::Net(s) => {
                let _ = s.stop(iroh::endpoint::VarInt::from_u32(code));
            }
        }
    }
}

// ---------- connections ----------

#[derive(Clone)]
pub enum Conn {
    Lan(quinn::Connection),
    Net(iroh::endpoint::Connection),
}

impl Conn {
    /// True for the internet (iroh) path. Used where the two transports have
    /// genuinely different semantics — currently only address bookkeeping.
    pub fn is_internet(&self) -> bool {
        matches!(self, Conn::Net(_))
    }

    /// The peer's authenticated Ed25519 identity key.
    ///
    /// LAN: the SubjectPublicKeyInfo of the certificate the peer proved
    /// possession of in the TLS handshake. Internet: iroh's `EndpointId`,
    /// which *is* an Ed25519 public key authenticated by the same TLS
    /// handshake. Both are the same 32 bytes for the same device, which is
    /// why a peer paired on the LAN is already pinned over the internet
    /// (PROTOCOL.md §2, §9.3).
    pub fn peer_pubkey(&self) -> Result<[u8; 32], WoooshError> {
        match self {
            Conn::Lan(c) => crate::transport::quic_peer_pubkey(c),
            Conn::Net(c) => Ok(*c.remote_id().as_bytes()),
        }
    }

    /// The peer's `ip:port`, when there is a stable one to record.
    ///
    /// `None` on the internet path on purpose: an iroh connection may be
    /// relayed or may migrate between paths, so there is no address that could
    /// serve as the §4.5 `last_addr` pin hint. Writing one would poison
    /// `pinned_key_for_addr` for the LAN path.
    pub fn remote_address(&self) -> Option<SocketAddr> {
        match self {
            Conn::Lan(c) => Some(c.remote_address()),
            Conn::Net(_) => None,
        }
    }

    pub async fn open_bi(&self) -> Result<(SendStream, RecvStream), ConnErr> {
        match self {
            Conn::Lan(c) => {
                let (s, r) = c.open_bi().await.map_err(ConnErr::from)?;
                Ok((SendStream::Lan(s), RecvStream::Lan(r)))
            }
            Conn::Net(c) => {
                let (s, r) = c.open_bi().await.map_err(ConnErr::from)?;
                Ok((SendStream::Net(s), RecvStream::Net(r)))
            }
        }
    }

    pub async fn accept_bi(&self) -> Result<(SendStream, RecvStream), ConnErr> {
        match self {
            Conn::Lan(c) => {
                let (s, r) = c.accept_bi().await.map_err(ConnErr::from)?;
                Ok((SendStream::Lan(s), RecvStream::Lan(r)))
            }
            Conn::Net(c) => {
                let (s, r) = c.accept_bi().await.map_err(ConnErr::from)?;
                Ok((SendStream::Net(s), RecvStream::Net(r)))
            }
        }
    }

    pub async fn open_uni(&self) -> Result<SendStream, ConnErr> {
        match self {
            Conn::Lan(c) => Ok(SendStream::Lan(c.open_uni().await.map_err(ConnErr::from)?)),
            Conn::Net(c) => Ok(SendStream::Net(c.open_uni().await.map_err(ConnErr::from)?)),
        }
    }

    pub async fn accept_uni(&self) -> Result<RecvStream, ConnErr> {
        match self {
            Conn::Lan(c) => Ok(RecvStream::Lan(c.accept_uni().await.map_err(ConnErr::from)?)),
            Conn::Net(c) => Ok(RecvStream::Net(c.accept_uni().await.map_err(ConnErr::from)?)),
        }
    }

    pub fn close(&self, code: u32, reason: &[u8]) {
        match self {
            Conn::Lan(c) => c.close(quinn::VarInt::from_u32(code), reason),
            Conn::Net(c) => c.close(iroh::endpoint::VarInt::from_u32(code), reason),
        }
    }

    pub async fn closed(&self) -> ConnErr {
        match self {
            Conn::Lan(c) => c.closed().await.into(),
            Conn::Net(c) => c.closed().await.into(),
        }
    }

    pub fn close_reason(&self) -> Option<ConnErr> {
        match self {
            Conn::Lan(c) => c.close_reason().map(ConnErr::from),
            Conn::Net(c) => c.close_reason().map(ConnErr::from),
        }
    }

    /// TLS 1.3 exporter, the input to the SAS derivation (PROTOCOL.md §4.3).
    /// Available on both stacks, so SAS is transcript-bound and MITM-detecting
    /// over the internet path exactly as it is on the LAN.
    pub fn export_keying_material(
        &self,
        out: &mut [u8],
        label: &[u8],
        context: &[u8],
    ) -> Result<(), WoooshError> {
        let unavailable = || WoooshError::Crypto("exporter unavailable".into());
        match self {
            Conn::Lan(c) => c
                .export_keying_material(out, label, context)
                .map(|_| ())
                .map_err(|_| unavailable()),
            Conn::Net(c) => c
                .export_keying_material(out, label, context)
                .map(|_| ())
                .map_err(|_| unavailable()),
        }
    }
}
