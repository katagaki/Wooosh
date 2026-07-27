import Foundation
import Network
import os

/// Advertise + browse for `_wooosh._tcp` (PROTOCOL.md §3.1). The core owns the
/// socket, so this is a pure mDNS registration and TXT `p` carries its UDP port.
///
/// Scan cadence is mDNSResponder's: neither API exposes the ≤ 2 s interval
/// §3.2/§3.3 specify. Do not fake it by restarting the browser on a timer, which
/// re-emits the whole result set and flickers rows DESIGN.md §5 holds still.
@MainActor
final class Discovery {
    static let serviceType = "_wooosh._tcp"

    /// Regenerated each launch, and deliberately not derived from the identity key.
    let rid: String

    private let registry: PeerRegistry
    private let queue = DispatchQueue(label: "com.tsubuzaki.Wooosh.discovery")
    private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "discovery")

    private var service: NetService?
    private var serviceDelegate: PublishDelegate?
    private var browser: NWBrowser?
    private var visibleRIDs: Set<String> = []
    private var sightings: [String: (result: NWBrowser.Result, port: UInt16?)] = [:]

    init(registry: PeerRegistry) {
        self.registry = registry
        self.rid = Data((0..<8).map { _ in UInt8.random(in: .min ... .max) }).lowercaseHex
    }

    // MARK: - Advertising

    /// `quicPort` is published as TXT `p` and mirrored into SRV so resolvers agree.
    func startAdvertising(
        displayName: String,
        deviceKind: DeviceKind,
        visibility: Visibility,
        quicPort: UInt16
    ) {
        stopAdvertising()
        guard let vis = visibility.txtValue, quicPort != 0 else { return }

        let instanceName = "\(displayName) (\(String(rid.suffix(4))))"
        let service = NetService(
            domain: "local.",
            type: Discovery.serviceType + ".",
            name: instanceName,
            port: Int32(quicPort)
        )
        var txt: [String: Data] = [
            "v": Data("1".utf8),
            "rid": Data(rid.utf8),
            "dn": Data(displayName.utf8),
            "dt": Data(deviceKind.rawValue.utf8),
            "p": Data(String(quicPort).utf8),
            "vis": Data(vis.utf8),
        ]
        // Keep under the 255-byte TXT limit; the name is a hint, the rid is not.
        if let data = NetService.data(fromTXTRecord: txt) as Data?, data.count > 255 {
            txt["dn"] = Data(displayName.prefix(40).utf8)
        }
        service.setTXTRecord(NetService.data(fromTXTRecord: txt))

        let delegate = PublishDelegate(logger: logger)
        service.delegate = delegate
        service.schedule(in: .main, forMode: .default)
        service.publish()

        self.service = service
        self.serviceDelegate = delegate
        logger.info("advertising \(instanceName) on QUIC port \(quicPort)")
    }

    func stopAdvertising() {
        service?.stop()
        service = nil
        serviceDelegate = nil
    }

    private final class PublishDelegate: NSObject, NetServiceDelegate {
        private let logger: Logger
        init(logger: Logger) { self.logger = logger }

        func netService(_ sender: NetService, didNotPublish errorDict: [String: NSNumber]) {
            logger.error("Bonjour publish failed: \(errorDict)")
        }
    }

    // MARK: - Browsing

    func startBrowsing() {
        stopBrowsing()
        let browser = NWBrowser(
            for: .bonjourWithTXTRecord(type: Discovery.serviceType, domain: nil),
            using: NWParameters()
        )
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            let snapshot: [Sighting] = results.compactMap { (result: NWBrowser.Result) -> Sighting? in
                guard case .bonjour(let txt) = result.metadata,
                      let rid = txt["rid"], !rid.isEmpty,
                      let dn = txt["dn"]
                else { return nil }
                // Unknown values stay nil: a neutral glyph beats a wrong one.
                let dt = DeviceKind(wire: txt["dt"])
                return Sighting(rid: rid, displayName: dn, deviceKind: dt,
                                port: txt["p"].flatMap(UInt16.init), result: result)
            }
            Task { @MainActor [weak self] in
                self?.process(snapshot: snapshot)
            }
        }
        browser.stateUpdateHandler = { [weak self] state in
            if case .failed(let error) = state {
                Task { @MainActor [weak self] in
                    self?.logger.error("Browser failed: \(error)")
                }
            }
        }
        self.browser = browser
        browser.start(queue: queue)
    }

    func stopBrowsing() {
        browser?.cancel()
        browser = nil
        visibleRIDs = []
        sightings = [:]
    }

    // MARK: - Resolution (DESIGN.md §4 `connect_peer`)

    func address(forRID rid: String) async -> String? {
        guard let sighting = sightings[rid] else { return nil }
        return await BonjourResolver.resolve(result: sighting.result, txtPort: sighting.port)
    }

    struct Sighting {
        let rid: String
        let displayName: String
        let deviceKind: DeviceKind?
        let port: UInt16?
        let result: NWBrowser.Result
    }

    /// NWBrowser emits add/remove, not announces, so absence starts the grace.
    private func process(snapshot: [Sighting]) {
        var current = Set<String>()
        for sighting in snapshot where sighting.rid != rid {
            current.insert(sighting.rid)
            sightings[sighting.rid] = (sighting.result, sighting.port)
            registry.sighted(rid: sighting.rid, displayName: sighting.displayName,
                             deviceKind: sighting.deviceKind)
        }
        for lost in visibleRIDs.subtracting(current) {
            registry.lost(rid: lost)
        }
        visibleRIDs = current
    }
}
