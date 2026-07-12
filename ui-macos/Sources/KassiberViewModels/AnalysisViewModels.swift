import Foundation
import Observation
import KassiberDaemonKit

public struct ActivityEventRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let transactionID: String
    public let transactionReference: String
    public let wallet: String
    public let changedAt: Date?
    public let source: String
    public let reason: String
    public let summary: String
    public let families: [String]
    public let stale: Bool
    public let fields: [String]
}

@MainActor
@Observable
public final class ActivityViewModel {
    public var dateDays = 30
    public var source = "all"
    public var family = "all"
    public var wallet = ""
    public var transaction = ""
    public var pricingOnly = false
    public var aiOnly = false
    public var staleOnly = false
    public private(set) var events: [ActivityEventRow] = []
    public private(set) var staleCount = 0
    public private(set) var nextCursor: String?
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?
    public private(set) var actionMessage: String?

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func load(reset: Bool = true) async {
        isLoading = true
        defer { isLoading = false }
        do {
            var args: [String: JSONValue] = ["limit": .integer(100), "include_stale": .bool(false)]
            if dateDays > 0 {
                args["start"] = .string(ISO8601DateFormatter().string(from: Date().addingTimeInterval(-Double(dateDays) * 86_400)))
            }
            if source != "all" { args["source"] = .string(source) }
            if family != "all" { args["field_family"] = .string(family) }
            if !wallet.trimmingCharacters(in: .whitespaces).isEmpty { args["wallet"] = .string(wallet) }
            if !transaction.trimmingCharacters(in: .whitespaces).isEmpty { args["transaction"] = .string(transaction) }
            if pricingOnly { args["pricing_only"] = true }
            if aiOnly { args["ai_only"] = true }
            if staleOnly { args["stale_only"] = true }
            if !reset, let nextCursor { args["cursor"] = .string(nextCursor) }
            async let historyCall = daemon.invoke(.uiActivityHistory, args: args)
            async let staleCall = daemon.invoke(.uiActivityStale, args: nil)
            let (history, stale) = try await (historyCall, staleCall)
            if let error = history.error ?? stale.error { errorMessage = error.message; return }
            let object = history.data?.objectValue ?? [:]
            let parsed = object.objects("events").compactMap(Self.parseEvent)
            events = reset ? parsed : events + parsed
            nextCursor = object.string("next_cursor")
            staleCount = Int(stale.data?.objectValue?.int("edit_count") ?? 0)
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }

    public func revert(_ event: ActivityEventRow) async {
        do {
            let result = try await daemon.invoke(.uiTransactionsHistoryRevert, args: [
                "transaction": .string(event.transactionID), "event": .string(event.id),
                "source": .string("gui"), "reason": .string("Reverted from Activity")
            ])
            if let error = result.error { errorMessage = error.message; return }
            actionMessage = "reverted"
            await load()
        } catch { errorMessage = String(describing: error) }
    }

    public func processJournals() async {
        do {
            let result = try await daemon.invoke(.uiJournalsProcess, args: nil)
            if let error = result.error { errorMessage = error.message; return }
            actionMessage = "processed"
            await load()
        } catch { errorMessage = String(describing: error) }
    }

    private static func parseEvent(_ row: [String: JSONValue]) -> ActivityEventRow? {
        guard let id = row.string("id"), let transactionID = row.string("transaction_id") else { return nil }
        let anchor = row["report_anchor"]?.objectValue ?? [:]
        let fieldLabels = row.objects("fields").map { field in
            let label = field.string("label", "field") ?? ""
            let before = display(field["before_label"] ?? field["before_value"])
            let after = display(field["after_label"] ?? field["after_value"])
            return "\(label): \(before) → \(after)"
        }
        return ActivityEventRow(
            id: id, transactionID: transactionID,
            transactionReference: row.string("transaction_external_id") ?? transactionID,
            wallet: row.string("wallet_label") ?? "", changedAt: DaemonValueParser.date(row.string("changed_at")),
            source: row.string("source", "source_label") ?? "", reason: row.string("reason") ?? "",
            summary: row.string("summary") ?? "", families: row["families"]?.arrayValue?.compactMap(\.stringValue) ?? [],
            stale: anchor.bool("stale_for_reports") ?? false, fields: fieldLabels
        )
    }
}

public struct PrivacyFindingRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let kind: String
    public let severity: String
    public let title: String
    public let detail: String
    public let evidence: String
    public let transactionID: String?
    public let routesToSourceFunds: Bool
}

