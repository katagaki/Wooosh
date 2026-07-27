import os
import SwiftUI
import UniformTypeIdentifiers
#if os(iOS)
import PhotosUI
#endif

/// A pre-armed share-extension batch is sent immediately instead of prompting for a pick.
struct SendTransferSheet: View {
    let peer: Peer

    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    @State private var transfer: Transfer?
    @State private var showingFileImporter = false
    @State private var sendFailed = false
    /// mDNS resolve + QUIC handshake, before the first byte moves; can fail on its own.
    @State private var connecting = false
    #if os(iOS)
    @State private var photoItems: [PhotosPickerItem] = []
    @State private var loadingPhotos = false
    #endif

    var body: some View {
        NavigationStack {
            Group {
                if let transfer {
                    TransferProgressView(transfer: transfer) { dismiss() }
                } else if connecting {
                    connectingState
                } else {
                    picker
                }
            }
            .navigationTitle(L.t(transfer == nil ? "transfer_nav_send_files" : "transfer_nav_sending"))
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                if transfer == nil {
                    ToolbarItem(placement: .cancellationAction) {
                        CancelButton { dismiss() }
                    }
                }
            }
        }
        .interactiveDismissDisabled(transfer?.isActive == true)
        .task {
            guard model.pendingShareBatch != nil else { return }
            await deliver { await model.sendPendingBatch(to: peer) }
        }
        .fileImporter(
            isPresented: $showingFileImporter,
            allowedContentTypes: [.item],
            allowsMultipleSelection: true
        ) { result in
            guard case .success(let urls) = result, !urls.isEmpty else { return }
            Task { await deliver { await model.sendFiles(to: peer, urls: urls) } }
        }
        .alert(L.t("error_send_failed_title"), isPresented: $sendFailed) {
            Button(L.t("action_ok"), role: .cancel) {}
        } message: {
            Text(model.lastSendError ?? L.t("error_send_failed_body"))
        }
        #if os(macOS)
        .frame(minWidth: 460, minHeight: 480)
        #endif
    }

    private func deliver(_ body: () async -> Transfer?) async {
        connecting = true
        let result = await body()
        connecting = false
        transfer = result
        sendFailed = result == nil
    }

    /// The unpaired peer's consent sheet asks its user to compare against exactly this phrase.
    private var showsFingerprint: Bool {
        !model.isPaired(peer) && !model.fingerprintPhrase.isEmpty
    }

    private var connectingState: some View {
        VStack(spacing: 16) {
            ProgressView()
                .controlSize(.large)
            Text(L.f("transfer_connecting_to", peer.displayName))
                .foregroundStyle(.secondary)
            if showsFingerprint {
                FingerprintCallout(phrase: model.fingerprintPhrase)
                    .padding(.top, 8)
            }
        }
        .frame(maxWidth: 420)
        .padding(24)
    }

    #if os(iOS)
    /// Last resort when a picked item exposes no file representation: keep the bytes and
    /// ask the item for its content type rather than assuming .jpg.
    private func fallbackImport(_ item: PhotosPickerItem, index: Int) async -> URL? {
        Logger(subsystem: "com.tsubuzaki.Wooosh", category: "photos")
            .error("Picked item \(index) had no file representation; original filename unavailable")
        guard let data = try? await item.loadTransferable(type: Data.self) else { return nil }
        let type = item.supportedContentTypes.first
        let ext = type?.preferredFilenameExtension
            ?? (type?.conforms(to: .movie) == true ? "mov" : "jpg")
        let directory = PickedMediaFile.importsDirectory
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = StorageRouter.uniqueDestination(for: "Media \(index + 1).\(ext)", in: directory)
        do {
            try data.write(to: url, options: .atomic)
            return url
        } catch {
            return nil
        }
    }
    #endif

    private var picker: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(spacing: 16) {
                    Image(systemName: peer.symbolName)
                        .font(.system(size: 44))
                        .foregroundStyle(.tint)
                    Text(peer.displayName)
                        .font(.title3.weight(.semibold))
                        .multilineTextAlignment(.center)
                    if model.isPaired(peer) {
                        Label(L.t("offer_paired_device"), systemImage: "checkmark.seal.fill")
                            .font(.subheadline)
                            .foregroundStyle(.green)
                    } else if showsFingerprint {
                        // On screen from the moment an unpaired device is targeted, so it is
                        // already there when the other side is asked to verify it.
                        FingerprintCallout(phrase: model.fingerprintPhrase)
                            .padding(.top, 4)
                    }
                    #if os(iOS)
                    if loadingPhotos {
                        ProgressView(L.t("transfer_preparing"))
                            .padding(.top, 8)
                    }
                    #endif
                }
                .frame(maxWidth: 420)
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 24)
                .padding(.top, 24)
            }
            .scrollBounceBehavior(.basedOnSize)

            SheetActions {
                Button(L.t("action_choose_files"), systemImage: "folder") {
                    showingFileImporter = true
                }
            } secondary: {
                #if os(iOS)
                PhotosPicker(selection: $photoItems, matching: .any(of: [.images, .videos])) {
                    Label(L.t("action_choose_photos"), systemImage: "photo.on.rectangle")
                }
                #else
                EmptyView()
                #endif
            }
        }
        #if os(iOS)
        .onChange(of: photoItems) { _, items in
            guard !items.isEmpty else { return }
            loadingPhotos = true
            Task {
                // Imported as a file, never as `Data`, so the asset's own name survives.
                var imported: [URL] = []
                for (index, item) in items.enumerated() {
                    if let media = try? await item.loadTransferable(type: PickedMediaFile.self) {
                        imported.append(media.url)
                    } else if let url = await fallbackImport(item, index: index) {
                        // Should not happen; beats silently dropping the item.
                        imported.append(url)
                    }
                }
                loadingPhotos = false
                photoItems = []
                if !imported.isEmpty {
                    await deliver { await model.sendPickedMedia(to: peer, urls: imported) }
                }
            }
        }
        #endif
    }
}
