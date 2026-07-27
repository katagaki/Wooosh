import SwiftUI

/// The action row every sheet pins to its bottom edge. iOS stacks full-width `.extraLarge`
/// buttons (~55 pt, over the 44 pt minimum), primary on top; macOS uses a trailing AppKit
/// footer row. No hard-coded heights, so labels grow with Dynamic Type.
struct SheetActions<Primary: View, Secondary: View>: View {
    @ViewBuilder var primary: Primary
    @ViewBuilder var secondary: Secondary

    /// Whether the primary button is also the sheet's default (Return) action. False for
    /// security confirmations: an unrequested sheet must not inherit a stray keystroke.
    var defaultAction: Bool = true

    init(defaultAction: Bool = true,
         @ViewBuilder primary: () -> Primary,
         @ViewBuilder secondary: () -> Secondary = { EmptyView() }) {
        self.defaultAction = defaultAction
        self.primary = primary()
        self.secondary = secondary()
    }

    var body: some View {
        #if os(macOS)
        HStack(spacing: 12) {
            Spacer(minLength: 0)
            secondary
                .buttonStyle(.glass)
            primary
                .buttonStyle(.glassProminent)
                .keyboardShortcut(defaultAction ? .defaultAction : nil)
        }
        .controlSize(.large)
        .padding(.horizontal, 20)
        .padding(.vertical, 16)
        #else
        VStack(spacing: 10) {
            primary
                .buttonStyle(.glassProminent)
                .keyboardShortcut(defaultAction ? .defaultAction : nil)
            secondary
                .buttonStyle(.glass)
        }
        .buttonSizing(.flexible)
        .controlSize(.extraLarge)
        .padding(.horizontal, 20)
        .padding(.top, 14)
        .padding(.bottom, 12)
        #endif
    }
}

// MARK: - Standard-role buttons
//
// Label-less `Button(role:action:)` lets the system supply the title, glyph and glass.
// iOS only: macOS sheet footers and toolbars are rows of spelled-out titles.

struct DoneButton: View {
    private let action: () -> Void

    init(action: @escaping () -> Void) { self.action = action }

    var body: some View {
        #if os(iOS)
        Button(role: .confirm, action: action)
        #else
        Button(L.t("action_done"), action: action)
        #endif
    }
}

/// `.cancel` carries the system and assistive-technology semantics (Escape on macOS)
/// that a plain titled button does not.
struct CancelButton: View {
    private let action: () -> Void

    init(action: @escaping () -> Void) { self.action = action }

    var body: some View {
        #if os(iOS)
        Button(role: .cancel, action: action)
        #else
        Button(L.t("action_cancel"), role: .cancel, action: action)
        #endif
    }
}

struct CloseButton: View {
    private let action: () -> Void

    init(action: @escaping () -> Void) { self.action = action }

    var body: some View {
        #if os(iOS)
        Button(role: .close, action: action)
        #else
        Button(L.t("action_close"), action: action)
        #endif
    }
}
