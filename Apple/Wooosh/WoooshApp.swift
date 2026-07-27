import SwiftUI
#if os(iOS)
import UIKit
#else
import AppKit
#endif

@main
struct WoooshApp: App {
    @State private var model = AppModel()

    private var terminationNotification: Notification.Name {
        #if os(iOS)
        UIApplication.willTerminateNotification
        #else
        NSApplication.willTerminateNotification
        #endif
    }

    private var root: some View {
        DeviceListView()
            .environment(model)
            .task {
                await model.start()
            }
            // Explicit teardown: releasing the FFI object implicitly drops the
            // tokio runtime at an arbitrary moment, which is how FFI apps hang on quit.
            .onReceive(NotificationCenter.default.publisher(for: terminationNotification)) { _ in
                model.shutdown()
            }
        #if os(iOS)
            .modifier(ReceivedFilePreview())
        #endif
    }

    #if os(macOS)
    var body: some Scene {
        WindowGroup {
            root
        }
        // The window is a device list, not a canvas. The minimum comes from
        // `DeviceListView`'s frame, so the list and its sheets cannot be clipped.
        .defaultSize(width: 460, height: 620)
        .windowResizability(.contentMinSize)
        .windowToolbarStyle(.unified)

        // Mac settings live behind ⌘, in the app menu; iOS keeps the sheet.
        Settings {
            SettingsView()
                .environment(model)
        }
    }
    #else
    var body: some Scene {
        WindowGroup {
            root
        }
    }
    #endif
}
