import Foundation
#if os(iOS)
import UIKit
#endif

/// The TXT `dt` vocabulary (PROTOCOL.md §3.1). Form factor alone cannot pick an
/// icon, so anything not in this list is *unknown*, never guessed at.
enum DeviceKind: String, Sendable, CaseIterable {
    case iphone
    case ipad
    case mac
    case windows
    case androidPhone = "android-phone"
    case androidTablet = "android-tablet"

    /// nil for absent, empty, unrecognised, or retired vocabulary, which degrades
    /// to the neutral glyph rather than to a confidently wrong one.
    init?(wire: String?) {
        guard let wire, let kind = DeviceKind(rawValue: wire) else { return nil }
        self = kind
    }

    static var current: DeviceKind {
        #if os(iOS)
        UIDevice.current.userInterfaceIdiom == .pad ? .ipad : .iphone
        #else
        .mac
        #endif
    }
}

/// The single place a device type becomes an SF Symbol, fed by the authoritative
/// TXT `dt` and by the core's coarser enum. Aligning the two is one edit here.
enum DeviceIcon {
    /// A generic icon is always acceptable; a confidently wrong one is not.
    static let neutral = "questionmark.square.dashed"

    static func symbol(for kind: DeviceKind?) -> String {
        switch kind {
        case .iphone: "iphone"
        case .ipad: "ipad"
        case .mac: "laptopcomputer"
        case .windows: "pc"
        // The user's explicit ask: non-Apple phones get the generic handset.
        case .androidPhone: "smartphone"
        // No neutral tablet glyph in this SDK, and `ipad` would be a lie.
        case .androidTablet: neutral
        case nil: neutral
        }
    }

    /// `phone` and `tablet` are ambiguous between Apple and Android, so they go
    /// neutral; `laptop` and `desktop` have honest vendor-neutral glyphs.
    static func symbol(forCoreType type: DeviceType?) -> String {
        switch type {
        case .laptop: "laptopcomputer"
        case .desktop: "desktopcomputer"
        case .phone, .tablet, nil: neutral
        }
    }

    /// The glyph is the only place a row states its device kind, so VoiceOver
    /// needs it in words. Unrecognised reads as "Device", never a guess.
    static func label(for kind: DeviceKind?) -> String {
        switch kind {
        case .iphone: L.t("device_kind_iphone")
        case .ipad: L.t("device_kind_ipad")
        case .mac: L.t("device_kind_mac")
        case .windows: L.t("device_kind_windows")
        case .androidPhone: L.t("device_kind_android_phone")
        case .androidTablet: L.t("device_kind_android_tablet")
        case nil: L.t("device_kind_unknown")
        }
    }
}
