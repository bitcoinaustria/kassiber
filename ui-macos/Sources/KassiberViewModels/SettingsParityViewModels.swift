import Foundation
import Observation
import KassiberDaemonKit

public struct SettingsBackendRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let kind: String
    public let chain: String
    public let network: String
    public let url: String
    public let timeout: Int
    public let notes: String
    public let isDefault: Bool
    public let enabled: Bool
    public let walletReferences: [String]
    public let hasCredentials: Bool
    public let trustSelfSigned: Bool
    public let silentPayments: Bool

    init?(_ row: [String: JSONValue]) {
        guard let name = row.string("name"), !name.isEmpty else { return nil }
        id = name
        self.name = name
        kind = row.string("kind") ?? ""
        chain = row.string("chain") ?? ""
        network = row.string("network") ?? ""
        url = row.string("url") ?? ""
        timeout = Int(row.int("timeout") ?? 10)
        notes = row.string("notes") ?? ""
        isDefault = row.bool("is_default") ?? false
        enabled = row.bool("enabled", "on") ?? true
        walletReferences = row["wallet_refs"]?.arrayValue?.compactMap(\.stringValue) ?? []
        hasCredentials = ["has_token", "has_auth_header", "has_cookiefile", "has_username"]
            .contains { row.bool($0) == true }
        trustSelfSigned = row.bool("insecure") ?? false
        silentPayments = row.bool("silent_payments") ?? false
    }
}

public struct BackendDraft: Equatable, Sendable {
    public var name = ""
    public var kind = "esplora"
    public var url = ""
    public var chain = "bitcoin"
    public var network = "main"
    public var authHeader = ""
    public var token = ""
    public var proxy = ""
    public var notes = ""
    public var certificate = ""
    public var trustSelfSigned = false
    public var cookieFile = ""
    public var username = ""
    public var password = ""
    public var commandoPeerID = ""
    public var lightningCLI = ""
    public var lightningDirectory = ""
    public var rpcFile = ""
    public var timeout = 10
    public var silentPayments = false

    public init() {}

    public init(row: SettingsBackendRow) {
        name = row.name
        kind = row.kind
        url = row.url
        chain = row.chain
        network = row.network
        timeout = row.timeout
        notes = row.notes
        trustSelfSigned = row.trustSelfSigned
        silentPayments = row.silentPayments
    }
}

public struct BackendProbeResult: Equatable, Sendable {
    public let ok: Bool
    public let headline: String
    public let details: [String]
    public let credentialReference: String?
}

@MainActor
@Observable
public final class BackendSettingsViewModel {
    public private(set) var backends: [SettingsBackendRow] = []
    public private(set) var defaultBackend: String?
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?
    public private(set) var resultMessage: String?
    public private(set) var probe: BackendProbeResult?

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func load() async {
        await perform(reload: false) {
            let envelope = try await daemon.invoke(.uiBackendsSettingsList, args: nil)
            try Self.requireSuccess(envelope)
            let data = envelope.data?.objectValue ?? [:]
            backends = data.objects("backends").compactMap(SettingsBackendRow.init)
            defaultBackend = data["summary"]?.objectValue?.string("default_backend")
        }
    }

    public func save(_ draft: BackendDraft, editing originalName: String? = nil) async {
        await perform {
            var args: [String: JSONValue]
            if let originalName {
                guard let original = backends.first(where: { $0.name == originalName }) else {
                    throw DaemonClientError.transport("backend_reload_required")
                }
                args = Self.backendUpdateArguments(draft, original: original)
                args["name"] = .string(originalName)
            } else {
                args = Self.backendArguments(draft)
                args["name"] = .string(draft.name.trimmingCharacters(in: .whitespacesAndNewlines))
            }
            let kind: DaemonKind = originalName == nil ? .uiBackendsCreate : .uiBackendsUpdate
            let envelope = try await daemon.invoke(kind, args: args)
            try Self.requireSuccess(envelope)
            resultMessage = originalName == nil ? "backend_created" : "backend_updated"
        }
    }

    public func delete(_ backend: SettingsBackendRow) async {
        await perform {
            let envelope = try await daemon.invoke(.uiBackendsDelete, args: ["name": .string(backend.name)])
            try Self.requireSuccess(envelope)
            resultMessage = "backend_deleted"
        }
    }

    public func setDefault(_ backend: SettingsBackendRow) async {
        await perform {
            let envelope = try await daemon.invoke(.uiBackendsSetDefault, args: ["name": .string(backend.name)])
            try Self.requireSuccess(envelope)
            resultMessage = "backend_default_changed"
        }
    }

    public func detectBitcoinCore() async {
        await perform(reload: false) {
            let envelope = try await daemon.invoke(.uiBackendsDetectCore, args: [:])
            try Self.requireSuccess(envelope)
            probe = Self.parseProbe(envelope.data, fallback: "Bitcoin Core")
        }
    }

    public func test(_ draft: BackendDraft) async {
        await perform(reload: false) {
            let kind: DaemonKind
            let args: [String: JSONValue]
            switch draft.kind.lowercased() {
            case "bitcoinrpc":
                kind = .uiBackendsBitcoinrpcTest
                var config: [String: JSONValue] = ["display_name": .string(draft.name)]
                if !draft.cookieFile.isEmpty { config["cookiefile"] = .string(draft.cookieFile) }
                if !draft.username.isEmpty { config["username"] = .string(draft.username) }
                if !draft.password.isEmpty { config["password"] = .string(draft.password) }
                args = [
                    "url": .string(draft.url), "network": .string(draft.network),
                    "config": .object(config), "timeout": .integer(Int64(draft.timeout)),
                ]
            case "electrum", "fulcrum":
                kind = .uiBackendsElectrumTest
                var values: [String: JSONValue] = [
                    "url": .string(draft.url),
                    "trust_self_signed": .bool(draft.trustSelfSigned),
                    "timeout": .integer(Int64(draft.timeout)),
                ]
                if !draft.certificate.isEmpty { values["certificate"] = .string(draft.certificate) }
                if !draft.proxy.isEmpty { values["proxy"] = .string(draft.proxy) }
                args = values
            case "lnd", "cln", "coreln", "core-lightning", "nwc":
                kind = .uiBackendsLightningTest
                args = Self.backendArguments(draft)
            default:
                kind = .uiBackendsHttpTest
                args = [
                    "url": .string(draft.url), "proxy": .string(draft.proxy),
                    "timeout": .integer(Int64(draft.timeout)),
                ]
            }
            let envelope = try await daemon.invoke(kind, args: args)
            try Self.requireSuccess(envelope)
            probe = Self.parseProbe(envelope.data, fallback: draft.name)
        }
    }

