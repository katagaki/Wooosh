import Foundation
import os
import WoooshCoreFFI

/// The core's `KeyStore` adapter (DESIGN.md §4 `PlatformAdapters.key_store`),
/// backed by the Keychain item this app has used since Milestone 1.
///
/// Milestone 3 makes the **core** the single source of identity. The app no
/// longer derives a DeviceID of its own; it only lends the core its storage.
/// The stored blob is the raw 32-byte Ed25519 seed, which is exactly what
/// `CryptoKit.Curve25519.Signing.PrivateKey.rawRepresentation` produced and
/// exactly what `ed25519_dalek::SigningKey::from_bytes` expects — so an
/// install that already has a key keeps it, and there is never more than one
/// keypair per install.
///
/// Called from a core thread inside `WoooshCore.start`, so this type is
/// deliberately not actor-isolated and touches no UI state.
final class KeychainKeyStore: KeyStore, @unchecked Sendable {
    private static let service = "com.tsubuzaki.Wooosh"
    private static let account = "identity.ed25519.private"

    private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "identity")

    /// Set when the Keychain refused us and the file fallback was used, so
    /// callers can log it instead of silently degrading.
    private(set) var usedFallback = false

    func loadIdentity() -> Data? {
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: Self.service,
            kSecAttrAccount: Self.account,
            kSecReturnData: true,
            kSecMatchLimit: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        switch status {
        case errSecSuccess:
            guard let data = item as? Data, data.count == 32 else {
                logger.error("Keychain identity has unexpected length; regenerating")
                return nil
            }
            return data
        case errSecItemNotFound:
            // Nothing pinned yet — but a previous run may have fallen back to
            // the file store (unsigned dev builds), so check that too.
            return fallbackLoad()
        default:
            logger.error("Keychain read failed (OSStatus \(status)); trying file fallback")
            return fallbackLoad()
        }
    }

    func storeIdentity(secret: Data) {
        guard secret.count == 32 else {
            logger.error("Refusing to store a \(secret.count)-byte identity secret")
            return
        }
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: Self.service,
            kSecAttrAccount: Self.account,
        ]
        var attributes = query
        attributes[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        attributes[kSecValueData] = secret

        var status = SecItemAdd(attributes as CFDictionary, nil)
        if status == errSecDuplicateItem {
            status = SecItemUpdate(
                query as CFDictionary,
                [kSecValueData: secret] as CFDictionary
            )
        }
        guard status == errSecSuccess else {
            logger.error("Keychain write failed (OSStatus \(status)); using file fallback")
            fallbackStore(secret)
            return
        }
        usedFallback = false
    }

    // MARK: - File fallback
    //
    // Ad-hoc-signed local builds (CODE_SIGNING_ALLOWED=NO) get no keychain
    // access group, so SecItemAdd fails with errSecMissingEntitlement. Rather
    // than generate a fresh identity on every launch — which would break the
    // "exactly one keypair per install" invariant this milestone exists to
    // fix — fall back to an app-private file. Properly signed builds never
    // reach this path.

    private var fallbackURL: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        let dir = base.appendingPathComponent("Wooosh", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("identity.key")
    }

    private func fallbackLoad() -> Data? {
        guard let data = try? Data(contentsOf: fallbackURL), data.count == 32 else { return nil }
        usedFallback = true
        return data
    }

    private func fallbackStore(_ secret: Data) {
        do {
            try secret.write(to: fallbackURL, options: .atomic)
            try? FileManager.default.setAttributes(
                [.posixPermissions: 0o600], ofItemAtPath: fallbackURL.path)
            usedFallback = true
        } catch {
            logger.error("Identity fallback write failed: \(error)")
        }
    }
}
