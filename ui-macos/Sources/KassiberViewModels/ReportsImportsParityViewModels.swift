import Foundation
import Observation
import KassiberDaemonKit

private enum ReportsImportsPresentationError: Error {
    case key(String)
}

public enum ReportsImportsPresentationArgument: Equatable, Hashable, Sendable {
    case integer(Int64)
    case text(String)
}

/// A user-facing message that crosses the Foundation-only view-model boundary
/// without freezing English prose into application state.
public enum ReportsImportsPresentationMessage: Equatable, Hashable, Sendable {
    case localized(String, [ReportsImportsPresentationArgument] = [])
    case literal(String)
}

// MARK: - Report package

public enum ReportsImportsExportKind: String, CaseIterable, Identifiable, Sendable {
    case reportPDF
    case reportCSV
    case reportXLSX
    case capitalGainsCSV
    case austrianPDF
    case austrianCSV
    case austrianXLSX
    case summaryPDF
    case auditPackage
    case transactionsCSV
    case transactionsXLSX
    case exitTaxPDF
    case exitTaxXLSX

    public var id: String { rawValue }
    public var localizationKey: String { "reportsParity.export.\(rawValue)" }

    var daemonKind: DaemonKind {
        switch self {
        case .reportPDF: .uiReportsExportPdf
        case .reportCSV: .uiReportsExportCsv
        case .reportXLSX: .uiReportsExportXlsx
        case .capitalGainsCSV: .uiReportsExportCapitalGainsCsv
        case .austrianPDF: .uiReportsExportAustrianE1kvPdf
        case .austrianCSV: .uiReportsExportAustrianE1kvCsv
        case .austrianXLSX: .uiReportsExportAustrianE1kvXlsx
        case .summaryPDF: .uiReportsExportSummaryPdf
        case .auditPackage: .uiReportsExportAuditPackage
        case .transactionsCSV: .uiTransactionsExportCsv
        case .transactionsXLSX: .uiTransactionsExportXlsx
        case .exitTaxPDF: .uiReportsExportExitTaxPdf
        case .exitTaxXLSX: .uiReportsExportExitTaxXlsx
        }
    }
}

public struct ReportsImportsWalletChoice: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
}

public struct ReportsImportsCaseChoice: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let status: String
}

public struct ReportsImportsReportRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let primary: String
    public let secondary: String
    public let detail: String
    public let fields: [ReportsImportsReportField]
}

public struct ReportsImportsReportField: Identifiable, Equatable, Sendable {
    public let key: String
    public let value: String
    public var id: String { key }
}

public struct ReportsImportsCapitalLot: Identifiable, Equatable, Sendable {
    public let id: String
    public let acquired: String
    public let disposed: String
    public let sats: Int64
    public let cost: Double
    public let proceeds: Double
    public let type: String
    public var gain: Double { proceeds - cost }
}

public struct ReportsImportsKennzahlRow: Identifiable, Equatable, Sendable {
    public let code: String
    public let label: String
    public let form: String
    public let formSection: String
    public let amount: Double?
    public let rowCount: Int
    public let source: String
    public let note: String
    public var id: String { code }
}

public struct ReportsImportsNeutralSwapLot: Identifiable, Equatable, Sendable {
    public let id: String
    public let date: String
    public let kind: String
    public let policy: String
    public let outWallet: String
    public let outAsset: String
    public let outSats: Int64
    public let inWallet: String
    public let inAsset: String
    public let inSats: Int64
    public let feeSats: Int64
    public let feeKind: String
    public let cost: Double
    public let proceeds: Double
    public let gain: Double
    public let marketValue: Double?
    public let marketDelta: Double?
}

public enum ReportsImportsReadinessAction: String, Equatable, Sendable {
    case journals
    case quarantine
}

public enum ReportsImportsReadinessTone: String, Equatable, Sendable {
    case ready
    case warning
    case blocked
    case neutral
}

public struct ReportsImportsReportReadiness: Equatable, Sendable {
    public let tone: ReportsImportsReadinessTone
    public let titleKey: String
    public let detailKey: String
    public let count: Int
    public let action: ReportsImportsReadinessAction?
}

@MainActor
@Observable
public final class ReportsImportsReportsViewModel {
    public var selection: ReportKind = .capitalGains
    public var selectedYear = Calendar.current.component(.year, from: Date())
    public var lightningConnection = ""
    public var summaryIncludeSnapshot = true
    public var selectedSummaryWalletIDs: Set<String> = []
    public var auditSourceFundsCaseID = ""
    public var auditIncludeCopiedAttachments = true
    public var auditIncludeURLReferences = true
    public var auditIncludeJournalState = true
    public var auditIncludeReviewState = true
    public var auditIncludeEditHistory = false
    public var xlsxVerify = true
    public var exportWallet = ""
    public var exitDepartureDate = Date()
    public var exitDestination = "eu_eea"

    public private(set) var jurisdiction = ""
    public private(set) var method = ""
    public private(set) var availableYears: [Int] = []
    public private(set) var rows: [ReportsImportsReportRow] = []
    public private(set) var metrics: [(String, String)] = []
    public private(set) var capitalLots: [ReportsImportsCapitalLot] = []
    public private(set) var kennzahlRows: [ReportsImportsKennzahlRow] = []
    public private(set) var neutralSwapLots: [ReportsImportsNeutralSwapLot] = []
    public private(set) var readiness = ReportsImportsReportReadiness(
        tone: .neutral,
        titleKey: "reportsParity.readiness.noRows",
        detailKey: "reportsParity.readiness.noRowsDetail",
        count: 0,
        action: nil
    )
    public private(set) var wallets: [ReportsImportsWalletChoice] = []
    public private(set) var sourceFundsCases: [ReportsImportsCaseChoice] = []
    public private(set) var artifact: ExportArtifact?
    public private(set) var isLoading = false
    public private(set) var isExporting = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient

    public init(daemon: any DaemonClient) {
        self.daemon = daemon
    }

    public func loadContext() async {
        do {
            let walletEnvelope = try await daemon.invoke(.uiWalletsList, args: nil)
            try Self.throwIfError(walletEnvelope)
            let walletRows = walletEnvelope.data?.objectValue?.objects("wallets") ?? []
            wallets = walletRows.compactMap { row in
                guard let id = row.string("id", "label"), !id.isEmpty else { return nil }
                return ReportsImportsWalletChoice(id: id, label: row.string("label") ?? id)
            }
            if selectedSummaryWalletIDs.isEmpty {
                selectedSummaryWalletIDs = Set(wallets.map(\.id))
            } else {
                selectedSummaryWalletIDs.formIntersection(wallets.map(\.id))
            }

            let casesEnvelope = try await daemon.invoke(.uiSourceFundsCasesList, args: nil)
            try Self.throwIfError(casesEnvelope)
            sourceFundsCases = (casesEnvelope.data?.objectValue?.objects("cases") ?? []).compactMap { row in
                guard let id = row.string("id"), !id.isEmpty else { return nil }
                return ReportsImportsCaseChoice(
                    id: id,
                    label: row.string("label", "target_external_id") ?? id,
                    status: row.string("status") ?? ""
                )
            }
        } catch {
            errorMessage = Self.message(error)
        }
    }

    public func load() async {
        isLoading = true
        defer { isLoading = false }
        do {
            let envelope = try await daemon.invoke(selection.daemonKind, args: reportArgs())
            try Self.throwIfError(envelope)
            guard let object = envelope.data?.objectValue else {
                throw ReportsImportsPresentationError.key("reportsParity.error.unexpectedReport")
            }
            parseReport(object)
            errorMessage = nil
        } catch {
            errorMessage = Self.message(error)
        }
    }

