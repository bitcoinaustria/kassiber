import Foundation
import KassiberDaemonKit

public struct TransactionGraphAnnotation: Identifiable, Equatable, Sendable {
    public let code: String
    public let label: String
    public let severity: String
    public let groupID: String?
    public let amountSats: Int64?

    public var id: String {
        [code, groupID ?? "", label].joined(separator: ":")
    }

    init(_ row: [String: JSONValue]) {
        code = row.string("code") ?? "annotation"
        label = row.string("label") ?? code
        severity = row.string("severity") ?? "info"
        groupID = row.string("groupId", "group_id")
        if let msat = row.int("amountMsat", "amount_msat") {
            amountSats = msat / 1_000
        } else if let btc = row.double("amountBtc", "amount_btc") {
            amountSats = Int64((btc * 100_000_000).rounded())
        } else {
            amountSats = nil
        }
    }
}

public struct TransactionGraphNode: Identifiable, Equatable, Sendable {
    public let id: String
    public let index: Int?
    public let outpoint: String?
    public let transactionID: String?
    public let address: String?
    public let scriptType: String?
    public let valueSats: Int64?
    public let valueState: String
    public let label: String
    public let wallet: String?
    public let ownership: String
    public let role: String
    public let overflow: Bool
    public let overflowCount: Int
    public let annotations: [TransactionGraphAnnotation]

    init?(_ row: [String: JSONValue]) {
        let fallbackID = row.string("outpoint", "txid", "address", "label")
        guard let id = row.string("id") ?? fallbackID, !id.isEmpty else { return nil }
        self.id = id
        index = row.int("index").map(Int.init)
        outpoint = row.string("outpoint")
        transactionID = row.string("txid")
        address = row.string("address")
        scriptType = row.string("scriptType", "script_type")
        if let sats = row.int("valueSats", "value_sats") {
            valueSats = sats
        } else if let btc = row.double("valueBtc", "value_btc") {
            valueSats = Int64((btc * 100_000_000).rounded())
        } else {
            valueSats = nil
        }
        valueState = row.string("valueState", "value_state") ?? (valueSats == nil ? "missing" : "known")
        label = row.string("label") ?? ""
        wallet = row.string("wallet")
        ownership = row.string("ownership") ?? "unknown"
        role = row.string("role") ?? "leg"
        overflow = row.bool("overflow") ?? false
        overflowCount = Int(row.int("overflowCount", "overflow_count") ?? 0)
        annotations = row.objects("annotations").map(TransactionGraphAnnotation.init)
    }

    public var reference: String {
        address ?? outpoint ?? transactionID ?? label
    }
}

public struct TransactionGraphWarning: Identifiable, Equatable, Sendable {
    public let code: String
    public let level: String
    public let message: String
    public var id: String { "\(code):\(message)" }

    init(_ row: [String: JSONValue]) {
        code = row.string("code") ?? "warning"
        level = row.string("level") ?? "warning"
        message = row.string("message") ?? code
    }
}

public struct TransactionGraphMetadata: Equatable, Sendable {
    public let id: String
    public let transactionID: String?
    public let asset: String
    public let network: String
    public let inputCount: Int
    public let outputCount: Int
    public let version: Int?
    public let locktime: Int?
    public let size: Int?
    public let virtualSize: Int?
    public let weight: Int?
    public let feeRateSatVByte: Double?

    init(_ row: [String: JSONValue]) {
        id = row.string("id") ?? ""
        transactionID = row.string("txid", "externalId", "external_id")
        asset = row.string("asset") ?? "BTC"
        network = row.string("network") ?? ""
        inputCount = Int(row.int("inputCount", "input_count") ?? 0)
        outputCount = Int(row.int("outputCount", "output_count") ?? 0)
        version = row.int("version").map(Int.init)
        locktime = row.int("locktime").map(Int.init)
        size = row.int("size").map(Int.init)
        virtualSize = row.int("vsize").map(Int.init)
        weight = row.int("weight").map(Int.init)
        feeRateSatVByte = row.double("feeRateSatVb", "fee_rate_sat_vb")
    }
}

public struct TransactionSwapRouteLeg: Equatable, Sendable {
    public let id: String?
    public let transactionID: String?
    public let role: String
    public let asset: String
    public let network: String
    public let amountSats: Int64?
    public let wallet: String?
    public let counterparty: String?

    init(_ row: [String: JSONValue]) {
        id = row.string("id")
        transactionID = row.string("txid", "externalId", "external_id")
        role = row.string("role") ?? ""
        asset = row.string("asset") ?? "BTC"
        network = row.string("network") ?? asset
        if let msat = row.int("amountMsat", "amount_msat") {
            amountSats = msat / 1_000
        } else if let btc = row.double("amountBtc", "amount_btc") {
            amountSats = Int64((btc * 100_000_000).rounded())
        } else {
            amountSats = nil
        }
        wallet = row["wallet"]?.objectValue?.string("label")
        counterparty = row.string("counterparty", "description")
    }

