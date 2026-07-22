import Foundation
import Observation
import KassiberDaemonKit

public enum ReportKind: String, CaseIterable, Identifiable, Sendable {
    case capitalGains
    case summary
    case balanceSheet
    case portfolio
    case taxSummary
    case balanceHistory
    case lightningProfitability

    public var id: String { rawValue }
    public var localizationKey: String { "report.\(rawValue)" }

    var daemonKind: DaemonKind {
        switch self {
        case .capitalGains: .uiReportsCapitalGains
        case .summary: .uiReportsSummary
        case .balanceSheet: .uiReportsBalanceSheet
        case .portfolio: .uiReportsPortfolioSummary
        case .taxSummary: .uiReportsTaxSummary
        case .balanceHistory: .uiReportsBalanceHistory
        case .lightningProfitability: .uiReportsLightningProfitability
        }
    }
}

public struct ReportRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let primary: String
    public let secondary: String
    public let amount: Double?
    public let currency: String?
    public let quantitySats: Int64?
}

@MainActor
@Observable
public final class ReportsViewModel {
    public var selection: ReportKind = .capitalGains {
        didSet { if oldValue != selection { Task { await load() } } }
    }
    public var year: Int? {
        didSet { if oldValue != year { Task { await load() } } }
    }
    public private(set) var availableYears: [Int] = []
    public private(set) var titleDetail = ""
    public private(set) var rows: [ReportRow] = []
    public private(set) var rawSummary: [(String, String)] = []
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient

    public init(daemon: any DaemonClient) {
        self.daemon = daemon
    }

    public func load() async {
        isLoading = true
        defer { isLoading = false }
        var args: [String: JSONValue]?
        if let year { args = ["year": .integer(Int64(year))] }
        do {
            let envelope = try await daemon.invoke(selection.daemonKind, args: args)
            if let error = envelope.error {
                errorMessage = error.message
                return
            }
            guard let data = envelope.data?.objectValue else {
                errorMessage = "The report response had an unexpected format."
                return
            }
            parse(data)
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    private func parse(_ data: [String: JSONValue]) {
        availableYears = (data["availableYears"]?.arrayValue ?? data["available_years"]?.arrayValue ?? [])
            .compactMap { $0.intValue.map(Int.init) }
        titleDetail = [
            data.string("jurisdictionCode", "jurisdiction_code"),
            data.string("method"),
            data.int("year").map(String.init),
        ].compactMap { $0 }.joined(separator: " · ")

        let candidateKeys: [String]
        switch selection {
        case .capitalGains: candidateKeys = ["lots", "kennzahlRows", "kennzahl_rows"]
        case .summary: candidateKeys = ["asset_flow", "wallet_flow", "transfer_pairs"]
        case .balanceSheet, .portfolio: candidateKeys = ["rows", "totals_by_asset"]
        case .taxSummary: candidateKeys = ["rows"]
        case .balanceHistory: candidateKeys = ["rows"]
        case .lightningProfitability: candidateKeys = ["rows", "channels", "forwards"]
        }
        var source: [[String: JSONValue]] = []
        for key in candidateKeys {
            let values = data.objects(key)
            if !values.isEmpty { source.append(contentsOf: values) }
        }
        rows = source.enumerated().map { index, row in
            let primary = row.string(
                "label", "asset", "description", "wallet", "account", "code",
                "disposed", "date", "year", "bucket"
            ) ?? "Row \(index + 1)"
            let secondary = row.string(
                "type", "entry_type", "method", "acquired", "wallet_label",
                "form", "status"
            ) ?? ""
            return ReportRow(
                id: row.string("id") ?? "\(selection.rawValue)-\(index)",
                primary: primary,
                secondary: secondary,
                amount: row.double(
                    "amount", "gain", "gain_loss", "gainEur", "fiat_value",
                    "market_value", "proceedsEur", "ending_balance_fiat"
                ),
                currency: row.string("fiat_currency", "currency") ?? data.string("fiatCurrency", "fiat_currency"),
                quantitySats: row.int(
                    "sats", "quantity_sat", "quantity_sats", "amount_sat", "balance_sat"
                )
            )
        }

        let summaryObject = data["summary"]?.objectValue
            ?? data["metrics"]?.objectValue
            ?? [:]
        rawSummary = summaryObject.sorted(by: { $0.key < $1.key }).compactMap { key, value in
            let display: String?
            switch value {
            case let .string(text): display = text
            case let .integer(number): display = String(number)
            case let .unsignedInteger(number): display = String(number)
            case let .number(number): display = String(format: "%.2f", number)
            case let .bool(flag): display = flag ? "Yes" : "No"
            default: display = nil
            }
            return display.map { (key.replacingOccurrences(of: "_", with: " "), $0) }
        }
    }
}
