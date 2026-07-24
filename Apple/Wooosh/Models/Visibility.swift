import Foundation

enum Visibility: String, CaseIterable, Identifiable {
    case everyone
    case pairedOnly
    case off

    var id: String { rawValue }

    /// TXT `vis` value (PROTOCOL.md §3.1). `nil` means do not advertise at all.
    var txtValue: String? {
        switch self {
        case .everyone: "e"
        case .pairedOnly: "p"
        case .off: nil
        }
    }

    var label: String {
        switch self {
        case .everyone: L.t("settings_visibility_everyone")
        case .pairedOnly: L.t("settings_visibility_paired")
        case .off: L.t("settings_visibility_off")
        }
    }
}
