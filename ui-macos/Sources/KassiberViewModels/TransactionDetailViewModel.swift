import Foundation
import Observation
import KassiberDaemonKit

public struct AttachmentRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let type: String
    public let reference: String
}

public struct HistoryRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let changedAt: Date?
    public let source: String
    public let summary: String
    public let detail: String
}

public struct TransactionPricingOption: Identifiable, Equatable, Sendable {
    public let id: String
    public let sourceKind: String?
    public let quality: String

    public init(id: String, sourceKind: String?, quality: String) {
        self.id = id
        self.sourceKind = sourceKind
        self.quality = quality
    }
}

public struct TransactionTaxOption: Identifiable, Equatable, Sendable {
    public let id: String
    public let regime: String
    public let category: String
    public let taxable: Bool

    public init(regime: String, category: String, taxable: Bool) {
        id = "\(regime):\(category)"
        self.regime = regime
        self.category = category
        self.taxable = taxable
    }
}

public struct TransactionJournalEvent: Identifiable, Equatable, Sendable {
    public let id: String
    public let entryType: String
    public let asset: String
    public let quantity: Double
    public let fiatValue: Double
    public let costBasis: Double?
    public let proceeds: Double?
    public let gainLoss: Double?
    public let atCategory: String?
    public let description: String

    init?(_ row: [String: JSONValue]) {
        guard let id = row.string("id"), !id.isEmpty else { return nil }
        self.id = id
        entryType = row.string("entryType", "entry_type") ?? ""
        asset = row.string("asset") ?? "BTC"
        quantity = row.double("quantity") ?? 0
        fiatValue = row.double("fiatValueEur", "fiat_value") ?? 0
        costBasis = row.double("costBasisEur", "cost_basis")
        proceeds = row.double("proceedsEur", "proceeds")
        gainLoss = row.double("gainLossEur", "gain_loss")
        atCategory = row.string("atCategory", "at_category")
        description = row.string("description") ?? ""
    }
}

public enum TransactionTaxEffectState: String, Equatable, Sendable {
    case pending
    case acquisition
    case income
    case disposal
    case transfer
}

public struct TransactionTaxEffect: Equatable, Sendable {
    public let state: TransactionTaxEffectState
    public let costBasis: Double?
    public let proceeds: Double?
    public let gainLoss: Double?
}

public struct TransactionLoanMark: Identifiable, Equatable, Sendable {
    public let transactionID: String
    public let loanID: String?
    public let role: String
    public let occurredAt: Date?
    public let description: String

    public var id: String { transactionID }

    init?(_ row: [String: JSONValue]) {
        guard let transactionID = row.string("transaction_id", "txid"), !transactionID.isEmpty else { return nil }
        self.transactionID = transactionID
        loanID = row.string("loan_id")
        role = row.string("role", "mark_as", "as") ?? ""
        occurredAt = DaemonValueParser.date(row.string("occurred_at"))
        description = row.string("description", "note") ?? ""
    }
}

@MainActor
@Observable
public final class TransactionResolverViewModel {
    public private(set) var transaction: TransactionRow?
    public private(set) var errorMessage: String?
    private let daemon: any DaemonClient
    private let reference: String
    public init(daemon: any DaemonClient, reference: String) { self.daemon = daemon; self.reference = reference }
    public func load() async {
        do {
            let result = try await daemon.invoke(.uiTransactionsResolve, args: ["query": .string(reference)])
            if let error = result.error { errorMessage = error.message; return }
            if let value = result.data?.objectValue?["transaction"] { transaction = TransactionRow(value) }
            if transaction == nil { errorMessage = "Transaction not found." }
        } catch { errorMessage = String(describing: error) }
    }
}

