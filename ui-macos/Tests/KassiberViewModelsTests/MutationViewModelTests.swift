import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Mutation view models")
@MainActor
struct MutationViewModelTests {
    @Test("Full rescan uses the global freshness pipeline")
    func fullRescanArguments() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiFreshnessRun: [
                DaemonRecord(kind: "ui.freshness.run.progress", data: [
                    "phase": "journal_refresh", "job_index": 1, "job_total": 1,
                ]),
                DaemonRecord(kind: "ui.freshness.run", data: ["completed": []]),
            ],
        ])
        let model = BookRefreshCoordinator(daemon: client)

        await model.run(.fullRescan)

        #expect(model.progress == 1)
        #expect(model.phase == "done")
        let call = try #require(await client.calls().first)
        #expect(call.kind == .uiFreshnessRun)
        #expect(call.args?["all"] == true)
        #expect(call.args?["journals"] == true)
        #expect(call.args?["auto_pair"] == true)
        #expect(call.args?["force_full"] == true)
    }

    @Test("Bare xpub detection feeds descriptor preview")
    func multiScriptPreview() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiWalletsDetectScriptTypes: [DaemonRecord(kind: "ui.wallets.detect_script_types", data: [
                "probed": true,
                "active": ["p2wpkh", "p2tr"],
            ])],
            .uiWalletsPreviewDescriptor: [DaemonRecord(kind: "ui.wallets.preview_descriptor", data: [
                "addresses": [[
                    "branch": "p2wpkh receive", "index": 0,
                    "address": "bcrt1qexample", "derivation_path": "m/0/0",
                ]],
            ])],
        ])
        let model = WalletMutationViewModel(daemon: client)
        model.walletMaterial = "xpub-test"

        await model.detectAndPreview()

        #expect(model.scriptTypes == ["p2wpkh", "p2tr"])
        #expect(model.preview.first?.address == "bcrt1qexample")
        let previewCall = try #require(await client.calls().last)
        #expect(previewCall.args?["script_types"] == ["p2tr", "p2wpkh"])
    }

    @Test("Import remains blocked until a confident clean preview")
    func importPreviewGate() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiWalletsLedgerPreview: [DaemonRecord(kind: "ui.wallets.ledger_preview", data: [
                "confident": false, "mapped": 3, "errors": 0, "preview": [],
            ])],
        ])
        let model = LedgerImportViewModel(daemon: client)
        let url = URL(fileURLWithPath: "/tmp/kassiber-native-import-test.csv")
        try "timestamp,amount\n2026-01-01,1".write(to: url, atomically: true, encoding: .utf8)

        await model.preview(url)

        #expect(model.canImport == false)
        #expect(model.mapped == 3)
    }
}
