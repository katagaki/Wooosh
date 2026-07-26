//! Public FFI surface (UniFFI proc-macro style), mirroring DESIGN.md §4.
//! Commands in, an event stream out: the shell never sees sockets or crypto.
//!
//! # Threading contract (DESIGN.md §4)
//!
//! Every exported method is **synchronous and blocking**: `start`,
//! `pair_with_qr`, `connect_peer`, `send`, `resume_transfer` and `stop` drive
//! Keychain access, QUIC handshakes and runtime shutdown before they return.
//! **None of them may be called on a UI thread**; dispatch them to a
//! background executor and hop back with the result.
//! `CoreEventListener.on_event` runs on the core's own dedicated event thread,
//! never on the caller's — marshal to the UI thread there.

use crate::engine::{parse_tid, Engine, EngineConfig};
use crate::error::WoooshError;
use crate::identity::Identity;
use std::path::PathBuf;
use std::sync::Mutex;

// ---------- records & enums ----------

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

    /// Parse the `dt` field of HELLO / the mDNS TXT record (PROTOCOL.md §3.1).
    /// Unknown or absent values yield `None`, so the shell shows a generic
    /// icon instead of guessing.
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
    /// Private staging directory for in-flight transfers + resume ledgers.
    pub staging_dir: String,
    /// Path of the JSON trust store (pinned peer keys).
    pub trust_store_path: String,
    /// UDP listen address, e.g. "0.0.0.0:0" (default: ephemeral port).
    pub listen_addr: Option<String>,
    /// Relay servers for the internet path (DESIGN.md §9.1).
    ///
    /// `null` (the default) uses n0's free public relays — nothing to deploy,
    /// and nothing is contacted until the user asks for a ticket. An empty
    /// list disables relays and address lookup entirely, so the internet path
    /// only ever makes direct connections. A non-empty list points at chosen
    /// or self-hosted relays.
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

/// Parsed QR pairing payload (UI-facing subset; the token stays internal to
/// `pair_with_qr`).
#[derive(Debug, Clone, uniffi::Record)]
pub struct QrInfo {
    pub pubkey: Vec<u8>,
    pub device_id: String,
    /// Display-name hint from the QR (unauthenticated, label only).
    pub device_name: Option<String>,
    pub hints: Vec<String>,
    pub expires_unix: u64,
    pub expired: bool,
}

/// Parsed internet ticket (PROTOCOL.md §9.2), so a shell can label the
/// "Receive from internet" UI before calling `redeem_ticket`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TicketInfo {
    /// Publisher's raw 32-byte Ed25519 key. Identical to the value the trust
    /// store pins, so `trusted_peers()` matching works without connecting.
    pub node_id: Vec<u8>,
    /// Rendered DeviceID — the same string events carry as `peer_id`.
    pub device_id: String,
    /// Display-name hint from the ticket (unauthenticated, label only).
    pub device_name: Option<String>,
    pub relay: Option<String>,
    pub expires_unix: u64,
    pub expired: bool,
}

/// Largest single file Wooosh will move over a **relayed** internet connection
/// (DESIGN.md §9.1). No limit applies on a direct path, which includes every
/// LAN transfer and every internet transfer that hole punched.
///
/// Exported so a shell can name the limit in its own copy instead of
/// hardcoding a number that would drift the moment the core changed it.
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
    /// Raw 32-byte Ed25519 identity key — the value `connect_peer`'s
    /// `expected_pubkey` and `revoke_peer` expect.
    pub pubkey: Vec<u8>,
    /// Rendered DeviceID (`Q7KM-3PXA-…`). Identical to the `peer_id` carried
    /// by every `CoreEvent`.
    pub device_id: String,
    /// Display name captured at pairing time.
    pub device_name: String,
    /// Device type from the peer's HELLO at pairing time, when it was known.
    pub device_type: Option<DeviceType>,
    /// 6-word fingerprint phrase for this key (same derivation as
    /// `fingerprint_phrase_for`).
    pub fingerprint: String,
    /// Unix seconds when the pin was first created.
    pub paired_at: u64,
    /// Unix seconds of the last authenticated contact.
    pub last_seen: u64,
}

