import Foundation
import os
import WoooshCoreFFI

/// Adapts the UniFFI-generated `WoooshCoreFFI.WoooshCore` to the app's
/// `WoooshCore` seam (DESIGN.md §4).
///
/// Threading contract: the Rust core delivers events on its own dedicated
/// callback thread. `EventBridge` does the only thing that is safe there —
/// a non-blocking `AsyncStream.yield` — and every translation into UI-facing
/// types happens afterwards on the main actor. Nothing in the callback path
/// takes a lock the UI holds, and nothing calls back into the core.
///
/// Blocking commands (`pair_with_qr`, `connect_peer` both `block_on` inside
/// the core) are dispatched off the main actor; the fast ones (`send`,
/// `respond_to_offer`, …) return immediately and are called inline.
@MainActor
final class RealCore: WoooshCore {
    let events: AsyncStream<CoreEvent>
    private let continuation: AsyncStream<CoreEvent>.Continuation

    private let ffi = WoooshCoreFFI.WoooshCore()
    private let keyStore = KeychainKeyStore()
    private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "core")

    private var started = false
    private var translateTask: Task<Void, Never>?
    /// Everything the shell has learned about a peer id, so events that carry
    /// only `peer_id` can still produce a labelled `PeerRef`.
    private var peerInfo: [String: PeerRef] = [:]
    /// Manifests of in-flight transfers, so `fileReady` can name its file.
    private var manifests: [String: [FileMeta]] = [:]

    private(set) var deviceID: String?
    private(set) var fingerprintPhrase: String?
    private(set) var listenAddress: String?

    init() {
        (events, continuation) = AsyncStream.makeStream(of: CoreEvent.self)
    }

    // MARK: - Lifecycle

    func start(config: CoreConfig) async throws {
        guard !started else { return }
        try FileManager.default.createDirectory(
            at: config.stagingDirectory, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(
            at: config.trustStoreURL.deletingLastPathComponent(),
            withIntermediateDirectories: true)

        let (rawEvents, rawContinuation) = AsyncStream.makeStream(of: WoooshCoreFFI.CoreEvent.self)
        let bridge = EventBridge(continuation: rawContinuation)

        let ffiConfig = WoooshCoreFFI.Config(
            deviceName: config.displayName,
            deviceType: config.deviceType.ffi,
            visibility: config.visibility.ffi,
            stagingDir: config.stagingDirectory.path,
            trustStorePath: config.trustStoreURL.path,
            listenAddr: config.listenAddress
        )
        // Off the main actor: `start` binds the QUIC socket and invokes the
        // host KeyStore synchronously on the calling thread. A Keychain read
        // there is enough to freeze the UI (observed: SecItemCopyMatching
        // putting an access prompt up with the main thread inside it).
        let ffi = self.ffi
        let keyStore = self.keyStore
        try await Task.detached(priority: .userInitiated) {
            try ffi.start(config: ffiConfig, keyStore: keyStore, listener: bridge)
        }.value
        started = true

        deviceID = try? ffi.deviceId()
        fingerprintPhrase = try? ffi.fingerprintPhrase()
        listenAddress = try? ffi.listenAddr()
        if keyStore.usedFallback {
            logger.error("Identity is in the file fallback store, not the Keychain (unsigned build?)")
        }
        logger.info("core started: \(self.deviceID ?? "?") on \(self.listenAddress ?? "?")")

        translateTask = Task { [weak self] in
            for await raw in rawEvents {
                guard let self, !Task.isCancelled else { return }
                self.translate(raw)
            }
        }
    }

    func stop() {
        guard started else { return }
        started = false
        translateTask?.cancel()
        translateTask = nil
        // Synchronous by design: the core joins its callback thread here, and
        // returning before that is done would let the tokio runtime be dropped
        // from under an in-flight callback.
        ffi.stop()
    }

    // MARK: - Configuration

    func setVisibility(_ mode: Visibility) {
        perform("setVisibility") { try self.ffi.setVisibility(mode: mode.ffi) }
    }

    // MARK: - Pairing

    func beginPairingQR() -> String {
        (try? ffi.beginPairingQr()) ?? ""
    }

    /// Parse-only: the QR is the only place the shell ever legitimately learns
    /// a peer's public key (and its name, which the progress UI shows while
    /// the connection is being made).
    @discardableResult
    func peerHint(forPairingPayload payload: String) -> PeerRef? {
        guard let info = try? WoooshCoreFFI.parsePairingQr(payload: payload) else { return nil }
        let ref = PeerRef(
            id: info.deviceId,
            displayName: info.deviceName ?? "Scanned Device",
            deviceType: nil,
            fingerprint: fingerprintPhrase(forPublicKey: info.pubkey) ?? "",
            publicKey: info.pubkey
        )
        peerInfo[info.deviceId] = ref
        return ref
    }

    func pairWithQR(payload: String) {
        let hinted = peerHint(forPairingPayload: payload)
        let ffi = self.ffi
        Task.detached(priority: .userInitiated) {
            do {
                // Success arrives as the core's own PairingResult event on
                // PAIR_ACCEPT; only the failure needs synthesizing here.
                _ = try ffi.pairWithQr(payload: payload)
            } catch {
                let message = coreErrorMessage(error)
                await MainActor.run { [weak self] in
                    guard let self else { return }
                    let peer = hinted ?? PeerRef(id: "", displayName: "Scanned Device",
                                                 deviceType: nil, fingerprint: "")
                    self.emit(.pairingResult(peer: peer, success: false, message: message))
                }
            }
        }
    }

    func confirmSAS(peerID: String, accepted: Bool) {
        perform("confirmSAS") { try self.ffi.confirmSas(peerId: peerID, accepted: accepted) }
    }

    func requestSASPairing(peerID: String) {
        do {
            try ffi.requestSasPairing(peerId: peerID)
        } catch {
            let peer = peerRef(for: peerID)
            emit(.pairingResult(peer: peer, success: false, message: coreErrorMessage(error)))
        }
    }

    // MARK: - Internet path (PROTOCOL.md §9)

    func beginInternetTicket() async throws -> String {
        let ffi = self.ffi
        // Off the main actor: binds the iroh endpoint and blocks on a home
        // relay for as long as ~15 s.
        return try await Task.detached(priority: .userInitiated) {
            try ffi.beginInternetTicket()
        }.value
    }

    func endInternetTicket() {
        perform("endInternetTicket") { try self.ffi.endInternetTicket() }
    }

    func setRelayURLs(_ urls: [String]?) async throws {
        let ffi = self.ffi
        // Off the main actor: closing the bound iroh endpoint is an
        // asynchronous shutdown the core drives with `block_on`.
        try await Task.detached(priority: .userInitiated) {
            try ffi.setRelayUrls(urls: urls)
        }.value
    }

    func ticketInfo(for ticket: String) -> TicketInfo? {
        guard let info = try? WoooshCoreFFI.parseInternetTicket(ticket: ticket) else { return nil }
        return TicketInfo(
            publicKey: info.nodeId,
            deviceID: info.deviceId,
            deviceName: info.deviceName,
            relay: info.relay,
            expired: info.expired
        )
    }

    /// Parse-only, the ticket twin of `peerHint(forPairingPayload:)`: a ticket
    /// is the other place the shell legitimately learns a peer's public key.
    @discardableResult
    func peerHint(forTicket ticket: String) -> PeerRef? {
        guard let info = ticketInfo(for: ticket) else { return nil }
        let ref = PeerRef(
            id: info.deviceID,
            displayName: info.deviceName ?? "Scanned Device",
            deviceType: nil,
            fingerprint: fingerprintPhrase(forPublicKey: info.publicKey) ?? "",
            publicKey: info.publicKey
        )
        peerInfo[info.deviceID] = ref
        return ref
    }

    func redeemTicket(_ ticket: String) {
        let hinted = peerHint(forTicket: ticket)
        let ffi = self.ffi
        Task.detached(priority: .userInitiated) {
            do {
                // Success arrives as the core's own PairingResult, same as the
                // LAN QR path; only the failure needs synthesizing here.
                _ = try ffi.redeemTicket(ticket: ticket)
            } catch {
                let message = coreErrorMessage(error)
                await MainActor.run { [weak self] in
                    guard let self else { return }
                    let peer = hinted ?? PeerRef(id: "", displayName: "Scanned Device",
                                                 deviceType: nil, fingerprint: "")
                    self.emit(.pairingResult(peer: peer, success: false, message: message))
                }
            }
        }
    }

    // MARK: - Connections

    func connectPeer(addr: String, expectedPublicKey: Data?) async throws -> String {
        let ffi = self.ffi
        return try await Task.detached(priority: .userInitiated) {
            try ffi.connectPeer(addr: addr, expectedPubkey: expectedPublicKey)
        }.value
    }

    // MARK: - Transfers

    func send(peerID: String, urls: [URL]) -> TransferID {
        do {
            let raw = try ffi.send(peerId: peerID, files: urls.map(\.path))
            return TransferID(raw: raw)
        } catch {
            // Keep the UI's contract (a transfer row always appears) and fail
            // it through the event stream like any other transfer error.
            let tid = TransferID(raw: UUID().uuidString)
            let message = coreErrorMessage(error)
            Task { [weak self] in
                self?.emit(.transferError(transferID: tid, message: message, resumable: false))
            }
            return tid
        }
    }

    func respondToOffer(transferID: TransferID, acceptedFileIDs: [UInt32]) {
        perform("respondToOffer") {
            try self.ffi.respondToOffer(transferId: transferID.raw, acceptedFileIds: acceptedFileIDs)
        }
    }

    func cancel(transferID: TransferID) {
        perform("cancel") { try self.ffi.cancel(transferId: transferID.raw, fileId: nil) }
    }

    func revokePeer(publicKey: Data) {
        perform("revokePeer") { _ = try self.ffi.revokePeer(pubkey: publicKey) }
    }

    // MARK: - Trust store (canonical, read-through)

    func trustedPeers() -> [TrustedPeerInfo] {
        do {
            return try ffi.trustedPeers().map(TrustedPeerInfo.init(ffi:))
        } catch {
            logger.error("trustedPeers failed: \(coreErrorMessage(error))")
            return []
        }
    }

    func fingerprintPhrase(forPublicKey publicKey: Data) -> String? {
        try? WoooshCoreFFI.fingerprintPhraseFor(pubkey: publicKey)
    }

    func deviceID(forPublicKey publicKey: Data) -> String? {
        try? WoooshCoreFFI.deviceIdFor(pubkey: publicKey)
    }

    // MARK: - Event translation (main actor)

    private func translate(_ raw: WoooshCoreFFI.CoreEvent) {
        switch raw {
        case .peerConnected(let peerId, let peerPubkey, let deviceName, let deviceType,
                            let fingerprint, let trusted):
            let ref = PeerRef(
                id: peerId,
                displayName: deviceName.isEmpty ? (peerInfo[peerId]?.displayName ?? "Unknown Device") : deviceName,
                deviceType: deviceType?.appType ?? peerInfo[peerId]?.deviceType,
                fingerprint: fingerprint,
                publicKey: peerPubkey
            )
            peerInfo[peerId] = ref
            emit(.peerConnected(peer: ref, trusted: trusted))

        case .peerDisconnected(let peerId):
            emit(.peerDisconnected(peerID: peerId))

        case .pairingSas(let peerId, let code):
            emit(.pairingSAS(peer: peerRef(for: peerId), sixDigits: code))

        case .ticketRedeemed(let peerId, let peerPubkey, let deviceName):
            let ref = PeerRef(
                id: peerId,
                displayName: deviceName.isEmpty ? L.t("peer_unnamed") : deviceName,
                deviceType: peerInfo[peerId]?.deviceType,
                fingerprint: fingerprintPhrase(forPublicKey: peerPubkey) ?? "",
                publicKey: peerPubkey
            )
            peerInfo[peerId] = ref
            emit(.ticketRedeemed(peer: ref))

        case .pairingResult(let peerId, let peerPubkey, let fingerprint,
                            let success, let message):
            var ref = peerRef(for: peerId)
            ref.fingerprint = fingerprint
            ref.publicKey = peerPubkey
            // On success the core puts the peer's device name in `message`.
            if success, let name = message, !name.isEmpty {
                ref.displayName = name
            }
            peerInfo[peerId] = ref
            emit(.pairingResult(peer: ref, success: success,
                                message: success ? nil : message))

        case .incomingOffer(let transferId, let peerId, let peerPubkey, let fromName,
                            let deviceType, let trusted, let fingerprint, let files, _):
            let ref = PeerRef(
                id: peerId,
                displayName: fromName,
                deviceType: deviceType?.appType ?? peerInfo[peerId]?.deviceType,
                fingerprint: fingerprint,
                publicKey: peerPubkey
            )
            peerInfo[peerId] = ref
            let manifest = files.map(FileMeta.init(ffi:))
            manifests[transferId] = manifest
            emit(.incomingOffer(transferID: TransferID(raw: transferId), from: ref,
                                trusted: trusted, manifest: manifest))

        case .transferStarted(let transferId, let peerId, let direction, let files, _):
            let manifest = files.map(FileMeta.init(ffi:))
            manifests[transferId] = manifest
            emit(.transferStarted(transferID: TransferID(raw: transferId), peerID: peerId,
                                  outgoing: direction == .send, manifest: manifest))

        case .progress(let transferId, let fileId, let bytesDone, let totalBytes,
                       let rateBps, let etaSecs):
            emit(.progress(transferID: TransferID(raw: transferId), fileID: fileId,
                           bytes: Int64(clamping: bytesDone),
                           totalBytes: Int64(clamping: totalBytes),
                           rate: Double(rateBps), eta: TimeInterval(etaSecs)))

        case .fileReady(let transferId, let fileId, let stagedPath, let kind):
            emit(.fileReady(transferID: TransferID(raw: transferId), fileID: fileId,
                            stagedURL: URL(fileURLWithPath: stagedPath), kind: kind.appKind))

        case .transferDone(let transferId, let okFiles, _, let bytesTransferred, let durationMs):
            manifests[transferId] = nil
            emit(.transferDone(
                transferID: TransferID(raw: transferId),
                summary: TransferSummary(fileCount: Int(okFiles),
                                         totalBytes: Int64(clamping: bytesTransferred),
                                         duration: Double(durationMs) / 1000)))

        case .transferError(let transferId, let error, let resumable):
            emit(.transferError(transferID: TransferID(raw: transferId),
                                message: error, resumable: resumable))

        case .keyChanged(let peerId, let expectedPubkey, let presentedPubkey):
            var ref = peerRef(for: peerId)
            // The pin is what identifies the peer the user trusted; the
            // presented key belongs to whoever answered.
            ref.publicKey = expectedPubkey
            ref.fingerprint = fingerprintPhrase(forPublicKey: expectedPubkey) ?? ref.fingerprint
            emit(.keyChanged(peer: ref, expectedPublicKey: expectedPubkey,
                             presentedPublicKey: presentedPubkey))
        }
    }

    /// Best label the shell can produce for a bare `peer_id`. Falls back to
    /// the core's own trust store rather than deriving anything locally.
    private func peerRef(for peerID: String) -> PeerRef {
        if let known = peerInfo[peerID] { return known }
        if let pinned = trustedPeers().first(where: { $0.deviceID == peerID }) {
            let ref = PeerRef(
                id: peerID,
                displayName: pinned.displayName,
                deviceType: pinned.deviceType,
                fingerprint: pinned.fingerprint,
                publicKey: pinned.publicKey
            )
            peerInfo[peerID] = ref
            return ref
        }
        let ref = PeerRef(id: peerID, displayName: "Unknown Device",
                          deviceType: nil, fingerprint: "")
        peerInfo[peerID] = ref
        return ref
    }

    private func emit(_ event: CoreEvent) {
        continuation.yield(event)
    }

    private func perform(_ label: String, _ body: () throws -> Void) {
        do {
            try body()
        } catch {
            logger.error("\(label) failed: \(coreErrorMessage(error))")
        }
    }
}

