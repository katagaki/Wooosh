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

/// Every entry point must land in `connecting` first, named and cancellable.
enum PairingPhase: Equatable {
    case idle
    /// `nil` name: the copy must stand alone, never splice a noun into a translation.
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

/// Carries both fingerprints so the warning is concrete (PROTOCOL.md §4.5).
struct KeyChangeWarning: Identifiable, Equatable {
    let peer: PeerRef
    let expectedFingerprint: String
    let presentedFingerprint: String?

    var id: String { peer.id }
}

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

    /// Identity comes from the core, the single source (PROTOCOL.md §2).
    private(set) var deviceIDString: String = L.t("settings_starting")
    private(set) var fingerprintPhrase: String = ""
    private(set) var listenAddress: String = ""
    private(set) var startupError: String?

    var activeSAS: SASRequest?
    var sasConfirming = false
    var pairingPhase: PairingPhase = .idle
    var keyChangeWarning: KeyChangeWarning?

    /// Read once via `takeRedeemedPeerID()`.
    private(set) var redeemedPeerID: String?

    /// Session-scoped and never on disk: the internet path never pairs (§9.4).
    @ObservationIgnored
    private var ticketPeers: Set<String> = []

    private(set) var ticketRedeemedPeerID: String?

    /// Peer behind each running transfer, so a completion can be attributed.
    @ObservationIgnored
    private var transferPeerIDs: [TransferID: String] = [:]

    @ObservationIgnored
    private var internetOutbox: [URL] = []

    /// Only the ticket path opens a send sheet on success.
    @ObservationIgnored
    private var pairingIsTicket = false

    var lastSendError: String?

    var pendingShareBatch: ShareBatch?

    @ObservationIgnored
    private var discovery: Discovery?
    @ObservationIgnored
    private var advertiseRestartTask: Task<Void, Never>?
    @ObservationIgnored
    private var eventPumpTask: Task<Void, Never>?
    @ObservationIgnored
    private var pairingTimeoutTask: Task<Void, Never>?
    /// Bumped on start/abandon so a late result cannot reopen the UI.
    @ObservationIgnored
    private var pairingGeneration = 0

    /// The UI cannot depend on the core's timeouts; this stops the spinner.
    static let pairingTimeout: Duration = .seconds(30)

    /// ~30 s hole punching plus a 20 s PAIR_ACCEPT wait exceeds the LAN budget.
    static let ticketTimeout: Duration = .seconds(75)
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

    /// Pushed to the core: it picks the endpoint bound at the next mint (§9.1).
    var relayPreference: RelayPreference {
        didSet {
            guard relayPreference != oldValue else { return }
            UserDefaults.standard.set(relayPreference.mode.rawValue, forKey: Self.relayModeKey)
            UserDefaults.standard.set(relayPreference.customURL, forKey: Self.relayURLKey)
            applyRelayPreference()
        }
    }

    /// The core keeps its last working config, so this is a correction to make.
    private(set) var relayError: String?

    private static let displayNameKey = "displayName"
    private static let visibilityKey = "visibility"
    private static let relayModeKey = "relayMode"
    private static let relayURLKey = "relayURL"

    init() {
        let defaults = UserDefaults.standard
        displayName = defaults.string(forKey: Self.displayNameKey) ?? Self.defaultDisplayName
        // Paired only by default: a fresh install must not accept from strangers.
        visibility = defaults.string(forKey: Self.visibilityKey)
            .flatMap(Visibility.init(rawValue:)) ?? .pairedOnly
        relayPreference = RelayPreference(
            mode: defaults.string(forKey: Self.relayModeKey)
                .flatMap(RelayMode.init(rawValue:)) ?? RelayPreference.default.mode,
            customURL: defaults.string(forKey: Self.relayURLKey) ?? ""
        )
    }

