import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Global search")
@MainActor
struct GlobalSearchViewModelTests {
    @Test("ranks localized pages and local connection data")
    func pagesAndConnections() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiOverviewSnapshot: [DaemonRecord(kind: "ui.overview.snapshot", data: [
                "connections": [["id": "cold", "label": "Cold Storage", "kind": "descriptor", "network": "main"]],
                "txs": [["id": "tx-1", "externalId": "abc123", "account": "Cold Storage", "counter": "Shop", "type": "expense"]],
            ])],
        ])
        let model = GlobalSearchViewModel(daemon: daemon)
        await model.load()
        model.query = "Cold"
        #expect(model.results.first?.destination == .connection("cold"))
        model.query = "Transaktionen"
        #expect(model.results.contains { $0.destination == .screen(.transactions) })
        model.aiFeaturesEnabled = false
        model.query = "assistant"
        #expect(model.results.allSatisfy { $0.destination != .screen(.assistant) })
    }

    @Test("indexes route-only sync and BTCPay setup, reports, settings, and review work")
    func completeStaticAndReviewIndex() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiOverviewSnapshot: [DaemonRecord(kind: "ui.overview.snapshot", data: [
                "status": ["needs_journals": true, "quarantines": 3],
                "connections": [], "txs": [],
            ])],
        ])
        let model = GlobalSearchViewModel(daemon: daemon)
        await model.load()

        model.query = "sync wallets"
        #expect(model.results.first(where: { $0.id == "action:sync-wallets" })?.destination == .screen(.connections))
        model.query = "btcpay api"
        #expect(model.results.contains { $0.destination == .action(.connectBTCPay) })
        model.query = "btcpay csv"
        #expect(model.results.contains { $0.destination == .action(.importBTCPay) })
        model.query = "export pdf"
        #expect(model.results.contains { $0.id == "action:export-report" && $0.destination == .screen(.reports) })
        model.query = "change passphrase"
        #expect(model.results.contains { $0.destination == .settings("security") })
        model.query = "replication"
        #expect(model.results.contains { $0.destination == .settings("replication") })
        model.query = "ledger stale"
        #expect(model.results.contains { $0.id == "review:journals" && $0.category == .reviewItem })
        model.query = "quarantine review"
        #expect(model.results.contains { $0.id == "review:quarantine" })
    }

    @Test("reload classifier accepts terminal mutations but rejects reads")
    func mutationReloadClassifier() {
        #expect(GlobalSearchViewModel.invalidatesIndex(DaemonRecord(
            kind: "native.request.activity", event: true,
            data: ["kind": "ui.profiles.switch", "state": "finished"]
        )))
        #expect(!GlobalSearchViewModel.invalidatesIndex(DaemonRecord(
            kind: "native.request.activity", event: true,
            data: ["kind": "ui.transactions.resolve", "state": "finished"]
        )))
        #expect(!GlobalSearchViewModel.invalidatesIndex(DaemonRecord(
            kind: "native.request.activity", event: true,
            data: ["kind": "ui.wallets.create", "state": "started"]
        )))
    }

    @Test("exact transaction lookup uses the allowlisted resolver")
    func exactLookup() async {
        let txid = String(repeating: "a", count: 64)
        let daemon = ScriptedDaemonClient(scripts: [
            .uiTransactionsResolve: [DaemonRecord(kind: "ui.transactions.resolve", data: [
                "query": .string(txid),
                "transaction": ["id": "internal-1", "externalId": .string(txid), "account": "Treasury"],
            ])],
        ])
        let model = GlobalSearchViewModel(daemon: daemon)
        model.query = txid
        await model.resolveExactTransactionIfNeeded()
        #expect(model.results.first?.destination == .transaction("internal-1"))
        let call = await daemon.calls().last
        #expect(call?.kind == .uiTransactionsResolve)
        #expect(call?.args?["query"] == .string(txid))
    }
}