// MARK: - Event bridge (core thread → main actor)

/// Runs on the core's dedicated callback thread. The only work it does is a
/// non-blocking hand-off; blocking here would stall every subsequent event.
private final class EventBridge: WoooshCoreFFI.CoreEventListener, @unchecked Sendable {
    private let continuation: AsyncStream<WoooshCoreFFI.CoreEvent>.Continuation

    init(continuation: AsyncStream<WoooshCoreFFI.CoreEvent>.Continuation) {
        self.continuation = continuation
    }

    func onEvent(event: WoooshCoreFFI.CoreEvent) {
        continuation.yield(event)
    }
}

// MARK: - Type bridging

private extension FileMeta {
    init(ffi: WoooshCoreFFI.OfferedFile) {
        self.init(id: ffi.fid, name: ffi.name,
                  size: Int64(clamping: ffi.size), mime: ffi.mime)
    }
}

private extension WoooshCoreFFI.FileKind {
    var appKind: FileKind {
        switch self {
        case .photo: .photo
        case .video: .video
        case .document: .document
        }
    }
}

private extension TrustedPeerInfo {
    init(ffi: WoooshCoreFFI.TrustedPeer) {
        self.init(
            publicKey: ffi.pubkey,
            deviceID: ffi.deviceId,
            displayName: ffi.deviceName,
            deviceType: ffi.deviceType?.appType,
            fingerprint: ffi.fingerprint,
            pairedAt: Date(timeIntervalSince1970: TimeInterval(ffi.pairedAt)),
            lastSeen: Date(timeIntervalSince1970: TimeInterval(ffi.lastSeen))
        )
    }
}

