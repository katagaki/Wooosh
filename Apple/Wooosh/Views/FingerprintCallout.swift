import SwiftUI

/// This device's own 6-word fingerprint, shown to the *sender* while an
/// unpaired receiver is deciding whether to accept.
///
/// The receiver's consent sheet asks the user to verify the sender's
/// fingerprint (PROTOCOL.md §2). That check is only performable if the sending
/// device actually puts the phrase on screen at the same moment — otherwise
/// the UI is asking for a comparison against nothing. So: large, monospaced,
/// selectable, and readable across a desk, exactly like the SAS code.
///
/// The phrase comes from the core (`fingerprintPhrase()`, surfaced by
/// `AppModel`); it is never re-derived in Swift.
struct FingerprintCallout: View {
    let phrase: String

    var body: some View {
        VStack(spacing: 8) {
            Label(L.t("verify_your_phrase_title"), systemImage: "person.badge.shield.checkmark")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.secondary)

            Text(phrase)
                .font(.system(.title3, design: .monospaced, weight: .semibold))
                .multilineTextAlignment(.center)
                .textSelection(.enabled)
                .lineLimit(3)
                .minimumScaleFactor(0.6)

            Text(L.t("verify_your_phrase_body"))
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 14)
        .padding(.horizontal, 16)
        // System semantic fill, not a hand-rolled material: it stays legible
        // under Reduce Transparency and in both appearances.
        .background(.fill.quaternary, in: .rect(cornerRadius: 16))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(L.f("verify_your_phrase_a11y", phrase))
    }
}
