import Foundation
import Observation
import os

#if os(iOS)
import UIKit
#endif

/// An in-flight SAS numeric-comparison exchange (PROTOCOL.md §4.3).
struct SASRequest: Identifiable, Equatable {
    let peer: PeerRef
    let sixDigits: String
    let startedAt: Date
    var id: String { peer.id + sixDigits }
}

/// Progress of the one pairing attempt that can be in flight, whichever entry
/// point started it (scanned/pasted QR, or a SAS request from a list row).
///
/// Pairing crosses a network and can take many seconds or never finish, so
/// every path into it must land in `connecting` first, name the peer, and offer
/// a way out. A silent wait reads as a hung app and users force-quit.
enum PairingPhase: Equatable {
    case idle
    /// `nil` name means the copy must stand on its own; never splice a
    /// placeholder noun into a translated sentence.
    case connecting(peerName: String?)
    case success(PeerRef)
    case failed(String)

    var isConnecting: Bool {
        if case .connecting = self { return true }
        return false
    }

    var failureMessage: String? {
        if case .failed(let message) = self { return message }
        return nil
    }
}

/// A pinned peer presenting a different key (PROTOCOL.md §4.5). Both
/// fingerprints are carried so the warning is concrete rather than "something
/// changed"; the phrases come from the core.
struct KeyChangeWarning: Identifiable, Equatable {
    let peer: PeerRef
    /// Phrase for the key the user actually paired with.
    let expectedFingerprint: String
    /// Phrase for the key that just answered; nil if none was presented.
    let presentedFingerprint: String?

    var id: String { peer.id }
}

/// Files handed over by the share extension, pre-armed for one-tap send.
struct ShareBatch: Identifiable, Equatable {
    let id: String
    let urls: [URL]
}

@MainActor
@Observable
final class AppModel {
    let registry = PeerRegistry()
    let trustStore = TrustStore()
    let transfers = TransferCenter()

    @ObservationIgnored
    private(set) var core: any WoooshCore = RealCore()

    /// Identity, as reported by the core — the single source (PROTOCOL.md §2).
    private(set) var deviceIDString: String = L.t("settings_starting")
    private(set) var fingerprintPhrase: String = ""
    private(set) var listenAddress: String = ""
    /// Set when starting the core failed; surfaced in Settings.
    private(set) var startupError: String?

    var activeSAS: SASRequest?
    var sasConfirming = false
    var pairingPhase: PairingPhase = .idle
    /// A pinned peer presented a different key — prominent alert
    /// (PROTOCOL.md §4.5).
    var keyChangeWarning: KeyChangeWarning?

    /// Reason the last send attempt never produced a transfer (usually a
    /// connect failure — PAIRING_REQUIRED, unreachable, …).
    var lastSendError: String?

    /// Batch staged by the share extension, awaiting a device tap.
    var pendingShareBatch: ShareBatch?

    @ObservationIgnored
    private var discovery: Discovery?
    @ObservationIgnored
    private var advertiseRestartTask: Task<Void, Never>?
    @ObservationIgnored
    private var eventPumpTask: Task<Void, Never>?
    @ObservationIgnored
    private var pairingTimeoutTask: Task<Void, Never>?
    /// Bumped whenever an attempt starts or is abandoned, so a result that
    /// arrives after the user gave up cannot reopen the UI.
    @ObservationIgnored
    private var pairingGeneration = 0