    private func perform(reload: Bool = true, _ operation: () async throws -> Void) async {
        isWorking = true
        defer { isWorking = false }
        do {
            try await operation()
            errorMessage = nil
            if reload {
                let envelope = try await daemon.invoke(.uiBackendsSettingsList, args: nil)
                try Self.requireSuccess(envelope)
                let data = envelope.data?.objectValue ?? [:]
                backends = data.objects("backends").compactMap(SettingsBackendRow.init)
                defaultBackend = data["summary"]?.objectValue?.string("default_backend")
            }
        } catch { errorMessage = Self.message(error) }
    }

    private static func backendArguments(_ draft: BackendDraft) -> [String: JSONValue] {
        var config: [String: JSONValue] = [:]
        let normalizedKind = draft.kind.lowercased()
        if ["esplora", "electrum", "fulcrum", "bitcoinrpc"].contains(normalizedKind),
           draft.chain.lowercased() == "bitcoin" {
            config["silent_payments"] = .bool(draft.silentPayments)
        }
        if ["electrum", "fulcrum", "lnd"].contains(normalizedKind) {
            config["insecure"] = .bool(draft.trustSelfSigned)
        }
        if !draft.certificate.isEmpty { config["certificate"] = .string(draft.certificate) }
        if !draft.cookieFile.isEmpty { config["cookiefile"] = .string(draft.cookieFile) }
        if !draft.username.isEmpty { config["username"] = .string(draft.username) }
        if !draft.password.isEmpty { config["password"] = .string(draft.password) }
        if !draft.commandoPeerID.isEmpty { config["commando_peer_id"] = .string(draft.commandoPeerID) }
        if !draft.lightningCLI.isEmpty { config["lightning_cli"] = .string(draft.lightningCLI) }
        if !draft.lightningDirectory.isEmpty { config["lightning_dir"] = .string(draft.lightningDirectory) }
        if !draft.rpcFile.isEmpty { config["rpc_file"] = .string(draft.rpcFile) }
        var args: [String: JSONValue] = [
            "name": .string(draft.name.trimmingCharacters(in: .whitespacesAndNewlines)),
            "kind": .string(draft.kind), "url": .string(draft.url),
            "chain": .string(draft.chain), "network": .string(draft.network),
            "timeout": .integer(Int64(draft.timeout)), "config": .object(config),
        ]
        if !draft.authHeader.isEmpty { args["auth_header"] = .string(draft.authHeader) }
        if !draft.token.isEmpty { args["token"] = .string(draft.token) }
        if !draft.proxy.isEmpty { args["tor_proxy"] = .string(draft.proxy) }
        if !draft.notes.isEmpty { args["notes"] = .string(draft.notes) }
        return args
    }

    /// Builds a partial update from the redacted settings row. Blank secret
    /// controls mean "keep the stored value"; they are never populated through
    /// a reveal call and never overwrite a stored credential accidentally.
    private static func backendUpdateArguments(
        _ draft: BackendDraft, original: SettingsBackendRow
    ) -> [String: JSONValue] {
        var args: [String: JSONValue] = [:]
        var config: [String: JSONValue] = [:]
        var clear: [JSONValue] = []
        let kind = draft.kind.lowercased()

        if draft.kind != original.kind { args["kind"] = .string(draft.kind) }
        if draft.url != original.url { args["url"] = .string(draft.url) }
        if draft.chain != original.chain { args["chain"] = .string(draft.chain) }
        if draft.network != original.network { args["network"] = .string(draft.network) }
        if draft.timeout != original.timeout { args["timeout"] = .integer(Int64(draft.timeout)) }
        if draft.notes != original.notes {
            if draft.notes.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                clear.append(.string("notes"))
            } else {
                args["notes"] = .string(draft.notes)
            }
        }

        if !draft.authHeader.isEmpty { args["auth_header"] = .string(draft.authHeader) }
        if !draft.token.isEmpty { args["token"] = .string(draft.token) }
        if !draft.proxy.isEmpty { args["tor_proxy"] = .string(draft.proxy) }
        if ["electrum", "fulcrum", "lnd"].contains(kind),
           draft.trustSelfSigned != original.trustSelfSigned {
            config["insecure"] = .bool(draft.trustSelfSigned)
        }
        if ["esplora", "electrum", "fulcrum", "bitcoinrpc"].contains(kind),
           draft.chain.lowercased() == "bitcoin", draft.silentPayments != original.silentPayments {
            config["silent_payments"] = .bool(draft.silentPayments)
        }
        if !draft.certificate.isEmpty { config["certificate"] = .string(draft.certificate) }
        if !draft.cookieFile.isEmpty { config["cookiefile"] = .string(draft.cookieFile) }
        if !draft.username.isEmpty { config["username"] = .string(draft.username) }
        if !draft.password.isEmpty { config["password"] = .string(draft.password) }
        if !draft.commandoPeerID.isEmpty { config["commando_peer_id"] = .string(draft.commandoPeerID) }
        if !draft.lightningCLI.isEmpty { config["lightning_cli"] = .string(draft.lightningCLI) }
        if !draft.lightningDirectory.isEmpty { config["lightning_dir"] = .string(draft.lightningDirectory) }
        if !draft.rpcFile.isEmpty { config["rpc_file"] = .string(draft.rpcFile) }
        if !config.isEmpty { args["config"] = .object(config) }
        if !clear.isEmpty { args["clear"] = .array(clear) }
        return args
    }

    private static func parseProbe(_ data: JSONValue?, fallback: String) -> BackendProbeResult {
        let object = data?.objectValue ?? [:]
        let logs = object["logs"]?.arrayValue?.compactMap(\.stringValue) ?? []
        let credential = object.string("credential_ref")
            ?? object["candidate"]?.objectValue?.string("credential_ref")
        let ok = object.bool("ok", "reachable") ?? !logs.contains { $0.lowercased().contains("failed") }
        let headline = object.string("message", "status", "version") ?? fallback
        return BackendProbeResult(ok: ok, headline: headline, details: logs, credentialReference: credential)
    }

    static func requireSuccess(_ envelope: DaemonEnvelope) throws {
        if let error = envelope.error { throw error }
        if envelope.kind == "auth_required" { throw DaemonClientError.transport("database_authentication_required") }
    }

    static func message(_ error: Error) -> String {
        if let daemon = error as? DaemonErrorPayload { return daemon.message }
        if let client = error as? DaemonClientError {
            switch client {
            case let .kindNotAllowed(value), let .daemonNotReady(value), let .daemonExited(value),
                 let .protocolError(value), let .requestConflict(value), let .transport(value):
                return value
            case let .requestTimedOut(kind, _): return "request_timed_out_\(kind)"
            }
        }
        return String(describing: error)
    }
}

