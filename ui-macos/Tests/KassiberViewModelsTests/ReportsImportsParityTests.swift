import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Reports, imports, and source-funds parity")
@MainActor
struct ReportsImportsParityTests {
    private func record(_ kind: DaemonKind, _ data: JSONValue = [:]) -> DaemonRecord {
        DaemonRecord(kind: kind.rawValue, data: data)
    }

    @Test("report reads use report-specific args and all handoff exports preserve their contracts")
    func reportContracts() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiWalletsList: [record(.uiWalletsList, ["wallets": [
                ["id": "wallet-a", "label": "Cold"],
                ["id": "wallet-b", "label": "Hot"],
            ]])],
            .uiSourceFundsCasesList: [record(.uiSourceFundsCasesList, ["cases": [
                ["id": "case-1", "label": "Auditor", "status": "ready"],
            ]])],
            .uiReportsTaxSummary: [record(.uiReportsTaxSummary, [
                "year": 2025,
                "jurisdiction_code": "AT",
                "method": "moving_average_at",
                "rows": [["code": "174", "amount": .number(1250.0)]],
            ])],
            .uiReportsLightningProfitability: [record(.uiReportsLightningProfitability, [
                "channels": [["id": "chan-1", "label": "Peer", "net_profit_sat": 42]],
            ])],
            .uiReportsExportSummaryPdf: [record(.uiReportsExportSummaryPdf, ["file": "/tmp/summary.pdf", "format": "pdf"])],
            .uiReportsExportAuditPackage: [record(.uiReportsExportAuditPackage, ["dir": "/tmp/audit", "format": "directory"])],
            .uiReportsExportAustrianE1kvXlsx: [record(.uiReportsExportAustrianE1kvXlsx, ["file": "/tmp/e1kv.xlsx", "format": "xlsx"])],
            .uiTransactionsExportCsv: [record(.uiTransactionsExportCsv, ["file": "/tmp/tx.csv", "format": "csv"])],
        ])
        let model = ReportsImportsReportsViewModel(daemon: client)
        model.selectedYear = 2025
        await model.loadContext()
        model.auditSourceFundsCaseID = "case-1"

        model.selection = .taxSummary
        await model.load()
        model.selection = .lightningProfitability
        model.lightningConnection = "node-1"
        await model.load()
        await model.export(.summaryPDF)
        await model.export(.auditPackage)
        await model.export(.austrianXLSX)
        model.exportWallet = "wallet-a"
        await model.export(.transactionsCSV)

        let calls = await client.calls()
        #expect(calls.contains { $0.kind == .uiReportsTaxSummary && $0.args == ["year": 2025] })
        #expect(calls.contains { $0.kind == .uiReportsLightningProfitability && $0.args == ["connection": "node-1"] })
        let summary = try #require(calls.first { $0.kind == .uiReportsExportSummaryPdf }?.args)
        #expect(summary["start"] == "2025-01-01T00:00:00Z")
        #expect(summary["end"] == "2025-12-31T23:59:59Z")
        #expect(summary["include_snapshot"] == true)
        #expect(summary["wallets"] == ["wallet-a", "wallet-b"])
        let audit = try #require(calls.first { $0.kind == .uiReportsExportAuditPackage }?.args)
        #expect(audit["source_funds_case"] == "case-1")
        #expect(audit["include_copied_attachments"] == true)
        #expect(audit["include_edit_history"] == false)
        #expect(calls.contains { $0.kind == .uiReportsExportAustrianE1kvXlsx && $0.args == ["year": 2025] })
        #expect(calls.contains { $0.kind == .uiTransactionsExportCsv && $0.args == ["wallet": "wallet-a"] })
        #expect(model.artifact?.sourceURL.path == "/tmp/tx.csv")
    }

    @Test("capital-gains package preserves typed lots, filing fields, swap audit, and readiness")
    func capitalGainsPackage() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiReportsCapitalGains: [record(.uiReportsCapitalGains, [
                "jurisdictionCode": "AT",
                "year": 2025,
                "availableYears": [.integer(2025), .integer(2024)],
                "method": "moving_average_at",
                "lots": [[
                    "acquired": "2024-02-01", "disposed": "2025-03-01",
                    "sats": .integer(125_000),
                    "costEur": .number(100.0),
                    "proceedsEur": .number(175.0),
                    "type": "LT", "extra_a": "one", "extra_b": "two",
                    "extra_c": "three", "extra_d": "four", "extra_e": "five",
                ]],
                "kennzahlRows": [[
                    "code": "174", "label": "Foreign realized crypto gains",
                    "form": "E 1kv", "formSection": "Capital assets",
                    "amount": .number(75.0), "rowCount": .integer(1), "source": "daemon",
                ]],
                "neutralSwapLots": [[
                    "pairId": "pair-1", "date": "2025-05-01", "kind": "chain_swap",
                    "policy": "carrying_value", "outWallet": "BTC", "outAsset": "BTC",
                    "outSats": .integer(100_000), "inWallet": "Liquid", "inAsset": "LBTC",
                    "inSats": .integer(99_500), "feeSats": .integer(500), "feeKind": "network",
                    "costEur": .number(80.0), "proceedsEur": .number(80.0),
                    "gainEur": .number(0.0), "marketValueEur": .number(82.0),
                    "marketDeltaEur": .number(2.0),
                ]],
                "status": ["needsJournals": .bool(false), "quarantines": .integer(2)],
            ])],
        ])
        let model = ReportsImportsReportsViewModel(daemon: client)
        model.selectedYear = 2025
        await model.load()

        #expect(model.capitalLots.first?.sats == 125_000)
        #expect(model.capitalLots.first?.gain == 75)
        #expect(model.kennzahlRows.first?.formSection == "Capital assets")
        #expect(model.neutralSwapLots.first?.marketDelta == 2)
        #expect(model.readiness.tone == .blocked)
        #expect(model.readiness.action == .quarantine)
        #expect(model.readiness.count == 2)
        #expect(model.rows.first?.fields.contains { $0.key == "extra e" && $0.value == "five" } == true)
        #expect(model.rows.first?.detail.contains("extra e: five") == true)
    }

    @Test("source-funds inventory and preview cover evidence, recipients, coverage, and disclosure options")
    func sourceFundsInventoryAndPreview() async throws {
        let scripts = sourceFundsInventoryScripts().merging([
            .uiSourceFundsPreview: [record(.uiSourceFundsPreview, [
                "target": ["label": "Exchange sale"],
                "narrative": ["paragraphs": ["Funds trace to a reviewed purchase."]],
                "findings": [["code": "ok", "severity": "info", "message": "Trace complete"]],
                "explain_gates": ["exportable": true, "blockers": [], "warnings": [["code": "review"]]],
            ])],
        ]) { _, new in new }
        let client = ScriptedDaemonClient(scripts: scripts)
        let model = ReportsImportsSourceFundsViewModel(daemon: client)
        await model.loadInventory()
        #expect(model.sources.first?.label == "Salary")
        #expect(model.links.first?.attachmentCount == 1)
        #expect(model.evidence.first?.label == "Payslip")
        #expect(model.recipients.first?.label == "Bank")
        #expect(model.coverage.first?.fullyTraced == 0.75)
        #expect(model.transactions.first?.id == "tx-target")
        #expect(model.targetTransaction == "tx-target")
        model.transactionFlow = "incoming"
        #expect(model.filteredTransactions.count == 1)

        model.targetTransaction = "tx-target"
        model.targetAmount = "0.75"
        model.reportPurpose = "planned_exchange_sale"
        model.plannedDestination = "Kraken"
        model.plannedNote = "OTC sale"
        model.selectedRecipientID = "recipient-1"
        model.diagramDetail = "detailed"
        model.amountPrecision = "sats"
        model.maskRecipient = true
        await model.preview()

        let call = try #require((await client.calls()).last { $0.kind == .uiSourceFundsPreview })
        #expect(call.args?["target_transaction"] == "tx-target")
        #expect(call.args?["target_amount"] == "0.75")
        #expect(call.args?["planned_destination"] == "Kraken")
        #expect(call.args?["recipient"] == "recipient-1")
        #expect(call.args?["report_options"]?["diagram_detail"] == "detailed")
        #expect(call.args?["report_options"]?["amount_precision"] == "sats")
        #expect(call.args?["report_options"]?["mask_recipient"] == true)
        #expect(model.exportable)
        #expect(model.warnings == 1)
    }

    @Test("source-funds mutations send source, link, review, attachment, recipient, case, and export args")
    func sourceFundsMutations() async throws {
        var scripts = sourceFundsInventoryScripts()
        scripts[.uiSourceFundsSourcesCreate] = [record(.uiSourceFundsSourcesCreate, ["id": "source-new", "label": "Gift"])]
        scripts[.uiSourceFundsSourcesAttach] = [record(.uiSourceFundsSourcesAttach, ["id": "source-new"])]
        scripts[.uiSourceFundsLinksCreate] = [record(.uiSourceFundsLinksCreate, ["id": "link-new"])]
        scripts[.uiSourceFundsLinksReview] = [record(.uiSourceFundsLinksReview, ["id": "link-1", "state": "reviewed"])]
        scripts[.uiSourceFundsLinksAttach] = [record(.uiSourceFundsLinksAttach, ["id": "link-1"])]
        scripts[.uiSourceFundsSuggest] = [record(.uiSourceFundsSuggest, ["inserted": 2])]
        scripts[.uiSourceFundsAssemble] = [record(.uiSourceFundsAssemble, ["auto_reviewed": 3, "awaiting_manual_review": 1])]
        scripts[.uiSourceFundsLinksBulkReview] = [record(.uiSourceFundsLinksBulkReview, ["reviewed": 2, "skipped": 1])]
        scripts[.uiSourceFundsRecipientsCreate] = [record(.uiSourceFundsRecipientsCreate, ["id": "recipient-new", "label": "Auditor"])]
        scripts[.uiSourceFundsRecipientsUpdate] = [record(.uiSourceFundsRecipientsUpdate, ["id": "recipient-1"])]
        scripts[.uiSourceFundsRecipientsDelete] = [record(.uiSourceFundsRecipientsDelete, ["deleted": true])]
        scripts[.uiSourceFundsCasesSave] = [record(.uiSourceFundsCasesSave, [
            "case": ["id": "case-new", "status": "ready"],
            "target": ["label": "Target"],
            "findings": [],
            "explain_gates": ["exportable": true, "blockers": [], "warnings": []],
        ])]
        scripts[.uiSourceFundsExportBundle] = [record(.uiSourceFundsExportBundle, ["file": "/tmp/source-funds.zip", "filename": "source-funds.zip"])]
        scripts[.uiSourceFundsPreview] = [record(.uiSourceFundsPreview, [
            "target": ["label": "Target"], "findings": [],
            "explain_gates": ["exportable": true, "blockers": [], "warnings": []],
        ])]
        let client = ScriptedDaemonClient(scripts: scripts)
        let model = ReportsImportsSourceFundsViewModel(daemon: client)
        await model.loadInventory()

        model.sourceType = "gift"
        model.sourceLabel = "Gift"
        model.sourceAmount = "0.5"
        model.selectedSourceAttachmentID = "attachment-1"
        await model.createSource()
        await model.attachSelectedSource("source-1")

        model.linkFromSource = "source-1"
        model.linkToTransaction = "tx-target"
        model.linkAmount = "0.5"
        model.selectedLinkAttachmentID = "attachment-1"
        await model.createLink()
        model.selectLink("link-1")
        model.linkAmount = "0.5"
        await model.reviewSelectedLink(state: "reviewed")
        await model.attachSelectedLink()

        model.targetTransaction = "tx-target"
        await model.suggest()
        #expect(model.lastMutationSummary == .localized(
            "sourceFundsParity.status.suggestionsAdded %lld",
            [.integer(2)]
        ))
        await model.assemble()
        #expect(model.lastMutationSummary == .localized(
            "sourceFundsParity.status.assembled %lld %lld",
            [.integer(3), .integer(1)]
        ))
        await model.bulkReview()
        #expect(model.lastMutationSummary == .localized(
            "sourceFundsParity.status.bulkReviewed %lld %lld",
            [.integer(2), .integer(1)]
        ))

        model.recipientLabel = "Auditor"
        await model.createRecipient()
        model.recipientLabel = "Bank updated"
        await model.updateRecipient("recipient-1")
        #expect(model.lastMutationSummary == .localized("sourceFundsParity.status.recipientUpdated"))
        await model.deleteRecipient("recipient-1")
        #expect(model.lastMutationSummary == .localized("sourceFundsParity.status.recipientDeleted"))

        model.targetTransaction = "tx-target"
        await model.saveCase(label: "Proof")
        await model.export(.uiSourceFundsExportBundle)

        let calls = await client.calls()
        #expect(calls.contains { $0.kind == .uiSourceFundsSourcesCreate && $0.args?["attachment_id"] == "attachment-1" })
        #expect(calls.contains { $0.kind == .uiSourceFundsSourcesAttach && $0.args?["source"] == "source-1" })
        #expect(calls.contains { $0.kind == .uiSourceFundsLinksCreate && $0.args?["from_source"] == "source-1" })
        #expect(calls.contains { $0.kind == .uiSourceFundsLinksReview && $0.args?["allocation_policy"] == "explicit" })
        #expect(calls.contains { $0.kind == .uiSourceFundsLinksAttach && $0.args?["attachment_id"] == "attachment-1" })
        #expect(calls.contains { $0.kind == .uiSourceFundsRecipientsCreate && $0.args?["kind"] == "auditor" })
        #expect(calls.contains { $0.kind == .uiSourceFundsCasesSave && $0.args?["case_label"] == "Proof" })
        #expect(calls.contains { $0.kind == .uiSourceFundsExportBundle && $0.args?["case"] == "case-new" })
        #expect(model.artifact?.sourceURL.path == "/tmp/source-funds.zip")
    }

    @Test("generic ledger, BIP329, Samourai, Wasabi, and template flows preserve exact payloads")
    func importContracts() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiWalletsLedgerPreview: [record(.uiWalletsLedgerPreview, [
                "rows_read": 2, "mapped": 2, "errors": 0, "confident": true,
                "preview": [["occurred_at": "2025-01-01", "kind": "buy", "amount": "0.1", "asset": "BTC"]],
                "problems": [],
            ])],
            .uiMetadataBip329Preview: [record(.uiMetadataBip329Preview, [
                "records": 1,
                "counts": ["exact": 1, "ambiguous": 0],
                "rows": [["type": "tx", "ref": "abc", "label": "salary", "match_status": "exact"]],
            ])],
            .uiMetadataBip329Import: [record(.uiMetadataBip329Import, ["records": 1, "transaction_tags_added": 1])],
            .uiMetadataBip329Export: [record(.uiMetadataBip329Export, ["file": "/tmp/labels.jsonl", "filename": "labels.jsonl", "exported": 1])],
            .uiWalletsImportSamourai: [record(.uiWalletsImportSamourai, ["label": "Samourai", "children": [["label": "Postmix"]]])],
            .uiWalletsImportFile: [record(.uiWalletsImportFile, ["imported": 4, "skipped": 0])],
            .uiTransactionsLedgerTemplate: [record(.uiTransactionsLedgerTemplate, ["file": "/tmp/template.xlsx", "filename": "template.xlsx", "format": "xlsx"])],
        ])
        let model = ReportsImportsImportViewModel(daemon: client)
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("ledger.csv")
        try Data("date,amount\n2025-01-01,0.1\n".utf8).write(to: file)
        let bip329File = FileManager.default.temporaryDirectory
            .appendingPathComponent("labels-\(UUID().uuidString).jsonl")
        try Data(#"{"type":"tx","ref":"abc","label":"salary"}"#.utf8).write(to: bip329File)
        defer {
            try? FileManager.default.removeItem(at: file)
            try? FileManager.default.removeItem(at: bip329File)
        }

        await model.previewGenericLedger(url: file)
        #expect(model.previewMapped == 2)
        #expect(model.previewRows.first?.amount == "0.1 BTC")
        #expect(model.filePreviewSummary?.detail == .localized(
            "importsParity.preview.mappedErrors %lld %lld",
            [.integer(2), .integer(0)]
        ))

        model.bip329Path = bip329File.path
        await model.previewBip329()
        await model.importBip329()
        model.bip329ExportMode = "all"
        model.bip329ExportWallet = "Cold"
        await model.exportBip329()
        #expect(model.bip329Rows.first?.status == "exact")

        model.samouraiLabel = "Samourai"
        model.samouraiSourceSetJSON = #"{"network":"main","children":[],"xpubs":[{"section":"postmix","script_type":"p2wpkh","root_path":"m/84'/0'/2147483646'","xpub":"xpub-example"}]}"#
        await model.importSamourai()

        model.format = .wasabiBundle
        model.wallet = "Wasabi"
        model.wasabiHistoryJSON = #"[{"txid":"abc"}]"#
        model.wasabiCoinsJSON = #"[]"#
        model.wasabiWalletInfoJSON = #"{"walletName":"main"}"#
        await model.previewCurrentFile()
        #expect(model.canRunImport)
        #expect(model.filePreviewSummary?.filename == .localized("importsParity.preview.wasabiBundle"))
        #expect(model.filePreviewSummary?.detail == .localized(
            "importsParity.preview.validatedSections %lld",
            [.integer(3)]
        ))
        await model.runImport()
        await model.exportLedgerTemplate(format: "xlsx")

        let calls = await client.calls()
        let ledger = try #require(calls.first { $0.kind == .uiWalletsLedgerPreview })
        #expect(ledger.args?["filename"] == "ledger.csv")
        let encoded = try #require(ledger.args?["source_bytes_base64"]?.stringValue)
        #expect(!encoded.isEmpty)
        #expect(calls.contains { $0.kind == .uiMetadataBip329Preview && $0.args?["file"] == .string(bip329File.path) })
        #expect(calls.contains { $0.kind == .uiMetadataBip329Export && $0.args?["mode"] == "all" && $0.args?["wallet"] == "Cold" })
        #expect(calls.contains { $0.kind == .uiWalletsImportSamourai && $0.args?["source_set"]?["network"] == "main" })
        let wasabi = try #require(calls.first { $0.kind == .uiWalletsImportFile })
        #expect(wasabi.args?["source_format"] == "wasabi_bundle")
        #expect(wasabi.args?["source_bundle"]?["gethistory"]?.arrayValue?.count == 1)
        #expect(calls.contains { $0.kind == .uiTransactionsLedgerTemplate && $0.args == ["format": "xlsx"] })
        #expect(model.artifact?.sourceURL.path == "/tmp/template.xlsx")
    }

    @Test("every file format requires a current preview and invalidates it when inputs change")
    func previewGateInvalidation() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiWalletsLedgerPreview: [record(.uiWalletsLedgerPreview, [
                "rows_read": 1, "mapped": 1, "errors": 0, "confident": true,
                "preview": [], "problems": [],
            ])],
            .uiMetadataBip329Preview: [record(.uiMetadataBip329Preview, [
                "records": 1, "counts": ["exact": 1],
            ])],
        ])
        let model = ReportsImportsImportViewModel(daemon: client)
        let file = FileManager.default.temporaryDirectory
            .appendingPathComponent("imports-\(UUID().uuidString).csv")
        let labels = FileManager.default.temporaryDirectory
            .appendingPathComponent("labels-\(UUID().uuidString).jsonl")
        try Data("date,amount\n2026-01-01,1\n".utf8).write(to: file)
        try Data(#"{"type":"tx","ref":"abc","label":"salary"}"#.utf8).write(to: labels)
        defer {
            try? FileManager.default.removeItem(at: file)
            try? FileManager.default.removeItem(at: labels)
        }

        model.sourcePath = file.path
        model.format = .genericLedger
        #expect(!model.canRunImport)
        await model.previewCurrentFile()
        #expect(model.canRunImport)

        model.format = .genericCSV
        try Data("date,direction,asset,amount\n2026-01-01,inbound,BTC,1\n".utf8).write(to: file)
        await model.previewCurrentFile()
        #expect(model.canRunImport)

        try Data("date,direction,asset,amount\n2026-01-01,inbound,BTC,1\n2026-01-02,inbound,BTC,2\n".utf8).write(to: file)
        #expect(!model.canRunImport)

        model.format = .wasabiBundle
        model.wasabiHistoryJSON = #"[{"txid":"abc"}]"#
        await model.previewCurrentFile()
        #expect(model.canRunImport)
        model.wasabiHistoryJSON = #"[{"txid":"changed"}]"#
        #expect(!model.canRunImport)

        model.bip329Path = labels.path
        await model.previewBip329()
        #expect(model.canImportBip329)
        try Data(#"{"type":"tx","ref":"changed","label":"salary"}"#.utf8).write(to: labels)
        #expect(!model.canImportBip329)
    }

    private func sourceFundsInventoryScripts() -> [DaemonKind: [DaemonRecord]] {
        [
            .uiSourceFundsSourcesList: [record(.uiSourceFundsSourcesList, ["sources": [[
                "id": "source-1", "label": "Salary", "source_type": "income", "asset": "BTC", "amount": .number(1.0),
                "attachments": [["id": "attachment-1"]],
            ]]])],
            .uiSourceFundsLinksList: [record(.uiSourceFundsLinksList, ["links": [[
                "id": "link-1", "from_source_id": "source-1", "to_transaction_id": "tx-target",
                "link_type": "manual_source", "state": "suggested", "confidence": "strong", "method": "manual",
                "asset": "BTC", "allocation_amount": .number(0.75), "allocation_policy": "explicit",
                "attachments": [["id": "attachment-1"]],
            ]]])],
            .uiSourceFundsEvidenceList: [record(.uiSourceFundsEvidenceList, ["attachments": [[
                "id": "attachment-1", "label": "Payslip", "attachment_type": "file", "transaction_id": "tx-source", "wallet": "Cold",
            ]]])],
            .uiSourceFundsRecipientsList: [record(.uiSourceFundsRecipientsList, ["recipients": [[
                "id": "recipient-1", "label": "Bank", "kind": "bank", "default_reveal_mode": "minimal", "active": true,
            ]]])],
            .uiSourceFundsCasesList: [record(.uiSourceFundsCasesList, ["cases": [[
                "id": "case-1", "label": "Proof", "target_external_id": "tx-target", "status": "ready",
            ]]])],
            .uiSourceFundsCoverage: [record(.uiSourceFundsCoverage, ["by_wallet": [[
                "wallet_id": "wallet-1", "wallet_label": "Cold", "total_inbound": .number(1.0),
                "buckets": [
                    "fully_traced": ["amount": .number(0.75)], "attested": ["amount": .number(0.1)],
                    "in_review": ["amount": .number(0.05)], "untraced": ["amount": .number(0.1)],
                ],
            ]]])],
            .uiTransactionsList: [record(.uiTransactionsList, ["txs": [[
                "id": "tx-target", "occurred_at": "2026-01-02T12:00:00Z", "type": "Receive",
                "asset": "BTC", "wallet": "Cold", "amountSat": 75_000_000, "confirmations": 6,
            ]]])],
        ]
    }
}