/// 6-word verification phrase for a peer public key (PROTOCOL.md §2), shown on
/// consent and trust-list screens. Pass any event's `peer_pubkey` or a
/// `TrustedPeer.pubkey`; shells must never reimplement the wordlist.
///
/// Errors with `InvalidArgument` unless `pubkey` is exactly 32 bytes.
#[uniffi::export]
pub fn fingerprint_phrase_for(pubkey: Vec<u8>) -> Result<String, WoooshError> {
    let pk = <[u8; 32]>::try_from(pubkey.as_slice())
        .map_err(|_| WoooshError::InvalidArgument("pubkey must be 32 bytes".into()))?;
    Ok(crate::identity::fingerprint_phrase_for(&pk))
}

/// Rendered DeviceID (`Q7KM-3PXA-…`) for a peer public key — the same string
/// events carry as `peer_id`, so a shell can key its UI off a `trusted_peers()`
/// pubkey without connecting first.
///
/// Errors with `InvalidArgument` unless `pubkey` is exactly 32 bytes.
#[uniffi::export]
pub fn device_id_for(pubkey: Vec<u8>) -> Result<String, WoooshError> {
    let pk = <[u8; 32]>::try_from(pubkey.as_slice())
        .map_err(|_| WoooshError::InvalidArgument("pubkey must be 32 bytes".into()))?;
    Ok(crate::identity::device_id_string_for(&pk))
}

/// Parse a scanned `wooosh-pair:1?...` payload so the shell can label the
/// pairing UI before calling `pair_with_qr`.
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
    /// A control channel is up. `peer_pubkey` is the peer's raw 32-byte
    /// Ed25519 key, taken from the certificate it proved possession of in the
    /// TLS handshake; pass it back to `connect_peer` / `revoke_peer`.
    /// `device_type` is the peer's HELLO `dt` (None if unrecognized).
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
    /// Pairing concluded (QR or SAS; success, failure or timeout). `message`
    /// is the peer's device name on success, the reason on failure.
    /// `peer_pubkey` is the key that was (or would have been) pinned.
    /// A peer redeemed this device's internet ticket (PROTOCOL.md §9.4).
    ///
    /// **Not a pairing.** Nothing is written to the trust store and the
    /// authorisation dies with the connection. This is the signal the sending
    /// shell waits on before handing over the files it staged when it showed
    /// the code.
    TicketRedeemed {
        peer_id: String,
        peer_pubkey: Vec<u8>,
        device_name: String,
    },
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
    /// Bytes can now flow (outgoing: DECISION accepted; incoming: offer
    /// accepted). Carries the resolved manifest for progress UI.
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
    /// `duration_ms` covers *this attempt only* — `resume_transfer` restarts
    /// the clock — measured from when bytes could start flowing (sender:
    /// DECISION received; receiver: offer accepted) to the last DONE.
    /// `bytes_transferred / duration_ms` is the attempt's average rate.
    TransferDone {
        transfer_id: String,
        ok_files: u32,
        failed_files: u32,
        bytes_transferred: u64,
        duration_ms: u64,
    },
    TransferError { transfer_id: String, error: String, resumable: bool },
    /// A pinned peer presented a different key (PROTOCOL.md §4.5). Surface it
    /// prominently; never silently re-pin. `peer_id` is the DeviceID of the
    /// *pinned* identity we expected; `presented_pubkey` is the key actually
    /// offered, when the handshake got far enough to observe it.
    KeyChanged { peer_id: String, expected_pubkey: Vec<u8>, presented_pubkey: Option<Vec<u8>> },
}

// ---------- host-implemented adapters ----------

/// Identity-key storage implemented by the host (Keychain / Keystore / DPAPI).
///
/// **Threading.** Both methods are invoked synchronously from inside
/// `WoooshCore.start`, on whichever thread called it, and may block — which is
/// why `start` must never be called on a UI thread.
#[uniffi::export(with_foreign)]
pub trait KeyStore: Send + Sync {
    /// Return the stored 32-byte Ed25519 secret key, or None on first launch.
    fn load_identity(&self) -> Option<Vec<u8>>;
    /// Persist the (newly generated) 32-byte secret key.
    fn store_identity(&self, secret: Vec<u8>);
}

