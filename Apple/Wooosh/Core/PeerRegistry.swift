import Foundation
import Observation

/// Ordering contract (DESIGN.md §5): appended at first sighting, NEVER re-sorted
/// or compacted. Stale peers gray out in place; only `clear()` removes anything.
@MainActor
@Observable
final class PeerRegistry {
    private(set) var peers: [Peer] = []

    static let staleGrace: Duration = .seconds(10)

    @ObservationIgnored
    private var staleTasks: [String: Task<Void, Never>] = [:]

    /// nil `deviceKind` renders neutrally rather than as a guess (§3.1).
    func sighted(rid: String, displayName: String, deviceKind: DeviceKind?) {
        staleTasks.removeValue(forKey: rid)?.cancel()
        if let index = peers.firstIndex(where: { $0.rid == rid }) {
            peers[index].displayName = displayName
            peers[index].deviceKind = deviceKind
            peers[index].isStale = false
        } else {
            peers.append(Peer(
                rid: rid,
                displayName: displayName,
                deviceKind: deviceKind,
                coreDeviceType: nil,
                discoveredAt: .now,
                isStale: false
            ))
        }
    }

    // MARK: - Core-connected peers

    func attach(corePeerID: String, toRID rid: String, trusted: Bool) {
        guard let index = peers.firstIndex(where: { $0.rid == rid }) else { return }
        peers[index].corePeerID = corePeerID
        peers[index].knownDeviceID = corePeerID
        peers[index].isTrusted = trusted
    }

    /// Append-only (DESIGN.md §5); a nil `deviceType` leaves what mDNS said.
    /// `viaTicket` relies on `PeerConnected` preceding `TicketRedeemed`, so the
    /// clearing call lands before the setting one.
    func connected(peerID: String, displayName: String, deviceType: DeviceType?,
                   trusted: Bool, viaTicket: Bool = false) {
        // DeviceID first, including dropped rows, so a reconnect reuses its row.
        if let index = peers.firstIndex(where: { $0.knownDeviceID == peerID }) {
            peers[index].corePeerID = peerID
            peers[index].isTrusted = trusted
            peers[index].isTicketOnly = viaTicket
            peers[index].isStale = false
            if !displayName.isEmpty { peers[index].displayName = displayName }
            if let deviceType { peers[index].coreDeviceType = deviceType }
            return
        }
        // Adopt a name-matched discovered row so one peer cannot appear twice.
        if let index = peers.firstIndex(where: {
            $0.knownDeviceID == nil && !displayName.isEmpty && $0.displayName == displayName
        }) {
            peers[index].corePeerID = peerID
            peers[index].knownDeviceID = peerID
            peers[index].isTrusted = trusted
            peers[index].isTicketOnly = viaTicket
            peers[index].isStale = false
            if let deviceType { peers[index].coreDeviceType = deviceType }
            return
        }
        peers.append(Peer(
            rid: "core:\(peerID)",
            displayName: displayName.isEmpty ? L.t("peer_unnamed") : displayName,
            deviceKind: nil,
            coreDeviceType: deviceType,
            discoveredAt: .now,
            isStale: false,
            corePeerID: peerID,
            knownDeviceID: peerID,
            isTrusted: trusted,
            isTicketOnly: viaTicket
        ))
    }

    /// A connection-only row grays out in place once its single-use ticket is
    /// spent; `isTicketOnly` stays set so it cannot re-acquire a paired badge.
    func ticketSessionEnded(peerID: String) {
        guard let index = peers.firstIndex(where: {
            $0.knownDeviceID == peerID && $0.isTicketOnly && $0.rid.hasPrefix("core:")
        }) else { return }
        peers[index].isStale = true
    }

    func disconnected(peerID: String) {
        guard let index = peers.firstIndex(where: { $0.corePeerID == peerID }) else { return }
        // `knownDeviceID` stays: a reconnect needs it to pin `expectedPublicKey`.
        peers[index].corePeerID = nil
        peers[index].isTrusted = false
        // Rows never move or vanish, so a connection-only row goes stale (§5).
        if peers[index].rid.hasPrefix("core:") {
            peers[index].isStale = true
        }
    }

    func peer(forCorePeerID peerID: String) -> Peer? {
        peers.first { $0.corePeerID == peerID }
    }

    func peer(forDeviceID deviceID: String) -> Peer? {
        peers.first { $0.knownDeviceID == deviceID }
    }

    func setTrusted(_ trusted: Bool, forDeviceID deviceID: String) {
        guard let index = peers.firstIndex(where: { $0.knownDeviceID == deviceID }) else { return }
        peers[index].isTrusted = trusted
    }

    /// Removal only *starts* staleness; the grace absorbs transient browse churn.
    func lost(rid: String) {
        guard peers.contains(where: { $0.rid == rid }) else { return }
        staleTasks[rid]?.cancel()
        staleTasks[rid] = Task { [weak self] in
            try? await Task.sleep(for: PeerRegistry.staleGrace)
            guard !Task.isCancelled else { return }
            self?.markStale(rid: rid)
            self?.staleTasks.removeValue(forKey: rid)
        }
    }

    func clear() {
        for task in staleTasks.values { task.cancel() }
        staleTasks.removeAll()
        peers.removeAll()
    }

    private func markStale(rid: String) {
        guard let index = peers.firstIndex(where: { $0.rid == rid }) else { return }
        peers[index].isStale = true
    }
}
