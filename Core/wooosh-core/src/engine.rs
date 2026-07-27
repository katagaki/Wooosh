//! The connection / pairing / transfer engine. Events leave through a std mpsc
//! channel, pumped to the host off-runtime so callbacks never block tokio.

use crate::api::{
    CoreEvent, DeviceType, FileKind, OfferedFile, TransferDirection, TrustedPeer, Visibility,
};
use crate::conn::{Conn, ConnErr, ReadErr, RecvStream, SendStream};
use crate::control::{FileMeta, Msg, ResumeHave, StreamHeader, MAX_FRAME, PROTOCOL_VERSION};
use crate::error::{close_codes, WoooshError};
use crate::identity::{self, Identity};
use crate::inet::{self, NetTicket, TicketPending};
use crate::ledger::{rehash_prefix, Ledger, LedgerFile};
use crate::pairing::{self, QrPayload, QrPending};
use crate::sanitize;
use crate::transport;
use crate::trust::TrustStore;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Notify};

const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole-attempt budget, not per hint; covers three quinn PTO retries.
const PAIR_CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
const DECISION_TIMEOUT: Duration = Duration::from_secs(120);
const PAIR_REPLY_TIMEOUT: Duration = Duration::from_secs(20);
/// Internet dial budget (§9.3): relay, candidates and punching precede data.
const NET_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Home-relay wait before publishing; direct candidates may still suffice.
const RELAY_READY_TIMEOUT: Duration = Duration::from_secs(15);
/// Hole-punch upgrade wait before falling back to the relay (DESIGN.md §9.1).
const DIRECT_PATH_TIMEOUT: Duration = Duration::from_secs(15);

/// Per-file cap on a **relayed** connection (DESIGN.md §9.1); direct is
/// uncapped. A relay is somebody else's bandwidth.
pub const RELAY_MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
const SMALL_FILE_LIMIT: u64 = 1024 * 1024; // <1 MiB => pipelined
const MAX_SLOTS: usize = 4;
const CHUNK: usize = 1024 * 1024;
const FSYNC_INTERVAL: u64 = 16 * 1024 * 1024;
const PROGRESS_INTERVAL: u64 = 8 * 1024 * 1024;

pub struct EngineConfig {
    pub bind_addr: SocketAddr,
    /// Internet-path relay selection; see `inet::bind_endpoint`.
    pub relay_urls: Option<Vec<String>>,
    pub staging_dir: PathBuf,
    pub trust_store_path: PathBuf,
    pub device_name: String,
    pub device_type: String,
    pub visibility: Visibility,
}

pub struct TransferStats {
    pub is_sender: bool,
    pub bytes_this_attempt: u64,
    pub resumed_from: u64,
    pub total: u64,
}

struct RuntimeCfg {
    device_name: String,
    device_type: String,
    visibility: Visibility,
}

pub struct Peer {
    pub conn: Conn,
    pub pubkey: [u8; 32],
    pub device_id: String,
    pub trusted: AtomicBool,
    /// Redeemed ticket (§9.4): one session, never persisted. The only gate to
    /// file data for an unpinned internet peer.
    pub ticket_authorized: AtomicBool,
    pub dn: Mutex<String>,
    /// Peer's HELLO `dt` (PROTOCOL.md §4.1), verbatim wire string.
    pub dt: Mutex<String>,
    out_tx: mpsc::UnboundedSender<Msg>,
}

impl Peer {
    fn send_msg(&self, msg: Msg) {
        let _ = self.out_tx.send(msg);
    }

    fn device_type(&self) -> Option<DeviceType> {
        DeviceType::from_wire(&self.dt.lock().unwrap())
    }

    fn fingerprint(&self) -> String {
        identity::fingerprint_phrase_for(&self.pubkey)
    }
}

struct SendFile {
    path: PathBuf,
    meta: FileMeta,
}

struct SendTransfer {
    tid: [u8; 16],
    peer_id: String,
    files: BTreeMap<u32, SendFile>,
    total: u64,
    accepted: Mutex<Option<Vec<u32>>>,
    offsets: Mutex<HashMap<u32, u64>>,
    done: Mutex<HashMap<u32, (bool, Option<String>)>>,
    decision_notify: Notify,
    resume_notify: Notify,
    cancelled: AtomicBool,
    bytes_this_attempt: AtomicU64,
    resumed_from: AtomicU64,
    started_at: Mutex<Instant>,
    done_emitted: AtomicBool,
}

struct RecvFileState {
    meta: FileMeta,
    safe_name: String,
    safe_rel: Option<PathBuf>,
    verified_off: u64,
    done: bool,
    ok: Option<bool>,
    hasher: Option<blake3::Hasher>,
    active: bool,
}

struct RecvTransfer {
    tid: [u8; 16],
    peer_id: String,
    peer_pubkey: [u8; 32],
    dir: PathBuf,
    /// Authority for `staging/<tid>/ledger.json`; 4 slot streams write it.
    ledger: Mutex<Option<Ledger>>,
    files: Mutex<BTreeMap<u32, RecvFileState>>,
    invalid: Mutex<Vec<(u32, String)>>,
    accepted: Mutex<Option<HashSet<u32>>>,
    cancelled: AtomicBool,
    bytes_this_attempt: AtomicU64,
    started_at: Mutex<Instant>,
    total: u64,
}

struct SasState {
    code: u32,
    local_confirmed: bool,
    remote_confirmed: bool,
    created: Instant,
}

pub struct Engine {
    inner: Arc<Inner>,
}

struct Inner {
    identity: Identity,
    endpoint: quinn::Endpoint,
    /// Lets the synchronous `shutdown` drive iroh's asynchronous close.
    rt: tokio::runtime::Handle,
    /// Changing this rebinds the endpoint; see `set_relay_urls`.
    relay_urls: Mutex<Option<Vec<String>>>,
    /// Bound lazily so a LAN-only install never talks to a relay.
    iroh: tokio::sync::Mutex<Option<iroh::Endpoint>>,
    ticket_pending: Mutex<Option<TicketPending>>,
    local_addr: SocketAddr,
    cfg: Mutex<RuntimeCfg>,
    trust: TrustStore,
    staging: PathBuf,
    events: std::sync::mpsc::Sender<CoreEvent>,
    peers: Mutex<HashMap<String, Arc<Peer>>>,
    qr_pending: Mutex<Option<QrPending>>,
    qr_waiters: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    sas: Mutex<HashMap<String, SasState>>,
    sends: Mutex<HashMap<[u8; 16], Arc<SendTransfer>>>,
    recvs: Mutex<HashMap<[u8; 16], Arc<RecvTransfer>>>,
}

fn tid_hex(tid: &[u8; 16]) -> String {
    hex::encode(tid)
}

