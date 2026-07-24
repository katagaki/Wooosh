#if DEBUG
import CryptoKit
import Foundation
import os

/// DEBUG-only, environment-driven harness for the Milestone-3 live interop
/// test against `wooosh-cli`.
///
/// It exists because the end-to-end check has to run the *real* app binary
/// (same `RealCore`, same event stream, same storage routing) on a machine
/// where clicking through the UI is not available. It drives exactly the same
/// entry points the views call — nothing here bypasses the core.
///
/// Inert unless `WOOOSH_AUTOTEST=1`.
///
///   WOOOSH_AUTOTEST=1                run the harness
///   WOOOSH_AUTOTEST_STATE=<path>     write deviceId/fingerprint/listenAddr JSON
///   WOOOSH_AUTOTEST_ACCEPT=1         auto-accept incoming offers
///   WOOOSH_AUTOTEST_PAIR=<payload>   pair with a pasted QR payload on launch
///   WOOOSH_AUTOTEST_SEND=<paths>     comma-separated files to send once paired
///   WOOOSH_AUTOTEST_SEND_TO=<name>   discover this peer over mDNS, then send
///   WOOOSH_AUTOTEST_RECONNECT=<addr> after the flow, reconnect to <addr>
///                                    pinning the key from `trustedPeers()`
@MainActor
enum AutoTest {
    private static let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "autotest")

    static func run(model: AppModel) {
        let env = ProcessInfo.processInfo.environment
        // UI demo mode: open one scripted screen and stop. Used to review the
        // interface on a simulator where Simulator.app (and therefore any way
        // to tap) is not installed.
        if let screen = env["WOOOSH_DEMO"].flatMap(AppModel.DebugScreen.init(rawValue:)) {
            Task { @MainActor in
                model.switchBackend(to: .mock)
                try? await Task.sleep(for: .milliseconds(600))
                model.debugPresent(screen: screen)
            }
            return
        }
        guard env["WOOOSH_AUTOTEST"] == "1" else { return }
        logger.notice("autotest: device \(model.deviceIDString, privacy: .public) listening on \(model.listenAddress, privacy: .public)")

        if let path = env["WOOOSH_AUTOTEST_STATE"] {
            let state: [String: String] = [
                "device_id": model.deviceIDString,
                "fingerprint": model.fingerprintPhrase,
                "listen_addr": model.listenAddress,
                "startup_error": model.startupError ?? "",
            ]
            if let data = try? JSONSerialization.data(withJSONObject: state, options: .prettyPrinted) {
                try? data.write(to: URL(fileURLWithPath: path))
            }
        }

        if env["WOOOSH_AUTOTEST_ACCEPT"] == "1" {
            Task { await autoAcceptOffers(model: model) }
        }
        let files = (env["WOOOSH_AUTOTEST_SEND"] ?? "")
            .split(separator: ",").map(String.init).filter { !$0.isEmpty }
        // Photos-picker path: the same files, but routed through
        // `PickedMediaFile` + `sendPickedMedia` instead of the document
        // importer, so the filename-preservation fix is exercised end to end.
        let pickedFiles = (env["WOOOSH_AUTOTEST_SEND_PICKED"] ?? "")
            .split(separator: ",").map(String.init).filter { !$0.isEmpty }
        let reconnect = env["WOOOSH_AUTOTEST_RECONNECT"].flatMap { $0.isEmpty ? nil : $0 }
        if let payload = env["WOOOSH_AUTOTEST_PAIR"], !payload.isEmpty {
            Task {
                await pairThenSend(model: model, payload: payload,
                                   files: files, pickedFiles: pickedFiles)
                reportTrustedPeers(model: model)
                if let reconnect { await reconnectPinned(model: model, addr: reconnect) }
            }
        } else if let name = env["WOOOSH_AUTOTEST_SEND_TO"], !name.isEmpty {
            Task {
                await sendToDiscovered(model: model, displayName: name,
                                       files: files, pickedFiles: pickedFiles)
                reportTrustedPeers(model: model)
                if let reconnect { await reconnectPinned(model: model, addr: reconnect) }
            }
        } else if reconnect != nil || env["WOOOSH_AUTOTEST_TRUST"] == "1" {
            Task {
                // Give the core a moment to finish start-up bookkeeping.
                try? await Task.sleep(for: .seconds(1))
                reportTrustedPeers(model: model)
                if let reconnect { await reconnectPinned(model: model, addr: reconnect) }
            }
        }
    }

    // MARK: - New-API verification

    /// Dumps the core's canonical trust store through the app's seam. This is
    /// the same `trustedPeers()` the Settings list and every pin lookup use.
    private static func reportTrustedPeers(model: AppModel) {
        model.trustStore.refresh()
        let peers = model.trustStore.devices
        logger.notice("autotest: trustedPeers() -> \(peers.count) peer(s)")
        for peer in peers {
            let hex = peer.publicKey.map { String(format: "%02x", $0) }.joined()
            let derived = model.core.deviceID(forPublicKey: peer.publicKey) ?? "-"
            let phrase = model.core.fingerprintPhrase(forPublicKey: peer.publicKey) ?? "-"
            logger.notice("""
                autotest: trusted name=\(peer.displayName, privacy: .public) \
                device_id=\(peer.deviceID, privacy: .public) \
                pubkey_len=\(peer.publicKey.count) pubkey=\(hex, privacy: .public) \
                device_id_for_pubkey=\(derived, privacy: .public) \
                id_matches=\(derived == peer.deviceID) \
                type=\(peer.deviceType?.rawValue ?? "unknown", privacy: .public) \
                fingerprint=\(peer.fingerprint, privacy: .public) \
                fingerprint_for_pubkey=\(phrase, privacy: .public) \
                fingerprint_matches=\(phrase == peer.fingerprint) \
                paired_at=\(peer.pairedAt.timeIntervalSince1970) \
                last_seen=\(peer.lastSeen.timeIntervalSince1970)
                """)
        }
    }

    /// Reconnects to `addr` explicitly pinning the pubkey from `trustedPeers()`
    /// — the path the shell now takes for every already-paired peer.
    private static func reconnectPinned(model: AppModel, addr: String) async {
        guard let pinned = model.trustStore.devices.first else {
            logger.error("autotest: reconnect requested but nothing is pinned")
            return
        }
        logger.notice("""
            autotest: reconnecting to \(addr, privacy: .public) with \
            expectedPubkey for \(pinned.deviceID, privacy: .public)
            """)
        do {
            let peerID = try await model.core.connectPeer(
                addr: addr, expectedPublicKey: pinned.publicKey)
            logger.notice("""
                autotest: pinned reconnect OK peer_id=\(peerID, privacy: .public) \
                matches_trusted=\(peerID == pinned.deviceID)
                """)
        } catch {
            logger.error("autotest: pinned reconnect FAILED: \(coreErrorMessage(error), privacy: .public)")
        }
    }

    /// Exercises the discovery path end to end: wait for the peer to show up
    /// over mDNS, then send — which resolves the Bonjour instance to a
    /// concrete `host:port` and calls `connect_peer` before the first byte.
    private static func sendToDiscovered(model: AppModel, displayName: String,
                                         files: [String],
                                         pickedFiles: [String] = []) async {
        var target: Peer?
        for _ in 0..<200 {
            if let peer = model.registry.peers.first(where: {
                $0.displayName.localizedCaseInsensitiveContains(displayName) && !$0.isStale
            }) {
                target = peer
                break
            }
            try? await Task.sleep(for: .milliseconds(100))
        }
        guard let peer = target else {
            logger.error("autotest: never discovered a peer named \(displayName, privacy: .public)")
            return
        }
        logger.notice("autotest: discovered \(peer.displayName, privacy: .public) rid=\(peer.rid, privacy: .public)")

        #if os(iOS)
        if !pickedFiles.isEmpty {
            await sendPicked(model: model, peer: peer, paths: pickedFiles)
        }
        #endif

        guard !files.isEmpty else { return }
        guard let transfer = await model.sendFiles(to: peer, urls: files.map { URL(fileURLWithPath: $0) }) else {
            logger.error("autotest: send failed: \(model.lastSendError ?? "unknown", privacy: .public)")
            return
        }
        await awaitCompletion(of: transfer)

        // Second pass over the *same discovered row*, after the core has torn
        // the connection down. By now the row carries the peer's DeviceID, so
        // `ensureConnection` looks the pin up in `trustedPeers()` and passes
        // `expectedPubkey` — the reconnect case the address fallback misses.
        let second = (ProcessInfo.processInfo.environment["WOOOSH_AUTOTEST_SEND2"] ?? "")
            .split(separator: ",").map(String.init).filter { !$0.isEmpty }
        guard !second.isEmpty else { return }
        // The core reports PeerDisconnected on its own QUIC idle timeout, so
        // this waits well past it rather than racing it.
        for _ in 0..<900 {
            if let row = model.registry.peers.first(where: { $0.rid == peer.rid }),
               !row.isConnected { break }
            try? await Task.sleep(for: .milliseconds(100))
        }
        guard let row = model.registry.peers.first(where: { $0.rid == peer.rid }) else { return }
        logger.notice("""
            autotest: reconnect-send to rid=\(row.rid, privacy: .public) \
            connected=\(row.isConnected) \
            knownDeviceID=\(row.knownDeviceID ?? "none", privacy: .public) \
            paired=\(model.isPaired(row))
            """)
        guard let retry = await model.sendFiles(to: row, urls: second.map { URL(fileURLWithPath: $0) }) else {
            logger.error("autotest: reconnect-send failed: \(model.lastSendError ?? "unknown", privacy: .public)")
            return
        }
        await awaitCompletion(of: retry)
    }

    /// Polls for the consent sheet's offer and accepts it, exactly as
    /// `IncomingOfferSheet`'s Accept button does, then reports where each
    /// file landed and its SHA-256 so the caller can compare against the
    /// bytes it sent.
    private static func autoAcceptOffers(model: AppModel) async {
        var reported = Set<String>()
        while !Task.isCancelled {
            if let offer = model.transfers.pendingOffer {
                logger.notice("autotest: accepting offer \(offer.id.raw, privacy: .public) with \(offer.files.count) file(s)")
                model.transfers.accept(offer: offer)
            }
            for transfer in model.transfers.transfers where transfer.direction == .incoming {
                for file in transfer.files {
                    guard case .saved(let label) = file.status,
                          !reported.contains(transfer.id.raw + file.name)
                    else { continue }
                    reported.insert(transfer.id.raw + file.name)
                    let path = file.savedURL?.path ?? "(no path)"
                    let digest = file.savedURL.flatMap(sha256(of:)) ?? "-"
                    logger.notice("""
                        autotest: received \(file.name, privacy: .public) -> \
                        \(label, privacy: .public) at \(path, privacy: .public) \
                        sha256=\(digest, privacy: .public)
                        """)
                }
            }
            try? await Task.sleep(for: .milliseconds(150))
        }
    }

    private static func sha256(of url: URL) -> String? {
        guard let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { return nil }
        return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    private static func pairThenSend(model: AppModel, payload: String,
                                     files: [String], pickedFiles: [String] = []) async {
        model.pairWithQR(payload: payload)
        // Wait for the pairing result the UI waits for.
        for _ in 0..<200 {
            if case .success(let peer) = model.pairingPhase {
                logger.notice("autotest: paired with \(peer.displayName, privacy: .public) (\(peer.id, privacy: .public))")
                break
            }
            if case .failed(let message) = model.pairingPhase {
                logger.error("autotest: pairing failed: \(message, privacy: .public)")
                return
            }
            try? await Task.sleep(for: .milliseconds(100))
        }
        guard let peer = model.registry.peers.first(where: { $0.isConnected }) else {
            logger.error("autotest: no connected peer to send to")
            return
        }
        #if os(iOS)
        if !pickedFiles.isEmpty {
            await sendPicked(model: model, peer: peer, paths: pickedFiles)
        }
        #endif
        guard !files.isEmpty else { return }
        let urls = files.map { URL(fileURLWithPath: $0) }
        guard let transfer = await model.sendFiles(to: peer, urls: urls) else {
            logger.error("autotest: send failed: \(model.lastSendError ?? "unknown", privacy: .public)")
            return
        }
        await awaitCompletion(of: transfer)
    }

    #if os(iOS)
    /// Drives the Photos-picker send path. `PickedMediaFile.importing` is the
    /// exact closure `loadTransferable(type: PickedMediaFile.self)` runs on the
    /// file the picker vends, so this covers everything the fix touches: the
    /// received file's name, staging, the manifest, and the wire.
    private static func sendPicked(model: AppModel, peer: Peer, paths: [String]) async {
        var imported: [URL] = []
        for path in paths {
            guard let media = try? PickedMediaFile.importing(URL(fileURLWithPath: path)) else {
                logger.error("autotest: could not import \(path, privacy: .public)")
                continue
            }
            logger.notice("autotest: picked import name=\(media.originalName, privacy: .public)")
            imported.append(media.url)
        }
        guard let transfer = await model.sendPickedMedia(to: peer, urls: imported) else {
            logger.error("autotest: picked send failed: \(model.lastSendError ?? "unknown", privacy: .public)")
            return
        }
        for file in transfer.files {
            logger.notice("""
                autotest: picked manifest name=\(file.name, privacy: .public) \
                mime=\(file.mime, privacy: .public) size=\(file.size)
                """)
        }
        await awaitCompletion(of: transfer)
    }
    #endif

    private static func awaitCompletion(of transfer: Transfer) async {
        logger.notice("autotest: transfer \(transfer.id.raw, privacy: .public) started")
        for _ in 0..<600 {
            switch transfer.state {
            case .done(let summary):
                logger.notice("autotest: transfer done, \(summary.fileCount) file(s), \(summary.totalBytes) bytes")
                return
            case .failed(let message, _):
                logger.error("autotest: transfer failed: \(message, privacy: .public)")
                return
            default:
                try? await Task.sleep(for: .milliseconds(100))
            }
        }
        logger.error("autotest: transfer timed out")
    }
}
#endif
