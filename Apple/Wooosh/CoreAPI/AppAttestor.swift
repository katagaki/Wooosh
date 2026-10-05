import DeviceCheck
import Foundation
import os
import WoooshCoreFFI

/// App Attest for rendezvous admission (PROTOCOL.md §9.2). The core calls this
/// on its own blocking threads, never the main thread, so DeviceCheck's
/// completion handlers are waited on directly.
///
/// The key ID lives in UserDefaults rather than the Keychain on purpose: App
/// Attest keys do not survive a reinstall, and neither does UserDefaults, so
/// the two can never disagree about whether a key exists.
final class AppAttestor: Attestor, @unchecked Sendable {
    private static let keyIDDefaultsKey = "appAttestKeyID"
    private let service = DCAppAttestService.shared
    private let logger = Logger(subsystem: "com.tsubuzaki.Wooosh", category: "attest")

    func kind() -> AttestationKind {
        // False in the Simulator, and on Macs before macOS 27.
        service.isSupported ? .appAttest : .none
    }

    func appAttest(clientDataHash: Data, attest: Bool) -> AppAttestEvidence? {
        guard service.isSupported else { return nil }
        if attest {
            // A key can be attested once, so a fresh attestation always gets a fresh key.
            guard let keyID = wait({ self.service.generateKey(completionHandler: $0) }),
                  let object = wait({ self.service.attestKey(keyID, clientDataHash: clientDataHash, completionHandler: $0) })
            else { return nil }
            UserDefaults.standard.set(keyID, forKey: Self.keyIDDefaultsKey)
            return AppAttestEvidence(keyId: Data(base64Encoded: keyID) ?? Data(), attestation: object, assertion: nil)
        }
        guard let keyID = UserDefaults.standard.string(forKey: Self.keyIDDefaultsKey),
              let assertion = wait({ self.service.generateAssertion(keyID, clientDataHash: clientDataHash, completionHandler: $0) })
        else { return nil }
        return AppAttestEvidence(keyId: Data(base64Encoded: keyID) ?? Data(), attestation: nil, assertion: assertion)
    }

    func playIntegrity(requestHash: String) -> String? { nil }

    func tpmEvidence() -> TpmEvidence? { nil }

    func tpmActivate(credentialBlob: Data, encryptedSecret: Data) -> Data? { nil }

    private func wait<T>(_ call: (@escaping @Sendable (T?, Error?) -> Void) -> Void) -> T? {
        let done = DispatchSemaphore(value: 0)
        let box = ResultBox<T>()
        call { value, error in
            box.value = value
            box.error = error
            done.signal()
        }
        done.wait()
        if let error = box.error {
            logger.error("App Attest failed: \(error.localizedDescription)")
        }
        return box.value
    }
}

private final class ResultBox<T>: @unchecked Sendable {
    var value: T?
    var error: Error?
}
