import SwiftUI

/// Transfers with a device that is not on this network (PROTOCOL.md §9). Send and receive
/// are segments of one screen, not a question asked first: switching costs a tap.
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

                // Switching tears the other side down, which ends any live code: a code
                // authorises one transfer and must not outlive the screen showing it.
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
