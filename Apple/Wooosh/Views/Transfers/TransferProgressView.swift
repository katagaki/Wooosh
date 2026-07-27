import SwiftUI

struct TransferProgressView: View {
    let transfer: Transfer
    var onDone: () -> Void = {}

    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 0) {
            header
                .padding(.horizontal)
                .padding(.vertical, 12)

            if showSenderFingerprint {
                FingerprintCallout(phrase: model.fingerprintPhrase)
                    .padding(.horizontal)
                    .padding(.bottom, 12)
            }

            Divider()

            List {
                ForEach(transfer.files) { file in
                    TransferFileRow(file: file)
                }
            }
            #if os(iOS)
            .listStyle(.plain)
            #endif

            footer
        }
    }

    /// The receiver is told to check the sender's fingerprint, so the sender must show it
    /// during exactly that window: outgoing, unpaired, still awaiting the decision.
    private var showSenderFingerprint: Bool {
        transfer.direction == .outgoing
            && !transfer.peerWasPaired
            && transfer.state == .connecting
            && !model.fingerprintPhrase.isEmpty
    }

    private var header: some View {
        HStack(spacing: 12) {
            Image(systemName: transfer.direction == .outgoing
                  ? "arrow.up.circle.fill" : "arrow.down.circle.fill")
                .font(.title)
                .foregroundStyle(Color.accentColor)
            VStack(alignment: .leading, spacing: 2) {
                Text(L.f(transfer.direction == .outgoing
                         ? "transfer_sending_to" : "transfer_receiving_from",
                         transfer.peer.displayName))
                    .font(.headline)
                Text(stateLine)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
        }
    }

    private var stateLine: String {
        switch transfer.state {
        case .awaitingConsent: L.t("transfer_state_awaiting_consent")
        case .connecting: L.t("transfer_state_connecting")
        case .transferring: transferringLine
        case .done(let summary): doneLine(summary)
        case .failed(let message, let resumable):
            resumable ? L.f("transfer_resumable", message) : message
        case .cancelled: L.t("transfer_state_cancelled")
        }
    }

    /// Counts inflect through the String Catalog; elapsed time is a second placeholder
    /// in the same format string, never a fragment glued on afterwards.
    private func doneLine(_ summary: TransferSummary) -> String {
        let sent = transfer.direction == .outgoing
        guard summary.duration > 0 else {
            return L.f(sent ? "transfer_done_sent" : "transfer_done_received", summary.fileCount)
        }
        return L.f(sent ? "transfer_done_sent_in" : "transfer_done_received_in",
                   summary.fileCount, TransferFormat.duration(summary.duration))
    }

    /// Self-contained system-formatted measurements joined by a separator, never a sentence.
    private var transferringLine: String {
        var parts = [
            L.f("transfer_progress_bytes",
                TransferFormat.bytes(transfer.transferredBytes),
                TransferFormat.bytes(transfer.totalBytes))
        ]
        if transfer.rate > 0 {
            parts.append(TransferFormat.rate(transfer.rate))
        }
        if let eta = transfer.eta, eta > 0.5 {
            parts.append(L.f("transfer_progress_eta", TransferFormat.duration(eta)))
        }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder
    private var footer: some View {
        if transfer.isActive {
            ProgressView(value: transfer.overallFraction)
                .padding(.horizontal, 20)
                .padding(.top, 14)
            // Destructive, so never the prominent button you hit by reflex.
            SheetActions {
                EmptyView()
            } secondary: {
                Button(L.t("action_cancel_transfer"), role: .destructive) {
                    model.transfers.cancel(transfer)
                }
            }
        } else {
            SheetActions {
                DoneButton { onDone() }
            }
        }
    }
}

struct TransferFileRow: View {
    let file: Transfer.File

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: symbolName)
                .foregroundStyle(Color.accentColor)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 3) {
                Text(file.name)
                    .font(.body)
                    .lineLimit(1)
                    .truncationMode(.middle)
                switch file.status {
                case .transferring:
                    ProgressView(value: file.fraction)
                        .controlSize(.small)
                default:
                    Text(statusLine)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            statusIcon
        }
        .padding(.vertical, 2)
    }

    private var statusLine: String {
        switch file.status {
        case .pending: TransferFormat.bytes(file.size)
        case .transferring: ""
        case .completed: TransferFormat.bytes(file.size)
        case .saved(let destination): L.f("transfer_saved_to", TransferFormat.bytes(file.size), destination)
        case .failed(let message): message
        }
    }

    @ViewBuilder
    private var statusIcon: some View {
        switch file.status {
        case .pending:
            Image(systemName: "circle.dotted").foregroundStyle(.tertiary)
        case .transferring:
            EmptyView()
        case .completed, .saved:
            Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
        case .failed:
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.red)
        }
    }

    private var symbolName: String {
        switch FileKind(mime: file.mime) {
        case .photo: "photo"
        case .video: "film"
        case .document: "doc"
        }
    }
}

enum TransferFormat {
    static func bytes(_ value: Int64) -> String {
        value.formatted(.byteCount(style: .file))
    }

    static func rate(_ bytesPerSecond: Double) -> String {
        L.f("transfer_progress_rate", bytes(Int64(bytesPerSecond)))
    }

    /// The system units formatter, so unit names and spacing follow the reader's locale.
    static func duration(_ seconds: TimeInterval) -> String {
        let rounded = max(seconds.rounded(), 1)
        let allowed: Set<Duration.UnitsFormatStyle.Unit> =
            rounded < 60 ? [.seconds] : (rounded < 3600 ? [.minutes] : [.hours, .minutes])
        return Duration.seconds(rounded)
            .formatted(.units(allowed: allowed, width: .abbreviated))
    }
}
