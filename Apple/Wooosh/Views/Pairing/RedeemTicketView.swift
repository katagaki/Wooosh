import SwiftUI

/// The receiving half of the internet path (PROTOCOL.md §9.4). Scanning is the consent, so
/// there is no second prompt, nothing is paired, and no fingerprint is shown.
struct RedeemTicketView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    @State private var pastedCode = ""

    var body: some View {
        Group {
            switch model.pairingPhase {
            case .connecting(let peerName):
                connecting(peerName: peerName)
            case .success:
                // A hand-off, not a screen.
                Color.clear.onAppear(perform: finish)
            case .failed(let message):
                failure(message)
            case .idle:
                scanner
            }
        }
        .onAppear { model.resetPairingPhase() }
    }

    /// Redeeming is the whole job: the sender then hands the files over on its own.
    private func finish() {
        _ = model.takeRedeemedPeerID()
        model.resetPairingPhase()
        dismiss()
    }

    // MARK: - Scan

    private var scanner: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(spacing: 16) {
                    #if os(iOS)
                    QRScannerView { scanned in
                        model.pairWithScannedCode(scanned)
                    }
                    .frame(maxWidth: .infinity)
                    .frame(height: 300)
                    .clipShape(.rect(cornerRadius: 20))
                    #else
                    // No camera on the Mac, so pasting is the whole flow.
                    Image(systemName: "qrcode.viewfinder")
                        .font(.system(size: 56))
                        .foregroundStyle(.secondary)
                        .padding(.top, 16)
                    #endif

                    Text(L.t("other_device_body"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)

                    TextField(L.t("pairing_paste_label"), text: $pastedCode)
                        .textFieldStyle(.roundedBorder)
                        .font(.system(.caption, design: .monospaced))
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        #endif
                }
                .frame(maxWidth: 460)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 20)
                .padding(.top, 8)
            }
            .scrollBounceBehavior(.basedOnSize)

            SheetActions {
                Button(L.t("action_pair_with_pasted")) {
                    model.pairWithScannedCode(pastedCode)
                }
                .disabled(pastedCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }

    // MARK: - Phases

    private func connecting(peerName: String?) -> some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)
            VStack(spacing: 16) {
                ProgressView()
                    .controlSize(.large)
                Text(peerName.map { L.f("transfer_connecting_to", $0) } ?? L.t("transfer_connecting_generic"))
                    .font(.title3.weight(.semibold))
                    .multilineTextAlignment(.center)
                // Hole punching is slower than the LAN, and silence that long reads as a hang.
                Text(L.t("other_device_connecting_body"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: 420)
            .padding(.horizontal, 24)
            Spacer(minLength: 0)
            SheetActions {
                EmptyView()
            } secondary: {
                CancelButton { model.cancelPairing() }
            }
        }
    }

    private func failure(_ message: String) -> some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)
            VStack(spacing: 16) {
                Image(systemName: "xmark.octagon.fill")
                    .font(.system(size: 56))
                    .foregroundStyle(.red)
                Text(L.t("pairing_failed_title"))
                    .font(.title3.weight(.semibold))
                    .multilineTextAlignment(.center)
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: 420)
            .padding(.horizontal, 24)
            Spacer(minLength: 0)
            SheetActions {
                Button(L.t("action_try_again")) {
                    model.resetPairingPhase()
                    pastedCode = ""
                }
            }
        }
    }
}