    init(
        id: String?, transactionID: String?, role: String, asset: String,
        network: String, amountSats: Int64?, wallet: String?, counterparty: String?
    ) {
        self.id = id
        self.transactionID = transactionID
        self.role = role
        self.asset = asset
        self.network = network
        self.amountSats = amountSats
        self.wallet = wallet
        self.counterparty = counterparty
    }
}

public struct TransactionSwapRoute: Identifiable, Equatable, Sendable {
    public let id: String
    public let kind: String
    public let routeKind: String
    public let policy: String?
    public let confidence: String?
    public let currentLeg: String
    public let feeSats: Int64?
    public let feeKind: String?
    public let out: TransactionSwapRouteLeg
    public let incoming: TransactionSwapRouteLeg

    init?(_ row: [String: JSONValue]) {
        guard let outRow = row["out"]?.objectValue,
              let inRow = row["in"]?.objectValue else { return nil }
        id = row.string("id") ?? "paired-route"
        kind = row.string("kind") ?? "pair"
        routeKind = row.string("routeKind", "route_kind") ?? Self.inferredKind(row)
        policy = row.string("policy")
        confidence = row.string("confidence")
        currentLeg = row.string("currentLeg", "current_leg") ?? "out"
        if let msat = row.int("swapFeeMsat", "swap_fee_msat") {
            feeSats = msat / 1_000
        } else if let btc = row.double("swapFeeBtc", "swap_fee_btc") {
            feeSats = Int64((btc * 100_000_000).rounded())
        } else {
            feeSats = nil
        }
        feeKind = row.string("swapFeeKind", "swap_fee_kind")
        out = TransactionSwapRouteLeg(outRow)
        incoming = TransactionSwapRouteLeg(inRow)
    }

    init(
        id: String, kind: String, routeKind: String, policy: String?,
        confidence: String?, currentLeg: String, feeSats: Int64?, feeKind: String?,
        out: TransactionSwapRouteLeg, incoming: TransactionSwapRouteLeg
    ) {
        self.id = id
        self.kind = kind
        self.routeKind = routeKind
        self.policy = policy
        self.confidence = confidence
        self.currentLeg = currentLeg
        self.feeSats = feeSats
        self.feeKind = feeKind
        self.out = out
        self.incoming = incoming
    }

    private static func inferredKind(_ row: [String: JSONValue]) -> String {
        let kind = row.string("kind")?.lowercased() ?? ""
        if kind.contains("coinjoin") || kind.contains("whirlpool") { return "coinjoin" }
        guard let out = row["out"]?.objectValue, let incoming = row["in"]?.objectValue else { return "pair" }
        if kind.contains("swap") || kind.hasPrefix("peg-")
            || out.string("asset")?.uppercased() != incoming.string("asset")?.uppercased() { return "swap" }
        if row.string("policy") == "carrying-value" { return "transfer" }
        return "pair"
    }
}

public struct TransactionGraphSnapshot: Equatable, Sendable {
    public let transaction: TransactionGraphMetadata?
    public let supportLevel: String
    public let unsupportedReason: String?
    public let warnings: [TransactionGraphWarning]
    public let inputs: [TransactionGraphNode]
    public let outputs: [TransactionGraphNode]
    public let fee: TransactionGraphNode?
    public let annotations: [TransactionGraphAnnotation]
    public let quarantineReason: String?
    public let linkedPairs: [TransactionGraphAnnotation]
    public let transferGroupIDs: [String]
    public let swapRoute: TransactionSwapRoute?

    public init(_ value: JSONValue?, pairFallback: TransactionPairRow? = nil, transaction fallback: TransactionRow? = nil) {
        let row = value?.objectValue ?? [:]
        transaction = row["transaction"]?.objectValue.map(TransactionGraphMetadata.init)
        supportLevel = row.string("supportLevel", "support_level") ?? "graphless"
        unsupportedReason = row.string("unsupportedReason", "unsupported_reason")
        warnings = row.objects("warnings").map(TransactionGraphWarning.init)
        inputs = row.objects("inputs").compactMap(TransactionGraphNode.init)
        outputs = row.objects("outputs").compactMap(TransactionGraphNode.init)
        fee = row["fee"]?.objectValue.flatMap(TransactionGraphNode.init)
        annotations = row.objects("annotations").map(TransactionGraphAnnotation.init)
        let accounting = row["accounting"]?.objectValue ?? [:]
        quarantineReason = accounting["quarantine"]?.objectValue?.string("reason")
        linkedPairs = accounting.objects("linkedPairs", "linked_pairs").map(TransactionGraphAnnotation.init)
        transferGroupIDs = accounting["transferGroupIds"]?.arrayValue?.compactMap(\.stringValue) ?? []
        swapRoute = row["swapRoute"]?.objectValue.flatMap(TransactionSwapRoute.init)
            ?? Self.fallbackRoute(pairFallback, transaction: fallback)
    }

