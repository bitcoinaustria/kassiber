import SwiftUI
import AppKit
import KassiberDaemonKit
import KassiberViewModels

private func exportLocalized(_ key: String) -> String { AppLocalization.string(key) }

@MainActor
func saveExportArtifact(_ artifact: ExportArtifact, title: String) throws -> URL? {
    let panel = NSSavePanel()
    panel.title = title
    panel.nameFieldStringValue = artifact.filename
    panel.canCreateDirectories = true
    guard panel.runModal() == .OK, let destination = panel.url else { return nil }
    let manager = FileManager.default
    if manager.fileExists(atPath: destination.path) {
        try manager.removeItem(at: destination)
    }
    try manager.copyItem(at: artifact.sourceURL, to: destination)
    return destination
}

struct ExitTaxScreen: View {
    @State private var model: ExitTaxViewModel
    @State private var exporter: ReportExportViewModel
    @Environment(\.locale) private var locale

    init(daemon: any DaemonClient) {
        _model = State(initialValue: ExitTaxViewModel(daemon: daemon))
        _exporter = State(initialValue: ReportExportViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                DatePicker(exportLocalized("exitTax.departure"), selection: $model.departureDate, displayedComponents: .date)
                Picker(exportLocalized("exitTax.destination"), selection: $model.destination) {
                    Text(exportLocalized("exitTax.euEea")).tag("eu_eea")
                    Text(exportLocalized("exitTax.thirdCountry")).tag("third_country")
                }
                .frame(width: 220)
                Button(exportLocalized("action.refresh")) { Task { await model.load() } }
                Spacer()
                Menu(exportLocalized("action.export")) {
                    Button("PDF") { runExport(.uiReportsExportExitTaxPdf) }
                    Button("XLSX") { runExport(.uiReportsExportExitTaxXlsx) }
                }
                .disabled(exporter.isExporting)
            }
            .padding(12)
            Divider()
            HStack(spacing: 24) {
                metric(exportLocalized("exitTax.marketValue"), model.totalMarketValue)
                metric(exportLocalized("exitTax.unrealized"), model.totalUnrealizedGain)
                Spacer()
            }
            .padding()
            Table(model.rows) {
                TableColumn(exportLocalized("field.asset"), value: \.asset)
                TableColumn(exportLocalized("field.quantity")) { row in
                    Text(KassiberFormatting.sats(row.quantitySats, locale: locale)).monospacedDigit().kassiberSensitive()
                }
                TableColumn(exportLocalized("exitTax.marketValue")) { row in
                    Text(row.marketValue, format: .currency(code: model.currency)).monospacedDigit().kassiberSensitive()
                }
                TableColumn(exportLocalized("exitTax.unrealized")) { row in
                    Text(row.unrealizedGain, format: .currency(code: model.currency)).monospacedDigit().kassiberSensitive()
                }
            }
            if let error = model.errorMessage ?? exporter.errorMessage {
                Text(AppLocalization.error(error)).foregroundStyle(.red).font(.caption).padding()
            }
        }
        .navigationTitle(exportLocalized("nav.exitTax"))
        .task { await model.load() }
    }

    private func metric(_ title: String, _ value: Double) -> some View {
        VStack(alignment: .leading) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            Text(value, format: .currency(code: model.currency)).font(.title2.monospacedDigit()).kassiberSensitive()
        }
    }

    private func runExport(_ kind: DaemonKind) {
        Task {
            await exporter.export(kind, args: model.args)
            if let artifact = exporter.artifact {
                _ = try? saveExportArtifact(artifact, title: exportLocalized("export.saveExitTax"))
                exporter.clearArtifact()
            }
        }
    }
}

struct SourceFundsScreen: View {
    @State private var model: SourceFundsViewModel
    @State private var exporter: ReportExportViewModel
    @State private var tab = 0

    init(daemon: any DaemonClient) {
        _model = State(initialValue: SourceFundsViewModel(daemon: daemon))
        _exporter = State(initialValue: ReportExportViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                TextField(exportLocalized("sourceFunds.target"), text: $model.targetTransaction)
                    .textFieldStyle(.roundedBorder)
                Picker(exportLocalized("sourceFunds.reveal"), selection: $model.revealMode) {
                    Text(exportLocalized("sourceFunds.standard")).tag("standard")
                    Text(exportLocalized("sourceFunds.redacted")).tag("redacted")
                    Text(exportLocalized("sourceFunds.full")).tag("full")
                }
                .frame(width: 180)
                Button(exportLocalized("sourceFunds.preview")) { Task { await model.preview() } }
                    .disabled(model.targetTransaction.isEmpty || model.isLoading)
                Menu(exportLocalized("action.export")) {
                    Button("PDF") { exportCase(.uiSourceFundsExportPdf) }
                    Button(exportLocalized("sourceFunds.bundle")) { exportCase(.uiSourceFundsExportBundle) }
                }
                .disabled(!model.exportable || exporter.isExporting)
            }
            .padding(12)
            Picker("", selection: $tab) {
                Text(exportLocalized("sourceFunds.sources")).tag(0)
                Text(exportLocalized("sourceFunds.links")).tag(1)
                Text(exportLocalized("sourceFunds.findings")).tag(2)
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 12)
            Divider().padding(.top, 10)
            tabContent
            if !model.targetLabel.isEmpty {
                Text(model.targetLabel).font(.caption).foregroundStyle(.secondary).padding(8)
            }
            if let error = model.errorMessage ?? exporter.errorMessage {
                Text(AppLocalization.error(error)).foregroundStyle(.red).font(.caption).padding(8)
            }
        }
        .navigationTitle(exportLocalized("nav.sourceFunds"))
        .task { await model.loadInventory() }
    }

    @ViewBuilder
    private var tabContent: some View {
        switch tab {
        case 0:
            Table(model.sources) {
                TableColumn(exportLocalized("field.name"), value: \.label)
                TableColumn(exportLocalized("field.type"), value: \.type)
                TableColumn(exportLocalized("field.asset"), value: \.asset)
                TableColumn(exportLocalized("field.amount")) { row in
                    if let amount = row.amount {
                        Text(amount, format: .number.precision(.fractionLength(0...8))).monospacedDigit()
                    } else {
                        Text("—")
                    }
                }
            }
        case 1:
            Table(model.links) {
                TableColumn(exportLocalized("sourceFunds.from"), value: \.from)
                TableColumn(exportLocalized("sourceFunds.to"), value: \.to)
                TableColumn(exportLocalized("field.status"), value: \.state)
                TableColumn(exportLocalized("sourceFunds.method"), value: \.method)
            }
        default:
            List(model.findings, id: \.self) { finding in
                Label(finding, systemImage: "exclamationmark.triangle")
            }
        }
    }

    private func exportCase(_ kind: DaemonKind) {
        Task {
            await model.saveCase()
            guard let caseID = model.savedCaseID else { return }
            await exporter.export(kind, args: ["case": .string(caseID)])
            if let artifact = exporter.artifact {
                _ = try? saveExportArtifact(artifact, title: exportLocalized("export.saveSourceFunds"))
                exporter.clearArtifact()
            }
        }
    }
}