fn elapsed_ms(started_at: &Mutex<Instant>) -> u64 {
    started_at
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

pub fn parse_tid(s: &str) -> Result<[u8; 16], WoooshError> {
    let v = hex::decode(s).map_err(|_| WoooshError::UnknownTransfer(s.into()))?;
    v.try_into().map_err(|_| WoooshError::UnknownTransfer(s.into()))
}

fn guess_mime(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "heic" => "image/heic",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn kind_for_mime(mime: &str) -> FileKind {
    if mime.starts_with("image/") {
        FileKind::Photo
    } else if mime.starts_with("video/") {
        FileKind::Video
    } else {
        FileKind::Document
    }
}

/// Rejections travel as close codes, never frames (§4.1): `close()` discards
/// buffered stream data.
fn close_reason_error(conn: &Conn) -> Option<WoooshError> {
    app_close_error(&conn.close_reason()?)
}

fn app_close_error(err: &ConnErr) -> Option<WoooshError> {
    Some(match err.app_code? {
        close_codes::PAIRING_REQUIRED => WoooshError::PairingRequired,
        close_codes::VERSION_MISMATCH => WoooshError::VersionMismatch,
        close_codes::QR_KEY_MISMATCH => WoooshError::QrKeyMismatch,
        close_codes::KEY_CHANGED => WoooshError::KeyChanged,
        close_codes::TOKEN_INVALID => {
            WoooshError::Pairing("TOKEN_INVALID: pairing token rejected by peer".into())
        }
        close_codes::UNTRUSTED_MSG => WoooshError::Protocol(
            "UNTRUSTED_MSG: peer refused a message not allowed on an untrusted channel".into(),
        ),
        _ => return None,
    })
}

struct PairFailure {
    err: WoooshError,
    /// The key the QR promised; `None` only when the payload did not parse.
    pubkey: Option<[u8; 32]>,
    /// A `PairingResult` was already emitted by the PAIR_REJECT dispatch.
    reported: bool,
}

impl PairFailure {
    fn new(err: WoooshError, pubkey: Option<[u8; 32]>) -> Self {
        Self { err, pubkey, reported: false }
    }

    fn reported(err: WoooshError) -> Self {
        Self { err, pubkey: None, reported: true }
    }
}

/// Typed close-code / key error beats transport error beats bare timeout.
fn connect_error_rank(e: &WoooshError) -> u8 {
    match e {
        WoooshError::Connect(m) if m.starts_with("connect timeout") => 0,
        WoooshError::Connect(_) => 1,
        _ => 2,
    }
}

fn local_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // no packets sent for UDP connect
    s.local_addr().ok().map(|a| a.ip())
}

/// Address hints for a pairing QR (PROTOCOL.md §4.2), best-first. Loopback is
/// last: a QR is normally scanned by another device. Only advertise what the
/// bind address makes truthful.
fn pairing_hints(bind_ip: IpAddr, lan_ip: Option<IpAddr>, port: u16) -> Vec<String> {
    let mut hints: Vec<String> = Vec::new();
    let mut push = |ip: IpAddr| {
        let h = SocketAddr::new(ip, port).to_string();
        if !hints.contains(&h) {
            hints.push(h);
        }
    };
    if bind_ip.is_unspecified() {
        if let Some(ip) = lan_ip.filter(|ip| !ip.is_loopback() && !ip.is_unspecified()) {
            push(ip);
        }
        push(match bind_ip {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        });
    } else {
        push(bind_ip);
    }
    hints
}

impl Engine {
    /// Must be called from within a tokio runtime context.
    pub fn new(
        cfg: EngineConfig,
        identity: Identity,
        events: std::sync::mpsc::Sender<CoreEvent>,
    ) -> Result<Arc<Engine>, WoooshError> {
        std::fs::create_dir_all(&cfg.staging_dir)?;
        let trust = TrustStore::open(cfg.trust_store_path.clone())?;
        let endpoint = transport::make_endpoint(&identity, cfg.bind_addr)?;
        let local_addr = endpoint.local_addr().map_err(WoooshError::from)?;
        let inner = Arc::new(Inner {
            identity,
            endpoint,
            local_addr,
            cfg: Mutex::new(RuntimeCfg {
                device_name: cfg.device_name,
                device_type: cfg.device_type,
                visibility: cfg.visibility,
            }),
            trust,
            staging: cfg.staging_dir,
            events,
            peers: Mutex::new(HashMap::new()),
            rt: tokio::runtime::Handle::current(),
            relay_urls: Mutex::new(cfg.relay_urls),
            iroh: tokio::sync::Mutex::new(None),
            ticket_pending: Mutex::new(None),
            qr_pending: Mutex::new(None),
            qr_waiters: Mutex::new(HashMap::new()),
            sas: Mutex::new(HashMap::new()),
            sends: Mutex::new(HashMap::new()),
            recvs: Mutex::new(HashMap::new()),
        });
        let accept_inner = inner.clone();
        tokio::spawn(async move { accept_inner.accept_loop().await });
        Ok(Arc::new(Engine { inner }))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr
    }

    pub fn identity(&self) -> &Identity {
        &self.inner.identity
    }

    pub fn set_visibility(&self, v: Visibility) {
        self.inner.cfg.lock().unwrap().visibility = v;
    }

    pub fn shutdown(&self) {
        self.inner.endpoint.close(close_codes::BYE.into(), b"bye");
        // Dropping the iroh endpoint unclosed aborts its socket.
        *self.inner.ticket_pending.lock().unwrap() = None;
        let ep = self.inner.iroh.try_lock().ok().and_then(|mut g| g.take());
        if let Some(ep) = ep {
            if tokio::runtime::Handle::try_current().is_err() {
                self.inner.rt.block_on(async {
                    let _ = tokio::time::timeout(Duration::from_secs(2), ep.close()).await;
                });
            } else {
                // block_on inside a runtime thread panics; settle for a spawn.
                self.inner.rt.spawn(async move { ep.close().await });
            }
        }
    }

    pub fn trusted_peers(&self) -> Vec<TrustedPeer> {
        self.inner
            .trust
            .entries()
            .into_iter()
            .map(|(pubkey, e)| TrustedPeer {
                pubkey: pubkey.to_vec(),
                device_id: e.device_id,
                device_name: e.dn,
                device_type: e.dt.as_deref().and_then(DeviceType::from_wire),
                fingerprint: identity::fingerprint_phrase_for(&pubkey),
                paired_at: e.paired_at,
                last_seen: e.last_seen,
            })
            .collect()
    }

    pub fn revoke_peer(&self, pubkey: &[u8; 32]) -> Result<bool, WoooshError> {
        let removed = self.inner.trust.revoke(pubkey)?;
        if removed {
            if let Some(p) = self
                .inner
                .peers
                .lock()
                .unwrap()
                .get(&identity::device_id_string_for(pubkey))
            {
                p.trusted.store(false, Ordering::SeqCst);
            }
        }
        Ok(removed)
    }

    pub fn begin_pairing_qr(&self) -> String {
        let (pending, token) = QrPending::new();
        *self.inner.qr_pending.lock().unwrap() = Some(pending);
        let hints = pairing_hints(
            self.inner.local_addr.ip(),
            local_ip(),
            self.inner.local_addr.port(),
        );
        QrPayload {
            version: 1,
            pubkey: self.inner.identity.public_key_bytes(),
            token,
            dn: Some(self.inner.cfg.lock().unwrap().device_name.clone()),
            hints,
            expires_unix: pairing::new_expiry_unix(),
        }
        .encode()
    }

    /// Sender side of QR pairing (PROTOCOL.md §4.2). Every exit must also emit
    /// a `PairingResult`, or the shell's event-driven pairing UI hangs.
    pub async fn pair_with_qr(&self, payload: &str) -> Result<String, WoooshError> {
        match self.pair_with_qr_inner(payload).await {
            Ok(peer_id) => Ok(peer_id),
            Err(f) => {
                if !f.reported {
                    self.inner.emit_pairing_failure(f.pubkey, f.err.to_string());
                }
                Err(f.err)
            }
        }
    }

    async fn pair_with_qr_inner(&self, payload: &str) -> Result<String, PairFailure> {
        let qr = QrPayload::parse(payload).map_err(|e| PairFailure::new(e, None))?;
        let key = qr.pubkey;
        if qr.is_expired() {
            let e = WoooshError::Pairing("QR payload expired".into());
            return Err(PairFailure::new(e, Some(key)));
        }
        let conn = self
            .inner
            .race_connect_qr(&qr.hints, key)
            .await
            .map_err(|e| PairFailure::new(e, Some(key)))?;
        let peer = self
            .inner
            .clone()
            .run_connection(conn, true)
            .await
            .map_err(|e| PairFailure::new(e, Some(key)))?;

        self.redeem_pair_token(peer, qr.token, key).await
    }

    /// §4.2 steps 3–4; LAN QR and internet tickets must never drift apart.
    async fn redeem_pair_token(
        &self,
        peer: Arc<Peer>,
        token: [u8; 32],
        key: [u8; 32],
    ) -> Result<String, PairFailure> {
        let (tx, rx) = oneshot::channel();
        self.inner
            .qr_waiters
            .lock()
            .unwrap()
            .insert(peer.device_id.clone(), tx);
        peer.send_msg(Msg::PairRequest { token: Some(token.to_vec()) });
        // A rejecting peer's close discards the buffered PAIR_REJECT frame, so
        // race the two or a rejection reads as a timeout.
        let conn = peer.conn.clone();
        let outcome = tokio::time::timeout(PAIR_REPLY_TIMEOUT, async move {
            tokio::select! {
                reply = rx => Ok(reply),
                closed = conn.closed() => Err(closed),
            }
        })
        .await;
        // A leftover waiter would be resolved by a later reply on this id.
        if !matches!(outcome, Ok(Ok(Ok(_)))) {
            self.inner.qr_waiters.lock().unwrap().remove(&peer.device_id);
        }
        match outcome {
            Ok(Ok(Ok(true))) => Ok(peer.device_id.clone()),
            // The PAIR_REJECT frame arrived, so `dispatch` already emitted.
            Ok(Ok(Ok(false))) => Err(PairFailure::reported(WoooshError::Pairing(
                "pairing rejected".into(),
            ))),
            Ok(Ok(Err(_))) => Err(PairFailure::new(
                WoooshError::Pairing("pairing aborted locally".into()),
                Some(key),
            )),
            Ok(Err(closed)) => Err(PairFailure::new(
                app_close_error(&closed).unwrap_or_else(|| {
                    WoooshError::Pairing(format!("connection closed while pairing: {closed}"))
                }),
                Some(key),
            )),
            Err(_) => Err(PairFailure::new(
                WoooshError::Pairing("timed out waiting for PAIR_ACCEPT".into()),
                Some(key),
            )),
        }
    }

    /// **The publisher is the sender** (§9.2), inverting the LAN OFFER rule.
    pub async fn begin_internet_ticket(&self) -> Result<String, WoooshError> {
        let ep = self.inner.iroh_endpoint().await?;
        // A ticket with no relay and no direct candidate cannot be dialled.
        let relays_off = self
            .inner
            .relay_urls
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|r| r.is_empty());
        let deadline = Instant::now() + RELAY_READY_TIMEOUT;
        let addr = loop {
            let a = ep.addr();
            let usable = if relays_off {
                a.ip_addrs().any(|s| !s.ip().is_unspecified())
            } else {
                a.relay_urls().next().is_some()
            };
            if usable || Instant::now() >= deadline {
                break a;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        // Relays on but none reachable: the code would only work on their LAN.
        if !relays_off && addr.relay_urls().next().is_none() {
            return Err(WoooshError::Connect(
                "no relay could be reached to publish a ticket".into(),
            ));
        }
        let (pending, token) = TicketPending::new();
        *self.inner.ticket_pending.lock().unwrap() = Some(pending);
        let ticket = NetTicket {
            version: 1,
            node_id: self.inner.identity.public_key_bytes(),
            token,
            dn: Some(self.inner.cfg.lock().unwrap().device_name.clone()),
            relay: addr.relay_urls().next().map(|u| u.to_string()),
            // A wildcard address is not somewhere a peer can dial.
            direct: addr
                .ip_addrs()
                .filter(|s| !s.ip().is_unspecified())
                .map(|a| a.to_string())
                .collect(),
            expires_unix: inet::new_expiry_unix(),
        };
        Ok(ticket.encode())
    }

    /// DESIGN.md §9.1. `None` is n0's public set, `Some(&[])` disables relays
    /// and address lookup entirely. Rebinding invalidates outstanding tickets,
    /// which advertise a relay this device no longer uses.
    pub async fn set_relay_urls(&self, urls: Option<Vec<String>>) -> Result<(), WoooshError> {
        // Validated before teardown, so a typo leaves the working config up.
        if let Some(list) = &urls {
            for u in list {
                u.parse::<iroh::RelayUrl>()
                    .map_err(|e| WoooshError::InvalidArgument(format!("relay url {u}: {e}")))?;
            }
        }
        let mut guard = self.inner.iroh.lock().await;
        *self.inner.relay_urls.lock().unwrap() = urls;
        *self.inner.ticket_pending.lock().unwrap() = None;
        if let Some(ep) = guard.take() {
            // Dropping without closing aborts iroh's socket; see `shutdown`.
            ep.close().await;
        }
        Ok(())
    }

    /// A ticket the user stopped expecting must stop working immediately.
    pub fn end_internet_ticket(&self) {
        *self.inner.ticket_pending.lock().unwrap() = None;
    }

    pub async fn redeem_ticket(&self, ticket: &str) -> Result<String, WoooshError> {
        match self.redeem_ticket_inner(ticket).await {
            Ok(peer_id) => Ok(peer_id),
            Err(f) => {
                if !f.reported {
                    self.inner.emit_pairing_failure(f.pubkey, f.err.to_string());
                }
                Err(f.err)
            }
        }
    }

    async fn redeem_ticket_inner(&self, ticket: &str) -> Result<String, PairFailure> {
        let t = NetTicket::parse(ticket).map_err(|e| PairFailure::new(e, None))?;
        let key = t.node_id;
        if t.is_expired() {
            return Err(PairFailure::new(
                WoooshError::Pairing("ticket expired".into()),
                Some(key),
            ));
        }
        let fail = |e: WoooshError| PairFailure::new(e, Some(key));
        let ep = self.inner.iroh_endpoint().await.map_err(fail)?;
        let addr = t.endpoint_addr().map_err(fail)?;
        let conn = tokio::time::timeout(NET_CONNECT_TIMEOUT, ep.connect(addr, transport::ALPN))
            .await
            .map_err(|_| fail(WoooshError::Connect("ticket connect timed out".into())))?
            .map_err(|e| fail(WoooshError::Connect(format!("iroh connect: {e}"))))?;
        let conn = Conn::Net(conn);
        // Pinning must never be implicit in someone else's library (§4.5).
        let presented = conn.peer_pubkey().map_err(fail)?;
        if presented != key {
            conn.close(close_codes::KEY_CHANGED, b"KEY_CHANGED");
            self.inner.emit(CoreEvent::KeyChanged {
                peer_id: identity::device_id_string_for(&key),
                expected_pubkey: key.to_vec(),
                presented_pubkey: Some(presented.to_vec()),
            });
            return Err(PairFailure::new(WoooshError::KeyChanged, Some(key)));
        }
        let peer = self.inner.clone().run_connection(conn, true).await.map_err(fail)?;
        self.redeem_pair_token(peer, t.token, key).await
    }

    pub async fn connect_peer(
        &self,
        addr: &str,
        expected_pubkey: Option<[u8; 32]>,
    ) -> Result<String, WoooshError> {
        let peer = self.inner.connect_to(addr, expected_pubkey, false).await?;
        Ok(peer.device_id.clone())
    }

    pub fn request_sas_pairing(&self, peer_id: &str) -> Result<(), WoooshError> {
        let peer = self.inner.get_peer(peer_id)?;
        let code = transport::derive_sas(&peer.conn)?;
        self.inner.new_sas_entry(&peer, code);
        peer.send_msg(Msg::PairRequest { token: None });
        Ok(())
    }

    pub fn confirm_sas(&self, peer_id: &str, accepted: bool) -> Result<(), WoooshError> {
        let peer = self.inner.get_peer(peer_id)?;
        if !accepted {
            self.inner.sas.lock().unwrap().remove(peer_id);
            peer.send_msg(Msg::PairReject);
            self.inner.emit(CoreEvent::PairingResult {
                peer_id: peer_id.to_string(),
                peer_pubkey: peer.pubkey.to_vec(),
                fingerprint: peer.fingerprint(),
                success: false,
                message: Some("rejected locally".into()),
            });
            return Ok(());
        }
        let finalize = {
            let mut sas = self.inner.sas.lock().unwrap();
            let entry = sas
                .get_mut(peer_id)
                .ok_or_else(|| WoooshError::Pairing("no SAS pairing in progress".into()))?;
            entry.local_confirmed = true;
            entry.remote_confirmed
        };
        peer.send_msg(Msg::PairConfirm);
        if finalize {
            self.inner.finalize_sas(&peer);
        }
        Ok(())
    }

    /// Tests only: current SAS code for a peer.
    pub fn sas_code(&self, peer_id: &str) -> Option<u32> {
        self.inner.sas.lock().unwrap().get(peer_id).map(|s| s.code)
    }

    pub async fn send(
        &self,
        peer_id: &str,
        files: Vec<PathBuf>,
    ) -> Result<String, WoooshError> {
        if files.is_empty() {
            return Err(WoooshError::InvalidArgument("no files".into()));
        }
        let peer = self.inner.get_peer(peer_id)?;
        let tid: [u8; 16] = rand::random();
        let inner = self.inner.clone();
        let peer_id_owned = peer_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = inner.clone().run_send(tid, peer, files).await {
                inner.emit(CoreEvent::TransferError {
                    transfer_id: tid_hex(&tid),
                    error: e.to_string(),
                    resumable: matches!(e, WoooshError::Connect(_) | WoooshError::Io(_)),
                });
                log::warn!("send to {peer_id_owned} failed: {e}");
            }
        });
        Ok(tid_hex(&tid))
    }

    pub async fn resume_transfer(&self, peer_id: &str, tid: [u8; 16]) -> Result<(), WoooshError> {
        let peer = self.inner.get_peer(peer_id)?;
        let st = self
            .inner
            .sends
            .lock()
            .unwrap()
            .get(&tid)
            .cloned()
            .ok_or_else(|| WoooshError::UnknownTransfer(tid_hex(&tid)))?;
        st.cancelled.store(false, Ordering::SeqCst);
        st.bytes_this_attempt.store(0, Ordering::SeqCst);
        st.done_emitted.store(false, Ordering::SeqCst);
        *st.started_at.lock().unwrap() = Instant::now();
        let inner = self.inner.clone();
        tokio::spawn(async move {
            if let Err(e) = inner.clone().run_resume(st, peer).await {
                inner.emit(CoreEvent::TransferError {
                    transfer_id: tid_hex(&tid),
                    error: e.to_string(),
                    resumable: true,
                });
            }
        });
        Ok(())
    }

    pub fn respond_to_offer(&self, tid: [u8; 16], accept: Vec<u32>) -> Result<(), WoooshError> {
        self.inner.respond_to_offer(tid, accept)
    }

    pub fn cancel(&self, tid: [u8; 16], fid: Option<u32>) -> Result<(), WoooshError> {
        let inner = &self.inner;
        let peer_id = if let Some(st) = inner.sends.lock().unwrap().get(&tid) {
            st.cancelled.store(true, Ordering::SeqCst);
            st.peer_id.clone()
        } else if let Some(rt) = inner.recvs.lock().unwrap().get(&tid) {
            rt.cancelled.store(true, Ordering::SeqCst);
            rt.peer_id.clone()
        } else {
            return Err(WoooshError::UnknownTransfer(tid_hex(&tid)));
        };
        if let Ok(peer) = inner.get_peer(&peer_id) {
            peer.send_msg(Msg::Cancel { tid, fid });
        }
        inner.emit(CoreEvent::TransferError {
            transfer_id: tid_hex(&tid),
            error: "cancelled".into(),
            resumable: false,
        });
        Ok(())
    }

    pub fn transfer_stats(&self, tid: &[u8; 16]) -> Option<TransferStats> {
        if let Some(st) = self.inner.sends.lock().unwrap().get(tid) {
            return Some(TransferStats {
                is_sender: true,
                bytes_this_attempt: st.bytes_this_attempt.load(Ordering::SeqCst),
                resumed_from: st.resumed_from.load(Ordering::SeqCst),
                total: st.total,
            });
        }
        if let Some(rt) = self.inner.recvs.lock().unwrap().get(tid) {
            return Some(TransferStats {
                is_sender: false,
                bytes_this_attempt: rt.bytes_this_attempt.load(Ordering::SeqCst),
                resumed_from: 0,
                total: rt.total,
            });
        }
        None
    }

    /// Tests only: push a raw control message to a connected peer.
    pub fn debug_send_control(&self, peer_id: &str, msg: Msg) -> Result<(), WoooshError> {
        self.inner.get_peer(peer_id)?.send_msg(msg);
        Ok(())
    }

    /// Tests only: true if the connection to `peer_id` is still open.
    pub fn peer_connected(&self, peer_id: &str) -> bool {
        self.inner
            .peers
            .lock()
            .unwrap()
            .get(peer_id)
            .map(|p| p.conn.close_reason().is_none())
            .unwrap_or(false)
    }
}

