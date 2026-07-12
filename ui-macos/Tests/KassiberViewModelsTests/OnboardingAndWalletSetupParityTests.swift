import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Onboarding and watch-only connection setup")
@MainActor
struct OnboardingAndWalletSetupParityTests {
    private let p2pkh = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
    private let p2sh = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
    private let bech32 = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
    private let wif = "5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ"
    private let xpub = "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet"

    @Test("Settings and Connections views do not bypass the bilingual catalog with English labels")
    func noRawEnglishViewLabels() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let source = try String(
            contentsOf: packageRoot.appending(path: "Sources/KassiberApp/SettingsConnectionsScreens.swift"),
            encoding: .utf8
        )
        let expression = try NSRegularExpression(
            pattern: #"(?:Text|TextField|SecureField|Button|Toggle|Section|GroupBox|Picker|Label|navigationTitle)\(\"[A-Za-z]"#
        )
        let matches = expression.matches(in: source, range: NSRange(source.startIndex..., in: source))
        #expect(matches.isEmpty, "User-facing English labels must use AppLocalization/parityString.")
    }

    @Test("address-list parser validates checksums, deduplicates, and never retains key material")
    func addressListSecurityBoundary() {
        let parsed = AddressListParser.parse("\(p2pkh),\(p2sh)\n\(bech32)\n\(p2pkh)\n\(wif)\n\(xpub)\nheader")
        #expect(parsed.valid == [p2pkh, p2sh, bech32])
        #expect(parsed.duplicates == 1)
        #expect(parsed.privateKeys == 1)
        #expect(parsed.publicKeys == 1)
        #expect(parsed.invalid == ["header"])
        #expect(!parsed.entries.contains(wif))
        #expect(!parsed.entries.contains(xpub))

        let scrubbed = AddressListParser.scrubKeyMaterial("\(p2pkh) \(wif) \(xpub)")
        #expect(scrubbed.text == p2pkh)
        #expect(scrubbed.privateKeys == 1)
        #expect(scrubbed.publicKeys == 1)
    }

    @Test("address, BIP352, and Liquid setup emit their distinct daemon contracts")
    func walletSetupContracts() async throws {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiWalletsCreate: [DaemonRecord(kind: "ui.wallets.create", data: ["wallet": ["id": "created"]])],
            .uiWalletsSync: [DaemonRecord(kind: "ui.wallets.sync", data: ["results": []])],
        ])
        let bitcoinCore = try #require(SettingsBackendRow([
            "name": "core", "kind": "bitcoinrpc", "chain": "bitcoin", "network": "main",
            "silent_payments": true, "is_default": true,
        ]))
        let silentElectrum = try #require(SettingsBackendRow([
            "name": "sp-electrum", "kind": "electrum", "chain": "bitcoin", "network": "main",
            "silent_payments": true,
        ]))
        let liquid = try #require(SettingsBackendRow([
            "name": "liquid", "kind": "liquid-esplora", "chain": "liquid", "network": "liquidv1",
        ]))

        let addresses = WalletConnectionSetupViewModel(kind: .addressList, daemon: daemon)
        addresses.configure(backends: [bitcoinCore, liquid])
        addresses.label = "Legacy Core"
        addresses.setAddressInput("\(p2pkh)\n\(wif)\n\(p2sh)")
        #expect(addresses.addressInput == "\(p2pkh)\n\(p2sh)")
        await addresses.create()
        #expect(addresses.didSave)

        let silent = WalletConnectionSetupViewModel(kind: .silentPayment, daemon: daemon)
        silent.silentPaymentScanMode = .serverAssisted
        silent.configure(backends: [silentElectrum, bitcoinCore, liquid])
        #expect(silent.availableBackends.map(\.name) == ["core"])
        silent.label = "Silent Receipts"
        silent.backend = "core"
        silent.silentPaymentMaterial = "sp(spscan1qexample,spend1qexample)"
        silent.silentPaymentStartHeight = "840000"
        silent.acknowledgeServerPrivacy = true
        await silent.create()
        #expect(silent.didSave)

        let liquidWallet = WalletConnectionSetupViewModel(kind: .liquidDescriptor, daemon: daemon)
        liquidWallet.configure(backends: [bitcoinCore, liquid])
        #expect(liquidWallet.availableBackends.map(\.name) == ["liquid"])
        liquidWallet.label = "Liquid Vault"
        liquidWallet.walletMaterial = "ct(slip77(private-blinding-key),elwpkh(xpub/0/*))"
        liquidWallet.syncAfterCreate = false
        await liquidWallet.create()
        #expect(liquidWallet.didSave)

        let calls = await daemon.calls().filter { $0.kind == .uiWalletsCreate }
        #expect(calls.count == 3)
        #expect(calls[0].args?["kind"] == "address")
        #expect(calls[0].args?["addresses"]?.arrayValue?.compactMap(\.stringValue) == [p2pkh, p2sh])
        #expect(calls[0].args?["wallet_material"] == nil)
        #expect(calls[1].args?["kind"] == "silent-payment")
        #expect(calls[1].args?["sp_scan_mode"] == "server_assisted")
        #expect(calls[1].args?["sp_scan_start_height"] == 840000)
        #expect(calls[1].args?["sp_acknowledge_server_warning"] == true)
        #expect(calls[2].args?["kind"] == "descriptor")
        #expect(calls[2].args?["chain"] == "liquid")
        #expect(calls[2].args?["network"] == "liquidv1")
        #expect(calls[2].args?["backend"] == "liquid")
    }

    @Test("Silent Payments gates scan origin and explicit privacy acknowledgements")
    func silentPaymentGates() throws {
        let daemon = ScriptedDaemonClient()
        let backend = try #require(SettingsBackendRow([
            "name": "scanner", "kind": "bitcoinrpc", "chain": "bitcoin", "silent_payments": true,
        ]))
        let model = WalletConnectionSetupViewModel(kind: .silentPayment, daemon: daemon)
        model.configure(backends: [backend])
        model.label = "Silent"
        model.silentPaymentMaterial = "sp(material)"
        #expect(model.validationIssue == .silentPaymentStartRequired)
        model.silentPaymentFullHistory = true
        #expect(model.validationIssue == .silentPaymentFullHistoryAcknowledgementRequired)
        model.acknowledgeFullHistory = true
        #expect(model.validationIssue == nil)
        model.silentPaymentScanMode = .serverAssisted
        #expect(model.validationIssue == .silentPaymentServerAcknowledgementRequired)
        model.acknowledgeServerPrivacy = true
        #expect(model.canSubmit)
    }

    @Test("special connection coordinator owns validation and staged submit routing")
    func specialConnectionCoordinator() async throws {
        let file = FileManager.default.temporaryDirectory
            .appending(path: "bip329-\(UUID().uuidString).jsonl")
        try Data(#"{"type":"tx","ref":"abc","label":"reviewed"}"#.utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        let daemon = ScriptedDaemonClient(scripts: [
            .uiConnectionsBtcpayDiscover: [DaemonRecord(
                kind: "ui.connections.btcpay.discover",
                data: [
                    "stores": [["id": "store-1", "name": "Shop"]],
                    "payment_methods": [[
                        "store_id": "store-1", "payment_method_id": "BTC-CHAIN",
                        "label": "Bitcoin", "enabled": .bool(true),
                        "sync_supported": .bool(true),
                    ]],
                ]
            )],
            .uiMetadataBip329Preview: [DaemonRecord(
                kind: "ui.metadata.bip329.preview",
                data: [
                    "records": .integer(1),
                    "counts": [
                        "exact": .integer(1), "ambiguous": .integer(0),
                        "unmatched": .integer(0), "conflicts": .integer(0),
                    ],
                ]
            )],
        ])
        let operations = ConnectionsParityViewModel(daemon: daemon)

        let btcpay = SpecialConnectionSetupViewModel(mode: .btcpay, operations: operations)
        #expect(!btcpay.canSubmit)
        btcpay.label = "Shop"
        btcpay.serverURL = "https://pay.example"
        btcpay.apiKey = "secret"
        #expect(btcpay.canSubmit)
        #expect(await btcpay.submit() == false)
        #expect(btcpay.storeID == "store-1")
        #expect(btcpay.selectedBTCPayMethodIDs == ["BTC-CHAIN"])

        let bip329 = SpecialConnectionSetupViewModel(mode: .bip329, operations: operations)
        #expect(!bip329.canSubmit)
        bip329.sourceFile = file.path
        #expect(bip329.canSubmit)
        #expect(await bip329.submit() == false)
        #expect(bip329.bip329Preview?.records == 1)
        #expect(operations.canImportBIP329(file: file.path))

        let calls = await daemon.calls()
        #expect(calls.contains {
            $0.kind == .uiConnectionsBtcpayDiscover
                && $0.args?["server_url"] == "https://pay.example"
        })
        #expect(calls.contains {
            $0.kind == .uiMetadataBip329Preview
                && $0.args?["file"]?.stringValue == file.path
        })
    }

    @Test("onboarding uses five gated steps and quick start still stops at Security")
    func onboardingGates() {
        let model = OnboardingParityViewModel(daemon: ScriptedDaemonClient())
        #expect(model.flowMode == .start)
        model.beginSetup()
        #expect(model.step == .essentials)
        #expect(model.stepComplete(.essentials))
        model.workspace = ""
        #expect(!model.canContinue)
        model.workspace = "My Books"
        model.goNext()
        #expect(model.step == .sync)
        model.backendMode = .skip
        #expect(!model.canContinue)
        model.skipBackendsAcknowledged = true
        #expect(model.canContinue)

        model.beginQuickStart()
        #expect(model.step == .security)
        #expect(model.databaseMode == .encrypted)
        #expect(!model.canContinue)
        model.databasePassphrase = "a sufficiently long passphrase"
        model.databasePassphraseConfirmation = model.databasePassphrase
        model.recoveryAcknowledged = true
        #expect(model.canContinue)
        model.goNext()
        #expect(model.step == .review)
        #expect(model.stepComplete(.review))
    }

    @Test("onboarding review commits exact backend and provider contracts")
    func onboardingFinishContract() async throws {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiSecretsInit: [DaemonRecord(kind: "ui.secrets.init", data: ["encrypted": true])],
            .uiOnboardingComplete: [DaemonRecord(kind: "ui.onboarding.complete", data: ["profile": ["id": "p1"]])],
            .aiProvidersCreate: [DaemonRecord(kind: "ai.providers.create", data: ["name": "remote-ai"])],
            .aiProvidersSetApiKey: [DaemonRecord(kind: "ai.providers.set_api_key", data: ["stored": true])],
            .aiProvidersSetDefault: [DaemonRecord(kind: "ai.providers.set_default", data: ["name": "remote-ai"])],
        ])
        let model = OnboardingParityViewModel(daemon: daemon)
        model.beginSetup()
        model.backendMode = .custom
        model.backend.name = "liquid-private"
        model.backend.kind = "liquid-esplora"
        model.backend.url = "https://liquid.example/api"
        model.aiMode = .remote
        model.provider.name = "remote-ai"
        model.provider.baseURL = "https://ai.example/v1"
        model.provider.apiKey = "secret"
        model.provider.acknowledged = true
        model.databasePassphrase = "a sufficiently long passphrase"
        model.databasePassphraseConfirmation = model.databasePassphrase
        model.recoveryAcknowledged = true
        model.jump(to: .review)
        #expect(model.canContinue)
        await model.finish()
        #expect(model.didComplete)

        let calls = await daemon.calls()
        let onboarding = try #require(calls.first { $0.kind == .uiOnboardingComplete })
        let backend = try #require(onboarding.args?["backend"]?.objectValue)
        #expect(backend["kind"] == "liquid-esplora")
        #expect(backend["chain"] == "liquid")
        #expect(backend["network"] == "liquidv1")
        let provider = try #require(calls.first { $0.kind == .aiProvidersCreate })
        #expect(provider.args?["kind"] == "remote")
        #expect(provider.args?["acknowledged"] == true)
        #expect(calls.contains { $0.kind == .aiProvidersSetApiKey && $0.args?["api_key"] == "secret" })
    }

    @Test("onboarding project import validates, activates, and persists the local data root")
    func onboardingImportProject() async throws {
        let root = FileManager.default.temporaryDirectory.appending(path: "kassiber-onboarding-\(UUID().uuidString)")
        let data = root.appending(path: "data", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: data, withIntermediateDirectories: true)
        let markers = "SQLite format 3\0 create table settings workspaces profiles workspace_id fiat_currency"
        try Data(markers.utf8).write(to: data.appending(path: "kassiber.sqlite3"))
        defer { try? FileManager.default.removeItem(at: root) }
        let suite = "kassiber-onboarding-tests-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let daemon = ScriptedDaemonClient(scripts: [
            .daemonUnlock: [DaemonRecord(kind: "daemon.unlock", data: ["unlocked": true])],
        ])
        let model = OnboardingParityViewModel(daemon: daemon, defaults: defaults)
        await model.importProject(root)
        #expect(model.didComplete)
        #expect(await daemon.activatedDataRoots() == [data.path])
        #expect(defaults.string(forKey: "projects.imported.dataRoot") == data.path)
        #expect(defaults.bool(forKey: "projects.imported.encrypted") == false)
    }

    @Test("encrypted project import remains gated until explicit unlock")
    func onboardingEncryptedImport() async throws {
        let root = FileManager.default.temporaryDirectory.appending(path: "kassiber-encrypted-import-\(UUID().uuidString)")
        let data = root.appending(path: "data", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: data, withIntermediateDirectories: true)
        try Data(repeating: 0x8a, count: 64).write(to: data.appending(path: "kassiber.sqlite3"))
        defer { try? FileManager.default.removeItem(at: root) }
        let suite = "kassiber-encrypted-import-tests-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let daemon = ScriptedDaemonClient(scripts: [
            .daemonUnlock: [DaemonRecord(kind: "daemon.unlock", data: ["unlocked": true])],
        ])
        let model = OnboardingParityViewModel(daemon: daemon, defaults: defaults)
        await model.importProject(root)
        #expect(model.awaitingImportedProjectUnlock)
        #expect(!model.didComplete)
        #expect(defaults.string(forKey: "projects.imported.dataRoot") == nil)
        model.importedProjectPassphrase = "local secret"
        await model.unlockImportedProject()
        #expect(model.didComplete)
        #expect(!model.awaitingImportedProjectUnlock)
        let unlock = try #require((await daemon.calls()).first { $0.kind == .daemonUnlock })
        #expect(unlock.args?["require_existing_project"] == true)
        #expect(unlock.args?["auth_response"]?.objectValue?["passphrase_secret"] == "local secret")
    }
}
