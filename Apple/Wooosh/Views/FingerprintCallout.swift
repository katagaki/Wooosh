import SwiftUI

/// The receiver is asked to verify this phrase (PROTOCOL.md §2), so the sender must have
/// it on screen at that moment, readable across a desk, and always sourced from the core.
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
        // System semantic fill: stays legible under Reduce Transparency and both appearances.
        .background(.fill.quaternary, in: .rect(cornerRadius: 16))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(L.f("verify_your_phrase_a11y", phrase))
    }
}
