//! Public FFI surface (UniFFI proc-macro style), mirroring DESIGN.md §4.
//!
//! Every exported method is **synchronous and blocking** and must never be
//! called on a UI thread. `CoreEventListener.on_event` runs on the core's own
//! event thread, so marshal to the UI thread there.

use crate::engine::{parse_tid, Engine, EngineConfig};
use crate::error::WoooshError;
use crate::identity::Identity;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, uniffi::Enum)]
pub enum Visibility {
    Everyone,
    PairedOnly,
    Off,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum DeviceType {
    Phone,
    Tablet,
    Laptop,
    Desktop,
}

impl DeviceType {
    pub(crate) fn as_wire(&self) -> &'static str {
        match self {
            DeviceType::Phone => "phone",
            DeviceType::Tablet => "tablet",
            DeviceType::Laptop => "laptop",
            DeviceType::Desktop => "desktop",
        }
    }

    /// `dt` of HELLO / mDNS TXT (PROTOCOL.md §3.1). Unknown values yield
    /// `None`, so the shell shows a generic icon instead of guessing.
    pub(crate) fn from_wire(s: &str) -> Option<DeviceType> {
        match s {
            "phone" => Some(DeviceType::Phone),
            "tablet" => Some(DeviceType::Tablet),
            "laptop" => Some(DeviceType::Laptop),
            "desktop" => Some(DeviceType::Desktop),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum FileKind {
    Photo,
    Video,
    Document,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Config {
    pub device_name: String,
    pub device_type: DeviceType,
    pub visibility: Visibility,
    pub staging_dir: String,
    pub trust_store_path: String,
    /// Default "0.0.0.0:0" (ephemeral port).
    pub listen_addr: Option<String>,
    /// Internet-path relays (DESIGN.md §9.1). `null` uses n0's public relays
    /// and contacts nothing until a ticket is asked for; an empty list
    /// disables relays and address lookup entirely.
    #[uniffi(default = None)]
    pub relay_urls: Option<Vec<String>>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct OfferedFile {
    pub fid: u32,
    pub name: String,
    pub rel_path: Option<String>,
    pub size: u64,
    pub mime: String,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum TransferDirection {
    Send,
    Receive,
}

/// Parsed QR pairing payload; the token stays internal to `pair_with_qr`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct QrInfo {
    pub pubkey: Vec<u8>,
    pub device_id: String,
    /// Unauthenticated hint, label only.
    pub device_name: Option<String>,
    pub hints: Vec<String>,
    pub expires_unix: u64,
    pub expired: bool,
}

/// Parsed internet ticket (PROTOCOL.md §9.2), for labelling the UI before
/// calling `redeem_ticket`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TicketInfo {
    /// Raw 32-byte Ed25519 key, identical to what the trust store pins, so
    /// `trusted_peers()` matching works without connecting.
    pub node_id: Vec<u8>,
    /// Same string events carry as `peer_id`.
    pub device_id: String,
    /// Unauthenticated hint, label only.
    pub device_name: Option<String>,
    pub relay: Option<String>,
    pub expires_unix: u64,
    pub expired: bool,
}

/// Largest single file over a **relayed** internet connection (DESIGN.md
/// §9.1). No limit on a direct path. Exported so shells never hardcode it.
#[uniffi::export]
pub fn relay_max_file_bytes() -> u64 {
    crate::engine::RELAY_MAX_FILE_BYTES
}

/// Parse a `wooosh-net:1?...` ticket without redeeming it.
#[uniffi::export]
pub fn parse_internet_ticket(ticket: String) -> Result<TicketInfo, WoooshError> {
    let t = crate::inet::NetTicket::parse(&ticket)?;
    Ok(TicketInfo {
        node_id: t.node_id.to_vec(),
        device_id: crate::identity::device_id_string_for(&t.node_id),
        device_name: t.dn.clone(),
        relay: t.relay.clone(),
        expires_unix: t.expires_unix,
        expired: t.is_expired(),
    })
}

/// One pinned peer as stored in the trust store (PROTOCOL.md §4.5).
#[derive(Debug, Clone, uniffi::Record)]
pub struct TrustedPeer {
    /// Raw 32 bytes, as `connect_peer` / `revoke_peer` expect.
    pub pubkey: Vec<u8>,
    /// Same string every `CoreEvent` carries as `peer_id`.
    pub device_id: String,
    /// Captured at pairing time.
    pub device_name: String,
    /// From the peer's HELLO at pairing time, when it was known.
    pub device_type: Option<DeviceType>,
    /// Same derivation as `fingerprint_phrase_for`.
    pub fingerprint: String,
    /// Unix seconds.
    pub paired_at: u64,
    /// Unix seconds of the last authenticated contact.
    pub last_seen: u64,
}

/// 6-word verification phrase for a peer key (PROTOCOL.md §2); shells must
/// never reimplement the wordlist. Errors unless `pubkey` is 32 bytes.
#[uniffi::export]
pub fn fingerprint_phrase_for(pubkey: Vec<u8>) -> Result<String, WoooshError> {
    let pk = <[u8; 32]>::try_from(pubkey.as_slice())
        .map_err(|_| WoooshError::InvalidArgument("pubkey must be 32 bytes".into()))?;
    Ok(crate::identity::fingerprint_phrase_for(&pk))
}

/// Rendered DeviceID (`Q7KM-3PXA-…`) for a peer key — the same string events
/// carry as `peer_id`. Errors unless `pubkey` is 32 bytes.
#[uniffi::export]
pub fn device_id_for(pubkey: Vec<u8>) -> Result<String, WoooshError> {
    let pk = <[u8; 32]>::try_from(pubkey.as_slice())
        .map_err(|_| WoooshError::InvalidArgument("pubkey must be 32 bytes".into()))?;
    Ok(crate::identity::device_id_string_for(&pk))
}

/// Parse a `wooosh-pair:1?...` payload without pairing.
#[uniffi::export]
pub fn parse_pairing_qr(payload: String) -> Result<QrInfo, WoooshError> {
    let qr = crate::pairing::QrPayload::parse(&payload)?;
    Ok(QrInfo {
        pubkey: qr.pubkey.to_vec(),
        device_id: crate::identity::device_id_string_for(&qr.pubkey),
        device_name: qr.dn.clone(),
        hints: qr.hints.clone(),
        expires_unix: qr.expires_unix,
        expired: qr.is_expired(),
    })
}

/// The single source of UI truth (DESIGN.md §4). Discovery events are absent
/// by design: the native shells own mDNS discovery.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum CoreEvent {
    /// `peer_pubkey` is the raw 32-byte key the peer proved possession of in
    /// the TLS handshake; pass it back to `connect_peer` / `revoke_peer`.
    PeerConnected {
        peer_id: String,
        peer_pubkey: Vec<u8>,
        device_name: String,
        device_type: Option<DeviceType>,
        fingerprint: String,
        trusted: bool,
    },
    PeerDisconnected { peer_id: String },
    PairingSas { peer_id: String, code: String },
    /// A peer redeemed this device's internet ticket (PROTOCOL.md §9.4).
    /// **Not a pairing:** nothing is written to the trust store and the
    /// authorisation dies with the connection.
    TicketRedeemed {
        peer_id: String,
        peer_pubkey: Vec<u8>,
        device_name: String,
    },
    /// `message` is the peer's device name on success, the reason on failure;
    /// `peer_pubkey` is the key that was (or would have been) pinned.
    PairingResult {
        peer_id: String,
        peer_pubkey: Vec<u8>,
        fingerprint: String,
        success: bool,
        message: Option<String>,
    },
    IncomingOffer {
        transfer_id: String,
        peer_id: String,
        peer_pubkey: Vec<u8>,
        from_name: String,
        device_type: Option<DeviceType>,
        trusted: bool,
        fingerprint: String,
        files: Vec<OfferedFile>,
        total_bytes: u64,
    },
    /// Bytes can now flow; carries the resolved manifest for progress UI.
    TransferStarted {
        transfer_id: String,
        peer_id: String,
        direction: TransferDirection,
        files: Vec<OfferedFile>,
        total_bytes: u64,
    },
    Progress {
        transfer_id: String,
        file_id: u32,
        bytes_done: u64,
        total_bytes: u64,
        rate_bps: u64,
        eta_secs: u64,
    },
    FileReady { transfer_id: String, file_id: u32, staged_path: String, kind: FileKind },
    /// `duration_ms` covers *this attempt only*: `resume_transfer` restarts
    /// the clock.
    TransferDone {
        transfer_id: String,
        ok_files: u32,
        failed_files: u32,
        bytes_transferred: u64,
        duration_ms: u64,
    },
    TransferError { transfer_id: String, error: String, resumable: bool },
    /// A pinned peer presented a different key (PROTOCOL.md §4.5). Surface it
    /// prominently; never silently re-pin. `peer_id` is the *pinned* identity
    /// we expected, `presented_pubkey` the key actually offered, if observed.
    KeyChanged { peer_id: String, expected_pubkey: Vec<u8>, presented_pubkey: Option<Vec<u8>> },
}

/// Identity-key storage implemented by the host (Keychain / Keystore / DPAPI).
/// Both methods run synchronously inside `WoooshCore.start` and may block,
/// which is why `start` must never be called on a UI thread.
#[uniffi::export(with_foreign)]
pub trait KeyStore: Send + Sync {
    /// 32-byte Ed25519 secret key, or None on first launch.
    fn load_identity(&self) -> Option<Vec<u8>>;
    fn store_identity(&self, secret: Vec<u8>);
}

/// Called on the core's dedicated event thread; calling back into the core
/// re-entrantly from here forfeits strict event ordering.
#[uniffi::export(with_foreign)]
pub trait CoreEventListener: Send + Sync {
    fn on_event(&self, event: CoreEvent);
}

/// File-based KeyStore used by the CLI.
#[derive(uniffi::Object)]
pub struct FileKeyStore {
    path: PathBuf,
}

#[uniffi::export]
impl FileKeyStore {
    #[uniffi::constructor]
    pub fn new(path: String) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { path: PathBuf::from(path) })
    }
}

impl KeyStore for FileKeyStore {
    fn load_identity(&self) -> Option<Vec<u8>> {
        std::fs::read(&self.path).ok().filter(|v| v.len() == 32)
    }

