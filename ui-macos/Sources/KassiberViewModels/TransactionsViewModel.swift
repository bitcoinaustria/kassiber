import Foundation
import Observation
import KassiberDaemonKit

@MainActor
@Observable
public final class TransactionsViewModel {
    public var walletScope: String? {
        didSet { if oldValue != walletScope { Task { await reload() } } }
    }
    public var search = ""
    public var flow: TransactionFlowFilter = .all
    public var reviewStatus = "all"
    public var feeOnly = false
    public var sort: TransactionSort = .dateDescending
    public var period: TransactionPeriodFilter = .auto
    public var paymentMethod: TransactionPaymentFilter = .all
    public var quickFilter: TransactionQuickFilter = .none
    public var chartMetric: TransactionChartMetric = .amount
    public var chartMode: TransactionChartMode = .external
    public var chartBucket: Date?
    public var chartSegment: TransactionFlowFilter?
    public var showWorkbench = true
    public private(set) var rows: [TransactionRow] = []
    public private(set) var wallets: [WalletRow] = []
    public private(set) var nextCursor: String?
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient

    public init(daemon: any DaemonClient, walletScope: String? = nil) {
        self.daemon = daemon
        self.walletScope = walletScope
    }

    public var visibleRows: [TransactionRow] {
        let query = search.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let indexed = rows.enumerated().filter { _, row in
            let matchesSearch = query.isEmpty || [
                row.id, row.transactionID ?? "", row.wallet, row.counterparty,
                row.type, row.tags.joined(separator: " "),
            ].contains { $0.lowercased().contains(query) }
            let matchesFlow = flow == .all || row.flow == flow
            let matchesStatus = reviewStatus == "all" || row.reviewStatus == reviewStatus
            let matchesFee = !feeOnly || row.feeSats > 0
            let matchesPayment = paymentMethod == .all || row.paymentMethod == paymentMethod.rawValue
            let matchesQuick: Bool
            switch quickFilter {
            case .none: matchesQuick = true
            case .externalFlow: matchesQuick = row.flow == .incoming || row.flow == .outgoing
            case .reviewQueue: matchesQuick = row.reviewStatus != "completed"
            case .noExplorerID: matchesQuick = !row.hasPublicExplorerID
            case .missingPrice: matchesQuick = row.isMissingPrice
            case .failedImport: matchesQuick = row.reviewStatus == "failed"
            }
            let matchesChartBucket: Bool
            if let chartBucket {
                guard let occurredAt = row.occurredAt else { return false }
                let calendar = Calendar(identifier: .gregorian)
                matchesChartBucket = calendar.date(from: calendar.dateComponents([.year, .month], from: occurredAt)) ==
                    calendar.date(from: calendar.dateComponents([.year, .month], from: chartBucket))
            } else {
                matchesChartBucket = true
            }
            let matchesChartSegment = chartSegment == nil || chartSegment == row.flow
            return matchesSearch && matchesFlow && matchesStatus && matchesFee && matchesPayment && matchesQuick && matchesChartBucket && matchesChartSegment
        }
        return indexed.sorted { left, right in
            let lhs = left.element
            let rhs = right.element
            let ordered: Bool?
            switch sort {
            case .dateDescending:
                ordered = compareOptional(lhs.occurredAt, rhs.occurredAt, descending: true)
            case .dateAscending:
                ordered = compareOptional(lhs.occurredAt, rhs.occurredAt, descending: false)
            case .amountDescending:
                ordered = lhs.amountSats == rhs.amountSats ? nil : lhs.amountSats > rhs.amountSats
            case .amountAscending:
                ordered = lhs.amountSats == rhs.amountSats ? nil : lhs.amountSats < rhs.amountSats
            }
            return ordered ?? (left.offset < right.offset)
        }.map(\.element)
    }

    public var metrics: TransactionWorkbenchMetrics {
        let source = workbenchRows
        return TransactionWorkbenchMetrics(
            incomingSats: source.filter { $0.flow == .incoming }.reduce(0) { $0 + abs($1.amountSats) },
            outgoingSats: source.filter { $0.flow == .outgoing }.reduce(0) { $0 + abs($1.amountSats) },
            internalCount: source.filter { $0.flow == .transfer || $0.flow == .swap || $0.flow == .layerTransition }.count,
            reviewCount: source.filter { $0.reviewStatus != "completed" }.count,
            missingPriceCount: source.filter(\.isMissingPrice).count
        )
    }