/// Event sink implemented by the host. Called on the core's dedicated event
/// thread; implementations may block briefly, but calling back into the core
/// re-entrantly from the callback forfeits strict event ordering.
#[uniffi::export(with_foreign)]
pub trait CoreEventListener: Send + Sync {
    fn on_event(&self, event: CoreEvent);
}

/// File-based KeyStore used by the CLI (and available to any host).
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

// ---------- the core object ----------

struct Started {
    runtime: tokio::runtime::Runtime,
    engine: std::sync::Arc<Engine>,
    event_thread: Option<std::thread::JoinHandle<()>>,
    event_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The engine handle the platform apps bind against.
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
        // Sync FFI calls arrive on host threads with no reactor; engine methods
        // that spawn tasks or arm timers panic without the runtime entered.
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

    /// Boot the engine: load or create the identity through the KeyStore
    /// adapter, bind the QUIC endpoint, start the event pump.
    ///
    /// **BLOCKING — never call this on a UI thread.** It runs entirely on the
    /// calling thread and invokes `KeyStore.load_identity` / `store_identity`
    /// synchronously; Keychain / Keystore / DPAPI can take arbitrarily long
    /// (first unlock, biometric prompt). Dispatch to a background executor.
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

        // Event pump on its own thread: a blocking host callback must never
        // occupy a tokio worker.
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
                        // Poll the flag as well as the channel: a sender clone
                        // stranded in a task that outlived runtime shutdown
                        // would otherwise wedge stop() forever.
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

    /// Shut down: closes the endpoint and stops the event pump.
    ///
    /// **BLOCKING — not for a UI thread.** Waits up to ~2 s for the runtime,
    /// then joins the event thread, so it can also block behind an in-flight
    /// `CoreEventListener.on_event` call.
    pub fn stop(&self) {
        let started = self.state.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(started) = started {
            let Started { runtime, engine, mut event_thread, event_stop } = started;
            engine.shutdown();
            runtime.shutdown_timeout(std::time::Duration::from_secs(2));
            event_stop.store(true, std::sync::atomic::Ordering::Relaxed);
            // The engine owns the event sender, so it must be dropped BEFORE
            // the join: otherwise the pump thread waits on a channel this very
            // scope is holding open.
            drop(engine);
            if let Some(t) = event_thread.take() {
                let _ = t.join();
            }
        }
    }

    // ----- identity -----

    pub fn device_id(&self) -> Result<String, WoooshError> {
        self.with_engine(|_, e| e.identity().device_id_string())
    }

    pub fn fingerprint_phrase(&self) -> Result<String, WoooshError> {
        self.with_engine(|_, e| e.identity().fingerprint_phrase())
    }

    pub fn public_key(&self) -> Result<Vec<u8>, WoooshError> {
        self.with_engine(|_, e| e.identity().public_key_bytes().to_vec())
    }

    /// Actual bound listen address ("ip:port") — the shell advertises this
    /// over its native mDNS discovery.
    pub fn listen_addr(&self) -> Result<String, WoooshError> {
        self.with_engine(|_, e| e.local_addr().to_string())
    }

    // ----- visibility -----

    pub fn set_visibility(&self, mode: Visibility) -> Result<(), WoooshError> {
        self.with_engine(|_, e| e.set_visibility(mode))
    }

    // ----- pairing -----

    /// Receiver side of QR pairing: returns the `wooosh-pair:1?...` payload
    /// to render as a QR code. Single-use token, 120 s expiry.
    pub fn begin_pairing_qr(&self) -> Result<String, WoooshError> {
        self.with_engine(|_, e| e.begin_pairing_qr())
    }

