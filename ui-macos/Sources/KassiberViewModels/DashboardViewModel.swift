import Foundation
import Observation
import KassiberDaemonKit

public struct DashboardConnection: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let kind: String
    public let balanceBTC: Double
    public let syncStatus: String
    public let lastSyncedAt: Date?
}

public struct DashboardPoint: Identifiable, Equatable, Sendable {
    public let id: String
    public let date: Date
    public let balanceBTC: Double
    public let fiatValue: Double
    public let costBasisEUR: Double
    public let priceEUR: Double?
}

public enum DashboardChartPeriod: String, CaseIterable, Identifiable, Sendable {
    case automatic = "auto"
    case days30 = "30days"
    case months3 = "3months"
    case months6 = "6months"
    case ytd
    case year1 = "1year"
    case years5 = "5years"
    case all
    public var id: String { rawValue }
}

public enum DashboardChartSeries: String, CaseIterable, Identifiable, Sendable {
    case balance
    case portfolioValue
    case costBasis
    case price
    case events
    public var id: String { rawValue }
}

public struct DashboardHolding: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let valueBTC: Double
    public let percentage: Double
}

public struct DashboardDriver: Identifiable, Equatable, Sendable {
    public let id: String
    public let valueBTC: Double
    public let count: Int
}

@MainActor
@Observable
public final class DashboardViewModel {
    public private(set) var bookLabel = ""
    public private(set) var workspaceLabel = ""
    public private(set) var totalBTC: Double?
    public private(set) var fiatBalance: Double?
    public private(set) var fiatCostBasis: Double?
    public private(set) var fiatUnrealized: Double?
    public private(set) var realizedYTD: Double?
    public private(set) var fiatCurrency = "EUR"
    public private(set) var transactionCount = 0
    public private(set) var needsJournals = false
    public private(set) var quarantines = 0
    public private(set) var connections: [DashboardConnection] = []
    public private(set) var recentTransactions: [TransactionRow] = []
    public private(set) var activityTransactions: [TransactionRow] = []
    public private(set) var portfolio: [DashboardPoint] = []
    public private(set) var holdings: [DashboardHolding] = []
    public private(set) var drivers: [DashboardDriver] = []
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?
    public private(set) var marketRatePair: String?
    public private(set) var marketRateValue: Double?

    private let daemon: any DaemonClient

    public init(daemon: any DaemonClient) {
        self.daemon = daemon
    }