extension WoooshCoreFFI.DeviceType {
    var appType: DeviceType {
        switch self {
        case .phone: .phone
        case .tablet: .tablet
        case .laptop: .laptop
        case .desktop: .desktop
        }
    }
}

extension DeviceType {
    var ffi: WoooshCoreFFI.DeviceType {
        switch self {
        case .phone: .phone
        case .tablet: .tablet
        case .laptop: .laptop
        case .desktop: .desktop
        }
    }
}

extension Visibility {
    var ffi: WoooshCoreFFI.Visibility {
        switch self {
        case .everyone: .everyone
        case .pairedOnly: .pairedOnly
        case .off: .off
        }
    }
}

// MARK: - Errors

/// Maps the core's typed errors to user-facing text. Two reasons this mapping
/// exists: reporting a close code such as `PAIRING_REQUIRED` (PROTOCOL.md
/// §4.1.1) as a generic transport failure is a conformance bug, and the core's
/// own messages are untranslatable internal English that must never reach the
/// screen. The raw text stays in the log.
private enum RelayLimit {
    /// Formatted once: the value is a compile-time constant in the core.
    static let text = ByteCountFormatter.string(
        fromByteCount: Int64(clamping: WoooshCoreFFI.relayMaxFileBytes()),
        countStyle: .file
    )
}

