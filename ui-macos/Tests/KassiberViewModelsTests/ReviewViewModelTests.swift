import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Review view models")
@MainActor
struct ReviewViewModelTests {
    @Test("journals combine state and event ledger")
    func journals() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiJournalsSnapshot: [DaemonRecord(kind: "ui.journals.snapshot", data: [
                "status": [
                    "workspace": "Books", "profile": "AT", "transactionCount": 3,
                    "journalEntryCount": 2, "needsJournals": false, "quarantines": 1,
                ],
                "entryTypes": [["type": "disposal", "count": 2, "gainLossEur": .number(42)]],
            ])],
            .uiJournalsEventsList: [DaemonRecord(kind: "ui.journals.events.list", data: [
                "events": [[
                    "id": "je1", "transactionId": "tx1", "occurredAt": "2026-01-01T00:00:00Z",
                    "entryType": "disposal", "wallet": "Cold", "asset": "BTC",
                    "quantityMsat": -100_000, "fiatValueEur": .number(10),
                    "gainLossEur": .number(4), "description": "Sale",
                ]],
            ])],
        ])
        let model = JournalsViewModel(daemon: daemon)
        await model.load()
        #expect(model.entryCount == 2)
        #expect(model.entryTypes.first?.type == "disposal")
        #expect(model.entries.first?.gainLoss == 4)
    }

    @Test("quarantine filters by daemon reason")
    func quarantine() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiJournalsQuarantine: [DaemonRecord(kind: "ui.journals.quarantine", data: [
                "summary": ["by_reason": [["reason": "missing_price", "count": 1]]],
                "items": [[
                    "transaction_id": "tx1", "external_id": "external", "occurred_at": "2026-01-01T00:00:00Z",
                    "wallet": "Cold", "direction": "inbound", "asset": "BTC",
                    "amount_msat": 100_000, "fee_msat": 0, "reason": "missing_price",
                    "detail": ["field": "fiat_rate"],
                ]],
            ])],
        ])
        let model = QuarantineViewModel(daemon: daemon)
        await model.load()
        model.selectedReason = "missing_price"
        #expect(model.visibleItems.map(\.id) == ["tx1"])
        #expect(model.visibleItems.first?.detail == "field: fiat_rate")
    }

    @Test("swap queue and persisted pairs stay separate")
    func swaps() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiTransfersSuggest: [DaemonRecord(kind: "ui.transfers.suggest", data: [
                "counts": ["exact": 1, "conflicts": 0],
                "candidates": [[
                    "out_id": "out", "in_id": "in", "out_wallet": "LN", "in_wallet": "Liquid",
                    "out_asset": "BTC", "in_asset": "LBTC", "out_amount_msat": 100_000,
                    "in_amount_msat": 99_000, "swap_fee_msat": 1_000, "confidence": "exact",
                    "method": "payment_hash", "default_kind": "submarine-swap",
                    "default_policy": "carrying-value", "conflict_size": 1,
                ]],
            ])],
            .uiTransfersList: [DaemonRecord(kind: "ui.transfers.list", data: [
                "pairs": [[
                    "id": "pair", "kind": "manual", "policy": "carrying-value",
                    "swap_fee_msat": 1_000, "pair_source": "manual",
                    "out": ["wallet": "Hot", "asset": "BTC", "amount_msat": 100_000],
                    "in": ["wallet": "Cold", "asset": "BTC", "amount_msat": 99_000],
                ]],
            ])],
            .uiTransfersRulesList: [DaemonRecord(kind: "ui.transfers.rules.list", data: ["rules": []])],
            .uiSavedViewsList: [DaemonRecord(kind: "ui.saved_views.list", data: ["views": []])],
        ])
        let model = SwapsViewModel(daemon: daemon)
        await model.load()
        #expect(model.candidates.first?.kind == "submarine-swap")
        #expect(model.pairs.first?.source == "manual")
        #expect(model.exactCount == 1)
        if let pair = model.pairs.first {
            await model.update(pair, kind: "chain-swap", policy: "taxable")
        }
        let update = await daemon.calls().last { $0.kind == .uiTransfersUpdate }
        #expect(update?.args?["pair_id"] == .string("pair"))
        #expect(update?.args?["kind"] == .string("chain-swap"))
        #expect(update?.args?["policy"] == .string("taxable"))
    }

    @Test("reconcile invokes cache-only identify with pasted text")
    func reconcile() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiWalletsIdentify: [DaemonRecord(kind: "ui.wallets.identify", data: [
                "summary": ["owned": 1, "external": 0, "unknown": 0, "invalid": 0, "wallets_scanned": 2],
                "results": [[
                    "input": "bc1qexample", "type": "address", "chain": "bitcoin",
                    "status": "owned", "classification": "owned_address", "note": "matched, locally",
                    "matches": [["wallet": "Cold", "branch": "receive", "address_index": 7]],
                ]],
            ])],
        ])
        let model = ReconcileViewModel(daemon: daemon)
        model.input = "bc1qexample"
        await model.check()
        #expect(model.owned == 1)
        #expect(model.results.first?.wallets == ["Cold"])
        #expect(model.results.first?.branch == "receive #7")
        #expect(model.resultsCSV.contains("\"matched, locally\""))
        #expect(model.resultsCSV.hasPrefix("input,type,chain,status,classification,wallet,branch,note"))
        let call = await daemon.calls().last
        #expect(call?.args?["text"] == .string("bc1qexample"))
    }
}
