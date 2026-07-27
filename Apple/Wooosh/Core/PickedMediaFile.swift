#if os(iOS)
import CoreTransferable
import Foundation
import os
import UniformTypeIdentifiers

/// Carried as a *file*, not raw bytes, so the original filename survives:
/// `Data` transferables force an invented name, and `PHAsset.originalFilename`
/// would mean escalating Photos access from add-only to full read.
struct PickedMediaFile: Transferable {
    /// Our own copy: the URL handed to the import closure dies with it.
    let url: URL

    var originalName: String { url.lastPathComponent }

    static var transferRepresentation: some TransferRepresentation {
        // Images and movies match most reliably; `.item` is the catch-all beneath
        // them (RAW, Live Photos, later additions). First match wins.
        FileRepresentation(importedContentType: .image) { received in
            try PickedMediaFile.importing(received.file)
        }
        FileRepresentation(importedContentType: .movie) { received in
            try PickedMediaFile.importing(received.file)
        }
        FileRepresentation(importedContentType: .item) { received in
            try PickedMediaFile.importing(received.file)
        }
    }

    /// Keeps the name; duplicates within one pick take the receiver's " (2)"
    /// convention so two `IMG_4021.HEIC` cannot overwrite each other.
    static func importing(_ file: URL) throws -> PickedMediaFile {
        let directory = importsDirectory
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let name = file.lastPathComponent.isEmpty ? "Media" : file.lastPathComponent
        let destination = StorageRouter.uniqueDestination(for: name, in: directory)
        try FileManager.default.copyItem(at: file, to: destination)
        return PickedMediaFile(url: destination)
    }

    static var importsDirectory: URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("WoooshPickerImports", isDirectory: true)
    }

    static func clearImports() {
        try? FileManager.default.removeItem(at: importsDirectory)
    }
}
#endif
