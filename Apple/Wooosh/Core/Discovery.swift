import Foundation
import Network
import os

/// Bonjour advertise + browse for `_wooosh._tcp` (PROTOCOL.md §3.1).
///
/// Milestone 3: the core owns the socket. Advertising is therefore a pure
/// mDNS registration (`NetService.publish`, no listener of our own) whose TXT
/// `p` field carries the QUIC UDP port the core actually bound
/// (`core.listenAddr()`). The Milestone-1 placeholder `NWListener` is gone —
/// it would have advertised a port nothing speaks the protocol on.
///
/// **Scan cadence is not ours to set.** PROTOCOL.md §3.2/§3.3 specify a ≤ 2 s
/// foreground announce/scan interval, but nothing in this file polls: `NWBrowser`
/// is purely event-driven (`browseResultsChangedHandler` fires when mDNSResponder's
/// cache changes) and `NetService.publish()` hands announce timing to
/// mDNSResponder as well. Neither API exposes a query or announce interval, so
/// there is no constant here to retune — the 2 s figure is met on Apple by the
/// system responder, and by the UDP fallback on platforms that implement it
/// (this shell does not). Restarting the browser on a timer to fake the cadence
/// would be actively harmful: each restart re-emits the whole result set as
/// adds/removes, which churns `PeerRegistry` and flickers rows that DESIGN.md §5
/// requires to hold still. The one lever the shell does own is the staleness
/// grace, and it stays at 10 s deliberately (`PeerRegistry.staleGrace`) —
/// faster scanning is for finding devices sooner, not dropping them sooner.
@MainActor
final class Discovery {
    static let serviceType = "_wooosh._tcp"

    /// Rotating discovery ID: 8 random bytes, lowercase hex, regenerated each
    /// app launch. Deliberately not derived from the identity key.
    let rid: String

    private let registry: PeerRegistry
    private let queue = DispatchQueue(label: "com.tsubuzaki.Wooosh.discovery")
    private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "discovery")

    private var service: NetService?
    private var serviceDelegate: PublishDelegate?
    private var browser: NWBrowser?
    /// rids present in the last browse-results snapshot (excluding our own).
    private var visibleRIDs: Set<String> = []
    /// Latest browse result + advertised QUIC port per rid, for resolution.
    private var sightings: [String: (result: NWBrowser.Result, port: UInt16?)] = [:]

    init(registry: PeerRegistry) {
        self.registry = registry
        self.rid = Data((0..<8).map { _ in UInt8.random(in: .min ... .max) }).lowercaseHex
    }

    // MARK: - Advertising

    /// - Parameter quicPort: the core's bound UDP port, published as TXT `p`
    ///   (and mirrored into the SRV record so resolvers agree).
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
        // Guard against an over-long display name pushing the record past the
        // 255-byte TXT limit; the name is a UI hint, the rid is not.
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
                // Unknown/absent/retired values stay nil — a neutral glyph is
                // always acceptable, a confidently wrong one is not.
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

    /// Resolves a discovered peer to the `ip:port` the core can dial.
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

    /// Diffs each browse snapshot against the last: presence is a sighting,
    /// absence starts the staleness grace period (NWBrowser emits add/remove
    /// events, not periodic announces — see PeerRegistry.lost).
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
