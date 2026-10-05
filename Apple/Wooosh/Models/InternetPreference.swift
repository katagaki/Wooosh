import Foundation

/// Which rendezvous service introduces two devices over the internet
/// (DESIGN.md §9.1), or none at all.
enum InternetMode: String, CaseIterable, Identifiable {
    /// The Wooosh Worker this build was compiled against.
    case wooosh
    /// A self-deployed copy of the Worker. Both devices must use it.
    case custom
    /// Wooosh contacts nothing and neither publishes nor redeems codes.
    case off

    var id: String { rawValue }

    var label: String {
        switch self {
        case .wooosh: L.t("settings_internet_wooosh")
        case .custom: L.t("settings_internet_custom")
        case .off: L.t("settings_internet_off")
        }
    }
}

/// The resolved setting, in the shape `setRendezvousURL` takes.
struct InternetPreference: Equatable {
    var mode: InternetMode
    /// Kept across mode changes so switching away and back loses nothing typed.
    var customURL: String

    static let `default` = InternetPreference(mode: .wooosh, customURL: "")

    var internetEnabled: Bool { mode != .off }

    /// `nil` = the build's service, `""` = off. `.custom` with a blank URL falls
    /// back to the build's service rather than silently switching the path off;
    /// `.off` resolves to `""` so a code path slipping past the UI still cannot
    /// reach a server.
    var coreValue: String? {
        switch mode {
        case .wooosh: nil
        case .off: ""
        case .custom:
            customURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                ? nil
                : customURL.trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }
}
