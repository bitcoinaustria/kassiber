import Foundation
import LocalAuthentication
import Security
import KassiberDaemonKit

enum MacOSNativeServices {
    static let keychain = MacOSKeychainSecretStore()
    static let touchID = MacOSTouchIDPassphraseManager(keychain: keychain)
}

final class MacOSKeychainSecretStore: DesktopNativeSecretStore, @unchecked Sendable {
    var availability: NativeSecretStoreAvailability {
        NativeSecretStoreAvailability(state: .available, identityStrength: signingIdentityStrength)
    }

    func get(service: String, account: String) throws -> Data? {
        var request = query(service: service, account: account)
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw keychainError(status, action: "read") }
        guard let data = result as? Data else {
            throw NativeSecretStoreError.operation("Keychain returned an invalid secret value")
        }
        return data
    }

    func exists(service: String, account: String) throws -> Bool {
        var request = query(service: service, account: account)
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        let status = SecItemCopyMatching(request as CFDictionary, nil)
        if status == errSecItemNotFound { return false }
        guard status == errSecSuccess else { throw keychainError(status, action: "inspect") }
        return true
    }

    func set(service: String, account: String, secret: Data) throws {
        let request = query(service: service, account: account)
        let update = SecItemUpdate(
            request as CFDictionary,
            [kSecValueData as String: secret] as CFDictionary
        )
        if update == errSecSuccess { return }
        guard update == errSecItemNotFound else { throw keychainError(update, action: "update") }
        var addition = request
        addition[kSecValueData as String] = secret
        addition[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlocked
        let status = SecItemAdd(addition as CFDictionary, nil)
        guard status == errSecSuccess else { throw keychainError(status, action: "store") }
    }

    func delete(service: String, account: String) throws {
        let status = SecItemDelete(query(service: service, account: account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw keychainError(status, action: "delete")
        }
    }

    private func query(service: String, account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    private func keychainError(_ status: OSStatus, action: String) -> NativeSecretStoreError {
        let detail = SecCopyErrorMessageString(status, nil) as String? ?? "OSStatus \(status)"
        return .operation("Could not \(action) Keychain secret: \(detail)")
    }

    private var signingIdentityStrength: String {
        if let declared = Bundle.main.object(forInfoDictionaryKey: "KassiberSigningIdentityStrength") as? String {
            return declared
        }
        return ProcessInfo.processInfo.environment["KASSIBER_PRODUCTION_SIGNED"] != nil
            ? "production"
            : ProcessInfo.processInfo.environment["KASSIBER_ADHOC_SIGNED"] != nil ? "adhoc" : "unknown_or_unsigned"
    }
}

actor MacOSTouchIDPassphraseManager: TouchIDPassphraseManaging {
    private static let service = "Kassiber Database Passphrase"
    private let keychain: MacOSKeychainSecretStore

    init(keychain: MacOSKeychainSecretStore) { self.keychain = keychain }

    func status(account: String) async -> TouchIDPassphraseStatus {
        let context = LAContext()
        var error: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error) else {
            return TouchIDPassphraseStatus(
                available: false,
                configured: false,
                reason: error?.localizedDescription ?? "Touch ID is not available or enrolled"
            )
        }
        do {
            return TouchIDPassphraseStatus(
                available: true,
                configured: try keychain.exists(service: Self.service, account: account)
            )
        } catch {
            return TouchIDPassphraseStatus(available: true, configured: false, reason: String(describing: error))
        }
    }

    func store(passphrase: String, account: String) async throws {
        guard !passphrase.isEmpty else { throw NativeSecretStoreError.operation("database passphrase must not be empty") }
        try keychain.set(service: Self.service, account: account, secret: Data(passphrase.utf8))
    }

    func retrieveAfterAuthentication(account: String, reason: String) async throws -> String? {
        guard try keychain.exists(service: Self.service, account: account) else { return nil }
        let context = LAContext()
        var availabilityError: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &availabilityError) else {
            throw NativeSecretStoreError.unavailable(
                availabilityError?.localizedDescription ?? "Touch ID is not available or enrolled"
            )
        }
        let accepted = try await context.evaluatePolicy(
            .deviceOwnerAuthenticationWithBiometrics,
            localizedReason: reason
        )
        guard accepted else { throw NativeSecretStoreError.operation("Touch ID authentication was cancelled or failed") }
        guard let bytes = try keychain.get(service: Self.service, account: account) else { return nil }
        guard let passphrase = String(data: bytes, encoding: .utf8) else {
            throw NativeSecretStoreError.operation("stored database passphrase is not UTF-8")
        }
        return passphrase
    }

    func delete(account: String) async throws {
        try keychain.delete(service: Self.service, account: account)
    }
}

func canonicalTouchIDAccount(for dataRoot: String?) -> String {
    let selected: URL
    if let dataRoot, !dataRoot.isEmpty {
        selected = URL(fileURLWithPath: dataRoot, isDirectory: true)
    } else if let configured = ProcessInfo.processInfo.environment["KASSIBER_DATA_ROOT"], !configured.isEmpty {
        selected = URL(fileURLWithPath: configured, isDirectory: true)
    } else {
        selected = FileManager.default.homeDirectoryForCurrentUser
            .appending(path: ".kassiber/projects/default/data", directoryHint: .isDirectory)
    }
    return selected.resolvingSymlinksInPath().standardizedFileURL.path
}
