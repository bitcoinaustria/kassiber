import Foundation
import Observation
import KassiberDaemonKit

public enum ConnectionCatalogCategory: String, CaseIterable, Identifiable, Hashable, Sendable {
    case wallets
    case nodes
    case lightning
    case merchant
    case exchanges
    case files

    public var id: String { rawValue }
}

public enum ConnectionCatalogStatus: String, Equatable, Sendable {
    case ready
    case planned
}

public enum ConnectionCatalogSetupKind: String, Equatable, Sendable {
    case descriptor
    case addressList = "address-list"
    case silentPayment = "silent-payment"
    case fileWallet = "file-wallet"
    case fileEnrichment = "file-enrichment"
    case samourai
    case btcpay
    case bullBitcoinWallet = "bullbitcoin-wallet"
    case bip329
    case backendSettings = "backend-settings"
}

public enum ConnectionCatalogRoute: Equatable, Sendable {
    case descriptor
    case addressList
    case silentPayment
    case liquidDescriptor
    case backend(String)
    case btcpay
    case bullBitcoinWallet
    case samourai
    case bip329
    case fileImport(String)
    case planned
}

public struct ConnectionCatalogSource: Identifiable, Equatable, Sendable {
    public let id: String
    public let category: ConnectionCatalogCategory
    public let status: ConnectionCatalogStatus
    public let pathLabel: String
    public let formatLabel: String?
    public let setupKind: ConnectionCatalogSetupKind?
    public let walletKind: String?
    public let sourceFormat: String?
    public let daemonSupported: Bool

    public var titleLocalizationKey: String { "connections.catalog.\(id).title" }
    public var descriptionLocalizationKey: String { "connections.catalog.\(id).description" }
    public var pathLocalizationKey: String {
        "connections.catalog.path.\(pathLabel.lowercased().replacingOccurrences(of: " ", with: "_"))"
    }
    public var isEnabled: Bool { status == .ready && daemonSupported && route != .planned }

    public var route: ConnectionCatalogRoute {
        guard status == .ready, let setupKind else { return .planned }
        switch setupKind {
        case .descriptor: return id == "liquid-descriptor" ? .liquidDescriptor : .descriptor
        case .addressList: return .addressList
        case .silentPayment: return .silentPayment
        case .samourai: return .samourai
        case .btcpay: return .btcpay
        case .bullBitcoinWallet: return .bullBitcoinWallet
        case .bip329: return .bip329
        case .backendSettings:
            let backendKind = switch id {
            case "bitcoin-core": "bitcoinrpc"
            case "electrum": "electrum"
            case "core-ln": "coreln"
            case "lnd": "lnd"
            default: "esplora"
            }
            return .backend(backendKind)
        case .fileWallet, .fileEnrichment:
            guard let sourceFormat else { return .planned }
            return .fileImport(sourceFormat)
        }
    }

    fileprivate init(
        id: String,
        category: ConnectionCatalogCategory,
        status: ConnectionCatalogStatus,
        pathLabel: String,
        formatLabel: String? = nil,
        setupKind: ConnectionCatalogSetupKind? = nil,
        walletKind: String? = nil,
        sourceFormat: String? = nil,
        daemonSupported: Bool = true
    ) {
        self.id = id
        self.category = category
        self.status = status
        self.pathLabel = pathLabel
        self.formatLabel = formatLabel
        self.setupKind = setupKind
        self.walletKind = walletKind
        self.sourceFormat = sourceFormat
        self.daemonSupported = daemonSupported
    }

    fileprivate func resolving(walletKinds: Set<String>, sourceFormats: Set<String>) -> Self {
        let walletSupported = walletKind.map(walletKinds.contains) ?? true
        let formatSupported = sourceFormat.map(sourceFormats.contains) ?? true
        return Self(
            id: id, category: category, status: status, pathLabel: pathLabel,
            formatLabel: formatLabel, setupKind: setupKind, walletKind: walletKind,
            sourceFormat: sourceFormat, daemonSupported: walletSupported && formatSupported
        )
    }
}

public struct ConnectionSourceCatalog: Equatable, Sendable {
    public let walletKinds: [String]
    public let sourceFormats: [String]
    public let sources: [ConnectionCatalogSource]

    public init(walletKinds: [String], sourceFormats: [String], sources: [ConnectionCatalogSource] = []) {
        self.walletKinds = walletKinds
        self.sourceFormats = sourceFormats
        self.sources = sources
    }
}

