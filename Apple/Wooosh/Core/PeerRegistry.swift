import Foundation
import Observation

/// Canonical peer list (DESIGN.md §5 / PROTOCOL.md §3.3).
///
/// Ordering contract: rows are appended at first sighting of a `rid` and the
/// array is NEVER re-sorted or compacted. Stale peers stay in place, grayed
/// out; rows are only removed by `clear()` (app relaunch or explicit refresh).
@MainActor
@Observable
final class PeerRegistry {
    private(set) var peers: [Peer] = []

    /// Grace period before a vanished peer is grayed out.
    static let staleGrace: Duration = .seconds(10)

    @ObservationIgnored
    private var staleTasks: [String: Task<Void, Never>] = [:]

    /// `deviceKind` is the peer's advertised TXT `dt`, or nil when it sent
    /// nothing we recognise — which renders neutrally rather than as a guess
    /// (PROTOCOL.md §3.1).
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

    /// Records the core's DeviceID against a discovered row after a
    /// successful `connect_peer`.
    func attach(corePeerID: String, toRID rid: String, trusted: Bool) {
        guard let index = peers.firstIndex(where: { $0.rid == rid }) else { return }
        peers[index].corePeerID = corePeerID
        peers[index].knownDeviceID = corePeerID
        peers[index].isTrusted = trusted
    }

    /// A `PeerConnected` event: update the row that already owns this
    /// DeviceID, or append one for a peer the shell never saw over mDNS
    /// (pasted QR payload, direct address). Append-only, like every other
    /// row (DESIGN.md §5).
    ///
    /// `deviceType` is the peer's own HELLO value, or nil when it did not
    /// report one — in which case an existing row keeps whatever mDNS said
    /// rather than being overwritten with a guess.
    func connected(peerID: String, displayName: String, deviceType: DeviceType?, trusted: Bool) {
        // Match on DeviceID first — including rows that were connected earlier
        // in the session and dropped, so a reconnect reuses the same row.
        if let index = peers.firstIndex(where: { $0.knownDeviceID == peerID }) {
            peers[index].corePeerID = peerID
            peers[index].isTrusted = trusted
            peers[index].isStale = false
            if !displayName.isEmpty { peers[index].displayName = displayName }
            if let deviceType { peers[index].coreDeviceType = deviceType }
            return
        }
        // Otherwise adopt a discovered row whose display name matches, so a
        // peer found over mDNS and then connected doesn't show up twice.
        if let index = peers.firstIndex(where: {
            $0.knownDeviceID == nil && !displayName.isEmpty && $0.displayName == displayName
        }) {
            peers[index].corePeerID = peerID
            peers[index].knownDeviceID = peerID
            peers[index].isTrusted = trusted
            peers[index].isStale = false
            if let deviceType { peers[index].coreDeviceType = deviceType }
            return
        }
        peers.append(Peer(
            rid: "core:\(peerID)",
            displayName: displayName.isEmpty ? L.t("peer_unnamed") : displayName,
            // Connection-only rows never saw a TXT record, so they carry only
            // the core's coarse form factor — which maps to a neutral glyph
            // unless it is unambiguous.
            deviceKind: nil,
            coreDeviceType: deviceType,
            discoveredAt: .now,
            isStale: false,
            corePeerID: peerID,
            knownDeviceID: peerID,
            isTrusted: trusted
        ))
    }

    func disconnected(peerID: String) {
        guard let index = peers.firstIndex(where: { $0.corePeerID == peerID }) else { return }
        // `knownDeviceID` is deliberately left in place: it is the key a later
        // reconnect uses to pin `expectedPublicKey`.
        peers[index].corePeerID = nil
        peers[index].isTrusted = false
        // A row that only ever existed because of the connection goes stale
        // rather than disappearing — rows never move or vanish (DESIGN.md §5).
        if peers[index].rid.hasPrefix("core:") {
            peers[index].isStale = true
        }
    }

    func peer(forCorePeerID peerID: String) -> Peer? {
        peers.first { $0.corePeerID == peerID }
    }

    /// Row for a DeviceID whether or not it is currently connected.
    func peer(forDeviceID deviceID: String) -> Peer? {
        peers.first { $0.knownDeviceID == deviceID }
    }

    /// Reflects a trust change made in the core (revoke) onto the live row.
    func setTrusted(_ trusted: Bool, forDeviceID deviceID: String) {
        guard let index = peers.firstIndex(where: { $0.knownDeviceID == deviceID }) else { return }
        peers[index].isTrusted = trusted
    }

    /// NWBrowser reported the peer's announcement as removed. Removal is only
    /// the *start* of staleness: the row is grayed out after a 10 s grace so
    /// transient browse churn doesn't flicker the list.
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
