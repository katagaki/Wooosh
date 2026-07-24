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

    /// Shared root of the main window on both platforms.
    private var root: some View {
        DeviceListView()
            .environment(model)
            .task {
                await model.start()
            }
            // The core owns a tokio runtime and a callback thread. Both are
            // torn down explicitly here; letting the FFI object be released
            // implicitly drops the runtime at an arbitrary moment, which is
            // how an FFI app hangs on quit.
            .onReceive(NotificationCenter.default.publisher(for: terminationNotification)) { _ in
                model.shutdown()
            }
    }

    #if os(macOS)
    var body: some Scene {
        WindowGroup {
            root
        }
        // Tall and narrow: the window is a device list, not a canvas. The
        // minimum comes from `DeviceListView`'s own frame, so neither the list
        // nor the sheets it presents can be squeezed until they clip.
        .defaultSize(width: 460, height: 620)
        .windowResizability(.contentMinSize)
        .windowToolbarStyle(.unified)

        // Mac settings belong behind ⌘, in the app menu, not in a sheet inside
        // the window. Same view, different scene — iOS keeps the sheet.
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
