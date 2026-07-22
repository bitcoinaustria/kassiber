import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Transaction detail frontend parity")
@MainActor
struct TransactionDetailParityTests {
    @Test("transaction rows retain pricing provenance and linked-pair metadata")
    func parsesDetailMetadata() {
        let transaction = makeTransaction([
            "pricingExternalRef": "Invoice 42",
            "pricingProvider": "kraken-csv",
            "pricingPair": "BTCEUR",
            "pricingTimestamp": "2026-07-10T00:00:00Z",
            "pricingFetchedAt": "2026-07-10T00:05:00Z",
            "pricingGranularity": "daily",
            "pricingMethod": "ohlcvt_csv",
            "quarantineReason": "insufficient_lots",
            "pair": [
                "id": "pair-1", "type": "transfer", "kind": "manual",
                "policy": "carrying-value", "outWallet": "Hot", "outAsset": "BTC",
                "outAmountSat": 10_000_000, "inWallet": "Cold", "inAsset": "BTC",
                "inAmountSat": 9_990_000, "feeSat": 10_000, "feeKind": "network",
            ],
        ])

        #expect(transaction.pricingExternalRef == "Invoice 42")
        #expect(transaction.pricingProvider == "kraken-csv")
        #expect(transaction.pricingPair == "BTCEUR")
        #expect(transaction.pricingGranularity == "daily")
        #expect(transaction.quarantineReason == "insufficient_lots")
        #expect(transaction.pair?.id == "pair-1")
        #expect(transaction.pair?.inAmountSats == 9_990_000)
    }

    @Test("classification, pricing, and Austrian tax edits use the daemon metadata contract")
    func savesCompleteMetadataDraft() async {
        let daemon = detailDaemon()
        let model = TransactionDetailViewModel(daemon: daemon, transaction: makeTransaction())

        model.classification = "Expense"
        model.tags = "accountant, Meals"
        model.note = "Receipt matched"
        model.reviewStatus = "review"
        model.selectPricing("manual_override")
        model.updateManualPrice("65000.00")
        model.manualCurrency = "eur"
        model.manualSource = "Invoice 42"
        model.selectTaxTreatment("outside:none")
        model.excluded = true

        #expect(model.manualValue == "6500.00")
        await model.save()

        let call = await daemon.calls().last { $0.kind == .uiTransactionsMetadataUpdate }
        #expect(call?.args?["transaction"] == .string("tx-1"))
        #expect(call?.args?["note"] == .string("Receipt matched"))
        #expect(call?.args?["tags"] == .array([.string("Expense"), .string("accountant"), .string("Meals")]))
        #expect(call?.args?["review_status"] == .string("review"))
        #expect(call?.args?["taxable"] == .bool(false))
        #expect(call?.args?["at_regime"] == .string("outside"))
        #expect(call?.args?["at_category"] == .string("none"))
        #expect(call?.args?["pricing_source_kind"] == .string("manual_override"))
        #expect(call?.args?["pricing_quality"] == .string("exact"))
        #expect(call?.args?["fiat_currency"] == .string("EUR"))
        #expect(call?.args?["fiat_rate"] == .string("65000.00"))
        #expect(call?.args?["fiat_value"] == .string("6500.00"))
        #expect(call?.args?["pricing_external_ref"] == .string("Invoice 42"))
        #expect(model.didSave)
        #expect(!model.hasChanges)
    }

    @Test("missing pricing can explicitly clear every persisted pricing field")
    func clearsPricing() async {
        let daemon = detailDaemon()
        let model = TransactionDetailViewModel(daemon: daemon, transaction: makeTransaction())
        model.selectPricing("missing")
        model.manualCurrency = ""
        model.manualPrice = ""
        model.manualValue = ""
        model.manualSource = ""

        await model.save()

        let args = await daemon.calls().last { $0.kind == .uiTransactionsMetadataUpdate }?.args
        #expect(args?["pricing_source_kind"] == .null)
        #expect(args?["pricing_quality"] == .string("missing"))
        #expect(args?["fiat_currency"] == .null)
        #expect(args?["fiat_rate"] == .null)
        #expect(args?["fiat_value"] == .null)
        #expect(args?["pricing_external_ref"] == .null)
    }

