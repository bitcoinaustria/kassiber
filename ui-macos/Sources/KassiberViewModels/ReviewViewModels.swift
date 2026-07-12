import Foundation
import Observation
import KassiberDaemonKit

public struct JournalEntryTypeRow: Identifiable, Equatable, Sendable {
    public let type: String
    public let count: Int
    public let gainLoss: Double
    public var id: String { type }
}

public struct JournalEntryRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let transactionID: String
    public let occurredAt: Date?
    public let type: String
    public let wallet: String
    public let asset: String
    public let quantityMSat: Int64
    public let fiatValue: Double
    public let gainLoss: Double
    public let description: String
}

@MainActor
@Observable
public final class JournalsViewModel {
    public private(set) var workspace = ""
    public private(set) var profile = ""
    public private(set) var transactionCount = 0
    public private(set) var entryCount = 0
    public private(set) var quarantineCount = 0
    public private(set) var needsProcessing = false
    public private(set) var lastProcessedAt: Date?
    public private(set) var entryTypes: [JournalEntryTypeRow] = []
    public private(set) var entries: [JournalEntryRow] = []
    public var selectedType: String?
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient

    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public var visibleEntries: [JournalEntryRow] {
        guard let selectedType else { return entries }
        return entries.filter { $0.type == selectedType }
    }

    public func load() async {
        isLoading = true
        defer { isLoading = false }
        do {
            async let snapshotEnvelope = daemon.invoke(.uiJournalsSnapshot, args: nil)
            async let eventsEnvelope = daemon.invoke(
                .uiJournalsEventsList,
                args: ["limit": .integer(250)]
            )
            let (snapshot, events) = try await (snapshotEnvelope, eventsEnvelope)
            if let error = snapshot.error ?? events.error {
                errorMessage = error.message
                return
            }
            parseSnapshot(snapshot.data)
            parseEvents(events.data)
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    private func parseSnapshot(_ data: JSONValue?) {
        guard let object = data?.objectValue else { return }
        let status = object["status"]?.objectValue ?? [:]
        workspace = status.string("workspace") ?? ""
        profile = status.string("profile") ?? ""
        transactionCount = Int(status.int("transactionCount", "transaction_count") ?? 0)
        entryCount = Int(status.int("journalEntryCount", "journal_entry_count") ?? 0)
        quarantineCount = Int(status.int("quarantines") ?? 0)
        needsProcessing = status.bool("needsJournals", "needs_journals") ?? false
        lastProcessedAt = DaemonValueParser.date(status.string("lastProcessedAt", "last_processed_at"))
        entryTypes = object.objects("entryTypes", "entry_types").compactMap { row in
            guard let type = row.string("type") else { return nil }
            return JournalEntryTypeRow(
                type: type,
                count: Int(row.int("count") ?? 0),
                gainLoss: row.double("gainLossEur", "gain_loss") ?? 0
            )
        }
    }

    private func parseEvents(_ data: JSONValue?) {
        guard let object = data?.objectValue else { return }
        entries = object.objects("events").compactMap { row in
            let id = row.string("id") ?? ""
            guard !id.isEmpty else { return nil }
            return JournalEntryRow(
                id: id,
                transactionID: row.string("transactionId", "transaction_id") ?? "",
                occurredAt: DaemonValueParser.date(row.string("occurredAt", "occurred_at")),
                type: row.string("entryType", "entry_type") ?? "",
                wallet: row.string("wallet") ?? "",
                asset: row.string("asset") ?? "BTC",
                quantityMSat: row.int("quantityMsat", "quantity_msat") ?? 0,
                fiatValue: row.double("fiatValueEur", "fiat_value") ?? 0,
                gainLoss: row.double("gainLossEur", "gain_loss") ?? 0,
                description: row.string("description") ?? ""
            )
        }
    }
}

public struct QuarantineReasonRow: Identifiable, Equatable, Sendable {
    public let reason: String
    public let count: Int
    public var id: String { reason }
}

public struct QuarantineItemRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let externalID: String
    public let occurredAt: Date?
    public let wallet: String
    public let direction: String
    public let asset: String
    public let amountMSat: Int64
    public let feeMSat: Int64
    public let reason: String
    public let detail: String
}

@MainActor
@Observable
public final class QuarantineViewModel {
    public private(set) var items: [QuarantineItemRow] = []
    public private(set) var reasons: [QuarantineReasonRow] = []
    public var selectedReason: String?
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public var visibleItems: [QuarantineItemRow] {
        guard let selectedReason else { return items }
        return items.filter { $0.reason == selectedReason }
    }

