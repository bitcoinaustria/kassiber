import Foundation
import Observation
import KassiberDaemonKit

public struct WalletRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let kind: String
    public let account: String
    public let chain: String
    public let network: String
    public let transactionCount: Int
    public let syncStatus: String
    public let lastSyncedAt: Date?
    public let deprecated: Bool
    public var balanceBTC: Double?

    static func parseList(_ data: JSONValue?) -> [WalletRow] {
        guard let object = data?.objectValue else { return [] }
        return object.objects("wallets").compactMap { row in
            let label = row.string("label") ?? ""
            guard !label.isEmpty else { return nil }
            let account = row["account"]?.objectValue
            return WalletRow(
                id: row.string("id") ?? label,
                label: label,
                kind: row.string("kind") ?? "wallet",
                account: account?.string("label", "code") ?? "",
                chain: row.string("chain") ?? "",
                network: row.string("network") ?? "",
                transactionCount: Int(row.int("transaction_count", "transactionCount") ?? 0),
                syncStatus: row.string("sync_status", "syncStatus") ?? "",
                lastSyncedAt: DaemonValueParser.date(row.string("last_synced_at", "lastSyncedAt")),
                deprecated: row.bool("deprecated") ?? false,
                balanceBTC: nil
            )
        }
    }
}

public struct WalletUTXO: Identifiable, Equatable, Sendable {
    public let id: String
    public let transactionID: String
    public let outpoint: String
    public let asset: String
    public let amountSats: Int64
    public let status: String
    public let confirmations: Int
    public let addressLabel: String
}

public struct WalletHistoryPoint: Identifiable, Equatable, Sendable {
    public let id: String
    public let date: Date
    public let amountSats: Int64
}

@MainActor
@Observable
public final class WalletsViewModel {
    public private(set) var wallets: [WalletRow] = []
    public var selectedWalletID: String? {
        didSet {
            if oldValue != selectedWalletID { Task { await loadSelectedWallet() } }
        }
    }
    public private(set) var recentTransactions: [TransactionRow] = []
    public private(set) var utxos: [WalletUTXO] = []
    public private(set) var history: [WalletHistoryPoint] = []
    public private(set) var inventoryMessage: String?
    public private(set) var priceEUR: Double?
    public private(set) var isLoading = false
    public private(set) var detailLoading = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient

    public init(daemon: any DaemonClient) {
        self.daemon = daemon
    }

    public var selectedWallet: WalletRow? {
        wallets.first { $0.id == selectedWalletID }
    }

    public func load() async {
        isLoading = true
        defer { isLoading = false }
        do {
            async let walletEnvelope = daemon.invoke(.uiWalletsList, args: nil)
            async let overviewEnvelope = daemon.invoke(.uiOverviewSnapshot, args: nil)
            let (walletResult, overviewResult) = try await (walletEnvelope, overviewEnvelope)
            if let error = walletResult.error {
                errorMessage = error.message
                return
            }
            var parsed = WalletRow.parseList(walletResult.data)
            let overview = overviewResult.data?.objectValue ?? [:]
            priceEUR = overview.double("priceEur", "price_eur")
            let connectionRows = overview.objects("connections")
            let balances: [String: Double] = Dictionary(
                uniqueKeysWithValues: connectionRows.compactMap { row in
                    guard let key = row.string("id", "label"), let balance = row.double("balance") else {
                        return nil
                    }
                    return (key, balance)
                }
            )
            for index in parsed.indices {
                parsed[index].balanceBTC = balances[parsed[index].id] ?? balances[parsed[index].label]
            }
            wallets = parsed
            if selectedWalletID == nil { selectedWalletID = wallets.first?.id }
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    public func loadSelectedWallet() async {
        guard let wallet = selectedWallet else {
            recentTransactions = []
            utxos = []
            history = []
            return
        }
        detailLoading = true
        defer { detailLoading = false }
        let args: [String: JSONValue] = ["wallet": .string(wallet.id)]
        do {
            async let transactionEnvelope = daemon.invoke(
                .uiTransactionsList,
                args: ["wallet": .string(wallet.id), "limit": .integer(12)]
            )
            async let utxoEnvelope = daemon.invoke(.uiWalletsUtxos, args: args)
            async let historyEnvelope = daemon.invoke(
                .uiReportsBalanceHistory,
                args: ["wallet": .string(wallet.id), "interval": .string("month"), "limit": .integer(24)]
            )
            let (transactionsResult, utxosResult, historyResult) = try await (
                transactionEnvelope,
                utxoEnvelope,
                historyEnvelope
            )
            recentTransactions = (transactionsResult.data?.objectValue?["txs"]?.arrayValue ?? [])
                .compactMap(TransactionRow.init)
            parseInventory(utxosResult.data)
            parseHistory(historyResult.data)
            errorMessage = transactionsResult.error?.message
                ?? utxosResult.error?.message
                ?? historyResult.error?.message
        } catch {
            errorMessage = String(describing: error)
        }
    }

    private func parseInventory(_ data: JSONValue?) {
        guard let object = data?.objectValue else {
            utxos = []
            return
        }
        let support = object["support"]?.objectValue ?? [:]
        inventoryMessage = support.bool("supported") == false ? "wallet.inventoryUnsupported" : nil
        utxos = object.objects("utxos").compactMap { row in
            let id = row.string("id", "outpoint") ?? ""
            guard !id.isEmpty else { return nil }
            return WalletUTXO(
                id: id,
                transactionID: row.string("transaction_id", "txid") ?? "",
                outpoint: row.string("outpoint") ?? "",
                asset: row.string("asset") ?? "BTC",
                amountSats: row.int("amount_sat") ?? 0,
                status: row.string("confirmation_status") ?? "",
                confirmations: Int(row.int("confirmations") ?? 0),
                addressLabel: row.string("address_label") ?? ""
            )
        }
    }

    private func parseHistory(_ data: JSONValue?) {
        guard let object = data?.objectValue else {
            history = []
            return
        }
        history = object.objects("rows").compactMap { row in
            guard
                let rawDate = row.string("date", "bucket", "occurred_at"),
                let date = DaemonValueParser.date(rawDate)
            else { return nil }
            return WalletHistoryPoint(
                id: rawDate,
                date: date,
                amountSats: row.int("balance_sat", "quantity_sat", "amount_sat") ?? 0
            )
        }
    }
}
