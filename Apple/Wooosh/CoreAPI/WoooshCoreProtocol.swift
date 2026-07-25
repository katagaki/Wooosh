import Foundation
import UniformTypeIdentifiers

// MARK: - Core ↔ shell API contract (DESIGN.md §4)
//
// The app's FFI seam: `RealCore` adapts the UniFFI-generated
// `WoooshCoreFFI.WoooshCore` object to it. Views and stores talk only to this
// protocol, never to the generated bindings.

/// Opaque transfer identifier (`tid` in PROTOCOL.md §5). Stable across resumes.
struct TransferID: Hashable, Identifiable, Sendable {
    let raw: String
    var id: String { raw }
}

/// One file in an OFFER manifest (PROTOCOL.md §5 `FileMeta`, UI subset).
struct FileMeta: Identifiable, Hashable, Sendable {
    let id: UInt32
    let name: String
    let size: Int64
    let mime: String
}

/// Storage-routing classification supplied by the core with `fileReady`
/// (DESIGN.md §6). The shell may override based on its own sniffing.
enum FileKind: String, Sendable {
    case photo
    case video
    case document

    /// Shell-side classification (MIME sniff + extension, DESIGN.md §6).
    init(mime: String) {
        if mime.hasPrefix("image/") { self = .photo }
        else if mime.hasPrefix("video/") { self = .video }
        else { self = .document }
    }

    init(filename: String) {
        guard let type = UTType(filenameExtension: (filename as NSString).pathExtension) else {
            self = .document
            return
        }
        if type.conforms(to: .movie) { self = .video }
        else if type.conforms(to: .image) { self = .photo }
        else { self = .document }
    }
}

/// Minimal peer reference used in core events. `id` is the peer's DeviceID
/// (BLAKE3(pubkey)[0..16], base32) as produced by the core.
struct PeerRef: Hashable, Identifiable, Sendable {
    let id: String
    var displayName: String
    /// The peer's self-declared type from its HELLO, when the core knew it.
    /// `nil` means "not reported" — the shell must not invent one, it just
    /// falls back to a neutral icon (see `icon`).
    var deviceType: DeviceType?
    /// 6-word fingerprint phrase (PROTOCOL.md §2), supplied by the core.
    var fingerprint: String
    /// Raw Ed25519 public key. Every identity-bearing core event now carries
    /// it, so the shell can pin a later `connectPeer` and revoke by key.
    var publicKey: Data?

    init(id: String, displayName: String, deviceType: DeviceType?,
         fingerprint: String, publicKey: Data? = nil) {
        self.id = id
        self.displayName = displayName
        self.deviceType = deviceType
        self.fingerprint = fingerprint
        self.publicKey = publicKey
    }

    /// The core's form-factor enum can't distinguish an iPhone from a Pixel,
    /// so ambiguous values render neutrally instead of being guessed at
    /// (PROTOCOL.md §3.1).
    var symbolName: String { DeviceIcon.symbol(forCoreType: deviceType) }
}

/// A parsed `wooosh-net:1?…` internet ticket (PROTOCOL.md §9.2), read without
/// dialling anything.
///
/// Everything here is label material except `publicKey`, which is the key the
/// handshake is pinned to. That key arriving out of band is what makes the
/// internet path MITM-proof with no extra ceremony.
struct TicketInfo: Sendable {
    let publicKey: Data
    let deviceID: String
    let deviceName: String?
    /// Home relay in the ticket; nil when it carries direct addresses only.
    let relay: String?
    let expired: Bool
}

/// One pinned peer, read straight from the core's canonical trust store
/// (PROTOCOL.md §4.5). The shell keeps no trust state of its own.
struct TrustedPeerInfo: Hashable, Identifiable, Sendable {
    /// Raw 32-byte Ed25519 identity key — what `connectPeer`'s
    /// `expectedPublicKey` and `revokePeer` take.
    let publicKey: Data
    /// Rendered DeviceID; identical to the `peerId` in every core event.
    let deviceID: String
    let displayName: String
    let deviceType: DeviceType?
    let fingerprint: String
    let pairedAt: Date
    let lastSeen: Date

    var id: String { deviceID }
    var symbolName: String { DeviceIcon.symbol(forCoreType: deviceType) }
}