    public func export(_ kind: ReportsImportsExportKind) async {
        isExporting = true
        defer { isExporting = false }
        do {
            let envelope = try await daemon.invoke(kind.daemonKind, args: exportArgs(kind))
            try Self.throwIfError(envelope)
            guard let object = envelope.data?.objectValue,
                  let path = object.string("file", "dir"), !path.isEmpty else {
                throw ReportsImportsPresentationError.key("reportsParity.error.missingArtifact")
            }
            let url = URL(fileURLWithPath: path)
            artifact = ExportArtifact(
                sourceURL: url,
                filename: object.string("filename") ?? url.lastPathComponent,
                format: object.string("format") ?? url.pathExtension
            )
            errorMessage = nil
        } catch {
            errorMessage = Self.message(error)
        }
    }

    public func clearArtifact() { artifact = nil }

    private func reportArgs() -> [String: JSONValue]? {
        switch selection {
        case .capitalGains, .taxSummary:
            return ["year": .integer(Int64(selectedYear))]
        case .lightningProfitability:
            let connection = lightningConnection.trimmingCharacters(in: .whitespacesAndNewlines)
            return connection.isEmpty ? nil : ["connection": .string(connection)]
        case .balanceHistory:
            return ["interval": .string("month"), "limit": .integer(120)]
        case .summary, .balanceSheet, .portfolio:
            return nil
        }
    }

    private func exportArgs(_ kind: ReportsImportsExportKind) -> [String: JSONValue]? {
        switch kind {
        case .reportPDF, .reportCSV:
            return exportWallet.isEmpty ? nil : ["wallet": .string(exportWallet)]
        case .reportXLSX:
            var args: [String: JSONValue] = ["verify": .bool(xlsxVerify)]
            if !exportWallet.isEmpty { args["wallet"] = .string(exportWallet) }
            return args
        case .capitalGainsCSV, .austrianPDF, .austrianCSV, .austrianXLSX:
            return ["year": .integer(Int64(selectedYear))]
        case .summaryPDF:
            var args: [String: JSONValue] = [
                "start": .string("\(selectedYear)-01-01T00:00:00Z"),
                "end": .string("\(selectedYear)-12-31T23:59:59Z"),
                "include_snapshot": .bool(summaryIncludeSnapshot),
            ]
            if !selectedSummaryWalletIDs.isEmpty {
                args["wallets"] = .array(selectedSummaryWalletIDs.sorted().map(JSONValue.string))
            }
            return args
        case .auditPackage:
            var args: [String: JSONValue] = [
                "include_copied_attachments": .bool(auditIncludeCopiedAttachments),
                "include_url_references": .bool(auditIncludeURLReferences),
                "include_journal_state": .bool(auditIncludeJournalState),
                "include_review_state": .bool(auditIncludeReviewState),
                "include_edit_history": .bool(auditIncludeEditHistory),
            ]
            if !auditSourceFundsCaseID.isEmpty {
                args["source_funds_case"] = .string(auditSourceFundsCaseID)
            }
            return args
        case .transactionsCSV, .transactionsXLSX:
            return exportWallet.isEmpty ? [:] : ["wallet": .string(exportWallet)]
        case .exitTaxPDF, .exitTaxXLSX:
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.dateFormat = "yyyy-MM-dd"
            return [
                "departure_date": .string(formatter.string(from: exitDepartureDate)),
                "destination": .string(exitDestination),
            ]
        }
    }

    private func parseReport(_ object: [String: JSONValue]) {
        jurisdiction = object.string("jurisdictionCode", "jurisdiction_code") ?? ""
        method = object.string("method") ?? ""
        availableYears = (object["availableYears"]?.arrayValue ?? object["available_years"]?.arrayValue ?? [])
            .compactMap { $0.intValue.map(Int.init) }
            .sorted(by: >)
        if availableYears.isEmpty, let year = object.int("year") { availableYears = [Int(year)] }

        capitalLots = object.objects("lots").enumerated().map { index, row in
            ReportsImportsCapitalLot(
                id: row.string("id") ?? "lot-\(index)",
                acquired: row.string("acquired", "acquired_at") ?? "",
                disposed: row.string("disposed", "disposed_at") ?? "",
                sats: row.int("sats", "quantity_sats", "quantity_sat") ?? 0,
                cost: row.double("costEur", "cost_eur", "cost_basis") ?? 0,
                proceeds: row.double("proceedsEur", "proceeds_eur", "proceeds") ?? 0,
                type: row.string("type", "capital_gains_type") ?? ""
            )
        }
        kennzahlRows = object.objects("kennzahlRows", "kennzahl_rows").enumerated().map { index, row in
            ReportsImportsKennzahlRow(
                code: row.string("code", "kennzahl") ?? "\(index + 1)",
                label: row.string("label") ?? "",
                form: row.string("form") ?? "",
                formSection: row.string("formSection", "form_section") ?? "",
                amount: row.double("amount"),
                rowCount: Int(row.int("rowCount", "row_count") ?? 0),
                source: row.string("source") ?? "",
                note: row.string("note") ?? ""
            )
        }
        neutralSwapLots = object.objects("neutralSwapLots", "neutral_swap_lots").enumerated().map { index, row in
            ReportsImportsNeutralSwapLot(
                id: row.string("pairId", "pair_id", "id") ?? "neutral-swap-\(index)",
                date: row.string("date") ?? "",
                kind: row.string("kind") ?? "",
                policy: row.string("policy") ?? "",
                outWallet: row.string("outWallet", "out_wallet") ?? "",
                outAsset: row.string("outAsset", "out_asset") ?? "",
                outSats: row.int("outSats", "out_sats") ?? 0,
                inWallet: row.string("inWallet", "in_wallet") ?? "",
                inAsset: row.string("inAsset", "in_asset") ?? "",
                inSats: row.int("inSats", "in_sats") ?? 0,
                feeSats: row.int("feeSats", "fee_sats") ?? 0,
                feeKind: row.string("feeKind", "fee_kind") ?? "",
                cost: row.double("costEur", "cost_eur") ?? 0,
                proceeds: row.double("proceedsEur", "proceeds_eur") ?? 0,
                gain: row.double("gainEur", "gain_eur") ?? 0,
                marketValue: row.double("marketValueEur", "market_value_eur"),
                marketDelta: row.double("marketDeltaEur", "market_delta_eur")
            )
        }
        readiness = Self.reportReadiness(
            status: object["status"]?.objectValue ?? [:],
            lots: capitalLots,
            kennzahlRows: kennzahlRows
        )

        let collectionKeys = [
            "lots", "kennzahlRows", "kennzahl_rows", "neutralSwapLots", "neutral_swap_lots",
            "rows", "channels", "forwards", "asset_flow", "wallet_flow", "transfer_pairs",
            "totals_by_asset", "history", "points",
        ]
        var parsed: [ReportsImportsReportRow] = []
        for key in collectionKeys {
            for (index, row) in object.objects(key).enumerated() {
                parsed.append(Self.reportRow(row, id: "\(key)-\(index)"))
            }
        }
        rows = parsed

        let metricObject = object["summary"]?.objectValue
            ?? object["metrics"]?.objectValue
            ?? object["totals"]?.objectValue
            ?? [:]
        metrics = metricObject.sorted(by: { $0.key < $1.key }).compactMap { key, value in
            Self.scalar(value).map { (key.replacingOccurrences(of: "_", with: " "), $0) }
        }
    }

    private static func reportRow(_ row: [String: JSONValue], id: String) -> ReportsImportsReportRow {
        let primary = row.string(
            "label", "asset", "description", "wallet", "account", "code", "date", "year", "bucket",
            "transaction_id", "channel_id", "peer_alias"
        ) ?? "—"
        let secondary = row.string(
            "type", "entry_type", "method", "form", "status", "wallet_label", "acquired_at", "disposed_at"
        ) ?? ""
        let ignored = Set(["id", "label", "asset", "description", "wallet", "account", "code", "date", "year", "bucket", "type", "entry_type", "method", "form", "status"])
        let fields = row.sorted(by: { $0.key < $1.key }).compactMap { key, value -> ReportsImportsReportField? in
            guard !ignored.contains(key), let scalar = scalar(value) else { return nil }
            return ReportsImportsReportField(
                key: key.replacingOccurrences(of: "_", with: " "),
                value: scalar
            )
        }
        return ReportsImportsReportRow(
            id: row.string("id", "transaction_id") ?? id,
            primary: primary,
            secondary: secondary,
            detail: fields.map { "\($0.key): \($0.value)" }.joined(separator: " · "),
            fields: fields
        )
    }

