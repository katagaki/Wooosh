import Foundation
import Observation
import os
import UniformTypeIdentifiers
#if os(iOS)
import UIKit
#endif

@MainActor
@Observable
final class Transfer: Identifiable {
    enum Direction { case outgoing, incoming }

    enum State: Equatable {
        /// Incoming: consent sheet is showing.
        case awaitingConsent
        /// Outgoing: waiting for the receiver's DECISION.
        case connecting
        case transferring
        case done(TransferSummary)
        case failed(message: String, resumable: Bool)
        case cancelled

        static func == (lhs: State, rhs: State) -> Bool {
            switch (lhs, rhs) {
            case (.awaitingConsent, .awaitingConsent), (.connecting, .connecting),
                 (.transferring, .transferring), (.done, .done), (.cancelled, .cancelled):
                true
            case (.failed(let m1, let r1), .failed(let m2, let r2)):
                m1 == m2 && r1 == r2
            default:
                false
            }
        }
    }

    struct File: Identifiable {
        enum Status: Equatable {
            case pending
            case transferring
            case completed
            /// Routed to final storage ("Photos", "Downloads", …).
            case saved(destination: String)
            case failed(String)
        }

        let id: UInt32
        let name: String
        let size: Int64
        let mime: String
        var bytes: Int64 = 0
        var status: Status = .pending
        /// nil for Photos insertions: no path the app may keep.
        var savedURL: URL?

        var fraction: Double { size > 0 ? Double(bytes) / Double(size) : 0 }
    }

    let id: TransferID
    let peer: PeerRef
    let direction: Direction
    var files: [File]
    var state: State
    /// Aggregate instantaneous rate (bytes/s) and ETA from the core.
    var rate: Double = 0
    var eta: TimeInterval?
    let peerWasPaired: Bool

    init(id: TransferID, peer: PeerRef, direction: Direction, files: [File],
         state: State, peerWasPaired: Bool) {
        self.id = id
        self.peer = peer
        self.direction = direction
        self.files = files
        self.state = state
        self.peerWasPaired = peerWasPaired
    }

    var totalBytes: Int64 { files.reduce(0) { $0 + $1.size } }
    var transferredBytes: Int64 {
        files.reduce(0) { $0 + (isFileFinished($1) ? $1.size : $1.bytes) }
    }
    var overallFraction: Double {
        totalBytes > 0 ? Double(transferredBytes) / Double(totalBytes) : 0
    }
    var isActive: Bool {
        switch state {
        case .connecting, .transferring: true
        default: false
        }
    }

    private func isFileFinished(_ file: File) -> Bool {
        switch file.status {
        case .completed, .saved: true
        default: false
        }
    }
}

extension Transfer: Equatable {
    nonisolated static func == (lhs: Transfer, rhs: Transfer) -> Bool {
        lhs === rhs
    }
}

/// Routes verified files to storage (DESIGN.md §6) and holds keep-awake.
@MainActor
@Observable
final class TransferCenter {
    private(set) var transfers: [Transfer] = []
    var pendingOffer: Transfer?

