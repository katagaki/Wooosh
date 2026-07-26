#if os(iOS)
import BackgroundTasks
import Foundation
import os
import UIKit

/// Background execution for incoming transfers (DESIGN.md §7).
///
/// `BGContinuedProcessingTask` (iOS 26) is the only mechanism Apple offers an
/// unprivileged app for user-initiated work that has to outlive the app going
/// to the background. It also renders the Lock Screen and Dynamic Island
/// progress UI itself, driven by `task.progress`, which is why Wooosh ships no
/// Live Activity of its own: two progress bars for one transfer is worse than
/// one, and the system's comes with a working Stop button.
///
/// One task covers the whole receive session rather than one per transfer or
/// per file. The scheduler caps concurrent tasks, and Apple's guidance is
/// explicit that many small tasks is the wrong shape for a transfer app.
@MainActor
final class BackgroundTransferTask {

    /// Aggregate state of everything currently being received.
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
    /// Set from submission until the task ends, so a second incoming transfer
    /// joins the running session instead of submitting a competing request.
    private var sessionIdentifier: String?
    private var onStop: (() -> Void)?
    private var lastPush: TimeInterval = 0

    // MARK: - Lifecycle

    /// Starts a background session if one is not already running.
    ///
    /// `onStop` fires when the system reclaims the assertion, which is also
    /// what the Stop button in the system UI does. The two are indistinguishable
    /// through this API, so the caller decides what to do about it.
    func begin(_ snapshot: Snapshot, onStop: @escaping () -> Void) {
        guard sessionIdentifier == nil else {
            update(snapshot)
            return
        }
        // A request may only be submitted on behalf of a foregrounded app. When
        // Wooosh is already in the background there is no assertion to be had
        // and nothing to fall back to, so this is a silent no-op rather than an
        // error: the transfer still runs for as long as the app survives.
        guard UIApplication.shared.applicationState != .background else {
            logger.notice("Not submitting a background task: app is not foregrounded.")
            return
        }

        // A fresh id per session. `register` must never see the same identifier
        // twice (the system kills the app for it), and the scheduler gives no
        // way to know when it is finished with one.
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
            // Only reachable if the identifier is not covered by the wildcard in
            // BGTaskSchedulerPermittedIdentifiers.
            logger.error("BGTaskScheduler refused to register \(identifier, privacy: .public).")
            self.onStop = nil
            return
        }

        let request = BGContinuedProcessingTaskRequest(
            identifier: identifier,
            title: Self.title(for: snapshot),
            subtitle: Self.subtitle(for: snapshot)
        )
        // Queue rather than fail: a receive is already committed on the wire, so
        // waiting for a slot beats abandoning the assertion outright.
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

    /// Reports progress. Safe to call before the system has started the task.
    func update(_ snapshot: Snapshot) {
        guard let task, sessionIdentifier != nil else { return }
        // The core emits Progress every 8 MiB per file, which on a fast LAN is
        // far above the few-per-second the scheduler wants. Pushing every one of
        // them costs enough lock traffic to get the task expired for looking
        // stalled, so coalesce. The final update goes through `finish`.
        let now = Date.timeIntervalSinceReferenceDate
        guard now - lastPush >= Self.minimumPushInterval else { return }
        lastPush = now
        apply(snapshot, to: task)
    }

    /// Ends the session and releases the assertion. Idempotent.
    func finish(_ snapshot: Snapshot?, success: Bool) {
        guard let identifier = sessionIdentifier else { return }
        sessionIdentifier = nil
        onStop = nil
        lastPush = 0
        if let task {
            if let snapshot { apply(snapshot, to: task) }
            task.setTaskCompleted(success: success)
        } else {
            // Submitted but never started: withdraw it so the scheduler does not
            // launch us into a session that is already over.
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
        // Clear first: `finish` must not try to complete a task the system has
        // already taken back.
        task = nil
        sessionIdentifier = nil
        onStop = nil
        stop?()
    }

    private func apply(_ snapshot: Snapshot, to task: BGContinuedProcessingTask) {
        // Order matters: NSProgress renders an indeterminate bar if the total is
        // set after the completed count.
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
