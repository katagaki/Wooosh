import Foundation
import os
import WoooshCoreFFI

/// Storage only; the app derives no identity of its own (DESIGN.md §4).
///
/// The blob is the raw 32-byte Ed25519 seed, the format `ed25519_dalek` expects;
/// changing it would strand an existing install's key. Called from a core thread
/// inside `start`, so this is not actor-isolated and touches no UI state.
final class KeychainKeyStore: KeyStore, @unchecked Sendable {
    private static let service = "com.tsubuzaki.Wooosh"
    private static let account = "identity.ed25519.private"

    private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "identity")

    /// Lets callers log the degradation instead of failing silently.
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
            // A previous unsigned run may have fallen back to the file store.
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
    // Ad-hoc-signed builds get no keychain access group, and regenerating each
    // launch would break one-keypair-per-install. Signed builds never get here.

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