public struct PrivacyTableRow: Identifiable, Equatable, Sendable {
    public enum Detail: Equatable, Sendable {
        case wallet(coinCount: Int, linkCount: Int)
        case transaction(tellCount: Int, tellKinds: String)
        case utxo(branchRole: String, sourceProximity: String)
    }

    public let id: String
    public let primary: String
    public let detail: Detail
    public let evidence: String
    public let walletID: String?
    public let amountMSat: Int64?
    public let clusterCount: Int
    public let unknownRoleCoinCount: Int
    public let walletPenaltyCount: Int

    public init(
        id: String,
        primary: String,
        detail: Detail,
        evidence: String,
        walletID: String? = nil,
        amountMSat: Int64? = nil,
        clusterCount: Int = 0,
        unknownRoleCoinCount: Int = 0,
        walletPenaltyCount: Int = 0
    ) {
        self.id = id
        self.primary = primary
        self.detail = detail
        self.evidence = evidence
        self.walletID = walletID
        self.amountMSat = amountMSat
        self.clusterCount = clusterCount
        self.unknownRoleCoinCount = unknownRoleCoinCount
        self.walletPenaltyCount = walletPenaltyCount
    }
}

public struct PrivacyScoreFactorRow: Identifiable, Equatable, Sendable {
    public let key: String
    public let linked: Int?
    public let leaking: Int?
    public let total: Int?
    public let weight: Double?
    public let points: Int
    public var id: String { key }
}

public struct PrivacySeverityCensus: Equatable, Sendable {
    public let alert: Int
    public let warning: Int
    public let info: Int
    public var total: Int { alert + warning + info }
}

public struct PrivacyWorstRisk: Equatable, Sendable {
    public let kind: String
    public let severity: String
    public let title: String
    public let answer: String
    public let evidence: String
}

public struct PrivacyAdversaryAssumption: Identifiable, Equatable, Sendable {
    public let code: String
    public let statement: String
    public let evidence: String
    public var id: String { code }
}

public struct PrivacyAdversaryCard: Identifiable, Equatable, Sendable {
    public let tier: String
    public let label: String
    public let evidence: String
    public let exposedClusterCount: Int
    public let walletCount: Int
    public let observerEntityCount: Int
    public let unknownCoverageStatus: String
    public let unknownNodeCount: Int
    public let assumptions: [PrivacyAdversaryAssumption]
    public var id: String { tier }
}

public struct PrivacyTimelineRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let kind: String
    public let category: String
    public let transactionID: String
    public let evidence: String
    public let detail: String
    public let newLinkage: Bool
}

public struct PrivacyEvidenceDrilldownRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let section: String
    public let kind: String
    public let evidenceLevel: String
    public let facts: [KeyValueRow]
}

public struct PrivacyCoverageSummary: Equatable, Sendable {
    public let evidence: String
    public let knownCoinCount: Int
    public let unknownCoinCount: Int
    public let unknownCoverageCount: Int
    public let degraded: Bool
}

public struct PrivacyHeuristicRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let status: String
}