public struct AIProviderRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let displayName: String
    public let baseURL: String
    public let kind: String
    public let defaultModel: String?
    public let hasAPIKey: Bool
    public let secretStore: String
    public let isDefault: Bool
    public let acknowledged: Bool

    fileprivate init?(_ row: [String: JSONValue]) {
        guard let name = row.string("name"), !name.isEmpty else { return nil }
        id = name; self.name = name
        displayName = row.string("display_name") ?? name
        baseURL = row.string("base_url") ?? ""
        kind = row.string("kind") ?? "local"
        defaultModel = row.string("default_model")
        hasAPIKey = row.bool("has_api_key") ?? false
        secretStore = row["secret_ref"]?.objectValue?.string("store_id") ?? ""
        isDefault = row.bool("is_default") ?? false
        acknowledged = row.string("acknowledged_at") != nil
    }
}

public struct AIProviderDraft: Equatable, Sendable {
    public var name = ""
    public var displayName = ""
    public var baseURL = "http://127.0.0.1:11434/v1"
    public var kind = "local"
    public var defaultModel = ""
    public var notes = ""
    public var apiKey = ""
    public var acknowledged = false
    public init() {}
    public init(row: AIProviderRow) {
        name = row.name; displayName = row.displayName; baseURL = row.baseURL
        kind = row.kind; defaultModel = row.defaultModel ?? ""; acknowledged = row.acknowledged
    }
}

@MainActor
@Observable
public final class AIProviderSettingsViewModel {
    public private(set) var providers: [AIProviderRow] = []
    public private(set) var policyWarning: String?
    public private(set) var nativeSecretStore: String?
    public private(set) var nativeSecretStoreAvailable = false
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func load() async { await perform(reload: false) { try await reload() } }

    public func save(_ draft: AIProviderDraft, editing originalName: String? = nil) async {
        await perform {
            var args: [String: JSONValue] = [
                "name": .string(originalName ?? draft.name.trimmingCharacters(in: .whitespacesAndNewlines)),
                "display_name": .string(draft.displayName), "base_url": .string(draft.baseURL),
                "kind": .string(draft.kind), "acknowledged": .bool(draft.acknowledged),
            ]
            if !draft.defaultModel.isEmpty { args["default_model"] = .string(draft.defaultModel) }
            if !draft.notes.isEmpty { args["notes"] = .string(draft.notes) }
            let kind: DaemonKind = originalName == nil ? .aiProvidersCreate : .aiProvidersUpdate
            let envelope = try await daemon.invoke(kind, args: args)
            try BackendSettingsViewModel.requireSuccess(envelope)
            if !draft.apiKey.isEmpty {
                let keyResult = try await daemon.invoke(.aiProvidersSetApiKey, args: [
                    "name": .string(originalName ?? draft.name), "api_key": .string(draft.apiKey),
                ])
                try BackendSettingsViewModel.requireSuccess(keyResult)
            }
        }
    }

    public func setDefault(_ row: AIProviderRow) async {
        await perform {
            let result = try await daemon.invoke(.aiProvidersSetDefault, args: ["name": .string(row.name)])
            try BackendSettingsViewModel.requireSuccess(result)
        }
    }