    fn store_identity(&self, secret: Vec<u8>) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, secret);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

struct Started {
    runtime: tokio::runtime::Runtime,
    engine: std::sync::Arc<Engine>,
    event_thread: Option<std::thread::JoinHandle<()>>,
    event_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(uniffi::Object)]
pub struct WoooshCore {
    state: Mutex<Option<Started>>,
}

impl WoooshCore {
    fn with_engine<R>(
        &self,
        f: impl FnOnce(&tokio::runtime::Runtime, &std::sync::Arc<Engine>) -> R,
    ) -> Result<R, WoooshError> {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let started = guard.as_ref().ok_or(WoooshError::NotStarted)?;
        // Host threads have no reactor; engine methods that spawn tasks or arm
        // timers panic without the runtime entered.
        let _entered = started.runtime.enter();
        Ok(f(&started.runtime, &started.engine))
    }
}

#[uniffi::export]
impl WoooshCore {
    #[uniffi::constructor]
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { state: Mutex::new(None) })
    }

    /// **BLOCKING — never call this on a UI thread.** Runs entirely on the
    /// calling thread and invokes `KeyStore` synchronously; Keychain /
    /// Keystore / DPAPI can take arbitrarily long.
    pub fn start(
        &self,
        config: Config,
        key_store: std::sync::Arc<dyn KeyStore>,
        listener: std::sync::Arc<dyn CoreEventListener>,
    ) -> Result<(), WoooshError> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return Err(WoooshError::AlreadyStarted);
        }

        let identity = match key_store.load_identity() {
            Some(bytes) => Identity::from_secret_bytes(&bytes)?,
            None => {
                let id = Identity::generate();
                key_store.store_identity(id.secret_bytes().to_vec());
                id
            }
        };

        let bind_addr: std::net::SocketAddr = config
            .listen_addr
            .as_deref()
            .unwrap_or("0.0.0.0:0")
            .parse()
            .map_err(|e| WoooshError::InvalidArgument(format!("listen_addr: {e}")))?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("wooosh-core")
            .build()
            .map_err(|e| WoooshError::Io(format!("tokio runtime: {e}")))?;

        // Own thread: a blocking host callback must never occupy a tokio worker.
        let (event_tx, event_rx) = std::sync::mpsc::channel::<CoreEvent>();
        let event_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag = event_stop.clone();
        let event_thread = std::thread::Builder::new()
            .name("wooosh-events".into())
            .spawn(move || {
                use std::sync::mpsc::RecvTimeoutError;
                loop {
                    match event_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                        Ok(ev) => listener.on_event(ev),
                        Err(RecvTimeoutError::Disconnected) => break,
                        // Poll the flag too: a sender clone stranded past
                        // runtime shutdown would wedge stop() forever.
                        Err(RecvTimeoutError::Timeout) => {
                            if stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
                                break;
                            }
                        }
                    }
                }
            })
            .map_err(|e| WoooshError::Io(format!("event thread: {e}")))?;

        let engine_cfg = EngineConfig {
            bind_addr,
            staging_dir: PathBuf::from(&config.staging_dir),
            trust_store_path: PathBuf::from(&config.trust_store_path),
            device_name: config.device_name.clone(),
            device_type: config.device_type.as_wire().to_string(),
            visibility: config.visibility.clone(),
            relay_urls: config.relay_urls.clone(),
        };
        let engine = {
            let _entered = runtime.enter();
            Engine::new(engine_cfg, identity, event_tx)?
        };

        *guard = Some(Started {
            runtime,
            engine,
            event_thread: Some(event_thread),
            event_stop,
        });
        Ok(())
    }

    /// **BLOCKING — not for a UI thread.** Waits up to ~2 s for the runtime,
    /// then joins the event thread behind any in-flight `on_event`.
    pub fn stop(&self) {
        let started = self.state.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(started) = started {
            let Started { runtime, engine, mut event_thread, event_stop } = started;
            engine.shutdown();
            runtime.shutdown_timeout(std::time::Duration::from_secs(2));
            event_stop.store(true, std::sync::atomic::Ordering::Relaxed);
            // The engine owns the event sender: drop it before the join or the
            // pump waits on a channel this scope holds open.
            drop(engine);
            if let Some(t) = event_thread.take() {
                let _ = t.join();
            }
        }
    }

    pub fn device_id(&self) -> Result<String, WoooshError> {
        self.with_engine(|_, e| e.identity().device_id_string())
    }

    pub fn fingerprint_phrase(&self) -> Result<String, WoooshError> {
        self.with_engine(|_, e| e.identity().fingerprint_phrase())
    }

    pub fn public_key(&self) -> Result<Vec<u8>, WoooshError> {
        self.with_engine(|_, e| e.identity().public_key_bytes().to_vec())
    }

    /// Bound "ip:port" — the shell advertises this over its own mDNS.
    pub fn listen_addr(&self) -> Result<String, WoooshError> {
        self.with_engine(|_, e| e.local_addr().to_string())
    }

    pub fn set_visibility(&self, mode: Visibility) -> Result<(), WoooshError> {
        self.with_engine(|_, e| e.set_visibility(mode))
    }

    /// Receiver side of QR pairing. Single-use token, 120 s expiry.
    pub fn begin_pairing_qr(&self) -> Result<String, WoooshError> {
        self.with_engine(|_, e| e.begin_pairing_qr())
    }

    /// Sender side of QR pairing, pinned to the QR key; returns the peer_id.
    /// Hints are dialled concurrently, first up wins. Every outcome also
    /// arrives as a `PairingResult` event, so a shell can drive off events.
    ///
    /// **BLOCKING — never call this on a UI thread.** Worst case ≈ 6 s connect
    /// (total, not per hint) plus a 20 s reply timeout.
    pub fn pair_with_qr(&self, payload: String) -> Result<String, WoooshError> {
        self.with_engine(|rt, e| rt.block_on(e.pair_with_qr(&payload)))?
    }

    /// Both sides then receive `PairingSas { code }` and must `confirm_sas`.
    pub fn request_sas_pairing(&self, peer_id: String) -> Result<(), WoooshError> {
        self.with_engine(|_, e| e.request_sas_pairing(&peer_id))?
    }

    pub fn confirm_sas(&self, peer_id: String, accepted: bool) -> Result<(), WoooshError> {
        self.with_engine(|_, e| e.confirm_sas(&peer_id, accepted))?
    }

    /// Receiver side of the internet path (PROTOCOL.md §9): publish on iroh
    /// and return a `wooosh-net:1?...` ticket. Connector = sender, as on LAN.
    /// The ticket is a capability: single-use token, 120 s expiry; anyone
    /// holding an unexpired one can connect and pair, so call
    /// `end_internet_ticket` as soon as the user leaves the screen.
    ///
    /// **BLOCKING — never call this on a UI thread.** The first call binds the
    /// iroh endpoint, waits up to ~15 s for a home relay, and is the first
    /// moment Wooosh contacts any relay at all.
    pub fn begin_internet_ticket(&self) -> Result<String, WoooshError> {
        self.with_engine(|rt, e| rt.block_on(e.begin_internet_ticket()))?
    }

    /// Invalidate the outstanding ticket immediately.
    pub fn end_internet_ticket(&self) -> Result<(), WoooshError> {
        self.with_engine(|_, e| e.end_internet_ticket())
    }

    /// Choose the relays the internet path uses (DESIGN.md §9.1): `None` =
    /// n0's public relays, `Some([])` = no relay and no address lookup,
    /// `Some(urls)` = a chosen set this device's tickets advertise. Relays
    /// only introduce devices, they never carry file data. Takes effect on the
    /// next ticket operation and invalidates any outstanding ticket. A
    /// malformed URL errors without disturbing the current setting.
    ///
    /// **BLOCKING — never call this on a UI thread.** Closes the bound iroh
    /// endpoint, an asynchronous shutdown.
    pub fn set_relay_urls(&self, urls: Option<Vec<String>>) -> Result<(), WoooshError> {
        self.with_engine(|rt, e| rt.block_on(e.set_relay_urls(urls)))?
    }

    /// Sender side of the internet path: dial the ticket's node, redeem its
    /// token, return the peer_id for `send`.
    ///
    /// **This does not pair** (PROTOCOL.md §9.4). Success arrives as
    /// `TicketRedeemed`, never `PairingResult`; failures do still arrive as
    /// `PairingResult { success: false }`.
    ///
    /// **BLOCKING — never call this on a UI thread.** Up to ~30 s of hole
    /// punching plus a 20 s pairing-reply timeout.
    pub fn redeem_ticket(&self, ticket: String) -> Result<String, WoooshError> {
        self.with_engine(|rt, e| rt.block_on(e.redeem_ticket(&ticket)))?
    }

    /// Connect to an address the shell discovered; returns the peer_id.
    ///
    /// `expected_pubkey` (32 raw bytes) pins the TLS handshake: a different
    /// key fails hard with `KeyChanged`. `None` does **not** opt out of
    /// pinning — the core re-applies its own pin whenever it can resolve the
    /// identity behind `addr` (PROTOCOL.md §4.5) — but passing it also pins
    /// the very first reconnect, before that address has been seen.
    ///
    /// **BLOCKING — never call this on a UI thread.** Up to ~10 s on an
    /// unreachable address.
    pub fn connect_peer(
        &self,
        addr: String,
        expected_pubkey: Option<Vec<u8>>,
    ) -> Result<String, WoooshError> {
        let expected = match expected_pubkey {
            Some(v) => Some(
                <[u8; 32]>::try_from(v.as_slice())
                    .map_err(|_| WoooshError::InvalidArgument("pubkey must be 32 bytes".into()))?,
            ),
            None => None,
        };
        self.with_engine(|rt, e| rt.block_on(e.connect_peer(&addr, expected)))?
    }

    /// Offer files to a peer; returns the transfer_id (hex). Streaming starts
    /// after the receiver's DECISION; completion arrives as TransferDone.
    ///
    /// **BLOCKING — not for a UI thread.** Registering the transfer blocks on
    /// the core's runtime.
    pub fn send(&self, peer_id: String, files: Vec<String>) -> Result<String, WoooshError> {
        let paths: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        self.with_engine(|rt, e| rt.block_on(e.send(&peer_id, paths)))?
    }

    /// RESUME_Q/RESUME_A (PROTOCOL.md §5); verified bytes are never re-sent.
    /// **BLOCKING — not for a UI thread** (same contract as `send`).
    pub fn resume_transfer(&self, peer_id: String, transfer_id: String) -> Result<(), WoooshError> {
        let tid = parse_tid(&transfer_id)?;
        self.with_engine(|rt, e| rt.block_on(e.resume_transfer(&peer_id, tid)))?
    }

    /// Empty list = decline all.
    pub fn respond_to_offer(
        &self,
        transfer_id: String,
        accepted_file_ids: Vec<u32>,
    ) -> Result<(), WoooshError> {
        let tid = parse_tid(&transfer_id)?;
        self.with_engine(|_, e| e.respond_to_offer(tid, accepted_file_ids))?
    }

    /// `file_id = None` cancels the whole transfer.
    pub fn cancel(&self, transfer_id: String, file_id: Option<u32>) -> Result<(), WoooshError> {
        let tid = parse_tid(&transfer_id)?;
        self.with_engine(|_, e| e.cancel(tid, file_id))?
    }

    /// The pinned peer set (PROTOCOL.md §4.5), read straight from
    /// `trust.json` rather than a local mirror that drifts. Ordered by
    /// `paired_at`, then device id.
    pub fn trusted_peers(&self) -> Result<Vec<TrustedPeer>, WoooshError> {
        self.with_engine(|_, e| e.trusted_peers())
    }

    /// Next contact with that device is untrusted again. Returns false if the
    /// key was not pinned.
    pub fn revoke_peer(&self, pubkey: Vec<u8>) -> Result<bool, WoooshError> {
        let pk = <[u8; 32]>::try_from(pubkey.as_slice())
            .map_err(|_| WoooshError::InvalidArgument("pubkey must be 32 bytes".into()))?;
        self.with_engine(|_, e| e.revoke_peer(&pk))?
    }
}

impl WoooshCore {
    #[doc(hidden)]
    pub fn transfer_stats(&self, transfer_id: &str) -> Option<crate::engine::TransferStats> {
        let tid = parse_tid(transfer_id).ok()?;
        self.with_engine(|_, e| e.transfer_stats(&tid)).ok().flatten()
    }

    #[doc(hidden)]
    pub fn debug_send_control(
        &self,
        peer_id: &str,
        msg: crate::control::Msg,
    ) -> Result<(), WoooshError> {
        self.with_engine(|_, e| e.debug_send_control(peer_id, msg))?
    }

    #[doc(hidden)]
    pub fn peer_connected(&self, peer_id: &str) -> bool {
        self.with_engine(|_, e| e.peer_connected(peer_id)).unwrap_or(false)
    }

    #[doc(hidden)]
    pub fn sas_code(&self, peer_id: &str) -> Option<u32> {
        self.with_engine(|_, e| e.sas_code(peer_id)).ok().flatten()
    }
}
