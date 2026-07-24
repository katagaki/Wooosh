import SwiftUI

/// SAS numeric-comparison sheet (PROTOCOL.md §4.3): both devices display a
/// 6-digit code derived from the TLS exporter secret; the user visually
/// compares. 60 s timeout, mismatch or timeout aborts without storing keys.
///
/// The code is symmetric — both devices show the same thing at the same time —
/// so, unlike the offer sheet, nothing here asks the user to compare against
/// something the other screen isn't showing.
struct SASSheet: View {
    let request: SASRequest

    @Environment(AppModel.self) private var model

    static let timeout: TimeInterval = 60

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                explanation
            }
            .scrollBounceBehavior(.basedOnSize)

            if model.sasConfirming {
                ProgressView(L.t("sas_confirming"))
                    .padding(.vertical, 24)
            } else {
                // `defaultAction: false` is the security fix, not a style
                // choice. This sheet is presented by an event from the *other*
                // device (`AppModel.handle(event: .pairingSAS)`), so it can
                // appear under the user's hands at any moment, and
                // `.interactiveDismissDisabled()` means keystrokes have nowhere
                // else to go. With the primary wired to `.defaultAction`, a
                // Return aimed at whatever was on screen a moment earlier lands
                // on "Codes Match" and confirms the pairing — the exact failure
                // SAS exists to prevent. Comparing two codes is a deliberate
                // human act; it must cost a deliberate press, never an
                // inherited one.
                SheetActions(defaultAction: false) {
                    Button(L.t("action_codes_match")) {
                        model.confirmSAS(accepted: true)
                    }
                } secondary: {
                    CancelButton { model.confirmSAS(accepted: false) }
                }
            }
        }
        .interactiveDismissDisabled()
        #if os(macOS)
        .frame(minWidth: 420, minHeight: 440)
        #else
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.hidden)
        #endif
    }

    private var explanation: some View {
        VStack(spacing: 16) {
            Image(systemName: request.peer.symbolName)
                .font(.system(size: 40))
                .foregroundStyle(.tint)
                .padding(.top, 8)

            Text(L.f("sas_title", request.peer.displayName))
                .font(.title3.weight(.semibold))
                .multilineTextAlignment(.center)

            Text(L.t("sas_body"))
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            Text(formattedCode)
                .font(.system(size: 44, weight: .semibold, design: .monospaced))
                .kerning(2)
                .minimumScaleFactor(0.5)
                .lineLimit(1)
                .textSelection(.enabled)
                .padding(.vertical, 4)

            countdown
        }
        .frame(maxWidth: 420)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 24)
        .padding(.top, 24)
        .padding(.bottom, 8)
    }

    private var formattedCode: String {
        let digits = request.sixDigits
        guard digits.count == 6 else { return digits }
        return digits.prefix(3) + " " + digits.suffix(3)
    }

    private var countdown: some View {
        TimelineView(.periodic(from: request.startedAt, by: 1)) { context in
            let remaining = max(0, Int(Self.timeout - context.date.timeIntervalSince(request.startedAt)))
            Text(L.f("sas_expires_in", remaining))
                .font(.caption)
                .foregroundStyle(remaining <= 10 ? .red : .secondary)
                .monospacedDigit()
                .onChange(of: remaining) { _, newValue in
                    if newValue <= 0 && !model.sasConfirming {
                        model.confirmSAS(accepted: false)
                    }
                }
        }
    }
}