@MainActor
@Observable
public final class TransactionDetailViewModel {
    public static let classificationOptions = [
        "Unlabeled", "Income", "Expense", "Transfer", "Swap", "Fee",
        "Merchant payment", "Gift", "Review", "Other",
    ]
    public static let tagSuggestions = [
        "Revenue", "BTCPay", "needs invoice", "client ACME", "Hosting", "Capex",
        "Meals", "Bank fees", "Consolidation", "Liquid", "Lightning",
        "manual review", "accountant",
    ]
    public static let reviewStatuses = ["completed", "pending", "failed", "review"]
    public static let pricingOptions = [
        TransactionPricingOption(id: "generic_import", sourceKind: "generic_import", quality: "exact"),
        TransactionPricingOption(id: "fmv_provider", sourceKind: "fmv_provider", quality: "provider_sample"),
        TransactionPricingOption(id: "manual_override", sourceKind: "manual_override", quality: "exact"),
        TransactionPricingOption(id: "missing", sourceKind: nil, quality: "missing"),
    ]
    public static let taxOptions = [
        TransactionTaxOption(regime: "neu", category: "neu_gain", taxable: true),
        TransactionTaxOption(regime: "neu", category: "income_general", taxable: true),
        TransactionTaxOption(regime: "neu", category: "neu_loss", taxable: true),
        TransactionTaxOption(regime: "outside", category: "none", taxable: false),
        TransactionTaxOption(regime: "neu", category: "neu_swap", taxable: false),
        TransactionTaxOption(regime: "alt", category: "alt_spekulation", taxable: true),
        TransactionTaxOption(regime: "alt", category: "alt_taxfree", taxable: false),
    ]

    public let transaction: TransactionRow
    public var classification: String
    public var note: String
    public var tags: String
    public var reviewStatus: String
    public var excluded: Bool
    public var taxable: Bool
    public var atRegime: String
    public var atCategory: String
    public private(set) var taxSelection: String
    public private(set) var pricingSelection: String
    public private(set) var pricingSourceKind: String?
    public private(set) var pricingQuality: String
    public var manualCurrency: String
    public var manualPrice: String
    public var manualValue: String
    public var manualSource: String
    public private(set) var attachments: [AttachmentRow] = []
    public private(set) var history: [HistoryRow] = []
    public private(set) var graph: [KeyValueRow] = []
    public private(set) var graphSnapshot: TransactionGraphSnapshot
    public private(set) var graphIsLoading = false
    public private(set) var selectedGraphRouteLeg: String?
    public private(set) var privacyContext = TransactionPrivacyContext.loading
    public private(set) var privacyIsLoading = false
    public private(set) var commercialContext: [KeyValueRow] = []
    public private(set) var journalEvents: [TransactionJournalEvent] = []
    public private(set) var journalNeedsProcessing = false
    public private(set) var loanMarks: [TransactionLoanMark] = []
    public private(set) var loanRole: String?
    public private(set) var pair: TransactionPairRow?
    public private(set) var isWorking = false
    public private(set) var isUnpairing = false
    public private(set) var errorMessage: String?
    public private(set) var didSave = false
    private let daemon: any DaemonClient
    private var baselineClassification = ""
    private var baselineNote = ""
    private var baselineTags: [String] = []
    private var baselineReviewStatus = ""
    private var baselineExcluded = false
    private var baselineTaxable = false
    private var baselineRegime = ""
    private var baselineCategory = ""
    private var baselinePricing = PricingBaseline()

    private struct PricingBaseline: Equatable {
        var sourceKind: String? = nil
        var quality = "missing"
        var currency = "EUR"
        var price = ""
        var value = ""
        var source = ""
    }