    public var hasFlowEvidence: Bool {
        !inputs.isEmpty || !outputs.isEmpty || fee != nil || swapRoute != nil
    }

    private static func fallbackRoute(
        _ pair: TransactionPairRow?, transaction: TransactionRow?
    ) -> TransactionSwapRoute? {
        guard let pair, let transaction else { return nil }
        let currentLeg: String
        if transaction.wallet == pair.outWallet { currentLeg = "out" }
        else if transaction.wallet == pair.inWallet { currentLeg = "in" }
        else { currentLeg = transaction.amountSats < 0 ? "out" : "in" }
        let kind = pair.kind ?? pair.type
        let outAsset = pair.outAsset ?? "BTC"
        let inAsset = pair.inAsset ?? "BTC"
        let normalized = kind.lowercased()
        let routeKind: String
        if normalized.contains("coinjoin") || normalized.contains("whirlpool") { routeKind = "coinjoin" }
        else if normalized.contains("swap") || normalized.hasPrefix("peg-") || outAsset != inAsset { routeKind = "swap" }
        else if pair.policy == "carrying-value" { routeKind = "transfer" }
        else { routeKind = "pair" }
        func network(asset: String, wallet: String?) -> String {
            if asset.uppercased().contains("LBTC") || wallet?.lowercased().contains("liquid") == true { return "Liquid" }
            return asset.uppercased() == "BTC" ? "Bitcoin" : asset
        }
        let reference = transaction.transactionID ?? transaction.id
        return TransactionSwapRoute(
            id: pair.id, kind: kind, routeKind: routeKind, policy: pair.policy,
            confidence: nil, currentLeg: currentLeg,
            feeSats: pair.feeSats == 0 ? nil : abs(pair.feeSats), feeKind: pair.feeKind,
            out: TransactionSwapRouteLeg(
                id: currentLeg == "out" ? transaction.id : nil,
                transactionID: currentLeg == "out" ? reference : nil,
                role: routeKind == "swap" && network(asset: outAsset, wallet: pair.outWallet) == "Liquid"
                    ? "consolidation" : "spend",
                asset: outAsset, network: network(asset: outAsset, wallet: pair.outWallet),
                amountSats: abs(pair.outAmountSats), wallet: pair.outWallet,
                counterparty: transaction.counterparty
            ),
            incoming: TransactionSwapRouteLeg(
                id: currentLeg == "in" ? transaction.id : nil,
                transactionID: currentLeg == "in" ? reference : nil,
                role: "receive", asset: inAsset,
                network: network(asset: inAsset, wallet: pair.inWallet),
                amountSats: abs(pair.inAmountSats), wallet: pair.inWallet,
                counterparty: transaction.counterparty
            )
        )
    }
}

public struct TransactionPrivacyContext: Equatable, Sendable {
    public let matchedTransactionID: String?
    public let evidenceLevel: String
    public let tellCount: Int
    public let walletPenaltyCount: Int
    public let tellKinds: [String]
    public let degraded: Bool
    public let message: String?

    public static let loading = TransactionPrivacyContext(
        matchedTransactionID: nil, evidenceLevel: "unknown", tellCount: 0,
        walletPenaltyCount: 0, tellKinds: [], degraded: false, message: nil
    )

    public static func parse(
        _ record: DaemonRecord?, references: [String]
    ) -> TransactionPrivacyContext {
        guard let record else {
            return .init(matchedTransactionID: nil, evidenceLevel: "unknown", tellCount: 0,
                walletPenaltyCount: 0, tellKinds: [], degraded: true, message: nil)
        }
        if let error = record.error {
            return .init(matchedTransactionID: nil, evidenceLevel: "unknown", tellCount: 0,
                walletPenaltyCount: 0, tellKinds: [], degraded: true, message: error.message)
        }
        let normalized = Set(references.filter { !$0.isEmpty }.map { $0.lowercased() })
        let rows = record.data?.objectValue?.objects("transaction_view") ?? []
        let match = rows.first { row in
            guard let id = row.string("txid")?.lowercased() else { return false }
            return normalized.contains(id)
        }
        guard let match else {
            return .init(matchedTransactionID: nil, evidenceLevel: "unknown", tellCount: 0,
                walletPenaltyCount: 0, tellKinds: [], degraded: true, message: nil)
        }
        return .init(
            matchedTransactionID: match.string("txid"),
            evidenceLevel: match.string("evidence_level") ?? "unknown",
            tellCount: Int(match.int("tell_count") ?? 0),
            walletPenaltyCount: Int(match.int("wallet_penalty_count") ?? 0),
            tellKinds: match["tell_kinds"]?.arrayValue?.compactMap(\.stringValue) ?? [],
            degraded: false, message: nil
        )
    }
}