private let frontendConnectionSources: [ConnectionCatalogSource] = [
    .init(id: "descriptor", category: .wallets, status: .ready, pathLabel: "watch_only_wallet", formatLabel: "descriptor/BSMS/xpub-family", setupKind: .descriptor, walletKind: "descriptor"),
    .init(id: "address-list", category: .wallets, status: .ready, pathLabel: "watch_only_wallet", formatLabel: "address list", setupKind: .addressList, walletKind: "address"),
    .init(id: "silent-payment", category: .wallets, status: .ready, pathLabel: "watch_only_wallet", formatLabel: "BIP352 / BIP392 sp()", setupKind: .silentPayment, walletKind: "silent-payment"),
    .init(id: "liquid-descriptor", category: .wallets, status: .ready, pathLabel: "watch_only_wallet", formatLabel: "Liquid descriptor/export", setupKind: .descriptor, walletKind: "descriptor"),
    .init(id: "samourai", category: .wallets, status: .ready, pathLabel: "watch_only_wallet_group", formatLabel: "Samourai/Whirlpool descriptors", setupKind: .samourai, walletKind: "samourai"),
    .init(id: "wasabi", category: .wallets, status: .ready, pathLabel: "wallet_export", formatLabel: "wasabi_bundle", setupKind: .fileWallet, walletKind: "wasabi", sourceFormat: "wasabi_bundle"),
    .init(id: "bitcoin-core", category: .nodes, status: .ready, pathLabel: "node_backend", formatLabel: "bitcoinrpc", setupKind: .backendSettings),
    .init(id: "electrum", category: .nodes, status: .ready, pathLabel: "index_backend", formatLabel: "electrum", setupKind: .backendSettings),
    .init(id: "esplora", category: .nodes, status: .ready, pathLabel: "index_backend", formatLabel: "esplora", setupKind: .backendSettings),
    .init(id: "sparrow", category: .wallets, status: .planned, pathLabel: "wallet_export", formatLabel: "descriptor/xpub"),
    .init(id: "specter", category: .wallets, status: .planned, pathLabel: "wallet_export", formatLabel: "descriptor"),
    .init(id: "bluewallet", category: .wallets, status: .planned, pathLabel: "wallet_export"),
    .init(id: "blockstream-green", category: .wallets, status: .planned, pathLabel: "wallet_export", formatLabel: "descriptor/xpub"),
    .init(id: "liana", category: .wallets, status: .planned, pathLabel: "wallet_export", formatLabel: "descriptor"),
    .init(id: "nunchuk", category: .wallets, status: .planned, pathLabel: "wallet_export"),
    .init(id: "bitbox", category: .wallets, status: .planned, pathLabel: "wallet_export"),
    .init(id: "trezor", category: .wallets, status: .planned, pathLabel: "wallet_export"),
    .init(id: "coldcard", category: .wallets, status: .planned, pathLabel: "wallet_export", formatLabel: "skeleton/descriptor"),
    .init(id: "ledger", category: .wallets, status: .ready, pathLabel: "csv_import", formatLabel: "ledgerlive_csv", setupKind: .fileWallet, walletKind: "ledgerlive", sourceFormat: "ledgerlive_csv"),
    .init(id: "foundation-passport", category: .wallets, status: .planned, pathLabel: "wallet_export"),
    .init(id: "core-ln", category: .lightning, status: .ready, pathLabel: "lightning_node", formatLabel: "coreln", setupKind: .backendSettings, walletKind: "coreln"),
    .init(id: "lnd", category: .lightning, status: .ready, pathLabel: "lightning_node", formatLabel: "LND REST", setupKind: .backendSettings, walletKind: "lnd"),
    .init(id: "zeus", category: .lightning, status: .planned, pathLabel: "lightning_wallet"),
    .init(id: "phoenix", category: .lightning, status: .ready, pathLabel: "csv_import", formatLabel: "phoenix_csv", setupKind: .fileWallet, walletKind: "phoenix", sourceFormat: "phoenix_csv"),
    .init(id: "bullbitcoin-wallet", category: .wallets, status: .ready, pathLabel: "wallet_csv_import", formatLabel: "bullbitcoin_wallet_csv", setupKind: .bullBitcoinWallet, walletKind: "bullbitcoin", sourceFormat: "bullbitcoin_wallet_csv"),
    .init(id: "btcpay", category: .merchant, status: .ready, pathLabel: "greenfield_api", formatLabel: "confirmed wallet history", setupKind: .btcpay),
    .init(id: "btcpay-csv", category: .merchant, status: .ready, pathLabel: "csv_import", formatLabel: "btcpay_csv", setupKind: .fileWallet, walletKind: "custom", sourceFormat: "btcpay_csv"),
    .init(id: "river", category: .exchanges, status: .ready, pathLabel: "csv_import", formatLabel: "river_csv", setupKind: .fileWallet, walletKind: "river", sourceFormat: "river_csv"),
    .init(id: "bullbitcoin", category: .exchanges, status: .ready, pathLabel: "csv_import", formatLabel: "bullbitcoin_csv", setupKind: .fileEnrichment, walletKind: "bullbitcoin", sourceFormat: "bullbitcoin_csv"),
    .init(id: "relai", category: .exchanges, status: .planned, pathLabel: "csv_import"),
    .init(id: "pocket-bitcoin", category: .exchanges, status: .ready, pathLabel: "csv_import", formatLabel: "pocketbitcoin_csv", setupKind: .fileEnrichment, walletKind: "pocketbitcoin", sourceFormat: "pocketbitcoin_csv"),
    .init(id: "swan-bitcoin", category: .exchanges, status: .planned, pathLabel: "csv_import"),
    .init(id: "strike", category: .exchanges, status: .ready, pathLabel: "custodial_platform", formatLabel: "strike_csv", setupKind: .fileWallet, walletKind: "strike", sourceFormat: "strike_csv"),
    .init(id: "21bitcoin", category: .exchanges, status: .ready, pathLabel: "csv_import", formatLabel: "21bitcoin_csv", setupKind: .fileWallet, walletKind: "21bitcoin", sourceFormat: "21bitcoin_csv"),
    .init(id: "coinfinity", category: .exchanges, status: .ready, pathLabel: "csv_import", formatLabel: "coinfinity_csv", setupKind: .fileEnrichment, walletKind: "coinfinity", sourceFormat: "coinfinity_csv"),
    .init(id: "bitpanda", category: .exchanges, status: .planned, pathLabel: "csv_api_import"),
    .init(id: "kraken", category: .exchanges, status: .planned, pathLabel: "api_import"),
    .init(id: "coinbase", category: .exchanges, status: .planned, pathLabel: "csv_api_import"),
    .init(id: "binance", category: .exchanges, status: .ready, pathLabel: "supplemental_csv", formatLabel: "binance_supplemental_csv", setupKind: .fileEnrichment, walletKind: "binance", sourceFormat: "binance_supplemental_csv"),
    .init(id: "generic-ledger", category: .files, status: .ready, pathLabel: "spreadsheet_import", formatLabel: "Excel (.xlsx) or CSV", setupKind: .fileWallet, walletKind: "custom", sourceFormat: "generic_ledger"),
    .init(id: "csv", category: .files, status: .ready, pathLabel: "csv_json_import", formatLabel: "generic csv/json", setupKind: .fileWallet, walletKind: "custom", sourceFormat: "csv"),
    .init(id: "bip329", category: .files, status: .ready, pathLabel: "label_import", formatLabel: "BIP329 JSONL", setupKind: .bip329),
]

public struct BTCPayStoreRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let currency: String
    fileprivate init?(_ row: [String: JSONValue]) {
        guard let id = row.string("id"), !id.isEmpty else { return nil }
        self.id = id; name = row.string("name") ?? id; currency = row.string("default_currency") ?? ""
    }
}

public struct BTCPayPaymentMethodRow: Identifiable, Equatable, Sendable {
    public let storeID: String
    public let paymentMethodID: String
    public let label: String
    public let enabled: Bool
    public let syncSupported: Bool
    public var id: String { "\(storeID):\(paymentMethodID)" }
    fileprivate init?(_ row: [String: JSONValue]) {
        guard let store = row.string("store_id"), let method = row.string("payment_method_id") else { return nil }
        storeID = store; paymentMethodID = method; label = row.string("label") ?? method
        enabled = row.bool("enabled") ?? true; syncSupported = row.bool("sync_supported") ?? false
    }
}

public struct BIP329PreviewSummary: Equatable, Sendable {
    public let records: Int
    public let exact: Int
    public let ambiguous: Int
    public let unmatched: Int
    public let conflicts: Int
}

public struct ConnectionOperationArtifact: Equatable, Sendable {
    public let sourcePath: String
    public let filename: String
    public let count: Int
}

