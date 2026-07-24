import Foundation

// Milestone 3: the app no longer owns an identity.
//
// It used to generate its own Ed25519 keypair and derive a DeviceID as
// `SHA256(pubkey)[0..16]`, while the Rust core independently owned a second
// keypair through its `KeyStore` adapter — two identities per install, and a
// DeviceID that no other implementation would ever agree with (the protocol
// says BLAKE3, PROTOCOL.md §2).
//
// The core is now the single source of identity. The app's contribution is
// storage only: `KeychainKeyStore` (CoreAPI/) hands the core the same
// Keychain-held 32-byte Ed25519 seed the app has always stored, so existing
// installs keep their key, and `core.deviceId()` / `core.fingerprintPhrase()`
// are what the UI displays.

// The base32 DeviceID encoder that used to live here is gone with the rest of
// the shell-side identity code: DeviceIDs are rendered by the core
// (`deviceIdFor` / `TrustedPeer.deviceId`), never by the app.

extension Data {
    var lowercaseHex: String {
        map { String(format: "%02x", $0) }.joined()
    }
}