    private static func reportReadiness(
        status: [String: JSONValue],
        lots: [ReportsImportsCapitalLot],
        kennzahlRows: [ReportsImportsKennzahlRow]
    ) -> ReportsImportsReportReadiness {
        if status.bool("needsJournals", "needs_journals") == true {
            return ReportsImportsReportReadiness(
                tone: .warning,
                titleKey: "reportsParity.readiness.processLedger",
                detailKey: "reportsParity.readiness.processLedgerDetail",
                count: 0,
                action: .journals
            )
        }
        let quarantines = Int(status.int("quarantines") ?? 0)
        if quarantines > 0 {
            return ReportsImportsReportReadiness(
                tone: .blocked,
                titleKey: "reportsParity.readiness.reviewQueue",
                detailKey: "reportsParity.readiness.reviewQueueDetail",
                count: quarantines,
                action: .quarantine
            )
        }
        let hasFilingRows = kennzahlRows.contains {
            $0.rowCount > 0 || abs($0.amount ?? 0) > 0.005
        }
        if lots.isEmpty && !hasFilingRows {
            return ReportsImportsReportReadiness(
                tone: .neutral,
                titleKey: "reportsParity.readiness.noRows",
                detailKey: "reportsParity.readiness.noRowsDetail",
                count: 0,
                action: nil
            )
        }
        return ReportsImportsReportReadiness(
            tone: .ready,
            titleKey: "reportsParity.readiness.ready",
            detailKey: "reportsParity.readiness.readyDetail",
            count: 0,
            action: nil
        )
    }

    fileprivate static func scalar(_ value: JSONValue) -> String? {
        switch value {
        case let .string(value): value
        case let .integer(value): String(value)
        case let .unsignedInteger(value): String(value)
        case let .number(value): String(format: "%.8g", value)
        case let .bool(value): value ? "Yes" : "No"
        case .null: nil
        case .object, .array: nil
        }
    }

    fileprivate static func throwIfError(_ envelope: DaemonEnvelope) throws {
        if let error = envelope.error { throw error }
    }

    fileprivate static func message(_ error: Error) -> String {
        if case let ReportsImportsPresentationError.key(key) = error { return key }
        if let daemonError = error as? DaemonErrorPayload {
            return [daemonError.message, daemonError.hint].compactMap { $0 }.joined(separator: " ")
        }
        return String(describing: error)
    }
}

// MARK: - Source of Funds workstation

public struct ReportsImportsEvidence: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let type: String
    public let transaction: String
    public let wallet: String
}

public struct ReportsImportsRecipient: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let kind: String
    public let revealMode: String
    public let notes: String
    public let active: Bool
}

public struct ReportsImportsSource: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let type: String
    public let asset: String
    public let amount: Double?
    public let description: String
    public let attachmentCount: Int
}

public struct ReportsImportsLink: Identifiable, Equatable, Sendable {
    public let id: String
    public let fromSource: String
    public let fromTransaction: String
    public let toTransaction: String
    public let type: String
    public let state: String
    public let confidence: String
    public let method: String
    public let amount: Double?
    public let explanation: String
    public let attachmentCount: Int
}

public struct ReportsImportsSourceFundsCase: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let target: String
    public let status: String
}

public struct ReportsImportsFinding: Identifiable, Equatable, Sendable {
    public let id: String
    public let severity: String
    public let message: String
    public let reference: String
}

public struct ReportsImportsCoverageRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let total: Double
    public let fullyTraced: Double
    public let attested: Double
    public let inReview: Double
    public let untraced: Double
}

@MainActor
@Observable
public final class ReportsImportsSourceFundsViewModel {
    public var targetTransaction = ""
    public var targetAmount = ""
    public var reportPurpose = "existing_transaction"
    public var plannedDestination = ""
    public var plannedNote = ""
    public var revealMode = "standard"
    public var selectedRecipientID = ""
    public var diagramDetail = "summary"
    public var amountPrecision = "btc"
    public var maskRecipient = false
    public var omittedSections: Set<String> = []
    public var revealOverrides: [String: String] = [:]
    public var transactionSearch = ""
    public var transactionFlow = "all"
    public var transactionStatus = "all"
    public var transactionNetwork = "all"
    public var transactionAsset = "all"
    public var transactionWallet = "all"
    public var transactionDays = 0

    public var sourceType = "fiat_purchase"
    public var sourceLabel = ""
    public var sourceAsset = "BTC"
    public var sourceAmount = ""
    public var sourceDescription = ""
    public var selectedSourceAttachmentID = ""

    public var selectedLinkID = ""
    public var linkFromTransaction = ""
    public var linkFromSource = ""
    public var linkToTransaction = ""
    public var linkType = "self_transfer"
    public var linkState = "reviewed"
    public var linkConfidence = "strong"
    public var linkAmount = ""
    public var linkFromAmount = ""
    public var linkExplanation = ""
    public var selectedLinkAttachmentID = ""

    public var recipientLabel = ""
    public var recipientKind = "auditor"
    public var recipientRevealMode = "standard"
    public var recipientNotes = ""

    public private(set) var sources: [ReportsImportsSource] = []
    public private(set) var links: [ReportsImportsLink] = []
    public private(set) var evidence: [ReportsImportsEvidence] = []
    public private(set) var recipients: [ReportsImportsRecipient] = []
    public private(set) var cases: [ReportsImportsSourceFundsCase] = []
    public private(set) var coverage: [ReportsImportsCoverageRow] = []
    public private(set) var transactions: [TransactionRow] = []
    public private(set) var findings: [ReportsImportsFinding] = []
    public private(set) var narrative: [String] = []
    public private(set) var targetLabel = ""
    public private(set) var exportable = false
    public private(set) var blockers = 0
    public private(set) var warnings = 0
    public private(set) var lastMutationSummary: ReportsImportsPresentationMessage?
    public private(set) var savedCaseID: String?
    public private(set) var artifact: ExportArtifact?
    public private(set) var isLoading = false
    public private(set) var isMutating = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient

    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public var transactionAssets: [String] { Array(Set(transactions.map(\.asset))).sorted() }
    public var transactionWallets: [String] { Array(Set(transactions.map(\.wallet).filter { !$0.isEmpty })).sorted() }
    public var transactionNetworks: [String] { Array(Set(transactions.map { Self.transactionNetwork($0) })).sorted() }
    public var filteredTransactions: [TransactionRow] {
        let cutoff = transactionDays > 0
            ? Calendar.current.date(byAdding: .day, value: -transactionDays, to: Date())
            : nil
        return transactions.filter { row in
            let searchable = [row.id, row.transactionID ?? "", row.wallet, row.counterparty, row.note, row.tags.joined(separator: " ")]
                .joined(separator: " ")
            let status = row.reviewStatus.lowercased().contains("review")
                ? "review"
                : row.confirmations <= 0 ? "pending" : "confirmed"
            return (transactionSearch.isEmpty || searchable.localizedCaseInsensitiveContains(transactionSearch))
                && (transactionFlow == "all" || row.flow.rawValue == transactionFlow)
                && (transactionStatus == "all" || status == transactionStatus)
                && (transactionNetwork == "all" || Self.transactionNetwork(row) == transactionNetwork)
                && (transactionAsset == "all" || row.asset == transactionAsset)
                && (transactionWallet == "all" || row.wallet == transactionWallet)
                && (cutoff.map { threshold in row.occurredAt.map { $0 >= threshold } ?? false } ?? true)
        }
    }

    public func selectTarget(_ transaction: TransactionRow) {
        targetTransaction = transaction.id
        if targetAmount.isEmpty { targetAmount = String(Double(abs(transaction.amountSats)) / 100_000_000) }
    }

