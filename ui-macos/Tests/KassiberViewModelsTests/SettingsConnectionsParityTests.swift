import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Settings and connections parity")
@MainActor
struct SettingsConnectionsParityTests {
    private static let backendList = DaemonRecord(kind: "ui.backends.settings.list", data: [
        "summary": ["default_backend": "mempool"],
        "backends": [[
            "name": "mempool", "kind": "esplora", "chain": "bitcoin", "network": "main",
            "url": "https://example.invalid/api", "timeout": 22, "notes": "Public fallback", "is_default": true,
        ]],
    ])

    @Test("backend inventory supports CRUD default and protocol-specific probes")
    func backendOperations() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiBackendsSettingsList: [Self.backendList],
            .uiBackendsCreate: [
                DaemonRecord(kind: "ui.backends.create", data: ["name": "local-core"]),
                DaemonRecord(kind: "ui.backends.create", data: ["name": "read-only-cln"]),
            ],
            .uiBackendsSetDefault: [DaemonRecord(kind: "ui.backends.set_default", data: ["default_backend": "mempool"])],
            .uiBackendsUpdate: [DaemonRecord(kind: "ui.backends.update", data: ["name": "mempool"])],
            .uiBackendsDelete: [DaemonRecord(kind: "ui.backends.delete", data: ["deleted": true])],
            .uiBackendsBitcoinrpcTest: [DaemonRecord(kind: "ui.backends.bitcoinrpc.test", data: ["ok": true, "credential_ref": "candidate-1"])],
            .uiBackendsDetectCore: [DaemonRecord(kind: "ui.backends.detect_core", data: ["ok": true, "message": "Core found"])],
        ])
        let model = BackendSettingsViewModel(daemon: client)
        await model.load()
        #expect(model.backends.map(\.name) == ["mempool"])

        var draft = BackendDraft()
        draft.name = "local-core"; draft.kind = "bitcoinrpc"; draft.url = "http://127.0.0.1:8332"
        draft.cookieFile = "/Users/test/.bitcoin/.cookie"
        draft.authHeader = "Bearer local-secret"
        draft.notes = "Loopback node"
        draft.timeout = 37
        await model.test(draft)
        #expect(model.probe?.ok == true)
        #expect(model.probe?.credentialReference == "candidate-1")
        await model.save(draft)
        var lightning = BackendDraft()
        lightning.name = "read-only-cln"; lightning.kind = "coreln"
        lightning.url = "cln://local"; lightning.chain = "bitcoin"; lightning.network = "main"
        lightning.commandoPeerID = "02peer"; lightning.token = "restricted-rune"
        lightning.lightningCLI = "/opt/homebrew/bin/lightning-cli"
        lightning.lightningDirectory = "/Users/test/.lightning/bitcoin"
        lightning.rpcFile = "/Users/test/.lightning/bitcoin/lightning-rpc"
        await model.save(lightning)
        var safeEdit = BackendDraft(row: try #require(model.backends.first))
        safeEdit.authHeader = "Bearer replacement"
        await model.save(safeEdit, editing: "mempool")
        await model.setDefault(try #require(model.backends.first))
        await model.delete(try #require(model.backends.first))
        await model.detectBitcoinCore()

        let calls = await client.calls()
        let probe = try #require(calls.first { $0.kind == .uiBackendsBitcoinrpcTest })
        #expect(probe.args?["config"]?.objectValue?["cookiefile"] == "/Users/test/.bitcoin/.cookie")
        let creates = calls.filter { $0.kind == .uiBackendsCreate }
        #expect(creates.count == 2)
        #expect(creates[0].args?["auth_header"] == "Bearer local-secret")
        #expect(creates[0].args?["notes"] == "Loopback node")
        #expect(creates[0].args?["timeout"] == 37)
        #expect(creates[0].args?["chain"] == "bitcoin")
        #expect(creates[1].args?["token"] == "restricted-rune")
        #expect(creates[1].args?["config"]?.objectValue?["commando_peer_id"] == "02peer")
        #expect(creates[1].args?["config"]?.objectValue?["lightning_cli"] == "/opt/homebrew/bin/lightning-cli")
        #expect(creates[1].args?["config"]?.objectValue?["lightning_dir"] == "/Users/test/.lightning/bitcoin")
        #expect(creates[1].args?["config"]?.objectValue?["rpc_file"] == "/Users/test/.lightning/bitcoin/lightning-rpc")
        let update = try #require(calls.first { $0.kind == .uiBackendsUpdate })
        #expect(update.args?["name"] == "mempool")
        #expect(update.args?["auth_header"] == "Bearer replacement")
        #expect(update.args?["url"] == nil)
        #expect(update.args?["kind"] == nil)
        #expect(update.args?["timeout"] == nil)
        #expect(update.args?["notes"] == nil)
        #expect(update.args?["config"] == nil)
        #expect(calls.contains { $0.kind == .uiBackendsSetDefault })
        #expect(calls.contains { $0.kind == .uiBackendsDelete })
        #expect(calls.contains { $0.kind == .uiBackendsDetectCore })
        #expect(!calls.contains { $0.kind == .backendsRevealToken })
    }

    @Test("AI providers keep API keys in the dedicated secret mutation and expose history policy")
    func providersAndHistory() async throws {
        let providerList = DaemonRecord(kind: "ai.providers.list", data: [
            "providers": [[
                "name": "ollama", "display_name": "Ollama", "base_url": "http://127.0.0.1:11434/v1",
                "kind": "local", "has_api_key": false, "is_default": true,
            ]],
        ])
        let client = ScriptedDaemonClient(scripts: [
            .aiProvidersList: [providerList],
            .aiProvidersCreate: [DaemonRecord(kind: "ai.providers.create", data: ["name": "remote"])],
            .aiProvidersSetApiKey: [DaemonRecord(kind: "ai.providers.set_api_key", data: ["has_api_key": true])],
            .aiProvidersSetDefault: [DaemonRecord(kind: "ai.providers.set_default", data: ["default": "ollama"])],
            .uiChatHistoryConfigure: [DaemonRecord(kind: "ui.chat.history.configure", data: [
                "history": "auto", "history_enabled": true, "database_encrypted": true,
            ])],
            .uiChatSessionsList: [DaemonRecord(kind: "ui.chat.sessions.list", data: ["sessions": [["id": "one"]]])],
            .uiChatSessionsClear: [DaemonRecord(kind: "ui.chat.sessions.clear", data: ["deleted": 1])],
        ])
        let providers = AIProviderSettingsViewModel(daemon: client)
        await providers.load()
        var draft = AIProviderDraft(); draft.name = "remote"; draft.baseURL = "https://ai.example/v1"
        draft.kind = "remote"; draft.apiKey = "secret"; draft.acknowledged = true
        await providers.save(draft)
        let history = ChatHistorySettingsViewModel(daemon: client)
        await history.load()
        #expect(history.mode == "auto")
        #expect(history.sessionCount == 1)
        await history.configure("off")
        await history.clear()
        #expect(history.sessionCount == 0)

        let calls = await client.calls()
        let metadata = try #require(calls.first { $0.kind == .aiProvidersCreate })
        #expect(metadata.args?["api_key"] == nil)
        #expect(calls.first { $0.kind == .aiProvidersSetApiKey }?.args?["api_key"] == "secret")
        #expect(calls.contains { $0.kind == .uiChatHistoryConfigure && $0.args?["history"] == "off" })
    }

    @Test("maintenance security rates replication and destructive actions preserve daemon contracts")
    func operationalSettings() async {
        let maintenance = DaemonRecord(kind: "ui.maintenance.settings", data: ["settings": [
            "auto_sync_before_report_reads": false, "market_rate_provider": "coinbase-exchange",
            "market_rate_providers": ["coinbase-exchange", "coingecko"],
            "source_classes": ["market_rates": true, "journals": true],
        ]])
        let client = ScriptedDaemonClient(scripts: [
            .uiMaintenanceSettings: [maintenance],
            .uiMaintenanceConfigure: [DaemonRecord(kind: "ui.maintenance.configure", data: maintenance.data)],
            .uiMaintenanceRun: [DaemonRecord(kind: "ui.maintenance.run", data: ["ready": true, "blockers": []])],
            .status: [DaemonRecord(kind: "status", data: ["database_encrypted": false])],
            .uiSecretsInit: [DaemonRecord(kind: "ui.secrets.init", data: ["encrypted": true, "backup_path": "/tmp/backup"])],
            .uiSecretsChangePassphrase: [DaemonRecord(kind: "ui.secrets.change_passphrase", data: ["changed": true])],
            .uiRatesRebuild: [DaemonRecord(kind: "ui.rates.rebuild", data: ["rebuilt": true])],
            .uiSyncStatus: [DaemonRecord(kind: "ui.sync.status", data: ["configured": true, "enabled": false, "transports": []])],
            .uiSyncEnable: [DaemonRecord(kind: "ui.sync.enable", data: ["enabled": true])],
            .uiSyncPush: [DaemonRecord(kind: "ui.sync.push", data: ["up_to_date": true])],
            .uiProfilesResetData: [DaemonRecord(kind: "ui.profiles.reset_data", data: ["removed": [:]])],
            .uiWorkspaceDelete: [DaemonRecord(kind: "ui.workspace.delete", data: ["deleted": true])],
        ])
        let maintenanceModel = MaintenanceSettingsViewModel(daemon: client)
        await maintenanceModel.load(); maintenanceModel.settings.autoSyncBeforeReportReads = true
        await maintenanceModel.save(); await maintenanceModel.run(syncMode: "if_enabled")
        let security = SecuritySettingsViewModel(daemon: client)
        await security.loadStatus(); await security.initializeEncryption(passphrase: "a very long passphrase", migrateCredentials: true)
        await security.changePassphrase(current: "old passphrase", new: "new passphrase")
        let rates = RatesSettingsViewModel(daemon: client)
        await rates.rebuild(source: "coinbase-exchange", pair: "BTC-EUR", days: 30, path: nil, reprice: true)
        let sync = ReplicationSettingsViewModel(daemon: client)
        await sync.load(); await sync.setEnabled(true); await sync.push(transport: nil)
        let destructive = DestructiveSettingsViewModel(daemon: client)
        await destructive.resetBook(name: "Private", clearSharedRates: true, passphrase: nil)
        await destructive.deleteWorkspace(name: "My Books", passphrase: "secret")

        let calls = await client.calls()
        #expect(calls.contains { $0.kind == .uiMaintenanceConfigure && $0.args?["auto_sync_before_report_reads"] == true })
        #expect(calls.contains { $0.kind == .uiSecretsInit && $0.args?["auth_response"]?.objectValue?["passphrase_secret"] == "a very long passphrase" })
        #expect(calls.contains { $0.kind == .uiRatesRebuild && $0.args?["reprice_transactions"] == true })
        #expect(calls.contains { $0.kind == .uiSyncEnable })
        #expect(calls.contains { $0.kind == .uiProfilesResetData && $0.args?["confirm"] == "RESET" })
        #expect(calls.contains { $0.kind == .uiWorkspaceDelete && $0.args?["confirm"] == "DELETE" })
    }

    @Test("connection catalog consumes safe options and public onboarding defaults")
    func connectionCatalog() async {
        let client = ScriptedDaemonClient(scripts: [
            .uiConnectionsSources: [DaemonRecord(kind: "ui.connections.sources", data: ["wallet_kinds": ["descriptor", "lnd"], "source_formats": ["btcpay_csv"]])],
            .uiBackendsSettingsList: [Self.backendList],
            .uiBackendsOptions: [DaemonRecord(kind: "ui.backends.options", data: ["backends": [["name": "mempool", "kind": "esplora"]]])],
            .uiBackendsPublicDefaults: [DaemonRecord(kind: "ui.backends.public_defaults", data: ["backends": [["name": "mempool", "kind": "esplora", "url": "https://public.example/api"]]])],
            .uiWalletsList: [DaemonRecord(kind: "ui.wallets.list", data: ["wallets": [["id": "w1", "label": "Cold", "kind": "descriptor"]]])],
            .uiOverviewSnapshot: [DaemonRecord(kind: "ui.overview.snapshot", data: ["priceEur": .number(70_000), "connections": [["id": "w1", "balance": .number(0.125)]]])],
        ])
        let model = ConnectionsParityViewModel(daemon: client)
        await model.load()
        #expect(model.catalog.walletKinds == ["descriptor", "lnd"])
        #expect(model.safeBackendOptions.map(\.name) == ["mempool"])
        #expect(model.publicDefaultBackends.first?.url == "https://public.example/api")
        #expect(model.wallets.first?.balanceBTC == 0.125)
        #expect(model.priceEUR == 70_000)
        let kinds = Set((await client.calls()).map(\.kind))
        #expect(kinds.isSuperset(of: [.uiConnectionsSources, .uiBackendsSettingsList, .uiBackendsOptions, .uiBackendsPublicDefaults, .uiWalletsList, .uiOverviewSnapshot]))
    }

    @Test("special connection flows and connection detail use guarded mutations")
    func connectionMutations() async throws {
        let labelsFile = FileManager.default.temporaryDirectory
            .appendingPathComponent("labels-\(UUID().uuidString).jsonl")
        try Data(#"{"type":"tx","ref":"abc","label":"salary"}"#.utf8).write(to: labelsFile)
        defer { try? FileManager.default.removeItem(at: labelsFile) }
        let walletList = DaemonRecord(kind: "ui.wallets.list", data: ["wallets": [[
            "id": "w1", "label": "Cold", "kind": "lnd", "chain": "bitcoin", "network": "main",
        ]]])
        let client = ScriptedDaemonClient(scripts: [
            .uiConnectionsBtcpayDiscover: [DaemonRecord(kind: "ui.connections.btcpay.discover", data: ["stores": [["id": "s1", "name": "Store"]], "payment_methods": [["store_id": "s1", "payment_method_id": "BTC-CHAIN", "sync_supported": true]]])],
            .uiConnectionsBtcpayCreate: [DaemonRecord(kind: "ui.connections.btcpay.create", data: ["wallets": []])],
            .uiConnectionsBullbitcoinWalletCreate: [DaemonRecord(kind: "ui.connections.bullbitcoin_wallet.create", data: ["wallets": []])],
            .uiWalletsImportSamourai: [DaemonRecord(kind: "ui.wallets.import_samourai", data: ["children": []])],
            .uiMetadataBip329Preview: [DaemonRecord(kind: "ui.metadata.bip329.preview", data: ["records": 2, "counts": ["exact": 1, "ambiguous": 1, "unmatched": 0, "conflicts": 0]])],
            .uiMetadataBip329Import: [DaemonRecord(kind: "ui.metadata.bip329.import", data: ["transaction_tags_added": 1])],
            .uiWalletsList: [walletList],
            .uiOverviewSnapshot: [DaemonRecord(kind: "ui.overview.snapshot", data: ["connections": [["id": "w1", "balance": .number(0.0009)]]])],
            .uiBackendsOptions: [DaemonRecord(kind: "ui.backends.options", data: ["backends": []])],
            .status: [DaemonRecord(kind: "status", data: ["database_encrypted": false])],
            .uiWalletsUtxos: [DaemonRecord(kind: "ui.wallets.utxos", data: ["utxos": [[
                "id": "coin-1", "outpoint": "abc:0", "transaction_id": "abc", "amount_sat": 90_000,
                "confirmation_status": "confirmed", "confirmations": 6, "address_label": "Savings",
            ]]])],
            .uiTransactionsList: [DaemonRecord(kind: "ui.transactions.list", data: ["txs": [[
                "id": "tx-1", "date": "2026-01-01", "type": "Receive", "wallet": "Cold", "amountSat": 90_000,
            ]]])],
            .uiReportsBalanceHistory: [DaemonRecord(kind: "ui.reports.balance_history", data: ["rows": [[
                "date": "2026-01-01", "balance_sat": 90_000,
            ]]])],
            .uiConnectionsNodeSnapshot: [DaemonRecord(kind: "ui.connections.node.snapshot", data: [
                "alias": "Node", "totalLocalBalanceSat": 60_000, "totalRemoteBalanceSat": 40_000,
                "channels": [["id": "channel-1", "peerAlias": "Peer", "state": "active", "capacitySat": 100_000, "localBalanceSat": 60_000, "remoteBalanceSat": 40_000, "isPrivate": false, "forwardCount": 2, "earnedRoutingSat": 12]],
                "forwards": [["id": "forward-1", "occurredAt": "2026-01-01T12:00:00Z", "inPeerAlias": "A", "outPeerAlias": "B", "amountInMsat": 500_000, "feeMsat": 2_000, "status": "settled"]],
                "routing": ["windowLabel": "Last 30 days", "routingRevenueSat": 12, "paymentCostSat": 2, "rebalanceCostSat": 1, "onchainCostSat": 3, "netProfitSat": 6],
            ])],
            .uiWalletsSync: [DaemonRecord(kind: "ui.wallets.sync.progress", data: ["phase": "import", "processed": 1, "total": 1]), DaemonRecord(kind: "ui.wallets.sync", data: ["results": []])],
            .uiWalletsUpdate: [DaemonRecord(kind: "ui.wallets.update", data: ["wallet": ["id": "w1"]])],
            .walletsRevealDescriptor: [DaemonRecord(kind: "wallets.reveal_descriptor", data: ["descriptor": "wpkh(xpub...)"])],
            .uiWalletsDelete: [DaemonRecord(kind: "ui.wallets.delete", data: ["deleted": true])],
        ])
        let setup = ConnectionsParityViewModel(daemon: client)
        await setup.discoverBTCPay(savedBackend: "btcpay", label: nil, serverURL: nil, apiKey: nil)
        await setup.createBTCPay(
            label: "Store", savedBackend: "btcpay", backendLabel: nil,
            serverURL: nil, apiKey: nil, storeID: "s1",
            paymentMethodIDs: ["BTC-CHAIN", "LBTC-CHAIN"],
            existingWalletRoutes: ["BTC-CHAIN": "w1", "LBTC-CHAIN": "liquid-1"],
            syncProvenance: true
        )
        await setup.createBullBitcoinWallet(
            label: "Bull", sourceFile: "/tmp/bull.csv", networks: ["bitcoin", "liquid"]
        )
        await setup.createBullBitcoinWallet(
            label: "Bull mapped", sourceFile: "/tmp/bull.csv", networks: ["bitcoin", "liquid"],
            existingRoutes: [(network: "bitcoin", wallet: "w1"), (network: "liquid", wallet: "liquid-1")]
        )
        await setup.importSamourai(label: "Samourai", backend: "mempool", network: "main", gapLimit: 20, deposit: "xpub", badbank: "", premix: "", postmix: "", ricochet: "")
        await setup.previewBIP329(file: labelsFile.path); await setup.importBIP329(file: labelsFile.path)

        let detail = ConnectionDetailParityViewModel(daemon: client)
        await detail.load(walletRef: "w1"); await detail.sync(walletRef: "w1")
        #expect(detail.recentTransactions.map(\.id) == ["tx-1"])
        #expect(detail.utxos.map(\.outpoint) == ["abc:0"])
        #expect(detail.balanceHistory.first?.amountSats == 90_000)
        #expect(detail.nodeChannels.first?.peer == "Peer")
        #expect(detail.nodeForwards.first?.route == "A → B")
        #expect(detail.nodeRouting?.netProfitSats == 6)
        await detail.update(walletRef: "w1", label: "Cold", archived: true, passphrase: nil)
        await detail.revealDescriptor(walletRef: "w1", passphrase: nil, plaintextConfirmed: true)
        await detail.delete(walletRef: "w1", label: "Cold", cascade: true, passphrase: nil, plaintextConfirmed: true)
        #expect(detail.revealedMaterial == "wpkh(xpub...)")
        detail.clearRevealedMaterial()
        #expect(detail.revealedMaterial == nil)
        #expect(detail.didDelete)
        let calls = await client.calls()
        let btcpay = try #require(calls.first { $0.kind == .uiConnectionsBtcpayCreate })
        #expect(btcpay.args?["mode"] == "existing_wallets")
        #expect(btcpay.args?["payment_method_ids"]?.arrayValue?.compactMap(\.stringValue) == ["BTC-CHAIN", "LBTC-CHAIN"])
        #expect(btcpay.args?["routes"]?.arrayValue?.count == 2)
        #expect(btcpay.args?["sync_provenance"] == true)
        let bullCalls = calls.filter { $0.kind == .uiConnectionsBullbitcoinWalletCreate }
        #expect(bullCalls.count == 2)
        #expect(bullCalls[0].args?["mode"] == "wallet_sources")
        #expect(bullCalls[0].args?["networks"]?.arrayValue?.compactMap(\.stringValue) == ["bitcoin", "liquid"])
        #expect(bullCalls[1].args?["mode"] == "existing_wallets")
        #expect(bullCalls[1].args?["routes"]?.arrayValue?.count == 2)
        #expect(calls.contains { $0.kind == .uiWalletsImportSamourai })
        #expect(calls.contains { $0.kind == .uiMetadataBip329Import })
        #expect(calls.contains { $0.kind == .uiReportsBalanceHistory && $0.args?["wallet"] == "w1" })
        #expect(calls.contains { $0.kind == .uiWalletsUpdate && $0.args?["auth_response"]?.objectValue?["plaintext_change_ack"] == "CHANGE LOCAL DATA" })
        #expect(calls.contains { $0.kind == .uiWalletsDelete && $0.args?["confirm_wallet"] == "Cold" })
    }

    @Test("advanced connection edit mirrors descriptor BTCPay file and provenance payloads")
    func advancedConnectionEdit() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiWalletsList: [DaemonRecord(kind: "ui.wallets.list", data: ["wallets": [[
                "id": "w1", "label": "Cold", "kind": "descriptor", "chain": "bitcoin", "network": "main",
                "sync_mode": "backend_descriptor", "descriptor": true,
                "backend": ["name": "mempool", "source": "explicit", "kind": "esplora"],
                "script_types": ["p2wpkh"],
                "btcpay_provenance": [
                    ["backend": "shop", "store_id": "store-1", "payment_method_id": "BTC-CHAIN"],
                    ["backend": "shop", "store_id": "store-2", "payment_method_id": "BTC-LN"],
                ],
            ]]])],
            .uiWalletsUtxos: [DaemonRecord(kind: "ui.wallets.utxos", data: ["utxos": []])],
            .uiTransactionsList: [DaemonRecord(kind: "ui.transactions.list", data: ["txs": []])],
            .uiReportsBalanceHistory: [DaemonRecord(kind: "ui.reports.balance_history", data: ["rows": []])],
            .uiOverviewSnapshot: [DaemonRecord(kind: "ui.overview.snapshot", data: ["connections": [[
                "id": "w1", "label": "Cold", "kind": "descriptor", "gap": 20,
            ]]])],
            .uiBackendsOptions: [DaemonRecord(kind: "ui.backends.options", data: ["backends": [
                ["name": "local-core", "kind": "bitcoinrpc", "chain": "bitcoin", "network": "main"],
                ["name": "shop", "kind": "btcpay"],
            ]])],
            .status: [DaemonRecord(kind: "status", data: ["database_encrypted": false])],
            .uiWalletsUpdate: [DaemonRecord(kind: "ui.wallets.update", data: ["wallet": ["id": "w1"]])],
        ])
        let model = ConnectionDetailParityViewModel(daemon: client)
        await model.load(walletRef: "w1")
        #expect(model.editMetadata?.editKind == .descriptor)
        #expect(model.editMetadata?.gapLimit == 20)
        #expect(model.editMetadata?.canClearLiveBackend == true)
        #expect(model.liveBackendOptions.map(\.name) == ["local-core"])
        #expect(model.btcpayBackendOptions.map(\.name) == ["shop"])

        let wallet = try #require(model.wallet)
        var descriptor = model.makeEditDraft(fallback: wallet)
        descriptor.label = "Cold Vault"
        descriptor.archived = true
        descriptor.backend = "local-core"
        descriptor.walletMaterial = "xpub-replacement"
        descriptor.scriptTypes = ["p2wpkh", "p2tr"]
        descriptor.gapLimit = "40"
        descriptor.removedProvenanceRouteIDs = [try #require(model.editMetadata?.provenanceRoutes.first?.id)]
        await model.updateConfiguration(
            walletRef: "w1", original: wallet, draft: descriptor,
            passphrase: nil, plaintextConfirmed: true
        )
        #expect(model.didUpdate)

        var btcpay = model.makeEditDraft(fallback: wallet)
        btcpay.editKind = .btcpay
        btcpay.backend = "shop"
        btcpay.storeID = "store-new"
        btcpay.paymentMethodID = "BTC-LN"
        await model.updateConfiguration(
            walletRef: "w1", original: wallet, draft: btcpay,
            passphrase: nil, plaintextConfirmed: true
        )

        var file = model.makeEditDraft(fallback: wallet)
        file.editKind = .fileWallet
        file.sourceFile = "/tmp/replacement.csv"
        await model.updateConfiguration(
            walletRef: "w1", original: wallet, draft: file,
            passphrase: nil, plaintextConfirmed: true
        )

        let updates = (await client.calls()).filter { $0.kind == .uiWalletsUpdate }
        #expect(updates.count == 3)
        let descriptorArgs = try #require(updates.first?.args)
        #expect(descriptorArgs["label"] == "Cold Vault")
        #expect(descriptorArgs["backend"] == "local-core")
        #expect(descriptorArgs["wallet_material"] == "xpub-replacement")
        #expect(descriptorArgs["script_types"]?.arrayValue?.compactMap(\.stringValue) == ["p2tr", "p2wpkh"])
        #expect(descriptorArgs["gap_limit"] == 40)
        #expect(descriptorArgs["btcpay_provenance"]?.arrayValue?.count == 1)
        #expect(descriptorArgs["auth_response"]?.objectValue?["plaintext_change_ack"] == "CHANGE LOCAL DATA")
        #expect(updates[1].args?["backend"] == "shop")
        #expect(updates[1].args?["store_id"] == "store-new")
        #expect(updates[1].args?["payment_method_id"] == "BTC-LN")
        #expect(updates[2].args?["source_file"] == "/tmp/replacement.csv")
    }

    @Test("Lightning node detail retains snapshot lifecycle activity and profitability fields")
    func lightningNodeDetail() async throws {
        let snapshot = DaemonRecord(kind: "ui.connections.node.snapshot", data: [
            "alias": "Routing Node", "pubkey": "02operator", "network": "mainnet",
            "implementationVersion": "Core Lightning v24.11", "peerCount": 4,
            "blockHeight": 901_234, "invoiceCount": 12, "paidInvoiceCount": 9,
            "expiredInvoiceCount": 3, "paymentCount": 8, "completedPaymentCount": 6,
            "failedPaymentCount": 2, "onchainBalanceSat": 50_000,
            "totalLocalBalanceSat": 600_000, "totalRemoteBalanceSat": 400_000,
            "totalCapacitySat": 1_000_000,
            "channels": [[
                "id": "channel-open", "shortChannelId": "900000x1x0",
                "fundingOutpoint": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:0",
                "peerAlias": "Open Peer", "peerPubkey": "03peer", "state": "active",
                "capacitySat": 1_000_000, "localBalanceSat": 600_000,
                "remoteBalanceSat": 400_000, "isPrivate": false, "isInitiator": true,
                "baseFeeMsat": 1_000, "feeRatePpm": 250,
                "openedAt": "2026-01-01T00:00:00Z", "forwardCount": 7,
                "earnedRoutingSat": 3_000, "htlcCount": 2,
                "lastActivityAt": "2026-06-01T12:00:00Z",
            ]],
            "closedChannels": [[
                "id": "channel-closed", "shortChannelId": "800000x2x1",
                "peerAlias": "Closed Peer", "state": "force_closed",
                "capacitySat": 250_000, "localBalanceSat": 0, "remoteBalanceSat": 0,
                "isPrivate": true, "isInitiator": false,
                "openedAt": "2025-01-01T00:00:00Z", "closedAt": "2026-02-01T00:00:00Z",
                "closeKind": "force", "forwardCount": 1, "earnedRoutingSat": 10,
            ]],
            "forwards": [[
                "id": "forward-1", "occurredAt": "2026-06-01T12:00:00Z",
                "inPeerAlias": "Inbound", "inShortChannelId": "900000x1x0",
                "outPeerAlias": "Outbound", "outShortChannelId": "900001x1x1",
                "amountInMsat": 900_000, "amountOutMsat": 895_000, "feeMsat": 5_000,
                "status": "failed", "failureReason": "temporary_channel_failure",
            ]],
            "routing": [
                "windowLabel": "Last 90 days", "routingRevenueSat": 3_000,
                "paymentCostSat": 300, "rebalanceCostSat": 200, "onchainCostSat": 500,
                "netProfitSat": 2_000, "forwardCount": 7, "paymentCount": 6,
                "rebalanceCount": 2,
            ],
            "capabilities": [
                "nodeSnapshot": true, "routingProfitability": true,
                "channelBalances": true, "channelLifecycle": true, "forwardEvents": true,
                "invoiceActivity": true, "paymentActivity": true, "onchainBalance": true,
            ],
            "connection": [
                "id": "node-1", "label": "My Node", "kind": "coreln",
                "lightningCapabilities": ["nodeSnapshot": true, "routingProfitability": true],
            ],
        ])
        let profitability = DaemonRecord(kind: "ui.reports.lightning_profitability", data: [
            "connection": ["id": "node-1", "label": "My Node", "kind": "coreln"],
            "windowLabel": "Last 90 days",
            "summary": [
                "routingRevenueSat": 3_000, "paymentCostSat": 300,
                "rebalanceCostSat": 200, "onchainCostSat": 500,
                "netProfitSat": 2_000, "forwardCount": 7, "paymentCount": 6,
                "rebalanceCount": 2,
            ],
            "channels": [[
                "channelId": "channel-open", "peerAlias": "Open Peer",
                "capacitySat": 1_000_000, "earnedRoutingSat": 3_000,
                "openCostSat": 2_500, "coversOpenCost": true,
            ]],
        ])
        let client = ScriptedDaemonClient(scripts: [
            .uiConnectionsNodeSnapshot: [snapshot],
            .uiReportsLightningProfitability: [profitability],
        ])
        let model = ConnectionDetailParityViewModel(daemon: client)
        await model.loadNode(walletRef: "node-1", windowDays: 90)

        #expect(model.nodeWindowDays == 90)
        #expect(model.nodeSummary?.pubkey == "02operator")
        #expect(model.nodeSummary?.implementationVersion == "Core Lightning v24.11")
        #expect(model.nodeSummary?.blockHeight == 901_234)
        #expect(model.nodeSummary?.failedOrExpiredCount == 5)
        #expect(model.nodeCapabilities?.channelLifecycle == true)
        #expect(model.nodeConnection?.kind == "coreln")
        let open = try #require(model.nodeChannels.first)
        #expect(open.fundingOutpoint?.hasSuffix(":0") == true)
        #expect(open.peerPubkey == "03peer")
        #expect(open.baseFeeMsat == 1_000)
        #expect(open.feeRatePPM == 250)
        #expect(open.htlcCount == 2)
        let closed = try #require(model.nodeClosedChannels.first)
        #expect(closed.closeKind == "force")
        #expect(closed.isInitiator == false)
        #expect(closed.isClosed)
        #expect(model.visibleNodeChannels.map(\.id) == ["channel-open"])
        model.showClosedNodeChannels = true
        #expect(model.visibleNodeChannels.map(\.id) == ["channel-open", "channel-closed"])
        model.selectNodeChannel("channel-closed")
        #expect(model.selectedNodeChannel?.peer == "Closed Peer")
        let forward = try #require(model.nodeForwards.first)
        #expect(forward.inShortChannelID == "900000x1x0")
        #expect(forward.amountOutMsat == 895_000)
        #expect(forward.failureReason == "temporary_channel_failure")
        #expect(model.nodeRouting?.rebalanceCount == 2)
        #expect(model.nodeProfitability?.channels.first?.openCostSats == 2_500)
        #expect(model.nodeProfitability?.channels.first?.coversOpenCost == true)

        let calls = await client.calls()
        #expect(calls.contains {
            $0.kind == .uiConnectionsNodeSnapshot && $0.args?["window_days"] == 90
        })
        #expect(calls.contains {
            $0.kind == .uiReportsLightningProfitability && $0.args?["window_days"] == 90
        })
    }

    @Test("privacy hygiene remains a local-only settings read")
    func privacyHygiene() async {
        let client = ScriptedDaemonClient(scripts: [
            .uiPrivacyHygieneSnapshot: [DaemonRecord(kind: "ui.privacy_hygiene.snapshot", data: [
                "summary": ["state": "partial", "risk_level": "high", "risk_count": 3, "unknown_count": 1],
                "coverage": ["transaction_scored": 7, "transaction_total": 10],
                "wallets": [["id": "w1", "label": "Cold", "state": "partial", "risk_level": "high", "risk_count": 3, "address": ["reused_address_count": 2]]],
            ])],
        ])
        let model = PrivacyHygieneSettingsViewModel(daemon: client)
        await model.load()
        #expect(model.riskLevel == "high")
        #expect(model.scoredTransactions == 7)
        #expect(model.wallets.first?.reusedAddresses == 2)
        #expect((await client.calls()).map(\.kind) == [.uiPrivacyHygieneSnapshot])
    }
}
