import Foundation
import Observation
import KassiberDaemonKit

public struct ExportArtifact: Equatable, Sendable {
    public let sourceURL: URL
    public let filename: String
    public let format: String
}

@MainActor
@Observable
public final class ReportExportViewModel {
    public private(set) var isExporting = false
    public private(set) var errorMessage: String?
    public private(set) var artifact: ExportArtifact?

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func export(_ kind: DaemonKind, args: [String: JSONValue]? = nil) async {
        isExporting = true
        defer { isExporting = false }
        do {
            let envelope = try await daemon.invoke(kind, args: args)
            if let error = envelope.error { throw error }
            guard let object = envelope.data?.objectValue,
                  let path = object.string("file", "dir"), !path.isEmpty else {
                throw DaemonClientError.protocolError("Export response did not include a local file.")
            }
            let url = URL(fileURLWithPath: path)
            artifact = ExportArtifact(
                sourceURL: url,
                filename: object.string("filename") ?? url.lastPathComponent,
                format: object.string("format") ?? url.pathExtension
            )
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    public func clearArtifact() { artifact = nil }
}

public struct ExitTaxRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let asset: String
    public let quantitySats: Int64
    public let marketValue: Double
    public let unrealizedGain: Double
}

@MainActor
@Observable
public final class ExitTaxViewModel {
    public var departureDate = Date()
    public var destination = "eu_eea"
    public private(set) var rows: [ExitTaxRow] = []
    public private(set) var currency = "EUR"
    public private(set) var totalMarketValue = 0.0
    public private(set) var totalUnrealizedGain = 0.0
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public var args: [String: JSONValue] {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyy-MM-dd"
        return [
            "departure_date": .string(formatter.string(from: departureDate)),
            "destination": .string(destination),
        ]
    }

    public func load() async {
        isLoading = true
        defer { isLoading = false }
        do {
            let envelope = try await daemon.invoke(.uiReportsExitTaxPreview, args: args)
            if let error = envelope.error { throw error }
            let object = envelope.data?.objectValue ?? [:]
            currency = object.string("fiatCurrency", "fiat_currency") ?? "EUR"
            let source = object.objects("rows", "positions", "holdings")
            rows = source.enumerated().map { index, row in
                ExitTaxRow(
                    id: row.string("id", "asset") ?? "row-\(index)",
                    asset: row.string("asset") ?? "BTC",
                    quantitySats: row.int("quantity_sat", "quantity_sats", "amount_sat") ?? 0,
                    marketValue: row.double("market_value", "marketValue") ?? 0,
                    unrealizedGain: row.double("unrealized_gain", "unrealizedGain", "gain") ?? 0
                )
            }
            let totals = object["totals"]?.objectValue ?? object["summary"]?.objectValue ?? [:]
            totalMarketValue = totals.double("market_value", "marketValue") ?? rows.reduce(0) { $0 + $1.marketValue }
            totalUnrealizedGain = totals.double("unrealized_gain", "unrealizedGain", "gain") ?? rows.reduce(0) { $0 + $1.unrealizedGain }
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }
}

public struct SourceFundsSourceRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let type: String
    public let asset: String
    public let amount: Double?
}

public struct SourceFundsLinkRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let from: String
    public let to: String
    public let state: String
    public let method: String
}

@MainActor
@Observable
public final class SourceFundsViewModel {
    public var targetTransaction = ""
    public var revealMode = "standard"
    public private(set) var sources: [SourceFundsSourceRow] = []
    public private(set) var links: [SourceFundsLinkRow] = []
    public private(set) var targetLabel = ""
    public private(set) var findings: [String] = []
    public private(set) var exportable = false
    public private(set) var savedCaseID: String?
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public var previewArgs: [String: JSONValue] {
        [
            "target_transaction": .string(targetTransaction.trimmingCharacters(in: .whitespacesAndNewlines)),
            "report_purpose": .string("existing_transaction"),
            "reveal_mode": .string(revealMode),
        ]
    }

    public func loadInventory() async {
        isLoading = true
        defer { isLoading = false }
        do {
            async let sourcesEnvelope = daemon.invoke(.uiSourceFundsSourcesList, args: nil)
            async let linksEnvelope = daemon.invoke(.uiSourceFundsLinksList, args: nil)
            let (sourceResult, linkResult) = try await (sourcesEnvelope, linksEnvelope)
            if let error = sourceResult.error ?? linkResult.error { throw error }
            sources = (sourceResult.data?.objectValue?.objects("sources") ?? []).compactMap { row in
                guard let id = row.string("id") else { return nil }
                return SourceFundsSourceRow(
                    id: id, label: row.string("label") ?? id,
                    type: row.string("source_type") ?? "", asset: row.string("asset") ?? "BTC",
                    amount: row.double("amount")
                )
            }
            links = (linkResult.data?.objectValue?.objects("links") ?? []).compactMap { row in
                guard let id = row.string("id") else { return nil }
                return SourceFundsLinkRow(
                    id: id, from: row.string("from_source_id", "from_transaction_id") ?? "",
                    to: row.string("to_transaction_id") ?? "", state: row.string("state") ?? "",
                    method: row.string("method") ?? ""
                )
            }
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }

    public func preview() async {
        guard !targetTransaction.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        isLoading = true
        defer { isLoading = false }
        do {
            let envelope = try await daemon.invoke(.uiSourceFundsPreview, args: previewArgs)
            if let error = envelope.error { throw error }
            let object = envelope.data?.objectValue ?? [:]
            targetLabel = object["target"]?.objectValue?.string("label") ?? targetTransaction
            let gates = object["explain_gates"]?.objectValue ?? [:]
            exportable = gates.bool("exportable") ?? false
            findings = object.objects("findings").compactMap { $0.string("message") }
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }

    public func saveCase() async {
        do {
            let envelope = try await daemon.invoke(.uiSourceFundsCasesSave, args: previewArgs)
            if let error = envelope.error { throw error }
            savedCaseID = envelope.data?.objectValue?["case"]?.objectValue?.string("id")
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }
}