    public func loadInventory() async {
        isLoading = true
        defer { isLoading = false }
        do {
            let sourceEnvelope = try await daemon.invoke(.uiSourceFundsSourcesList, args: nil)
            let linkEnvelope = try await daemon.invoke(.uiSourceFundsLinksList, args: nil)
            let evidenceEnvelope = try await daemon.invoke(.uiSourceFundsEvidenceList, args: nil)
            let recipientEnvelope = try await daemon.invoke(
                .uiSourceFundsRecipientsList,
                args: ["include_inactive": .bool(true)]
            )
            let casesEnvelope = try await daemon.invoke(.uiSourceFundsCasesList, args: nil)
            let coverageEnvelope = try await daemon.invoke(.uiSourceFundsCoverage, args: nil)
            let transactionsEnvelope = try await daemon.invoke(.uiTransactionsList, args: ["limit": .integer(500)])
            for envelope in [sourceEnvelope, linkEnvelope, evidenceEnvelope, recipientEnvelope, casesEnvelope, coverageEnvelope, transactionsEnvelope] {
                try ReportsImportsReportsViewModel.throwIfError(envelope)
            }
            parseSources(sourceEnvelope.data?.objectValue ?? [:])
            parseLinks(linkEnvelope.data?.objectValue ?? [:])
            parseEvidence(evidenceEnvelope.data?.objectValue ?? [:])
            parseRecipients(recipientEnvelope.data?.objectValue ?? [:])
            parseCases(casesEnvelope.data?.objectValue ?? [:])
            parseCoverage(coverageEnvelope.data?.objectValue ?? [:])
            transactions = (transactionsEnvelope.data?.objectValue?["txs"]?.arrayValue
                ?? transactionsEnvelope.data?.objectValue?["transactions"]?.arrayValue
                ?? []).compactMap(TransactionRow.init)
            if targetTransaction.isEmpty, let first = transactions.first { selectTarget(first) }
            errorMessage = nil
        } catch { errorMessage = ReportsImportsReportsViewModel.message(error) }
    }

