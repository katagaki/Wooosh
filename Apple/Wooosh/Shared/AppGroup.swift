import Foundation

/// Compiled into both the app and the extension, so keep it dependency-free.
enum AppGroup {
    static let identifier = "group.com.tsubuzaki.Wooosh"
    static let urlScheme = "wooosh"

    /// nil when the app-group entitlement is not effective (unsigned builds).
    static var containerURL: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: identifier)
    }

    static var batchesDirectory: URL? {
        containerURL?.appendingPathComponent("ShareBatches", isDirectory: true)
    }

    static func batchDirectory(id: String) -> URL? {
        // Batch ids are UUIDs minted by us; reject anything path-like.
        guard id.range(of: "^[A-Za-z0-9-]+$", options: .regularExpression) != nil else { return nil }
        return batchesDirectory?.appendingPathComponent(id, isDirectory: true)
    }

    static func sendURL(batchID: String) -> URL? {
        URL(string: "\(urlScheme)://send?batch=\(batchID)")
    }

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
