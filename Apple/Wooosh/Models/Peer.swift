import Foundation
#if os(iOS)
import UIKit
#endif

/// The Rust core's form-factor vocabulary (`WoooshCoreFFI.DeviceType`).
///
/// Deliberately *not* what the app advertises or renders any more: the wire
/// vocabulary is `DeviceKind` (PROTOCOL.md §3.1), which is platform-explicit.
/// This enum survives only because the FFI still speaks it; every use of it in
/// the UI goes through `DeviceIcon.symbol(forCoreType:)`, which refuses to
/// guess a platform from a form factor. When the core's enum is realigned,
/// that one function is the edit.
enum DeviceType: String {
    case phone
    case tablet
    case laptop
    case desktop

    /// What the shell reports to the core at startup. Coarse by definition —
    /// the precise value goes out over mDNS as `DeviceKind.current`.
    static var current: DeviceType {
        #if os(iOS)
        UIDevice.current.userInterfaceIdiom == .pad ? .tablet : .phone
        #else
        .laptop
        #endif
    }
}

struct Peer: Identifiable {
    /// Rotating discovery id for an mDNS row; for a peer that only ever
    /// arrived over a connection (pasted QR payload, direct address) this is
    /// `core:<DeviceID>` so the row still has a stable, unique key.
    let rid: String
    var displayName: String
    /// Platform-explicit type from the TXT `dt` field. `nil` means the peer
    /// advertised nothing we recognise — rendered neutrally, never guessed
    /// (PROTOCOL.md §3.1).
    var deviceKind: DeviceKind?
    /// Fallback for rows that only ever arrived over a core connection, where
    /// all we have is the core's coarse form factor.
    var coreDeviceType: DeviceType?
    /// Monotonic first-sighting timestamp. This is the list ordering key for
    /// the whole session — the peer list is append-only and never re-sorted
    /// (DESIGN.md §5 / PROTOCOL.md §3.3).
    let discoveredAt: ContinuousClock.Instant
    var isStale: Bool
    /// The core's DeviceID for the *live* connection; nil once it drops.
    var corePeerID: String?
    /// The last DeviceID this row was ever known by. Sticky across
    /// disconnects, unlike `corePeerID`, because it is what lets a reconnect
    /// look the peer's pinned key up in `trustedPeers()` and pass it as
    /// `expectedPublicKey` — the core's address-based fallback only covers a
    /// peer coming back on the same `ip:port`.
    var knownDeviceID: String?
    /// Whether the core reports the live connection as trusted (pinned key).
    var isTrusted: Bool = false

    var id: String { rid }
    var isConnected: Bool { corePeerID != nil }

    /// TXT `dt` wins — it is the only source that can tell an iPhone from a
    /// Pixel. Failing that, the core's coarse type, which only ever yields
    /// platform-neutral glyphs.
    var symbolName: String {
        if let deviceKind { return DeviceIcon.symbol(for: deviceKind) }
        return DeviceIcon.symbol(forCoreType: coreDeviceType)
    }
}
