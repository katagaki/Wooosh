import SwiftUI

/// Consent sheet for an incoming OFFER (DESIGN.md §5, PROTOCOL.md §4.4):
/// sender identity + trust state, file list with sizes and total, then
/// Accept (paired) / Accept Once (unpaired) / Decline. After acceptance the
/// sheet becomes the live progress view.
struct IncomingOfferSheet: View {
    let offer: Transfer

    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Group {
                if offer.state == .awaitingConsent {
                    consent
                } else {
                    TransferProgressView(transfer: offer) { dismiss() }
                }
            }
            .navigationTitle(L.t(offer.state == .awaitingConsent ? "offer_nav_incoming" : "offer_nav_receiving"))
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
        }
        .interactiveDismissDisabled(offer.state == .awaitingConsent || offer.isActive)
        #if os(macOS)
        .frame(minWidth: 460, minHeight: 520)
        #endif
    }

    private var consent: some View {
        VStack(spacing: 0) {
            List {
                Section {
                    VStack(spacing: 10) {
                        Image(systemName: offer.peer.symbolName)
                            .font(.system(size: 40))
                            .foregroundStyle(.tint)
                        Text(L.f("offer_title", offer.peer.displayName, offer.files.count))
                            .font(.title3.weight(.semibold))
                            .multilineTextAlignment(.center)
                        trustLine
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                    .listRowSeparator(.hidden)
                    #if os(iOS)
                    .listRowInsets(EdgeInsets(top: 8, leading: 0, bottom: 8, trailing: 0))
                    #endif
                }

                Section(L.f("files_count_and_size", offer.files.count, TransferFormat.bytes(offer.totalBytes))) {
                    ForEach(offer.files) { file in
                        HStack(spacing: 12) {
                            Image(systemName: symbolName(for: file))
                                .foregroundStyle(.tint)
                                .frame(width: 28)
                            Text(file.name)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Spacer(minLength: 8)
                            Text(TransferFormat.bytes(file.size))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
            #if os(iOS)
            .listStyle(.insetGrouped)
            #endif

            // Pinned below the scrolling list — the decision is always
            // reachable, however long the manifest is.
            SheetActions {
                Button(L.t(offer.peerWasPaired ? "action_accept" : "action_accept_once")) {
                    model.transfers.accept(offer: offer)
                }
            } secondary: {
                Button(L.t("action_decline"), role: .destructive) {
                    model.transfers.decline(offer: offer)
                    dismiss()
                }
            }
        }
    }

    @ViewBuilder
    private var trustLine: some View {
        if offer.peerWasPaired {
            Label(L.t("offer_paired_device"), systemImage: "checkmark.seal.fill")
                .font(.subheadline)
                .foregroundStyle(.green)
        } else {
            VStack(spacing: 6) {
                Label(L.t("offer_unpaired_warning"),
                      systemImage: "exclamationmark.shield")
                    .font(.subheadline)
                    .foregroundStyle(.orange)
                    .multilineTextAlignment(.center)
                // The sender now shows exactly this phrase while it waits for
                // this decision (see `TransferProgressView`), so the
                // comparison the copy above asks for is actually performable.
                Text(offer.peer.fingerprint)
                    .font(.system(.body, design: .monospaced, weight: .semibold))
                    .multilineTextAlignment(.center)
                    .minimumScaleFactor(0.6)
                    .textSelection(.enabled)
                    .padding(.vertical, 8)
                    .padding(.horizontal, 14)
                    .frame(maxWidth: .infinity)
                    .background(.fill.quaternary, in: .rect(cornerRadius: 14))
            }
        }
    }

    private func symbolName(for file: Transfer.File) -> String {
        switch FileKind(mime: file.mime) {
        case .photo: "photo"
        case .video: "film"
        case .document: "doc"
        }
    }
}