    public func preview() async {
        guard !targetTransaction.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            let envelope = try await daemon.invoke(.uiSourceFundsPreview, args: previewArgs())
            try ReportsImportsReportsViewModel.throwIfError(envelope)
            parsePreview(envelope.data?.objectValue ?? [:])
            errorMessage = nil
        } catch { errorMessage = ReportsImportsReportsViewModel.message(error) }
    }

    public func suggest() async {
        guard requireTarget() else { return }
        await mutate(.uiSourceFundsSuggest, args: ["target_transaction": .string(targetTransaction)]) { object in
            self.lastMutationSummary = .localized(
                "sourceFundsParity.status.suggestionsAdded %lld",
                [.integer(object.int("inserted") ?? 0)]
            )
        }
    }

    public func assemble() async {
        guard requireTarget() else { return }
        await mutate(.uiSourceFundsAssemble, args: ["target_transaction": .string(targetTransaction)]) { object in
            let reviewed = object.int("auto_reviewed") ?? 0
            let manual = object.int("awaiting_manual_review") ?? 0
            self.lastMutationSummary = .localized(
                "sourceFundsParity.status.assembled %lld %lld",
                [.integer(reviewed), .integer(manual)]
            )
        }
    }

    public func bulkReview() async {
        guard requireTarget() else { return }
        await mutate(.uiSourceFundsLinksBulkReview, args: ["target_transaction": .string(targetTransaction)]) { object in
            self.lastMutationSummary = .localized(
                "sourceFundsParity.status.bulkReviewed %lld %lld",
                [
                    .integer(object.int("reviewed") ?? 0),
                    .integer(object.int("skipped") ?? 0),
                ]
            )
        }
    }

    public func createSource() async {
        var args: [String: JSONValue] = [
            "source_type": .string(sourceType),
            "label": .string(sourceLabel),
            "asset": .string(sourceAsset),
            "description": .string(sourceDescription),
        ]
        if !sourceAmount.isEmpty { args["amount"] = .string(sourceAmount) }
        if !selectedSourceAttachmentID.isEmpty { args["attachment_id"] = .string(selectedSourceAttachmentID) }
        await mutate(.uiSourceFundsSourcesCreate, args: args) { object in
            self.lastMutationSummary = object.string("label", "id").map(
                ReportsImportsPresentationMessage.literal
            ) ?? .localized("sourceFundsParity.status.sourceCreated")
            self.sourceLabel = ""
            self.sourceAmount = ""
            self.sourceDescription = ""
        }
    }

    public func attachSelectedSource(_ sourceID: String) async {
        guard !selectedSourceAttachmentID.isEmpty else { return }
        await mutate(.uiSourceFundsSourcesAttach, args: [
            "source": .string(sourceID),
            "attachment_id": .string(selectedSourceAttachmentID),
        ]) { _ in
            self.lastMutationSummary = .localized("sourceFundsParity.status.evidenceAttached")
        }
    }

    public func createLink() async {
        var args: [String: JSONValue] = [
            "to_transaction": .string(linkToTransaction.isEmpty ? targetTransaction : linkToTransaction),
            "link_type": .string(linkType),
            "state": .string(linkState),
            "confidence": .string(linkConfidence),
            "method": .string("manual"),
            "allocation_policy": .string("explicit"),
            "explanation": .string(linkExplanation),
        ]
        if !linkAmount.isEmpty { args["allocation_amount"] = .string(linkAmount) }
        if !linkFromAmount.isEmpty { args["from_allocation_amount"] = .string(linkFromAmount) }
        if !linkFromTransaction.isEmpty { args["from_transaction"] = .string(linkFromTransaction) }
        if !linkFromSource.isEmpty { args["from_source"] = .string(linkFromSource) }
        if !selectedLinkAttachmentID.isEmpty { args["attachment_id"] = .string(selectedLinkAttachmentID) }
        await mutate(.uiSourceFundsLinksCreate, args: args) { object in
            self.lastMutationSummary = object.string("id").map(
                ReportsImportsPresentationMessage.literal
            ) ?? .localized("sourceFundsParity.status.linkCreated")
            self.linkAmount = ""
            self.linkFromAmount = ""
            self.linkExplanation = ""
        }
    }

    public func reviewSelectedLink(state: String) async {
        guard let link = links.first(where: { $0.id == selectedLinkID }) else { return }
        var args: [String: JSONValue] = [
            "link": .string(link.id),
            "state": .string(state),
            "link_type": .string(linkType),
            "confidence": .string(linkConfidence),
            "explanation": .string(linkExplanation),
        ]
        if !linkAmount.isEmpty { args["allocation_amount"] = .string(linkAmount) }
        if !linkFromAmount.isEmpty { args["from_allocation_amount"] = .string(linkFromAmount) }
        if state == "reviewed" { args["allocation_policy"] = .string("explicit") }
        await mutate(.uiSourceFundsLinksReview, args: args) { _ in
            self.lastMutationSummary = .localized(
                state == "reviewed"
                    ? "sourceFundsParity.status.linkAccepted"
                    : "sourceFundsParity.status.linkRejected"
            )
        }
    }

    public func attachSelectedLink() async {
        guard !selectedLinkID.isEmpty, !selectedLinkAttachmentID.isEmpty else { return }
        await mutate(.uiSourceFundsLinksAttach, args: [
            "link": .string(selectedLinkID),
            "attachment_id": .string(selectedLinkAttachmentID),
        ]) { _ in
            self.lastMutationSummary = .localized("sourceFundsParity.status.evidenceAttached")
        }
    }

    public func selectLink(_ id: String) {
        selectedLinkID = id
        guard let link = links.first(where: { $0.id == id }) else { return }
        linkFromTransaction = link.fromTransaction
        linkFromSource = link.fromSource
        linkToTransaction = link.toTransaction
        linkType = link.type
        linkState = link.state
        linkConfidence = link.confidence
        linkAmount = link.amount.map { String($0) } ?? ""
        linkExplanation = link.explanation
    }

    public func createRecipient() async {
        await mutate(.uiSourceFundsRecipientsCreate, args: [
            "label": .string(recipientLabel),
            "kind": .string(recipientKind),
            "default_reveal_mode": .string(recipientRevealMode),
            "notes": .string(recipientNotes),
        ]) { object in
            self.lastMutationSummary = object.string("label", "id").map(
                ReportsImportsPresentationMessage.literal
            ) ?? .localized("sourceFundsParity.status.recipientCreated")
            self.recipientLabel = ""
            self.recipientNotes = ""
        }
    }

    public func updateRecipient(_ recipientID: String) async {
        await mutate(.uiSourceFundsRecipientsUpdate, args: [
            "recipient": .string(recipientID),
            "label": .string(recipientLabel),
            "kind": .string(recipientKind),
            "default_reveal_mode": .string(recipientRevealMode),
            "notes": .string(recipientNotes),
        ]) { _ in
            self.lastMutationSummary = .localized("sourceFundsParity.status.recipientUpdated")
        }
    }

    public func deleteRecipient(_ recipientID: String) async {
        await mutate(.uiSourceFundsRecipientsDelete, args: ["recipient": .string(recipientID)]) { _ in
            self.lastMutationSummary = .localized("sourceFundsParity.status.recipientDeleted")
        }
    }

    public func editRecipient(_ recipient: ReportsImportsRecipient) {
        selectedRecipientID = recipient.id
        recipientLabel = recipient.label
        recipientKind = recipient.kind
        recipientRevealMode = recipient.revealMode
        recipientNotes = recipient.notes
    }

    public func saveCase(label: String = "") async {
        var args = previewArgs()
        if !label.isEmpty { args["case_label"] = .string(label) }
        await mutate(.uiSourceFundsCasesSave, args: args, reload: false) { object in
            let caseObject = object["case"]?.objectValue ?? [:]
            self.savedCaseID = caseObject.string("id")
            self.lastMutationSummary = caseObject.string("id").map(
                ReportsImportsPresentationMessage.literal
            ) ?? .localized("sourceFundsParity.status.caseSaved")
            self.parsePreview(object)
        }
    }

    public func export(_ kind: DaemonKind, caseID: String? = nil) async {
        isMutating = true
        defer { isMutating = false }
        do {
            let id = caseID ?? savedCaseID
            let args = id.map { ["case": JSONValue.string($0)] }
            let envelope = try await daemon.invoke(kind, args: args)
            try ReportsImportsReportsViewModel.throwIfError(envelope)
            guard let object = envelope.data?.objectValue,
                  let path = object.string("file", "dir") else {
                throw ReportsImportsPresentationError.key("reportsParity.error.missingArtifact")
            }
            let url = URL(fileURLWithPath: path)
            artifact = ExportArtifact(
                sourceURL: url,
                filename: object.string("filename") ?? url.lastPathComponent,
                format: object.string("format") ?? url.pathExtension
            )
            errorMessage = nil
        } catch { errorMessage = ReportsImportsReportsViewModel.message(error) }
    }

    public func clearArtifact() { artifact = nil }

    public func previewArgs() -> [String: JSONValue] {
        var args: [String: JSONValue] = [
            "target_transaction": .string(targetTransaction.trimmingCharacters(in: .whitespacesAndNewlines)),
            "report_purpose": .string(reportPurpose),
            "reveal_mode": .string(revealMode),
            "report_options": .object([
                "diagram_detail": .string(diagramDetail),
                "amount_precision": .string(amountPrecision),
                "mask_recipient": .bool(maskRecipient),
                "omit_sections": .array(omittedSections.sorted().map(JSONValue.string)),
                "reveal_overrides": .object(revealOverrides.mapValues(JSONValue.string)),
            ]),
        ]
        if !targetAmount.isEmpty { args["target_amount"] = .string(targetAmount) }
        if reportPurpose == "planned_exchange_sale" {
            if !plannedDestination.isEmpty { args["planned_destination"] = .string(plannedDestination) }
            if !plannedNote.isEmpty { args["planned_note"] = .string(plannedNote) }
        }
        if !selectedRecipientID.isEmpty { args["recipient"] = .string(selectedRecipientID) }
        return args
    }

    private func mutate(
        _ kind: DaemonKind,
        args: [String: JSONValue],
        reload: Bool = true,
        apply: @escaping @MainActor ([String: JSONValue]) -> Void
    ) async {
        isMutating = true
        defer { isMutating = false }
        do {
            let envelope = try await daemon.invoke(kind, args: args)
            try ReportsImportsReportsViewModel.throwIfError(envelope)
            apply(envelope.data?.objectValue ?? [:])
            errorMessage = nil
            if reload { await loadInventory() }
            if !targetTransaction.isEmpty { await preview() }
        } catch { errorMessage = ReportsImportsReportsViewModel.message(error) }
    }

    private func requireTarget() -> Bool {
        if targetTransaction.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            errorMessage = "sourceFundsParity.error.chooseTarget"
            return false
        }
        return true
    }

    private static func transactionNetwork(_ row: TransactionRow) -> String {
        let value = row.wallet.lowercased()
        if value.contains("lightning") || value.contains("phoenix") || value.contains(" ln") { return "lightning" }
        if value.contains("liquid") || row.asset.uppercased() == "LBTC" { return "liquid" }
        if ["kraken", "bitpanda", "coinbase", "river", "exchange"].contains(where: value.contains) { return "exchange" }
        return "on_chain"
    }

    private func parseSources(_ object: [String: JSONValue]) {
        sources = object.objects("sources").compactMap { row in
            guard let id = row.string("id") else { return nil }
            return ReportsImportsSource(
                id: id,
                label: row.string("label") ?? id,
                type: row.string("source_type") ?? "",
                asset: row.string("asset") ?? "BTC",
                amount: row.double("amount"),
                description: row.string("description") ?? "",
                attachmentCount: row["attachments"]?.arrayValue?.count ?? 0
            )
        }
    }

    private func parseLinks(_ object: [String: JSONValue]) {
        links = object.objects("links").compactMap { row in
            guard let id = row.string("id") else { return nil }
            return ReportsImportsLink(
                id: id,
                fromSource: row.string("from_source_id") ?? "",
                fromTransaction: row.string("from_transaction_id") ?? "",
                toTransaction: row.string("to_transaction_id") ?? "",
                type: row.string("link_type") ?? "",
                state: row.string("state") ?? "",
                confidence: row.string("confidence") ?? "",
                method: row.string("method") ?? "",
                amount: row.double("allocation_amount"),
                explanation: row.string("explanation") ?? "",
                attachmentCount: row["attachments"]?.arrayValue?.count ?? 0
            )
        }
        if selectedLinkID.isEmpty { selectedLinkID = links.first?.id ?? "" }
    }

    private func parseEvidence(_ object: [String: JSONValue]) {
        evidence = object.objects("attachments").compactMap { row in
            guard let id = row.string("id") else { return nil }
            return ReportsImportsEvidence(
                id: id,
                label: row.string("label", "original_filename") ?? id,
                type: row.string("attachment_type", "media_type") ?? "",
                transaction: row.string("transaction_id", "external_id") ?? "",
                wallet: row.string("wallet") ?? ""
            )
        }
    }

    private func parseRecipients(_ object: [String: JSONValue]) {
        recipients = object.objects("recipients").compactMap { row in
            guard let id = row.string("id") else { return nil }
            return ReportsImportsRecipient(
                id: id,
                label: row.string("label") ?? id,
                kind: row.string("kind") ?? "",
                revealMode: row.string("default_reveal_mode") ?? "standard",
                notes: row.string("notes") ?? "",
                active: row.bool("active") ?? true
            )
        }
    }

    private func parseCases(_ object: [String: JSONValue]) {
        cases = object.objects("cases").compactMap { row in
            guard let id = row.string("id") else { return nil }
            return ReportsImportsSourceFundsCase(
                id: id,
                label: row.string("label") ?? id,
                target: row.string("target_external_id", "target_transaction_id") ?? "",
                status: row.string("status") ?? ""
            )
        }
    }

    private func parseCoverage(_ object: [String: JSONValue]) {
        let rows = object.objects("by_wallet") + object.objects("by_asset")
        coverage = rows.enumerated().map { index, row in
            let buckets = row["buckets"]?.objectValue ?? [:]
            func amount(_ key: String) -> Double {
                buckets[key]?.objectValue?.double("amount") ?? 0
            }
            return ReportsImportsCoverageRow(
                id: row.string("wallet_id", "asset") ?? "coverage-\(index)",
                label: row.string("wallet_label", "asset") ?? "—",
                total: row.double("total_inbound") ?? 0,
                fullyTraced: amount("fully_traced"),
                attested: amount("attested"),
                inReview: amount("in_review"),
                untraced: amount("untraced")
            )
        }
    }

    private func parsePreview(_ object: [String: JSONValue]) {
        targetLabel = object["target"]?.objectValue?.string("label") ?? targetTransaction
        let gates = object["explain_gates"]?.objectValue ?? [:]
        exportable = gates.bool("exportable") ?? false
        let blockerRows = gates.objects("blockers")
        let warningRows = gates.objects("warnings")
        blockers = blockerRows.count
        warnings = warningRows.count
        findings = object.objects("findings").enumerated().map { index, row in
            ReportsImportsFinding(
                id: row.string("code", "ref") ?? "finding-\(index)",
                severity: row.string("severity") ?? "info",
                message: row.string("message") ?? "",
                reference: row.string("ref") ?? ""
            )
        }
        narrative = object["narrative"]?.objectValue?["paragraphs"]?.arrayValue?.compactMap(\.stringValue) ?? []
        savedCaseID = object["case"]?.objectValue?.string("id") ?? savedCaseID
    }
}

