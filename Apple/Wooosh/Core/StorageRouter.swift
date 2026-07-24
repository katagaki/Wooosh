import Foundation
#if os(iOS)
import Photos
#endif

/// Routes a verified staged file to its platform-correct destination on
/// `fileReady` (DESIGN.md §6):
///
/// - iOS: photos/videos → Photos library (add-only permission); documents →
///   app Documents/ (visible in Files via UIFileSharingEnabled).
/// - macOS: everything → ~/Downloads (sandbox Downloads entitlement).
///
/// Collision policy: append " (2)", " (3)"… — never overwrite. The staged
/// file is removed once routing succeeds; a transfer is never reported
/// complete for a file the user can't find.
enum StorageRouter {
    enum RoutingError: LocalizedError {
        case photosPermissionDenied
        case destinationUnavailable

        var errorDescription: String? {
            switch self {
            case .photosPermissionDenied:
                "Wooosh isn't allowed to add to your Photos library."
            case .destinationUnavailable:
                L.t("error_storage_unavailable")
            }
        }
    }

    /// Where a received file ended up: a short user-facing label ("Photos",
    /// "Downloads", "Files") and, when it is a file on disk, its final URL.
    /// Photos-library insertions have no path the app may keep.
    struct Placement {
        let label: String
        let url: URL?
    }

    /// Throws if the file could not be placed.
    static func route(stagedURL: URL, kind: FileKind) async throws -> Placement {
        #if os(iOS)
        switch kind {
        case .photo, .video:
            try await saveToPhotoLibrary(stagedURL: stagedURL, kind: kind)
            try? FileManager.default.removeItem(at: stagedURL)
            return Placement(label: L.t("storage_location_photos"), url: nil)
        case .document:
            guard let documents = FileManager.default.urls(
                for: .documentDirectory, in: .userDomainMask).first else {
                throw RoutingError.destinationUnavailable
            }
            let url = try moveAvoidingCollision(from: stagedURL, intoDirectory: documents)
            return Placement(label: L.t("storage_location_files"), url: url)
        }
        #else
        guard let downloads = FileManager.default.urls(
            for: .downloadsDirectory, in: .userDomainMask).first else {
            throw RoutingError.destinationUnavailable
        }
        let url = try moveAvoidingCollision(from: stagedURL, intoDirectory: downloads)
        return Placement(label: L.t("storage_location_downloads"), url: url)
        #endif
    }

    #if os(iOS)
    private static func saveToPhotoLibrary(stagedURL: URL, kind: FileKind) async throws {
        let status = await PHPhotoLibrary.requestAuthorization(for: .addOnly)
        guard status == .authorized || status == .limited else {
            throw RoutingError.photosPermissionDenied
        }
        try await PHPhotoLibrary.shared().performChanges {
            let request = PHAssetCreationRequest.forAsset()
            let options = PHAssetResourceCreationOptions()
            options.shouldMoveFile = false
            request.addResource(
                with: kind == .video ? .video : .photo,
                fileURL: stagedURL,
                options: options
            )
        }
    }
    #endif

    /// Moves `source` into `directory`, appending " (2)", " (3)"… on collision.
    @discardableResult
    static func moveAvoidingCollision(from source: URL, intoDirectory directory: URL) throws -> URL {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let destination = uniqueDestination(for: source.lastPathComponent, in: directory)
        try FileManager.default.moveItem(at: source, to: destination)
        return destination
    }

    static func uniqueDestination(for filename: String, in directory: URL) -> URL {
        let base = (filename as NSString).deletingPathExtension
        let ext = (filename as NSString).pathExtension
        var candidate = directory.appendingPathComponent(filename)
        var counter = 2
        while FileManager.default.fileExists(atPath: candidate.path) {
            let name = ext.isEmpty ? "\(base) (\(counter))" : "\(base) (\(counter)).\(ext)"
            candidate = directory.appendingPathComponent(name)
            counter += 1
        }
        return candidate
    }
}
