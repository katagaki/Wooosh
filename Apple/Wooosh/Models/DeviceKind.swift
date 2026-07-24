import Foundation
#if os(iOS)
import UIKit
#endif

/// The platform-explicit device vocabulary carried in the mDNS TXT `dt` field
/// (PROTOCOL.md §3.1).
///
/// Form factor alone cannot pick a correct icon — an Android phone and an
/// iPhone are both "a phone", and drawing an iPhone glyph for a Pixel is
/// wrong. Anything not in this list is *unknown*, never guessed at.
enum DeviceKind: String, Sendable, CaseIterable {
    case iphone
    case ipad
    case mac
    case windows
    case androidPhone = "android-phone"
    case androidTablet = "android-tablet"

    /// Parses a TXT `dt` value. Returns nil for an absent, empty or
    /// unrecognized value — including peers still advertising the retired
    /// `phone`/`tablet`/`laptop`/`desktop` vocabulary, which degrade to the
    /// neutral glyph rather than to a confidently wrong one. New values added
    /// to the protocol later land here too, by construction.
    init?(wire: String?) {
        guard let wire, let kind = DeviceKind(rawValue: wire) else { return nil }
        self = kind
    }

    /// What this device advertises.
    static var current: DeviceKind {
        #if os(iOS)
        UIDevice.current.userInterfaceIdiom == .pad ? .ipad : .iphone
        #else
        .mac
        #endif
    }
}

/// The single place that turns a device type into an SF Symbol. Two sources
/// feed it: the TXT `dt` vocabulary above (authoritative), and the core's
/// coarser form-factor enum arriving on `PeerConnected` / `IncomingOffer`. If
/// the core's enum is ever aligned with PROTOCOL.md §3.1,
/// `symbol(forCoreType:)` is the only edit.
enum DeviceIcon {
    /// Used whenever the device type is unknown or ambiguous. A generic icon
    /// is always acceptable; a confidently wrong one is not (PROTOCOL.md §3.1).
    static let neutral = "questionmark.square.dashed"

    static func symbol(for kind: DeviceKind?) -> String {
        switch kind {
        case .iphone: "iphone"
        case .ipad: "ipad"
        case .mac: "laptopcomputer"
        case .windows: "pc"
        // The user's explicit ask: non-Apple phones get the generic handset.
        case .androidPhone: "smartphone"
        // No neutral tablet glyph ships in this SDK (`tablet` does not exist),
        // and `ipad` would be a lie, so this falls back to neutral.
        case .androidTablet: neutral
        case nil: neutral
        }
    }

    /// Conservative bridge for the core's form-factor enum. `phone` and
    /// `tablet` are ambiguous between Apple and Android hardware, so they get
    /// the neutral glyph; `laptop` and `desktop` map to platform-neutral
    /// computer glyphs, which are honest for any vendor.
    static func symbol(forCoreType type: DeviceType?) -> String {
        switch type {
        case .laptop: "laptopcomputer"
        case .desktop: "desktopcomputer"
        case .phone, .tablet, nil: neutral
        }
    }

    /// Spoken name for the glyph. The glyph is the only place a row states
    /// what kind of device it is, so VoiceOver has to be told in words. An
    /// unrecognised device is "Device", never a plausible-looking guess, for
    /// exactly the reason `neutral` exists.
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
