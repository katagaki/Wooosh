import SwiftUI
#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// "Pair a device" (PROTOCOL.md §4.2): show our QR payload for the other
/// device to scan, or scan/paste theirs.
struct PairDeviceView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    /// Two tabs, split by role rather than by transport: showing a code and
    /// scanning one are the two things a person can do here. Which *kind* of
    /// code is a choice inside "Show", because it is the same act either way.
    private enum Mode: String, CaseIterable, Identifiable {
        case show
        case scan
        var id: String { rawValue }
        var label: String {
            switch self {
            case .show: L.t("pairing_tab_my_code")
            case .scan: L.t("pairing_tab_scan")
            }
        }
    }

    @State private var mode: Mode = .show
    @State private var payload = ""
    @State private var pastedPayload = ""
    @State private var copied = false

    var body: some View {
        NavigationStack {
            Group {
                switch model.pairingPhase {
                case .connecting(let peerName):
                    pairingProgress(peerName: peerName)
                case .success(let peer):
                    pairingSuccess(peer)
                case .failed(let message):
                    pairingFailure(message)
                case .idle:
                    content
                }
            }
            .navigationTitle(L.t("pairing_title"))
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    CloseButton { dismiss() }
                }
            }
        }
        .onAppear {
            // No-op while an attempt is in flight, so reopening the sheet
            // cannot drop the user back into a silent screen.
            model.resetPairingPhase()
            payload = model.beginPairingQR()
        }
        #if os(macOS)
        .frame(minWidth: 460, minHeight: 560)
        #else
        // The QR needs the full sheet; the status screens are a few lines and
        // look abandoned in it, so they get a half sheet instead.
        .presentationDetents(model.pairingPhase == .idle ? [.large] : [.medium, .large])
        #endif
    }

    private var content: some View {
        VStack(spacing: 16) {
            Picker(L.t("pairing_title"), selection: $mode) {
                ForEach(Mode.allCases) { mode in
                    Text(mode.label).tag(mode)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal)
            .padding(.top, 8)

            switch mode {
            case .show: showCode
            case .scan: scanCode
            }
        }
    }

    // MARK: - Show a code

    private var showCode: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(spacing: 16) {
                    QRCodeView(payload: payload)
                        .frame(width: 240, height: 240)
                        .padding(14)
                        // The code has to stay high-contrast for scanners, so
                        // this one surface is deliberately opaque white rather
                        // than glass.
                        .background(.white, in: .rect(cornerRadius: 20))

                    Text(L.t("pairing_show_body"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)

                    Text(L.t("pairing_show_expiry"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)

                    Text(payload)
                        .font(.system(.caption2, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .background(.fill.quaternary, in: .rect(cornerRadius: 12))
                }
                .frame(maxWidth: 420)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 20)
                .padding(.top, 8)
            }
            .scrollBounceBehavior(.basedOnSize)

            SheetActions {
                Button(L.t(copied ? "action_copied" : "action_copy_code"),
                       systemImage: copied ? "checkmark" : "doc.on.doc") {
                    copyPayload()
                }
            } secondary: {
                Button(L.t("action_new_code"), systemImage: "arrow.clockwise") {
                    payload = model.beginPairingQR()
                }
            }
        }
    }

    private func copyPayload() {
        #if os(iOS)
        UIPasteboard.general.string = payload
        #else
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(payload, forType: .string)
        #endif
        copied = true
        Task {
            try? await Task.sleep(for: .seconds(2))
            copied = false
        }
    }

    // MARK: - Scan / paste their code

    private var scanCode: some View {
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
                    Text(L.t("pairing_scan_focus_hint"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                    #else
                    Image(systemName: "qrcode.viewfinder")
                        .font(.system(size: 56))
                        .foregroundStyle(.secondary)
                        .padding(.top, 16)
                    Text(L.t("pairing_scan_body_desktop"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                    #endif

                    // The scanner takes both kinds of code and picks the path
                    // itself, so say so rather than making the user work out
                    // which tab a code they were sent belongs to.
                    Text(L.t("pairing_scan_accepts"))
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)

                    TextField(L.t("pairing_paste_label"), text: $pastedPayload)
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
                    model.pairWithScannedCode(pastedPayload)
                }
                .disabled(pastedPayload.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }

    // MARK: - Phases

    /// Must always offer a way out: a user who cannot cancel a long connect
    /// force-quits the app instead.
    private func pairingProgress(peerName: String?) -> some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)
            VStack(spacing: 16) {
                ProgressView()
                    .controlSize(.large)
                Text(peerName.map { L.f("transfer_connecting_to", $0) } ?? L.t("transfer_connecting_generic"))
                    .font(.title3.weight(.semibold))
                    .multilineTextAlignment(.center)
                Text(L.t("pairing_progress_body"))
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

    private func pairingSuccess(_ peer: PeerRef) -> some View {
        phaseResult(
            symbol: "checkmark.seal.fill",
            tint: .green,
            title: L.f("pairing_success_title", peer.displayName),
            message: L.t("pairing_success_body")
        ) {
            DoneButton { dismiss() }
        }
    }

    private func pairingFailure(_ message: String) -> some View {
        phaseResult(
            symbol: "xmark.octagon.fill",
            tint: .red,
            title: L.t("pairing_failed_title"),
            message: message
        ) {
            Button(L.t("action_try_again")) {
                model.resetPairingPhase()
                pastedPayload = ""
            }
        }
    }

    private func phaseResult<Action: View>(
        symbol: String, tint: Color, title: String, message: String,
        @ViewBuilder action: () -> Action
    ) -> some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)
            VStack(spacing: 16) {
                Image(systemName: symbol)
                    .font(.system(size: 56))
                    .foregroundStyle(tint)
                Text(title)
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
            SheetActions(primary: action)
        }
    }
}
