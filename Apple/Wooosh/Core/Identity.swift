import Foundation

// The core is the single source of identity (PROTOCOL.md §2). The app
// contributes storage only, through `KeychainKeyStore`; DeviceIDs and
// fingerprint phrases are always rendered by the core, never derived here.

extension Data {
    var lowercaseHex: String {
        map { String(format: "%02x", $0) }.joined()
    }
}