    public init(daemon: any DaemonClient, transaction: TransactionRow) {
        self.daemon = daemon
        self.transaction = transaction

        let persistedClassification = Self.classificationOptions.first {
            $0 != "Unlabeled" && transaction.tags.contains($0)
        }
        classification = persistedClassification ?? "Unlabeled"
        note = transaction.note
        tags = transaction.tags
            .filter { $0 != persistedClassification }
            .joined(separator: ", ")
        reviewStatus = transaction.reviewStatus
        excluded = transaction.excluded

        let defaultTax = Self.defaultTaxOption(for: transaction)
        let taxOption = Self.taxOptions.first {
            $0.regime == transaction.atRegime && $0.category == transaction.atCategory
        } ?? defaultTax
        let initialRegime = transaction.atRegime ?? taxOption.regime
        let initialCategory = transaction.atCategory ?? taxOption.category
        atRegime = initialRegime
        atCategory = initialCategory
        taxable = transaction.taxable ?? taxOption.taxable
        taxSelection = "\(initialRegime):\(initialCategory)"

        let initialSourceKind = transaction.pricingSourceKind ?? (transaction.rate == nil ? nil : "generic_import")
        let initialPricingQuality = transaction.pricingQuality ?? (transaction.rate == nil ? "missing" : "exact")
        pricingSourceKind = initialSourceKind
        pricingQuality = initialPricingQuality
        pricingSelection = Self.pricingSelection(sourceKind: initialSourceKind, quality: initialPricingQuality)
        manualCurrency = transaction.fiatCurrency
        manualPrice = transaction.rate.map { String($0) } ?? ""
        manualValue = transaction.fiatValue.map { String($0) } ?? ""
        manualSource = transaction.pricingExternalRef ?? ""
        pair = transaction.pair
        graphSnapshot = TransactionGraphSnapshot(nil, pairFallback: transaction.pair, transaction: transaction)
        selectedGraphRouteLeg = graphSnapshot.swapRoute?.currentLeg
        captureBaseline()
    }

    public var tagValues: [String] {
        var values = splitTags(tags)
        if classification != "Unlabeled", !classification.isEmpty {
            values.insert(classification, at: 0)
        }
        return uniqueTags(values)
    }

    public var availableTagSuggestions: [String] {
        let existing = Set(tagValues)
        return Self.tagSuggestions.filter { !existing.contains($0) }
    }

    public var amountBTC: Double {
        abs(Double(transaction.amountSats)) / 100_000_000
    }

    public var pricingIsMissing: Bool {
        pricingSourceKind == nil || pricingQuality == "missing" || manualValue.isEmpty
    }

    public var pricingHasCacheProvenance: Bool {
        transaction.pricingProvider != nil || transaction.pricingPair != nil
    }

    public var pricingMoment: Date? {
        guard let timestamp = transaction.pricingTimestamp,
              let parsed = DaemonValueParser.date(timestamp) else { return nil }
        if transaction.pricingGranularity == "daily",
           transaction.pricingProvider == "kraken-csv",
           transaction.pricingMethod == "ohlcvt_csv" {
            return Calendar(identifier: .iso8601).date(byAdding: .day, value: -1, to: parsed)
        }
        return parsed
    }

    public var pricingMomentIsTradingDay: Bool {
        transaction.pricingGranularity == "daily"
    }

    public var pricingFetchedMoment: Date? {
        DaemonValueParser.date(transaction.pricingFetchedAt)
    }

    public var isBasisQuarantine: Bool {
        let reason = transaction.quarantineReason?.lowercased() ?? ""
        return reason.contains("basis") || reason.contains("lot") || reason.contains("insufficient")
    }

    public var canUnpair: Bool {
        pair != nil && pair?.kind != "journal-derived"
    }

    public var currentLoanMark: TransactionLoanMark? {
        loanMarks.first { mark in
            mark.transactionID == transaction.id || mark.transactionID == transaction.transactionID
        }
    }

    public var linkedLoanMarks: [TransactionLoanMark] {
        guard let current = currentLoanMark, let loanID = current.loanID else { return [] }
        return loanMarks.filter { $0.transactionID != current.transactionID && $0.loanID == loanID }
    }

    public var loanLinkCandidates: [TransactionLoanMark] {
        guard let current = currentLoanMark else { return [] }
        return loanMarks.filter { $0.transactionID != current.transactionID && $0.loanID != current.loanID }
    }

    public var loanMarkOptions: [String] {
        transaction.amountSats < 0
            ? ["collateral_lock", "loan_principal_repaid"]
            : ["collateral_release", "loan_principal_received"]
    }

