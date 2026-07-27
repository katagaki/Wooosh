import Foundation
#if os(iOS)
import UIKit
#endif

/// Exists only because the FFI speaks it; the wire vocabulary is `DeviceKind`.
enum DeviceType: String {
    case phone
    case tablet
    case laptop
    case desktop

    /// Coarse by definition; mDNS carries the precise `DeviceKind.current`.
    static var current: DeviceType {
        #if os(iOS)
        UIDevice.current.userInterfaceIdiom == .pad ? .tablet : .phone
        #else
        .laptop
        #endif
    }
}

struct Peer: Identifiable {
    /// mDNS discovery id, or `core:<DeviceID>` for a connection-only row.
    let rid: String
    var displayName: String
    /// From TXT `dt`; `nil` renders neutrally, never guessed (PROTOCOL.md §3.1).
    var deviceKind: DeviceKind?
    var coreDeviceType: DeviceType?
    /// The session's ordering key: append-only, never re-sorted (DESIGN.md §5).
    let discoveredAt: ContinuousClock.Instant
    var isStale: Bool
    var corePeerID: String?
    /// Sticky across disconnects: a reconnect needs it to look up the pinned key.
    var knownDeviceID: String?
    var isTrusted: Bool = false
    /// Admitted by a ticket, not a pin (PROTOCOL.md §9.4), so never badge it paired.
    /// Cleared on `PeerConnected`, not on disconnect: a checkmark re-appearing as
    /// the session drops is the same wrong claim, just quieter.
    var isTicketOnly: Bool = false

    var id: String { rid }
    var isConnected: Bool { corePeerID != nil }

    /// TXT `dt` wins: it is the only source that tells an iPhone from a Pixel.
    var symbolName: String {
        if let deviceKind { return DeviceIcon.symbol(for: deviceKind) }
        return DeviceIcon.symbol(forCoreType: coreDeviceType)
    }
}
