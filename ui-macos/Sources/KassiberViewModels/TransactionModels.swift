import Foundation
import KassiberDaemonKit

public enum TransactionFlowFilter: String, CaseIterable, Identifiable, Sendable {
    case all
    case incoming
    case outgoing
    case transfer
    case swap
    case layerTransition = "layer-transition"

    public var id: String { rawValue }
}

public enum TransactionPeriodFilter: String, CaseIterable, Identifiable, Sendable {
    case auto
    case days30 = "30days"
    case months3 = "3months"
    case months6 = "6months"
    case ytd
    case year1 = "1year"
    case years5 = "5years"
    case years10 = "10years"
    case years15 = "15years"
    case all

    public var id: String { rawValue }
}

public enum TransactionQuickFilter: String, CaseIterable, Identifiable, Sendable {
    case none
    case externalFlow = "external_flow"
    case reviewQueue = "review_queue"
    case noExplorerID = "no_explorer_id"
    case missingPrice = "missing_price"
    case failedImport = "failed_import"

    public var id: String { rawValue }
}

public enum TransactionPaymentFilter: String, CaseIterable, Identifiable, Sendable {
    case all
    case onChain = "On-chain"
    case exchange = "Exchange"
    case lightning = "Lightning"
    case liquid = "Liquid"

    public var id: String { rawValue }
}

public enum TransactionChartMetric: String, CaseIterable, Identifiable, Sendable {
    case amount
    case count

    public var id: String { rawValue }
}

public enum TransactionChartMode: String, CaseIterable, Identifiable, Sendable {
    case external
    case all

    public var id: String { rawValue }
}

public enum TransactionSort: String, CaseIterable, Identifiable, Sendable {
    case dateDescending
    case dateAscending
    case amountDescending
    case amountAscending

    public var id: String { rawValue }
}

public struct TransactionPairRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let type: String
    public let kind: String?
    public let policy: String?
    public let outWallet: String?
    public let outAsset: String?
    public let outAmountSats: Int64
    public let inWallet: String?
    public let inAsset: String?
    public let inAmountSats: Int64
    public let feeSats: Int64
    public let feeKind: String?

    init?(_ value: JSONValue) {
        guard let row = value.objectValue, let id = row.string("id"), !id.isEmpty else { return nil }
        self.id = id
        type = row.string("type") ?? ""
        kind = row.string("kind")
        policy = row.string("policy")
        outWallet = row.string("outWallet", "out_wallet")
        outAsset = row.string("outAsset", "out_asset")
        outAmountSats = row.int("outAmountSat", "out_amount_sat") ?? 0
        inWallet = row.string("inWallet", "in_wallet")
        inAsset = row.string("inAsset", "in_asset")
        inAmountSats = row.int("inAmountSat", "in_amount_sat") ?? 0
        feeSats = row.int("feeSat", "fee_sat") ?? 0
        feeKind = row.string("feeKind", "fee_kind")
    }
}