    /// Client-side ceiling on a pairing attempt. The core has its own
    /// per-hint timeouts, but the UI must never depend on them: if no event
    /// ever arrives, this is what stops the spinner from spinning forever.
    static let pairingTimeout: Duration = .seconds(30)
    @ObservationIgnored
    private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "app")

    var displayName: String {
        didSet {
            guard displayName != oldValue else { return }
            UserDefaults.standard.set(displayName, forKey: Self.displayNameKey)
            scheduleAdvertiseRestart()
        }
    }

    var visibility: Visibility {
        didSet {
            guard visibility != oldValue else { return }
            UserDefaults.standard.set(visibility.rawValue, forKey: Self.visibilityKey)
            core.setVisibility(visibility)
            restartAdvertising()
        }
    }

    private static let displayNameKey = "displayName"
    private static let visibilityKey = "visibility"

    init() {
        let defaults = UserDefaults.standard
        displayName = defaults.string(forKey: Self.displayNameKey) ?? Self.defaultDisplayName
        visibility = defaults.string(forKey: Self.visibilityKey)
            .flatMap(Visibility.init(rawValue:)) ?? .everyone
    }

    // MARK: - Lifecycle

    func start() async {
        guard discovery == nil else { return }
        await startCore()
        let discovery = Discovery(registry: registry)
        self.discovery = discovery
        discovery.startBrowsing()
        restartAdvertising()
    }

    /// Explicit refresh: the only way (besides relaunch) that rows leave the list.
    func refresh() {
        registry.clear()
        discovery?.startBrowsing()
    }

    /// Called on app termination. Skipping it leaves the tokio runtime to be
    /// dropped from `deinit` at an arbitrary point, which is how FFI shutdowns
    /// hang.
    func shutdown() {
        eventPumpTask?.cancel()
        eventPumpTask = nil
        discovery?.stopAdvertising()
        discovery?.stopBrowsing()
        core.stop()
    }

    private func startCore() async {
        do {
            try await core.start(config: CoreConfig(
                displayName: displayName,
                deviceType: .current,
                visibility: visibility,
                stagingDirectory: Self.stagingDirectory,
                trustStoreURL: Self.trustStoreURL,
                listenAddress: nil
            ))
            startupError = nil
        } catch {
            startupError = coreErrorMessage(error)
            logger.error("Core start failed: \(self.startupError ?? "")")
        }
        deviceIDString = core.deviceID ?? L.t("settings_starting")
        fingerprintPhrase = core.fingerprintPhrase ?? ""
        listenAddress = core.listenAddress ?? ""
        transfers.attach(core: core)
        trustStore.attach(core: core)
        startEventPump()
    }

    // MARK: - Core event pump

    private func startEventPump() {
        eventPumpTask?.cancel()
        let events = core.events
        eventPumpTask = Task { [weak self] in
            for await event in events {
                guard let self, !Task.isCancelled else { return }
                self.handle(event: event)
            }
        }
    }

    private func handle(event: CoreEvent) {
        switch event {
        case .peerConnected(let peer, let trusted):
            registry.connected(peerID: peer.id, displayName: peer.displayName,
                               deviceType: peer.deviceType, trusted: trusted)
        case .peerDisconnected(let peerID):
            registry.disconnected(peerID: peerID)
        case .pairingSAS(let peer, let sixDigits):
            sasConfirming = false
            // The SAS sheet takes over from the connecting spinner.
            pairingTimeoutTask?.cancel()
            pairingTimeoutTask = nil
            if pairingPhase.isConnecting { pairingPhase = .idle }
            activeSAS = SASRequest(peer: peer, sixDigits: sixDigits, startedAt: .now)
        case .pairingResult(let peer, let success, let message):
            pairingTimeoutTask?.cancel()
            pairingTimeoutTask = nil
            if success {
                // Re-read the core's trust.json rather than mirroring the event
                // shell-side. Runs even for an abandoned attempt: the pin is
                // real either way.
                trustStore.refresh()
                registry.connected(peerID: peer.id, displayName: peer.displayName,
                                   deviceType: peer.deviceType, trusted: true)
                pairingPhase = .success(peer)
            } else if pairingPhase != .idle {
                // Only surface a failure the user is still waiting on; a late
                // result for a cancelled attempt stays silent.
                pairingPhase = .failed(message ?? L.t("error_pairing_failed"))
            }
            if activeSAS?.peer.id == peer.id {
                activeSAS = nil
                sasConfirming = false
            }
        case .keyChanged(let peer, let expected, let presented):
            trustStore.markKeyChanged(deviceID: peer.id)
            keyChangeWarning = KeyChangeWarning(
                peer: peer,
                expectedFingerprint: core.fingerprintPhrase(forPublicKey: expected) ?? "",
                presentedFingerprint: presented.flatMap { core.fingerprintPhrase(forPublicKey: $0) }
            )
        default:
            transfers.handle(event: event)
        }
    }

    // MARK: - Pairing intents

    func beginPairingQR() -> String {
        core.beginPairingQR()
    }

    func pairWithQR(payload: String) {
        // Named from the payload so the first frame after the scan already
        // says who we are talking to.
        let name = core.peerHint(forPairingPayload: payload)?.displayName
        beginPairingAttempt(peerName: name)
        core.pairWithQR(payload: payload)
    }

    func requestSASPairing(with peer: Peer) {
        beginPairingAttempt(peerName: peer.displayName)
        let generation = pairingGeneration
        Task {
            do {
                // Blocking mDNS resolve + QUIC handshake, off the main actor
                // inside the core adapter, so the UI stays cancellable.
                let peerID = try await ensureConnection(to: peer)
                guard generation == pairingGeneration else { return }
                core.requestSASPairing(peerID: peerID)
            } catch {
                guard generation == pairingGeneration else { return }
                failPairing(coreErrorMessage(error))
            }
        }
    }

    private func beginPairingAttempt(peerName: String?) {
        pairingGeneration += 1
        let generation = pairingGeneration
        pairingPhase = .connecting(peerName: peerName)
        pairingTimeoutTask?.cancel()
        pairingTimeoutTask = Task { [weak self] in
            try? await Task.sleep(for: Self.pairingTimeout)
            guard !Task.isCancelled, let self, self.pairingGeneration == generation,
                  self.pairingPhase.isConnecting
            else { return }
            self.failPairing(L.t("error_pairing_timeout"))
        }
    }

    /// User-initiated abort. The core's pairing call is blocking and cannot be
    /// interrupted, so the attempt is disowned rather than killed: a result
    /// landing afterwards is ignored instead of reopening a screen the user
    /// already left.
    func cancelPairing() {
        pairingGeneration += 1
        pairingTimeoutTask?.cancel()
        pairingTimeoutTask = nil
        pairingPhase = .idle
        activeSAS = nil
        sasConfirming = false
    }

    func failPairing(_ message: String) {
        pairingTimeoutTask?.cancel()
        pairingTimeoutTask = nil
        pairingPhase = .failed(message)
    }

    /// Clears a finished attempt (dismissing a failure, reopening the sheet).
    /// Deliberately a no-op while an attempt is still in flight — resetting
    /// there is what would put the UI back to silence.
    func resetPairingPhase() {
        guard !pairingPhase.isConnecting else { return }
        pairingTimeoutTask?.cancel()
        pairingTimeoutTask = nil
        pairingPhase = .idle
    }

    func confirmSAS(accepted: Bool) {
        guard let sas = activeSAS else { return }
        guard accepted else {
            core.confirmSAS(peerID: sas.peer.id, accepted: false)
            activeSAS = nil
            sasConfirming = false
            resetPairingPhase()
            return
        }
        sasConfirming = true
        core.confirmSAS(peerID: sas.peer.id, accepted: true)
        // "Confirming…" is another wait on the other device, and must not hang
        // forever with no explanation either.
        pairingGeneration += 1
        let generation = pairingGeneration
        pairingTimeoutTask?.cancel()
        pairingTimeoutTask = Task { [weak self] in
            try? await Task.sleep(for: Self.pairingTimeout)
            guard !Task.isCancelled, let self, self.pairingGeneration == generation,
                  self.sasConfirming
            else { return }
            self.sasConfirming = false
            self.activeSAS = nil
            self.failPairing(L.t("error_sas_timeout"))
        }
    }

    /// Revokes the pin in the core and re-reads the resulting trust list.
    func revoke(device: TrustedPeerInfo) {
        core.revokePeer(publicKey: device.publicKey)
        trustStore.refresh()
        registry.setTrusted(false, forDeviceID: device.deviceID)
    }

    // MARK: - Connecting (DESIGN.md §4 `connect_peer`)

    /// Resolves a discovered row to an address and opens the QUIC connection,
    /// returning the core's peer id.
    ///
    /// The pin lookup is keyed by DeviceID against `trustedPeers()`, never by
    /// discovery id or display name. Passing the key matters: the core's own
    /// fallback resolves a pin from the address, which only matches a peer that
    /// came back on the same `ip:port`.
    @discardableResult
    func ensureConnection(to peer: Peer) async throws -> String {
        if let existing = peer.corePeerID { return existing }
        guard let address = await discovery?.address(forRID: peer.rid) else {
            throw ConnectionError.unresolved(peer.displayName)
        }
        let pinned = peer.knownDeviceID.flatMap { trustStore.publicKey(forDeviceID: $0) }
        logger.notice("""
            resolved \(peer.rid, privacy: .public) -> \(address, privacy: .public) \
            expectedPubkey=\(pinned == nil ? "none" : "pinned", privacy: .public) \
            deviceID=\(peer.knownDeviceID ?? "unknown", privacy: .public)
            """)
        let peerID = try await core.connectPeer(addr: address, expectedPublicKey: pinned)
        registry.attach(corePeerID: peerID, toRID: peer.rid,
                        trusted: trustStore.isPaired(deviceID: peerID))
        return peerID
    }

    enum ConnectionError: LocalizedError {
        case unresolved(String)

        var errorDescription: String? {
            switch self {
            case .unresolved(let name):
                L.f("error_unreachable", name)
            }
        }
    }

    /// Paired-ness is decided by the core's trust store, keyed by DeviceID —
    /// never by display name or discovery id. A row the shell has not yet
    /// connected to has no DeviceID, so it shows no checkmark until it does.
    func isPaired(_ peer: Peer) -> Bool {
        if let deviceID = peer.knownDeviceID, trustStore.isPaired(deviceID: deviceID) {
            return true
        }
        return peer.isTrusted
    }

    // MARK: - Sending

    func peerRef(for peer: Peer, peerID: String) -> PeerRef {
        let pinned = trustStore.device(forDeviceID: peerID)
        return PeerRef(
            id: peerID,
            displayName: peer.displayName,
            // Core-facing form factor only; the row's precise `deviceKind` has
            // no equivalent in the core's enum (PROTOCOL.md §3.1).
            deviceType: pinned?.deviceType ?? peer.coreDeviceType,
            fingerprint: pinned?.fingerprint ?? "",
            publicKey: pinned?.publicKey
        )
    }

    /// Copies picker-provided (possibly security-scoped) URLs into outgoing
    /// staging, connects if needed, then hands them to the core.
    func sendFiles(to peer: Peer, urls: [URL]) async -> Transfer? {
        let staged = stageOutgoing(urls: urls, securityScoped: true)
        guard !staged.isEmpty else {
            lastSendError = L.t("error_files_unreadable")
            return nil
        }
        return await send(to: peer, urls: staged)
    }

    #if os(iOS)
    /// Sends photos/videos imported from `PhotosPicker`. The URLs already carry
    /// the asset's original filename and staging preserves it verbatim, so the
    /// receiver sees "IMG_4021.HEIC" and not a name this app invented.
    func sendPickedMedia(to peer: Peer, urls: [URL]) async -> Transfer? {
        defer { PickedMediaFile.clearImports() }
        let staged = stageOutgoing(urls: urls, securityScoped: false)
        guard !staged.isEmpty else {
            lastSendError = L.t("error_files_unprepared")
            return nil
        }
        return await send(to: peer, urls: staged)
    }
    #endif

    /// Sends the pre-armed share batch to the tapped device.
    func sendPendingBatch(to peer: Peer) async -> Transfer? {
        guard let batch = pendingShareBatch else { return nil }
        pendingShareBatch = nil
        // The batch directory is left in place while the core reads from it;
        // stale batches are cleaned up on the next explicit discard.
        return await send(to: peer, urls: batch.urls)
    }

    private func send(to peer: Peer, urls: [URL]) async -> Transfer? {
        do {
            let peerID = try await ensureConnection(to: peer)
            lastSendError = nil
            return transfers.send(
                peer: peerRef(for: peer, peerID: peerID),
                paired: isPaired(peer),
                urls: urls
            )
        } catch {
            lastSendError = coreErrorMessage(error)
            logger.error("send to \(peer.displayName) failed: \(self.lastSendError ?? "")")
            return nil
        }
    }

    /// Copies one batch into a single staging directory, keeping every file's
    /// own name. One directory per batch (not per file) is what makes the
    /// " (2)" suffix kick in for two picked items with the same name — the
    /// receiver's collision policy, applied at the source.
    private func stageOutgoing(urls: [URL], securityScoped: Bool) -> [URL] {
        let dir = Self.outgoingDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return urls.compactMap { url in
            let accessing = securityScoped && url.startAccessingSecurityScopedResource()
            defer { if accessing { url.stopAccessingSecurityScopedResource() } }
            let destination = StorageRouter.uniqueDestination(for: url.lastPathComponent, in: dir)
            do {
                try FileManager.default.copyItem(at: url, to: destination)
                return destination
            } catch {
                logger.error("Failed to stage \(url.lastPathComponent): \(error)")
                return nil
            }
        }
    }

    // MARK: - Share extension handoff (DESIGN.md §8)

    /// Handles `wooosh://send?batch=<id>` from the share extension.
    func handleIncomingURL(_ url: URL) {
        guard url.scheme == AppGroup.urlScheme,
              url.host == "send",
              let batchID = URLComponents(url: url, resolvingAgainstBaseURL: false)?
                  .queryItems?.first(where: { $0.name == "batch" })?.value
        else { return }
        let files = AppGroup.stagedFiles(batchID: batchID)
        guard !files.isEmpty else {
            logger.error("Share batch \(batchID) is empty or unavailable")
            return
        }
        pendingShareBatch = ShareBatch(id: batchID, urls: files)
    }

    func discardPendingBatch() {
        if let batch = pendingShareBatch {
            AppGroup.removeBatch(id: batch.id)
        }
        pendingShareBatch = nil
    }

    // MARK: - Advertising

    private func restartAdvertising() {
        advertiseRestartTask?.cancel()
        guard let port = Self.port(of: core.listenAddress) else {
            logger.error("No core listen port yet; not advertising")
            return
        }
        listenAddress = core.listenAddress ?? ""
        discovery?.startAdvertising(
            displayName: displayName,
            deviceKind: .current,
            visibility: visibility,
            quicPort: port
        )
    }

    /// Debounced so per-keystroke display-name edits don't churn the record.
    private func scheduleAdvertiseRestart() {
        advertiseRestartTask?.cancel()
        advertiseRestartTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled else { return }
            self?.restartAdvertising()
        }
    }

    /// "0.0.0.0:52323" / "[::]:52323" → 52323.
    static func port(of address: String?) -> UInt16? {
        guard let address, let colon = address.lastIndex(of: ":") else { return nil }
        return UInt16(address[address.index(after: colon)...])
    }

    private static var defaultDisplayName: String {
        #if os(iOS)
        UIDevice.current.name
        #else
        Host.current().localizedName ?? "Mac"
        #endif
    }

    // MARK: - Directories

    /// Core-private staging for incoming files (verified before routing).
    /// Application Support, not Caches: a half-received 4 GB file must not be
    /// evictable mid-transfer, and the resume ledger lives here too.
    static var stagingDirectory: URL {
        supportSubdirectory("staging")
    }

    /// The core's canonical trust store (pinned peer keys).
    static var trustStoreURL: URL {
        supportSubdirectory(".").appendingPathComponent("trust.json")
    }

    /// Sender-side copies of picked files (security-scope-free).
    static var outgoingDirectory: URL {
        cachesSubdirectory("Outgoing")
    }

    private static func supportSubdirectory(_ name: String) -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        var dir = base.appendingPathComponent("Wooosh", isDirectory: true)
        if name != "." { dir = dir.appendingPathComponent(name, isDirectory: true) }
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private static func cachesSubdirectory(_ name: String) -> URL {
        let base = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        let dir = base.appendingPathComponent(name, isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }
}
