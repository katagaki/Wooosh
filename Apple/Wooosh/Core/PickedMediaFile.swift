#if os(iOS)
import CoreTransferable
import Foundation
import os
import UniformTypeIdentifiers

/// A photo or video imported from `PhotosPicker`, carried as a *file* rather
/// than as raw bytes so the asset's original filename survives.
///
/// `loadTransferable(type: Data.self)` hands over bytes only: the caller has to
/// invent a name, which is how picked media used to arrive at the receiver as
/// "Media 1.jpg" instead of "IMG_4021.HEIC". `FileRepresentation` instead
/// yields a `ReceivedTransferredFile` whose `file.lastPathComponent` is the
/// name Photos itself reports, with the extension that actually matches the
/// bytes we were handed — nothing is transcoded or renamed. This is the same
/// mechanism the share extension already relies on
/// (`NSItemProvider.loadFileRepresentation`).
///
/// It needs no extra entitlement. The `PHAsset.originalFilename` route would
/// also work, but it requires escalating Photos access from add-only to full
/// read — too high a price for a filename.
struct PickedMediaFile: Transferable {
    /// Our own copy of the item. The URL handed to the import closure is only
    /// valid for the duration of that closure, so the bytes are copied out
    /// immediately, under the original name.
    let url: URL

    var originalName: String { url.lastPathComponent }

    static var transferRepresentation: some TransferRepresentation {
        // Images and movies are listed explicitly because those are the two
        // concrete families the picker vends and the ones the runtime matches
        // most reliably; `.item` is the catch-all underneath them (RAW, Live
        // Photo pairs, anything added later). First match wins, so the order
        // matters.
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

    /// Copies a received file into our own import scratch directory, keeping
    /// its name. Duplicate names inside one pick are disambiguated with the
    /// same " (2)" convention the receiver uses, so two `IMG_4021.HEIC` never
    /// overwrite each other.
    static func importing(_ file: URL) throws -> PickedMediaFile {
        let directory = importsDirectory
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let name = file.lastPathComponent.isEmpty ? "Media" : file.lastPathComponent
        let destination = StorageRouter.uniqueDestination(for: name, in: directory)
        try FileManager.default.copyItem(at: file, to: destination)
        return PickedMediaFile(url: destination)
    }

    /// Scratch space for picker imports, emptied once a pick has been staged
    /// for sending.
    static var importsDirectory: URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("WoooshPickerImports", isDirectory: true)
    }

    static func clearImports() {
        try? FileManager.default.removeItem(at: importsDirectory)
    }
}
#endif
