#if os(iOS)
import BackgroundTasks
import Foundation
import os
import UIKit

/// `BGContinuedProcessingTask` is the only mechanism an unprivileged app has for
/// work outliving the foreground (DESIGN.md §7). It renders the Lock Screen and
/// Dynamic Island progress itself, hence no Live Activity. One task per receive
/// session, because the scheduler caps concurrent tasks.
@MainActor
final class BackgroundTransferTask {

    struct Snapshot: Equatable {
        var peerName: String
        var transferCount: Int
        var completedBytes: Int64
        var totalBytes: Int64
        var rate: Double

        var percent: Int {
            guard totalBytes > 0 else { return 0 }
            return Int((completedBytes * 100) / totalBytes).clamped(to: 0...100)
        }
    }

    private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "background")

    private var task: BGContinuedProcessingTask?
    /// Set while a session runs, so a second transfer joins instead of competing.
    private var sessionIdentifier: String?
    private var onStop: (() -> Void)?
    private var lastPush: TimeInterval = 0

    // MARK: - Lifecycle

    /// `onStop` covers both reclaim and the Stop button; they are indistinguishable.
    func begin(_ snapshot: Snapshot, onStop: @escaping () -> Void) {
        guard sessionIdentifier == nil else {
            update(snapshot)
            return
        }
        // Submission requires a foregrounded app; there is no fallback to try.
        guard UIApplication.shared.applicationState != .background else {
            logger.notice("Not submitting a background task: app is not foregrounded.")
            return
        }

        // Fresh id per session: `register` seeing one twice kills the app.
        let identifier = "\(Self.identifierPrefix).\(UUID().uuidString)"
        self.onStop = onStop

        let registered = BGTaskScheduler.shared.register(
            forTaskWithIdentifier: identifier, using: nil
        ) { [weak self] task in
            guard let task = task as? BGContinuedProcessingTask else { return }
            // The launch handler runs on a scheduler-owned background queue.
            Task { @MainActor [weak self] in self?.attach(task) }
        }
        guard registered else {
            // Only reachable if BGTaskSchedulerPermittedIdentifiers misses it.
            logger.error("BGTaskScheduler refused to register \(identifier, privacy: .public).")
            self.onStop = nil
            return
        }

        let request = BGContinuedProcessingTaskRequest(
            identifier: identifier,
            title: Self.title(for: snapshot),
            subtitle: Self.subtitle(for: snapshot)
        )
        // Queue rather than fail: the receive is already committed on the wire.
        request.strategy = .queue

        do {
            try BGTaskScheduler.shared.submit(request)
            sessionIdentifier = identifier
            logger.info("Submitted background task \(identifier, privacy: .public).")
        } catch {
            logger.error("Background task submission failed: \(error.localizedDescription)")
            self.onStop = nil
        }
    }

    func update(_ snapshot: Snapshot) {
        guard let task, sessionIdentifier != nil else { return }
        // Progress every 8 MiB per file outruns what the scheduler wants, and
        // pushing all of them gets the task expired for looking stalled.
        let now = Date.timeIntervalSinceReferenceDate
        guard now - lastPush >= Self.minimumPushInterval else { return }
        lastPush = now
        apply(snapshot, to: task)
    }

    func finish(_ snapshot: Snapshot?, success: Bool) {
        guard let identifier = sessionIdentifier else { return }
        sessionIdentifier = nil
        onStop = nil
        lastPush = 0
        if let task {
            if let snapshot { apply(snapshot, to: task) }
            task.setTaskCompleted(success: success)
        } else {
            // Submitted but never started: withdraw, or it launches into a dead session.
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: identifier)
        }
        task = nil
        logger.info("Ended background task \(identifier, privacy: .public) success=\(success).")
    }

    // MARK: - Internals

    private func attach(_ task: BGContinuedProcessingTask) {
        // The session ended between submission and launch.
        guard sessionIdentifier == task.identifier else {
            task.setTaskCompleted(success: true)
            return
        }
        self.task = task
        task.expirationHandler = { [weak self] in
            Task { @MainActor [weak self] in self?.handleExpiration() }
        }
    }

    private func handleExpiration() {
        logger.notice("Background task expired or was stopped by the user.")
        let stop = onStop
        // Clear first: `finish` must not complete a task the system took back.
        task = nil
        sessionIdentifier = nil
        onStop = nil
        stop?()
    }

    private func apply(_ snapshot: Snapshot, to task: BGContinuedProcessingTask) {
        // NSProgress renders indeterminate if the total is set after the count.
        task.progress.totalUnitCount = max(snapshot.totalBytes, 1)
        task.progress.completedUnitCount = min(snapshot.completedBytes, max(snapshot.totalBytes, 1))
        task.updateTitle(Self.title(for: snapshot), subtitle: Self.subtitle(for: snapshot))
    }

    private static func title(for snapshot: Snapshot) -> String {
        snapshot.transferCount <= 1
            ? L.f("transfer_receiving_from", snapshot.peerName)
            : L.f("notification_title_multiple", snapshot.transferCount)
    }

    private static func subtitle(for snapshot: Snapshot) -> String {
        snapshot.rate > 0
            ? L.f("notification_progress_percent_rate", snapshot.percent,
                  TransferFormat.rate(snapshot.rate))
            : L.f("notification_progress_percent", snapshot.percent)
    }

    /// Must match the wildcard in `BGTaskSchedulerPermittedIdentifiers`.
    private static let identifierPrefix = "com.tsubuzaki.Wooosh.transfer"
    private static let minimumPushInterval: TimeInterval = 0.25
}

private extension Int {
    func clamped(to range: ClosedRange<Int>) -> Int {
        Swift.min(Swift.max(self, range.lowerBound), range.upperBound)
    }
}
#endif
