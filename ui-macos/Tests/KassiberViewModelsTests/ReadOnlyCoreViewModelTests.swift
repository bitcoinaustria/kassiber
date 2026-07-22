import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Read-only core view models")
@MainActor
struct ReadOnlyCoreViewModelTests {
    @Test("dashboard maps daemon totals without client recomputation")
    func dashboard() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiOverviewSnapshot: [DaemonRecord(kind: "ui.overview.snapshot", data: [
                "status": [
                    "workspace": "Personal",
                    "profile": "Austria",
                    "transactionCount": 9,
                    "needsJournals": true,
                    "quarantines": 2,
                ],
                "fiat": [
                    "fiatCurrency": "EUR",
                    "eurBalance": .number(12_345.67),
                    "eurCostBasis": .number(8_000),
                    "eurUnrealized": .number(4_345.67),
                    "eurRealizedYTD": .number(200),
                ],
                "connections": [["id": "w1", "label": "Cold", "kind": "descriptor", "balance": .number(0.5)]],
                "txs": [Self.transaction(id: "t1", amount: 50_000)],
                "portfolioSeries": [[
                    "date": "2026-01-01",
                    "balanceBtc": .number(0.5),
                    "valueEur": .number(12_345.67),
                    "costBasisEur": .number(8_000),
                    "priceEur": .number(70_000),
                ]],
            ])],
        ])
        let model = DashboardViewModel(daemon: daemon)
        await model.load()
        #expect(model.bookLabel == "Austria")
        #expect(model.fiatBalance == 12_345.67)
        #expect(model.totalBTC == 0.5)
        #expect(model.needsJournals)
        #expect(model.quarantines == 2)
        #expect(model.recentTransactions.count == 1)
        #expect(model.activityTransactions.count == 1)
        #expect(model.portfolio.first?.costBasisEUR == 8_000)
        #expect(model.portfolio.first?.priceEUR == 70_000)
        #expect(model.holdings.first?.label == "Cold")
        #expect(model.drivers.first?.id == "incoming")
    }

    @Test("transactions apply client filters and preserve wallet server scope")
    func transactions() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiWalletsList: [DaemonRecord(kind: "ui.wallets.list", data: [
                "wallets": [["id": "w1", "label": "Cold"]],
            ])],
            .uiTransactionsList: [DaemonRecord(kind: "ui.transactions.list", data: [
                "txs": [
                    Self.transaction(id: "incoming", amount: 20_000, fee: 0, status: "completed"),
                    Self.transaction(id: "outgoing", amount: -10_000, fee: 200, status: "review"),
                ],
            ])],
        ])
        let model = TransactionsViewModel(daemon: daemon, walletScope: "w1")
        await model.load()
        model.flow = .outgoing
        model.feeOnly = true
        #expect(model.visibleRows.map(\.id) == ["outgoing"])
        let calls = await daemon.calls()
        let transactionCall = calls.first { $0.kind == .uiTransactionsList }
        #expect(transactionCall?.args?["wallet"] == .string("w1"))
    }

    @Test("transaction workbench filters stay in lockstep with daemon pagination")
    func transactionWorkbenchFilters() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiTransactionsList: [DaemonRecord(kind: "ui.transactions.list", data: [
                "txs": [
                    Self.transaction(id: "incoming", amount: 20_000, fee: 0, status: "completed"),
                    Self.transaction(id: "outgoing", amount: -10_000, fee: 200, status: "review"),
                ],
            ])],
        ])
        let model = TransactionsViewModel(daemon: daemon)
        model.period = .months3
        model.flow = .outgoing
        model.reviewStatus = "review"
        model.paymentMethod = .onChain
        model.feeOnly = true
        model.quickFilter = .reviewQueue
        model.sort = .amountAscending
        await model.reload()

        #expect(model.visibleRows.map(\.id) == ["outgoing"])
        #expect(model.metrics.reviewCount == 1)
        let call = await daemon.calls().last
        #expect(call?.args?["period"] == "3months")
        #expect(call?.args?["flow"] == "outgoing")
        #expect(call?.args?["status"] == "review")
        #expect(call?.args?["payment_method"] == "On-chain")
        #expect(call?.args?["with_fees"] == true)
        #expect(call?.args?["quick"] == "review_queue")
        #expect(call?.args?["sort"] == "amount")
        #expect(call?.args?["order"] == "asc")
    }

    @Test("transaction handoff resolves a precise row through the allowlisted read")
    func transactionHandoffResolution() async throws {
        let txid = String(repeating: "a", count: 64)
        let daemon = ScriptedDaemonClient(scripts: [
            .uiTransactionsResolve: [DaemonRecord(kind: "ui.transactions.resolve", data: [
                "transaction": [
                    "id": "internal-1", "externalId": .string(txid),
                    "account": "Treasury", "type": "deposit", "amountSat": 42,
                ],
            ])],
        ])
        let model = TransactionsViewModel(daemon: daemon)

        let resolved = try #require(await model.resolveTransaction(txid))

        #expect(resolved.id == "internal-1")
        #expect(resolved.transactionID == txid)
        let call = await daemon.calls().last
        #expect(call?.kind == .uiTransactionsResolve)
        #expect(call?.args?["query"] == .string(txid))
    }

    @Test("wallet detail combines safe wallet, transaction, UTXO, and history reads")
    func wallets() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiWalletsList: [DaemonRecord(kind: "ui.wallets.list", data: [
                "wallets": [[
                    "id": "w1", "label": "Cold", "kind": "descriptor",
                    "transaction_count": 1, "deprecated": false,
                ]],
            ])],
            .uiOverviewSnapshot: [DaemonRecord(kind: "ui.overview.snapshot", data: [
                "priceEur": .number(70_000),
                "connections": [["id": "w1", "label": "Cold", "balance": .number(0.25)]],
            ])],
            .uiTransactionsList: [DaemonRecord(kind: "ui.transactions.list", data: [
                "txs": [Self.transaction(id: "t1", amount: 25_000_000)],
            ])],
            .uiWalletsUtxos: [DaemonRecord(kind: "ui.wallets.utxos", data: [
                "support": ["supported": true],
                "utxos": [[
                    "id": "coin", "outpoint": "tx:0", "txid": "tx", "asset": "BTC",
                    "amount_sat": 25_000_000, "confirmation_status": "confirmed",
                    "confirmations": 3, "address_label": "receive #1",
                ]],
            ])],
            .uiReportsBalanceHistory: [DaemonRecord(kind: "ui.reports.balance_history", data: [
                "rows": [["date": "2026-01-01", "balance_sat": 25_000_000]],
            ])],
        ])
        let model = WalletsViewModel(daemon: daemon)
        await model.load()
        await model.loadSelectedWallet()
        #expect(model.selectedWallet?.balanceBTC == 0.25)
        #expect(model.priceEUR == 70_000)
        #expect(model.recentTransactions.count == 1)
        #expect(model.utxos.first?.amountSats == 25_000_000)
        #expect(model.history.first?.amountSats == 25_000_000)
    }

    @Test("reports parse capital-gains lots and available years")
    func reports() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiReportsCapitalGains: [DaemonRecord(kind: "ui.reports.capital_gains", data: [
                "jurisdictionCode": "AT",
                "year": 2025,
                "method": "moving_average_at",
                "availableYears": [2026, 2025],
                "lots": [[
                    "disposed": "2025-11-04", "acquired": "2022-03-18",
                    "sats": 12_000_000, "proceedsEur": .number(8_204.18),
                ]],
            ])],
        ])
        let model = ReportsViewModel(daemon: daemon)
        await model.load()
        #expect(model.availableYears == [2026, 2025])
        #expect(model.rows.first?.primary == "2025-11-04")
        #expect(model.rows.first?.quantitySats == 12_000_000)
        #expect(model.titleDetail.contains("AT"))
    }

    private static func transaction(
        id: String,
        amount: Int64,
        fee: Int64 = 0,
        status: String = "completed"
    ) -> JSONValue {
        [
            "id": .string(id),
            "date": "2026-01-01 12:00",
            "type": amount < 0 ? "Expense" : "Income",
            "account": "Cold",
            "counter": "Counterparty",
            "amountSat": .integer(amount),
            "feeSat": .integer(fee),
            "eur": .number(10),
            "reviewStatus": .string(status),
            "conf": 1,
        ]
    }
}