impl Inner {
    fn emit(&self, ev: CoreEvent) {
        let _ = self.events.send(ev);
    }

    /// Without a terminal event the shell's pairing sheet spins forever.
    fn emit_pairing_failure(&self, pubkey: Option<[u8; 32]>, message: String) {
        let (peer_id, peer_pubkey, fingerprint) = match pubkey {
            Some(k) => (
                identity::device_id_string_for(&k),
                k.to_vec(),
                identity::fingerprint_phrase_for(&k),
            ),
            // Payload did not parse: no peer identity to name.
            None => (String::new(), Vec::new(), String::new()),
        };
        self.emit(CoreEvent::PairingResult {
            peer_id,
            peer_pubkey,
            fingerprint,
            success: false,
            message: Some(message),
        });
    }

    fn get_peer(&self, peer_id: &str) -> Result<Arc<Peer>, WoooshError> {
        self.peers
            .lock()
            .unwrap()
            .get(peer_id)
            .cloned()
            .ok_or_else(|| WoooshError::UnknownPeer(peer_id.to_string()))
    }

    /// The async mutex prevents two endpoints bound on one key.
    async fn iroh_endpoint(self: &Arc<Self>) -> Result<iroh::Endpoint, WoooshError> {
        let mut guard = self.iroh.lock().await;
        if let Some(ep) = guard.as_ref() {
            return Ok(ep.clone());
        }
        let relays = self.relay_urls.lock().unwrap().clone();
        let ep = inet::bind_endpoint(&self.identity, relays.as_deref()).await?;
        let inner = self.clone();
        let accept_ep = ep.clone();
        tokio::spawn(async move { inner.iroh_accept_loop(accept_ep).await });
        *guard = Some(ep.clone());
        Ok(ep)
    }

    /// Nothing below this line knows which transport it is on.
    async fn iroh_accept_loop(self: Arc<Self>, ep: iroh::Endpoint) {
        while let Some(incoming) = ep.accept().await {
            let inner = self.clone();
            tokio::spawn(async move {
                let vis = inner.cfg.lock().unwrap().visibility.clone();
                if matches!(vis, Visibility::Off) {
                    incoming.ignore();
                    return;
                }
                match incoming.await {
                    Ok(conn) => {
                        if let Err(e) = inner.clone().run_connection(Conn::Net(conn), false).await {
                            log::debug!("incoming internet connection ended: {e}");
                        }
                    }
                    Err(e) => log::debug!("iroh handshake failed: {e}"),
                }
            });
        }
    }

    async fn accept_loop(self: Arc<Self>) {
        while let Some(incoming) = self.endpoint.accept().await {
            let inner = self.clone();
            tokio::spawn(async move {
                let vis = inner.cfg.lock().unwrap().visibility.clone();
                if matches!(vis, Visibility::Off) {
                    incoming.refuse();
                    return;
                }
                match incoming.await {
                    Ok(conn) => {
                        if let Err(e) = inner.clone().run_connection(Conn::Lan(conn), false).await {
                            log::debug!("incoming connection ended: {e}");
                        }
                    }
                    Err(e) => log::debug!("handshake failed: {e}"),
                }
            });
        }
    }

    async fn connect_to(
        self: &Arc<Self>,
        addr: &str,
        expected_pubkey: Option<[u8; 32]>,
        qr_context: bool,
    ) -> Result<Arc<Peer>, WoooshError> {
        let conn = self
            .connect_quic(addr, expected_pubkey, qr_context, CONNECT_TIMEOUT)
            .await?;
        self.clone().run_connection(conn, true).await
    }

