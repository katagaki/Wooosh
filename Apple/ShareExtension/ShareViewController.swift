import Foundation
import UniformTypeIdentifiers

#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// Share extension entry point (DESIGN.md §8).
///
/// The extension never transfers by itself (iOS extension memory is tight):
/// it copies the shared items into the App Group container, then opens the
/// main app via `wooosh://send?batch=<id>` so the device list appears
/// pre-armed to send the batch on tap.
#if os(iOS)
final class ShareViewController: UIViewController {
    private let exporter = ShareBatchExporter()
    private var started = false

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        let spinner = UIActivityIndicatorView(style: .large)
        spinner.startAnimating()
        spinner.translatesAutoresizingMaskIntoConstraints = false
        let label = UILabel()
        label.text = NSLocalizedString("share_ext_preparing", bundle: .main, comment: "")
        label.font = .preferredFont(forTextStyle: .callout)
        label.textColor = .secondaryLabel
        label.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(spinner)
        view.addSubview(label)
        NSLayoutConstraint.activate([
            spinner.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            spinner.centerYAnchor.constraint(equalTo: view.centerYAnchor, constant: -24),
            label.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            label.topAnchor.constraint(equalTo: spinner.bottomAnchor, constant: 16),
        ])
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !started else { return }
        started = true
        exporter.export(extensionContext: extensionContext) { [weak self] batchID in
            DispatchQueue.main.async {
                guard let self else { return }
                if let batchID, let url = AppGroup.sendURL(batchID: batchID) {
                    self.openMainApp(url: url)
                }
                self.extensionContext?.completeRequest(returningItems: nil)
            }
        }
    }

    /// UIApplication.open is NS_EXTENSION_UNAVAILABLE; the standard
    /// workaround walks the responder chain and performs `openURL:`.
    private func openMainApp(url: URL) {
        let selector = sel_registerName("openURL:")
        var responder: UIResponder? = self
        while let current = responder {
            if current.responds(to: selector), !(current is UIViewController) {
                _ = current.perform(selector, with: url)
                return
            }
            responder = current.next
        }
        // Fallback; works in some extension contexts.
        extensionContext?.open(url)
    }
}
#else
final class ShareViewController: NSViewController {
    private let exporter = ShareBatchExporter()
    private var started = false

    override func loadView() {
        let container = NSView(frame: NSRect(x: 0, y: 0, width: 320, height: 140))
        let spinner = NSProgressIndicator()
        spinner.style = .spinning
        spinner.startAnimation(nil)
        spinner.translatesAutoresizingMaskIntoConstraints = false
        let label = NSTextField(labelWithString: NSLocalizedString("share_ext_preparing", bundle: .main, comment: ""))
        label.textColor = .secondaryLabelColor
        label.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(spinner)
        container.addSubview(label)
        NSLayoutConstraint.activate([
            spinner.centerXAnchor.constraint(equalTo: container.centerXAnchor),
            spinner.centerYAnchor.constraint(equalTo: container.centerYAnchor, constant: -16),
            label.centerXAnchor.constraint(equalTo: container.centerXAnchor),
            label.topAnchor.constraint(equalTo: spinner.bottomAnchor, constant: 12),
        ])
        view = container
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        guard !started else { return }
        started = true
        exporter.export(extensionContext: extensionContext) { [weak self] batchID in
            DispatchQueue.main.async {
                guard let self else { return }
                if let batchID, let url = AppGroup.sendURL(batchID: batchID) {
                    NSWorkspace.shared.open(url)
                }
                self.extensionContext?.completeRequest(returningItems: nil)
            }
        }
    }
}
#endif

/// Copies every shared attachment into a new App Group batch directory.
final class ShareBatchExporter {
    /// Calls back with the batch id, or nil when nothing could be staged.
    func export(extensionContext: NSExtensionContext?,
                completion: @escaping (String?) -> Void) {
        let providers = (extensionContext?.inputItems ?? [])
            .compactMap { $0 as? NSExtensionItem }
            .flatMap { $0.attachments ?? [] }
        let batchID = UUID().uuidString
        guard !providers.isEmpty,
              let batchDir = AppGroup.batchDirectory(id: batchID) else {
            completion(nil)
            return
        }
        do {
            try FileManager.default.createDirectory(at: batchDir, withIntermediateDirectories: true)
        } catch {
            completion(nil)
            return
        }

        let group = DispatchGroup()
        let queue = DispatchQueue(label: "com.tsubuzaki.Wooosh.share.export")
        var stagedCount = 0

        for (index, provider) in providers.enumerated() {
            group.enter()
            stage(provider: provider, index: index, into: batchDir, queue: queue) { staged in
                queue.async {
                    if staged { stagedCount += 1 }
                    group.leave()
                }
            }
        }

        group.notify(queue: queue) {
            if stagedCount > 0 {
                completion(batchID)
            } else {
                try? FileManager.default.removeItem(at: batchDir)
                completion(nil)
            }
        }
    }

    private func stage(provider: NSItemProvider, index: Int, into batchDir: URL,
                       queue: DispatchQueue, completion: @escaping (Bool) -> Void) {
        let typeIdentifier = preferredTypeIdentifier(for: provider)
        guard let typeIdentifier else {
            completion(false)
            return
        }
        provider.loadFileRepresentation(forTypeIdentifier: typeIdentifier) { url, _ in
            // The provided URL is only valid inside this callback — copy now.
            guard let url else {
                completion(false)
                return
            }
            let name = url.lastPathComponent.isEmpty ? "Item \(index + 1)" : url.lastPathComponent
            let destination = Self.uniqueDestination(for: name, in: batchDir)
            do {
                try FileManager.default.copyItem(at: url, to: destination)
                completion(true)
            } catch {
                completion(false)
            }
        }
    }

    private func preferredTypeIdentifier(for provider: NSItemProvider) -> String? {
        // Prefer concrete file-backed types; fall back to anything data-like.
        for candidate in [UTType.movie, .image, .fileURL, .data] {
            if provider.hasItemConformingToTypeIdentifier(candidate.identifier) {
                // loadFileRepresentation needs the registered identifier, not
                // the abstract parent, to get the original filename.
                if let registered = provider.registeredTypeIdentifiers.first(where: {
                    UTType($0)?.conforms(to: candidate) == true
                }) {
                    return registered
                }
                return candidate.identifier
            }
        }
        return provider.registeredTypeIdentifiers.first
    }

    private static func uniqueDestination(for filename: String, in directory: URL) -> URL {
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
