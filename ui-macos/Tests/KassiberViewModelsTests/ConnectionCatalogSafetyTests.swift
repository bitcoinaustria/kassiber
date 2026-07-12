import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Connection catalog and import preview safety")
@MainActor
struct ConnectionCatalogSafetyTests {
    @Test("frontend catalog resolves daemon capabilities and keeps planned entries disabled")
    func catalogResolution() async {
        let walletKinds = [
            "descriptor", "address", "silent-payment", "samourai", "wasabi", "ledgerlive",
            "coreln", "lnd", "phoenix", "bullbitcoin", "custom", "river",
            "pocketbitcoin", "strike", "21bitcoin", "coinfinity", "binance",
        ]
        let sourceFormats = [
            "wasabi_bundle", "ledgerlive_csv", "phoenix_csv", "bullbitcoin_wallet_csv",
            "btcpay_csv", "river_csv", "bullbitcoin_csv", "pocketbitcoin_csv",
            "strike_csv", "21bitcoin_csv", "coinfinity_csv", "binance_supplemental_csv",
            "generic_ledger", "csv",
        ]
        let daemon = ScriptedDaemonClient(scripts: [
            .uiConnectionsSources: [DaemonRecord(kind: "ui.connections.sources", data: [
                "wallet_kinds": .array(walletKinds.map { ["kind": .string($0)] }),
                "source_formats": .array(sourceFormats.map(JSONValue.string)),
            ])],
            .uiBackendsSettingsList: [DaemonRecord(kind: "ui.backends.settings.list", data: ["backends": []])],
            .uiBackendsOptions: [DaemonRecord(kind: "ui.backends.options", data: ["backends": []])],
            .uiBackendsPublicDefaults: [DaemonRecord(kind: "ui.backends.public_defaults", data: ["backends": []])],
            .uiWalletsList: [DaemonRecord(kind: "ui.wallets.list", data: ["wallets": []])],
            .uiOverviewSnapshot: [DaemonRecord(kind: "ui.overview.snapshot", data: ["connections": []])],
        ])
        let model = ConnectionsParityViewModel(daemon: daemon)

        await model.load()

        #expect(model.catalog.sources.count == 42)
        #expect(Set(model.catalog.sources.map(\.id)).count == model.catalog.sources.count)
        #expect(model.catalog.sources.first { $0.id == "address-list" }?.route == .addressList)
        #expect(model.catalog.sources.first { $0.id == "core-ln" }?.route == .backend("coreln"))
        #expect(model.catalog.sources.first { $0.id == "phoenix" }?.route == .fileImport("phoenix_csv"))
        #expect(model.catalog.sources.first { $0.id == "sparrow" }?.isEnabled == false)
        let readySourcesAreEnabled = model.catalog.sources
            .filter { $0.status == .ready }
            .allSatisfy { $0.isEnabled }
        #expect(readySourcesAreEnabled)
    }

    @Test("arbitrary two-column CSV cannot arm a specialized import")
    func rejectsArbitraryCSV() async throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("unsafe-preview-\(UUID().uuidString).csv")
        defer { try? FileManager.default.removeItem(at: file) }
        try Data("name,value\nalice,1\n".utf8).write(to: file)
        let model = ReportsImportsImportViewModel(daemon: ScriptedDaemonClient())
        model.format = .phoenix
        model.sourcePath = file.path

        await model.previewCurrentFile()

        #expect(!model.canRunImport)
        #expect(!model.previewConfident)
        #expect(model.previewMapped == 0)
        #expect(model.previewProblems == [.localized("importsParity.error.unrecognizedHeaders")])
        #expect(model.errorMessage == "importsParity.error.unsafePreview")
    }

    @Test("recognized specialized headers produce a current import preview")
    func acceptsRecognizedCSV() async throws {
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("phoenix-preview-\(UUID().uuidString).csv")
        defer { try? FileManager.default.removeItem(at: file) }
        try Data("date,id,type,amount_msat\n2026-01-01,payment-1,lightning_received,1000\n".utf8)
            .write(to: file)
        let model = ReportsImportsImportViewModel(daemon: ScriptedDaemonClient())
        model.format = .phoenix
        model.sourcePath = file.path

        await model.previewCurrentFile()

        #expect(model.canRunImport)
        #expect(model.previewConfident)
        #expect(model.previewMapped == 1)
        #expect(model.errorMessage == nil)
    }
}
