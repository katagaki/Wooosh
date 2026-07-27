import Foundation

// The core is the single source of identity (PROTOCOL.md §2): the app lends
// storage only, and never derives a DeviceID or fingerprint phrase itself.

extension Data {
    var lowercaseHex: String {
        map { String(format: "%02x", $0) }.joined()
    }
}