    public var chartRows: [TransactionFlowChartRow] {
        let calendar = Calendar(identifier: .gregorian)
        let grouped = Dictionary(grouping: workbenchRows.compactMap { row -> (Date, TransactionRow)? in
            guard let date = row.occurredAt else { return nil }
            return (calendar.date(from: calendar.dateComponents([.year, .month], from: date)) ?? date, row)
        }, by: \.0)
        return grouped.keys.sorted().flatMap { date in
            let rows = grouped[date, default: []].map(\.1)
            let flows: [TransactionFlowFilter] = [.incoming, .outgoing, .transfer, .swap, .layerTransition]
            return flows.compactMap { flow -> TransactionFlowChartRow? in
                let matches = rows.filter { $0.flow == flow }
                guard !matches.isEmpty else { return nil }
                let value = chartMetric == .count
                    ? Double(matches.count)
                    : Double(matches.reduce(0) { $0 + abs($1.amountSats) })
                return TransactionFlowChartRow(date: date, flow: flow, value: value, count: matches.count)
            }
        }
    }

    public func clearChartSelection() {
        chartBucket = nil
        chartSegment = nil
    }

    private var workbenchRows: [TransactionRow] {
        chartMode == .external
            ? rows.filter { $0.flow == .incoming || $0.flow == .outgoing }
            : rows
    }

    public func load() async {
        async let walletLoad: Void = loadWallets()
        async let transactionLoad: Void = reload()
        _ = await (walletLoad, transactionLoad)
    }

    public func reload() async {
        rows = []
        nextCursor = nil
        await loadPage(cursor: nil, replacing: true)
    }

    public func loadMore() async {
        guard let nextCursor, !isLoading else { return }
        await loadPage(cursor: nextCursor, replacing: false)
    }

    /// Resolves an internal id, txid, or external reference without walking
    /// paginated transaction pages. Global search uses this allowlisted read to
    /// hand a precise result to the native detail sheet.
    public func resolveTransaction(_ query: String) async -> TransactionRow? {
        let reference = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !reference.isEmpty else { return nil }
        if let local = rows.first(where: {
            $0.id == reference || $0.transactionID == reference || $0.explorerID == reference
        }) {
            return local
        }
        do {
            let envelope = try await daemon.invoke(
                .uiTransactionsResolve,
                args: ["query": .string(reference)]
            )
            if let error = envelope.error {
                errorMessage = error.message
                return nil
            }
            guard let value = envelope.data?.objectValue?["transaction"],
                  let transaction = TransactionRow(value) else {
                return nil
            }
            errorMessage = nil
            return transaction
        } catch {
            errorMessage = String(describing: error)
            return nil
        }
    }

    private func loadWallets() async {
        do {
            let envelope = try await daemon.invoke(.uiWalletsList, args: nil)
            wallets = WalletRow.parseList(envelope.data)
        } catch {
            if errorMessage == nil { errorMessage = String(describing: error) }
        }
    }

    private func loadPage(cursor: String?, replacing: Bool) async {
        isLoading = true
        defer { isLoading = false }
        var args: [String: JSONValue] = ["limit": .integer(100)]
        if let walletScope, !walletScope.isEmpty { args["wallet"] = .string(walletScope) }
        if let cursor { args["cursor"] = .string(cursor) }
        if period != .auto && period != .all { args["period"] = .string(period.rawValue) }
        if reviewStatus != "all" { args["status"] = .string(reviewStatus) }
        if flow != .all { args["flow"] = .string(flow.rawValue) }
        if paymentMethod != .all { args["payment_method"] = .string(paymentMethod.rawValue) }
        if feeOnly { args["with_fees"] = .bool(true) }
        if quickFilter != .none { args["quick"] = .string(quickFilter.rawValue) }
        switch sort {
        case .dateDescending: args["sort"] = "occurred-at"; args["order"] = "desc"
        case .dateAscending: args["sort"] = "occurred-at"; args["order"] = "asc"
        case .amountDescending: args["sort"] = "amount"; args["order"] = "desc"
        case .amountAscending: args["sort"] = "amount"; args["order"] = "asc"
        }
        do {
            let envelope = try await daemon.invoke(.uiTransactionsList, args: args)
            if let error = envelope.error {
                errorMessage = error.message
                return
            }
            guard let data = envelope.data?.objectValue else {
                errorMessage = "The transaction response had an unexpected format."
                return
            }
            let page = (data["txs"]?.arrayValue ?? []).compactMap(TransactionRow.init)
            rows = replacing ? page : rows + page
            nextCursor = data.string("nextCursor", "next_cursor")
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    private func compareOptional<T: Comparable>(_ left: T?, _ right: T?, descending: Bool) -> Bool? {
        switch (left, right) {
        case let (left?, right?) where left != right:
            descending ? left > right : left < right
        case (_?, nil): true
        case (nil, _?): false
        default: nil
        }
    }
}

public struct TransactionWorkbenchMetrics: Equatable, Sendable {
    public let incomingSats: Int64
    public let outgoingSats: Int64
    public let internalCount: Int
    public let reviewCount: Int
    public let missingPriceCount: Int
}

public struct TransactionFlowChartRow: Identifiable, Equatable, Sendable {
    public let date: Date
    public let flow: TransactionFlowFilter
    public let value: Double
    public let count: Int
    public var id: String { "\(date.timeIntervalSince1970)-\(flow.rawValue)" }
}