    @Test("linked tab loads transaction journals, summarizes tax, and unpairs by pair id")
    func loadsLinkedWorkflow() async {
        let daemon = detailDaemon(
            pair: true,
            journalEvents: [[
                "id": "journal-1", "entryType": "disposal", "asset": "BTC",
                "quantity": .number(-0.1), "fiatValueEur": .number(7_000),
                "costBasisEur": .number(5_000), "proceedsEur": .number(7_000),
                "gainLossEur": .number(2_000), "atCategory": "neu_gain",
                "description": "Sale",
            ]]
        )
        let model = TransactionDetailViewModel(
            daemon: daemon,
            transaction: makeTransaction([
                "pair": [
                    "id": "pair-1", "type": "transfer", "kind": "manual",
                    "outWallet": "Hot", "outAmountSat": 10_000_000,
                    "inWallet": "Cold", "inAmountSat": 9_990_000,
                ],
            ])
        )

        await model.load()

        #expect(model.journalEvents.count == 1)
        #expect(model.taxEffect.state == .disposal)
        #expect(model.taxEffect.costBasis == 5_000)
        #expect(model.taxEffect.proceeds == 7_000)
        #expect(model.taxEffect.gainLoss == 2_000)
        let journalCall = await daemon.calls().last { $0.kind == .uiJournalsEventsList }
        #expect(journalCall?.args?["transaction"] == .string("tx-1"))
        #expect(journalCall?.args?["limit"] == .integer(20))

        await model.unpair()
        let unpairCall = await daemon.calls().last { $0.kind == .uiTransfersUnpair }
        #expect(unpairCall?.args?["pair_id"] == .string("pair-1"))
        #expect(model.pair == nil)
    }

    @Test("details load a typed flow graph, swap route, warnings, annotations, and privacy context")
    func loadsFlowAndPrivacyContext() async {
        let graph: JSONValue = [
            "transaction": [
                "id": "tx-1", "txid": "external-1", "asset": "BTC",
                "network": "bitcoin", "inputCount": 1, "outputCount": 2,
                "version": 2, "vsize": 141, "feeRateSatVb": .number(3.5),
            ],
            "supportLevel": "full",
            "warnings": [["code": "ownership_index", "level": "info", "message": "Ownership is locally derived."]],
            "inputs": [[
                "id": "in-1", "outpoint": "parent:0", "valueSats": 10_001_000,
                "ownership": "owned", "role": "input", "scriptType": "v0_p2wpkh",
                "annotations": [["code": "common_input", "label": "Common input ownership"]],
            ]],
            "outputs": [[
                "id": "out-1", "address": "bc1qrecipient", "valueSats": 10_000_000,
                "ownership": "external", "role": "external_recipient",
            ]],
            "fee": ["id": "fee", "valueSats": 1_000, "ownership": "network_fee", "role": "fee"],
            "annotations": [["code": "linked_pair", "label": "Paired movement", "groupId": "group-1"]],
            "accounting": [
                "quarantine": ["reason": "missing_cost_basis"],
                "linkedPairs": [["code": "manual", "label": "Manual pair"]],
                "transferGroupIds": ["group-1"],
            ],
            "swapRoute": [
                "id": "route-1", "kind": "submarine_swap", "routeKind": "swap",
                "policy": "carrying-value", "currentLeg": "out", "swapFeeMsat": 25_000,
                "out": [
                    "id": "tx-1", "txid": "external-1", "role": "spend",
                    "asset": "BTC", "network": "Bitcoin", "amountBtc": .number(0.1),
                    "wallet": ["label": "Cold"],
                ],
                "in": [
                    "id": "liquid-1", "role": "receive", "asset": "LBTC",
                    "network": "Liquid", "amountBtc": .number(0.099),
                    "wallet": ["label": "Liquid"],
                ],
            ],
        ]
        let privacy: JSONValue = [
            "transaction_view": [[
                "txid": "external-1", "tell_count": 3, "wallet_penalty_count": 1,
                "tell_kinds": ["address_reuse", "round_amount"], "evidence_level": "exact",
            ]],
        ]
        let daemon = detailDaemon(graph: graph, privacy: privacy)
        let model = TransactionDetailViewModel(daemon: daemon, transaction: makeTransaction())

        await model.load()

        #expect(model.graphSnapshot.supportLevel == "full")
        #expect(model.graphSnapshot.inputs.first?.outpoint == "parent:0")
        #expect(model.graphSnapshot.outputs.first?.ownership == "external")
        #expect(model.graphSnapshot.fee?.valueSats == 1_000)
        #expect(model.graphSnapshot.swapRoute?.incoming.asset == "LBTC")
        #expect(model.graphSnapshot.swapRoute?.feeSats == 25)
        #expect(model.graphSnapshot.quarantineReason == "missing_cost_basis")
        #expect(model.graphSnapshot.transferGroupIDs == ["group-1"])
        #expect(model.privacyContext.matchedTransactionID == "external-1")
        #expect(model.privacyContext.tellCount == 3)
        #expect(model.privacyContext.walletPenaltyCount == 1)
        #expect(model.privacyContext.tellKinds == ["address_reuse", "round_amount"])
        #expect(!model.privacyContext.degraded)

        let privacyCall = await daemon.calls().last { $0.kind == .uiReportsPrivacyMirror }
        #expect(privacyCall?.args == nil)

        await model.selectGraphRouteLeg("in")
        let routeCall = await daemon.calls().last { $0.kind == .uiTransactionsGraph }
        #expect(routeCall?.args?["transaction"] == .string("liquid-1"))
        #expect(routeCall?.args?["allowPublicLookup"] == .bool(false))
        #expect(model.selectedGraphRouteLeg == "in")
        #expect(!model.graphIsLoading)
    }

