import Foundation

/// Shared App Group locations (DESIGN.md §8). Compiled into BOTH the main
/// app and the share extension — keep it dependency-free.
enum AppGroup {
    static let identifier = "group.com.tsubuzaki.Wooosh"
    static let urlScheme = "wooosh"

    /// Group container root, or nil when the app-group entitlement is not
    /// effective (e.g. unsigned CI builds on the iOS simulator).
    static var containerURL: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: identifier)
    }

    /// Staging area the share extension writes batches into.
    static var batchesDirectory: URL? {
        containerURL?.appendingPathComponent("ShareBatches", isDirectory: true)
    }

    static func batchDirectory(id: String) -> URL? {
        // Batch ids are UUIDs minted by us; reject anything path-like.
        guard id.range(of: "^[A-Za-z0-9-]+$", options: .regularExpression) != nil else { return nil }
        return batchesDirectory?.appendingPathComponent(id, isDirectory: true)
    }

    /// Deep link the extension opens to hand off to the main app.
    static func sendURL(batchID: String) -> URL? {
        URL(string: "\(urlScheme)://send?batch=\(batchID)")
    }

    /// Files previously staged by the share extension for `batchID`.
    static func stagedFiles(batchID: String) -> [URL] {
        guard let dir = batchDirectory(id: batchID),
              let contents = try? FileManager.default.contentsOfDirectory(
                at: dir, includingPropertiesForKeys: [.fileSizeKey],
                options: [.skipsHiddenFiles]) else { return [] }
        return contents.sorted { $0.lastPathComponent < $1.lastPathComponent }
    }

    static func removeBatch(id: String) {
        guard let dir = batchDirectory(id: id) else { return }
        try? FileManager.default.removeItem(at: dir)
    }
}