// MARK: - Import and interchange workstation

public enum ReportsImportsFileFormat: String, CaseIterable, Identifiable, Sendable {
    case genericLedger = "generic_ledger"
    case genericCSV = "csv"
    case wasabiBundle = "wasabi_bundle"
    case ledgerLive = "ledgerlive_csv"
    case phoenix = "phoenix_csv"
    case bullBitcoinWallet = "bullbitcoin_wallet_csv"
    case btcpay = "btcpay_csv"
    case river = "river_csv"
    case bullBitcoin = "bullbitcoin_csv"
    case pocketBitcoin = "pocketbitcoin_csv"
    case strike = "strike_csv"
    case twentyOneBitcoin = "21bitcoin_csv"
    case coinfinity = "coinfinity_csv"
    case binanceSupplemental = "binance_supplemental_csv"

    public var id: String { rawValue }
    public var localizationKey: String { "importsParity.format.\(rawValue)" }

    fileprivate var acceptedCSVHeaderSets: [Set<String>] {
        switch self {
        case .genericLedger, .wasabiBundle:
            []
        case .genericCSV:
            [
                ["date", "direction", "asset", "amount"],
                ["occurred_at", "direction", "asset", "amount"],
                ["timestamp", "direction", "currency", "amount"],
            ]
        case .btcpay:
            [["transactionid", "timestamp", "currency", "amount"]]
        case .phoenix:
            [["date", "id", "type", "amount_msat"]]
        case .river:
            [["date", "sent amount", "sent currency", "received amount", "received currency"]]
        case .bullBitcoin:
            [["order_status", "payin_amount", "payin_currency", "payout_amount", "payout_currency", "completed_at (utc)", "transaction_id"]]
        case .bullBitcoinWallet:
            [["date", "type", "direction", "amount_sats", "amount_btc", "fee_sats", "status", "txid", "network"]]
        case .coinfinity:
            [["order id", "type", "date", "amount eur", "amount crypto", "crypto", "rate eur", "mining fee crypto", "total fee eur", "transaction"]]
        case .twentyOneBitcoin:
            [["id", "transaction_date", "buy_asset", "buy_amount", "sell_asset", "sell_amount", "fee_asset", "fee_amount", "transaction_type"]]
        case .pocketBitcoin:
            [["type", "date", "reference", "price.currency", "price.amount", "cost.currency", "cost.amount", "fee.currency", "fee.amount", "value.currency", "value.amount"]]
        case .strike:
            [["reference", "date & time (utc)", "transaction type", "amount btc"]]
        case .ledgerLive:
            [["operation date", "currency ticker", "operation type", "operation amount", "operation hash"]]
        case .binanceSupplemental:
            [
                ["timestamp utc", "base asset symbol", "quote asset amount + symbol", "trading fee (in quote asset)", "base asset amount + symbol"],
                ["id", "amount", "asset", "divtime"],
            ]
        }
    }

    fileprivate func acceptsCSVHeader(_ header: Set<String>) -> Bool {
        acceptedCSVHeaderSets.contains { $0.isSubset(of: header) }
    }
}

private func normalizedImportHeader(_ value: Substring) -> String {
    String(value)
        .replacingOccurrences(of: "\u{feff}", with: "")
        .trimmingCharacters(in: CharacterSet(charactersIn: " \t\"'"))
        .replacingOccurrences(of: "\u{00a0}", with: " ")
        .split(whereSeparator: \.isWhitespace)
        .joined(separator: " ")
        .lowercased()
}

public struct ReportsImportsPreviewRow: Identifiable, Equatable, Sendable {
    public let id: Int
    public let date: String
    public let kind: String
    public let amount: String
    public let value: String
}

public struct ReportsImportsFilePreviewSummary: Equatable, Sendable {
    public let filename: ReportsImportsPresentationMessage
    public let byteCount: Int
    public let estimatedRecords: Int
    public let detail: ReportsImportsPresentationMessage
}

public struct ReportsImportsBip329Row: Identifiable, Equatable, Sendable {
    public let id: Int
    public let type: String
    public let reference: String
    public let label: String
    public let status: String
}

@MainActor
@Observable
public final class ReportsImportsImportViewModel {
    public var format: ReportsImportsFileFormat = .genericLedger
    public var sourcePath = ""
    public var wallet = ""
    public var newWalletLabel = ""
    public var importMode = "relevant"
    public var createWalletFirst = false
    public var syncAfterCreate = true

    public var bip329Path = ""
    public var bip329ExportMode = "stored"
    public var bip329ExportWallet = ""

    public var samouraiLabel = "Samourai"
    public var samouraiBackend = ""
    public var samouraiNetwork = "main"
    public var samouraiGapLimit = 20
    public var samouraiSourceSetJSON = ""
    public var samouraiDeposit = ""
    public var samouraiBadbank = ""
    public var samouraiPremix = ""
    public var samouraiPostmix = ""

    public var wasabiHistoryJSON = ""
    public var wasabiCoinsJSON = ""
    public var wasabiWalletInfoJSON = ""
    public var wasabiAdditionalJSON = ""

    public private(set) var previewRows: [ReportsImportsPreviewRow] = []
    public private(set) var previewProblems: [ReportsImportsPresentationMessage] = []
    public private(set) var previewMapped = 0
    public private(set) var previewRead = 0
    public private(set) var previewConfident = false
    public private(set) var filePreviewSummary: ReportsImportsFilePreviewSummary?
    public private(set) var bip329Rows: [ReportsImportsBip329Row] = []
    public private(set) var bip329Counts: [(String, Int)] = []
    public private(set) var resultMetrics: [(String, String)] = []
    public private(set) var artifact: ExportArtifact?
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient
    private var previewedImportKey: String?
    private var previewedBip329Key: String?

    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func previewGenericLedger(url: URL) async {
        format = .genericLedger
        sourcePath = url.path
        await previewCurrentFile()
    }

    public var canRunImport: Bool {
        guard previewConfident, let previewedImportKey,
              let current = try? importPreviewKey() else { return false }
        return previewedImportKey == current
    }