    public func clearDefault() async {
        await perform {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.aiProvidersClearDefault, args: [:]))
        }
    }

    public func delete(_ row: AIProviderRow) async {
        await perform {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                .aiProvidersDelete, args: ["name": .string(row.name)]
            ))
        }
    }

    public func acknowledge(_ row: AIProviderRow) async {
        await perform {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                .aiProvidersAcknowledge, args: ["name": .string(row.name)]
            ))
        }
    }

    public func moveAPIKey(_ row: AIProviderRow, to storeID: String, replacementKey: String? = nil) async {
        await perform {
            var args: [String: JSONValue] = ["name": .string(row.name), "store_id": .string(storeID)]
            if let replacementKey { args["api_key"] = .string(replacementKey) }
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.aiProvidersMoveApiKey, args: args))
        }
    }

    public func test(_ row: AIProviderRow) async {
        await perform(reload: false) {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                .aiTestConnection, args: ["provider": .string(row.name)]
            ))
        }
    }

    private func perform(reload shouldReload: Bool = true, _ operation: () async throws -> Void) async {
        isWorking = true; defer { isWorking = false }
        do {
            try await operation()
            if shouldReload { try await reload() }
            errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    private func reload() async throws {
        let envelope = try await daemon.invoke(.aiProvidersList, args: nil)
        try BackendSettingsViewModel.requireSuccess(envelope)
        let data = envelope.data?.objectValue ?? [:]
        providers = data.objects("providers").compactMap(AIProviderRow.init)
        let policy = data["secret_store_policy"]?.objectValue ?? [:]
        let defaults = policy["default"]?.objectValue ?? [:]
        policyWarning = defaults.string("warning")
        nativeSecretStore = defaults.string("native_store_id")
        nativeSecretStoreAvailable = defaults.bool("native_available") ?? false
    }
}

public struct MaintenanceSettings: Equatable, Sendable {
    public var autoSyncBeforeReportReads = false
    public var backgroundEnabled = false
    public var reportReadSync = false
    public var onchainWallet = false
    public var btcpayWallet = false
    public var btcpayProvenance = false
    public var marketRates = true
    public var journals = true
    public var bitcoinRailCarryingValue = true
    public var marketRateProvider = "coinbase-exchange"
    public var marketRateProviders: [String] = []
    public var activeRatePair = ""
    public init() {}
}

@MainActor
@Observable
public final class MaintenanceSettingsViewModel {
    public var settings = MaintenanceSettings()
    public private(set) var blockers: [String] = []
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?
    public private(set) var resultMessage: String?
    private let daemon: any DaemonClient

    public init(daemon: any DaemonClient) { self.daemon = daemon }
    public func load() async { await operation(reload: false) { try await reload() } }

    public func save() async {
        await operation {
            let envelope = try await daemon.invoke(.uiMaintenanceConfigure, args: [
                "auto_sync_before_report_reads": .bool(settings.autoSyncBeforeReportReads),
                "background_enabled": .bool(settings.backgroundEnabled),
                "report_read_sync": .bool(settings.reportReadSync),
                "onchain_wallet": .bool(settings.onchainWallet),
                "btcpay_wallet": .bool(settings.btcpayWallet),
                "btcpay_provenance": .bool(settings.btcpayProvenance),
                "market_rates": .bool(settings.marketRates),
                "journals": .bool(settings.journals),
                "bitcoin_rail_carrying_value": .bool(settings.bitcoinRailCarryingValue),
                "market_rate_provider": .string(settings.marketRateProvider),
            ])
            try BackendSettingsViewModel.requireSuccess(envelope)
            resultMessage = "maintenance_saved"
        }
    }

    public func run(syncMode: String) async {
        await operation {
            let envelope = try await daemon.invoke(.uiMaintenanceRun, args: ["sync": .string(syncMode)])
            try BackendSettingsViewModel.requireSuccess(envelope)
            let data = envelope.data?.objectValue ?? [:]
            blockers = (data["blockers"]?.arrayValue ?? []).map(Self.describe)
            resultMessage = data.bool("ready") == true ? "maintenance_ready" : "maintenance_blocked"
        }
    }

    private func operation(reload shouldReload: Bool = true, _ body: () async throws -> Void) async {
        isWorking = true; defer { isWorking = false }
        do {
            try await body(); if shouldReload { try await reload() }; errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    private func reload() async throws {
        let envelope = try await daemon.invoke(.uiMaintenanceSettings, args: nil)
        try BackendSettingsViewModel.requireSuccess(envelope)
        parse(envelope.data)
    }

    private func parse(_ data: JSONValue?) {
        let root = data?.objectValue ?? [:]
        let row = root["settings"]?.objectValue ?? root
        let sources = row["source_classes"]?.objectValue ?? [:]
        settings.autoSyncBeforeReportReads = row.bool("auto_sync_before_report_reads") ?? false
        settings.backgroundEnabled = row.bool("background_enabled") ?? false
        settings.reportReadSync = row.bool("report_read_sync") ?? false
        settings.onchainWallet = sources.bool("onchain_wallet") ?? false
        settings.btcpayWallet = sources.bool("btcpay_wallet") ?? false
        settings.btcpayProvenance = sources.bool("btcpay_provenance") ?? false
        settings.marketRates = sources.bool("market_rates") ?? true
        settings.journals = sources.bool("journals") ?? true
        settings.bitcoinRailCarryingValue = row.bool("bitcoin_rail_carrying_value") ?? true
        settings.marketRateProvider = row.string("market_rate_provider") ?? "coinbase-exchange"
        settings.marketRateProviders = row["market_rate_providers"]?.arrayValue?.compactMap(\.stringValue) ?? []
        settings.activeRatePair = row.string("active_rate_pair") ?? ""
    }

    private static func describe(_ value: JSONValue) -> String {
        if let text = value.stringValue { return text }
        guard let row = value.objectValue else { return String(describing: value) }
        return row.string("message", "code", "kind") ?? String(describing: row)
    }
}

@MainActor
@Observable
public final class SecuritySettingsViewModel {
    public private(set) var isWorking = false
    public private(set) var isEncrypted = false
    public private(set) var resultMessage: String?
    public private(set) var backupPath: String?
    public private(set) var errorMessage: String?
    public private(set) var touchIDWarning: String?
    public private(set) var touchIDStatus = TouchIDPassphraseStatus(
        available: false,
        configured: false
    )
    private let daemon: any DaemonClient
    private let touchIDManager: any TouchIDPassphraseManaging
    private var touchIDAccount: String

    public init(
        daemon: any DaemonClient,
        touchIDManager: any TouchIDPassphraseManaging = UnavailableTouchIDPassphraseManager(),
        touchIDAccount: String = "default"
    ) {
        self.daemon = daemon
        self.touchIDManager = touchIDManager
        self.touchIDAccount = touchIDAccount
    }

    public func loadStatus() async {
        await perform {
            let envelope = try await daemon.invoke(.status, args: nil)
            try BackendSettingsViewModel.requireSuccess(envelope)
            let data = envelope.data?.objectValue ?? [:]
            isEncrypted = data.bool("database_encrypted") ?? false
            updateTouchIDAccount(data.string("data_root"))
        }
        await refreshTouchIDStatus()
    }

    public func initializeEncryption(passphrase: String, migrateCredentials: Bool) async {
        await perform {
            let envelope = try await daemon.invoke(.uiSecretsInit, args: [
                "auth_response": ["passphrase_secret": .string(passphrase)],
                "migrate_credentials": .bool(migrateCredentials),
            ])
            try BackendSettingsViewModel.requireSuccess(envelope)
            let data = envelope.data?.objectValue ?? [:]
            isEncrypted = data.bool("encrypted") ?? true
            backupPath = data.string("backup_path")
            resultMessage = data.bool("already_encrypted") == true ? "already_encrypted" : "encryption_enabled"
        }
    }

    public func changePassphrase(current: String, new: String) async {
        await perform {
            let envelope = try await daemon.invoke(.uiSecretsChangePassphrase, args: [
                "auth_response": ["passphrase_secret": .string(current)],
                "new_passphrase_secret": .string(new),
            ])
            try BackendSettingsViewModel.requireSuccess(envelope)
            resultMessage = "passphrase_changed"
            if touchIDStatus.configured {
                do {
                    try await touchIDManager.store(passphrase: new, account: touchIDAccount)
                    touchIDWarning = nil
                } catch {
                    try? await touchIDManager.delete(account: touchIDAccount)
                    touchIDWarning = String(describing: error)
                    resultMessage = "passphrase_changed_touch_id_disabled"
                }
            }
        }
        await refreshTouchIDStatus()
    }

    public func lock() async {
        await perform {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.daemonLock, args: [:]))
            resultMessage = "locked"
        }
    }

    public func unlock(passphrase: String) async {
        await perform {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                .daemonUnlock, args: ["auth_response": ["passphrase_secret": .string(passphrase)]]
            ))
            resultMessage = "unlocked"
        }
    }

    public func enrollTouchID(passphrase: String) async {
        await perform {
            let verified = try await daemon.invoke(
                .daemonUnlock,
                args: ["auth_response": ["passphrase_secret": .string(passphrase)]]
            )
            try BackendSettingsViewModel.requireSuccess(verified)
            let root = verified.data?.objectValue
            updateTouchIDAccount((root?["status"]?.objectValue ?? root)?.string("data_root"))
            try await touchIDManager.store(passphrase: passphrase, account: touchIDAccount)
            resultMessage = "touch_id_enabled"
        }
        await refreshTouchIDStatus()
    }

    public func forgetTouchID() async {
        await perform {
            try await touchIDManager.delete(account: touchIDAccount)
            resultMessage = "touch_id_disabled"
        }
        await refreshTouchIDStatus()
    }

    public func refreshTouchIDStatus() async {
        touchIDStatus = await touchIDManager.status(account: touchIDAccount)
    }

    private func updateTouchIDAccount(_ dataRoot: String?) {
        guard let dataRoot, !dataRoot.isEmpty else { return }
        touchIDAccount = URL(fileURLWithPath: dataRoot, isDirectory: true)
            .resolvingSymlinksInPath().standardizedFileURL.path
    }

    private func perform(_ body: () async throws -> Void) async {
        isWorking = true; defer { isWorking = false }
        do { try await body(); errorMessage = nil }
        catch { errorMessage = BackendSettingsViewModel.message(error) }
    }
}

@MainActor
@Observable
public final class RatesSettingsViewModel {
    public private(set) var summary = ""
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?
    public private(set) var resultMessage: String?
    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func load() async {
        await perform {
            let envelope = try await daemon.invoke(.uiRatesSummary, args: nil)
            try BackendSettingsViewModel.requireSuccess(envelope)
            let row = envelope.data?.objectValue ?? [:]
            let pair = row.string("pair", "active_pair") ?? row["summary"]?.objectValue?.string("pair") ?? ""
            let count = row.int("samples") ?? row["summary"]?.objectValue?.int("samples") ?? 0
            summary = pair.isEmpty ? String(count) : "\(pair) · \(count)"
        }
    }