struct TransferSummary: Sendable {
    let fileCount: Int
    let totalBytes: Int64
    let duration: TimeInterval
}

struct CoreConfig: Sendable {
    let displayName: String
    let deviceType: DeviceType
    let visibility: Visibility
    /// Core-private staging directory for incoming `.part` files.
    let stagingDirectory: URL
    /// Core-owned trust store (pinned peer keys); canonical.
    let trustStoreURL: URL
    /// UDP bind address; nil = ephemeral port on all interfaces.
    let listenAddress: String?
}

/// Event stream (core → shell) — the single source of UI truth for
/// pairing and transfers (DESIGN.md §4).
enum CoreEvent: Sendable {
    /// A QUIC connection to a peer is up. This is how a peer that the shell
    /// never saw over mDNS (a pasted QR payload, a manual address) enters the
    /// device list.
    case peerConnected(peer: PeerRef, trusted: Bool)
    case peerDisconnected(peerID: String)
    case pairingSAS(peer: PeerRef, sixDigits: String)
    /// `trusted` is the core's verdict (pinned key present), not a guess.
    case incomingOffer(transferID: TransferID, from: PeerRef, trusted: Bool, manifest: [FileMeta])
    case transferStarted(transferID: TransferID, peerID: String, outgoing: Bool, manifest: [FileMeta])
    case progress(transferID: TransferID, fileID: UInt32, bytes: Int64, totalBytes: Int64, rate: Double, eta: TimeInterval)
    /// Core finished hash verification in staging; the shell now routes the
    /// file to its platform destination (DESIGN.md §6) and cleans staging.
    case fileReady(transferID: TransferID, fileID: UInt32, stagedURL: URL, kind: FileKind)
    case transferDone(transferID: TransferID, summary: TransferSummary)
    case transferError(transferID: TransferID, message: String, resumable: Bool)
    /// A pinned peer presented a different key (PROTOCOL.md §4.5). Never
    /// silently re-pin — surface prominently and require re-pairing. Both keys
    /// come through so the warning can name the two fingerprints; `presented`
    /// is nil when the mismatch was detected before a key was even offered.
    case keyChanged(peer: PeerRef, expectedPublicKey: Data, presentedPublicKey: Data?)
    /// Pairing completion signal (QR or SAS), used to dismiss pairing UI and
    /// update the trust list.
    case pairingResult(peer: PeerRef, success: Bool, message: String?)
    /// A peer redeemed this device's internet ticket (PROTOCOL.md §9.4).
    ///
    /// **Not a pairing.** Nothing is written to the trust store and the
    /// authorisation dies with the connection. This is what the sending screen
    /// waits on before handing over the files it staged.
    case ticketRedeemed(peer: PeerRef)
}

@MainActor
protocol WoooshCore: AnyObject {
    /// Single-consumer event stream; the shell pumps this into its stores.
    var events: AsyncStream<CoreEvent> { get }

    /// Async because starting the core binds a socket and calls back into
    /// the host `KeyStore` on the calling thread — a Keychain read can block
    /// (and can even put a system prompt up). Never run it on the main actor.
    func start(config: CoreConfig) async throws
    /// Closes the endpoint and stops the event pump. Must be safe to call
    /// twice and must not deadlock against the callback thread.
    func stop()

    /// Identity — owned by the core (PROTOCOL.md §2). `nil` before `start`.
    var deviceID: String? { get }
    var fingerprintPhrase: String? { get }
    /// Bound "ip:port" of the core's QUIC socket, for the mDNS TXT `p` field.
    var listenAddress: String? { get }

    func setVisibility(_ mode: Visibility)

    /// Returns the `wooosh-pair:1?...` payload to render as QR/copyable text
    /// (PROTOCOL.md §4.2). Token is single-use, 120 s expiry.
    func beginPairingQR() -> String
    /// Sender-side QR pairing: connect to the peer named in the payload,
    /// verify its cert key against the QR key, redeem the token. Resolves
    /// asynchronously through a `pairingResult` event.
    func pairWithQR(payload: String)
    /// Reads the peer identity out of a pairing payload *without* connecting,
    /// so the UI can say "Connecting to Aki's MacBook Pro…" the instant the
    /// code is scanned instead of "Pairing…". Returns nil for an unparseable
    /// payload. Parsing lives in the core (PROTOCOL.md §4.2); the shell never
    /// picks the payload apart itself.
    func peerHint(forPairingPayload payload: String) -> PeerRef?
    /// SAS numeric-comparison verdict from the user (PROTOCOL.md §4.3).
    func confirmSAS(peerID: String, accepted: Bool)
    /// Initiate camera-less SAS pairing with a connected peer
    /// (PROTOCOL.md §4.3 step 1 — sending `PAIR_REQUEST {}`).
    func requestSASPairing(peerID: String)