    public var taxEffect: TransactionTaxEffect {
        if journalEvents.isEmpty {
            return TransactionTaxEffect(state: .pending, costBasis: nil, proceeds: nil, gainLoss: nil)
        }
        if journalEvents.contains(where: { ["transfer_out", "transfer_in"].contains($0.entryType) })
            || transaction.flow == .transfer {
            return TransactionTaxEffect(state: .transfer, costBasis: nil, proceeds: nil, gainLoss: nil)
        }
        let disposals = journalEvents.filter {
            ["disposal", "fee", "transfer_fee", "neutral_swap"].contains($0.entryType)
        }
        if !disposals.isEmpty {
            return TransactionTaxEffect(
                state: .disposal,
                costBasis: sumJournal(disposals) { [$0.costBasis] },
                proceeds: sumJournal(disposals) { [$0.proceeds, $0.fiatValue] },
                gainLoss: sumJournal(disposals) { [$0.gainLoss] }
            )
        }
        let income = journalEvents.filter { $0.entryType == "income" }
        if !income.isEmpty {
            return TransactionTaxEffect(
                state: .income,
                costBasis: sumJournal(income) { [$0.costBasis] },
                proceeds: sumJournal(income) { [$0.proceeds, $0.fiatValue] },
                gainLoss: sumJournal(income) { [$0.gainLoss] }
            )
        }
        let acquisitions = journalEvents.filter { $0.entryType == "acquisition" || $0.quantity > 0 }
        if !acquisitions.isEmpty {
            return TransactionTaxEffect(
                state: .acquisition,
                costBasis: sumJournal(acquisitions) { [$0.fiatValue] },
                proceeds: nil,
                gainLoss: nil
            )
        }
        return TransactionTaxEffect(state: .pending, costBasis: nil, proceeds: nil, gainLoss: nil)
    }

    public var hasChanges: Bool {
        classification != baselineClassification
            || note != baselineNote
            || tagValues != baselineTags
            || reviewStatus != baselineReviewStatus
            || excluded != baselineExcluded
            || taxable != baselineTaxable
            || atRegime != baselineRegime
            || atCategory != baselineCategory
            || currentPricingBaseline != baselinePricing
    }

    public func selectPricing(_ selection: String) {
        guard let option = Self.pricingOptions.first(where: { $0.id == selection }) else { return }
        pricingSelection = option.id
        pricingSourceKind = option.sourceKind
        pricingQuality = option.quality
    }

    public func chooseExactManualPrice() {
        selectPricing("manual_override")
    }

    public func updateManualPrice(_ rawPrice: String) {
        chooseExactManualPrice()
        manualPrice = rawPrice
        if let price = parseManualDecimal(rawPrice), amountBTC > 0 {
            manualValue = String(format: "%.2f", price * amountBTC)
        } else {
            manualValue = ""
        }
    }

    public func updateManualValue(_ rawValue: String) {
        chooseExactManualPrice()
        manualValue = rawValue
        if let value = parseManualDecimal(rawValue), amountBTC > 0 {
            manualPrice = String(format: "%.2f", value / amountBTC)
        } else {
            manualPrice = ""
        }
    }

    public func selectTaxTreatment(_ selection: String) {
        guard let option = Self.taxOptions.first(where: { $0.id == selection }) else { return }
        taxSelection = option.id
        atRegime = option.regime
        atCategory = option.category
        taxable = option.taxable
    }

    public func addSuggestedTag(_ tag: String) {
        tags = uniqueTags(splitTags(tags) + [tag]).joined(separator: ", ")
    }

