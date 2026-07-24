import Foundation
#if os(iOS)
import UIKit
#endif

/// Keeps the device awake while transfers are active (DESIGN.md §7):
/// - iOS: `UIApplication.isIdleTimerDisabled` while active.
/// - macOS: `ProcessInfo.beginActivity(.idleSystemSleepDisabled)`.
/// Released as soon as no transfer is active.
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