    public func importKrakenCSV(path: String, pair: String?, operation: String) async {
        await perform {
            var args: [String: JSONValue] = ["path": .string(path), "operation": .string(operation)]
            if let pair, !pair.isEmpty { args["pair"] = .string(pair) }
            let envelope = try await daemon.invoke(.uiRatesKrakenCsvImport, args: args)
            try BackendSettingsViewModel.requireSuccess(envelope)
            let totals = envelope.data?.objectValue?["totals"]?.objectValue ?? [:]
            resultMessage = "\(totals.int("samples") ?? 0)"
        }
    }

    public func rebuild(source: String, pair: String?, days: Int, path: String?, reprice: Bool) async {
        await perform {
            var args: [String: JSONValue] = [
                "source": .string(source), "days": .integer(Int64(days)),
                "reprice_transactions": .bool(reprice),
            ]
            if let pair, !pair.isEmpty { args["pair"] = .string(pair) }
            if let path, !path.isEmpty { args["path"] = .string(path) }
            let stream = try await daemon.stream(.uiRatesRebuild, args: args)
            for try await record in stream {
                if let error = record.error { throw error }
                if record.kind == DaemonKind.uiRatesRebuild.rawValue { resultMessage = "rates_rebuilt" }
            }
        }
    }

    private func perform(_ body: () async throws -> Void) async {
        isWorking = true; defer { isWorking = false }
        do { try await body(); errorMessage = nil }
        catch { errorMessage = BackendSettingsViewModel.message(error) }
    }
}

public struct SyncTransportRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let kind: String
    public let label: String
    public let lastPushAt: String?
    public let lastPullAt: String?
    fileprivate init?(_ row: [String: JSONValue]) {
        guard let id = row.string("id"), !id.isEmpty else { return nil }
        self.id = id; kind = row.string("kind") ?? ""; label = row.string("label") ?? id
        lastPushAt = row.string("last_push_at"); lastPullAt = row.string("last_pull_at")
    }
}

public struct SyncMemberRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let role: String
    public let revoked: Bool
    fileprivate init?(_ row: [String: JSONValue]) {
        guard let id = row.string("id"), !id.isEmpty else { return nil }
        self.id = id; name = row.string("display_name", "member_name") ?? id
        role = row.string("role") ?? ""; revoked = row.string("revoked_at") != nil
    }
}

public struct SyncDeviceRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let memberName: String
    public let isLocal: Bool
    fileprivate init?(_ row: [String: JSONValue]) {
        guard let id = row.string("id"), !id.isEmpty else { return nil }
        self.id = id; label = row.string("label") ?? id
        memberName = row.string("member_name") ?? ""; isLocal = (row.int("local_device") ?? 0) == 1
    }
}

public struct SyncConflictRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let table: String
    public let field: String
    public let localValue: JSONValue?
    public let remoteValue: JSONValue?
    fileprivate init?(_ row: [String: JSONValue]) {
        guard let id = row.string("id", "conflict_id"), !id.isEmpty else { return nil }
        self.id = id; table = row.string("table", "table_name") ?? ""; field = row.string("field", "field_name") ?? ""
        localValue = row["local_value"]; remoteValue = row["remote_value"]
    }
}

@MainActor
@Observable
public final class ReplicationSettingsViewModel {
    public private(set) var configured = false
    public private(set) var enabled = false
    public private(set) var transports: [SyncTransportRow] = []
    public private(set) var members: [SyncMemberRow] = []
    public private(set) var devices: [SyncDeviceRow] = []
    public private(set) var conflicts: [SyncConflictRow] = []
    public private(set) var isWorking = false
    public private(set) var resultMessage: String?
    public private(set) var errorMessage: String?
    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func load() async {
        await perform(reload: false) {
            let status = try await daemon.invoke(.uiSyncStatus, args: nil)
            try BackendSettingsViewModel.requireSuccess(status)
            let data = status.data?.objectValue ?? [:]
            configured = data.bool("configured") ?? false; enabled = data.bool("enabled") ?? false
            transports = data.objects("transports").compactMap(SyncTransportRow.init)
            members = data.objects("members_list", "members").compactMap(SyncMemberRow.init)
            devices = data.objects("devices_list", "devices").compactMap(SyncDeviceRow.init)
            conflicts = data.objects("conflicts").compactMap(SyncConflictRow.init)
        }
    }

    public func setEnabled(_ shouldEnable: Bool, displayName: String = "Owner", deviceLabel: String = Host.current().localizedName ?? "Mac") async {
        await perform {
            let kind: DaemonKind = shouldEnable ? .uiSyncEnable : .uiSyncDisable
            let args: [String: JSONValue] = shouldEnable
                ? ["display_name": .string(displayName), "device_label": .string(deviceLabel)] : [:]
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(kind, args: args))
            resultMessage = shouldEnable ? "sync_enabled" : "sync_disabled"
        }
    }

    public func configureTransport(id: String?, kind: String, label: String, config: [String: JSONValue]) async {
        await perform {
            var args: [String: JSONValue] = ["kind": .string(kind), "label": .string(label), "config": .object(config)]
            if let id { args["id"] = .string(id) }
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.uiSyncTransportsConfigure, args: args))
            resultMessage = "transport_saved"
        }
    }

    public func deleteTransport(_ row: SyncTransportRow) async {
        await perform {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                .uiSyncTransportsDelete, args: ["transport": .string(row.id)]
            ))
        }
    }

    public func push(transport: String?) async { await transfer(.uiSyncPush, transport: transport) }
    public func pull(transport: String?) async { await transfer(.uiSyncPull, transport: transport) }
    private func transfer(_ kind: DaemonKind, transport: String?) async {
        await perform {
            var args: [String: JSONValue] = [:]; if let transport { args["transport"] = .string(transport) }
            let records = try await daemon.stream(kind, args: args)
            for try await record in records { if let error = record.error { throw error } }
            resultMessage = kind == .uiSyncPush ? "sync_pushed" : "sync_pulled"
        }
    }

    public func joinRequest(displayName: String, deviceLabel: String) async {
        await perform(reload: false) {
            let envelope = try await daemon.invoke(.uiSyncJoinRequest, args: [
                "display_name": .string(displayName), "device_label": .string(deviceLabel),
            ])
            try BackendSettingsViewModel.requireSuccess(envelope)
            resultMessage = envelope.data?.objectValue?.string("request", "join_request", "request_id")
        }
    }

    public func createInvite(memberID: String, role: String) async {
        await perform(reload: false) {
            let envelope = try await daemon.invoke(.uiSyncInvite, args: [
                "member": .string(memberID), "role": .string(role),
            ])
            try BackendSettingsViewModel.requireSuccess(envelope)
            resultMessage = envelope.data?.objectValue?.string("invitation")
        }
    }

    public func join(invitation: String) async {
        await perform {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                .uiSyncJoin, args: ["invitation": .string(invitation)]
            ))
        }
    }

    public func revokeMember(_ row: SyncMemberRow) async {
        await perform {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                .uiSyncMembersRevoke, args: ["member": .string(row.id)]
            ))
        }
    }

    public func revokeDevice(_ row: SyncDeviceRow) async {
        await perform {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                .uiSyncDevicesRevoke, args: ["device": .string(row.id)]
            ))
        }
    }

    public func resolveConflict(_ row: SyncConflictRow, choice: String) async {
        await perform {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                .uiSyncConflictsResolve, args: ["conflict": .string(row.id), "choice": .string(choice)]
            ))
        }
    }

    private func perform(reload shouldReload: Bool = true, _ body: () async throws -> Void) async {
        isWorking = true; defer { isWorking = false }
        do { try await body(); errorMessage = nil; if shouldReload { await load() } }
        catch { errorMessage = BackendSettingsViewModel.message(error) }
    }
}