    public func load() async {
        isLoading = true
        defer { isLoading = false }
        do {
            let envelope = try await daemon.invoke(
                .uiJournalsQuarantine,
                args: ["limit": .integer(100)]
            )
            if let error = envelope.error {
                errorMessage = error.message
                return
            }
            guard let object = envelope.data?.objectValue else { return }
            let summary = object["summary"]?.objectValue ?? [:]
            reasons = summary.objects("by_reason").compactMap { row in
                guard let reason = row.string("reason") else { return nil }
                return QuarantineReasonRow(reason: reason, count: Int(row.int("count") ?? 0))
            }
            items = object.objects("items").compactMap { row in
                let id = row.string("transaction_id") ?? ""
                guard !id.isEmpty else { return nil }
                let detail = row["detail"]?.objectValue?.sorted(by: { $0.key < $1.key }).compactMap { key, value -> String? in
                    switch value {
                    case let .string(text): "\(key): \(text)"
                    case let .integer(number): "\(key): \(number)"
                    case let .number(number): "\(key): \(number)"
                    default: nil
                    }
                }.joined(separator: " · ") ?? ""
                return QuarantineItemRow(
                    id: id,
                    externalID: row.string("external_id") ?? "",
                    occurredAt: DaemonValueParser.date(row.string("occurred_at")),
                    wallet: row.string("wallet") ?? "",
                    direction: row.string("direction") ?? "",
                    asset: row.string("asset") ?? "BTC",
                    amountMSat: row.int("amount_msat") ?? 0,
                    feeMSat: row.int("fee_msat") ?? 0,
                    reason: row.string("reason") ?? "unknown",
                    detail: detail
                )
            }
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }
}

public struct TransferCandidateRow: Identifiable, Equatable, Sendable {
    public var id: String { outID + "→" + inID }
    public let outID: String
    public let inID: String
    public let outWallet: String
    public let inWallet: String
    public let outAsset: String
    public let inAsset: String
    public let outAmountMSat: Int64
    public let inAmountMSat: Int64
    public let feeMSat: Int64
    public let confidence: String
    public let method: String
    public let kind: String
    public let policy: String
    public let conflictSize: Int
}

public struct PairedTransferRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let outWallet: String
    public let inWallet: String
    public let outAsset: String
    public let inAsset: String
    public let outAmountMSat: Int64
    public let inAmountMSat: Int64
    public let feeMSat: Int64
    public let kind: String
    public let policy: String
    public let source: String
}

public struct SwapRuleRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let kind: String
    public let policy: String
    public let enabled: Bool
    public let predicate: String
}

public struct SwapSavedViewRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let filter: String
}