    // MARK: - Internet path (DESIGN.md §9.1, PROTOCOL.md §9)

    /// Publishes this device on the internet path and returns the
    /// `wooosh-net:1?…` ticket to render as QR/copyable text.
    ///
    /// Async because it is genuinely slow and genuinely consequential: the
    /// first call binds the iroh endpoint and waits up to ~15 s for a home
    /// relay, and it is the first moment Wooosh contacts a relay at all. That
    /// is why nothing calls it until the user asks for a code.
    func beginInternetTicket() async throws -> String

    /// Invalidates the outstanding ticket. A ticket is a capability, so this
    /// runs the moment the user leaves the screen rather than waiting for the
    /// 120 s expiry.
    func endInternetTicket()

    /// Sender-side internet path: dial the ticket's node, redeem its token,
    /// pin the key it was minted with. Resolves through a `pairingResult`
    /// event exactly like `pairWithQR`.
    func redeemTicket(_ ticket: String)

    /// Reads the peer identity out of a ticket *without* connecting, so the UI
    /// can name the device while the dial runs. Returns nil for anything that
    /// is not a Wooosh ticket.
    func peerHint(forTicket ticket: String) -> PeerRef?

    /// Ticket details for a payload the shell needs to inspect before acting
    /// on it (expiry, relay). Returns nil when the payload is not a ticket.
    func ticketInfo(for ticket: String) -> TicketInfo?

    /// Chooses the relays the internet path uses.
    ///
    /// - `nil` — n0's free public relays.
    /// - `[]` — no relay and no address lookup; nothing leaves this device
    ///   except to addresses carried in a ticket.
    /// - a list — a chosen or self-hosted relay. This device's tickets
    ///   advertise it, so a redeemer uses it without configuring anything.
    ///
    /// Throws on a malformed URL, leaving the working setting in place.
    func setRelayURLs(_ urls: [String]?) async throws

    /// Connect to a resolved address (shell-side mDNS resolution, or the
    /// Tailscale / direct-IP path, DESIGN.md §9.2). Returns the peer id.
    ///
    /// `expectedPublicKey` pins the handshake for an already-paired peer.
    /// Passing nil is *not* "unpinned": the core resolves the pin from its own
    /// trust store by address, and that fallback only matches a peer that came
    /// back on the same `ip:port`. The shell passing the key (looked up in
    /// `trustedPeers()` by DeviceID) is what actually closes the gap, so do it
    /// whenever the DeviceID is known. Throws `.KeyChanged` on a mismatch.
    func connectPeer(addr: String, expectedPublicKey: Data?) async throws -> String

    /// The core's canonical pinned-peer set (PROTOCOL.md §4.5), ordered by
    /// pairing time. Re-read after every successful pairing and after
    /// `revokePeer` — the shell keeps no trust mirror of its own.
    func trustedPeers() -> [TrustedPeerInfo]

    /// 6-word fingerprint phrase for a raw 32-byte key, computed by the core.
    /// The shell never reimplements this derivation. Returns nil if the key is
    /// not 32 bytes.
    func fingerprintPhrase(forPublicKey publicKey: Data) -> String?

    /// DeviceID for a raw 32-byte key, computed by the core. Matches the
    /// `peerID` carried by every event.
    func deviceID(forPublicKey publicKey: Data) -> String?

    func send(peerID: String, urls: [URL]) -> TransferID
    func respondToOffer(transferID: TransferID, acceptedFileIDs: [UInt32])
    func cancel(transferID: TransferID)

    /// Drops a pinned key (PROTOCOL.md §4.5). No-op when the key is unknown.
    func revokePeer(publicKey: Data)
}