@MainActor
@Observable
public final class PrivacyMirrorViewModel {
    public private(set) var score = 0
    public private(set) var scoreBase = 100
    public private(set) var scoreIsGrounded = false
    public private(set) var scoreFactors: [PrivacyScoreFactorRow] = []
    public private(set) var severityCensus = PrivacySeverityCensus(alert: 0, warning: 0, info: 0)
    public private(set) var coverage = 0.0
    public private(set) var coverageSummary = PrivacyCoverageSummary(
        evidence: "unknown", knownCoinCount: 0, unknownCoinCount: 0,
        unknownCoverageCount: 0, degraded: false
    )
    public private(set) var evidenceLevel = "unknown"
    public private(set) var worstRisk = ""
    public private(set) var worstRiskModel = PrivacyWorstRisk(
        kind: "", severity: "info", title: "", answer: "", evidence: "unknown"
    )
    public private(set) var localOnly: Bool?
    public private(set) var readOnly: Bool?
    public private(set) var advisoryOnly: Bool?
    public private(set) var linkageScore = 0
    public private(set) var linkableClusterCount = 0
    public private(set) var adversaryViewCount = 0
    public private(set) var walletCount = 0
    public private(set) var transactionTellCount = 0
    public private(set) var utxoCount = 0
    public private(set) var unknownCount = 0
    public private(set) var findings: [PrivacyFindingRow] = []
    public private(set) var adversaryCards: [PrivacyAdversaryCard] = []
    public private(set) var wallets: [PrivacyTableRow] = []
    public private(set) var transactions: [PrivacyTableRow] = []
    public private(set) var utxos: [PrivacyTableRow] = []
    public private(set) var timeline: [PrivacyTimelineRow] = []
    public private(set) var evidenceDrilldowns: [PrivacyEvidenceDrilldownRow] = []
    public var psbt = ""
    public private(set) var psbtResult: [KeyValueRow] = []
    public private(set) var transactionGraph: [KeyValueRow] = []
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?
    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public var guardrailsNominal: Bool {
        localOnly != false && readOnly != false && advisoryOnly != false && !coverageSummary.degraded
    }

    public var grade: String {
        switch score {
        case 90...: "A+"
        case 75...: "B"
        case 50...: "C"
        case 25...: "D"
        default: "F"
        }
    }

    public static let heuristics: [PrivacyHeuristicRow] = [
        .init(id: "h3", name: "Common input ownership", status: "computed"),
        .init(id: "h8", name: "Address reuse", status: "computed"),
        .init(id: "h2", name: "Change detection", status: "computed"),
        .init(id: "h1", name: "Round amounts", status: "computed"),
        .init(id: "h6", name: "Fee fingerprinting", status: "computed"),
        .init(id: "h7", name: "OP_RETURN metadata", status: "computed"),
        .init(id: "h11", name: "Wallet fingerprinting", status: "computed"),
        .init(id: "script", name: "Script type analysis", status: "computed"),
        .init(id: "witness", name: "Witness data", status: "computed"),
        .init(id: "dust", name: "Dust output detection", status: "computed"),
        .init(id: "unnecessary", name: "Unnecessary inputs", status: "computed"),
        .init(id: "h9", name: "UTXO analysis", status: "computed"),
        .init(id: "h10", name: "Address type", status: "computed"),
        .init(id: "h4", name: "CoinJoin detection", status: "computed"),
        .init(id: "consolidation", name: "Consolidation patterns", status: "partial"),
        .init(id: "utxo-age", name: "UTXO age spread", status: "partial"),
        .init(id: "bip69", name: "BIP69 ordering", status: "partial"),
        .init(id: "coinsel", name: "Coin selection", status: "partial"),
        .init(id: "dust-spend", name: "Dust spending", status: "partial"),
        .init(id: "h17", name: "Multisig / escrow", status: "partial"),
        .init(id: "coinbase", name: "Coinbase", status: "partial"),
        .init(id: "spending", name: "Spending patterns", status: "partial"),
        .init(id: "recurring", name: "Recurring payment", status: "partial"),
        .init(id: "highactivity", name: "High activity", status: "partial"),
        .init(id: "h5", name: "Transaction entropy", status: "not_local"),
        .init(id: "anon", name: "Anonymity sets", status: "not_local"),
        .init(id: "peel", name: "Peel chain", status: "not_local"),
        .init(id: "tx0", name: "CoinJoin premix", status: "not_local"),
        .init(id: "postmix", name: "Post-mix consolidation", status: "not_local"),
        .init(id: "ricochet", name: "Ricochet", status: "not_local"),
        .init(id: "entity", name: "Known entity", status: "not_local"),
        .init(id: "exchange", name: "Exchange pattern", status: "not_local"),
        .init(id: "bip47", name: "BIP47 notification", status: "not_local"),
        .init(id: "timing", name: "Timing analysis", status: "not_local"),
    ]

