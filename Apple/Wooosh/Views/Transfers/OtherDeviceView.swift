import SwiftUI

/// Transfers with a device that is not on this network (PROTOCOL.md §9).
///
/// Sending and receiving are two directions of one job, so they are two
/// segments of a single screen rather than a question asked before the screen
/// opens. Asking first makes the user commit to a direction while looking at
/// nothing, and gets it wrong often enough that the dialog was really a
/// speed bump: here both directions are visible and switching costs a tap.
struct OtherDeviceView: View {
    @Environment(\.dismiss) private var dismiss

    private enum Direction: Hashable {
        case send
        case receive
    }

    @State private var direction: Direction = .send

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                Picker(L.t("other_device_title"), selection: $direction) {
                    Text(L.t("other_device_send")).tag(Direction.send)
                    Text(L.t("other_device_receive")).tag(Direction.receive)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .padding(.horizontal, 20)
                .padding(.top, 12)

                // Each side owns its own state and its own bottom actions. The
                // switch tears the other one down, which is what ends a live
                // code: it authorises one transfer and must not outlive the
                // screen that is showing it.
                switch direction {
                case .send:
                    SendOverInternetView()
                case .receive:
                    RedeemTicketView()
                }
            }
            .navigationTitle(L.t("other_device_title"))
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    CloseButton { dismiss() }
                }
            }
        }
        #if os(macOS)
        .frame(minWidth: 460, minHeight: 560)
        #endif
    }
}