    public var canImportBip329: Bool {
        guard !bip329Path.isEmpty, let previewedBip329Key,
              let current = try? Self.fileFingerprint(path: bip329Path) else { return false }
        return previewedBip329Key == current
    }

    public func previewCurrentFile() async {
        invalidateFilePreview()
        isWorking = true
        defer { isWorking = false }
        do {
            if format == .wasabiBundle {
                let bundle = try buildWasabiBundle()
                let object = bundle.objectValue ?? [:]
                let historyCount = object["gethistory"]?.arrayValue?.count ?? 0
                let encoded = try JSONEncoder().encode(bundle)
                previewRead = historyCount
                previewMapped = historyCount
                previewConfident = true
                filePreviewSummary = ReportsImportsFilePreviewSummary(
                    filename: .localized("importsParity.preview.wasabiBundle"),
                    byteCount: encoded.count,
                    estimatedRecords: historyCount,
                    detail: .localized(
                        "importsParity.preview.validatedSections %lld",
                        [.integer(Int64(object.count))]
                    )
                )
            } else {
                let url = URL(fileURLWithPath: sourcePath)
                let data = try Data(contentsOf: url, options: [.mappedIfSafe])
                guard !data.isEmpty else {
                    throw ReportsImportsPresentationError.key("importsParity.error.emptyFile")
                }
                if format == .genericLedger {
                    let envelope = try await daemon.invoke(.uiWalletsLedgerPreview, args: [
                        "filename": .string(url.lastPathComponent),
                        "source_bytes_base64": .string(data.base64EncodedString()),
                    ])
                    try ReportsImportsReportsViewModel.throwIfError(envelope)
                    let object = envelope.data?.objectValue ?? [:]
                    previewRead = Int(object.int("rows_read") ?? 0)
                    previewMapped = Int(object.int("mapped") ?? 0)
                    previewRows = object.objects("preview").enumerated().map { index, row in
                        ReportsImportsPreviewRow(
                            id: index,
                            date: row.string("occurred_at") ?? "",
                            kind: row.string("kind", "direction") ?? "",
                            amount: [row.string("amount"), row.string("asset")].compactMap { $0 }.joined(separator: " "),
                            value: [row.string("fiat_value"), row.string("fiat_currency")].compactMap { $0 }.joined(separator: " ")
                        )
                    }
                    previewProblems = object.objects("problems").map { row in
                        .localized(
                            "importsParity.preview.problemRow %lld %@",
                            [
                                .integer(row.int("row") ?? 0),
                                .text(row.string("message") ?? ""),
                            ]
                        )
                    }
                    let errorCount = Int(object.int("errors") ?? 0)
                    previewConfident = (object.bool("confident") ?? true) && errorCount == 0
                    filePreviewSummary = ReportsImportsFilePreviewSummary(
                        filename: .literal(url.lastPathComponent), byteCount: data.count,
                        estimatedRecords: previewRead,
                        detail: .localized(
                            "importsParity.preview.mappedErrors %lld %lld",
                            [.integer(Int64(previewMapped)), .integer(Int64(errorCount))]
                        )
                    )
                } else {
                    guard let text = String(data: data, encoding: .utf8) else {
                        throw ReportsImportsPresentationError.key("importsParity.error.invalidUtf8")
                    }
                    let lines = text.split(whereSeparator: { $0.isNewline }).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
                    guard !lines.isEmpty else {
                        throw ReportsImportsPresentationError.key("importsParity.error.noRows")
                    }
                    let delimiter: Character = lines[0].contains(";") ? ";" : lines[0].contains("\t") ? "\t" : ","
                    let header = Set(lines[0]
                        .split(separator: delimiter, omittingEmptySubsequences: false)
                        .map(normalizedImportHeader))
                    let columns = header.count
                    let rows = max(0, lines.count - 1)
                    previewRead = rows
                    previewConfident = rows > 0 && format.acceptsCSVHeader(header)
                    previewMapped = previewConfident ? rows : 0
                    if !previewConfident {
                        previewProblems = [.localized("importsParity.error.unrecognizedHeaders")]
                    }
                    filePreviewSummary = ReportsImportsFilePreviewSummary(
                        filename: .literal(url.lastPathComponent), byteCount: data.count,
                        estimatedRecords: rows,
                        detail: .localized(
                            "importsParity.preview.columnsRows %lld %lld",
                            [.integer(Int64(columns)), .integer(Int64(rows))]
                        )
                    )
                }
            }
            guard previewConfident else {
                throw ReportsImportsPresentationError.key("importsParity.error.unsafePreview")
            }
            previewedImportKey = try importPreviewKey()
            errorMessage = nil
        } catch { errorMessage = ReportsImportsReportsViewModel.message(error) }
    }

    public func runImport() async {
        guard canRunImport else {
            errorMessage = "importsParity.error.previewRequired"
            return
        }
        isWorking = true
        defer { isWorking = false }
        do {
            if createWalletFirst {
                var createArgs: [String: JSONValue] = [
                    "label": .string(newWalletLabel),
                    "kind": .string(format == .wasabiBundle ? "wasabi" : "custom"),
                    "source_format": .string(format.rawValue),
                ]
                if !sourcePath.isEmpty { createArgs["source_file"] = .string(sourcePath) }
                let created = try await daemon.invoke(.uiWalletsCreate, args: createArgs)
                try ReportsImportsReportsViewModel.throwIfError(created)
                wallet = newWalletLabel
                if syncAfterCreate {
                    let synced = try await daemon.invoke(.uiWalletsSync, args: ["wallet": .string(newWalletLabel)])
                    try ReportsImportsReportsViewModel.throwIfError(synced)
                    parseResult(synced.data?.objectValue ?? [:])
                } else {
                    parseResult(created.data?.objectValue ?? [:])
                }
            } else {
                var args: [String: JSONValue] = [
                    "source_format": .string(format.rawValue),
                ]
                if !wallet.isEmpty { args["wallet"] = .string(wallet) }
                if !sourcePath.isEmpty { args["source_file"] = .string(sourcePath) }
                if [.bullBitcoin, .binanceSupplemental].contains(format) {
                    args["mode"] = .string(importMode)
                }
                if format == .wasabiBundle, !wasabiHistoryJSON.isEmpty {
                    args["source_bundle"] = try buildWasabiBundle()
                }
                let envelope = try await daemon.invoke(.uiWalletsImportFile, args: args)
                try ReportsImportsReportsViewModel.throwIfError(envelope)
                parseResult(envelope.data?.objectValue ?? [:])
            }
            previewedImportKey = nil
            errorMessage = nil
        } catch { errorMessage = ReportsImportsReportsViewModel.message(error) }
    }

    public func previewBip329() async {
        previewedBip329Key = nil
        await bip329(.uiMetadataBip329Preview)
        if errorMessage == nil { previewedBip329Key = try? Self.fileFingerprint(path: bip329Path) }
    }

    public func importBip329() async {
        guard canImportBip329 else {
            errorMessage = "importsParity.error.bip329PreviewRequired"
            return
        }
        await bip329(.uiMetadataBip329Import)
        if errorMessage == nil { previewedBip329Key = nil }
    }

    public func exportBip329() async {
        var args: [String: JSONValue] = ["mode": .string(bip329ExportMode)]
        if !bip329ExportWallet.isEmpty { args["wallet"] = .string(bip329ExportWallet) }
        await runExport(.uiMetadataBip329Export, args: args)
    }

