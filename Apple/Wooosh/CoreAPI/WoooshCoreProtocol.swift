import Foundation
import UniformTypeIdentifiers

// MARK: - Core ↔ shell API contract (DESIGN.md §4)
// Views and stores talk only to this protocol, never to the generated bindings.

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

/// Supplied by the core with `fileReady`; the shell may override (DESIGN.md §6).
enum FileKind: String, Sendable {
    case photo
    case video
    case document

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

/// `id` is the core's DeviceID: BLAKE3(pubkey)[0..16], base32.
struct PeerRef: Hashable, Identifiable, Sendable {
    let id: String
    var displayName: String
    /// `nil` is "not reported": fall back to a neutral icon, never invent one.
    var deviceType: DeviceType?
    /// 6-word phrase (PROTOCOL.md §2), computed by the core.
    var fingerprint: String
    var publicKey: Data?

    init(id: String, displayName: String, deviceType: DeviceType?,
         fingerprint: String, publicKey: Data? = nil) {
        self.id = id
        self.displayName = displayName
        self.deviceType = deviceType
        self.fingerprint = fingerprint
        self.publicKey = publicKey
    }

    /// The core's enum cannot tell an iPhone from a Pixel (PROTOCOL.md §3.1).
    var symbolName: String { DeviceIcon.symbol(forCoreType: deviceType) }
}

/// Parsed without dialling (PROTOCOL.md §9.2). `publicKey` pins the handshake:
/// arriving out of band is what makes the internet path MITM-proof.
struct TicketInfo: Sendable {
    let publicKey: Data
    let deviceID: String
    let deviceName: String?
    /// nil when the ticket carries direct addresses only.
    let relay: String?
    let expired: Bool
}

/// Straight from the core's trust store; the shell keeps none (PROTOCOL.md §4.5).
struct TrustedPeerInfo: Hashable, Identifiable, Sendable {
    let publicKey: Data
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
    let stagingDirectory: URL
    let trustStoreURL: URL
    /// nil = ephemeral port on all interfaces.
    let listenAddress: String?
}

/// The single source of UI truth for pairing and transfers (DESIGN.md §4).
enum CoreEvent: Sendable {
    /// How a peer never seen over mDNS enters the device list.
    case peerConnected(peer: PeerRef, trusted: Bool)
    case peerDisconnected(peerID: String)
    case pairingSAS(peer: PeerRef, sixDigits: String)
    /// `trusted` is the core's verdict (pinned key present), not a guess.
    case incomingOffer(transferID: TransferID, from: PeerRef, trusted: Bool, manifest: [FileMeta])
    case transferStarted(transferID: TransferID, peerID: String, outgoing: Bool, manifest: [FileMeta])
    case progress(transferID: TransferID, fileID: UInt32, bytes: Int64, totalBytes: Int64, rate: Double, eta: TimeInterval)
    /// Hash-verified in staging; the shell now routes and cleans up (DESIGN.md §6).
    case fileReady(transferID: TransferID, fileID: UInt32, stagedURL: URL, kind: FileKind)
    case transferDone(transferID: TransferID, summary: TransferSummary)
    case transferError(transferID: TransferID, message: String, resumable: Bool)
    /// Never silently re-pin: surface prominently and require re-pairing
    /// (PROTOCOL.md §4.5). `presented` is nil when no key was ever offered.
    case keyChanged(peer: PeerRef, expectedPublicKey: Data, presentedPublicKey: Data?)
    case pairingResult(peer: PeerRef, success: Bool, message: String?)
    /// **Not a pairing**: nothing is pinned, and it dies with the connection.
    case ticketRedeemed(peer: PeerRef)
}

@MainActor
protocol WoooshCore: AnyObject {
    /// Single-consumer.
    var events: AsyncStream<CoreEvent> { get }

    /// Async because it calls back into the host `KeyStore` on the calling
    /// thread, and a Keychain read can block or prompt. Never on the main actor.
    func start(config: CoreConfig) async throws
    /// Must be safe to call twice and must not deadlock the callback thread.
    func stop()

    /// Owned by the core (PROTOCOL.md §2). `nil` before `start`.
    var deviceID: String? { get }
    var fingerprintPhrase: String? { get }
    /// Bound "ip:port", for the mDNS TXT `p` field.
    var listenAddress: String? { get }

    func setVisibility(_ mode: Visibility)

    /// `wooosh-pair:1?...` payload; single-use token, 120 s expiry (§4.2).
    func beginPairingQR() -> String
    /// Resolves asynchronously through a `pairingResult` event.
    func pairWithQR(payload: String)
    /// Parsing stays in the core, and names the peer before the dial (§4.2).
    func peerHint(forPairingPayload payload: String) -> PeerRef?
    /// SAS numeric-comparison verdict from the user (PROTOCOL.md §4.3).
    func confirmSAS(peerID: String, accepted: Bool)
    /// Camera-less pairing: sends `PAIR_REQUEST {}` (PROTOCOL.md §4.3).
    func requestSASPairing(peerID: String)

    // MARK: - Internet path (DESIGN.md §9.1, PROTOCOL.md §9)

    /// The first call binds the iroh endpoint, waits up to ~15 s for a home relay,
    /// and is the first relay contact at all, so nothing calls it unasked.
    func beginInternetTicket() async throws -> String

    /// A ticket is a capability: it dies on leaving the screen, not at expiry.
    func endInternetTicket()

    /// Pins the minting key; resolves through `pairingResult` like `pairWithQR`.
    func redeemTicket(_ ticket: String)

    /// Names the device while the dial runs; nil for a non-Wooosh ticket.
    func peerHint(forTicket ticket: String) -> PeerRef?

    /// nil when the payload is not a ticket.
    func ticketInfo(for ticket: String) -> TicketInfo?

    /// `nil` is n0's public relays; `[]` means no relay and no address lookup, so
    /// nothing leaves the device except to addresses carried in a ticket; a list
    /// is advertised in this device's tickets. Throws on a malformed URL.
    func setRelayURLs(_ urls: [String]?) async throws

    /// nil `expectedPublicKey` is *not* "unpinned": the core falls back to
    /// resolving a pin by address, which only matches the same `ip:port`, so pass
    /// the key whenever the DeviceID is known. Throws `.KeyChanged` on mismatch.
    func connectPeer(addr: String, expectedPublicKey: Data?) async throws -> String

    /// Canonical; re-read after every pairing and after `revokePeer`.
    func trustedPeers() -> [TrustedPeerInfo]

    /// Derived by the core; the shell never reimplements it. nil unless 32 bytes.
    func fingerprintPhrase(forPublicKey publicKey: Data) -> String?

    func deviceID(forPublicKey publicKey: Data) -> String?

    func send(peerID: String, urls: [URL]) -> TransferID
    func respondToOffer(transferID: TransferID, acceptedFileIDs: [UInt32])
    func cancel(transferID: TransferID)

    /// Drops a pinned key (PROTOCOL.md §4.5). No-op when the key is unknown.
    func revokePeer(publicKey: Data)
}
