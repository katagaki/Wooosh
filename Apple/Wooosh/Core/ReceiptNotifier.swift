#if os(iOS)
import Foundation
import Observation
import os
import QuickLook
import SwiftUI
import UIKit
import UserNotifications

/// Posted once a transfer is finished *and* routed (DESIGN.md §6). A document
/// opens in Quick Look; a photo hands off, add-only access leaving no URL.
@MainActor
@Observable
final class ReceiptNotifier: NSObject, UNUserNotificationCenterDelegate {

    static let shared = ReceiptNotifier()

    var previewURL: URL?

    @ObservationIgnored
    private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "notifications")
    @ObservationIgnored private var didRequestAuthorization = false

    private override init() {
        super.init()
        UNUserNotificationCenter.current().delegate = self
    }

    /// Asked when a transfer is incoming, so the prompt has visible cause.
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
        // Only a lone file has an unambiguous thing to open.
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
        // `userInfo` is `[AnyHashable: Any]` and cannot cross actors; these can.
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

    /// Wooosh is already on screen: the transfer card is the notification.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        []
    }

    // MARK: - Internals

    /// No public API opens the library at an asset, so this is the most possible.
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

struct ReceivedFilePreview: ViewModifier {
    @Bindable private var notifier = ReceiptNotifier.shared

    func body(content: Content) -> some View {
        content.quickLookPreview($notifier.previewURL)
    }
}
#endif
