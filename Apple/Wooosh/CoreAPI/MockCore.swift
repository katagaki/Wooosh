import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

// Scripted engine kept alive behind the DEBUG menu (DESIGN.md §11): the
// entire UI stays demoable with no peer on the network. `RealCore` is the
// default; this one only runs when the debug Engine picker selects it.
//
// Outgoing sends emit progress ticks then fileReady and transferDone; debug
// hooks simulate an incoming offer, an incoming SAS pairing request, and a
// KEY_CHANGED event; pairWithQR/confirmSAS succeed after a short delay.
@MainActor
final class MockCore: WoooshCore {

    let events: AsyncStream<CoreEvent>
    private let continuation: AsyncStream<CoreEvent>.Continuation

    private var config: CoreConfig?
    private var transferTasks: [TransferID: Task<Void, Never>] = [:]
    /// Peers for which a SAS exchange is in flight, so confirmSAS can resolve.
    private var pendingSASPeers: [String: PeerRef] = [:]
    /// Manifests of simulated incoming offers awaiting respondToOffer.
    private var pendingOffers: [TransferID: (peer: PeerRef, manifest: [FileMeta])] = [:]
    /// The mock's stand-in for the core's trust.json — same read-through
    /// contract, so the shell exercises the real code path.
    private var pinned: [String: TrustedPeerInfo] = [:]

    /// Wire-realistic-ish rate, deliberately slowed so short demo transfers
    /// remain visible in the UI. Mock — replaced by wooosh-core in Milestone 3.
    private let mockRate: Double = 24 * 1_024 * 1_024 // bytes/sec

    init() {
        (events, continuation) = AsyncStream.makeStream(of: CoreEvent.self)
    }

    // MARK: - Lifecycle

    func start(config: CoreConfig) async throws {
        self.config = config
        try FileManager.default.createDirectory(
            at: config.stagingDirectory, withIntermediateDirectories: true)
    }

    func stop() {
        for task in transferTasks.values { task.cancel() }
        transferTasks.removeAll()
        continuation.finish()
    }

    var deviceID: String? { "MOCK-DEV1-CE00-0000-0000-0000-00" }
    var fingerprintPhrase: String? { "mock demo core not a real device" }
    var listenAddress: String? { "127.0.0.1:0" }

    func setVisibility(_ mode: Visibility) {
        // Mock: discovery is still handled natively in Milestone 2 (Discovery.swift).
    }

    // MARK: - Pairing

    func beginPairingQR() -> String {
        // PROTOCOL.md §4.2 payload shape.
        let pk = Data((0..<32).map { _ in UInt8.random(in: .min ... .max) }).base64EncodedString()
        let token = Data((0..<32).map { _ in UInt8.random(in: .min ... .max) }).base64EncodedString()
        let exp = Int(Date().timeIntervalSince1970) + 120
        return "wooosh-pair:1?pk=\(pk)?tok=\(token)?hints=192.168.1.20:52323?exp=\(exp)"
    }

    func peerHint(forPairingPayload payload: String) -> PeerRef? {
        guard payload.hasPrefix("wooosh-pair:1?"), payload.contains("pk=") else { return nil }
        return Self.qrPeer
    }

