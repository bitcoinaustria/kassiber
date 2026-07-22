import Foundation

public struct NativeSecretStoreAvailability: Equatable, Sendable {
    public enum State: String, Equatable, Sendable { case available, lockedNeedsUnlock = "locked_needs_unlock", unavailable }
    public let state: State
    public let identityStrength: String
    public let reason: String?

    public init(state: State, identityStrength: String = "unknown_or_unsigned", reason: String? = nil) {
        self.state = state; self.identityStrength = identityStrength; self.reason = reason
    }

    var jsonValue: JSONValue {
        var object: [String: JSONValue] = ["state": .string(state.rawValue)]
        if state == .available { object["identity_strength"] = .string(identityStrength) }
        if let reason { object["reason"] = .string(reason) }
        return .object(object)
    }
}

public protocol DesktopNativeSecretStore: Sendable {
    var availability: NativeSecretStoreAvailability { get }
    func get(service: String, account: String) throws -> Data?
    func exists(service: String, account: String) throws -> Bool
    func set(service: String, account: String, secret: Data) throws
    func delete(service: String, account: String) throws
}

public enum NativeSecretStoreError: Error, Equatable, Sendable, CustomStringConvertible {
    case unavailable(String)
    case operation(String)

    public var description: String {
        switch self { case let .unavailable(message), let .operation(message): message }
    }
}

public enum NativeSecretStoreIdentifiers {
    public static let macOSKeychain = "macos_keychain"
    public static let sqlcipherInline = "sqlcipher_inline"
}

public struct UnavailableNativeSecretStore: DesktopNativeSecretStore {
    public init() {}
    public var availability: NativeSecretStoreAvailability {
        NativeSecretStoreAvailability(
            state: .unavailable,
            reason: "Native secret storage was not injected by the desktop host"
        )
    }
    public func get(service: String, account: String) throws -> Data? { throw error }
    public func exists(service: String, account: String) throws -> Bool { throw error }
    public func set(service: String, account: String, secret: Data) throws { throw error }
    public func delete(service: String, account: String) throws { throw error }
    private var error: NativeSecretStoreError {
        .unavailable("Native secret storage was not injected by the desktop host")
    }
}

public struct TouchIDPassphraseStatus: Equatable, Sendable {
    public let available: Bool
    public let configured: Bool
    public let reason: String?
    public init(available: Bool, configured: Bool, reason: String? = nil) {
        self.available = available; self.configured = configured; self.reason = reason
    }
}

public protocol TouchIDPassphraseManaging: Sendable {
    func status(account: String) async -> TouchIDPassphraseStatus
    func store(passphrase: String, account: String) async throws
    func retrieveAfterAuthentication(account: String, reason: String) async throws -> String?
    func delete(account: String) async throws
}

public struct UnavailableTouchIDPassphraseManager: TouchIDPassphraseManaging {
    public init() {}
    public func status(account: String) async -> TouchIDPassphraseStatus {
        TouchIDPassphraseStatus(available: false, configured: false, reason: "Touch ID service was not injected by the desktop host")
    }
    public func store(passphrase: String, account: String) async throws { throw unavailable }
    public func retrieveAfterAuthentication(account: String, reason: String) async throws -> String? { throw unavailable }
    public func delete(account: String) async throws { throw unavailable }
    private var unavailable: NativeSecretStoreError {
        .unavailable("Touch ID service was not injected by the desktop host")
    }
}

struct NativeSecretStorePolicy: Equatable, Sendable {
    static let inlineStoreID = NativeSecretStoreIdentifiers.sqlcipherInline
    let storeID: String
    let reason: String
    let nativeAvailable: Bool
    let warning: String?

    var jsonValue: JSONValue {
        var object: [String: JSONValue] = [
            "store_id": .string(storeID), "reason": .string(reason),
            "native_store_id": .string(NativeSecretStoreIdentifiers.macOSKeychain),
            "native_available": .bool(nativeAvailable),
        ]
        if let warning { object["warning"] = .string(warning) }
        return .object(object)
    }

    static func select(availability: NativeSecretStoreAvailability, requested: String?) -> Self {
        let available = availability.state == .available
        let warning = availability.identityStrength == "production" ? nil
            : "Keychain storage is experimental for unsigned or ad-hoc macOS builds; rebuilding or app identity changes can trigger access prompts."
        if let requested, !requested.isEmpty {
            if requested == inlineStoreID {
                return Self(storeID: inlineStoreID, reason: "requested", nativeAvailable: available, warning: warning)
            }
            if requested == NativeSecretStoreIdentifiers.macOSKeychain, available {
                return Self(storeID: requested, reason: "requested", nativeAvailable: true, warning: warning)
            }
            return Self(storeID: inlineStoreID, reason: "native_unavailable", nativeAvailable: false, warning: availability.reason)
        }
        if available && availability.identityStrength == "production" {
            return Self(storeID: NativeSecretStoreIdentifiers.macOSKeychain, reason: "production_default", nativeAvailable: true, warning: nil)
        }
        return Self(storeID: inlineStoreID, reason: "unsigned_macos_default", nativeAvailable: available, warning: warning)
    }
}