    /// A dead hint is indistinguishable from a slow one until it times out, so
    /// serial dialling costs a full timeout per dead entry. Never make this
    /// serial again. Only the handshake is raced, so the single-use token is
    /// redeemed once.
    async fn race_connect_qr(
        self: &Arc<Self>,
        hints: &[String],
        expected_pubkey: [u8; 32],
    ) -> Result<Conn, WoooshError> {
        let no_hints = || WoooshError::Connect("no hints in QR payload".into());
        if hints.is_empty() {
            return Err(no_hints());
        }
        let mut set = tokio::task::JoinSet::new();
        for (idx, hint) in hints.iter().enumerate() {
            let inner = self.clone();
            let hint = hint.clone();
            set.spawn(async move {
                let r = inner
                    .connect_quic(&hint, Some(expected_pubkey), true, PAIR_CONNECT_TIMEOUT)
                    .await;
                (idx, r)
            });
        }
        let mut winner: Option<Conn> = None;
        let mut best: Option<(usize, WoooshError)> = None;
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((_, Ok(conn))) => {
                    winner = Some(conn);
                    break;
                }
                Ok((idx, Err(e))) => {
                    let better = match &best {
                        None => true,
                        Some((bi, be)) => {
                            let (r, br) = (connect_error_rank(&e), connect_error_rank(be));
                            // Tie: report the earlier, better-ranked hint.
                            r > br || (r == br && idx < *bi)
                        }
                    };
                    if better {
                        best = Some((idx, e));
                    }
                }
                Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
                Err(_) => {}
            }
        }
        // Two hints can reach the same device; hang up on late winners.
        set.abort_all();
        while let Some(joined) = set.join_next().await {
            if let Ok((_, Ok(conn))) = joined {
                conn.close(close_codes::BYE, b"bye");
            }
        }
        match winner {
            Some(conn) => Ok(conn),
            None => Err(best.map(|(_, e)| e).unwrap_or_else(no_hints)),
        }
    }

    /// Separate from `connect_to` so pairing races only the handshake.
    async fn connect_quic(
        self: &Arc<Self>,
        addr: &str,
        expected_pubkey: Option<[u8; 32]>,
        qr_context: bool,
        connect_timeout: Duration,
    ) -> Result<Conn, WoooshError> {
        let sockaddr: SocketAddr = tokio::net::lookup_host(addr)
            .await
            .map_err(|e| WoooshError::Connect(format!("resolve {addr}: {e}")))?
            .next()
            .ok_or_else(|| WoooshError::Connect(format!("no address for {addr}")))?;
        // Pinning must not depend on the shell passing the right argument
        // (§4.5): fall back to the identity last seen at this exact `ip:port`.
        let expected_pubkey = match expected_pubkey {
            Some(k) => Some(k),
            None => {
                let from_store = self.trust.pinned_key_for_addr(&sockaddr.to_string());
                if from_store.is_some() {
                    log::debug!("pinning {sockaddr} from the trust store (no key supplied)");
                }
                from_store
            }
        };
        let (cfg, seen_key) = transport::client_config(&self.identity, expected_pubkey)?;
        let connecting = self
            .endpoint
            .connect_with(cfg, sockaddr, "wooosh")
            .map_err(|e| WoooshError::Connect(e.to_string()))?;
        let conn = match tokio::time::timeout(connect_timeout, connecting)
            .await
            .map_err(|_| WoooshError::Connect(format!("connect timeout to {addr}")))?
        {
            Ok(c) => c,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("WOOOSH_KEY_MISMATCH") {
                    if qr_context {
                        return Err(WoooshError::QrKeyMismatch);
                    }
                    let presented = *seen_key.lock().unwrap_or_else(|e| e.into_inner());
                    self.emit(CoreEvent::KeyChanged {
                        peer_id: expected_pubkey
                            .map(|k| identity::device_id_string_for(&k))
                            .unwrap_or_default(),
                        expected_pubkey: expected_pubkey.map(|k| k.to_vec()).unwrap_or_default(),
                        presented_pubkey: presented.map(|k| k.to_vec()),
                    });
                    return Err(WoooshError::KeyChanged);
                }
                return Err(app_close_error(&ConnErr::from(e)).unwrap_or(WoooshError::Connect(msg)));
            }
        };
        Ok(Conn::Lan(conn))
    }

    async fn run_connection(
        self: Arc<Self>,
        conn: Conn,
        is_client: bool,
    ) -> Result<Arc<Peer>, WoooshError> {
        let pubkey = conn.peer_pubkey()?;
        let device_id = identity::device_id_string_for(&pubkey);
        let trusted = self.trust.contains(&pubkey);

        let (mut send, mut recv) = if is_client {
            conn.open_bi().await.map_err(|e| WoooshError::Connect(e.to_string()))?
        } else {
            tokio::time::timeout(HELLO_TIMEOUT, conn.accept_bi())
                .await
                .map_err(|_| WoooshError::Connect("timeout waiting for control stream".into()))?
                .map_err(|e| WoooshError::Connect(e.to_string()))?
        };

        let hello = {
            let cfg = self.cfg.lock().unwrap();
            Msg::Hello {
                v: PROTOCOL_VERSION,
                device_id: self.identity.device_id().to_vec(),
                dn: cfg.device_name.clone(),
                dt: cfg.device_type.clone(),
                caps: vec![],
            }
        };
        // HELLO (PROTOCOL.md §4.1): answering before deciding to keep the
        // connection leaks the display name and races the CONNECTION_CLOSE.
        if is_client {
            write_frame(&mut send, &hello).await?;
        }
        let peer_hello = match tokio::time::timeout(HELLO_TIMEOUT, read_frame(&mut recv)).await {
            Err(_) => return Err(WoooshError::Connect("timeout waiting for HELLO".into())),
            Ok(Err(e)) => return Err(close_reason_error(&conn).unwrap_or(e)),
            Ok(Ok(None)) => {
                return Err(close_reason_error(&conn).unwrap_or_else(|| {
                    WoooshError::Protocol("stream closed before HELLO".into())
                }))
            }
            Ok(Ok(Some(m))) => m,
        };
        let (peer_dn, peer_dt, peer_v, claimed_device_id) = match &peer_hello {
            Msg::Hello { v, dn, dt, device_id, .. } => {
                (dn.clone(), dt.clone(), *v, device_id.clone())
            }
            _ => {
                conn.close(close_codes::VERSION_MISMATCH, b"expected HELLO");
                return Err(WoooshError::Protocol("first message was not HELLO".into()));
            }
        };
        if peer_v < 1 {
            conn.close(close_codes::VERSION_MISMATCH, b"no common version");
            return Err(WoooshError::VersionMismatch);
        }
        // Identity binding (§4.1.1): the HELLO DeviceID must derive from the
        // proven key; claiming a pinned identity is KEY_CHANGED (§4.5).
        if !claimed_device_id.is_empty()
            && claimed_device_id.as_slice() != identity::device_id_for(&pubkey).as_slice()
        {
            let claimed = <[u8; 16]>::try_from(claimed_device_id.as_slice())
                .map(|id| identity::render_device_id(&id))
                .unwrap_or_default();
            conn.close(close_codes::KEY_CHANGED, b"KEY_CHANGED");
            if let Some(pinned) = self.trust.pinned_key_for_device_id(&claimed) {
                self.emit(CoreEvent::KeyChanged {
                    peer_id: claimed,
                    expected_pubkey: pinned.to_vec(),
                    presented_pubkey: Some(pubkey.to_vec()),
                });
                return Err(WoooshError::KeyChanged);
            }
            return Err(WoooshError::Protocol(
                "HELLO device_id does not match the presented certificate key".into(),
            ));
        }

        if !is_client && !trusted {
            let vis = self.cfg.lock().unwrap().visibility.clone();
            // A live ticket is an invitation; the token needs a chance (§4.2).
            let invited =
                conn.is_internet() && self.ticket_pending.lock().unwrap().is_some();
            if matches!(vis, Visibility::PairedOnly) && !invited {
                conn.close(close_codes::PAIRING_REQUIRED, b"PAIRING_REQUIRED");
                return Err(WoooshError::PairingRequired);
            }
        }
        if !is_client {
            write_frame(&mut send, &hello).await?;
        }

        if trusted {
            // Lets a later connect_peer(addr, None) re-apply the pin (§4.5).
            // Skipped for internet connections: a relayed address would
            // mis-pin a LAN dial.
            if let Some(addr) = conn.remote_address() {
                self.trust.note_addr(&pubkey, &addr.to_string());
            }
        }

        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let peer = Arc::new(Peer {
            conn: conn.clone(),
            pubkey,
            device_id: device_id.clone(),
            trusted: AtomicBool::new(trusted),
            ticket_authorized: AtomicBool::new(false),
            dn: Mutex::new(peer_dn.clone()),
            dt: Mutex::new(peer_dt.clone()),
            out_tx,
        });
        self.peers.lock().unwrap().insert(device_id.clone(), peer.clone());
        self.emit(CoreEvent::PeerConnected {
            peer_id: device_id.clone(),
            peer_pubkey: pubkey.to_vec(),
            device_name: peer_dn,
            device_type: DeviceType::from_wire(&peer_dt),
            fingerprint: identity::fingerprint_phrase_for(&pubkey),
            trusted,
        });

        tokio::spawn(write_loop(send, out_rx));
        {
            let inner = self.clone();
            let peer = peer.clone();
            tokio::spawn(async move {
                if let Err(e) = inner.clone().control_read_loop(peer.clone(), recv).await {
                    log::debug!("control loop for {} ended: {e}", peer.device_id);
                }
            });
        }
        {
            let inner = self.clone();
            let peer = peer.clone();
            tokio::spawn(async move { inner.uni_accept_loop(peer).await });
        }
        {
            let inner = self.clone();
            let peer = peer.clone();
            tokio::spawn(async move {
                let reason = peer.conn.closed().await;
                let mut peers = inner.peers.lock().unwrap();
                if let Some(cur) = peers.get(&peer.device_id) {
                    if Arc::ptr_eq(cur, &peer) {
                        peers.remove(&peer.device_id);
                    }
                }
                drop(peers);
                inner.emit(CoreEvent::PeerDisconnected { peer_id: peer.device_id.clone() });
                log::debug!("peer {} disconnected: {reason}", peer.device_id);
            });
        }
        Ok(peer)
    }

    async fn control_read_loop(
        self: Arc<Self>,
        peer: Arc<Peer>,
        mut recv: RecvStream,
    ) -> Result<(), WoooshError> {
        loop {
            let Some(msg) = read_frame(&mut recv).await? else {
                return Ok(());
            };
            self.dispatch(&peer, msg).await?;
        }
    }

    fn untrusted_violation(&self, peer: &Peer, what: &str) {
        log::warn!("untrusted peer {} sent {what}; closing", peer.device_id);
        peer.conn
            .close(close_codes::UNTRUSTED_MSG, b"message not allowed on untrusted channel");
    }

    async fn dispatch(self: &Arc<Self>, peer: &Arc<Peer>, msg: Msg) -> Result<(), WoooshError> {
        let trusted = peer.trusted.load(Ordering::SeqCst);
        match msg {
            Msg::Hello { .. } => {} // late HELLO: ignore
            Msg::Bye => peer.conn.close(close_codes::BYE, b"bye"),
            Msg::Unknown { t } => peer.send_msg(Msg::ErrUnsupported { t }),
            Msg::ErrUnsupported { t } => {
                log::warn!("peer {} does not support message t={t}", peer.device_id)
            }

            Msg::PairRequest { token: Some(token) } => {
                // A token only redeems on the transport it was issued for.
                let ok = if peer.conn.is_internet() {
                    let mut pending = self.ticket_pending.lock().unwrap();
                    let ok = pending.as_mut().map(|p| p.redeem(&token)).unwrap_or(false);
                    if ok {
                        *pending = None;
                    }
                    ok
                } else {
                    let mut pending = self.qr_pending.lock().unwrap();
                    let ok = pending.as_mut().map(|p| p.redeem(&token)).unwrap_or(false);
                    if ok {
                        *pending = None;
                    }
                    ok
                };
                if ok {
                    if peer.conn.is_internet() {
                        // §9.4: one session, nothing written to the trust store.
                        peer.ticket_authorized.store(true, Ordering::SeqCst);
                        peer.send_msg(Msg::PairAccept);
                        self.emit(CoreEvent::TicketRedeemed {
                            peer_id: peer.device_id.clone(),
                            peer_pubkey: peer.pubkey.to_vec(),
                            device_name: peer.dn.lock().unwrap().clone(),
                        });
                    } else {
                        let dn = self.pin_peer(peer)?;
                        peer.send_msg(Msg::PairAccept);
                        self.emit(CoreEvent::PairingResult {
                            peer_id: peer.device_id.clone(),
                            peer_pubkey: peer.pubkey.to_vec(),
                            fingerprint: peer.fingerprint(),
                            success: true,
                            message: Some(dn),
                        });
                    }
                } else {
                    peer.send_msg(Msg::PairReject);
                    peer.conn.close(close_codes::TOKEN_INVALID, b"TOKEN_INVALID");
                }
            }
            Msg::PairRequest { token: None } => {
                let code = transport::derive_sas(&peer.conn)?;
                self.new_sas_entry(peer, code);
            }
            Msg::PairAccept if peer.conn.is_internet() => {
                // Symmetric with the publisher: one-shot, no trust written.
                peer.ticket_authorized.store(true, Ordering::SeqCst);
                if let Some(tx) = self.qr_waiters.lock().unwrap().remove(&peer.device_id) {
                    let _ = tx.send(true);
                }
                self.emit(CoreEvent::TicketRedeemed {
                    peer_id: peer.device_id.clone(),
                    peer_pubkey: peer.pubkey.to_vec(),
                    device_name: peer.dn.lock().unwrap().clone(),
                });
            }
            Msg::PairAccept => {
                let dn = self.pin_peer(peer)?;
                if let Some(tx) = self.qr_waiters.lock().unwrap().remove(&peer.device_id) {
                    let _ = tx.send(true);
                }
                self.emit(CoreEvent::PairingResult {
                    peer_id: peer.device_id.clone(),
                    peer_pubkey: peer.pubkey.to_vec(),
                    fingerprint: peer.fingerprint(),
                    success: true,
                    message: Some(dn),
                });
            }
            Msg::PairConfirm => {
                let finalize = {
                    let mut sas = self.sas.lock().unwrap();
                    match sas.get_mut(&peer.device_id) {
                        Some(e) => {
                            e.remote_confirmed = true;
                            e.local_confirmed
                        }
                        None => false,
                    }
                };
                if finalize {
                    self.finalize_sas(peer);
                }
            }
            Msg::PairReject => {
                self.sas.lock().unwrap().remove(&peer.device_id);
                if let Some(tx) = self.qr_waiters.lock().unwrap().remove(&peer.device_id) {
                    let _ = tx.send(false);
                }
                self.emit(CoreEvent::PairingResult {
                    peer_id: peer.device_id.clone(),
                    peer_pubkey: peer.pubkey.to_vec(),
                    fingerprint: peer.fingerprint(),
                    success: false,
                    message: Some("rejected by peer".into()),
                });
            }

            Msg::Offer { tid, files, total, note } => {
                let vis = self.cfg.lock().unwrap().visibility.clone();
                // §9.4: `trusted` is false even for a legitimate transfer.
                let invited = peer.ticket_authorized.load(Ordering::SeqCst);
                if !trusted && !invited && !matches!(vis, Visibility::Everyone) {
                    peer.conn.close(close_codes::PAIRING_REQUIRED, b"PAIRING_REQUIRED");
                    return Ok(());
                }
                self.handle_offer(peer, tid, files, total, note)?;
            }
            Msg::Decision { tid, accept } => {
                let st = self.sends.lock().unwrap().get(&tid).cloned();
                match st {
                    // Honored even untrusted (accept-once, PROTOCOL.md §4.4).
                    Some(st) if st.peer_id == peer.device_id => {
                        *st.accepted.lock().unwrap() = Some(accept);
                        st.decision_notify.notify_waiters();
                    }
                    _ => {
                        if !trusted {
                            self.untrusted_violation(peer, "DECISION");
                        }
                    }
                }
            }
            Msg::ResumeQ { tid } => {
                if !trusted {
                    // Untrusted channels carry only HELLO / PAIR_* / OFFER
                    // (§4.1); accept-once cannot resume.
                    self.untrusted_violation(peer, "RESUME_Q");
                    return Ok(());
                }
                self.clone().handle_resume_q(peer, tid).await?;
            }
            Msg::ResumeA { tid, have } => {
                let st = self.sends.lock().unwrap().get(&tid).cloned();
                match st {
                    Some(st) if st.peer_id == peer.device_id => {
                        let mut offsets = st.offsets.lock().unwrap();
                        let mut resumed = 0u64;
                        for h in have {
                            resumed += h.verified_off;
                            offsets.insert(h.fid, h.verified_off);
                        }
                        drop(offsets);
                        st.resumed_from.store(resumed, Ordering::SeqCst);
                        st.resume_notify.notify_waiters();
                    }
                    _ => {
                        if !trusted {
                            self.untrusted_violation(peer, "RESUME_A");
                        }
                    }
                }
            }
            Msg::Done { tid, fid, ok, err } => {
                let st = self.sends.lock().unwrap().get(&tid).cloned();
                match st {
                    Some(st) if st.peer_id == peer.device_id => {
                        let (all_done, ok_count, fail_count) = {
                            let mut done = st.done.lock().unwrap();
                            done.insert(fid, (ok, err));
                            let accepted = st.accepted.lock().unwrap();
                            let acc = accepted.as_ref().cloned().unwrap_or_default();
                            let all = acc.iter().all(|f| done.contains_key(f));
                            let okc = done.values().filter(|(o, _)| *o).count() as u32;
                            let fc = done.values().filter(|(o, _)| !*o).count() as u32;
                            (all, okc, fc)
                        };
                        if all_done && !st.done_emitted.swap(true, Ordering::SeqCst) {
                            self.emit(CoreEvent::TransferDone {
                                transfer_id: tid_hex(&tid),
                                ok_files: ok_count,
                                failed_files: fail_count,
                                bytes_transferred: st.bytes_this_attempt.load(Ordering::SeqCst),
                                duration_ms: elapsed_ms(&st.started_at),
                            });
                        }
                    }
                    _ => {
                        if !trusted {
                            self.untrusted_violation(peer, "DONE");
                        }
                    }
                }
            }
            Msg::Cancel { tid, fid: _ } => {
                let known_send = self
                    .sends
                    .lock()
                    .unwrap()
                    .get(&tid)
                    .map(|st| {
                        if st.peer_id == peer.device_id {
                            st.cancelled.store(true, Ordering::SeqCst);
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                let known_recv = self
                    .recvs
                    .lock()
                    .unwrap()
                    .get(&tid)
                    .map(|rt| {
                        if rt.peer_id == peer.device_id {
                            rt.cancelled.store(true, Ordering::SeqCst);
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if known_send || known_recv {
                    self.emit(CoreEvent::TransferError {
                        transfer_id: tid_hex(&tid),
                        error: "cancelled by peer".into(),
                        resumable: false,
                    });
                } else if !trusted {
                    self.untrusted_violation(peer, "CANCEL");
                }
            }
        }
        Ok(())
    }

    /// Persist the pin for a newly paired peer; returns its display name.
    fn pin_peer(&self, peer: &Arc<Peer>) -> Result<String, WoooshError> {
        let dn = peer.dn.lock().unwrap().clone();
        let dt = peer.dt.lock().unwrap().clone();
        // §4.5 `last_addr` is a LAN-only hint; see the note in run_connection.
        let addr = peer.conn.remote_address().map(|a| a.to_string());
        self.trust.insert(
            &peer.pubkey,
            &dn,
            if dt.is_empty() { None } else { Some(dt.as_str()) },
            addr.as_deref(),
        )?;
        peer.trusted.store(true, Ordering::SeqCst);
        Ok(dn)
    }

    fn new_sas_entry(self: &Arc<Self>, peer: &Arc<Peer>, code: u32) {
        self.sas.lock().unwrap().insert(
            peer.device_id.clone(),
            SasState {
                code,
                local_confirmed: false,
                remote_confirmed: false,
                created: Instant::now(),
            },
        );
        self.emit(CoreEvent::PairingSas {
            peer_id: peer.device_id.clone(),
            code: format!("{code:06}"),
        });
        // SAS confirmation expires after 60 s (PROTOCOL.md §4.3).
        let inner = self.clone();
        let peer_id = peer.device_id.clone();
        let peer_pubkey = peer.pubkey;
        tokio::spawn(async move {
            tokio::time::sleep(pairing::SAS_TIMEOUT).await;
            let expired = {
                let mut sas = inner.sas.lock().unwrap();
                match sas.get(&peer_id) {
                    Some(e) if e.created.elapsed() >= pairing::SAS_TIMEOUT => {
                        sas.remove(&peer_id);
                        true
                    }
                    _ => false,
                }
            };
            if expired {
                inner.emit(CoreEvent::PairingResult {
                    peer_id,
                    peer_pubkey: peer_pubkey.to_vec(),
                    fingerprint: identity::fingerprint_phrase_for(&peer_pubkey),
                    success: false,
                    message: Some("SAS confirmation timed out".into()),
                });
            }
        });
    }

    fn finalize_sas(&self, peer: &Arc<Peer>) {
        self.sas.lock().unwrap().remove(&peer.device_id);
        let dn = match self.pin_peer(peer) {
            Ok(dn) => dn,
            Err(e) => {
                log::error!("trust store write failed: {e}");
                return;
            }
        };
        self.emit(CoreEvent::PairingResult {
            peer_id: peer.device_id.clone(),
            peer_pubkey: peer.pubkey.to_vec(),
            fingerprint: peer.fingerprint(),
            success: true,
            message: Some(dn),
        });
    }

    fn handle_offer(
        self: &Arc<Self>,
        peer: &Arc<Peer>,
        tid: [u8; 16],
        files: Vec<FileMeta>,
        total: u64,
        _note: Option<String>,
    ) -> Result<(), WoooshError> {
        // The cap is enforced on both ends (DESIGN.md §9.1): the relay spent
        // is usually the *receiver's*. Declined outright, never partially.
        if !peer.conn.is_direct() && files.iter().any(|f| f.size > RELAY_MAX_FILE_BYTES) {
            log::warn!(
                "declining relayed offer from {}: a file exceeds {} bytes",
                peer.device_id,
                RELAY_MAX_FILE_BYTES
            );
            peer.send_msg(Msg::Decision { tid, accept: Vec::new() });
            return Ok(());
        }
        let dir = self.staging.join(tid_hex(&tid));
        let mut states = BTreeMap::new();
        let mut invalid = Vec::new();
        let mut offered = Vec::new();
        for f in files {
            // Receiver-side sanitization (PROTOCOL.md §5).
            let safe_name = match sanitize::sanitize_name(&f.name) {
                Ok(n) => n,
                Err(e) => {
                    invalid.push((f.fid, format!("invalid name: {e}")));
                    continue;
                }
            };
            let safe_rel = match &f.rel_path {
                Some(rp) => match sanitize::sanitize_rel_path(rp) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        invalid.push((f.fid, format!("invalid rel_path: {e}")));
                        continue;
                    }
                },
                None => None,
            };
            offered.push(OfferedFile {
                fid: f.fid,
                name: safe_name.clone(),
                rel_path: safe_rel.as_ref().map(|p| p.to_string_lossy().to_string()),
                size: f.size,
                mime: f.mime.clone(),
            });
            states.insert(
                f.fid,
                RecvFileState {
                    meta: f,
                    safe_name,
                    safe_rel,
                    verified_off: 0,
                    done: false,
                    ok: None,
                    hasher: None,
                    active: false,
                },
            );
        }
        let rt = Arc::new(RecvTransfer {
            tid,
            peer_id: peer.device_id.clone(),
            peer_pubkey: peer.pubkey,
            dir,
            ledger: Mutex::new(None),
            files: Mutex::new(states),
            invalid: Mutex::new(invalid),
            accepted: Mutex::new(None),
            cancelled: AtomicBool::new(false),
            bytes_this_attempt: AtomicU64::new(0),
            started_at: Mutex::new(Instant::now()),
            total,
        });
        self.recvs.lock().unwrap().insert(tid, rt);
        let dn = peer.dn.lock().unwrap().clone();
        self.emit(CoreEvent::IncomingOffer {
            transfer_id: tid_hex(&tid),
            peer_id: peer.device_id.clone(),
            peer_pubkey: peer.pubkey.to_vec(),
            from_name: dn,
            device_type: peer.device_type(),
            trusted: peer.trusted.load(Ordering::SeqCst),
            fingerprint: peer.fingerprint(),
            files: offered,
            total_bytes: total,
        });
        Ok(())
    }

    fn respond_to_offer(&self, tid: [u8; 16], accept: Vec<u32>) -> Result<(), WoooshError> {
        let rt = self
            .recvs
            .lock()
            .unwrap()
            .get(&tid)
            .cloned()
            .ok_or_else(|| WoooshError::UnknownTransfer(tid_hex(&tid)))?;
        let peer = self.get_peer(&rt.peer_id)?;
        let valid: Vec<u32> = {
            let files = rt.files.lock().unwrap();
            accept.iter().copied().filter(|f| files.contains_key(f)).collect()
        };
        if !valid.is_empty() {
            std::fs::create_dir_all(&rt.dir)?;
            let mut ledger = Ledger {
                tid_hex: tid_hex(&tid),
                sender_pubkey_b64: {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD.encode(rt.peer_pubkey)
                },
                files: BTreeMap::new(),
            };
            let files = rt.files.lock().unwrap();
            for fid in &valid {
                let st = &files[fid];
                let part = rt.dir.join(format!("{fid}.part"));
                let f = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(false)
                    .open(&part)?;
                f.set_len(st.meta.size)?; // preallocation (PROTOCOL.md §6)
                ledger.files.insert(
                    *fid,
                    LedgerFile {
                        fid: *fid,
                        name: st.safe_name.clone(),
                        rel_path: st
                            .safe_rel
                            .as_ref()
                            .map(|p| p.to_string_lossy().to_string()),
                        size: st.meta.size,
                        b3_hex: hex::encode(st.meta.b3),
                        mime: st.meta.mime.clone(),
                        verified_off: 0,
                        done: false,
                    },
                );
            }
            drop(files);
            ledger.save(&rt.dir)?;
            *rt.ledger.lock().unwrap() = Some(ledger);
        }
        *rt.accepted.lock().unwrap() = Some(valid.iter().copied().collect());
        if !valid.is_empty() {
            let (files, total): (Vec<OfferedFile>, u64) = {
                let states = rt.files.lock().unwrap();
                let files: Vec<OfferedFile> = valid
                    .iter()
                    .filter_map(|fid| states.get(fid))
                    .map(|s| OfferedFile {
                        fid: s.meta.fid,
                        name: s.safe_name.clone(),
                        rel_path: s.safe_rel.as_ref().map(|p| p.to_string_lossy().to_string()),
                        size: s.meta.size,
                        mime: s.meta.mime.clone(),
                    })
                    .collect();
                let total = files.iter().map(|f| f.size).sum();
                (files, total)
            };
            self.emit(CoreEvent::TransferStarted {
                transfer_id: tid_hex(&tid),
                peer_id: rt.peer_id.clone(),
                direction: TransferDirection::Receive,
                files,
                total_bytes: total,
            });
        }
        peer.send_msg(Msg::Decision { tid, accept: valid });
        for (fid, err) in rt.invalid.lock().unwrap().drain(..) {
            peer.send_msg(Msg::Done { tid, fid, ok: false, err: Some(err) });
        }
        Ok(())
    }

    async fn handle_resume_q(
        self: Arc<Self>,
        peer: &Arc<Peer>,
        tid: [u8; 16],
    ) -> Result<(), WoooshError> {
        // Not in memory means the receiver restarted: reload from the ledger.
        let rt = {
            let existing = self.recvs.lock().unwrap().get(&tid).cloned();
            match existing {
                Some(rt) => {
                    if rt.peer_pubkey != peer.pubkey {
                        self.untrusted_violation(peer, "RESUME_Q for another sender's transfer");
                        return Ok(());
                    }
                    rt
                }
                None => {
                    let dir = self.staging.join(tid_hex(&tid));
                    let Some(ledger) = Ledger::load(&dir)? else {
                        peer.send_msg(Msg::ResumeA { tid, have: vec![] });
                        return Ok(());
                    };
                    use base64::Engine as _;
                    let expect = base64::engine::general_purpose::STANDARD.encode(peer.pubkey);
                    if ledger.sender_pubkey_b64 != expect {
                        self.untrusted_violation(peer, "RESUME_Q for another sender's transfer");
                        return Ok(());
                    }
                    let mut states = BTreeMap::new();
                    let mut accepted = HashSet::new();
                    let mut total = 0u64;
                    for (fid, lf) in &ledger.files {
                        accepted.insert(*fid);
                        total += lf.size;
                        let b3: [u8; 32] = hex::decode(&lf.b3_hex)
                            .ok()
                            .and_then(|v| v.try_into().ok())
                            .ok_or_else(|| WoooshError::Io("corrupt ledger b3".into()))?;
                        states.insert(
                            *fid,
                            RecvFileState {
                                meta: FileMeta {
                                    fid: *fid,
                                    name: lf.name.clone(),
                                    rel_path: lf.rel_path.clone(),
                                    size: lf.size,
                                    mime: lf.mime.clone(),
                                    b3,
                                    mtime: 0,
                                },
                                safe_name: lf.name.clone(),
                                safe_rel: lf.rel_path.as_ref().map(PathBuf::from),
                                verified_off: lf.verified_off,
                                done: lf.done,
                                ok: if lf.done { Some(true) } else { None },
                                hasher: None,
                                active: false,
                            },
                        );
                    }
                    let rt = Arc::new(RecvTransfer {
                        tid,
                        peer_id: peer.device_id.clone(),
                        peer_pubkey: peer.pubkey,
                        dir,
                        ledger: Mutex::new(Some(ledger)),
                        files: Mutex::new(states),
                        invalid: Mutex::new(Vec::new()),
                        accepted: Mutex::new(Some(accepted)),
                        cancelled: AtomicBool::new(false),
                        bytes_this_attempt: AtomicU64::new(0),
                        started_at: Mutex::new(Instant::now()),
                        total,
                    });
                    self.recvs.lock().unwrap().insert(tid, rt.clone());
                    rt
                }
            }
        };

        // Bytes on disk outrank the ledger's verified_off (DESIGN.md §6/§7).
        let mut have = Vec::new();
        let fids: Vec<u32> = rt.files.lock().unwrap().keys().copied().collect();
        for fid in fids {
            let (size, verified_off, done) = {
                let files = rt.files.lock().unwrap();
                let st = &files[&fid];
                (st.meta.size, st.verified_off, st.done)
            };
            if done {
                have.push(ResumeHave { fid, verified_off: size });
                continue;
            }
            let part = rt.dir.join(format!("{fid}.part"));
            let hasher = if verified_off > 0 && part.exists() {
                let p = part.clone();
                match tokio::task::spawn_blocking(move || rehash_prefix(&p, verified_off)).await {
                    Ok(Ok(h)) => Some(h),
                    _ => {
                        // Prefix unusable: restart this file from zero.
                        rt.files.lock().unwrap().get_mut(&fid).unwrap().verified_off = 0;
                        Some(blake3::Hasher::new())
                    }
                }
            } else {
                Some(blake3::Hasher::new())
            };
            let off = {
                let mut files = rt.files.lock().unwrap();
                let st = files.get_mut(&fid).unwrap();
                st.hasher = hasher;
                st.verified_off
            };
            have.push(ResumeHave { fid, verified_off: off });
        }
        {
            let states = rt.files.lock().unwrap();
            let files: Vec<OfferedFile> = states
                .values()
                .filter(|s| !s.done)
                .map(|s| OfferedFile {
                    fid: s.meta.fid,
                    name: s.safe_name.clone(),
                    rel_path: s.safe_rel.as_ref().map(|p| p.to_string_lossy().to_string()),
                    size: s.meta.size,
                    mime: s.meta.mime.clone(),
                })
                .collect();
            if !files.is_empty() {
                let total = files.iter().map(|f| f.size).sum();
                self.emit(CoreEvent::TransferStarted {
                    transfer_id: tid_hex(&tid),
                    peer_id: rt.peer_id.clone(),
                    direction: TransferDirection::Receive,
                    files,
                    total_bytes: total,
                });
            }
        }
        peer.send_msg(Msg::ResumeA { tid, have });
        Ok(())
    }

    async fn uni_accept_loop(self: Arc<Self>, peer: Arc<Peer>) {
        loop {
            match peer.conn.accept_uni().await {
                Ok(stream) => {
                    let inner = self.clone();
                    let peer = peer.clone();
                    tokio::spawn(async move {
                        if let Err(e) = inner.handle_file_stream(peer, stream).await {
                            log::debug!("file stream ended with error: {e}");
                        }
                    });
                }
                Err(_) => return, // connection closed
            }
        }
    }

    /// One uni stream = one big file, or small files back-to-back (§6).
    async fn handle_file_stream(
        self: &Arc<Self>,
        peer: Arc<Peer>,
        mut stream: RecvStream,
    ) -> Result<(), WoooshError> {
        loop {
            let Some(hlen) = read_u32_or_eof(&mut stream).await? else {
                return Ok(()); // clean FIN at a file boundary
            };
            if hlen == 0 || hlen > 4096 {
                return Err(WoooshError::Protocol(format!("bad stream header len {hlen}")));
            }
            let mut hbuf = vec![0u8; hlen as usize];
            stream
                .read_exact(&mut hbuf)
                .await
                .map_err(|e| WoooshError::Protocol(format!("stream header: {e}")))?;
            let header = StreamHeader::decode(&hbuf).map_err(WoooshError::Protocol)?;

            let rt = self.recvs.lock().unwrap().get(&header.tid).cloned();
            let Some(rt) = rt else {
                stream.stop(close_codes::UNTRUSTED_MSG);
                return Err(WoooshError::Protocol("stream for unknown transfer".into()));
            };
            if rt.peer_pubkey != peer.pubkey {
                stream.stop(close_codes::UNTRUSTED_MSG);
                return Err(WoooshError::Protocol("stream from wrong sender".into()));
            }
            let accepted = {
                let acc = rt.accepted.lock().unwrap();
                acc.as_ref().map(|a| a.contains(&header.fid)).unwrap_or(false)
            };
            if !accepted {
                // No file streams before DECISION (PROTOCOL.md §5).
                stream.stop(close_codes::UNTRUSTED_MSG);
                return Err(WoooshError::Protocol("stream for unaccepted file".into()));
            }
            self.receive_one_file(&peer, &rt, &header, &mut stream).await?;
            if rt.cancelled.load(Ordering::SeqCst) {
                return Ok(());
            }
        }
    }

    async fn receive_one_file(
        self: &Arc<Self>,
        peer: &Arc<Peer>,
        rt: &Arc<RecvTransfer>,
        header: &StreamHeader,
        stream: &mut RecvStream,
    ) -> Result<(), WoooshError> {
        let fid = header.fid;
        let (size, expected_b3, mut hasher, verified_off, mime) = {
            let mut files = rt.files.lock().unwrap();
            let st = files
                .get_mut(&fid)
                .ok_or_else(|| WoooshError::Protocol(format!("unknown fid {fid}")))?;
            if st.done || st.active {
                return Err(WoooshError::Protocol(format!("fid {fid} already handled")));
            }
            if header.off != st.verified_off {
                return Err(WoooshError::Protocol(format!(
                    "offset mismatch for fid {fid}: sender {} vs ledger {}",
                    header.off, st.verified_off
                )));
            }
            st.active = true;
            let hasher = st.hasher.take().unwrap_or_default();
            (st.meta.size, st.meta.b3, hasher, st.verified_off, st.meta.mime.clone())
        };

        let part = rt.dir.join(format!("{fid}.part"));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&part)
            .await?;
        file.seek(std::io::SeekFrom::Start(verified_off)).await?;

        let mut remaining = size - verified_off;
        let mut written = verified_off;
        let mut since_sync: u64 = 0;
        let mut since_progress: u64 = 0;
        let mut buf = vec![0u8; CHUNK];
        let result: Result<(), WoooshError> = loop {
            if remaining == 0 {
                break Ok(());
            }
            if rt.cancelled.load(Ordering::SeqCst) {
                break Err(WoooshError::Transfer("cancelled".into()));
            }
            let n = std::cmp::min(remaining, CHUNK as u64) as usize;
            if let Err(e) = stream.read_exact(&mut buf[..n]).await {
                break Err(WoooshError::Transfer(format!("stream read: {e}")));
            }
            hasher.update(&buf[..n]);
            if let Err(e) = file.write_all(&buf[..n]).await {
                break Err(WoooshError::Io(e.to_string()));
            }
            remaining -= n as u64;
            written += n as u64;
            since_sync += n as u64;
            since_progress += n as u64;
            rt.bytes_this_attempt.fetch_add(n as u64, Ordering::SeqCst);

            // Ledger fsync every 16 MiB (§6). Failures must `break Err`: `?`
            // skips cleanup and leaves the file pinned `active`.
            if since_sync >= FSYNC_INTERVAL {
                since_sync = 0;
                if let Err(e) = file.flush().await {
                    break Err(WoooshError::Io(e.to_string()));
                }
                if let Err(e) = file.sync_data().await {
                    break Err(WoooshError::Io(e.to_string()));
                }
                self.persist_verified_off(rt, fid, written);
            }
            if since_progress >= PROGRESS_INTERVAL {
                since_progress = 0;
                self.emit_progress(rt, fid, written, size);
            }
        };

        match result {
            Ok(()) => {
                file.flush().await?;
                file.sync_data().await?;
                drop(file);
                let actual = hasher.finalize();
                let ok = actual.as_bytes() == &expected_b3;
                if ok {
                    // Nothing leaves staging until the hash matches.
                    let final_path = self.finalize_file(rt, fid).await?;
                    self.persist_file_done(rt, fid, size);
                    {
                        let mut files = rt.files.lock().unwrap();
                        let st = files.get_mut(&fid).unwrap();
                        st.done = true;
                        st.ok = Some(true);
                        st.verified_off = size;
                        st.active = false;
                    }
                    self.emit_progress(rt, fid, size, size);
                    self.emit(CoreEvent::FileReady {
                        transfer_id: tid_hex(&rt.tid),
                        file_id: fid,
                        staged_path: final_path.to_string_lossy().to_string(),
                        kind: kind_for_mime(&mime),
                    });
                    peer.send_msg(Msg::Done { tid: rt.tid, fid, ok: true, err: None });
                } else {
                    {
                        let mut files = rt.files.lock().unwrap();
                        let st = files.get_mut(&fid).unwrap();
                        st.done = true;
                        st.ok = Some(false);
                        st.active = false;
                    }
                    peer.send_msg(Msg::Done {
                        tid: rt.tid,
                        fid,
                        ok: false,
                        err: Some("HASH_MISMATCH".into()),
                    });
                }
                self.maybe_recv_complete(rt);
                Ok(())
            }
            Err(e) => {
                // Persist what we have so a resume can pick up from here.
                let _ = file.flush().await;
                let _ = file.sync_data().await;
                drop(file);
                let safe_off = written; // flushed + synced above
                self.persist_verified_off(rt, fid, safe_off);
                {
                    let mut files = rt.files.lock().unwrap();
                    if let Some(st) = files.get_mut(&fid) {
                        st.verified_off = safe_off;
                        st.hasher = None; // re-hash the prefix on resume
                        st.active = false;
                    }
                }
                Err(e)
            }
        }
    }

    fn emit_progress(&self, rt: &RecvTransfer, fid: u32, bytes: u64, total: u64) {
        let elapsed = rt.started_at.lock().unwrap().elapsed().as_secs_f64();
        let attempt = rt.bytes_this_attempt.load(Ordering::SeqCst);
        let rate = if elapsed > 0.0 { attempt as f64 / elapsed } else { 0.0 };
        let eta = if rate > 0.0 { ((total - bytes) as f64 / rate) as u64 } else { 0 };
        self.emit(CoreEvent::Progress {
            transfer_id: tid_hex(&rt.tid),
            file_id: fid,
            bytes_done: bytes,
            total_bytes: total,
            rate_bps: rate as u64,
            eta_secs: eta,
        });
    }

    /// Best-effort: the ledger only accelerates a resume, and propagating a
    /// write failure would strand every file queued on a pipelined slot.
    fn write_ledger(&self, rt: &RecvTransfer, fid: u32, f: impl FnOnce(&mut LedgerFile)) {
        let mut guard = rt.ledger.lock().unwrap_or_else(|e| e.into_inner());
        let Some(ledger) = guard.as_mut() else { return };
        let Some(lf) = ledger.files.get_mut(&fid) else { return };
        f(lf);
        if let Err(e) = ledger.save(&rt.dir) {
            log::error!("ledger persist failed for transfer {}: {e}", tid_hex(&rt.tid));
        }
    }

    fn persist_verified_off(&self, rt: &RecvTransfer, fid: u32, off: u64) {
        {
            let mut files = rt.files.lock().unwrap();
            if let Some(st) = files.get_mut(&fid) {
                st.verified_off = off;
            }
        }
        self.write_ledger(rt, fid, |lf| lf.verified_off = off);
    }

    fn persist_file_done(&self, rt: &RecvTransfer, fid: u32, size: u64) {
        self.write_ledger(rt, fid, |lf| {
            lf.verified_off = size;
            lf.done = true;
        });
    }

    /// The shell routes the file from staging on `FileReady`.
    async fn finalize_file(&self, rt: &RecvTransfer, fid: u32) -> Result<PathBuf, WoooshError> {
        let (part, mut dest) = {
            let files = rt.files.lock().unwrap();
            let st = &files[&fid];
            let files_root = rt.dir.join("files");
            let dest = match &st.safe_rel {
                Some(rel) => sanitize::join_under_root(&files_root, rel)
                    .map_err(|e| WoooshError::Protocol(format!("rel_path: {e}")))?,
                None => files_root.join(&st.safe_name),
            };
            (rt.dir.join(format!("{fid}.part")), dest)
        };
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Collisions append " (2)", " (3)"… — never overwrite.
        if tokio::fs::try_exists(&dest).await.unwrap_or(false) {
            let stem = dest
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let ext = dest.extension().map(|e| e.to_string_lossy().to_string());
            for i in 2u32.. {
                let candidate = match &ext {
                    Some(e) => dest.with_file_name(format!("{stem} ({i}).{e}")),
                    None => dest.with_file_name(format!("{stem} ({i})")),
                };
                if !tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
                    dest = candidate;
                    break;
                }
            }
        }
        tokio::fs::rename(&part, &dest).await?;
        Ok(dest)
    }

    fn maybe_recv_complete(&self, rt: &Arc<RecvTransfer>) {
        let (all_done, okc, fc) = {
            let files = rt.files.lock().unwrap();
            let acc = rt.accepted.lock().unwrap();
            let Some(acc) = acc.as_ref() else { return };
            let all = acc.iter().all(|f| files.get(f).map(|s| s.done).unwrap_or(true));
            let okc = files.values().filter(|s| s.ok == Some(true)).count() as u32;
            let fc = files.values().filter(|s| s.ok == Some(false)).count() as u32;
            (all, okc, fc)
        };
        if all_done {
            self.emit(CoreEvent::TransferDone {
                transfer_id: tid_hex(&rt.tid),
                ok_files: okc,
                failed_files: fc,
                bytes_transferred: rt.bytes_this_attempt.load(Ordering::SeqCst),
                duration_ms: elapsed_ms(&rt.started_at),
            });
        }
    }

    async fn run_send(
        self: Arc<Self>,
        tid: [u8; 16],
        peer: Arc<Peer>,
        paths: Vec<PathBuf>,
    ) -> Result<(), WoooshError> {
        // §9.4: the redeemed token is the gate, since `trusted` is false.
        // Without it, anyone who learned the endpoint id gets the files.
        if peer.conn.is_internet()
            && !peer.ticket_authorized.load(Ordering::SeqCst)
            && !peer.trusted.load(Ordering::SeqCst)
        {
            return Err(WoooshError::Protocol(
                "refusing to send to an internet peer that has not redeemed a ticket".into(),
            ));
        }
        // Which limits apply cannot be decided until punching resolves.
        if !peer.conn.wait_for_direct(DIRECT_PATH_TIMEOUT).await {
            // Still relayed, so the cap applies (DESIGN.md §9.1). Metadata
            // only, and before the OFFER, so nothing is hashed or accepted.
            let to_check = paths.clone();
            let oversized = tokio::task::spawn_blocking(move || {
                to_check.iter().any(|p| {
                    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0) > RELAY_MAX_FILE_BYTES
                })
            })
            .await
            .map_err(|e| WoooshError::Io(e.to_string()))?;
            if oversized {
                return Err(WoooshError::RelayFileTooLarge);
            }
        }
        let mut files = BTreeMap::new();
        let mut metas = Vec::new();
        let mut total = 0u64;
        for (i, path) in paths.iter().enumerate() {
            let fid = (i + 1) as u32;
            let p = path.clone();
            let (size, b3, mtime) = tokio::task::spawn_blocking(move || -> Result<_, WoooshError> {
                let md = std::fs::metadata(&p)?;
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let mut hasher = blake3::Hasher::new();
                let mut f = std::fs::File::open(&p)?;
                std::io::copy(&mut f, &mut hasher)?;
                Ok((md.len(), *hasher.finalize().as_bytes(), mtime))
            })
            .await
            .map_err(|e| WoooshError::Io(e.to_string()))??;
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .ok_or_else(|| WoooshError::InvalidArgument(format!("bad path {path:?}")))?;
            let meta = FileMeta {
                fid,
                name: name.clone(),
                rel_path: None,
                size,
                mime: guess_mime(&name),
                b3,
                mtime,
            };
            total += size;
            metas.push(meta.clone());
            files.insert(fid, SendFile { path: path.clone(), meta });
        }

        let st = Arc::new(SendTransfer {
            tid,
            peer_id: peer.device_id.clone(),
            files,
            total,
            accepted: Mutex::new(None),
            offsets: Mutex::new(HashMap::new()),
            done: Mutex::new(HashMap::new()),
            decision_notify: Notify::new(),
            resume_notify: Notify::new(),
            cancelled: AtomicBool::new(false),
            bytes_this_attempt: AtomicU64::new(0),
            resumed_from: AtomicU64::new(0),
            started_at: Mutex::new(Instant::now()),
            done_emitted: AtomicBool::new(false),
        });
        self.sends.lock().unwrap().insert(tid, st.clone());

        peer.send_msg(Msg::Offer { tid, files: metas, total, note: None });

        // Sender must not open file streams before DECISION (PROTOCOL.md §5).
        let waited = tokio::time::timeout(DECISION_TIMEOUT, async {
            loop {
                if st.accepted.lock().unwrap().is_some() {
                    return;
                }
                st.decision_notify.notified().await;
            }
        })
        .await;
        if waited.is_err() {
            return Err(WoooshError::Transfer("timed out waiting for DECISION".into()));
        }
        let accepted = st.accepted.lock().unwrap().clone().unwrap_or_default();
        if accepted.is_empty() {
            self.emit(CoreEvent::TransferError {
                transfer_id: tid_hex(&tid),
                error: "declined by receiver".into(),
                resumable: false,
            });
            return Ok(());
        }
        *st.started_at.lock().unwrap() = Instant::now();
        self.emit_transfer_started(&st, TransferDirection::Send, &accepted);
        self.stream_files(&st, &peer, &accepted).await
    }

    fn emit_transfer_started(
        &self,
        st: &SendTransfer,
        direction: TransferDirection,
        accepted: &[u32],
    ) {
        let files: Vec<OfferedFile> = accepted
            .iter()
            .filter_map(|fid| st.files.get(fid))
            .map(|f| OfferedFile {
                fid: f.meta.fid,
                name: f.meta.name.clone(),
                rel_path: f.meta.rel_path.clone(),
                size: f.meta.size,
                mime: f.meta.mime.clone(),
            })
            .collect();
        let total = files.iter().map(|f| f.size).sum();
        self.emit(CoreEvent::TransferStarted {
            transfer_id: tid_hex(&st.tid),
            peer_id: st.peer_id.clone(),
            direction,
            files,
            total_bytes: total,
        });
    }

    async fn run_resume(
        self: Arc<Self>,
        st: Arc<SendTransfer>,
        peer: Arc<Peer>,
    ) -> Result<(), WoooshError> {
        peer.send_msg(Msg::ResumeQ { tid: st.tid });
        let waited = tokio::time::timeout(Duration::from_secs(30), st.resume_notify.notified()).await;
        if waited.is_err() {
            return Err(WoooshError::Transfer("timed out waiting for RESUME_A".into()));
        }
        let accepted: Vec<u32> = st.offsets.lock().unwrap().keys().copied().collect();
        if accepted.is_empty() {
            return Err(WoooshError::Transfer("peer has nothing to resume".into()));
        }
        *st.accepted.lock().unwrap() = Some(accepted.clone());
        st.done.lock().unwrap().clear();
        // Files the receiver already has send no DONE.
        let mut fully_done = Vec::new();
        {
            let offsets = st.offsets.lock().unwrap();
            for fid in &accepted {
                let size = st.files.get(fid).map(|f| f.meta.size).unwrap_or(0);
                if offsets.get(fid).copied().unwrap_or(0) >= size {
                    fully_done.push(*fid);
                }
            }
        }
        {
            let mut done = st.done.lock().unwrap();
            for fid in fully_done {
                done.insert(fid, (true, None));
            }
        }
        self.emit_transfer_started(&st, TransferDirection::Send, &accepted);
        self.stream_files(&st, &peer, &accepted).await
    }

    /// ≤4 slots; big files get a uni stream each, small files (<1 MiB) are
    /// pipelined over shared slot streams (PROTOCOL.md §6).
    async fn stream_files(
        self: &Arc<Self>,
        st: &Arc<SendTransfer>,
        peer: &Arc<Peer>,
        accepted: &[u32],
    ) -> Result<(), WoooshError> {
        let offsets = st.offsets.lock().unwrap().clone();
        let mut big = VecDeque::new();
        let mut small = VecDeque::new();
        for fid in accepted {
            let Some(f) = st.files.get(fid) else { continue };
            let off = offsets.get(fid).copied().unwrap_or(0);
            if off >= f.meta.size {
                continue; // receiver already has it
            }
            if f.meta.size < SMALL_FILE_LIMIT {
                small.push_back(*fid);
            } else {
                big.push_back(*fid);
            }
        }
        let small_slots = std::cmp::min(2, small.len());
        let big_slots = big.len();
        let big = Arc::new(Mutex::new(big));
        let small = Arc::new(Mutex::new(small));
        let slots = std::cmp::max(1, std::cmp::min(MAX_SLOTS, big_slots + small_slots));
        let mut workers = Vec::new();
        for _ in 0..slots {
            let inner = self.clone();
            let st = st.clone();
            let peer = peer.clone();
            let big = big.clone();
            let small = small.clone();
            let offsets = offsets.clone();
            workers.push(tokio::spawn(async move {
                loop {
                    let fid = big.lock().unwrap().pop_front();
                    let Some(fid) = fid else { break };
                    let off = offsets.get(&fid).copied().unwrap_or(0);
                    inner.send_one_file(&st, &peer, fid, off, None).await?;
                }
                let has_small = !small.lock().unwrap().is_empty();
                if has_small {
                    let mut stream = peer
                        .conn
                        .open_uni()
                        .await
                        .map_err(|e| WoooshError::Transfer(format!("open_uni: {e}")))?;
                    loop {
                        let fid = small.lock().unwrap().pop_front();
                        let Some(fid) = fid else { break };
                        let off = offsets.get(&fid).copied().unwrap_or(0);
                        inner.send_one_file(&st, &peer, fid, off, Some(&mut stream)).await?;
                    }
                    stream.finish();
                }
                Ok::<(), WoooshError>(())
            }));
        }
        let mut first_err = None;
        for w in workers {
            match w.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => first_err = Some(first_err.unwrap_or(e)),
                Err(e) => first_err = Some(WoooshError::Transfer(format!("worker panic: {e}"))),
            }
        }
        if let Some(e) = first_err {
            if st.cancelled.load(Ordering::SeqCst) {
                return Ok(());
            }
            self.emit(CoreEvent::TransferError {
                transfer_id: tid_hex(&st.tid),
                error: e.to_string(),
                resumable: true,
            });
            return Err(e);
        }
        // A pure catch-up resume sends no bytes, so no DONE arrives.
        let all_done = {
            let done = st.done.lock().unwrap();
            accepted.iter().all(|f| done.contains_key(f))
        };
        if all_done && !st.done_emitted.swap(true, Ordering::SeqCst) {
            let (okc, fc) = {
                let done = st.done.lock().unwrap();
                (
                    done.values().filter(|(o, _)| *o).count() as u32,
                    done.values().filter(|(o, _)| !*o).count() as u32,
                )
            };
            self.emit(CoreEvent::TransferDone {
                transfer_id: tid_hex(&st.tid),
                ok_files: okc,
                failed_files: fc,
                bytes_transferred: st.bytes_this_attempt.load(Ordering::SeqCst),
                duration_ms: elapsed_ms(&st.started_at),
            });
        }
        Ok(())
    }

    /// `shared` is the pipelined slot stream; `None` opens a dedicated one.
    async fn send_one_file(
        self: &Arc<Self>,
        st: &Arc<SendTransfer>,
        peer: &Arc<Peer>,
        fid: u32,
        off: u64,
        shared: Option<&mut SendStream>,
    ) -> Result<(), WoooshError> {
        let mut own_stream = None;
        let stream: &mut SendStream = match shared {
            Some(s) => s,
            None => {
                own_stream = Some(
                    peer.conn
                        .open_uni()
                        .await
                        .map_err(|e| WoooshError::Transfer(format!("open_uni: {e}")))?,
                );
                own_stream.as_mut().unwrap()
            }
        };
        let (path, size) = {
            let f = st
                .files
                .get(&fid)
                .ok_or_else(|| WoooshError::Transfer(format!("unknown fid {fid}")))?;
            (f.path.clone(), f.meta.size)
        };
        let header = StreamHeader { tid: st.tid, fid, off };
        let hbytes = header.encode();
        stream
            .write_all(&(hbytes.len() as u32).to_be_bytes())
            .await
            .map_err(|e| WoooshError::Transfer(format!("write header: {e}")))?;
        stream
            .write_all(&hbytes)
            .await
            .map_err(|e| WoooshError::Transfer(format!("write header: {e}")))?;

        let mut file = tokio::fs::File::open(&path).await?;
        if off > 0 {
            file.seek(std::io::SeekFrom::Start(off)).await?;
        }
        let mut remaining = size - off;
        let mut sent = off;
        let mut since_progress = 0u64;
        let mut buf = vec![0u8; CHUNK];
        while remaining > 0 {
            if st.cancelled.load(Ordering::SeqCst) {
                return Err(WoooshError::Transfer("cancelled".into()));
            }
            let n = std::cmp::min(remaining, CHUNK as u64) as usize;
            file.read_exact(&mut buf[..n]).await?;
            stream
                .write_all(&buf[..n])
                .await
                .map_err(|e| WoooshError::Transfer(format!("stream write: {e}")))?;
            remaining -= n as u64;
            sent += n as u64;
            since_progress += n as u64;
            st.bytes_this_attempt.fetch_add(n as u64, Ordering::SeqCst);
            if since_progress >= PROGRESS_INTERVAL {
                since_progress = 0;
                let elapsed = st.started_at.lock().unwrap().elapsed().as_secs_f64();
                let attempt = st.bytes_this_attempt.load(Ordering::SeqCst);
                let rate = if elapsed > 0.0 { attempt as f64 / elapsed } else { 0.0 };
                let eta = if rate > 0.0 { ((size - sent) as f64 / rate) as u64 } else { 0 };
                self.emit(CoreEvent::Progress {
                    transfer_id: tid_hex(&st.tid),
                    file_id: fid,
                    bytes_done: sent,
                    total_bytes: size,
                    rate_bps: rate as u64,
                    eta_secs: eta,
                });
            }
        }
        if let Some(mut s) = own_stream {
            s.finish();
        }
        Ok(())
    }
}