@MainActor
@Observable
public final class ConnectionsParityViewModel {
    public private(set) var catalog = ConnectionSourceCatalog(
        walletKinds: [], sourceFormats: [],
        sources: frontendConnectionSources.map {
            $0.resolving(walletKinds: [], sourceFormats: [])
        }
    )
    public private(set) var backends: [SettingsBackendRow] = []
    public private(set) var safeBackendOptions: [SettingsBackendRow] = []
    public private(set) var publicDefaultBackends: [SettingsBackendRow] = []
    public private(set) var wallets: [WalletRow] = []
    public private(set) var priceEUR: Double?
    public private(set) var btcpayStores: [BTCPayStoreRow] = []
    public private(set) var btcpayPaymentMethods: [BTCPayPaymentMethodRow] = []
    public private(set) var bip329Preview: BIP329PreviewSummary?
    public private(set) var artifact: ConnectionOperationArtifact?
    public private(set) var isWorking = false
    public private(set) var resultMessage: String?
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient
    private var bip329PreviewedKey: String?
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func load() async {
        isWorking = true; defer { isWorking = false }
        do {
            async let sourcesCall = daemon.invoke(.uiConnectionsSources, args: nil)
            async let backendCall = daemon.invoke(.uiBackendsSettingsList, args: nil)
            async let optionsCall = daemon.invoke(.uiBackendsOptions, args: nil)
            async let defaultsCall = daemon.invoke(.uiBackendsPublicDefaults, args: nil)
            async let walletCall = daemon.invoke(.uiWalletsList, args: nil)
            async let overviewCall = daemon.invoke(.uiOverviewSnapshot, args: nil)
            let (sources, backendEnvelope, optionsEnvelope, defaultsEnvelope, walletEnvelope, overviewEnvelope) = try await (
                sourcesCall, backendCall, optionsCall, defaultsCall, walletCall, overviewCall
            )
            try BackendSettingsViewModel.requireSuccess(sources)
            try BackendSettingsViewModel.requireSuccess(backendEnvelope)
            try BackendSettingsViewModel.requireSuccess(optionsEnvelope)
            try BackendSettingsViewModel.requireSuccess(defaultsEnvelope)
            try BackendSettingsViewModel.requireSuccess(walletEnvelope)
            try BackendSettingsViewModel.requireSuccess(overviewEnvelope)
            let sourceData = sources.data?.objectValue ?? [:]
            let walletKinds = sourceData["wallet_kinds"]?.arrayValue?.compactMap { value in
                    value.stringValue ?? value.objectValue?.string("kind", "id", "name")
                } ?? []
            let sourceFormats = sourceData["source_formats"]?.arrayValue?.compactMap(\.stringValue) ?? []
            catalog = ConnectionSourceCatalog(
                walletKinds: walletKinds,
                sourceFormats: sourceFormats,
                sources: frontendConnectionSources.map {
                    $0.resolving(walletKinds: Set(walletKinds), sourceFormats: Set(sourceFormats))
                }
            )
            backends = (backendEnvelope.data?.objectValue ?? [:]).objects("backends").compactMap(SettingsBackendRow.init)
            safeBackendOptions = (optionsEnvelope.data?.objectValue ?? [:]).objects("backends").compactMap(SettingsBackendRow.init)
            publicDefaultBackends = (defaultsEnvelope.data?.objectValue ?? [:]).objects("backends").compactMap(SettingsBackendRow.init)
            var parsedWallets = WalletRow.parseList(walletEnvelope.data)
            let overviewData = overviewEnvelope.data?.objectValue ?? [:]
            priceEUR = overviewData.double("priceEur", "price_eur")
            let balances = Dictionary(uniqueKeysWithValues: overviewData
                .objects("connections").compactMap { row -> (String, Double)? in
                    guard let id = row.string("id", "label"), let balance = row.double("balance") else { return nil }
                    return (id, balance)
                })
            for index in parsedWallets.indices {
                parsedWallets[index].balanceBTC = balances[parsedWallets[index].id] ?? balances[parsedWallets[index].label]
            }
            wallets = parsedWallets
            errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    public func discoverBTCPay(savedBackend: String?, label: String?, serverURL: String?, apiKey: String?) async {
        await operation(reload: false) {
            let envelope = try await daemon.invoke(.uiConnectionsBtcpayDiscover, args: Self.btcpayInstanceArgs(
                savedBackend: savedBackend, label: label, serverURL: serverURL, apiKey: apiKey
            ))
            try BackendSettingsViewModel.requireSuccess(envelope)
            let data = envelope.data?.objectValue ?? [:]
            btcpayStores = data.objects("stores").compactMap(BTCPayStoreRow.init)
            btcpayPaymentMethods = data.objects("payment_methods").compactMap(BTCPayPaymentMethodRow.init)
            resultMessage = "btcpay_discovered"
        }
    }

    public func testBTCPay(
        savedBackend: String?, label: String?, serverURL: String?, apiKey: String?,
        storeID: String, paymentMethodID: String
    ) async {
        await operation(reload: false) {
            var args = Self.btcpayInstanceArgs(
                savedBackend: savedBackend, label: label, serverURL: serverURL, apiKey: apiKey
            )
            args["store_id"] = .string(storeID)
            args["payment_method_id"] = .string(paymentMethodID)
            let envelope = try await daemon.invoke(.uiConnectionsBtcpayTest, args: args)
            try BackendSettingsViewModel.requireSuccess(envelope)
            resultMessage = envelope.data?.objectValue?.bool("ok") == true ? "btcpay_ok" : "btcpay_failed"
        }
    }

    public func createBTCPay(
        label: String,
        savedBackend: String?,
        backendLabel: String?,
        serverURL: String?,
        apiKey: String?,
        storeID: String,
        paymentMethodIDs: [String],
        existingWalletRoutes: [String: String] = [:],
        syncProvenance: Bool = true
    ) async {
        await operation {
            var args = Self.btcpayInstanceArgs(
                savedBackend: savedBackend, label: backendLabel, serverURL: serverURL, apiKey: apiKey
            )
            args["label"] = .string(label)
            args["store_id"] = .string(storeID)
            args["payment_method_ids"] = .array(paymentMethodIDs.map(JSONValue.string))
            args["sync_provenance"] = .bool(syncProvenance)
            if !existingWalletRoutes.isEmpty {
                args["mode"] = .string("existing_wallets")
                args["routes"] = .array(paymentMethodIDs.compactMap { method in
                    guard let wallet = existingWalletRoutes[method], !wallet.isEmpty else { return nil }
                    return .object([
                        "store_id": .string(storeID), "payment_method_id": .string(method),
                        "action": .string("existing_wallet"), "wallet": .string(wallet),
                    ])
                })
            } else {
                args["mode"] = .string("wallet_sources")
            }
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.uiConnectionsBtcpayCreate, args: args))
            resultMessage = "btcpay_saved"
        }
    }

