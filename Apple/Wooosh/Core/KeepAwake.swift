import Foundation
#if os(iOS)
import UIKit
#endif

/// Held while a transfer is active, released as soon as none is (DESIGN.md §7).
@MainActor
enum KeepAwake {
    #if os(macOS)
    private static var activityToken: NSObjectProtocol?
    #endif

    static func setActive(_ active: Bool) {
        #if os(iOS)
        UIApplication.shared.isIdleTimerDisabled = active
        #else
        if active {
            guard activityToken == nil else { return }
            activityToken = ProcessInfo.processInfo.beginActivity(
                options: .idleSystemSleepDisabled,
                reason: "Wooosh file transfer in progress"
            )
        } else if let token = activityToken {
            ProcessInfo.processInfo.endActivity(token)
            activityToken = nil
        }
        #endif
    }
}