    @ObservationIgnored private weak var core: (any WoooshCore)?
    @ObservationIgnored private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "transfers")
    #if os(iOS)
    @ObservationIgnored private let backgroundTask = BackgroundTransferTask()
    @ObservationIgnored private var notifiedReceipts: Set<TransferID> = []
    #endif

    func attach(core: any WoooshCore) {
        self.core = core
    }

    func transfer(for id: TransferID) -> Transfer? {
        transfers.first { $0.id == id }
    }

    // MARK: - User intents

    func send(peer: PeerRef, paired: Bool, urls: [URL]) -> Transfer? {
        guard let core else { return nil }
        let tid = core.send(peerID: peer.id, urls: urls)
        let files = urls.enumerated().map { index, url in
            let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize).flatMap { Int64($0) } ?? 0
            let mime = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType
                ?? "application/octet-stream"
            return Transfer.File(id: UInt32(index), name: url.lastPathComponent,
                                 size: max(size, 1), mime: mime)
        }
        let transfer = Transfer(id: tid, peer: peer, direction: .outgoing,
                                files: files, state: .connecting, peerWasPaired: paired)
        transfers.append(transfer)
        updateKeepAwake()
        return transfer
    }

    func accept(offer: Transfer, fileIDs: [UInt32]? = nil) {
        let accepted = fileIDs ?? offer.files.map(\.id)
        offer.files.removeAll { !accepted.contains($0.id) }
        offer.state = .transferring
        pendingOffer = nil
        transfers.append(offer)
        core?.respondToOffer(transferID: offer.id, acceptedFileIDs: accepted)
        requestNotificationAuthorization()
        updateKeepAwake()
    }

    func decline(offer: Transfer) {
        pendingOffer = nil
        core?.respondToOffer(transferID: offer.id, acceptedFileIDs: [])
    }

    func cancel(_ transfer: Transfer) {
        core?.cancel(transferID: transfer.id)
        transfer.state = .cancelled
        if pendingOffer?.id == transfer.id { pendingOffer = nil }
        updateKeepAwake()
    }

    // MARK: - Core events

    func handle(event: CoreEvent) {
        switch event {
        case .ticketRedeemed, .peerConnected, .peerDisconnected, .pairingSAS,
             .pairingResult, .keyChanged:
            // Owned by AppModel; listed so a new event cannot be swallowed here.
            break

        case .incomingOffer(let tid, let from, let trusted, let manifest):
            handleIncomingOffer(tid: tid, from: from, trusted: trusted, manifest: manifest)
        case .transferStarted(let tid, _, _, let manifest):
            handleTransferStarted(tid: tid, manifest: manifest)
        case .progress(let tid, let fileID, let bytes, let totalBytes, let rate, let eta):
            handleProgress(tid: tid, fileID: fileID, bytes: bytes,
                           totalBytes: totalBytes, rate: rate, eta: eta)
        case .fileReady(let tid, let fileID, let stagedURL, let kind):
            handleFileReady(tid: tid, fileID: fileID, stagedURL: stagedURL, kind: kind)
        case .transferDone(let tid, let summary):
            guard let transfer = transfer(for: tid) else { return }
            transfer.state = .done(summary)
            transfer.eta = nil
            for index in transfer.files.indices where transfer.files[index].status == .transferring {
                transfer.files[index].status = .completed
            }
            updateKeepAwake()
            maybeNotifyReceived(transfer)
        case .transferError(let tid, let message, let resumable):
            guard let transfer = transfer(for: tid) else { return }
            // The core's wording is an internal token; map it to real copy.
            transfer.state = .failed(message: transferErrorMessage(message), resumable: resumable)
            updateKeepAwake()
        }
    }

    /// Real sizes and file ids; the shell's provisional ids were positional.
    private func handleTransferStarted(tid: TransferID, manifest: [FileMeta]) {
        guard let transfer = transfer(for: tid), !manifest.isEmpty else { return }
        transfer.files = manifest.map {
            Transfer.File(id: $0.id, name: $0.name, size: $0.size, mime: $0.mime)
        }
        if transfer.state == .connecting { transfer.state = .transferring }
    }

    private func handleIncomingOffer(tid: TransferID, from: PeerRef, trusted: Bool,
                                     manifest: [FileMeta]) {
        let files = manifest.map {
            Transfer.File(id: $0.id, name: $0.name, size: $0.size, mime: $0.mime)
        }
        // `trusted` is the core's verdict, not a guess against a discovery id.
        let transfer = Transfer(id: tid, peer: from, direction: .incoming,
                                files: files, state: .awaitingConsent,
                                peerWasPaired: trusted)
        // Pairing already *is* the consent (PROTOCOL.md §4); re-prompting trains
        // the user to dismiss the sheet that matters, the unpaired sender's.
        if trusted {
            // Not `accept(offer:)`: that would clear another sender's live sheet.
            transfer.state = .transferring
            transfers.append(transfer)
            core?.respondToOffer(transferID: tid, acceptedFileIDs: manifest.map(\.id))
            requestNotificationAuthorization()
            updateKeepAwake()
            return
        }
        // One sheet at a time; a second offer is declined (rate limiting is core's).
        if pendingOffer == nil {
            pendingOffer = transfer
        } else {
            core?.respondToOffer(transferID: tid, acceptedFileIDs: [])
        }
    }

    private func handleProgress(tid: TransferID, fileID: UInt32, bytes: Int64,
                                totalBytes: Int64, rate: Double, eta: TimeInterval) {
        guard let transfer = transfer(for: tid) else { return }
        if transfer.state == .connecting { transfer.state = .transferring }
        transfer.rate = rate
        transfer.eta = eta
        guard let index = transfer.files.firstIndex(where: { $0.id == fileID }) else { return }
        transfer.files[index].bytes = bytes
        if bytes >= totalBytes {
            if transfer.files[index].status == .pending || transfer.files[index].status == .transferring {
                transfer.files[index].status = .completed
            }
        } else {
            transfer.files[index].status = .transferring
        }
        #if os(iOS)
        // Not `updateKeepAwake()`: the active set has not changed.
        if let snapshot = receiveSnapshot() { backgroundTask.update(snapshot) }
        #endif
    }

    private func handleFileReady(tid: TransferID, fileID: UInt32, stagedURL: URL, kind: FileKind) {
        guard let transfer = transfer(for: tid) else { return }
        // Matched on the manifest `fid`: staging names need not match.
        let index = transfer.files.firstIndex { $0.id == fileID }
        guard transfer.direction == .incoming else {
            // Outgoing: per-file completion signal only — nothing to route.
            if let index { transfer.files[index].status = .completed }
            return
        }
        Task {
            do {
                let placement = try await StorageRouter.route(stagedURL: stagedURL, kind: kind)
                if let index {
                    transfer.files[index].status = .saved(destination: placement.label)
                    transfer.files[index].savedURL = placement.url
                }
            } catch {
                logger.error("Routing failed for \(stagedURL.lastPathComponent): \(error)")
                if let index {
                    transfer.files[index].status = .failed(L.t("error_save_failed"))
                }
            }
            // Routing is async: whichever of this and transferDone is second posts.
            maybeNotifyReceived(transfer)
        }
    }

    /// Asked when files are about to arrive, so the prompt has visible cause.
    private func requestNotificationAuthorization() {
        #if os(iOS)
        ReceiptNotifier.shared.requestAuthorizationIfNeeded()
        #endif
    }

    /// Only once finished *and* routed: opening a file still in staging is a lie.
    private func maybeNotifyReceived(_ transfer: Transfer) {
        #if os(iOS)
        guard transfer.direction == .incoming, case .done = transfer.state else { return }
        let settled = transfer.files.allSatisfy {
            switch $0.status {
            case .saved, .failed: true
            default: false
            }
        }
        guard settled, notifiedReceipts.insert(transfer.id).inserted else { return }
        ReceiptNotifier.shared.notifyReceived(transfer)
        #endif
    }

    // MARK: - Keep-awake and background execution (DESIGN.md §7)

    private func updateKeepAwake() {
        KeepAwake.setActive(transfers.contains { $0.isActive })
        #if os(iOS)
        guard let snapshot = receiveSnapshot() else {
            // `success` reports only that the session ended in an orderly way.
            backgroundTask.finish(nil, success: true)
            return
        }
        backgroundTask.begin(snapshot) { [weak self] in
            guard let self else { return }
            // Expiry and the system Stop button are indistinguishable here. In the
            // foreground only the assertion was lost, so keep going; backgrounded
            // nothing can progress, so stop rather than leave a dead card.
            guard UIApplication.shared.applicationState == .background else { return }
            for transfer in transfers where transfer.direction == .incoming && transfer.isActive {
                cancel(transfer)
            }
        }
        #endif
    }

    #if os(iOS)
    private func receiveSnapshot() -> BackgroundTransferTask.Snapshot? {
        let incoming = transfers.filter { $0.direction == .incoming && $0.isActive }
        guard let first = incoming.first else { return nil }
        return BackgroundTransferTask.Snapshot(
            peerName: first.peer.displayName,
            transferCount: incoming.count,
            completedBytes: incoming.reduce(0) { $0 + $1.transferredBytes },
            totalBytes: incoming.reduce(0) { $0 + $1.totalBytes },
            rate: incoming.reduce(0) { $0 + $1.rate }
        )
    }
    #endif
}
