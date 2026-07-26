#if os(iOS)
import Foundation
import Observation
import os
import QuickLook
import SwiftUI
import UIKit
import UserNotifications

/// The "files arrived" notification and what tapping it opens.
///
/// Posted once a received transfer has finished and every file has been routed
/// (DESIGN.md §6). Tapping opens the file itself: a document through Quick Look
/// inside Wooosh, a photo or video by handing off to Photos, which is as far as
/// add-only library access can go. Wooosh never holds a readable URL for
/// anything it put in the photo library, so there is nothing to preview here.
@MainActor
@Observable
final class ReceiptNotifier: NSObject, UNUserNotificationCenterDelegate {

    static let shared = ReceiptNotifier()

    /// Set when the user taps a notification for a received document. The root
    /// view presents Quick Look on it and clears it.
    var previewURL: URL?

    @ObservationIgnored
    private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "notifications")
    @ObservationIgnored private var didRequestAuthorization = false

    private override init() {
        super.init()
        UNUserNotificationCenter.current().delegate = self
    }

    /// Asked for the first time when a transfer is actually incoming, not at
    /// launch: a permission prompt means more when the thing it is for is
    /// about to happen.
    func requestAuthorizationIfNeeded() {
        guard !didRequestAuthorization else { return }
        didRequestAuthorization = true
        Task {
            do {
                _ = try await UNUserNotificationCenter.current()
                    .requestAuthorization(options: [.alert, .sound])
            } catch {
                logger.error("Notification authorization failed: \(error.localizedDescription)")
            }
        }
    }

    func notifyReceived(_ transfer: Transfer) {
        let saved = transfer.files.filter {
            if case .saved = $0.status { return true } else { return false }
        }
        guard !saved.isEmpty else { return }

        let content = UNMutableNotificationContent()
        content.title = L.f("notification_received_title", saved.count)
        content.body = saved.count == 1
            ? saved[0].name
            : L.f("notification_received_from", transfer.peer.displayName)
        content.sound = .default
        // Only a lone file has an unambiguous thing to open. Several at once
        // just bring Wooosh forward, where the transfer card lists them.
        if let single = saved.first, saved.count == 1 {
            content.userInfo = single.savedURL.map { [Self.pathKey: $0.path] }
                ?? [Self.photosKey: true]
        }

        let request = UNNotificationRequest(
            identifier: "received-\(transfer.id.raw)",
            content: content,
            trigger: nil
        )
        UNUserNotificationCenter.current().add(request) { [logger] error in
            if let error { logger.error("Could not post notification: \(error.localizedDescription)") }
        }
    }

    // MARK: - UNUserNotificationCenterDelegate

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        // Read out of the payload here: `userInfo` is `[AnyHashable: Any]` and
        // so cannot cross into the main actor, but these two can.
        let userInfo = response.notification.request.content.userInfo
        let savedPath = userInfo[Self.pathKey] as? String
        let inPhotos = userInfo[Self.photosKey] as? Bool == true
        await MainActor.run {
            if let path = savedPath {
                let url = URL(fileURLWithPath: path)
                // The user may have moved or deleted it from Files since.
                guard FileManager.default.fileExists(atPath: path) else {
                    logger.notice("Received file is no longer at its saved path.")
                    return
                }
                previewURL = url
            } else if inPhotos {
                openPhotos()
            }
        }
    }

    /// Wooosh is already on screen, so a banner over its own transfer card
    /// would be noise. The card is the notification in that case.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        []
    }

    // MARK: - Internals

    /// There is no public API to open the photo library at a specific asset,
    /// and add-only access cannot read one back to show it here either, so the
    /// most Wooosh can do is bring Photos forward.
    private func openPhotos() {
        guard let url = URL(string: "photos-redirect://") else { return }
        UIApplication.shared.open(url) { [logger] opened in
            if !opened { logger.notice("Could not open Photos.") }
        }
    }

    // Read by the delegate callback, which the system invokes off the main actor.
    private nonisolated static let pathKey = "savedPath"
    private nonisolated static let photosKey = "inPhotos"
}

/// Presents Quick Look on whatever document the user tapped a notification for.
/// Applied once at the root so the preview survives whichever screen is up.
struct ReceivedFilePreview: ViewModifier {
    @Bindable private var notifier = ReceiptNotifier.shared

    func body(content: Content) -> some View {
        content.quickLookPreview($notifier.previewURL)
    }
}
#endif