    /// Sender side of QR pairing: parse payload, connect (pinned to the QR
    /// key), redeem the token. Returns the paired peer_id.
    ///
    /// The QR's address hints are dialled concurrently; the first connection
    /// to come up wins, so a stale hint never delays the ones behind it.
    ///
    /// Every outcome — success, rejection, bad or expired QR, nothing
    /// reachable — also arrives as a `PairingResult` event, so a shell can
    /// drive its pairing UI entirely off events and never wedge.
    ///
    /// **BLOCKING — never call this on a UI thread.** Worst case ≈ 6 s connect
    /// timeout (total, not per hint) plus a 20 s reply timeout.
    pub fn pair_with_qr(&self, payload: String) -> Result<String, WoooshError> {
        self.with_engine(|rt, e| rt.block_on(e.pair_with_qr(&payload)))?
    }

    /// Start SAS pairing with a connected (untrusted) peer. Both sides then
    /// receive `PairingSas { code }` and must call `confirm_sas`.
    pub fn request_sas_pairing(&self, peer_id: String) -> Result<(), WoooshError> {
        self.with_engine(|_, e| e.request_sas_pairing(&peer_id))?
    }

    pub fn confirm_sas(&self, peer_id: String, accepted: bool) -> Result<(), WoooshError> {
        self.with_engine(|_, e| e.confirm_sas(&peer_id, accepted))?
    }

    // ----- internet path (DESIGN.md §9.1, PROTOCOL.md §9) -----

    /// Receiver side of the internet path: publish this device on iroh and
    /// return a `wooosh-net:1?...` ticket to show as text or a QR code. The
    /// sender redeems it with `redeem_ticket` and then offers files, so the
    /// roles match the LAN path exactly (connector = sender).
    ///
    /// The ticket carries this device's identity key, a single-use pairing
    /// token and a 120 s expiry. It is a capability: anyone holding an
    /// unexpired one can connect and pair. Call `end_internet_ticket` as soon
    /// as the user leaves the screen.
    ///
    /// **BLOCKING — never call this on a UI thread.** The first call binds the
    /// iroh endpoint and waits up to ~15 s for a home relay; it is also the
    /// first moment Wooosh contacts any relay at all.
    pub fn begin_internet_ticket(&self) -> Result<String, WoooshError> {
        self.with_engine(|rt, e| rt.block_on(e.begin_internet_ticket()))?
    }

    /// Invalidate the outstanding ticket immediately.
    pub fn end_internet_ticket(&self) -> Result<(), WoooshError> {
        self.with_engine(|_, e| e.end_internet_ticket())
    }

    /// Choose the relays the internet path uses (DESIGN.md §9.1).
    ///
    /// - `None` — n0's free public relays. Shared infrastructure with no
    ///   uptime guarantee; fine because Wooosh only ever uses a relay to
    ///   introduce two devices, never to carry file data.
    /// - `Some([])` — no relay and no address lookup. Nothing leaves this
    ///   device except to addresses carried in a ticket.
    /// - `Some(urls)` — a chosen or self-hosted relay set. This device's
    ///   tickets advertise it, so a redeemer uses it with no configuration of
    ///   their own.
    ///
    /// Takes effect on the next ticket operation, and invalidates any
    /// outstanding ticket, which advertises a relay that is no longer in use.
    /// Errors on a malformed URL without disturbing the current setting.
    ///
    /// **BLOCKING — never call this on a UI thread.** It closes the bound iroh
    /// endpoint, which is an asynchronous shutdown.
    pub fn set_relay_urls(&self, urls: Option<Vec<String>>) -> Result<(), WoooshError> {
        self.with_engine(|rt, e| rt.block_on(e.set_relay_urls(urls)))?
    }

    /// Sender side of the internet path: dial the ticket's node over iroh,
    /// redeem its token, and return the peer_id to pass to `send`.
    ///
    /// **This does not pair** (PROTOCOL.md §9.4). Success arrives as
    /// `TicketRedeemed`, never `PairingResult`, and nothing is written to the
    /// trust store — a shell that waits for `PairingResult` here will wait for
    /// ever. Failures do still arrive as `PairingResult { success: false }`.
    ///
    /// **BLOCKING — never call this on a UI thread.** Up to ~30 s of hole
    /// punching plus a 20 s pairing-reply timeout.
    pub fn redeem_ticket(&self, ticket: String) -> Result<String, WoooshError> {
        self.with_engine(|rt, e| rt.block_on(e.redeem_ticket(&ticket)))?
    }

