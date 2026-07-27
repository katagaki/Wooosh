import Foundation

/// A relay only introduces two devices (DESIGN.md §9.1): file data always
/// travels directly, and a transfer with no direct path is refused, not relayed.
enum RelayMode: String, CaseIterable, Identifiable {
    /// n0's free public relays, shared with every other iroh user.
    case publicRelays
    /// Advertised in this device's tickets, so the redeemer needs no setup.
    case custom
    /// Wooosh contacts nothing and neither publishes nor redeems tickets.
    case off

    var id: String { rawValue }

    var label: String {
        switch self {
        case .publicRelays: L.t("settings_relay_public")
        case .custom: L.t("settings_relay_custom")
        case .off: L.t("settings_relay_off")
        }
    }
}

/// The resolved setting, in the shape `setRelayURLs` takes.
struct RelayPreference: Equatable {
    var mode: RelayMode
    /// Kept across mode changes so switching away and back loses nothing typed.
    var customURL: String

    static let `default` = RelayPreference(mode: .publicRelays, customURL: "")

    /// When false, nothing publishes or redeems, so no endpoint can be bound.
    var internetEnabled: Bool { mode != .off }

    /// `nil` = public set, `[]` = none. `.custom` with a blank URL falls back to
    /// public rather than silently disabling the path; `.off` resolves to `[]` so
    /// that a code path slipping past the UI still cannot reach a relay.
    var coreValue: [String]? {
        switch mode {
        case .publicRelays: nil
        case .off: []
        case .custom:
            customURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                ? nil
                : [customURL.trimmingCharacters(in: .whitespacesAndNewlines)]
        }
    }
}
