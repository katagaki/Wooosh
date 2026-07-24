import SwiftUI

/// Shown over the device list while a pairing attempt started from a row is
/// in flight (there is no sheet up to host it).
///
/// Pairing dials a peer across a network and has been measured at 19 s, so it
/// must always name the peer and always offer Cancel. Silence reads as a dead
/// app and gets force-quit.
struct PairingProgressOverlay: View {
    let peerName: String?
    let cancel: () -> Void

    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        ZStack {
            // Scrim, so the list underneath reads as not-interactive.
            Rectangle()
                .fill(.black.opacity(0.18))
                .ignoresSafeArea()

            card
                .frame(maxWidth: 320)
                .padding(24)
        }
        .accessibilityAddTraits(.isModal)
    }

    private var card: some View {
        VStack(spacing: 16) {
            ProgressView()
                .controlSize(.large)
            Text(peerName.map { L.f("transfer_connecting_to", $0) } ?? L.t("transfer_connecting_generic"))
                .font(.headline)
                .multilineTextAlignment(.center)
            Text(L.t("pairing_progress_body"))
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            CancelButton(action: cancel)
                .buttonStyle(.glass)
                .controlSize(.large)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 28)
        .modifier(PairingCardBackground(reduceTransparency: reduceTransparency))
    }
}

/// A floating pane, so it takes real Liquid Glass — except under Reduce
/// Transparency, where it falls back to an opaque surface rather than
/// degrading into something hard to read.
private struct PairingCardBackground: ViewModifier {
    let reduceTransparency: Bool

    func body(content: Content) -> some View {
        if reduceTransparency {
            content.background(.background, in: .rect(cornerRadius: 28))
        } else {
            content.glassEffect(.regular, in: .rect(cornerRadius: 28))
        }
    }
}