async fn read_u32_or_eof(stream: &mut RecvStream) -> Result<Option<u32>, WoooshError> {
    let mut b = [0u8; 4];
    match stream.read_exact(&mut b).await {
        Ok(()) => Ok(Some(u32::from_be_bytes(b))),
        Err(ReadErr::FinishedEarly(0)) => Ok(None),
        Err(e) => Err(WoooshError::Protocol(format!("frame length: {e}"))),
    }
}

async fn read_frame(recv: &mut RecvStream) -> Result<Option<Msg>, WoooshError> {
    let Some(len) = read_u32_or_eof(recv).await? else {
        return Ok(None);
    };
    if len > MAX_FRAME {
        return Err(WoooshError::Protocol(format!("frame too large: {len}")));
    }
    let mut body = vec![0u8; len as usize];
    recv.read_exact(&mut body)
        .await
        .map_err(|e| WoooshError::Protocol(format!("frame body: {e}")))?;
    Msg::decode(&body).map(Some).map_err(WoooshError::Protocol)
}

async fn write_frame(send: &mut SendStream, msg: &Msg) -> Result<(), WoooshError> {
    let body = msg.encode();
    send.write_all(&(body.len() as u32).to_be_bytes())
        .await
        .map_err(|e| WoooshError::Protocol(format!("frame write: {e}")))?;
    send.write_all(&body)
        .await
        .map_err(|e| WoooshError::Protocol(format!("frame write: {e}")))?;
    Ok(())
}