@MainActor
@Observable
public final class DestructiveSettingsViewModel {
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?
    public private(set) var resultMessage: String?
    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func resetBook(name: String, clearSharedRates: Bool, passphrase: String?) async {
        await destructive(.uiProfilesResetData, args: [
            "confirm": "RESET", "confirm_profile": .string(name),
            "clear_shared_rates": .bool(clearSharedRates),
            "auth_response": Self.auth(passphrase, ackKey: "plaintext_delete_ack", ack: "DELETE LOCAL DATA"),
        ])
    }

    public func deleteWorkspace(name: String, passphrase: String?) async {
        await destructive(.uiWorkspaceDelete, args: [
            "confirm": "DELETE", "confirm_workspace": .string(name),
            "auth_response": Self.auth(passphrase, ackKey: "plaintext_delete_ack", ack: "DELETE LOCAL DATA"),
        ])
    }

    private func destructive(_ kind: DaemonKind, args: [String: JSONValue]) async {
        isWorking = true; defer { isWorking = false }
        do {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(kind, args: args))
            resultMessage = kind == .uiWorkspaceDelete ? "workspace_deleted" : "book_reset"
            errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    private static func auth(_ passphrase: String?, ackKey: String, ack: String) -> JSONValue {
        if let passphrase, !passphrase.isEmpty { return ["passphrase_secret": .string(passphrase)] }
        return .object([ackKey: .string(ack)])
    }
}

public enum OnboardingFlowMode: String, Equatable, Sendable { case start, setup }
public enum OnboardingStep: Int, CaseIterable, Equatable, Sendable {
    case essentials, sync, ai, security, review
}
public enum OnboardingBackendMode: String, CaseIterable, Equatable, Sendable {
    case recommended, custom, skip
}
public enum OnboardingAIMode: String, CaseIterable, Equatable, Sendable {
    case local, remote, disabled
}
public enum OnboardingDatabaseMode: String, CaseIterable, Equatable, Sendable {
    case encrypted, plaintext
}

/// The first-run coordinator mirrors Tauri's start chooser and five gated
/// steps. All workflow state and payload validation stays Foundation-only so
/// the SwiftUI view is a thin native form over a testable contract.
@MainActor
@Observable
public final class OnboardingParityViewModel {
    public private(set) var flowMode = OnboardingFlowMode.start
    public private(set) var step = OnboardingStep.essentials
    public var workspace = "My Books"
    public var profile = "Private"
    public var taxCountry = "at"
    public var fiatCurrency = "EUR"
    public var longTermDays = 365
    public var gainsAlgorithm = "MOVING_AVERAGE_AT"
    public var backendMode = OnboardingBackendMode.recommended
    public var skipBackendsAcknowledged = false
    public var backend = BackendDraft()
    public var aiMode = OnboardingAIMode.local
    public var provider = AIProviderDraft()
    public var databaseMode = OnboardingDatabaseMode.encrypted
    public var databasePassphrase = ""
    public var databasePassphraseConfirmation = ""
    public var recoveryAcknowledged = false
    public var plaintextAcknowledged = false
    public var migrateCredentials = true
    public var enableTouchID = true
    public private(set) var publicDefaultBackends: [SettingsBackendRow] = []
    public private(set) var importedProject: ImportedProjectSelection?
    public var importedProjectPassphrase = ""
    public private(set) var awaitingImportedProjectUnlock = false
    public private(set) var regtestAvailable = false
    public private(set) var touchIDStatus = TouchIDPassphraseStatus(
        available: false,
        configured: false
    )
    public private(set) var touchIDWarning: String?
    public private(set) var isWorking = false
    public private(set) var didComplete = false
    public private(set) var errorMessage: String?
    private let daemon: any DaemonClient
    private let touchIDManager: any TouchIDPassphraseManaging
    private let touchIDAccount: String
    private let defaults: UserDefaults
    public init(
        daemon: any DaemonClient,
        touchIDManager: any TouchIDPassphraseManaging = UnavailableTouchIDPassphraseManager(),
        touchIDAccount: String = "default",
        defaults: UserDefaults = .standard
    ) {
        self.daemon = daemon
        self.touchIDManager = touchIDManager
        self.touchIDAccount = touchIDAccount
        self.defaults = defaults
        provider.name = "ollama"
        provider.displayName = "Ollama"
        provider.baseURL = "http://localhost:11434/v1"
        provider.kind = "local"
        backend.name = "fulcrum"
        backend.kind = "electrum"
        backend.url = "ssl://index.bitcoin-austria.at:50002"
        backend.chain = "bitcoin"
        backend.network = "main"
        regtestAvailable = Self.regtestSelection() != nil
    }

    public var canContinue: Bool { stepComplete(step) && !isWorking }

    public func beginSetup() {
        flowMode = .setup
        step = .essentials
        errorMessage = nil
    }

    /// Express path keeps Tauri's recommended accounting, sync, and local-AI
    /// defaults, but still stops at Security so encryption is never skipped.
    public func beginQuickStart() {
        resetRecommendedDefaults()
        flowMode = .setup
        step = .security
        errorMessage = nil
    }

    public func returnToStart() {
        flowMode = .start
        errorMessage = nil
    }

    public func goBack() {
        guard let previous = OnboardingStep(rawValue: step.rawValue - 1) else {
            returnToStart(); return
        }
        step = previous
    }

    public func goNext() {
        guard canContinue, let next = OnboardingStep(rawValue: step.rawValue + 1) else { return }
        step = next
    }

    public func jump(to target: OnboardingStep) { step = target }