@MainActor
@Observable
public final class SwapsViewModel {
    public enum Tab: String, CaseIterable, Identifiable { case review, paired, rules; public var id: String { rawValue } }
    public var tab: Tab = .review
    public private(set) var candidates: [TransferCandidateRow] = []
    public private(set) var pairs: [PairedTransferRow] = []
    public private(set) var rules: [SwapRuleRow] = []
    public private(set) var savedViews: [SwapSavedViewRow] = []
    public private(set) var exactCount = 0
    public private(set) var conflictCount = 0
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?
    public private(set) var actionMessage: String?

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func load() async {
        isLoading = true
        defer { isLoading = false }
        do {
            async let suggestions = daemon.invoke(.uiTransfersSuggest, args: nil)
            async let paired = daemon.invoke(.uiTransfersList, args: nil)
            async let rulesCall = daemon.invoke(.uiTransfersRulesList, args: nil)
            async let viewsCall = daemon.invoke(.uiSavedViewsList, args: ["surface": .string("swap_candidates")])
            let (suggestionsResult, pairsResult, rulesResult, viewsResult) = try await (suggestions, paired, rulesCall, viewsCall)
            if let error = suggestionsResult.error ?? pairsResult.error ?? rulesResult.error ?? viewsResult.error {
                errorMessage = error.message
                return
            }
            parseCandidates(suggestionsResult.data)
            parsePairs(pairsResult.data)
            rules = rulesResult.data?.objectValue?.objects("rules").compactMap { row in
                guard let id = row.string("id") else { return nil }
                return SwapRuleRow(id: id, name: row.string("name") ?? id, kind: row.string("kind") ?? "",
                    policy: row.string("policy") ?? "", enabled: row.bool("enabled") ?? true,
                    predicate: row["predicate"]?.objectValue?.sorted(by: { $0.key < $1.key }).map { "\($0.key)=\($0.value.stringValue ?? String(describing: $0.value))" }.joined(separator: " · ") ?? "")
            } ?? []
            savedViews = viewsResult.data?.objectValue?.objects("views").compactMap { row in
                guard let id = row.string("id") else { return nil }
                return SwapSavedViewRow(id: id, name: row.string("name") ?? id,
                    filter: row["filter"]?.objectValue?.keys.sorted().joined(separator: ", ") ?? "")
            } ?? []
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    public func pair(_ candidate: TransferCandidateRow) async {
        await mutate(.uiTransfersPair, args: [
            "tx_out": .string(candidate.outID), "tx_in": .string(candidate.inID),
            "kind": .string(candidate.kind), "policy": .string(candidate.policy),
            "pair_source": .string("manual"), "confidence_at_pair": .string(candidate.confidence),
        ], message: "paired")
    }

    public func dismiss(_ candidate: TransferCandidateRow) async {
        await mutate(.uiTransfersDismiss, args: [
            "tx_out": .string(candidate.outID), "tx_in": .string(candidate.inID),
            "reason": .string("Dismissed from native review"),
        ], message: "dismissed")
    }

    public func pairAllExact() async {
        await mutate(.uiTransfersBulkPair, args: ["confidence": .string("exact")], message: "bulk_paired")
    }

    public func unpair(_ pair: PairedTransferRow) async {
        await mutate(.uiTransfersUnpair, args: ["pair_id": .string(pair.id)], message: "unpaired")
    }

    public func update(_ pair: PairedTransferRow, kind: String, policy: String) async {
        await mutate(.uiTransfersUpdate, args: [
            "pair_id": .string(pair.id), "kind": .string(kind), "policy": .string(policy),
        ], message: "updated")
    }

    public func createRule(name: String, confidence: String, kind: String, policy: String) async {
        await mutate(.uiTransfersRulesCreate, args: [
            "name": .string(name), "predicate": .object(["confidence": .string(confidence)]),
            "kind": .string(kind), "policy": .string(policy), "enabled": .bool(true),
        ], message: "rule_created")
    }
    public func toggle(_ rule: SwapRuleRow) async { await mutate(.uiTransfersRulesSetEnabled, args: ["rule_id": .string(rule.id), "enabled": .bool(!rule.enabled)], message: "rule_updated") }
    public func delete(_ rule: SwapRuleRow) async { await mutate(.uiTransfersRulesDelete, args: ["rule_id": .string(rule.id)], message: "rule_deleted") }
    public func applyRules() async { await mutate(.uiTransfersRulesApply, args: [:], message: "rules_applied") }
    public func createSavedView(name: String) async { await mutate(.uiSavedViewsCreate, args: ["surface": .string("swap_candidates"), "name": .string(name), "filter": .object([:])], message: "view_created") }
    public func delete(_ view: SwapSavedViewRow) async { await mutate(.uiSavedViewsDelete, args: ["view_id": .string(view.id)], message: "view_deleted") }

    private func mutate(_ kind: DaemonKind, args: [String: JSONValue], message: String) async {
        isLoading = true
        defer { isLoading = false }
        do {
            let result = try await daemon.invoke(kind, args: args)
            if let error = result.error { errorMessage = error.message; return }
            actionMessage = message
            await load()
        } catch { errorMessage = String(describing: error) }
    }

    private func parseCandidates(_ data: JSONValue?) {
        guard let object = data?.objectValue else { return }
        let counts = object["counts"]?.objectValue ?? [:]
        exactCount = Int(counts.int("exact") ?? 0)
        conflictCount = Int(counts.int("conflicts") ?? 0)
        candidates = object.objects("candidates").compactMap { row in
            guard let outID = row.string("out_id"), let inID = row.string("in_id") else { return nil }
            return TransferCandidateRow(
                outID: outID,
                inID: inID,
                outWallet: row.string("out_wallet_label", "out_wallet") ?? "",
                inWallet: row.string("in_wallet_label", "in_wallet") ?? "",
                outAsset: row.string("out_asset") ?? "BTC",
                inAsset: row.string("in_asset") ?? "BTC",
                outAmountMSat: row.int("out_amount_msat") ?? 0,
                inAmountMSat: row.int("in_amount_msat") ?? 0,
                feeMSat: row.int("swap_fee_msat") ?? 0,
                confidence: row.string("confidence") ?? "",
                method: row.string("method") ?? "",
                kind: row.string("default_kind") ?? "",
                policy: row.string("default_policy") ?? "",
                conflictSize: Int(row.int("conflict_size") ?? 1)
            )
        }
    }

    private func parsePairs(_ data: JSONValue?) {
        guard let object = data?.objectValue else { return }
        pairs = object.objects("pairs").compactMap { row in
            guard let id = row.string("id") else { return nil }
            let out = row["out"]?.objectValue ?? [:]
            let incoming = row["in"]?.objectValue ?? [:]
            return PairedTransferRow(
                id: id,
                outWallet: out.string("wallet") ?? "",
                inWallet: incoming.string("wallet") ?? "",
                outAsset: out.string("asset") ?? "BTC",
                inAsset: incoming.string("asset") ?? "BTC",
                outAmountMSat: out.int("amount_msat") ?? 0,
                inAmountMSat: incoming.int("amount_msat") ?? 0,
                feeMSat: row.int("swap_fee_msat") ?? 0,
                kind: row.string("kind") ?? "",
                policy: row.string("policy") ?? "",
                source: row.string("pair_source") ?? ""
            )
        }
    }
}

public struct ReconcileResultRow: Identifiable, Equatable, Sendable {
    public var id: String { input }
    public let input: String
    public let type: String
    public let chain: String
    public let status: String
    public let classification: String
    public let wallets: [String]
    public let branch: String
    public let note: String
}

@MainActor
@Observable
public final class ReconcileViewModel {
    public var input = ""
    public var csvText = ""
    public private(set) var csvName = ""
    public private(set) var results: [ReconcileResultRow] = []
    public private(set) var owned = 0
    public private(set) var external = 0
    public private(set) var unknown = 0
    public private(set) var invalid = 0
    public private(set) var walletsScanned = 0
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public var resultsCSV: String {
        let header = ["input", "type", "chain", "status", "classification", "wallet", "branch", "note"]
        let lines = results.map { row in
            [
                row.input, row.type, row.chain, row.status, row.classification,
                row.wallets.joined(separator: "; "), row.branch, row.note,
            ].map(Self.csvCell).joined(separator: ",")
        }
        return ([header.joined(separator: ",")] + lines).joined(separator: "\n")
    }

    public func check(onchain: Bool = false) async {
        let text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !csvText.isEmpty else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            var args: [String: JSONValue] = [:]
            if !text.isEmpty { args["text"] = .string(text) }
            if !csvText.isEmpty { args["csv_text"] = .string(csvText) }
            let envelope = try await daemon.invoke(onchain ? .uiWalletsIdentifyOnchain : .uiWalletsIdentify, args: args)
            if let error = envelope.error {
                errorMessage = error.message
                return
            }
            guard let object = envelope.data?.objectValue else { return }
            let summary = object["summary"]?.objectValue ?? [:]
            owned = Int(summary.int("owned") ?? 0)
            external = Int(summary.int("external") ?? 0)
            unknown = Int(summary.int("unknown") ?? 0)
            invalid = Int(summary.int("invalid") ?? 0)
            walletsScanned = Int(summary.int("wallets_scanned") ?? 0)
            results = object.objects("results").compactMap { row in
                guard let input = row.string("input") else { return nil }
                let matches = row.objects("matches")
                var wallets = row["wallets"]?.arrayValue?.compactMap(\.stringValue) ?? []
                wallets.append(contentsOf: matches.compactMap { $0.string("wallet") })
                let primaryMatch = matches.first
                let branchName = primaryMatch?.string("branch") ?? row.string("branch") ?? ""
                let branchIndex = primaryMatch?.int("address_index") ?? row.int("address_index")
                let branch = branchIndex.map { "\(branchName.isEmpty ? "address" : branchName) #\($0)" }
                    ?? branchName
                return ReconcileResultRow(
                    input: input,
                    type: row.string("type") ?? "",
                    chain: row.string("chain") ?? "",
                    status: row.string("status") ?? "unknown",
                    classification: row.string("classification") ?? "",
                    wallets: Array(Set(wallets)).sorted(),
                    branch: branch,
                    note: row.string("note") ?? ""
                )
            }
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    public func loadCSV(name: String, text: String) {
        csvName = name
        csvText = text
    }

    private static func csvCell(_ value: String) -> String {
        guard value.contains(where: { $0 == "," || $0 == "\"" || $0.isNewline }) else { return value }
        return "\"\(value.replacingOccurrences(of: "\"", with: "\"\""))\""
    }
}