public struct TransactionRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let transactionID: String?
    public let dateLabel: String
    public let occurredAt: Date?
    public let type: String
    public let asset: String
    public let wallet: String
    public let counterparty: String
    public let amountSats: Int64
    public let feeSats: Int64
    public let fiatValue: Double?
    public let fiatCurrency: String
    public let rate: Double?
    public let reviewStatus: String
    public let confirmations: Int
    public let tags: [String]
    public let note: String
    public let excluded: Bool
    public let taxable: Bool?
    public let atRegime: String?
    public let atCategory: String?
    public let chain: String
    public let network: String
    public let paymentMethod: String
    public let explorerID: String?
    public let pricingQuality: String?
    public let pricingSourceKind: String?
    public let pricingExternalRef: String?
    public let pricingProvider: String?
    public let pricingPair: String?
    public let pricingTimestamp: String?
    public let pricingFetchedAt: String?
    public let pricingGranularity: String?
    public let pricingMethod: String?
    public let quarantineReason: String?
    public let pair: TransactionPairRow?

    public var flow: TransactionFlowFilter {
        let normalized = type.lowercased()
        if normalized.contains("layer") || normalized.contains("peg-") { return .layerTransition }
        if normalized.contains("swap") || normalized == "mint" || normalized == "melt" { return .swap }
        if normalized == "transfer" || normalized == "consolidation" || normalized == "rebalance" { return .transfer }
        return amountSats < 0 ? .outgoing : .incoming
    }

    public var hasPublicExplorerID: Bool {
        guard let explorerID else { return false }
        return explorerID.count == 64 && explorerID.allSatisfy(\.isHexDigit)
    }

    public var isMissingPrice: Bool {
        rate == nil || (rate ?? 0) <= 0 || pricingQuality == "missing" || pricingSourceKind == "missing"
    }

    init?(_ value: JSONValue) {
        guard let row = value.objectValue else { return nil }
        let id = row.string("id", "transaction_id", "externalId") ?? ""
        guard !id.isEmpty else { return nil }
        self.id = id
        transactionID = row.string("externalId", "explorerId", "txid", "transaction_id")
        dateLabel = row.string("date", "occurredAt", "occurred_at") ?? ""
        occurredAt = DaemonValueParser.date(row.string("occurredAt", "occurred_at", "date"))
        type = row.string("type", "direction") ?? "Transaction"
        asset = row.string("asset") ?? "BTC"
        wallet = row.string("account", "wallet", "wallet_label") ?? ""
        counterparty = row.string("counter", "counterparty", "description") ?? ""
        amountSats = row.int("amountSat", "amount_sat", "quantity_sat") ?? 0
        feeSats = row.int("feeSat", "fee_sat") ?? 0
        fiatValue = row.double("eur", "fiat_value", "value")
        fiatCurrency = row.string("fiatCurrency", "fiat_currency") ?? "EUR"
        rate = row.double("rate")
        reviewStatus = row.string("reviewStatus", "review_status", "status") ?? "completed"
        confirmations = Int(row.int("conf", "confirmations") ?? 0)
        tags = row["tags"]?.arrayValue?.compactMap(\.stringValue)
            ?? row.string("tag").map { [$0] }
            ?? []
        note = row.string("note") ?? ""
        excluded = row.bool("excluded") ?? false
        taxable = row["taxable"]?.boolValue
        atRegime = row.string("atRegime", "at_regime")
        atCategory = row.string("atCategory", "at_category")
        chain = row.string("chain") ?? ""
        network = row.string("network") ?? ""
        explorerID = row.string("explorerId", "externalId", "txid")
        pricingQuality = row.string("pricingQuality", "pricing_quality")
        pricingSourceKind = row.string("pricingSourceKind", "pricing_source_kind")
        pricingExternalRef = row.string("pricingExternalRef", "pricing_external_ref")
        pricingProvider = row.string("pricingProvider", "pricing_provider")
        pricingPair = row.string("pricingPair", "pricing_pair")
        pricingTimestamp = row.string("pricingTimestamp", "pricing_timestamp")
        pricingFetchedAt = row.string("pricingFetchedAt", "pricing_fetched_at")
        pricingGranularity = row.string("pricingGranularity", "pricing_granularity")
        pricingMethod = row.string("pricingMethod", "pricing_method")
        quarantineReason = row.string("quarantineReason", "quarantine_reason")
        pair = row["pair"].flatMap(TransactionPairRow.init)
        if asset.caseInsensitiveCompare("LBTC") == .orderedSame
            || chain.caseInsensitiveCompare("liquid") == .orderedSame
            || wallet.localizedCaseInsensitiveContains("liquid") {
            paymentMethod = TransactionPaymentFilter.liquid.rawValue
        } else if wallet.localizedCaseInsensitiveContains("lightning")
            || wallet.localizedCaseInsensitiveContains("phoenix")
            || wallet.lowercased().hasPrefix("ln ") {
            paymentMethod = TransactionPaymentFilter.lightning.rawValue
        } else if ["kraken", "bitstamp", "coinbase", "bitpanda", "river", "bullbitcoin", "coinfinity", "strike", "exchange"]
            .contains(where: wallet.localizedCaseInsensitiveContains) {
            paymentMethod = TransactionPaymentFilter.exchange.rawValue
        } else {
            paymentMethod = TransactionPaymentFilter.onChain.rawValue
        }
    }
}