async fn write_loop(mut send: SendStream, mut rx: mpsc::UnboundedReceiver<Msg>) {
    while let Some(msg) = rx.recv().await {
        if write_frame(&mut send, &msg).await.is_err() {
            return;
        }
    }
    send.finish();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn qr_hints_put_lan_first_and_loopback_last() {
        let h = pairing_hints(ip("0.0.0.0"), Some(ip("10.229.10.242")), 57364);
        assert_eq!(h, vec!["10.229.10.242:57364", "127.0.0.1:57364"]);
    }

    #[test]
    fn qr_hints_omit_lan_guess_that_is_itself_loopback() {
        let h = pairing_hints(ip("0.0.0.0"), Some(ip("127.0.0.1")), 5000);
        assert_eq!(h, vec!["127.0.0.1:5000"]);
        let h = pairing_hints(ip("0.0.0.0"), None, 5000);
        assert_eq!(h, vec!["127.0.0.1:5000"]);
    }

    #[test]
    fn qr_hints_follow_a_specific_bind() {
        let h = pairing_hints(ip("127.0.0.1"), Some(ip("10.229.10.242")), 5000);
        assert_eq!(h, vec!["127.0.0.1:5000"]);
        let h = pairing_hints(ip("192.168.1.5"), Some(ip("192.168.1.5")), 5000);
        assert_eq!(h, vec!["192.168.1.5:5000"]);
    }

    #[test]
    fn qr_hints_bracket_ipv6() {
        let h = pairing_hints(ip("::"), Some(ip("fd00::1")), 5000);
        assert_eq!(h, vec!["[fd00::1]:5000", "[::1]:5000"]);
    }

    #[test]
    fn connect_errors_rank_specific_over_generic() {
        let timeout = WoooshError::Connect("connect timeout to 10.0.0.1:5000".into());
        let refused = WoooshError::Connect("connection refused".into());
        assert!(connect_error_rank(&refused) > connect_error_rank(&timeout));
        assert!(connect_error_rank(&WoooshError::QrKeyMismatch) > connect_error_rank(&refused));
    }
}