    private func applyRelayPreference() {
        let value = relayPreference.coreValue
        Task {
            do {
                try await core.setRelayURLs(value)
                relayError = nil
            } catch {
                relayError = L.t("error_relay_url_invalid")
                logger.error("setRelayURLs failed: \(coreErrorMessage(error))")
            }
        }
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

    /// Must run on termination, or `deinit` drops the tokio runtime at an
    /// arbitrary point, which is how FFI shutdowns hang.
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
        // Free before anything mints a ticket: the iroh endpoint is unbound.
        applyRelayPreference()
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
            pairingTimeoutTask?.cancel()
            pairingTimeoutTask = nil
            if pairingPhase.isConnecting { pairingPhase = .idle }
            activeSAS = SASRequest(peer: peer, sixDigits: sixDigits, startedAt: .now)
        case .pairingResult(let peer, let success, let message):
            pairingTimeoutTask?.cancel()
            pairingTimeoutTask = nil
            if success {
                if pairingIsTicket { redeemedPeerID = peer.id }
                pairingIsTicket = false
                // Runs even for an abandoned attempt: the pin is real either way.
                trustStore.refresh()
                registry.connected(peerID: peer.id, displayName: peer.displayName,
                                   deviceType: peer.deviceType, trusted: true)
                pairingPhase = .success(peer)
            } else if pairingPhase != .idle {
                pairingPhase = .failed(message ?? L.t("error_pairing_failed"))
            }
            if activeSAS?.peer.id == peer.id {
                activeSAS = nil
                sasConfirming = false
            }
        case .ticketRedeemed(let peer):
            // Not a pairing: session-only, and it dies with the connection.
            ticketPeers.insert(peer.id)
            // `viaTicket`: an old pin would otherwise badge a row the ticket admitted.
            registry.connected(peerID: peer.id, displayName: peer.displayName,
                               deviceType: peer.deviceType, trusted: false,
                               viaTicket: true)
            ticketRedeemedPeerID = peer.id

        case .incomingOffer(let tid, let from, let trusted, let manifest) where ticketPeers.contains(from.id):
            // Scanning the code *was* the consent: forwarded consented, not trusted.
            transfers.handle(event: .incomingOffer(transferID: tid, from: from,
                                                   trusted: true, manifest: manifest))
            _ = trusted

        case .keyChanged(let peer, let expected, let presented):
            trustStore.markKeyChanged(deviceID: peer.id)
            keyChangeWarning = KeyChangeWarning(
                peer: peer,
                expectedFingerprint: core.fingerprintPhrase(forPublicKey: expected) ?? "",
                presentedFingerprint: presented.flatMap { core.fingerprintPhrase(forPublicKey: $0) }
            )
        case .transferStarted(let tid, let peerID, _, _):
            // `transferDone` does not name its peer; ticket rows need to know.
            transferPeerIDs[tid] = peerID
            transfers.handle(event: event)

        case .transferDone(let tid, _):
            endTicketSession(transferID: tid)
            transfers.handle(event: event)

        case .transferError(let tid, _, _):
            endTicketSession(transferID: tid)
            transfers.handle(event: event)

        default:
            transfers.handle(event: event)
        }
    }

    /// A ticket authorises one transfer, so the row dies when that transfer does.
    private func endTicketSession(transferID: TransferID) {
        guard let peerID = transferPeerIDs.removeValue(forKey: transferID) else { return }
        registry.ticketSessionEnded(peerID: peerID)
    }

    // MARK: - Pairing intents

    func beginPairingQR() -> String {
        core.beginPairingQR()
    }

    func pairWithQR(payload: String) {
        let name = core.peerHint(forPairingPayload: payload)?.displayName
        beginPairingAttempt(peerName: name)
        core.pairWithQR(payload: payload)
    }

    /// Payloads and tickets look alike to a camera, so the scheme picks the path.
    func pairWithScannedCode(_ code: String) {
        let trimmed = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("wooosh-net:") else {
            pairWithQR(payload: trimmed)
            return
        }
        // The code would work with the internet path on; say so, don't just fail.
        guard relayPreference.internetEnabled else {
            failPairing(L.t("error_internet_off"))
            return
        }
        redeemTicket(trimmed)
    }

    func redeemTicket(_ ticket: String) {
        let name = core.peerHint(forTicket: ticket)?.displayName
        beginPairingAttempt(peerName: name, timeout: Self.ticketTimeout)
        pairingIsTicket = true
        core.redeemTicket(ticket)
    }

    /// Reading clears: one redemption opens exactly one send sheet.
    func takeRedeemedPeerID() -> String? {
        defer { redeemedPeerID = nil }
        return redeemedPeerID
    }

    /// Throws: silence would not distinguish a slow relay from a broken one.
    func beginInternetTicket() async throws -> String {
        try await core.beginInternetTicket()
    }

    func endInternetTicket() {
        core.endInternetTicket()
        internetOutbox = []
        ticketRedeemedPeerID = nil
    }

    func stageInternetSend(urls: [URL]) -> Bool {
        internetOutbox = stageOutgoing(urls: urls, securityScoped: true)
        return !internetOutbox.isEmpty
    }

    /// The core refuses unless that peer really redeemed (PROTOCOL.md §9.4).
    @discardableResult
    func completeInternetSend(to peerID: String) -> Transfer? {
        guard !internetOutbox.isEmpty else { return nil }
        let urls = internetOutbox
        internetOutbox = []
        ticketRedeemedPeerID = nil
        let peer = PeerRef(id: peerID,
                           displayName: registry.peer(forDeviceID: peerID)?.displayName
                               ?? L.t("peer_unnamed"),
                           deviceType: nil, fingerprint: "")
        // `paired: false` is the truth: accept-once, with nothing pinned.
        return transfers.send(peer: peer, paired: false, urls: urls)
    }

    func requestSASPairing(with peer: Peer) {
        beginPairingAttempt(peerName: peer.displayName)
        let generation = pairingGeneration
        Task {
            do {
                // Blocking resolve + handshake; off the main actor in the adapter.
                let peerID = try await ensureConnection(to: peer)
                guard generation == pairingGeneration else { return }
                core.requestSASPairing(peerID: peerID)
            } catch {
                guard generation == pairingGeneration else { return }
                failPairing(coreErrorMessage(error))
            }
        }
    }

    private func beginPairingAttempt(peerName: String?, timeout: Duration = AppModel.pairingTimeout) {
        pairingIsTicket = false
        pairingGeneration += 1
        let generation = pairingGeneration
        pairingPhase = .connecting(peerName: peerName)
        pairingTimeoutTask?.cancel()
        pairingTimeoutTask = Task { [weak self] in
            try? await Task.sleep(for: timeout)
            guard !Task.isCancelled, let self, self.pairingGeneration == generation,
                  self.pairingPhase.isConnecting
            else { return }
            self.failPairing(L.t("error_pairing_timeout"))
        }
    }

    /// The core's pairing call cannot be interrupted, so the attempt is disowned.
    func cancelPairing() {
        pairingIsTicket = false
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

    /// A no-op mid-attempt: resetting there is what puts the UI back to silence.
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

    func revoke(device: TrustedPeerInfo) {
        core.revokePeer(publicKey: device.publicKey)
        trustStore.refresh()
        registry.setTrusted(false, forDeviceID: device.deviceID)
    }

    // MARK: - Connecting (DESIGN.md §4 `connect_peer`)

    /// Pass the key: the core's fallback resolves by address, matching only a
    /// peer back on the same `ip:port`.
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

    /// Keyed by DeviceID: a row not yet connected to shows no checkmark.
    func isPaired(_ peer: Peer) -> Bool {
        // Pinned or not, the ticket authorised this session and dies with it (§9).
        if peer.isTicketOnly { return false }
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
            // Core-facing form factor only; `deviceKind` has no core equivalent.
            deviceType: pinned?.deviceType ?? peer.coreDeviceType,
            fingerprint: pinned?.fingerprint ?? "",
            publicKey: pinned?.publicKey
        )
    }

    func sendFiles(to peer: Peer, urls: [URL]) async -> Transfer? {
        let staged = stageOutgoing(urls: urls, securityScoped: true)
        guard !staged.isEmpty else {
            lastSendError = L.t("error_files_unreadable")
            return nil
        }
        return await send(to: peer, urls: staged)
    }

    #if os(iOS)
    /// Picker URLs carry the original filename; staging preserves it verbatim.
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

    func sendPendingBatch(to peer: Peer) async -> Transfer? {
        guard let batch = pendingShareBatch else { return nil }
        pendingShareBatch = nil
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

    /// One directory per batch applies the receiver's " (2)" policy at the source.
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

    /// Application Support, not Caches: nothing here may be evicted mid-transfer.
    static var stagingDirectory: URL {
        supportSubdirectory("staging")
    }

    static var trustStoreURL: URL {
        supportSubdirectory(".").appendingPathComponent("trust.json")
    }

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