    public func createBullBitcoinWallet(
        label: String, sourceFile: String, networks: [String], existingRoutes: [(network: String, wallet: String)] = []
    ) async {
        await operation {
            let args: [String: JSONValue]
            if existingRoutes.isEmpty {
                args = [
                    "mode": "wallet_sources", "label": .string(label), "source_file": .string(sourceFile),
                    "networks": .array(networks.map(JSONValue.string)),
                ]
            } else {
                args = [
                    "mode": "existing_wallets", "label": .string(label), "source_file": .string(sourceFile),
                    "routes": .array(existingRoutes.map { .object([
                        "network": .string($0.network), "wallet": .string($0.wallet),
                    ]) }),
                ]
            }
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(
                .uiConnectionsBullbitcoinWalletCreate, args: args
            ))
            resultMessage = "bullbitcoin_saved"
        }
    }

    public func importSamourai(
        label: String, backend: String?, network: String, gapLimit: Int,
        deposit: String, badbank: String, premix: String, postmix: String, ricochet: String
    ) async {
        await operation {
            var sources: [String: JSONValue] = [:]
            for (name, value) in [
                ("deposit", deposit), ("badbank", badbank), ("premix", premix),
                ("postmix", postmix), ("ricochet", ricochet),
            ] where !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                sources[name] = .string(value.trimmingCharacters(in: .whitespacesAndNewlines))
            }
            var args: [String: JSONValue] = [
                "label": .string(label), "network": .string(network),
                "gap_limit": .integer(Int64(gapLimit)), "source_set": .object(sources),
            ]
            if let backend, !backend.isEmpty { args["backend"] = .string(backend) }
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.uiWalletsImportSamourai, args: args))
            resultMessage = "samourai_imported"
        }
    }

    public func previewBIP329(file: String) async {
        bip329Preview = nil
        bip329PreviewedKey = nil
        await operation(reload: false) {
            let envelope = try await daemon.invoke(.uiMetadataBip329Preview, args: ["file": .string(file)])
            try BackendSettingsViewModel.requireSuccess(envelope)
            let data = envelope.data?.objectValue ?? [:]
            let counts = data["counts"]?.objectValue ?? [:]
            bip329Preview = BIP329PreviewSummary(
                records: Int(data.int("records") ?? 0), exact: Int(counts.int("exact") ?? 0),
                ambiguous: Int(counts.int("ambiguous") ?? 0), unmatched: Int(counts.int("unmatched") ?? 0),
                conflicts: Int(counts.int("conflicts") ?? 0)
            )
            bip329PreviewedKey = try Self.fileFingerprint(file)
        }
    }

    public func canImportBIP329(file: String) -> Bool {
        guard bip329Preview != nil, !file.isEmpty,
              let current = try? Self.fileFingerprint(file) else { return false }
        return bip329PreviewedKey == current
    }

    public func importBIP329(file: String) async {
        await operation {
            guard canImportBIP329(file: file) else {
                throw DaemonClientError.transport("bip329_preview_required")
            }
            let envelope = try await daemon.invoke(.uiMetadataBip329Import, args: ["file": .string(file)])
            try BackendSettingsViewModel.requireSuccess(envelope)
            resultMessage = "\(envelope.data?.objectValue?.int("transaction_tags_added") ?? 0)"
        }
    }

    public func exportBIP329(mode: String, wallet: String?) async {
        await operation(reload: false) {
            var args: [String: JSONValue] = ["mode": .string(mode)]
            if let wallet, !wallet.isEmpty { args["wallet"] = .string(wallet) }
            let envelope = try await daemon.invoke(.uiMetadataBip329Export, args: args)
            try BackendSettingsViewModel.requireSuccess(envelope)
            let data = envelope.data?.objectValue ?? [:]
            artifact = ConnectionOperationArtifact(
                sourcePath: data.string("file") ?? "", filename: data.string("filename") ?? "labels.jsonl",
                count: Int(data.int("exported") ?? 0)
            )
        }
    }

    private func operation(reload shouldReload: Bool = true, _ body: () async throws -> Void) async {
        isWorking = true; defer { isWorking = false }
        do { try await body(); errorMessage = nil; if shouldReload { await load() } }
        catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    private static func btcpayInstanceArgs(
        savedBackend: String?, label: String?, serverURL: String?, apiKey: String?
    ) -> [String: JSONValue] {
        if let savedBackend, !savedBackend.isEmpty { return ["backend": .string(savedBackend)] }
        return [
            "backend_label": .string(label ?? "btcpay"), "server_url": .string(serverURL ?? ""),
            "api_key": .string(apiKey ?? ""),
        ]
    }

    private static func fileFingerprint(_ path: String) throws -> String {
        let url = URL(fileURLWithPath: path).standardizedFileURL
        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        let size = (attributes[.size] as? NSNumber)?.int64Value ?? -1
        let modified = (attributes[.modificationDate] as? Date)?.timeIntervalSince1970 ?? -1
        return "\(url.path):\(size):\(modified)"
    }
}

public enum ConnectionNodeDetailTab: String, CaseIterable, Identifiable, Sendable {
    case overview
    case channels
    case activity
    case profitability
    case accounting
    public var id: String { rawValue }
}

public struct ConnectionNodeCapabilities: Equatable, Sendable {
    public let nodeSnapshot: Bool
    public let routingProfitability: Bool
    public let channelBalances: Bool
    public let channelLifecycle: Bool
    public let forwardEvents: Bool
    public let invoiceActivity: Bool
    public let paymentActivity: Bool
    public let onchainBalance: Bool
}

public struct ConnectionNodeIdentity: Equatable, Sendable {
    public let id: String
    public let label: String
    public let kind: String
}

public struct ConnectionNodeSummary: Equatable, Sendable {
    public let alias: String
    public let pubkey: String
    public let network: String
    public let implementationVersion: String?
    public let peerCount: Int
    public let blockHeight: Int64?
    public let invoiceCount: Int
    public let paidInvoiceCount: Int
    public let expiredInvoiceCount: Int
    public let paymentCount: Int
    public let completedPaymentCount: Int
    public let failedPaymentCount: Int
    public let onchainBalanceSats: Int64
    public let totalCapacitySats: Int64
    public let syncedToChain: Bool
    public let activeChannels: Int
    public let localBalanceSats: Int64
    public let remoteBalanceSats: Int64
    public let forwardingFeesSats: Int64

    public var failedOrExpiredCount: Int { failedPaymentCount + expiredInvoiceCount }
}

public struct ConnectionNodeChannelRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let shortChannelID: String?
    public let fundingOutpoint: String?
    public let peer: String
    public let peerPubkey: String?
    public let state: String
    public let capacitySats: Int64
    public let localBalanceSats: Int64
    public let remoteBalanceSats: Int64
    public let isPrivate: Bool
    public let isInitiator: Bool
    public let baseFeeMsat: Int64?
    public let feeRatePPM: Int64?
    public let openedAt: Date?
    public let closedAt: Date?
    public let closeKind: String?
    public let forwardCount: Int?
    public let earnedSats: Int64?
    public let htlcCount: Int?
    public let lastActivityAt: Date?

    public var isClosed: Bool {
        ["closed", "force_closed"].contains(state) || closedAt != nil
    }
}

public struct ConnectionNodeForwardRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let occurredAt: Date?
    public let inPeer: String
    public let inShortChannelID: String?
    public let outPeer: String
    public let outShortChannelID: String?
    public let amountInMsat: Int64
    public let amountOutMsat: Int64
    public let feeMsat: Int64
    public let status: String
    public let failureReason: String?

    public var route: String { [inPeer, outPeer].filter { !$0.isEmpty }.joined(separator: " → ") }
    public var amountSats: Int64 { amountInMsat / 1000 }
    public var amountOutSats: Int64 { amountOutMsat / 1000 }
    public var feeSats: Int64 { feeMsat / 1000 }
}

public struct ConnectionNodeRoutingSummary: Equatable, Sendable {
    public let window: String
    public let revenueSats: Int64
    public let paymentCostSats: Int64
    public let rebalanceCostSats: Int64
    public let onchainCostSats: Int64
    public let netProfitSats: Int64
    public let forwardCount: Int
    public let paymentCount: Int
    public let rebalanceCount: Int
}

public struct ConnectionNodeProfitabilityChannel: Identifiable, Equatable, Sendable {
    public let id: String
    public let peer: String
    public let capacitySats: Int64
    public let earnedSats: Int64
    public let openCostSats: Int64
    public let coversOpenCost: Bool
}

public struct ConnectionNodeProfitabilityReport: Equatable, Sendable {
    public let connection: ConnectionNodeIdentity
    public let window: String
    public let summary: ConnectionNodeRoutingSummary
    public let channels: [ConnectionNodeProfitabilityChannel]
}

public struct ConnectionSyncProgress: Equatable, Sendable {
    public let phase: String
    public let processed: Int
    public let total: Int
    public let imported: Int
    public let skipped: Int
}

public enum ConnectionEditKind: String, Equatable, Sendable {
    case descriptor
    case btcpay
    case fileWallet
    case basic
}

public struct ConnectionProvenanceRoute: Identifiable, Equatable, Sendable {
    public let backend: String
    public let storeID: String
    public let paymentMethodID: String
    public var id: String { "\(backend)\u{0}\(storeID)\u{0}\(paymentMethodID)" }

    fileprivate init?(_ row: [String: JSONValue]) {
        guard let backend = row.string("backend"), !backend.isEmpty,
              let storeID = row.string("store_id"), !storeID.isEmpty,
              let paymentMethodID = row.string("payment_method_id"), !paymentMethodID.isEmpty else { return nil }
        self.backend = backend
        self.storeID = storeID
        self.paymentMethodID = paymentMethodID
    }

    fileprivate var json: JSONValue {
        .object([
            "backend": .string(backend), "store_id": .string(storeID),
            "payment_method_id": .string(paymentMethodID),
        ])
    }
}