    public func load() async {
        isLoading = true
        defer { isLoading = false }
        do {
            let envelope = try await daemon.invoke(.uiOverviewSnapshot, args: nil)
            if let error = envelope.error {
                errorMessage = error.message
                return
            }
            guard let data = envelope.data?.objectValue else {
                errorMessage = "The overview response had an unexpected format."
                return
            }
            let status = data["status"]?.objectValue ?? [:]
            bookLabel = status.string("profile") ?? ""
            workspaceLabel = status.string("workspace") ?? ""
            transactionCount = Int(status.int("transactionCount", "transaction_count") ?? 0)
            needsJournals = status.bool("needsJournals", "needs_journals") ?? false
            quarantines = Int(status.int("quarantines") ?? 0)

            let fiat = data["fiat"]?.objectValue ?? [:]
            fiatCurrency = fiat.string("fiatCurrency", "fiat_currency") ?? "EUR"
            fiatBalance = fiat.double("eurBalance", "fiat_balance")
            fiatCostBasis = fiat.double("eurCostBasis", "fiat_cost_basis")
            fiatUnrealized = fiat.double("eurUnrealized", "fiat_unrealized")
            realizedYTD = fiat.double("eurRealizedYTD", "fiat_realized_ytd")
            marketRatePair = data["marketRate"]?.objectValue?.string("pair")
            marketRateValue = data["marketRate"]?.objectValue?.double("rate") ?? data.double("priceEur", "price_eur")

            connections = data.objects("connections").compactMap { row in
                let label = row.string("label") ?? ""
                guard !label.isEmpty else { return nil }
                return DashboardConnection(
                    id: row.string("id") ?? label,
                    label: label,
                    kind: row.string("kind") ?? "wallet",
                    balanceBTC: row.double("balance") ?? 0,
                    syncStatus: row.string("status", "sync_status") ?? "",
                    lastSyncedAt: DaemonValueParser.date(row.string("lastSyncAt", "last_synced_at"))
                )
            }
            let txValues = data["txs"]?.arrayValue ?? []
            let activityValues = data["activityTxs"]?.arrayValue ?? txValues
            activityTransactions = activityValues.compactMap(TransactionRow.init)
            recentTransactions = txValues
                .compactMap(TransactionRow.init)
                .prefix(8)
                .map { $0 }
            portfolio = data.objects("portfolioSeries", "portfolio_series").compactMap { row in
                guard
                    let dateString = row.string("date"),
                    let date = DaemonValueParser.date(dateString)
                else { return nil }
                return DashboardPoint(
                    id: dateString,
                    date: date,
                    balanceBTC: row.double("balanceBtc", "balance_btc") ?? 0,
                    fiatValue: row.double("valueEur", "fiat_value") ?? 0,
                    costBasisEUR: row.double("costBasisEur", "cost_basis_eur") ?? 0,
                    priceEUR: row.double("priceEur", "price_eur")
                )
            }
            if portfolio.isEmpty, let series = data["balanceSeries"]?.arrayValue {
                let calendar = Calendar(identifier: .gregorian)
                portfolio = series.enumerated().compactMap { index, value in
                    guard let balance = value.doubleValue else { return nil }
                    let date = calendar.date(byAdding: .month, value: index - series.count + 1, to: Date()) ?? Date()
                    return DashboardPoint(
                        id: date.ISO8601Format(), date: date, balanceBTC: balance,
                        fiatValue: balance * (marketRateValue ?? 0), costBasisEUR: 0,
                        priceEUR: marketRateValue
                    )
                }
            }
            let positiveConnections = connections.filter { $0.balanceBTC > 0 }
            let holdingsTotal = positiveConnections.reduce(0) { $0 + $1.balanceBTC }
            holdings = positiveConnections
                .sorted { $0.balanceBTC > $1.balanceBTC }
                .prefix(3)
                .map { connection in
                    DashboardHolding(
                        id: connection.id,
                        label: connection.label,
                        valueBTC: connection.balanceBTC,
                        percentage: holdingsTotal > 0 ? connection.balanceBTC / holdingsTotal : 0
                    )
                }
            if positiveConnections.count > 3 {
                let top = holdings.reduce(0) { $0 + $1.valueBTC }
                holdings.append(DashboardHolding(
                    id: "other",
                    label: "Other sources",
                    valueBTC: max(0, holdingsTotal - top),
                    percentage: holdingsTotal > 0 ? max(0, holdingsTotal - top) / holdingsTotal : 0
                ))
            }
            let driverFlows: [(String, TransactionFlowFilter)] = [
                ("incoming", .incoming), ("outgoing", .outgoing),
                ("swap", .swap), ("fees", .all)
            ]
            drivers = driverFlows.map { key, flow in
                let rows = activityTransactions.filter { row in
                    guard !row.excluded else { return false }
                    if key == "fees" { return row.feeSats > 0 }
                    return row.flow == flow
                }
                let amount = rows.reduce(0.0) { total, row in
                    if key == "fees" { return total + Double(abs(row.feeSats)) / 100_000_000 }
                    return total + Double(abs(row.amountSats)) / 100_000_000
                }
                return DashboardDriver(id: key, valueBTC: amount, count: rows.count)
            }
            totalBTC = portfolio.last?.balanceBTC
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }
    public func refreshLatestRate() async {
        do {
            var args: [String: JSONValue] = [:]
            if let marketRatePair { args["pair"] = .string(marketRatePair) }
            let result = try await daemon.invoke(.uiRatesLatest, args: args)
            if let error = result.error { errorMessage = error.message; return }
            await load()
        } catch { errorMessage = String(describing: error) }
    }
}
