import Foundation

/// Which relays the internet path may use (DESIGN.md §9.1).
///
/// A relay only ever introduces two devices; file data always travels
/// directly, and a transfer that cannot find a direct path is refused rather
/// than relayed. So this setting decides who helps the two devices meet, not
/// who carries the files.
enum RelayMode: String, CaseIterable, Identifiable {
    /// n0's free public relays, shared with every other iroh user.
    case publicRelays
    /// A relay the user runs or chose. This device's tickets advertise it, so
    /// the other device uses it without configuring anything.
    case custom
    /// Internet transfers are off. Wooosh contacts nothing and neither
    /// publishes nor redeems tickets; only devices on the same network.
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
    /// Only meaningful for `.custom`; kept across mode changes so switching
    /// away and back does not lose what the user typed.
    var customURL: String

    static let `default` = RelayPreference(mode: .publicRelays, customURL: "")

    /// Whether the internet path is available at all. When false the shells
    /// neither publish nor redeem tickets, so nothing can bind an endpoint.
    var internetEnabled: Bool { mode != .off }

    /// `nil` = n0's public set, `[]` = no relays at all, otherwise the chosen
    /// relay. A `.custom` mode with a blank URL is not a valid configuration,
    /// so it falls back to the public set rather than silently disabling the
    /// internet path.
    ///
    /// `.off` still resolves to `[]` rather than being left unset: the UI is
    /// what stops a ticket being made, and this makes sure that even a code
    /// path that got past it cannot reach a relay.
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