    // ----- connections -----

    /// Connect to a peer address discovered by the native shell. Returns the
    /// peer_id.
    ///
    /// `expected_pubkey` (32 raw bytes, e.g. from `TrustedPeer.pubkey` or any
    /// event's `peer_pubkey`) pins the TLS handshake: a different key fails
    /// hard with `KeyChanged` and no connection is established.
    ///
    /// Passing `None` does **not** opt out of pinning — the core re-applies
    /// the pin from its own trust store whenever it can resolve the identity
    /// behind `addr` (PROTOCOL.md §4.5, DESIGN.md §4), so a SAS-paired peer is
    /// protected even if the shell forgets the key. Passing it explicitly is
    /// still preferred: it also pins the very first reconnect, before the core
    /// has ever seen that address.
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

    // ----- transfers -----

    /// Offer files to a peer; returns the transfer_id (hex). Streaming starts
    /// after the receiver's DECISION; completion arrives as TransferDone.
    ///
    /// **BLOCKING — not for a UI thread.** Hashing and streaming happen in the
    /// background and report through events, but registering the transfer
    /// blocks on the core's runtime.
    pub fn send(&self, peer_id: String, files: Vec<String>) -> Result<String, WoooshError> {
        let paths: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        self.with_engine(|rt, e| rt.block_on(e.send(&peer_id, paths)))?
    }

    /// Resume a previously offered transfer after reconnecting to the peer
    /// (RESUME_Q/RESUME_A, PROTOCOL.md §5). Verified bytes are never re-sent.
    ///
    /// **BLOCKING — not for a UI thread** (same contract as `send`).
    pub fn resume_transfer(&self, peer_id: String, transfer_id: String) -> Result<(), WoooshError> {
        let tid = parse_tid(&transfer_id)?;
        self.with_engine(|rt, e| rt.block_on(e.resume_transfer(&peer_id, tid)))?
    }

    /// Answer an IncomingOffer. Empty list = decline all.
    pub fn respond_to_offer(
        &self,
        transfer_id: String,
        accepted_file_ids: Vec<u32>,
    ) -> Result<(), WoooshError> {
        let tid = parse_tid(&transfer_id)?;
        self.with_engine(|_, e| e.respond_to_offer(tid, accepted_file_ids))?
    }

    /// Cancel a whole transfer (file_id = None) or one file.
    pub fn cancel(&self, transfer_id: String, file_id: Option<u32>) -> Result<(), WoooshError> {
        let tid = parse_tid(&transfer_id)?;
        self.with_engine(|_, e| e.cancel(tid, file_id))?
    }

    // ----- trust management -----

    /// The pinned peer set (PROTOCOL.md §4.5) — the shell's trust list, read
    /// straight from `trust.json` rather than a local mirror that drifts.
    /// Ordered by `paired_at`, then device id. Re-read after every
    /// `PairingResult { success: true }` and after `revoke_peer`.
    pub fn trusted_peers(&self) -> Result<Vec<TrustedPeer>, WoooshError> {
        self.with_engine(|_, e| e.trusted_peers())
    }

    /// Remove a pinned key; next contact with that device is untrusted again.
    /// `pubkey` is the raw 32 bytes from `TrustedPeer.pubkey` or from any
    /// event's `peer_pubkey`. Returns false if the key was not pinned.
    pub fn revoke_peer(&self, pubkey: Vec<u8>) -> Result<bool, WoooshError> {
        let pk = <[u8; 32]>::try_from(pubkey.as_slice())
            .map_err(|_| WoooshError::InvalidArgument("pubkey must be 32 bytes".into()))?;
        self.with_engine(|_, e| e.revoke_peer(&pk))?
    }
}

// ---------- test/debug helpers (not FFI-exported) ----------

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
