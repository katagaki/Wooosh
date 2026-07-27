import Foundation
import Observation

/// Read straight from the core's `trust.json` (PROTOCOL.md §4.5). Persists
/// nothing of its own: a shell-side mirror could disagree about who is pinned.
@MainActor
@Observable
final class TrustStore {
    private(set) var devices: [TrustedPeerInfo] = []

    /// Transient warning state; trust itself is only ever changed by the core.
    private(set) var keyChangedIDs: Set<String> = []

    @ObservationIgnored
    private weak var core: (any WoooshCore)?

    func attach(core: any WoooshCore) {
        self.core = core
        keyChangedIDs.removeAll()
        refresh()
    }

    /// Call at launch, after every successful `pairingResult`, and after revoke.
    func refresh() {
        devices = core?.trustedPeers() ?? []
        // A peer that is no longer pinned cannot be "key changed" any more.
        let pinned = Set(devices.map(\.deviceID))
        keyChangedIDs.formIntersection(pinned)
    }

    func markKeyChanged(deviceID: String) {
        keyChangedIDs.insert(deviceID)
    }

    func device(forDeviceID deviceID: String) -> TrustedPeerInfo? {
        devices.first { $0.deviceID == deviceID }
    }

    func publicKey(forDeviceID deviceID: String) -> Data? {
        device(forDeviceID: deviceID)?.publicKey
    }

    /// Pinned and not currently flagged as having changed keys.
    func isPaired(deviceID: String) -> Bool {
        device(forDeviceID: deviceID) != nil && !keyChangedIDs.contains(deviceID)
    }

    func hasKeyChanged(deviceID: String) -> Bool {
        keyChangedIDs.contains(deviceID)
    }
}