    public func load() async {
        do {
            async let attachmentsCall = daemon.invoke(.uiAttachmentsList, args: ["transaction": .string(transaction.id)])
            async let historyCall = daemon.invoke(.uiTransactionsHistory, args: ["transaction": .string(transaction.id), "limit": .integer(100)])
            async let graphCall = daemon.invoke(.uiTransactionsGraph, args: ["transaction": .string(transaction.id), "allowPublicLookup": .bool(false)])
            async let loansCall = daemon.invoke(.uiLoansList, args: nil)
            async let commercialCall = daemon.invoke(.uiTransactionsCommercialContext, args: ["transaction": .string(transaction.id)])
            async let journalCall = daemon.invoke(.uiJournalsEventsList, args: ["transaction": .string(transaction.id), "limit": .integer(20)])
            let (attachmentsResult, historyResult, graphResult, loansResult, commercialResult, journalResult) = try await (
                attachmentsCall, historyCall, graphCall, loansCall, commercialCall, journalCall
            )
            attachments = (attachmentsResult.data?.objectValue?.objects("attachments") ?? []).compactMap { row in
                guard let id = row.string("id") else { return nil }
                return AttachmentRow(id: id, label: row.string("label", "filename") ?? id,
                    type: row.string("attachment_type", "media_type") ?? "file", reference: row.string("url", "original_name") ?? "")
            }
            history = (historyResult.data?.objectValue?.objects("events") ?? []).compactMap { row in
                guard let id = row.string("id") else { return nil }
                let detail = row.objects("fields").map { field in
                    "\(field.string("label", "field") ?? ""): \(detailDisplay(field["before_label"] ?? field["before_value"])) → \(detailDisplay(field["after_label"] ?? field["after_value"]))"
                }.joined(separator: " · ")
                return HistoryRow(id: id, changedAt: DaemonValueParser.date(row.string("changed_at")), source: row.string("source_label", "source") ?? "", summary: row.string("summary") ?? "", detail: detail)
            }
            graph = detailFlatten(graphResult.data)
            graphSnapshot = TransactionGraphSnapshot(
                graphResult.data, pairFallback: pair, transaction: transaction
            )
            selectedGraphRouteLeg = graphSnapshot.swapRoute?.currentLeg
            commercialContext = detailFlatten(commercialResult.data)
            loanMarks = (loansResult.data?.objectValue?.objects("marks") ?? []).compactMap(TransactionLoanMark.init)
            loanRole = currentLoanMark?.role
            journalEvents = (journalResult.data?.objectValue?.objects("events") ?? []).compactMap(TransactionJournalEvent.init)
            journalNeedsProcessing = journalResult.data?.objectValue?["summary"]?.objectValue?.bool("needsJournals") ?? false
            errorMessage = [attachmentsResult, historyResult, graphResult, loansResult, commercialResult, journalResult]
                .compactMap { $0.error?.message }
                .first
            await loadPrivacyContext()
        } catch { errorMessage = String(describing: error) }
    }

    private func loadPrivacyContext() async {
        privacyIsLoading = true
        defer { privacyIsLoading = false }
        let references = [transaction.id, transaction.transactionID, transaction.explorerID]
            .compactMap { $0 }
        do {
            let result = try await daemon.invoke(.uiReportsPrivacyMirror, args: nil)
            privacyContext = .parse(result, references: references)
        } catch {
            privacyContext = .init(
                matchedTransactionID: nil, evidenceLevel: "unknown", tellCount: 0,
                walletPenaltyCount: 0, tellKinds: [], degraded: true,
                message: String(describing: error)
            )
        }
    }