public struct ConnectionEditMetadata: Equatable, Sendable {
    public let editKind: ConnectionEditKind
    public let walletKind: String
    public let chain: String
    public let network: String
    public let syncMode: String
    public let syncSource: String
    public let sourceFormat: String
    public let backendName: String
    public let backendSource: String
    public let backendKind: String
    public let gapLimit: Int?
    public let birthday: String
    public let scriptTypes: [String]
    public let provenanceRoutes: [ConnectionProvenanceRoute]
    public let hasDescriptor: Bool
    public let hasChangeDescriptor: Bool
    public let isNode: Bool

    public var canEditLiveBackend: Bool {
        ["backend_descriptor", "backend_addresses", "backend_silent_payment"].contains(syncMode)
            || ["descriptor", "xpub", "address", "samourai"].contains(walletKind)
    }

    public var canClearLiveBackend: Bool {
        canEditLiveBackend && chain.lowercased() != "liquid" && backendSource == "explicit"
    }
}

public struct ConnectionEditDraft: Equatable, Sendable {
    public var editKind: ConnectionEditKind = .basic
    public var label = ""
    public var archived = false
    /// Empty means keep the current backend; `clearBackend` explicitly opts
    /// back into the default Bitcoin backend.
    public var backend = ""
    public var clearBackend = false
    /// Replacement-only. Existing descriptor/xpub material is never loaded
    /// into this draft because revealing it requires a separate auth flow.
    public var walletMaterial = ""
    public var scriptTypes: Set<String> = []
    public var gapLimit = ""
    public var birthday = ""
    public var storeID = ""
    public var paymentMethodID = ""
    public var sourceFile = ""
    public var removedProvenanceRouteIDs: Set<String> = []
    public init() {}
}

@MainActor
@Observable
public final class ConnectionDetailParityViewModel {
    public private(set) var wallet: WalletRow?
    public private(set) var recentTransactions: [TransactionRow] = []
    public private(set) var utxos: [WalletUTXO] = []
    public private(set) var balanceHistory: [WalletHistoryPoint] = []
    public private(set) var inventoryMessage: String?
    public private(set) var utxoCount = 0
    public private(set) var recentTransactionCount = 0
    public private(set) var nodeSummary: ConnectionNodeSummary?
    public private(set) var nodeChannels: [ConnectionNodeChannelRow] = []
    public private(set) var nodeClosedChannels: [ConnectionNodeChannelRow] = []
    public private(set) var nodeForwards: [ConnectionNodeForwardRow] = []
    public private(set) var nodeRouting: ConnectionNodeRoutingSummary?
    public private(set) var nodeCapabilities: ConnectionNodeCapabilities?
    public private(set) var nodeConnection: ConnectionNodeIdentity?
    public private(set) var nodeProfitability: ConnectionNodeProfitabilityReport?
    public private(set) var nodeProfitabilityError: String?
    public private(set) var nodeWindowDays = 30
    public var selectedNodeTab: ConnectionNodeDetailTab = .overview
    public var showClosedNodeChannels = false
    public private(set) var selectedNodeChannelID: String?
    public private(set) var editMetadata: ConnectionEditMetadata?
    public private(set) var backendOptions: [SettingsBackendRow] = []
    public private(set) var databaseEncrypted = false
    public private(set) var syncProgress: ConnectionSyncProgress?
    public private(set) var revealedMaterial: String?
    public private(set) var isWorking = false
    public private(set) var isNodeWorking = false
    public private(set) var didDelete = false
    public private(set) var didUpdate = false
    public private(set) var errorMessage: String?
    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public static let nodeWindowOptions = [7, 30, 90, 365]

    public var visibleNodeChannels: [ConnectionNodeChannelRow] {
        showClosedNodeChannels ? nodeChannels + nodeClosedChannels : nodeChannels
    }

    public var selectedNodeChannel: ConnectionNodeChannelRow? {
        guard let selectedNodeChannelID else { return nil }
        return (nodeChannels + nodeClosedChannels).first { $0.id == selectedNodeChannelID }
    }

    public func selectNodeChannel(_ id: String?) { selectedNodeChannelID = id }

