//! One QUIC connection, two stacks: our own `quinn` endpoint on the LAN and an
//! `iroh` endpoint keyed by the same Ed25519 identity over the internet.
//!
//! iroh 1.x ships n0's `quinn` fork, so its connection and stream types are
//! distinct from ours. This module is the entire adaptation layer; everything
//! above it is transport-blind. Do not fork the protocol code to add a
//! transport, extend these enums.

use crate::error::WoooshError;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// Hole punching completes in a handful of round trips, so a coarse poll costs
/// nothing.
const DIRECT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// `app_code` is the QUIC *application* close code, the only reliable
/// rejection signal (PROTOCOL.md §4.1.2); transport failures leave it `None`.
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

/// A slot stream FINs at a file boundary (PROTOCOL.md §6), so a clean end of
/// stream must be distinguishable from a real error.
#[derive(Debug)]
pub enum ReadErr {
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

#[derive(Clone)]
pub enum Conn {
    Lan(quinn::Connection),
    Net(iroh::endpoint::Connection),
}

impl Conn {
    pub fn is_internet(&self) -> bool {
        matches!(self, Conn::Net(_))
    }

    /// The peer's authenticated Ed25519 identity key: the cert SPKI on the
    /// LAN, iroh's `EndpointId` over the internet. Both are the same 32 bytes
    /// for the same device, so a LAN pairing is already pinned over the
    /// internet (PROTOCOL.md §2, §9.3).
    pub fn peer_pubkey(&self) -> Result<[u8; 32], WoooshError> {
        match self {
            Conn::Lan(c) => crate::transport::quic_peer_pubkey(c),
            Conn::Net(c) => Ok(*c.remote_id().as_bytes()),
        }
    }

    /// Whether bulk data may flow right now. An internet connection comes up
    /// relayed and only becomes direct once hole punching succeeds; this is
    /// what keeps large transfers off shared relays (DESIGN.md §9.1). Only the
    /// *selected* path counts, since that is where the bytes would go.
    pub fn is_direct(&self) -> bool {
        match self {
            Conn::Lan(_) => true,
            Conn::Net(c) => c.paths().iter().any(|p| p.is_selected() && p.is_ip()),
        }
    }

    /// Polled rather than driven off `paths_stream`: the wait happens once per
    /// transfer and a borrowed stream would have to outlive it for no gain.
    pub async fn wait_for_direct(&self, timeout: Duration) -> bool {
        if self.is_direct() {
            return true;
        }
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            tokio::time::sleep(DIRECT_POLL_INTERVAL).await;
            if self.is_direct() {
                return true;
            }
        }
        false
    }

    /// `None` on the internet path on purpose: an iroh connection may be
    /// relayed or migrate, so it has no stable §4.5 `last_addr` pin hint and
    /// writing one would poison `pinned_key_for_addr` for the LAN path.
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
    /// Available on both stacks, so SAS is transcript-bound on either path.
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
