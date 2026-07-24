import Foundation
import Observation

/// The user-visible trust list (PROTOCOL.md §4.5), read straight from the
/// core's canonical `trust.json` via `trustedPeers()`.
///
/// This deliberately holds no persisted state of its own. The previous
/// UserDefaults mirror could disagree with the core about who was pinned (and
/// carried its own copy of the pubkeys); the core is now the single source, so
/// this type is a refresh-on-demand cache plus the one piece of state the core
/// does not model: which peers have shown a KEY_CHANGED *this session*.
@MainActor
@Observable
final class TrustStore {
    /// Pinned peers, in the core's order (paired-at, then DeviceID).
    private(set) var devices: [TrustedPeerInfo] = []

    /// DeviceIDs that presented a different key since launch. Transient
    /// warning state — trust itself is only ever changed by the core.
    private(set) var keyChangedIDs: Set<String> = []

    @ObservationIgnored
    private weak var core: (any WoooshCore)?

    func attach(core: any WoooshCore) {
        self.core = core
        keyChangedIDs.removeAll()
        refresh()
    }

    /// Re-reads the core's trust store. Call at launch, after every successful
    /// `pairingResult`, and after `revokePeer`.
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

    /// The pinned key for a DeviceID — what `connectPeer` should pin with.
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
