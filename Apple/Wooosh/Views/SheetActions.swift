import SwiftUI

/// The action row every sheet pins to its bottom edge.
///
/// One place decides what a sheet's primary action looks like, and it decides
/// it *per platform* — the two conventions are genuinely different and the
/// same layout cannot serve both:
///
/// - **iOS**: stacked, full width (`.buttonSizing(.flexible)`), `.extraLarge`
///   control size — around 55 pt tall at the default type size, well over the
///   44 pt minimum, and thumb-reachable at the bottom of the sheet. Primary on
///   top.
/// - **macOS**: a trailing-aligned row of normally-sized Mac buttons, primary
///   rightmost, secondary to its left — the standard AppKit sheet footer.
///   Full-width extra-large buttons look like a phone on a Mac.
///
/// Nothing hard-codes a height, so buttons grow with Dynamic Type instead of
/// clipping their labels. The roles are explicit parameters rather than a flat
/// builder precisely because the two platforms order them differently.
struct SheetActions<Primary: View, Secondary: View>: View {
    @ViewBuilder var primary: Primary
    @ViewBuilder var secondary: Secondary

    /// Whether the primary button is also the sheet's default (Return) action.
    ///
    /// True for ordinary sheets, where Return doing the obvious thing is a
    /// convenience. **False for security confirmations.** A sheet the user did
    /// not ask for — an inbound SAS request arrives from the *other* device and
    /// presents itself the instant the event lands — must not inherit a Return
    /// keystroke that was aimed at whatever was on screen a moment earlier. See
    /// `SASSheet`.
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
// iOS 26 added `Button(role:action:)` — a button with a role and *no* label,
// for which the system supplies the standard title/glyph and the Liquid Glass
// treatment it gives its own Done/Cancel/Close. Verified against
// iPhoneOS27.0.sdk: `ButtonRole.confirm` and `.close` are `iOS 26.0, macOS
// 26.0+`, and the label-less initialiser is `extension Button where Label ==
// DefaultButtonLabel { init(role: ButtonRole, action: ...) }`, same
// availability. The app's floor is 26 on both, so no `#available` is needed.
//
// Scoped to iOS on purpose, the same way this file already scopes sheet button
// metrics: on macOS these buttons live in an AppKit sheet footer or a window
// toolbar, both of which are rows of *spelled-out* titles. A system-supplied
// glyph in that row reads as a different control, so the Mac keeps its words.

/// The bare confirmation button — "Done".
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

/// The bare cancel button.
///
/// `.cancel` also carries the semantics assistive technology and the system
/// expect of a cancel button (Escape on macOS), which a plain titled button
/// does not.
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

/// The bare dismissal button for a sheet that neither confirms nor cancels
/// anything — "Close".
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
