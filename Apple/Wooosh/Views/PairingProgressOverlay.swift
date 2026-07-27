import SwiftUI

/// Shown over the device list when a row-initiated pairing has no sheet to host it.
/// Pairing has been measured at 19 s, so it must always name the peer and offer Cancel.
struct PairingProgressOverlay: View {
    let peerName: String?
    let cancel: () -> Void

    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        ZStack {
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

/// Real Liquid Glass, except under Reduce Transparency where it falls back to opaque.
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