    @Test("paired transactions retain a useful fallback route when the graph is graphless")
    func buildsPairFallbackRoute() async {
        let transaction = makeTransaction([
            "amountSat": -10_000_000,
            "pair": [
                "id": "pair-1", "type": "transfer", "kind": "peg-out",
                "policy": "carrying-value", "outWallet": "Cold", "outAsset": "BTC",
                "outAmountSat": 10_000_000, "inWallet": "Liquid", "inAsset": "LBTC",
                "inAmountSat": 9_990_000, "feeSat": 10_000,
            ],
        ])
        let snapshot = TransactionGraphSnapshot(
            ["supportLevel": "graphless"], pairFallback: transaction.pair, transaction: transaction
        )

        #expect(snapshot.swapRoute?.routeKind == "swap")
        #expect(snapshot.swapRoute?.out.network == "Bitcoin")
        #expect(snapshot.swapRoute?.incoming.network == "Liquid")
        #expect(snapshot.swapRoute?.currentLeg == "out")
        #expect(snapshot.hasFlowEvidence)
    }

    @Test("new transaction drafts mirror the frontend-only flow and pricing calculations")
    func calculatesNewTransactionDraft() {
        let instant = Date(timeIntervalSince1970: 1_700_000_000)
        var draft = NewTransactionDraft(now: instant, wallet: "Cold")

        draft.updateAmount("10,000,000")
        draft.updatePrice("65000")
        #expect(draft.totalValue == "6500.00")
        #expect(draft.signedBTC == 0.1)
        #expect(draft.fromDisplay == "External")
        #expect(draft.toDisplay == "Cold")

        draft.selectFlow("swap")
        draft.updateAmount("10_000_000", field: "send")
        draft.updateAmount("9 900 000", field: "receive")
        #expect(draft.isTwoLeg)
        #expect(draft.classification == "Swap")
        #expect(draft.taxTreatment == "neu:neu_swap")
        #expect(abs(draft.signedBTC - -0.001) < 0.000_000_001)

        draft.selectNetwork("Liquid")
        #expect(draft.asset == "LBTC")
        #expect(draft.receiveAsset == "LBTC")
        #expect(draft.occurredAt == instant)
    }

    private func makeTransaction(_ overrides: [String: JSONValue] = [:]) -> TransactionRow {
        var row: [String: JSONValue] = [
            "id": "tx-1",
            "externalId": "external-1",
            "occurredAt": "2026-07-10T12:00:00Z",
            "type": "Income",
            "asset": "BTC",
            "account": "Cold",
            "counter": "Client",
            "amountSat": 10_000_000,
            "feeSat": 500,
            "eur": .number(6_000),
            "rate": .number(60_000),
            "fiatCurrency": "EUR",
            "pricingSourceKind": "generic_import",
            "pricingQuality": "exact",
            "reviewStatus": "completed",
            "taxable": true,
            "atRegime": "neu",
            "atCategory": "income_general",
            "tags": ["Income", "accountant"],
            "note": "",
            "excluded": false,
            "chain": "bitcoin",
            "network": "mainnet",
        ]
        row.merge(overrides) { _, replacement in replacement }
        return TransactionRow(.object(row))!
    }

    private func detailDaemon(
        pair: Bool = false,
        journalEvents: [JSONValue] = [],
        graph: JSONValue = [:],
        privacy: JSONValue = ["transaction_view": []]
    ) -> ScriptedDaemonClient {
        ScriptedDaemonClient(scripts: [
            .uiTransactionsMetadataUpdate: [DaemonRecord(kind: "ui.transactions.metadata.update", data: ["updated": true])],
            .uiAttachmentsList: [DaemonRecord(kind: "ui.attachments.list", data: ["attachments": []])],
            .uiTransactionsHistory: [DaemonRecord(kind: "ui.transactions.history", data: ["events": []])],
            .uiTransactionsGraph: [
                DaemonRecord(kind: "ui.transactions.graph", data: graph),
                DaemonRecord(kind: "ui.transactions.graph", data: graph),
            ],
            .uiReportsPrivacyMirror: [DaemonRecord(kind: "ui.reports.privacy_mirror", data: privacy)],
            .uiLoansList: [DaemonRecord(kind: "ui.loans.list", data: ["marks": []])],
            .uiTransactionsCommercialContext: [DaemonRecord(kind: "ui.transactions.commercial_context", data: [:])],
            .uiJournalsEventsList: [DaemonRecord(kind: "ui.journals.events.list", data: [
                "summary": ["needsJournals": false], "events": .array(journalEvents),
            ])],
            .uiTransfersUnpair: [DaemonRecord(kind: "ui.transfers.unpair", data: ["unpaired": .bool(pair)])],
        ])
    }
}
