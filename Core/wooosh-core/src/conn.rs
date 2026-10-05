//! One QUIC connection, two paths: our LAN endpoint, or an internet session
//! whose socket hides hole punching and relaying (rendezvous/pathsock.rs).
//! Both are plain `quinn` connections, so everything above this module is
//! path-blind. Do not fork the protocol code per path; extend `Conn`.

use crate::error::WoooshError;
use crate::rendezvous::NetSession;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Punching completes in a handful of round trips, so a coarse poll costs nothing.
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

pub struct SendStream(quinn::SendStream);

impl SendStream {
    pub async fn write_all(&mut self, buf: &[u8]) -> Result<(), String> {
        self.0.write_all(buf).await.map_err(|e| e.to_string())
    }

    pub fn finish(&mut self) {
        let _ = self.0.finish();
    }
}

pub struct RecvStream(quinn::RecvStream);

impl RecvStream {
    pub async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), ReadErr> {
        self.0.read_exact(buf).await.map_err(ReadErr::from)
    }

    pub fn stop(&mut self, code: u32) {
        let _ = self.0.stop(quinn::VarInt::from_u32(code));
    }
}

#[derive(Clone)]
pub struct Conn {
    quic: quinn::Connection,
    /// Present on the internet path; keeps its socket, endpoint and room alive.
    net: Option<Arc<NetSession>>,
}

impl Conn {
    pub fn lan(quic: quinn::Connection) -> Self {
        Conn { quic, net: None }
    }

    pub fn net(session: NetSession) -> Self {
        Conn { quic: session.conn.clone(), net: Some(Arc::new(session)) }
    }

    pub fn is_internet(&self) -> bool {
        self.net.is_some()
    }

    /// The peer's authenticated Ed25519 identity key from its certificate,
    /// on either path (PROTOCOL.md §2, §9.7).
    pub fn peer_pubkey(&self) -> Result<[u8; 32], WoooshError> {
        crate::transport::quic_peer_pubkey(&self.quic)
    }

    /// Whether bulk data may flow uncapped right now. An internet session may
    /// be relayed until punching succeeds (PROTOCOL.md §9.6).
    pub fn is_direct(&self) -> bool {
        self.net.as_ref().map_or(true, |n| n.is_direct())
    }

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

    /// `None` on the internet path on purpose: the address quinn sees there is
    /// synthetic, and a NAT mapping is no §4.5 `last_addr` pin hint either.
    pub fn remote_address(&self) -> Option<SocketAddr> {
        match self.net {
            None => Some(self.quic.remote_address()),
            Some(_) => None,
        }
    }

    pub async fn open_bi(&self) -> Result<(SendStream, RecvStream), ConnErr> {
        let (s, r) = self.quic.open_bi().await.map_err(ConnErr::from)?;
        Ok((SendStream(s), RecvStream(r)))
    }

    pub async fn accept_bi(&self) -> Result<(SendStream, RecvStream), ConnErr> {
        let (s, r) = self.quic.accept_bi().await.map_err(ConnErr::from)?;
        Ok((SendStream(s), RecvStream(r)))
    }

    pub async fn open_uni(&self) -> Result<SendStream, ConnErr> {
        Ok(SendStream(self.quic.open_uni().await.map_err(ConnErr::from)?))
    }

    pub async fn accept_uni(&self) -> Result<RecvStream, ConnErr> {
        Ok(RecvStream(self.quic.accept_uni().await.map_err(ConnErr::from)?))
    }

    pub fn close(&self, code: u32, reason: &[u8]) {
        self.quic.close(quinn::VarInt::from_u32(code), reason)
    }

    pub async fn closed(&self) -> ConnErr {
        self.quic.closed().await.into()
    }

    pub fn close_reason(&self) -> Option<ConnErr> {
        self.quic.close_reason().map(ConnErr::from)
    }

    /// TLS 1.3 exporter, the input to the SAS derivation (PROTOCOL.md §4.3).
    pub fn export_keying_material(&self, out: &mut [u8], label: &[u8], context: &[u8]) -> Result<(), WoooshError> {
        self.quic
            .export_keying_material(out, label, context)
            .map_err(|_| WoooshError::Crypto("exporter unavailable".into()))
    }
}
