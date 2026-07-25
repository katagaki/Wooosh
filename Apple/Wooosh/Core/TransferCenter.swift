import Foundation
import Observation
import os
import UniformTypeIdentifiers

/// Shell-side view state for one transfer, fed by the core event stream.
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
            /// Incoming only: routed to final storage ("Photos", "Downloads", …).
            case saved(destination: String)
            case failed(String)
        }

        let id: UInt32
        let name: String
        let size: Int64
        let mime: String
        var bytes: Int64 = 0
        var status: Status = .pending
        /// Final on-disk location after routing, when there is one (Photos
        /// insertions have no path the app may keep).
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
    /// Whether the sender was paired at offer time (drives consent UI).
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
    /// Identity equality — used by SwiftUI onChange for sheet presentation.
    nonisolated static func == (lhs: Transfer, rhs: Transfer) -> Bool {
        lhs === rhs
    }
}

/// Owns all transfer view state, applies core transfer events, routes
/// verified files to storage (DESIGN.md §6), and holds keep-awake while any
/// transfer is active.
@MainActor
@Observable
final class TransferCenter {
    private(set) var transfers: [Transfer] = []
    /// Incoming offer currently awaiting user consent (one at a time).
    var pendingOffer: Transfer?

    @ObservationIgnored private weak var core: (any WoooshCore)?
    @ObservationIgnored private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "transfers")

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
            // Owned by AppModel; listed explicitly so a new core event cannot
            // be silently swallowed here.
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
        case .transferError(let tid, let message, let resumable):
            guard let transfer = transfer(for: tid) else { return }
            // The core's wording is an internal token; map it to real copy.
            transfer.state = .failed(message: transferErrorMessage(message), resumable: resumable)
            updateKeepAwake()
        }
    }

    /// The core resolved the manifest for an outgoing transfer (real sizes,
    /// real file ids — the shell's provisional ids were only positional).
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
        // `trusted` is the core's verdict (the sender's key is pinned), not a
        // shell-side guess against a discovery id.
        let transfer = Transfer(id: tid, peer: from, direction: .incoming,
                                files: files, state: .awaitingConsent,
                                peerWasPaired: trusted)
        // Pairing already *is* the consent (PROTOCOL.md §4). Asking again for
        // every transfer from a device the user deliberately pinned turns the
        // prompt into something to dismiss without reading, which is worse for
        // the case that actually matters: the unpaired sender, who still gets
        // the full sheet with the fingerprint to verify.
        if trusted {
            // Not `accept(offer:)`: that clears `pendingOffer`, which here could
            // belong to a *different*, unpaired sender whose sheet is on screen
            // and unanswered.
            transfer.state = .transferring
            transfers.append(transfer)
            core?.respondToOffer(transferID: tid, acceptedFileIDs: manifest.map(\.id))
            updateKeepAwake()
            return
        }
        // One consent sheet at a time; a second simultaneous offer replaces
        // nothing — it is silently declined (rate limiting is core's job).
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
    }

    private func handleFileReady(tid: TransferID, fileID: UInt32, stagedURL: URL, kind: FileKind) {
        guard let transfer = transfer(for: tid) else { return }
        // Matched on the manifest `fid`, never on the staged file name:
        // staging names are `<fid>.part`-derived and need not match.
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
        }
    }

    // MARK: - Keep-awake (DESIGN.md §7)

    private func updateKeepAwake() {
        KeepAwake.setActive(transfers.contains { $0.isActive })
    }
}