    public func selectGraphRouteLeg(_ key: String) async {
        guard ["out", "in"].contains(key), let route = graphSnapshot.swapRoute else { return }
        selectedGraphRouteLeg = key
        let leg = key == "out" ? route.out : route.incoming
        guard let reference = leg.id ?? leg.transactionID, !reference.isEmpty else { return }
        graphIsLoading = true
        defer { graphIsLoading = false }
        do {
            let result = try await daemon.invoke(.uiTransactionsGraph, args: [
                "transaction": .string(reference), "allowPublicLookup": .bool(false),
            ])
            if let error = result.error { errorMessage = error.message; return }
            graph = detailFlatten(result.data)
            graphSnapshot = TransactionGraphSnapshot(
                result.data, pairFallback: pair, transaction: transaction
            )
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }

    public func markLoan(as role: String) async {
        do {
            let result = try await daemon.invoke(.uiLoansMark, args: ["txid": .string(transaction.id), "as": .string(role)])
            if let error = result.error { errorMessage = error.message; return }
            await load()
        } catch { errorMessage = String(describing: error) }
    }
    public func unmarkLoan() async {
        do {
            let result = try await daemon.invoke(.uiLoansUnmark, args: ["txid": .string(transaction.id)])
            if let error = result.error { errorMessage = error.message; return }
            await load()
        } catch { errorMessage = String(describing: error) }
    }
    public func linkLoan(transactionIDs: [String], loanID: String? = nil) async {
        do {
            var args: [String: JSONValue] = ["txids": .array(transactionIDs.map(JSONValue.string))]
            if let loanID, !loanID.isEmpty { args["loan_id"] = .string(loanID) }
            let result = try await daemon.invoke(.uiLoansLink, args: args)
            if let error = result.error { errorMessage = error.message; return }
            await load()
        } catch { errorMessage = String(describing: error) }
    }

    public func unpair() async {
        guard let pair, canUnpair else { return }
        isUnpairing = true
        defer { isUnpairing = false }
        do {
            let result = try await daemon.invoke(.uiTransfersUnpair, args: ["pair_id": .string(pair.id)])
            if let error = result.error { errorMessage = error.message; return }
            self.pair = nil
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }

    public func save() async {
        isWorking = true
        didSave = false
        defer { isWorking = false }
        do {
            var args: [String: JSONValue] = [
                "transaction": .string(transaction.id),
                "note": note.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? .null : .string(note),
                "tags": .array(tagValues.map(JSONValue.string)),
                "excluded": .bool(excluded),
            ]
            if reviewStatus != baselineReviewStatus || taxable != baselineTaxable
                || atRegime != baselineRegime || atCategory != baselineCategory {
                args["review_status"] = .string(reviewStatus)
                args["taxable"] = .bool(taxable)
                args["at_regime"] = atRegime.isEmpty ? .null : .string(atRegime)
                args["at_category"] = atCategory.isEmpty ? .null : .string(atCategory)
            }
            if currentPricingBaseline != baselinePricing {
                args["pricing_source_kind"] = pricingSourceKind.map(JSONValue.string) ?? .null
                args["pricing_quality"] = .string(pricingQuality)
                args["fiat_currency"] = manualCurrency.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? .null
                    : .string(manualCurrency.trimmingCharacters(in: .whitespacesAndNewlines).uppercased())
                args["fiat_rate"] = parseManualDecimal(manualPrice) == nil ? .null : .string(manualPrice)
                args["fiat_value"] = parseManualDecimal(manualValue) == nil ? .null : .string(manualValue)
                args["pricing_external_ref"] = manualSource.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? .null
                    : .string(manualSource.trimmingCharacters(in: .whitespacesAndNewlines))
            }
            let result = try await daemon.invoke(.uiTransactionsMetadataUpdate, args: args)
            if let error = result.error { errorMessage = error.message; return }
            captureBaseline()
            didSave = true
            errorMessage = nil
            await load()
        } catch { errorMessage = String(describing: error) }
    }

    public func addFile(path: String, label: String? = nil) async {
        await attachmentMutation(.uiAttachmentsAdd, args: ["transaction": .string(transaction.id), "file_path": .string(path), "label": .string(label ?? URL(fileURLWithPath: path).lastPathComponent)])
    }
    public func addURL(_ url: String) async { await attachmentMutation(.uiAttachmentsAdd, args: ["transaction": .string(transaction.id), "url": .string(url), "label": .string(url)]) }
    public func rename(_ attachment: AttachmentRow, label: String) async { await attachmentMutation(.uiAttachmentsRename, args: ["attachment": .string(attachment.id), "label": .string(label)]) }
    public func copyAttachments(from sourceTransaction: String, attachmentIDs: [String]) async {
        await attachmentMutation(.uiAttachmentsCopy, args: [
            "transaction": .string(transaction.id), "source_transaction": .string(sourceTransaction),
            "attachments": .array(attachmentIDs.map(JSONValue.string)),
        ])
    }
    public func remove(_ attachment: AttachmentRow) async { await attachmentMutation(.uiAttachmentsRemove, args: ["attachment": .string(attachment.id)]) }
    public func open(_ attachment: AttachmentRow) async -> URL? {
        do {
            let result = try await daemon.invoke(.uiAttachmentsOpen, args: ["attachment": .string(attachment.id)])
            if let error = result.error { errorMessage = error.message; return nil }
            let object = result.data?.objectValue ?? [:]
            if let url = object.string("url") { return URL(string: url) }
            if let path = object.string("file", "path") { return URL(fileURLWithPath: path) }
        } catch { errorMessage = String(describing: error) }
        return nil
    }
    public func revert(_ event: HistoryRow) async {
        do {
            let result = try await daemon.invoke(.uiTransactionsHistoryRevert, args: ["transaction": .string(transaction.id), "event": .string(event.id), "source": .string("gui")])
            if let error = result.error { errorMessage = error.message; return }
            await load()
        } catch { errorMessage = String(describing: error) }
    }
    private func attachmentMutation(_ kind: DaemonKind, args: [String: JSONValue]) async {
        do { let result = try await daemon.invoke(kind, args: args); if let error = result.error { errorMessage = error.message; return }; await load() }
        catch { errorMessage = String(describing: error) }
    }
    public func createDirectPayout(asset: String, amount: String, counterparty: String) async {
        do {
            let result = try await daemon.invoke(.uiTransfersPayoutsCreate, args: [
                "tx_out": .string(transaction.id), "payout_asset": .string(asset),
                "payout_amount": .string(amount), "counterparty": .string(counterparty),
                "kind": .string("direct-swap-payout"),
            ])
            if let error = result.error { errorMessage = error.message; return }
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }

    private var currentPricingBaseline: PricingBaseline {
        PricingBaseline(
            sourceKind: pricingSourceKind,
            quality: pricingQuality,
            currency: manualCurrency,
            price: manualPrice,
            value: manualValue,
            source: manualSource
        )
    }

    private func captureBaseline() {
        baselineClassification = classification
        baselineNote = note
        baselineTags = tagValues
        baselineReviewStatus = reviewStatus
        baselineExcluded = excluded
        baselineTaxable = taxable
        baselineRegime = atRegime
        baselineCategory = atCategory
        baselinePricing = currentPricingBaseline
    }

    private static func pricingSelection(sourceKind: String?, quality: String) -> String {
        if sourceKind == nil || quality == "missing" { return "missing" }
        return pricingOptions.contains(where: { $0.id == sourceKind }) ? sourceKind! : "generic_import"
    }

    private static func defaultTaxOption(for transaction: TransactionRow) -> TransactionTaxOption {
        if transaction.type.caseInsensitiveCompare("Fee") == .orderedSame {
            return taxOptions.first { $0.id == "outside:none" }!
        }
        let id: String
        switch transaction.flow {
        case .incoming: id = "neu:income_general"
        case .transfer: id = "outside:none"
        case .swap, .layerTransition: id = "neu:neu_swap"
        default: id = "neu:neu_gain"
        }
        return taxOptions.first { $0.id == id }!
    }
}

private func splitTags(_ value: String) -> [String] {
    value.split(separator: ",")
        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { !$0.isEmpty }
}

private func uniqueTags(_ values: [String]) -> [String] {
    var seen = Set<String>()
    return values.filter { seen.insert($0).inserted }
}

private func parseManualDecimal(_ value: String) -> Double? {
    let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines)
        .replacingOccurrences(of: ",", with: ".")
    guard !normalized.isEmpty, let parsed = Double(normalized), parsed.isFinite else { return nil }
    return parsed
}

private func sumJournal(
    _ events: [TransactionJournalEvent],
    values: (TransactionJournalEvent) -> [Double?]
) -> Double {
    events.reduce(0) { total, event in
        for value in values(event) {
            if let value, value.isFinite { return total + value }
        }
        return total
    }
}

private func detailDisplay(_ value: JSONValue?) -> String {
    guard let value else { return "—" }
    switch value { case let .string(v): return v; case let .integer(v): return String(v); case let .unsignedInteger(v): return String(v); case let .number(v): return String(v); case let .bool(v): return String(v); case .null: return "—"; case let .array(v): return v.map(detailDisplay).joined(separator: ", "); case .object: return "…" }
}
private func detailFlatten(_ value: JSONValue?, prefix: String = "") -> [KeyValueRow] {
    guard let value else { return [] }
    if case let .object(object) = value { return object.sorted { $0.key < $1.key }.flatMap { detailFlatten($0.value, prefix: prefix.isEmpty ? $0.key : "\(prefix).\($0.key)") } }
    return [KeyValueRow(key: prefix, value: detailDisplay(value))]
}