    public func stepComplete(_ candidate: OnboardingStep) -> Bool {
        switch candidate {
        case .essentials:
            return !workspace.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && !profile.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && (taxCountry == "at" || longTermDays > 0)
        case .sync:
            switch backendMode {
            case .recommended: return true
            case .skip: return skipBackendsAcknowledged
            case .custom:
                return !backend.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    && Self.validBackendURL(backend.url, kind: backend.kind)
            }
        case .ai:
            switch aiMode {
            case .disabled: return true
            case .local:
                return Self.validAIURL(provider.baseURL, loopbackOnly: true)
            case .remote:
                return !provider.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    && provider.acknowledged
                    && Self.validAIURL(provider.baseURL, loopbackOnly: false)
            }
        case .security:
            if databaseMode == .plaintext { return plaintextAcknowledged }
            return databasePassphrase.count >= 12
                && databasePassphrase == databasePassphraseConfirmation
                && recoveryAcknowledged
        case .review:
            return [.essentials, .sync, .ai, .security].allSatisfy(stepComplete)
        }
    }

    public func loadCatalog() async {
        do {
            let envelope = try await daemon.invoke(.uiBackendsPublicDefaults, args: nil)
            try BackendSettingsViewModel.requireSuccess(envelope)
            publicDefaultBackends = (envelope.data?.objectValue ?? [:]).objects("backends").compactMap(SettingsBackendRow.init)
            errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
        touchIDStatus = await touchIDManager.status(account: touchIDAccount)
    }

    public func importProject(_ pickedURL: URL) async {
        isWorking = true
        defer { isWorking = false }
        do {
            let selection = try ImportedProjectInspector.inspect(pickedURL)
            guard let switching = daemon as? any DaemonDataRootSwitching else {
                throw DaemonClientError.transport("project_import_unavailable")
            }
            try await switching.activateDataRoot(selection.dataRoot)
            importedProject = selection
            importedProjectPassphrase = ""
            awaitingImportedProjectUnlock = selection.encrypted
            if selection.encrypted {
                didComplete = false
                errorMessage = nil
            } else {
                try await unlockActivatedProject(passphrase: nil)
            }
        } catch {
            didComplete = false
            errorMessage = BackendSettingsViewModel.message(error)
        }
    }

    public func unlockImportedProject() async {
        guard importedProject != nil else { return }
        isWorking = true
        defer { isWorking = false }
        do {
            try await unlockActivatedProject(passphrase: importedProjectPassphrase)
        } catch {
            didComplete = false
            awaitingImportedProjectUnlock = true
            errorMessage = BackendSettingsViewModel.message(error)
        }
    }

    public func openRegtestDemo() async {
        guard let selection = Self.regtestSelection() else {
            errorMessage = "regtest_demo_unavailable"
            return
        }
        await importProject(URL(fileURLWithPath: selection.stateRoot, isDirectory: true))
    }

    public func finish() async {
        isWorking = true; defer { isWorking = false }
        do {
            guard stepComplete(.review) else {
                throw DaemonClientError.transport("onboarding_steps_incomplete")
            }
            if databaseMode == .encrypted {
                let encrypted = try await daemon.invoke(.uiSecretsInit, args: [
                    "auth_response": ["passphrase_secret": .string(databasePassphrase)],
                    "migrate_credentials": .bool(migrateCredentials),
                ])
                try BackendSettingsViewModel.requireSuccess(encrypted)
                if enableTouchID && touchIDStatus.available {
                    do {
                        try await touchIDManager.store(
                            passphrase: databasePassphrase,
                            account: touchIDAccount
                        )
                        touchIDWarning = nil
                    } catch {
                        // Encryption is already committed. Match the desktop
                        // flow by keeping Touch ID enrollment best-effort and
                        // allowing setup to finish; Settings can retry later.
                        touchIDWarning = String(describing: error)
                        enableTouchID = false
                    }
                    touchIDStatus = await touchIDManager.status(account: touchIDAccount)
                }
            }
            let allowedAlgorithms = taxCountry == "at"
                ? Set(["MOVING_AVERAGE_AT", "FIFO", "LIFO", "HIFO", "LOFO"])
                : Set(["MOVING_AVERAGE", "FIFO", "LIFO", "HIFO", "LOFO"])
            let resolvedAlgorithm = allowedAlgorithms.contains(gainsAlgorithm)
                ? gainsAlgorithm
                : (taxCountry == "at" ? "MOVING_AVERAGE_AT" : "FIFO")
            var args: [String: JSONValue] = [
                "workspace_label": .string(workspace.trimmingCharacters(in: .whitespacesAndNewlines)),
                "profile_label": .string(profile.trimmingCharacters(in: .whitespacesAndNewlines)),
                "tax_country": .string(taxCountry), "fiat_currency": .string(fiatCurrency),
                "tax_long_term_days": .integer(Int64(taxCountry == "at" ? 0 : longTermDays)),
                "gains_algorithm": .string(resolvedAlgorithm),
            ]
            if backendMode == .custom {
                let liquid = backend.kind.lowercased() == "liquid-esplora"
                args["backend"] = .object([
                    "name": .string(backend.name.trimmingCharacters(in: .whitespacesAndNewlines)),
                    "kind": .string(backend.kind),
                    "url": .string(backend.url.trimmingCharacters(in: .whitespacesAndNewlines)),
                    "chain": .string(liquid ? "liquid" : "bitcoin"),
                    "network": .string(liquid ? "liquidv1" : "main"),
                ])
            }
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.uiOnboardingComplete, args: args))
            if aiMode != .disabled {
                provider.kind = aiMode == .local ? "local" : "remote"
                let providerName = provider.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? (aiMode == .local ? "ollama" : "remote")
                    : provider.name.trimmingCharacters(in: .whitespacesAndNewlines)
                let saved = try await daemon.invoke(.aiProvidersCreate, args: [
                    "name": .string(providerName),
                    "display_name": .string(provider.displayName.isEmpty ? providerName : provider.displayName),
                    "base_url": .string(provider.baseURL.trimmingCharacters(in: .whitespacesAndNewlines)),
                    "kind": .string(provider.kind),
                    "acknowledged": .bool(provider.acknowledged),
                ])
                if saved.error?.code == "conflict" {
                    try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.aiProvidersUpdate, args: [
                        "name": .string(providerName),
                        "display_name": .string(provider.displayName.isEmpty ? providerName : provider.displayName),
                        "base_url": .string(provider.baseURL.trimmingCharacters(in: .whitespacesAndNewlines)),
                        "kind": .string(provider.kind),
                        "acknowledged": .bool(provider.acknowledged),
                    ]))
                } else { try BackendSettingsViewModel.requireSuccess(saved) }
                if !provider.apiKey.isEmpty {
                    try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.aiProvidersSetApiKey, args: [
                        "name": .string(providerName), "api_key": .string(provider.apiKey),
                    ]))
                }
                try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                    .aiProvidersSetDefault, args: ["name": .string(providerName)]
                ))
            }
            didComplete = true; errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    private func resetRecommendedDefaults() {
        workspace = "My Books"
        profile = "Private"
        taxCountry = "at"
        fiatCurrency = "EUR"
        longTermDays = 365
        gainsAlgorithm = "MOVING_AVERAGE_AT"
        backendMode = .recommended
        skipBackendsAcknowledged = false
        aiMode = .local
        provider.name = "ollama"
        provider.displayName = "Ollama"
        provider.baseURL = "http://localhost:11434/v1"
        provider.kind = "local"
        provider.apiKey = ""
        provider.defaultModel = ""
        provider.acknowledged = false
        databaseMode = .encrypted
        databasePassphrase = ""
        databasePassphraseConfirmation = ""
        recoveryAcknowledged = false
        plaintextAcknowledged = false
        migrateCredentials = true
        enableTouchID = true
    }

    private func unlockActivatedProject(passphrase: String?) async throws {
        var args: [String: JSONValue] = ["require_existing_project": .bool(true)]
        if let passphrase, !passphrase.isEmpty {
            args["auth_response"] = ["passphrase_secret": .string(passphrase)]
        }
        let envelope = try await daemon.invoke(.daemonUnlock, args: args)
        if envelope.kind == "auth_required" {
            awaitingImportedProjectUnlock = true
            throw DaemonClientError.transport("import_project_passphrase_required")
        }
        try BackendSettingsViewModel.requireSuccess(envelope)
        guard let selection = importedProject else { return }
        defaults.set(selection.stateRoot, forKey: "projects.imported.stateRoot")
        defaults.set(selection.dataRoot, forKey: "projects.imported.dataRoot")
        defaults.set(selection.database, forKey: "projects.imported.database")
        defaults.set(selection.encrypted, forKey: "projects.imported.encrypted")
        awaitingImportedProjectUnlock = false
        importedProjectPassphrase = ""
        didComplete = true
        errorMessage = nil
    }

    private static func validBackendURL(_ raw: String, kind: String) -> Bool {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(), components.host != nil else { return false }
        if kind.lowercased() == "electrum" { return ["ssl", "tcp"].contains(scheme) && components.port != nil }
        return ["http", "https"].contains(scheme)
    }

    private static func validAIURL(_ raw: String, loopbackOnly: Bool) -> Bool {
        guard let components = URLComponents(string: raw.trimmingCharacters(in: .whitespacesAndNewlines)),
              let scheme = components.scheme?.lowercased(), ["http", "https"].contains(scheme),
              let host = components.host?.lowercased(), !host.isEmpty else { return false }
        guard loopbackOnly else { return true }
        return host == "localhost" || host == "::1" || host.hasPrefix("127.")
    }

    private static func regtestSelection() -> ImportedProjectSelection? {
        let environment = ProcessInfo.processInfo.environment
        let root: URL
        if let configured = environment["KASSIBER_REGTEST_DEMO_HOME"], !configured.isEmpty {
            root = URL(fileURLWithPath: configured, isDirectory: true)
        } else {
            root = FileManager.default.homeDirectoryForCurrentUser
                .appending(path: ".kassiber/regtest-demo", directoryHint: .isDirectory)
        }
        return try? ImportedProjectInspector.inspect(root)
    }
}