    func pairWithQR(payload: String) {
        // Mock: accept any syntactically plausible payload after a delay long
        // enough that the "Connecting to…" state is actually observable.
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(2.5))
            guard let self else { return }
            guard payload.hasPrefix("wooosh-pair:1?"), payload.contains("pk=") else {
                self.emit(.pairingResult(peer: Self.qrPeer, success: false,
                                         message: "That code is not a valid Wooosh pairing code."))
                return
            }
            self.pin(Self.qrPeer)
            self.emit(.pairingResult(peer: Self.qrPeer, success: true, message: nil))
        }
    }

    func requestSASPairing(peerID: String) {
        let peer = PeerRef(
            id: peerID,
            displayName: knownPeerName(peerID) ?? "Nearby Device",
            deviceType: .laptop,
            fingerprint: MockCore.fingerprint(for: peerID),
            publicKey: MockCore.pubkey(for: peerID)
        )
        pendingSASPeers[peerID] = peer
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(0.8))
            self?.emit(.pairingSAS(peer: peer, sixDigits: String(format: "%06d", Int.random(in: 0..<1_000_000))))
        }
    }

    func confirmSAS(peerID: String, accepted: Bool) {
        let peer = pendingSASPeers.removeValue(forKey: peerID) ?? PeerRef(
            id: peerID, displayName: knownPeerName(peerID) ?? "Nearby Device",
            deviceType: .laptop, fingerprint: MockCore.fingerprint(for: peerID),
            publicKey: MockCore.pubkey(for: peerID))
        guard accepted else { return } // Abort: key NOT stored (PROTOCOL.md §4.3).
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(1.2))
            guard let self else { return }
            self.pin(peer)
            self.emit(.pairingResult(peer: peer, success: true, message: nil))
        }
    }

    func connectPeer(addr: String, expectedPublicKey: Data?) async throws -> String {
        // Mock: pretend the address resolved to a device we already know.
        try? await Task.sleep(for: .milliseconds(300))
        return "MOCK-PEER-\(abs(addr.hashValue) % 100_000)"
    }

    func revokePeer(publicKey: Data) {
        pinned = pinned.filter { $0.value.publicKey != publicKey }
    }

    // MARK: - Trust store

    func trustedPeers() -> [TrustedPeerInfo] {
        pinned.values.sorted {
            ($0.pairedAt, $0.deviceID) < ($1.pairedAt, $1.deviceID)
        }
    }

    func fingerprintPhrase(forPublicKey publicKey: Data) -> String? {
        guard publicKey.count == 32 else { return nil }
        return MockCore.fingerprint(for: publicKey.map { String(format: "%02x", $0) }.joined())
    }

    func deviceID(forPublicKey publicKey: Data) -> String? {
        guard publicKey.count == 32 else { return nil }
        return pinned.values.first { $0.publicKey == publicKey }?.deviceID
            ?? "MOCK-" + publicKey.prefix(4).map { String(format: "%02X", $0) }.joined()
    }

    private func pin(_ peer: PeerRef) {
        pinned[peer.id] = TrustedPeerInfo(
            publicKey: peer.publicKey ?? MockCore.pubkey(for: peer.id),
            deviceID: peer.id,
            displayName: peer.displayName,
            deviceType: peer.deviceType,
            fingerprint: peer.fingerprint,
            pairedAt: pinned[peer.id]?.pairedAt ?? .now,
            lastSeen: .now
        )
    }

    // MARK: - Transfers

    func send(peerID: String, urls: [URL]) -> TransferID {
        let tid = TransferID(raw: UUID().uuidString)
        let files: [(url: URL, size: Int64)] = urls.map { url in
            let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize).flatMap { Int64($0) } ?? 0
            return (url, max(size, 1))
        }
        transferTasks[tid] = Task { [weak self] in
            guard let self else { return }
            // Connection + OFFER/DECISION round trip.
            try? await Task.sleep(for: .seconds(0.7))
            let started = ContinuousClock.now
            var totalSent: Int64 = 0
            let grandTotal = files.reduce(Int64(0)) { $0 + $1.size }
            for (index, file) in files.enumerated() {
                let ok = await self.emitProgressTicks(
                    tid: tid, fileID: UInt32(index),
                    size: file.size, alreadySent: totalSent, grandTotal: grandTotal)
                guard ok else { return }
                totalSent += file.size
                // Sender-side per-file completion signal. Kind is advisory;
                // the shell only routes fileReady on *incoming* transfers.
                self.emit(.fileReady(transferID: tid, fileID: UInt32(index),
                                     stagedURL: file.url,
                                     kind: FileKind(filename: file.url.lastPathComponent)))
            }
            let elapsed = started.duration(to: .now)
            self.emit(.transferDone(transferID: tid, summary: TransferSummary(
                fileCount: files.count, totalBytes: grandTotal,
                duration: Double(elapsed.components.seconds) + Double(elapsed.components.attoseconds) / 1e18)))
            self.transferTasks[tid] = nil
        }
        return tid
    }

    func respondToOffer(transferID: TransferID, acceptedFileIDs: [UInt32]) {
        guard let offer = pendingOffers.removeValue(forKey: transferID) else { return }
        guard !acceptedFileIDs.isEmpty else { return } // DECISION with empty accept = decline.
        let accepted = offer.manifest.filter { acceptedFileIDs.contains($0.id) }
        transferTasks[transferID] = Task { [weak self] in
            guard let self else { return }
            try? await Task.sleep(for: .seconds(0.5))
            let started = ContinuousClock.now
            var totalReceived: Int64 = 0
            let grandTotal = accepted.reduce(Int64(0)) { $0 + $1.size }
            for meta in accepted {
                let ok = await self.emitProgressTicks(
                    tid: transferID, fileID: meta.id,
                    size: meta.size, alreadySent: totalReceived, grandTotal: grandTotal)
                guard ok else { return }
                totalReceived += meta.size
                // "Verified in staging" — write a real staged file, then hand
                // it to the shell for storage routing (DESIGN.md §6).
                if let staged = self.writeStagedFile(tid: transferID, meta: meta) {
                    self.emit(.fileReady(transferID: transferID, fileID: meta.id,
                                         stagedURL: staged,
                                         kind: FileKind(mime: meta.mime)))
                }
            }
            let elapsed = started.duration(to: .now)
            self.emit(.transferDone(transferID: transferID, summary: TransferSummary(
                fileCount: accepted.count, totalBytes: grandTotal,
                duration: Double(elapsed.components.seconds) + Double(elapsed.components.attoseconds) / 1e18)))
            self.transferTasks[transferID] = nil
        }
    }

    func cancel(transferID: TransferID) {
        transferTasks.removeValue(forKey: transferID)?.cancel()
        pendingOffers.removeValue(forKey: transferID)
    }

    /// Emits paced progress for one file. Returns false if cancelled.
    private func emitProgressTicks(
        tid: TransferID, fileID: UInt32, size: Int64,
        alreadySent: Int64, grandTotal: Int64
    ) async -> Bool {
        // Every file takes at least ~0.8 s so the demo UI is visible.
        let duration = max(Double(size) / mockRate, 0.8)
        let tickInterval = 0.12
        let ticks = max(Int(duration / tickInterval), 1)
        for tick in 1...ticks {
            try? await Task.sleep(for: .seconds(tickInterval))
            if Task.isCancelled { return false }
            let bytes = Int64(Double(size) * Double(tick) / Double(ticks))
            let jitteredRate = mockRate * Double.random(in: 0.82...1.12)
            let remaining = Double(grandTotal - alreadySent - bytes)
            emit(.progress(transferID: tid, fileID: fileID, bytes: bytes, totalBytes: size,
                           rate: jitteredRate, eta: max(remaining / jitteredRate, 0)))
        }
        return !Task.isCancelled
    }

    // MARK: - Debug simulation hooks (drive the demo; not part of WoooshCore)

    /// Simulates a nearby device offering files (incoming OFFER, PROTOCOL.md §5).
    /// `paired` controls whether the consent sheet shows the trusted or the
    /// fingerprint/Accept-once variant.
    func debugSimulateIncomingOffer(from peer: PeerRef? = nil) {
        let sender = peer ?? PeerRef(
            id: "mock-sender", displayName: "Aki's MacBook Pro",
            deviceType: .laptop, fingerprint: MockCore.fingerprint(for: "mock-sender"))
        let tid = TransferID(raw: UUID().uuidString)
        // No video item: the mock cannot fabricate a playable video file that
        // PHAssetCreationRequest would accept. Routing supports .video anyway.
        let manifest = [
            FileMeta(id: 0, name: "IMG_4283.png", size: 3_412_884, mime: "image/png"),
            FileMeta(id: 1, name: "IMG_4284.png", size: 2_108_339, mime: "image/png"),
            FileMeta(id: 2, name: "Quarterly Notes.pdf", size: 1_284_920, mime: "application/pdf"),
            FileMeta(id: 3, name: "Reading List.txt", size: 4_812, mime: "text/plain"),
        ]
        pendingOffers[tid] = (sender, manifest)
        emit(.incomingOffer(transferID: tid, from: sender, trusted: false, manifest: manifest))
    }

    /// Simulates a peer initiating SAS pairing with us (PROTOCOL.md §4.3).
    func debugSimulateIncomingSASRequest(from peer: PeerRef? = nil) {
        let requester = peer ?? PeerRef(
            id: "mock-sas-peer", displayName: "Yuki's iPhone",
            deviceType: .phone, fingerprint: MockCore.fingerprint(for: "mock-sas-peer"))
        pendingSASPeers[requester.id] = requester
        emit(.pairingSAS(peer: requester, sixDigits: String(format: "%06d", Int.random(in: 0..<1_000_000))))
    }

    /// Simulates a pinned peer presenting a different key (PROTOCOL.md §4.5).
    func debugSimulateKeyChanged(peer: PeerRef) {
        emit(.keyChanged(
            peer: peer,
            expectedPublicKey: peer.publicKey ?? MockCore.pubkey(for: peer.id),
            presentedPublicKey: MockCore.pubkey(for: peer.id + "-impostor")
        ))
    }

    // MARK: - Helpers

    private func emit(_ event: CoreEvent) {
        continuation.yield(event)
    }

    /// Hook for AppModel to give the mock better display names for real
    /// discovered peers (keyed by rid). Mock-only convenience.
    var knownPeerName: (String) -> String? = { _ in nil }

    private static let qrPeer = PeerRef(
        id: "mock-qr-peer", displayName: "Scanned Device",
        deviceType: .desktop, fingerprint: MockCore.fingerprint(for: "mock-qr-peer"),
        publicKey: MockCore.pubkey(for: "mock-qr-peer"))

    /// Deterministic 32-byte stand-in identity key, so the mock's trust list
    /// and revoke-by-key path behave like the core's.
    static func pubkey(for id: String) -> Data {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        return Data((0..<32).map { _ -> UInt8 in
            for byte in id.utf8 { hash = (hash ^ UInt64(byte)) &* 0x0000_0100_0000_01b3 }
            hash = hash &* 6364136223846793005 &+ 1442695040888963407
            return UInt8(truncatingIfNeeded: hash >> 24)
        })
    }

    /// Deterministic 6-word fingerprint phrase stand-in (PROTOCOL.md §2).
    static func fingerprint(for id: String) -> String {
        let words = ["amber", "birch", "coral", "delta", "ember", "fjord",
                     "grove", "haven", "inlet", "jasper", "kelp", "lumen",
                     "maple", "north", "opal", "pine", "quartz", "ridge",
                     "slate", "tide", "umber", "vale", "willow", "zephyr"]
        var hash: UInt64 = 5381
        for byte in id.utf8 { hash = hash &* 33 &+ UInt64(byte) }
        return (0..<6).map { index -> String in
            hash = hash &* 6364136223846793005 &+ 1442695040888963407 &+ UInt64(index)
            return words[Int(hash % UInt64(words.count))]
        }.joined(separator: "-")
    }

    /// Writes a plausible staged file for a simulated incoming transfer.
    private func writeStagedFile(tid: TransferID, meta: FileMeta) -> URL? {
        guard let staging = config?.stagingDirectory else { return nil }
        let dir = staging.appendingPathComponent(tid.raw, isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appendingPathComponent(meta.name)
        let data: Data
        if meta.mime.hasPrefix("image/") {
            data = Self.makePlaceholderPNG(seed: meta.id) ?? Data("png-generation-failed".utf8)
        } else if meta.mime == "text/plain" {
            data = Data("Received via Wooosh (mock transfer)\n\(meta.name)\n".utf8)
        } else {
            // Placeholder document bytes; size is symbolic, not meta.size.
            data = Data("Wooosh mock document: \(meta.name)\n".utf8) + Data(count: 32_768)
        }
        do {
            try data.write(to: url, options: .atomic)
            return url
        } catch {
            return nil
        }
    }

    /// Generates a real, valid PNG so PHAssetCreationRequest accepts it.
    private static func makePlaceholderPNG(seed: UInt32) -> Data? {
        let size = 480
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        guard let context = CGContext(
            data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
            space: colorSpace, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return nil }
        let hue = Double((seed &* 97) % 255) / 255.0
        context.setFillColor(CGColor(red: hue, green: 0.45, blue: 1.0 - hue, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: size, height: size))
        context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.85))
        context.fillEllipse(in: CGRect(x: size / 4, y: size / 4, width: size / 2, height: size / 2))
        guard let image = context.makeImage() else { return nil }
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            data as CFMutableData, UTType.png.identifier as CFString, 1, nil
        ) else { return nil }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return data as Data
    }
}
