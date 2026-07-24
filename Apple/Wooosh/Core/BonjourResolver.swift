import Foundation
import Network
import os

/// Turns a browsed `_wooosh._tcp` instance into a concrete `host:port` for
/// `connect_peer` (DESIGN.md §4).
///
/// `NWBrowser` hands back an opaque `.service` endpoint, not an address, and
/// the core needs a literal `ip:port`. Two resolution paths are implemented:
///
/// 1. **`NetService.resolve`** (primary). It performs the SRV/A/AAAA lookup
///    without opening a socket, which matters because our service is
///    advertised over `_tcp` (per PROTOCOL.md §3.1) while the actual
///    transport is QUIC over UDP — there is nothing listening on TCP to
///    connect to. It also works when the record's SRV port is stale, because
///    we take the port from the TXT `p` field instead.
/// 2. **`NWConnection`** on the `.service` endpoint (fallback), reading
///    `currentPath?.remoteEndpoint` once the connection is ready. Used only
///    when `NetService` yields nothing.
///
/// The port always comes from TXT `p` when present — that is the QUIC UDP
/// port the peer's core actually bound; the SRV port is only a mirror of it.
@MainActor
enum BonjourResolver {
    private static let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "discovery")

    /// - Parameters:
    ///   - result: the browse result for the peer.
    ///   - txtPort: the peer's QUIC port from TXT `p`, if it published one.
    static func resolve(
        result: NWBrowser.Result,
        txtPort: UInt16?,
        timeout: TimeInterval = 5
    ) async -> String? {
        guard case .service(let name, let type, let domain, _) = result.endpoint else {
            // Already an address (UDP-fallback discovery path, §3.2).
            if case .hostPort(let host, let port) = result.endpoint {
                return format(host: hostString(host), port: txtPort ?? port.rawValue)
            }
            return nil
        }

        if let host = await resolveWithNetService(name: name, type: type, domain: domain,
                                                  timeout: timeout) {
            return format(host: host.address, port: txtPort ?? host.port)
        }
        logger.notice("NetService resolution failed for \(name); trying NWConnection")
        if let host = await resolveWithConnection(endpoint: result.endpoint, timeout: timeout) {
            return format(host: host.address, port: txtPort ?? host.port)
        }
        return nil
    }

    private static func format(host: String, port: UInt16) -> String {
        host.contains(":") ? "[\(host)]:\(port)" : "\(host):\(port)"
    }

    nonisolated private static func hostString(_ host: NWEndpoint.Host) -> String {
        switch host {
        case .name(let name, _): return name
        case .ipv4(let address): return "\(address)"
        case .ipv6(let address): return "\(address)"
        @unknown default: return "\(host)"
        }
    }

    // MARK: - NetService

    private static func resolveWithNetService(
        name: String, type: String, domain: String, timeout: TimeInterval
    ) async -> (address: String, port: UInt16)? {
        // NWBrowser reports the domain without the trailing dot and the type
        // without it too; NetService wants both fully qualified.
        let service = NetService(
            domain: domain.hasSuffix(".") ? domain : domain + ".",
            type: type.hasSuffix(".") ? type : type + ".",
            name: name
        )
        let delegate = ResolveDelegate()
        service.delegate = delegate
        return await withCheckedContinuation { continuation in
            // `NetService.delegate` is weak and the service must outlive the
            // resolve, so both are held by this closure.
            delegate.finish = {
                service.stop()
                continuation.resume(returning: bestAddress(of: service))
                withExtendedLifetime(delegate) {}
            }
            service.schedule(in: .main, forMode: .default)
            service.resolve(withTimeout: timeout)
        }
    }

    /// Ranks the resolved addresses: routable IPv4 first, then routable IPv6,
    /// then loopback.
    ///
    /// - IPv4 over IPv6 because an IPv6 link-local address needs a scope
    ///   suffix the core's `lookup_host` would have to re-parse.
    /// - Routable over loopback because a peer's `127.0.0.1` is meaningless
    ///   from another machine, and even for a peer on *this* machine the
    ///   host's own LAN address reaches it just as well (peers bind
    ///   `0.0.0.0`). mDNS happily hands back `127.0.0.1` first for a service
    ///   registered locally, and dialling it was observed to hang the QUIC
    ///   handshake from inside the app bundle.
    private static func bestAddress(of service: NetService?) -> (address: String, port: UInt16)? {
        guard let service, let addresses = service.addresses, !addresses.isEmpty else { return nil }
        let ranked = addresses.compactMap(parse(sockaddr:)).sorted { lhs, rhs in
            rank(lhs) < rank(rhs)
        }
        guard let best = ranked.first else { return nil }
        return (best.address, best.port)
    }

    private static func rank(_ parsed: (address: String, port: UInt16, isIPv4: Bool)) -> Int {
        let loopback = parsed.address.hasPrefix("127.") || parsed.address == "::1"
        switch (loopback, parsed.isIPv4) {
        case (false, true): return 0
        case (false, false): return 1
        case (true, true): return 2
        case (true, false): return 3
        }
    }

    private static func parse(sockaddr data: Data) -> (address: String, port: UInt16, isIPv4: Bool)? {
        data.withUnsafeBytes { raw -> (String, UInt16, Bool)? in
            guard let base = raw.bindMemory(to: sockaddr.self).baseAddress else { return nil }
            var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            guard getnameinfo(base, socklen_t(data.count), &host, socklen_t(host.count),
                              nil, 0, NI_NUMERICHOST) == 0 else { return nil }
            let address = String(cString: host)
            let family = base.pointee.sa_family
            let port: UInt16
            if family == sa_family_t(AF_INET) {
                port = raw.load(fromByteOffset: 2, as: UInt16.self).bigEndian
            } else if family == sa_family_t(AF_INET6) {
                port = raw.load(fromByteOffset: 2, as: UInt16.self).bigEndian
            } else {
                return nil
            }
            return (address, port, family == sa_family_t(AF_INET))
        }
    }

    private final class ResolveDelegate: NSObject, NetServiceDelegate {
        var finish: (() -> Void)?
        private var done = false

        private func complete() {
            guard !done else { return }
            done = true
            finish?()
            finish = nil
        }

        func netServiceDidResolveAddress(_ sender: NetService) { complete() }
        func netService(_ sender: NetService, didNotResolve errorDict: [String: NSNumber]) {
            complete()
        }
        func netServiceDidStop(_ sender: NetService) { complete() }
    }

    // MARK: - NWConnection fallback

    nonisolated private static func resolveWithConnection(
        endpoint: NWEndpoint, timeout: TimeInterval
    ) async -> (address: String, port: UInt16)? {
        // UDP: the transport really is UDP, and a TCP probe would be refused
        // (nothing listens on the advertised TCP port) before the path is
        // populated.
        let connection = NWConnection(to: endpoint, using: .udp)
        let box = ResumeBox()
        return await withCheckedContinuation { continuation in
            let finish: @Sendable ((address: String, port: UInt16)?) -> Void = { value in
                guard box.claim() else { return }
                connection.cancel()
                continuation.resume(returning: value)
            }
            connection.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    if let remote = connection.currentPath?.remoteEndpoint,
                       case .hostPort(let host, let port) = remote {
                        finish((hostString(host), port.rawValue))
                    } else {
                        finish(nil)
                    }
                case .failed, .cancelled:
                    finish(nil)
                default:
                    break
                }
            }
            connection.start(queue: .global(qos: .userInitiated))
            DispatchQueue.global().asyncAfter(deadline: .now() + timeout) { finish(nil) }
        }
    }

    private final class ResumeBox: @unchecked Sendable {
        private let lock = NSLock()
        private var claimed = false
        func claim() -> Bool {
            lock.lock()
            defer { lock.unlock() }
            if claimed { return false }
            claimed = true
            return true
        }
    }
}