    public func load() async {
        isLoading = true; defer { isLoading = false }
        do {
            let result = try await daemon.invoke(.uiReportsPrivacyMirror, args: nil)
            if let error = result.error { errorMessage = error.message; return }
            let object = result.data?.objectValue ?? [:]
            let summary = object["summary"]?.objectValue ?? [:]
            let scoreObject = summary["privacy_score"]?.objectValue ?? [:]
            localOnly = object.bool("local_only")
            readOnly = object.bool("read_only")
            advisoryOnly = object.bool("advisory_only")
            evidenceLevel = summary.string("evidence_level") ?? "unknown"
            let coverageObject = object["coverage"]?.objectValue ?? [:]
            coverageSummary = PrivacyCoverageSummary(
                evidence: coverageObject.string("evidence_level") ?? "unknown",
                knownCoinCount: Int(coverageObject.int("source_proximity_known_coin_count") ?? 0),
                unknownCoinCount: Int(coverageObject.int("source_proximity_unknown_coin_count") ?? 0),
                unknownCoverageCount: Int(coverageObject.int("unknown_coverage_count") ?? 0),
                degraded: coverageObject.bool("degraded") ?? false
            )
            findings = Self.findingRows(object)
            severityCensus = Self.census(findings)
            scoreIsGrounded = scoreObject.double("value") != nil
            scoreBase = Int(scoreObject.int("base") ?? 100)
            scoreFactors = scoreObject.objects("factors").enumerated().map { index, row in
                PrivacyScoreFactorRow(
                    key: row.string("key") ?? "factor-\(index)",
                    linked: row.int("linked").map(Int.init),
                    leaking: row.int("leaking").map(Int.init),
                    total: row.int("total").map(Int.init),
                    weight: row.double("weight"),
                    points: Int(row.int("points") ?? 0)
                )
            }
            if let groundedScore = scoreObject.double("value") {
                score = max(0, min(100, Int(groundedScore.rounded())))
            } else {
                scoreBase = 70
                scoreFactors = Self.fallbackFactors(severityCensus)
                score = max(0, min(100, scoreBase + scoreFactors.reduce(0) { $0 + $1.points }))
            }
            coverage = scoreObject.double("coverage_ratio") ?? 0
            let worst = summary["worst_risk"]?.objectValue ?? [:]
            worstRiskModel = PrivacyWorstRisk(
                kind: worst.string("kind") ?? "",
                severity: worst.string("severity") ?? "info",
                title: worst.string("title") ?? "",
                answer: worst.string("answer") ?? "",
                evidence: worst.string("evidence_level") ?? "unknown"
            )
            worstRisk = worstRiskModel.answer.isEmpty ? worstRiskModel.title : worstRiskModel.answer
            linkageScore = Int(summary.int("linkage_score") ?? 0)
            linkableClusterCount = Int(summary.int("linkable_cluster_count") ?? 0)
            adversaryViewCount = Int(summary.int("adversary_view_count") ?? 0)
            walletCount = Int(summary.int("wallet_count") ?? 0)
            transactionTellCount = Int(summary.int("transaction_tell_count") ?? 0)
            utxoCount = Int(summary.int("utxo_count") ?? 0)
            unknownCount = Int(summary.int("unknown_count") ?? 0)
            adversaryCards = Self.adversaryRows(object)
            wallets = object.objects("wallet_view").enumerated().map { index, row in
                PrivacyTableRow(id: row.string("wallet_id") ?? "wallet-\(index)",
                    primary: row.string("wallet_id") ?? "—",
                    detail: .wallet(
                        coinCount: Int(row.int("coin_count") ?? 0),
                        linkCount: Int(row.int("linkage_edge_count") ?? 0)
                    ),
                    evidence: row.string("evidence_level") ?? "unknown",
                    walletID: row.string("wallet_id"), amountMSat: row.int("amount_msat"),
                    clusterCount: Int(row.int("cluster_count") ?? 0),
                    unknownRoleCoinCount: Int(row.int("unknown_role_coin_count") ?? 0))
            }
            transactions = object.objects("transaction_view").enumerated().map { index, row in
                PrivacyTableRow(id: row.string("txid") ?? "transaction-\(index)",
                    primary: row.string("txid") ?? "—",
                    detail: .transaction(
                        tellCount: Int(row.int("tell_count") ?? 0),
                        tellKinds: display(row["tell_kinds"])
                    ),
                    evidence: row.string("evidence_level") ?? "unknown",
                    walletPenaltyCount: Int(row.int("wallet_penalty_count") ?? 0))
            }
            utxos = object.objects("utxo_view").enumerated().map { index, row in
                PrivacyTableRow(id: row.string("coin_id") ?? "coin-\(index)",
                    primary: row.string("coin_id") ?? "—",
                    detail: .utxo(
                        branchRole: row.string("branch_role") ?? "unknown",
                        sourceProximity: row.string("source_proximity") ?? "unknown"
                    ),
                    evidence: row.string("evidence_level") ?? "unknown",
                    walletID: row.string("wallet_id"), amountMSat: row.int("amount_msat"))
            }
            timeline = Self.timelineRows(object)
            evidenceDrilldowns = Self.evidenceRows(object)
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }

    public func analyzePSBT() async {
        do {
            let result = try await daemon.invoke(.uiReportsPsbtPrivacy, args: ["psbt": .string(psbt)])
            if let error = result.error { errorMessage = error.message; return }
            psbtResult = flatten(result.data)
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }
    public func loadGraph(transaction: String) async {
        do {
            let result = try await daemon.invoke(.uiTransactionsGraph, args: ["transaction": .string(transaction), "allowPublicLookup": .bool(false)])
            if let error = result.error { errorMessage = error.message; return }
            transactionGraph = flatten(result.data)
        } catch { errorMessage = String(describing: error) }
    }

    private static func findingRows(_ object: [String: JSONValue]) -> [PrivacyFindingRow] {
        var rows: [PrivacyFindingRow] = []
        for (index, source) in object.objects("transaction_view").enumerated() {
            let transactionID = source.string("txid")
            let kind = source["tell_kinds"]?.arrayValue?.compactMap(\.stringValue).first
                ?? "transaction_tell"
            let leaking = (source.int("wallet_penalty_count") ?? 0) > 0
                || (source.int("tell_count") ?? 0) > 0
            rows.append(PrivacyFindingRow(
                id: "tx:\(transactionID ?? String(index))", kind: kind,
                severity: leaking ? "warning" : "info", title: kind,
                detail: transactionID ?? "", evidence: source.string("evidence_level") ?? "unknown",
                transactionID: transactionID, routesToSourceFunds: false
            ))
        }
        for (index, source) in object.objects("unknowns").enumerated() {
            let kind = source.string("code") ?? "unknown_coverage"
            rows.append(PrivacyFindingRow(
                id: "unknown:\(source.string("code", "source") ?? String(index))", kind: kind,
                severity: "info", title: source.string("title", "message", "code") ?? kind,
                detail: source.string("detail", "message") ?? "", evidence: source.string("evidence_level") ?? "unknown",
                transactionID: nil,
                routesToSourceFunds: ["source_proximity_coverage_gaps", "unknown_provenance"].contains(kind)
            ))
        }
        let coverage = object["coverage"]?.objectValue ?? [:]
        if coverage.bool("degraded") == true {
            rows.append(PrivacyFindingRow(
                id: "coverage:degraded", kind: "coverage_degraded", severity: "info",
                title: "coverage_degraded", detail: "", evidence: coverage.string("evidence_level") ?? "unknown",
                transactionID: nil, routesToSourceFunds: true
            ))
        }
        let rank = ["alert": 0, "warning": 1, "info": 2]
        return rows.sorted { (rank[$0.severity] ?? 2) < (rank[$1.severity] ?? 2) }
    }

    private static func census(_ findings: [PrivacyFindingRow]) -> PrivacySeverityCensus {
        PrivacySeverityCensus(
            alert: findings.count { $0.severity == "alert" },
            warning: findings.count { $0.severity == "warning" },
            info: findings.count { $0.severity != "alert" && $0.severity != "warning" }
        )
    }

    private static func fallbackFactors(_ census: PrivacySeverityCensus) -> [PrivacyScoreFactorRow] {
        [("alert", census.alert, -18), ("warning", census.warning, -9), ("info", census.info, -3)]
            .compactMap { key, count, penalty in
                guard count > 0 else { return nil }
                return PrivacyScoreFactorRow(
                    key: key, linked: nil, leaking: nil, total: count,
                    weight: nil, points: count * penalty
                )
            }
    }

    private static func adversaryRows(_ object: [String: JSONValue]) -> [PrivacyAdversaryCard] {
        object.objects("adversary_cards").enumerated().map { index, row in
            let summary = row["summary"]?.objectValue ?? [:]
            let unknown = summary["unknown_coverage"]?.objectValue ?? [:]
            return PrivacyAdversaryCard(
                tier: row.string("tier") ?? "observer-\(index)",
                label: row.string("label") ?? "",
                evidence: row.string("evidence_level") ?? "unknown",
                exposedClusterCount: Int(summary.int("exposed_cluster_count") ?? 0),
                walletCount: Int(summary.int("wallet_count") ?? 0),
                observerEntityCount: Int(summary.int("observer_entity_count") ?? 0),
                unknownCoverageStatus: unknown.string("status") ?? "",
                unknownNodeCount: Int(unknown.int("node_count") ?? 0),
                assumptions: row.objects("model_assumptions").enumerated().map { assumptionIndex, assumption in
                    PrivacyAdversaryAssumption(
                        code: assumption.string("code") ?? "assumption-\(assumptionIndex)",
                        statement: assumption.string("statement") ?? "",
                        evidence: assumption.string("evidence_level") ?? "unknown"
                    )
                }
            )
        }
    }

    private static func timelineRows(_ object: [String: JSONValue]) -> [PrivacyTimelineRow] {
        object.objects("timeline").enumerated().map { index, row in
            PrivacyTimelineRow(
                id: row.string("id") ?? "timeline-\(index)", kind: row.string("kind") ?? "",
                category: row.string("category") ?? "", transactionID: row.string("txid") ?? "",
                evidence: row.string("evidence_level") ?? "unknown", detail: row.string("detail") ?? "",
                newLinkage: row.bool("new_linkage") ?? false
            )
        }
    }

    private static func evidenceRows(_ object: [String: JSONValue]) -> [PrivacyEvidenceDrilldownRow] {
        object.objects("evidence_drilldowns").enumerated().map { index, row in
            let id = row.string("id") ?? "evidence-\(index)"
            let facts = row["evidence"]?.objectValue.map { flatten(.object($0)) } ?? []
            return PrivacyEvidenceDrilldownRow(
                id: "\(row.string("section") ?? "evidence"):\(id)",
                section: row.string("section") ?? "", kind: row.string("kind") ?? "",
                evidenceLevel: row.string("evidence_level") ?? "unknown", facts: facts
            )
        }
    }
}

public struct EgressRecordRow: Identifiable, Equatable, Sendable {
    public let id: Int64
    public let date: Date?
    public let subsystem: String
    public let endpoint: String
    public let operation: String
    public let bytesOut: Int64
    public let status: String
    public let proxy: Bool
}

@MainActor
@Observable
public final class EgressViewModel {
    public private(set) var records: [EgressRecordRow] = []
    public private(set) var unexpected = 0
    public private(set) var updateRequests = 0
    public private(set) var databaseClassification = ""
    public private(set) var databasePrefix = ""
    public private(set) var allowlistComplete = false
    public var actionableOnly = false
    public private(set) var errorMessage: String?
    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }
    public var visibleRecords: [EgressRecordRow] { actionableOnly ? records.filter { $0.status == "unexpected" } : records }

    public func load() async {
        do {
            let result = try await daemon.invoke(.uiEgressSnapshot, args: ["limit": .integer(1000)])
            if let error = result.error { errorMessage = error.message; return }
            let object = result.data?.objectValue ?? [:]
            let summary = object["summary"]?.objectValue ?? [:]
            unexpected = Int(summary.int("unexpected") ?? 0); updateRequests = Int(summary.int("update") ?? 0)
            allowlistComplete = object.bool("allowlist_complete") ?? false
            let db = object["db_header"]?.objectValue ?? [:]
            databaseClassification = db.string("classification", "format", "kind") ?? "unknown"
            databasePrefix = db.string("prefix_hex") ?? ""
            records = object.objects("records").map { row in
                let port = row.int("port").map(String.init) ?? ""
                return EgressRecordRow(id: row.int("id") ?? 0, date: DaemonValueParser.date(row.string("ts")),
                    subsystem: row.string("subsystem") ?? "unknown",
                    endpoint: row.string("host").map { port.isEmpty ? $0 : "\($0):\(port)" } ?? "",
                    operation: [row.string("method"), row.string("operation")].compactMap { $0 }.joined(separator: " "),
                    bytesOut: row.int("bytes_out") ?? 0, status: row.string("allowlist_status") ?? "unknown",
                    proxy: row.bool("via_proxy") ?? false)
            }
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }
}

public struct WorkspaceOverviewRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let currency: String
    public let transactionCount: Int
    public let walletCount: Int
    public let quarantines: Int
    public let ready: Bool
}

@MainActor
@Observable
public final class BirdsEyeViewModel {
    public private(set) var workspaceID = ""
    public private(set) var workspaceLabel = ""
    public private(set) var profiles: [WorkspaceOverviewRow] = []
    public private(set) var chartPoints: [DashboardPoint] = []
    public private(set) var chartTransactions: [TransactionRow] = []
    public private(set) var chartFiatCurrency = "EUR"
    public private(set) var chartMarketRate: Double?
    public private(set) var errorMessage: String?
    public private(set) var isRefreshing = false
    public private(set) var refreshProgress = 0.0
    public private(set) var refreshDetail = ""
    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }
    public func load(workspaceID requested: String? = nil) async {
        do {
            var workspace = requested
            if workspace == nil {
                let profiles = try await daemon.invoke(.uiProfilesSnapshot, args: nil)
                workspace = profiles.data?.objectValue?.string("activeWorkspaceId")
            }
            guard let workspace, !workspace.isEmpty else { profiles = []; return }
            let result = try await daemon.invoke(.uiWorkspaceOverviewSnapshot, args: ["workspace_id": .string(workspace)])
            if let error = result.error { errorMessage = error.message; return }
            let object = result.data?.objectValue ?? [:]
            workspaceID = workspace; workspaceLabel = object["workspace"]?.objectValue?.string("label") ?? ""
            chartFiatCurrency = object["fiat"]?.objectValue?.string("fiatCurrency", "fiat_currency") ?? "EUR"
            chartMarketRate = object["marketRate"]?.objectValue?.double("rate")
            chartTransactions = (object["activityTxs"]?.arrayValue ?? object["txs"]?.arrayValue ?? [])
                .compactMap(TransactionRow.init)
            chartPoints = object.objects("portfolioSeries", "portfolio_series").compactMap { row in
                guard let dateString = row.string("date"), let date = DaemonValueParser.date(dateString) else { return nil }
                return DashboardPoint(
                    id: dateString,
                    date: date,
                    balanceBTC: row.double("balanceBtc", "balance_btc") ?? 0,
                    fiatValue: row.double("valueEur", "fiat_value") ?? 0,
                    costBasisEUR: row.double("costBasisEur", "cost_basis_eur") ?? 0,
                    priceEUR: row.double("priceEur", "price_eur")
                )
            }
            profiles = object.objects("books").compactMap { row in
                let profile = row["profile"]?.objectValue ?? row
                guard let id = profile.string("id") else { return nil }
                let status = row["status"]?.objectValue ?? [:]
                let readiness = row["readiness"]?.objectValue ?? [:]
                return WorkspaceOverviewRow(id: id, label: profile.string("label", "name") ?? id,
                    currency: profile.string("fiatCurrency", "fiat_currency") ?? "",
                    transactionCount: Int(status.int("transactionCount", "transaction_count") ?? row.int("transactionCount") ?? 0),
                    walletCount: Int(status.int("walletCount", "wallet_count") ?? row.int("walletCount") ?? 0),
                    quarantines: Int(status.int("quarantines") ?? row.int("quarantines") ?? 0), ready: readiness.bool("ready") ?? false)
            }
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }
    public func switchBook(_ profileID: String) async {
        do {
            let result = try await daemon.invoke(.uiProfilesSwitch, args: ["profile_id": .string(profileID)])
            if let error = result.error { errorMessage = error.message; return }
            await load(workspaceID: workspaceID)
        } catch { errorMessage = String(describing: error) }
    }
    public func refreshWorkspace() async {
        guard !workspaceID.isEmpty else { return }
        isRefreshing = true; refreshProgress = 0; refreshDetail = ""
        defer { isRefreshing = false }
        do {
            let records = try await daemon.stream(.uiWorkspaceFreshnessRun, args: ["workspace_id": .string(workspaceID)])
            for try await record in records {
                let row = record.data?.objectValue ?? [:]
                refreshDetail = [row["profile"]?.objectValue?.string("label"), row.string("phase")].compactMap { $0 }.joined(separator: " · ")
                if let processed = row.double("processed"), let total = row.double("total"), total > 0 { refreshProgress = min(1, processed / total) }
                if let error = record.error { errorMessage = error.message }
            }
            await load(workspaceID: workspaceID)
        } catch { errorMessage = String(describing: error) }
    }
}

public struct KeyValueRow: Identifiable, Equatable, Sendable {
    public let key: String
    public let value: String
    public var id: String { key }
}

private func display(_ value: JSONValue?) -> String {
    guard let value else { return "—" }
    switch value {
    case let .string(value): return value
    case let .integer(value): return String(value)
    case let .unsignedInteger(value): return String(value)
    case let .number(value): return String(format: "%.4f", value)
    case let .bool(value): return value ? "yes" : "no"
    case .null: return "—"
    case let .array(values): return values.map { display($0) }.joined(separator: ", ")
    case let .object(values): return values.sorted(by: { $0.key < $1.key }).map { "\($0.key): \(display($0.value))" }.joined(separator: " · ")
    }
}

private func flatten(_ value: JSONValue?, prefix: String = "") -> [KeyValueRow] {
    guard let value else { return [] }
    if case let .object(object) = value {
        return object.sorted(by: { $0.key < $1.key }).flatMap { key, child in
            flatten(child, prefix: prefix.isEmpty ? key : "\(prefix).\(key)")
        }
    }
    return [KeyValueRow(key: prefix, value: display(value))]
}