    public func importSamourai() async {
        isWorking = true
        defer { isWorking = false }
        do {
            let sourceSet: JSONValue
            if samouraiSourceSetJSON.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                sourceSet = try buildSamouraiSourceSet()
            } else {
                sourceSet = try decodeJSON(samouraiSourceSetJSON)
            }
            var args: [String: JSONValue] = [
                "label": .string(samouraiLabel),
                "network": .string(samouraiNetwork),
                "gap_limit": .integer(Int64(samouraiGapLimit)),
                "source_set": sourceSet,
            ]
            if !samouraiBackend.isEmpty { args["backend"] = .string(samouraiBackend) }
            let envelope = try await daemon.invoke(.uiWalletsImportSamourai, args: args)
            try ReportsImportsReportsViewModel.throwIfError(envelope)
            parseResult(envelope.data?.objectValue ?? [:])
            errorMessage = nil
        } catch { errorMessage = ReportsImportsReportsViewModel.message(error) }
    }

    public func exportLedgerTemplate(format: String) async {
        await runExport(.uiTransactionsLedgerTemplate, args: ["format": .string(format)])
    }

    public func clearArtifact() { artifact = nil }

    private func invalidateFilePreview() {
        previewedImportKey = nil
        previewRows = []
        previewProblems = []
        previewMapped = 0
        previewRead = 0
        previewConfident = false
        filePreviewSummary = nil
    }

    private func importPreviewKey() throws -> String {
        if format == .wasabiBundle {
            var hasher = Hasher()
            hasher.combine(format.rawValue)
            hasher.combine(wasabiHistoryJSON)
            hasher.combine(wasabiCoinsJSON)
            hasher.combine(wasabiWalletInfoJSON)
            hasher.combine(wasabiAdditionalJSON)
            return "wasabi:\(hasher.finalize())"
        }
        guard !sourcePath.isEmpty else {
            throw ReportsImportsPresentationError.key("importsParity.error.chooseFile")
        }
        return "\(format.rawValue):\(try Self.fileFingerprint(path: sourcePath))"
    }

    private static func fileFingerprint(path: String) throws -> String {
        let url = URL(fileURLWithPath: path).standardizedFileURL
        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        let size = (attributes[.size] as? NSNumber)?.int64Value ?? -1
        let modified = (attributes[.modificationDate] as? Date)?.timeIntervalSince1970 ?? -1
        return "\(url.path):\(size):\(modified)"
    }

    private func bip329(_ kind: DaemonKind) async {
        isWorking = true
        defer { isWorking = false }
        do {
            let envelope = try await daemon.invoke(kind, args: ["file": .string(bip329Path)])
            try ReportsImportsReportsViewModel.throwIfError(envelope)
            let object = envelope.data?.objectValue ?? [:]
            let counts = object["counts"]?.objectValue ?? object["preview"]?.objectValue?["counts"]?.objectValue ?? [:]
            let parsedCounts = counts.sorted(by: { $0.key < $1.key }).compactMap { key, value in
                value.intValue.map { (key, Int($0)) }
            }
            if !parsedCounts.isEmpty { bip329Counts = parsedCounts }
            let parsedRows = object.objects("rows").enumerated().map { index, row in
                ReportsImportsBip329Row(
                    id: index,
                    type: row.string("type") ?? "",
                    reference: row.string("ref") ?? "",
                    label: row.string("label") ?? "",
                    status: row.string("match_status") ?? ""
                )
            }
            if !parsedRows.isEmpty { bip329Rows = parsedRows }
            parseResult(object)
            errorMessage = nil
        } catch { errorMessage = ReportsImportsReportsViewModel.message(error) }
    }

    private func runExport(_ kind: DaemonKind, args: [String: JSONValue]) async {
        isWorking = true
        defer { isWorking = false }
        do {
            let envelope = try await daemon.invoke(kind, args: args)
            try ReportsImportsReportsViewModel.throwIfError(envelope)
            guard let object = envelope.data?.objectValue,
                  let path = object.string("file", "dir") else {
                throw ReportsImportsPresentationError.key("reportsParity.error.missingArtifact")
            }
            let url = URL(fileURLWithPath: path)
            artifact = ExportArtifact(
                sourceURL: url,
                filename: object.string("filename") ?? url.lastPathComponent,
                format: object.string("format") ?? url.pathExtension
            )
            parseResult(object)
            errorMessage = nil
        } catch { errorMessage = ReportsImportsReportsViewModel.message(error) }
    }

    private func parseResult(_ object: [String: JSONValue]) {
        resultMetrics = object.sorted(by: { $0.key < $1.key }).compactMap { key, value in
            ReportsImportsReportsViewModel.scalar(value).map { (key.replacingOccurrences(of: "_", with: " "), $0) }
        }
    }

    private func buildWasabiBundle() throws -> JSONValue {
        var object: [String: JSONValue] = [:]
        if !wasabiAdditionalJSON.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            guard case let .object(additional) = try decodeJSON(wasabiAdditionalJSON) else {
                throw ReportsImportsPresentationError.key("importsParity.error.wasabiSections")
            }
            object.merge(additional) { _, new in new }
        }
        object["gethistory"] = try decodeJSON(wasabiHistoryJSON)
        if !wasabiCoinsJSON.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            object["listcoins"] = try decodeJSON(wasabiCoinsJSON)
        }
        if !wasabiWalletInfoJSON.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            object["getwalletinfo"] = try decodeJSON(wasabiWalletInfoJSON)
        }
        return .object(object)
    }

    private func buildSamouraiSourceSet() throws -> JSONValue {
        let materials = [
            ("deposit", samouraiDeposit),
            ("badbank", samouraiBadbank),
            ("premix", samouraiPremix),
            ("postmix", samouraiPostmix),
        ]
        let coinType = ["main", "mainnet"].contains(samouraiNetwork.lowercased()) ? 0 : 1
        let hardened: [String: Int64] = [
            "badbank": 2_147_483_644,
            "premix": 2_147_483_645,
            "postmix": 2_147_483_646,
        ]
        var children: [JSONValue] = []
        var xpubs: [JSONValue] = []
        for (section, raw) in materials {
            let material = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !material.isEmpty else { continue }
            let prefix = String(material.prefix(4)).lowercased()
            let extended = ["xpub", "tpub", "ypub", "upub", "zpub", "vpub"].contains(prefix)
            let scriptType: String
            if section != "deposit" { scriptType = "p2wpkh" }
            else if ["ypub", "upub"].contains(prefix) { scriptType = "p2sh-p2wpkh" }
            else if ["zpub", "vpub"].contains(prefix) { scriptType = "p2wpkh" }
            else if material.lowercased().hasPrefix("pkh(") { scriptType = "p2pkh" }
            else if material.lowercased().hasPrefix("sh(wpkh(") { scriptType = "p2sh-p2wpkh" }
            else if extended {
                throw ReportsImportsPresentationError.key("importsParity.error.depositXpub")
            } else { scriptType = "p2wpkh" }

            let root: String
            if section == "deposit" {
                let purpose = scriptType == "p2pkh" ? 44 : scriptType == "p2sh-p2wpkh" ? 49 : 84
                root = "m/\(purpose)'/\(coinType)'/0'"
            } else {
                root = "m/84'/\(coinType)'/\(hardened[section] ?? 0)'"
            }
            if extended {
                xpubs.append(.object([
                    "section": .string(section), "script_type": .string(scriptType),
                    "root_path": .string(root), "xpub": .string(material),
                ]))
            } else {
                let descriptorLines = material
                    .split(whereSeparator: { $0.isNewline })
                    .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
                    .filter { !$0.isEmpty && !$0.hasPrefix("#") }
                guard let receive = descriptorLines.first else { continue }
                var child: [String: JSONValue] = [
                    "section": .string(section), "script_type": .string(scriptType),
                    "root_path": .string(root), "descriptor": .string(receive),
                ]
                if descriptorLines.count > 1 { child["change_descriptor"] = .string(descriptorLines[1]) }
                children.append(.object(child))
            }
        }
        guard !children.isEmpty || !xpubs.isEmpty else {
            throw ReportsImportsPresentationError.key("importsParity.error.samouraiSource")
        }
        return .object([
            "network": .string(samouraiNetwork),
            "children": .array(children),
            "xpubs": .array(xpubs),
        ])
    }

    private func decodeJSON(_ text: String) throws -> JSONValue {
        guard let data = text.data(using: .utf8), !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw ReportsImportsPresentationError.key("importsParity.error.jsonRequired")
        }
        return try JSONDecoder().decode(JSONValue.self, from: data)
    }
}
