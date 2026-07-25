import SwiftUI

/// Sending to a device that is not on this network (PROTOCOL.md §9).
///
/// The sender presents a code and waits; the recipient scans it and downloads.
/// **Nothing is paired.** The code authorises one transfer and dies with it, so
/// no fingerprint is shown: there is no prior relationship to check one
/// against, and asking the user to verify something they cannot is worse than
/// asking nothing.
struct SendOverInternetView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    @State private var picking = false
    @State private var ticket: String?
    @State private var minting = false
    @State private var failed = false
    @State private var sent = false

    var body: some View {
        NavigationStack {
            content
                .navigationTitle(L.t("internet_send_title"))
                #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
                #endif
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        CloseButton { dismiss() }
                    }
                }
        }
        .fileImporter(isPresented: $picking, allowedContentTypes: [.item],
                      allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result, !urls.isEmpty else { return }
            guard model.stageInternetSend(urls: urls) else {
                failed = true
                return
            }
            mint()
        }
        // The code is a live capability, so it dies with the screen rather than
        // lingering until its expiry.
        .onDisappear { if ticket != nil || minting { model.endInternetTicket() } }
        // The whole point of the screen: someone scanned, so hand the files over.
        .onChange(of: model.ticketRedeemedPeerID) { _, peerID in
            guard let peerID, !sent else { return }
            sent = true
            model.completeInternetSend(to: peerID)
        }
        #if os(macOS)
        .frame(minWidth: 460, minHeight: 520)
        #endif
    }

    @ViewBuilder
    private var content: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(spacing: 16) {
                    if sent {
                        status(symbol: "checkmark.circle.fill", tint: .green,
                               title: L.t("internet_send_started_title"),
                               body: L.t("internet_send_started_body"))
                    } else if let ticket {
                        QRCodeView(payload: ticket, accessibilityKey: "internet_qr_a11y")
                            .frame(width: 240, height: 240)
                            .padding(14)
                            .background(.white, in: .rect(cornerRadius: 20))
                        Text(L.t("internet_send_ready_body"))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                        ProgressView()
                            .controlSize(.small)
                        Text(L.t("internet_send_waiting"))
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    } else if minting {
                        ProgressView()
                            .controlSize(.large)
                            .padding(.top, 40)
                        Text(L.t("internet_preparing"))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    } else {
                        status(symbol: "globe", tint: .secondary,
                               title: L.t("internet_send_title"),
                               body: L.t("internet_send_intro_body"))
                        Text(L.t("internet_relay_note"))
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                    }
                    if failed {
                        Text(L.t("error_ticket_failed"))
                            .font(.callout)
                            .foregroundStyle(.red)
                            .multilineTextAlignment(.center)
                    }
                }
                .frame(maxWidth: 420)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 20)
                .padding(.top, 8)
            }
            .scrollBounceBehavior(.basedOnSize)

            SheetActions {
                if sent {
                    DoneButton { dismiss() }
                } else if ticket == nil {
                    Button(L.t("action_choose_files"), systemImage: "doc.badge.plus") {
                        picking = true
                    }
                    .disabled(minting)
                } else {
                    CancelButton { dismiss() }
                }
            }
        }
    }

    private func status(symbol: String, tint: Color, title: String, body: String) -> some View {
        VStack(spacing: 16) {
            Image(systemName: symbol)
                .font(.system(size: 56))
                .foregroundStyle(tint)
                .padding(.top, 16)
            Text(title)
                .font(.title3.weight(.semibold))
                .multilineTextAlignment(.center)
            Text(body)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
    }

    private func mint() {
        minting = true
        failed = false
        Task {
            defer { minting = false }
            do {
                ticket = try await model.beginInternetTicket()
            } catch {
                failed = true
            }
        }
    }
}