    public func load(walletRef: String) async {
        isWorking = true; defer { isWorking = false }
        do {
            async let walletCall = daemon.invoke(.uiWalletsList, args: nil)
            async let utxoCall = daemon.invoke(.uiWalletsUtxos, args: ["wallet": .string(walletRef)])
            async let transactionCall = daemon.invoke(.uiTransactionsList, args: [
                "wallet": .string(walletRef), "limit": .integer(12),
            ])
            async let historyCall = daemon.invoke(.uiReportsBalanceHistory, args: [
                "wallet": .string(walletRef), "interval": .string("month"), "limit": .integer(24),
            ])
            async let overviewCall = daemon.invoke(.uiOverviewSnapshot, args: nil)
            async let backendCall = daemon.invoke(.uiBackendsOptions, args: nil)
            async let statusCall = daemon.invoke(.status, args: nil)
            let (walletEnvelope, utxoEnvelope, transactionEnvelope, historyEnvelope, overviewEnvelope, backendEnvelope, statusEnvelope) = try await (
                walletCall, utxoCall, transactionCall, historyCall, overviewCall, backendCall, statusCall
            )
            try BackendSettingsViewModel.requireSuccess(walletEnvelope)
            try BackendSettingsViewModel.requireSuccess(utxoEnvelope)
            try BackendSettingsViewModel.requireSuccess(transactionEnvelope)
            try BackendSettingsViewModel.requireSuccess(historyEnvelope)
            try BackendSettingsViewModel.requireSuccess(overviewEnvelope)
            try BackendSettingsViewModel.requireSuccess(backendEnvelope)
            try BackendSettingsViewModel.requireSuccess(statusEnvelope)
            wallet = WalletRow.parseList(walletEnvelope.data).first { $0.id == walletRef || $0.label == walletRef }
            let walletRows = walletEnvelope.data?.objectValue?.objects("wallets") ?? []
            let rawWallet = walletRows.first { row in
                row.string("id") == walletRef || row.string("label") == walletRef
            } ?? [:]
            let overviewRows = overviewEnvelope.data?.objectValue?.objects("connections") ?? []
            let overviewRow = overviewRows.first { row in
                row.string("id") == walletRef || row.string("label") == walletRef
            } ?? [:]
            editMetadata = Self.parseEditMetadata(wallet: rawWallet, overview: overviewRow)
            backendOptions = (backendEnvelope.data?.objectValue ?? [:]).objects("backends").compactMap(SettingsBackendRow.init)
            databaseEncrypted = statusEnvelope.data?.objectValue?.bool("database_encrypted") ?? false
            recentTransactions = (transactionEnvelope.data?.objectValue?["txs"]?.arrayValue ?? [])
                .compactMap(TransactionRow.init)
            recentTransactionCount = recentTransactions.count
            parseInventory(utxoEnvelope.data)
            parseHistory(historyEnvelope.data)
            utxoCount = utxos.count
            errorMessage = nil
            if let wallet, ["lnd", "cln", "coreln", "core-lightning", "nwc"].contains(wallet.kind) {
                await loadNode(walletRef: walletRef)
            }
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    public func makeEditDraft(fallback: WalletRow) -> ConnectionEditDraft {
        var draft = ConnectionEditDraft()
        draft.label = fallback.label
        draft.archived = fallback.deprecated
        if let editMetadata {
            draft.editKind = editMetadata.editKind
            draft.scriptTypes = Set(editMetadata.scriptTypes)
            draft.gapLimit = editMetadata.gapLimit.map(String.init) ?? ""
            draft.birthday = editMetadata.birthday
        }
        return draft
    }

    public var liveBackendOptions: [SettingsBackendRow] {
        guard let metadata = editMetadata else { return [] }
        return backendOptions.filter { row in
            let kind = row.kind.lowercased()
            guard !["btcpay", "coreln", "lnd", "cln", "nwc"].contains(kind) else { return false }
            let optionChain: String
            if !row.chain.isEmpty { optionChain = row.chain.lowercased() }
            else if ["liquid", "liquid-blockstream"].contains(row.name.lowercased()) { optionChain = "liquid" }
            else { optionChain = "bitcoin" }
            return optionChain == (metadata.chain.isEmpty ? "bitcoin" : metadata.chain.lowercased())
        }
    }

    public var btcpayBackendOptions: [SettingsBackendRow] {
        backendOptions.filter { $0.kind.lowercased() == "btcpay" }
    }

    public func updateConfiguration(
        walletRef: String,
        original: WalletRow,
        draft: ConnectionEditDraft,
        passphrase: String?,
        plaintextConfirmed: Bool
    ) async {
        isWorking = true
        defer { isWorking = false }
        do {
            guard let metadata = editMetadata else {
                throw DaemonClientError.transport("connection_configuration_loading")
            }
            var args = try Self.updateArguments(original: original, metadata: metadata, draft: draft)
            guard !args.isEmpty else {
                throw DaemonClientError.transport("connection_change_required")
            }
            args["wallet"] = .string(walletRef)
            args["auth_response"] = try localAuthentication(
                passphrase: passphrase, plaintextConfirmed: plaintextConfirmed,
                key: "plaintext_change_ack", value: "CHANGE LOCAL DATA"
            )
            let envelope = try await daemon.invoke(.uiWalletsUpdate, args: args)
            try BackendSettingsViewModel.requireSuccess(envelope)
            if envelope.kind == "auth_required" {
                throw DaemonClientError.transport("database_passphrase_required_to_save")
            }
            didUpdate = true
            errorMessage = nil
        } catch {
            didUpdate = false
            errorMessage = BackendSettingsViewModel.message(error)
        }
    }

    public func loadNode(walletRef: String, windowDays: Int = 30) async {
        isNodeWorking = true
        defer { isNodeWorking = false }
        do {
            let envelope = try await daemon.invoke(.uiConnectionsNodeSnapshot, args: [
                "connection": .string(walletRef), "window_days": .integer(Int64(windowDays)),
            ])
            try BackendSettingsViewModel.requireSuccess(envelope)
            let data = envelope.data?.objectValue ?? [:]
            let summary = data["summary"]?.objectValue ?? data
            nodeWindowDays = max(1, min(365, windowDays))
            nodeChannels = data.objects("channels").compactMap(Self.parseNodeChannel)
                .sorted { $0.capacitySats > $1.capacitySats }
            nodeClosedChannels = data.objects("closedChannels").compactMap(Self.parseNodeChannel)
                .sorted { $0.capacitySats > $1.capacitySats }
            if nodeChannels.isEmpty, !nodeClosedChannels.isEmpty { showClosedNodeChannels = true }
            if let selectedNodeChannelID,
               !(nodeChannels + nodeClosedChannels).contains(where: { $0.id == selectedNodeChannelID }) {
                self.selectedNodeChannelID = nil
            }
            nodeForwards = data.objects("forwards").compactMap { row in
                guard let id = row.string("id"), !id.isEmpty else { return nil }
                return ConnectionNodeForwardRow(
                    id: id,
                    occurredAt: DaemonValueParser.date(row.string("occurredAt")),
                    inPeer: row.string("inPeerAlias") ?? "",
                    inShortChannelID: row.string("inShortChannelId"),
                    outPeer: row.string("outPeerAlias") ?? "",
                    outShortChannelID: row.string("outShortChannelId"),
                    amountInMsat: row.int("amountInMsat") ?? 0,
                    amountOutMsat: row.int("amountOutMsat") ?? 0,
                    feeMsat: row.int("feeMsat") ?? 0,
                    status: row.string("status") ?? "",
                    failureReason: row.string("failureReason")
                )
            }.sorted { ($0.occurredAt ?? .distantPast) > ($1.occurredAt ?? .distantPast) }
            if let routing = data["routing"]?.objectValue {
                nodeRouting = Self.parseNodeRouting(routing)
            } else { nodeRouting = nil }
            let capabilities = data["capabilities"]?.objectValue
                ?? data["connection"]?.objectValue?["lightningCapabilities"]?.objectValue
                ?? [:]
            nodeCapabilities = ConnectionNodeCapabilities(
                nodeSnapshot: capabilities.bool("nodeSnapshot") ?? false,
                routingProfitability: capabilities.bool("routingProfitability") ?? false,
                channelBalances: capabilities.bool("channelBalances") ?? false,
                channelLifecycle: capabilities.bool("channelLifecycle") ?? false,
                forwardEvents: capabilities.bool("forwardEvents") ?? false,
                invoiceActivity: capabilities.bool("invoiceActivity") ?? false,
                paymentActivity: capabilities.bool("paymentActivity") ?? false,
                onchainBalance: capabilities.bool("onchainBalance") ?? false
            )
            if let connection = data["connection"]?.objectValue {
                nodeConnection = ConnectionNodeIdentity(
                    id: connection.string("id") ?? walletRef,
                    label: connection.string("label") ?? wallet?.label ?? walletRef,
                    kind: connection.string("kind") ?? wallet?.kind ?? ""
                )
            } else {
                nodeConnection = ConnectionNodeIdentity(
                    id: walletRef, label: wallet?.label ?? walletRef, kind: wallet?.kind ?? ""
                )
            }
            nodeSummary = ConnectionNodeSummary(
                alias: data.string("alias") ?? data["node"]?.objectValue?.string("alias") ?? "",
                pubkey: data.string("pubkey") ?? "",
                network: data.string("network") ?? wallet?.network ?? "",
                implementationVersion: data.string("implementationVersion"),
                peerCount: Int(data.int("peerCount") ?? 0),
                blockHeight: data.int("blockHeight"),
                invoiceCount: Int(data.int("invoiceCount") ?? 0),
                paidInvoiceCount: Int(data.int("paidInvoiceCount") ?? 0),
                expiredInvoiceCount: Int(data.int("expiredInvoiceCount") ?? 0),
                paymentCount: Int(data.int("paymentCount") ?? 0),
                completedPaymentCount: Int(data.int("completedPaymentCount") ?? 0),
                failedPaymentCount: Int(data.int("failedPaymentCount") ?? 0),
                onchainBalanceSats: summary.int("onchainBalanceSat", "onchain_balance_sat") ?? 0,
                totalCapacitySats: summary.int("totalCapacitySat", "total_capacity_sat") ?? 0,
                syncedToChain: data.bool("synced_to_chain") ?? summary.bool("synced_to_chain") ?? false,
                activeChannels: nodeChannels.filter { $0.state == "active" }.count,
                localBalanceSats: summary.int("totalLocalBalanceSat", "local_balance_sat", "local_balance_sats") ?? 0,
                remoteBalanceSats: summary.int("totalRemoteBalanceSat", "remote_balance_sat", "remote_balance_sats") ?? 0,
                forwardingFeesSats: data["routing"]?.objectValue?.int("routingRevenueSat")
                    ?? summary.int("forwarding_fees_sat", "fees_earned_sat") ?? 0
            )
            errorMessage = nil
            if nodeCapabilities?.routingProfitability == true {
                await loadNodeProfitability(walletRef: walletRef, windowDays: nodeWindowDays)
            } else {
                nodeProfitability = nil
                nodeProfitabilityError = nil
            }
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    private func loadNodeProfitability(walletRef: String, windowDays: Int) async {
        do {
            let envelope = try await daemon.invoke(.uiReportsLightningProfitability, args: [
                "connection": .string(walletRef), "window_days": .integer(Int64(windowDays)),
            ])
            try BackendSettingsViewModel.requireSuccess(envelope)
            let data = envelope.data?.objectValue ?? [:]
            let connection = data["connection"]?.objectValue ?? [:]
            let identity = ConnectionNodeIdentity(
                id: connection.string("id") ?? nodeConnection?.id ?? walletRef,
                label: connection.string("label") ?? nodeConnection?.label ?? walletRef,
                kind: connection.string("kind") ?? nodeConnection?.kind ?? ""
            )
            let summary = data["summary"]?.objectValue ?? [:]
            let routing = Self.parseNodeRouting(
                summary, windowFallback: data.string("windowLabel") ?? nodeRouting?.window ?? ""
            )
            let channels = data.objects("channels").compactMap { row -> ConnectionNodeProfitabilityChannel? in
                guard let id = row.string("channelId"), !id.isEmpty else { return nil }
                return ConnectionNodeProfitabilityChannel(
                    id: id,
                    peer: row.string("peerAlias") ?? "",
                    capacitySats: row.int("capacitySat") ?? 0,
                    earnedSats: row.int("earnedRoutingSat") ?? 0,
                    openCostSats: row.int("openCostSat") ?? 0,
                    coversOpenCost: row.bool("coversOpenCost") ?? false
                )
            }
            nodeProfitability = ConnectionNodeProfitabilityReport(
                connection: identity, window: data.string("windowLabel") ?? routing.window,
                summary: routing, channels: channels
            )
            nodeProfitabilityError = nil
        } catch {
            nodeProfitability = nil
            nodeProfitabilityError = BackendSettingsViewModel.message(error)
        }
    }

    private static func parseNodeChannel(_ row: [String: JSONValue]) -> ConnectionNodeChannelRow? {
        guard let id = row.string("id", "shortChannelId", "fundingOutpoint"), !id.isEmpty else { return nil }
        return ConnectionNodeChannelRow(
            id: id,
            shortChannelID: row.string("shortChannelId"),
            fundingOutpoint: row.string("fundingOutpoint"),
            peer: row.string("peerAlias") ?? "",
            peerPubkey: row.string("peerPubkey"),
            state: row.string("state") ?? "",
            capacitySats: row.int("capacitySat") ?? 0,
            localBalanceSats: row.int("localBalanceSat") ?? 0,
            remoteBalanceSats: row.int("remoteBalanceSat") ?? 0,
            isPrivate: row.bool("isPrivate") ?? false,
            isInitiator: row.bool("isInitiator") ?? true,
            baseFeeMsat: row.int("baseFeeMsat"),
            feeRatePPM: row.int("feeRatePpm"),
            openedAt: DaemonValueParser.date(row.string("openedAt")),
            closedAt: DaemonValueParser.date(row.string("closedAt")),
            closeKind: row.string("closeKind"),
            forwardCount: row.int("forwardCount").map(Int.init),
            earnedSats: row.int("earnedRoutingSat"),
            htlcCount: row.int("htlcCount").map(Int.init),
            lastActivityAt: DaemonValueParser.date(row.string("lastActivityAt"))
        )
    }

    private static func parseNodeRouting(
        _ row: [String: JSONValue], windowFallback: String = ""
    ) -> ConnectionNodeRoutingSummary {
        ConnectionNodeRoutingSummary(
            window: row.string("windowLabel") ?? windowFallback,
            revenueSats: row.int("routingRevenueSat") ?? 0,
            paymentCostSats: row.int("paymentCostSat") ?? 0,
            rebalanceCostSats: row.int("rebalanceCostSat") ?? 0,
            onchainCostSats: row.int("onchainCostSat") ?? 0,
            netProfitSats: row.int("netProfitSat") ?? 0,
            forwardCount: Int(row.int("forwardCount") ?? 0),
            paymentCount: Int(row.int("paymentCount") ?? 0),
            rebalanceCount: Int(row.int("rebalanceCount") ?? 0)
        )
    }

    public func sync(walletRef: String, forceFull: Bool = false) async {
        isWorking = true; defer { isWorking = false }
        do {
            let records = try await daemon.stream(.uiWalletsSync, args: [
                "wallet": .string(walletRef), "force_full": .bool(forceFull),
            ])
            for try await record in records {
                if let error = record.error { throw error }
                if record.kind == "ui.wallets.sync.progress", let row = record.data?.objectValue {
                    syncProgress = ConnectionSyncProgress(
                        phase: row.string("phase") ?? "", processed: Int(row.int("processed") ?? 0),
                        total: Int(row.int("total") ?? 0), imported: Int(row.int("imported") ?? 0),
                        skipped: Int(row.int("skipped") ?? 0)
                    )
                }
            }
            errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    public func update(walletRef: String, label: String, archived: Bool, passphrase: String?) async {
        await operation {
            let envelope = try await daemon.invoke(.uiWalletsUpdate, args: [
                "wallet": .string(walletRef), "label": .string(label), "deprecated": .bool(archived),
                "auth_response": Self.auth(passphrase, key: "plaintext_change_ack", value: "CHANGE LOCAL DATA"),
            ])
            try BackendSettingsViewModel.requireSuccess(envelope)
        }
    }

    public func delete(
        walletRef: String, label: String, cascade: Bool, passphrase: String?,
        plaintextConfirmed: Bool = false
    ) async {
        await operation {
            let envelope = try await daemon.invoke(.uiWalletsDelete, args: [
                "wallet": .string(walletRef), "confirm": "DELETE", "confirm_wallet": .string(label),
                "cascade": .bool(cascade),
                "auth_response": try localAuthentication(
                    passphrase: passphrase, plaintextConfirmed: plaintextConfirmed,
                    key: "plaintext_delete_ack", value: "DELETE LOCAL DATA"
                ),
            ])
            try BackendSettingsViewModel.requireSuccess(envelope); didDelete = true
        }
    }

    public func revealDescriptor(
        walletRef: String, passphrase: String?, plaintextConfirmed: Bool = false
    ) async {
        clearRevealedMaterial()
        await operation {
            let envelope = try await daemon.invoke(.walletsRevealDescriptor, args: [
                "wallet": .string(walletRef),
                "auth_response": try localAuthentication(
                    passphrase: passphrase, plaintextConfirmed: plaintextConfirmed,
                    key: "plaintext_reveal_ack", value: "REVEAL LOCAL DATA"
                ),
            ])
            try BackendSettingsViewModel.requireSuccess(envelope)
            let data = envelope.data?.objectValue ?? [:]
            revealedMaterial = data.string("descriptor", "wallet_material", "xpub")
                ?? data["wallet"]?.objectValue?.string("descriptor", "wallet_material", "xpub")
        }
    }

    public func clearRevealedMaterial() { revealedMaterial = nil }

    private func parseInventory(_ data: JSONValue?) {
        let object = data?.objectValue ?? [:]
        let support = object["support"]?.objectValue ?? [:]
        inventoryMessage = support.bool("supported") == false ? "wallet.inventoryUnsupported" : nil
        utxos = object.objects("utxos").compactMap { row in
            let id = row.string("id", "outpoint") ?? ""
            guard !id.isEmpty else { return nil }
            return WalletUTXO(
                id: id,
                transactionID: row.string("transaction_id", "txid") ?? "",
                outpoint: row.string("outpoint") ?? id,
                asset: row.string("asset") ?? "BTC",
                amountSats: row.int("amount_sat") ?? 0,
                status: row.string("confirmation_status") ?? "",
                confirmations: Int(row.int("confirmations") ?? 0),
                addressLabel: row.string("address_label") ?? ""
            )
        }
    }

    private func parseHistory(_ data: JSONValue?) {
        let object = data?.objectValue ?? [:]
        balanceHistory = object.objects("rows").compactMap { row in
            guard let raw = row.string("date", "bucket", "occurred_at"),
                  let date = DaemonValueParser.date(raw) else { return nil }
            return WalletHistoryPoint(
                id: raw,
                date: date,
                amountSats: row.int("balance_sat", "quantity_sat", "amount_sat") ?? 0
            )
        }
    }

    private func operation(_ body: () async throws -> Void) async {
        isWorking = true; defer { isWorking = false }
        do { try await body(); errorMessage = nil }
        catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    private func localAuthentication(
        passphrase: String?, plaintextConfirmed: Bool, key: String, value: String
    ) throws -> JSONValue {
        if databaseEncrypted {
            guard let passphrase, !passphrase.isEmpty else {
                throw DaemonClientError.transport("database_passphrase_required")
            }
            return .object(["passphrase_secret": .string(passphrase)])
        }
        guard plaintextConfirmed else {
            throw DaemonClientError.transport("plaintext_confirmation_required")
        }
        return .object([key: .string(value)])
    }

    private static func parseEditMetadata(
        wallet row: [String: JSONValue],
        overview: [String: JSONValue]
    ) -> ConnectionEditMetadata {
        let kind = row.string("kind") ?? overview.string("kind") ?? ""
        let syncMode = row.string("sync_mode", "syncMode") ?? overview.string("syncMode", "sync_mode") ?? ""
        let syncSource = row.string("sync_source", "syncSource") ?? overview.string("syncSource", "sync_source") ?? ""
        let sourceFormat = overview.string("sourceFormat", "source_format") ?? ""
        let editKind: ConnectionEditKind
        if kind == "btcpay" || syncSource == "btcpay" || syncMode == "btcpay" {
            editKind = .btcpay
        } else if ["descriptor", "xpub"].contains(kind) {
            editKind = .descriptor
        } else if syncMode == "file_import" || !sourceFormat.isEmpty
                    || ["river", "phoenix", "csv"].contains(kind) {
            editKind = .fileWallet
        } else {
            editKind = .basic
        }
        let backend = row["backend"]?.objectValue ?? [:]
        let routes = row.objects("btcpay_provenance").compactMap(ConnectionProvenanceRoute.init)
        return ConnectionEditMetadata(
            editKind: editKind,
            walletKind: kind,
            chain: row.string("chain") ?? overview.string("chain") ?? "bitcoin",
            network: row.string("network") ?? overview.string("network") ?? "",
            syncMode: syncMode,
            syncSource: syncSource,
            sourceFormat: sourceFormat,
            backendName: backend.string("name") ?? "",
            backendSource: backend.string("source") ?? "",
            backendKind: backend.string("kind") ?? "",
            gapLimit: overview.int("gap").map(Int.init),
            birthday: row.string("birthday") ?? overview.string("birthday") ?? "",
            scriptTypes: row["script_types"]?.arrayValue?.compactMap(\.stringValue) ?? [],
            provenanceRoutes: routes,
            hasDescriptor: row.bool("descriptor") ?? false,
            hasChangeDescriptor: row.bool("change_descriptor") ?? false,
            isNode: ["lnd", "cln", "coreln", "core-lightning", "nwc"].contains(kind)
        )
    }

    static func updateArguments(
        original: WalletRow,
        metadata: ConnectionEditMetadata,
        draft: ConnectionEditDraft
    ) throws -> [String: JSONValue] {
        var args: [String: JSONValue] = [:]
        var clear: [String] = []
        let label = draft.label.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !label.isEmpty else { throw DaemonClientError.transport("connection_name_required") }
        if label != original.label { args["label"] = .string(label) }
        if draft.archived != original.deprecated { args["deprecated"] = .bool(draft.archived) }

        if metadata.canEditLiveBackend && draft.editKind != .btcpay {
            if draft.clearBackend { clear.append("backend") }
            else if !draft.backend.isEmpty { args["backend"] = .string(draft.backend) }
        }

        switch draft.editKind {
        case .descriptor:
            let material = draft.walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines)
            if !material.isEmpty {
                if material.hasPrefix("xpub") || material.hasPrefix("tpub") {
                    guard !draft.scriptTypes.isEmpty else {
                        throw DaemonClientError.transport("bare_xpub_script_type_required")
                    }
                    args["script_types"] = .array(draft.scriptTypes.sorted().map(JSONValue.string))
                }
                args["wallet_material"] = .string(material)
            } else if draft.scriptTypes.sorted() != metadata.scriptTypes.sorted(), !draft.scriptTypes.isEmpty {
                args["script_types"] = .array(draft.scriptTypes.sorted().map(JSONValue.string))
            }
            let gap = draft.gapLimit.trimmingCharacters(in: .whitespacesAndNewlines)
            if !gap.isEmpty {
                guard let value = Int(gap), (1...5000).contains(value) else {
                    throw DaemonClientError.transport("gap_limit_out_of_range")
                }
                if value != metadata.gapLimit { args["gap_limit"] = .integer(Int64(value)) }
            }
            let birthday = draft.birthday.trimmingCharacters(in: .whitespacesAndNewlines)
            if !birthday.isEmpty && birthday != metadata.birthday {
                let height = Int(birthday)
                let date = ISO8601DateFormatter().date(from: birthday + (birthday.count == 10 ? "T00:00:00Z" : ""))
                guard (height != nil && height! >= 0) || date != nil else {
                    throw DaemonClientError.transport("wallet_birthday_invalid")
                }
                args["birthday"] = .string(birthday)
            }
        case .btcpay:
            if !draft.backend.isEmpty { args["backend"] = .string(draft.backend) }
            if !draft.storeID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                args["store_id"] = .string(draft.storeID.trimmingCharacters(in: .whitespacesAndNewlines))
            }
            if !draft.paymentMethodID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                args["payment_method_id"] = .string(draft.paymentMethodID.trimmingCharacters(in: .whitespacesAndNewlines))
            }
        case .fileWallet:
            let source = draft.sourceFile.trimmingCharacters(in: .whitespacesAndNewlines)
            if !source.isEmpty { args["source_file"] = .string(source) }
        case .basic:
            break
        }

        if !draft.removedProvenanceRouteIDs.isEmpty {
            let remaining = metadata.provenanceRoutes.filter { !draft.removedProvenanceRouteIDs.contains($0.id) }
            if remaining.isEmpty { clear.append("btcpay_provenance") }
            else { args["btcpay_provenance"] = .array(remaining.map(\.json)) }
        }
        if !clear.isEmpty { args["clear"] = .array(clear.map(JSONValue.string)) }
        return args
    }

    private static func auth(_ passphrase: String?, key: String, value: String) -> JSONValue {
        if let passphrase, !passphrase.isEmpty { return ["passphrase_secret": .string(passphrase)] }
        return .object([key: .string(value)])
    }
}