@MainActor
@Observable
public final class ChatHistorySettingsViewModel {
    public var mode = "auto"
    public private(set) var effectiveEnabled = false
    public private(set) var databaseEncrypted = false
    public private(set) var sessionCount = 0
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?
    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func load() async {
        isWorking = true; defer { isWorking = false }
        do {
            async let configCall = daemon.invoke(.uiChatHistoryConfigure, args: [:])
            async let sessionsCall = daemon.invoke(.uiChatSessionsList, args: ["limit": .integer(200)])
            let (config, sessions) = try await (configCall, sessionsCall)
            try BackendSettingsViewModel.requireSuccess(config)
            try BackendSettingsViewModel.requireSuccess(sessions)
            parse(config.data)
            sessionCount = (sessions.data?.objectValue?["sessions"]?.arrayValue ?? []).count
            errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    public func configure(_ requestedMode: String) async {
        isWorking = true; defer { isWorking = false }
        do {
            let envelope = try await daemon.invoke(.uiChatHistoryConfigure, args: ["history": .string(requestedMode)])
            try BackendSettingsViewModel.requireSuccess(envelope)
            parse(envelope.data); errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    public func clear() async {
        isWorking = true; defer { isWorking = false }
        do {
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.uiChatSessionsClear, args: [:]))
            sessionCount = 0; errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    private func parse(_ data: JSONValue?) {
        let row = data?.objectValue ?? [:]
        mode = row.string("history") ?? "auto"
        effectiveEnabled = row.bool("history_enabled") ?? false
        databaseEncrypted = row.bool("database_encrypted") ?? false
    }
}

public struct PrivacyHygieneWalletRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let state: String
    public let riskLevel: String
    public let transactionCount: Int
    public let riskCount: Int
    public let unknownCount: Int
    public let reusedAddresses: Int
    fileprivate init?(_ row: [String: JSONValue]) {
        guard let id = row.string("id", "label"), !id.isEmpty else { return nil }
        self.id = id; label = row.string("label") ?? id; state = row.string("state") ?? "unknown"
        riskLevel = row.string("risk_level") ?? "none"; transactionCount = Int(row.int("transaction_count") ?? 0)
        riskCount = Int(row.int("risk_count") ?? 0); unknownCount = Int(row.int("unknown_count") ?? 0)
        reusedAddresses = Int(row["address"]?.objectValue?.int("reused_address_count") ?? 0)
    }
}

@MainActor
@Observable
public final class PrivacyHygieneSettingsViewModel {
    public private(set) var state = ""
    public private(set) var riskLevel = ""
    public private(set) var riskCount = 0
    public private(set) var unknownCount = 0
    public private(set) var scoredTransactions = 0
    public private(set) var totalTransactions = 0
    public private(set) var wallets: [PrivacyHygieneWalletRow] = []
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?
    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }
    public func load() async {
        isLoading = true; defer { isLoading = false }
        do {
            let envelope = try await daemon.invoke(.uiPrivacyHygieneSnapshot, args: nil)
            try BackendSettingsViewModel.requireSuccess(envelope)
            let root = envelope.data?.objectValue ?? [:]
            let summary = root["summary"]?.objectValue ?? [:]
            let coverage = root["coverage"]?.objectValue ?? [:]
            state = summary.string("state") ?? ""; riskLevel = summary.string("risk_level") ?? ""
            riskCount = Int(summary.int("risk_count") ?? 0); unknownCount = Int(summary.int("unknown_count") ?? 0)
            scoredTransactions = Int(coverage.int("transaction_scored") ?? 0)
            totalTransactions = Int(coverage.int("transaction_total") ?? 0)
            wallets = root.objects("wallets").compactMap(PrivacyHygieneWalletRow.init)
            errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }
}