func coreErrorMessage(_ error: Error) -> String {
    guard let error = error as? WoooshCoreFFI.WoooshError else {
        return L.t("error_transfer_failed")
    }
    switch error {
    case .PairingRequired: return L.t("error_pairing_required")
    case .VersionMismatch: return L.t("error_version_mismatch")
    case .KeyChanged: return L.t("error_key_changed")
    case .QrKeyMismatch: return L.t("error_qr_key_mismatch")
    case .InvalidQrPayload: return L.t("error_invalid_qr")
    case .Pairing(let message):
        return message.contains("expired") ? L.t("error_pairing_expired") : L.t("error_pairing_failed")
    case .RelayFileTooLarge:
        // The limit comes from the core so the copy cannot drift from the rule.
        return L.f("error_relay_file_too_large", RelayLimit.text)
    case .Connect: return L.t("error_connect")
    case .UnknownPeer: return L.t("error_unknown_peer")
    case .NotStarted: return L.t("error_not_started")
    case .AlreadyStarted: return L.t("error_already_started")
    case .Crypto: return L.t("error_crypto")
    case .Io: return L.t("error_io")
    case .Transfer(let message), .Protocol(let message), .InvalidArgument(let message):
        return transferErrorMessage(message)
    case .UnknownTransfer: return L.t("error_unknown_transfer")
    }
}

/// The core reports transfer outcomes as short English tokens, and they are the
/// only description of why a transfer stopped. Recognised here and answered
/// with real copy rather than shown raw.
func transferErrorMessage(_ raw: String) -> String {
    let text = raw.lowercased()
    if text.contains("declined") || text.contains("rejected") {
        return L.t("error_declined_by_peer")
    }
    if text.contains("cancelled by peer") || text.contains("canceled by peer") {
        return L.t("error_cancelled_by_peer")
    }
    if text.contains("cancel") { return L.t("transfer_state_cancelled") }
    if text.contains("timed out") || text.contains("timeout") { return L.t("error_no_answer") }
    return L.t("error_transfer_failed")
}
