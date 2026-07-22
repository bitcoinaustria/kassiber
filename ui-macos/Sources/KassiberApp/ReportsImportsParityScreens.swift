import SwiftUI
import AppKit
import UniformTypeIdentifiers
import KassiberDaemonKit
import KassiberViewModels

private func reportsImportsLocalized(_ key: String) -> String { AppLocalization.string(key) }
private func reportsImportsDisplayMessage(_ value: String) -> String {
    reportsImportsLocalized(value)
}

private func reportsImportsDisplayMessage(_ message: ReportsImportsPresentationMessage) -> String {
    switch message {
    case let .literal(value):
        return value
    case let .localized(key, arguments):
        let format = reportsImportsLocalized(key)
        guard !arguments.isEmpty else { return format }
        let values: [CVarArg] = arguments.map { argument in
            switch argument {
            case let .integer(value): value
            case let .text(value): value
            }
        }
        return String(
            format: format,
            locale: AppLocalization.locale,
            arguments: values
        )
    }
}

@MainActor
private func reportsImportsPickFile(types: [UTType]) -> URL? {
    let panel = NSOpenPanel()
    panel.canChooseFiles = true
    panel.canChooseDirectories = false
    panel.allowsMultipleSelection = false
    panel.allowedContentTypes = types
    return panel.runModal() == .OK ? panel.url : nil
}

@MainActor
@discardableResult
private func reportsImportsSave(_ artifact: ExportArtifact, titleKey: String) -> URL? {
    do { return try saveExportArtifact(artifact, title: reportsImportsLocalized(titleKey)) }
    catch { return nil }
}

// MARK: - Complete report package

struct ReportsImportsReportsScreen: View {
    @State private var model: ReportsImportsReportsViewModel
    @State private var lastSavedURL: URL?
    @Environment(\.kassiberNavigate) private var navigate

    init(daemon: any DaemonClient) {
        _model = State(initialValue: ReportsImportsReportsViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            reportToolbar
            Divider()
            HSplitView {
                ScrollView {
                    VStack(alignment: .leading, spacing: 12) {
                        reportIdentity
                        if !model.metrics.isEmpty { metricGrid }
                        reportOptions
                    }
                    .padding(16)
                }
                .frame(minWidth: 290, idealWidth: 340, maxWidth: 420)

                Group {
                    if model.isLoading && model.rows.isEmpty {
                        ProgressView(reportsImportsLocalized("state.loading"))
                    } else if model.selection == .capitalGains {
                        capitalGainsPackage
                    } else if model.rows.isEmpty {
                        ContentUnavailableView(
                            reportsImportsLocalized("reportsParity.noRows"),
                            systemImage: "doc.text.magnifyingglass",
                            description: Text(
                                model.errorMessage.map(reportsImportsDisplayMessage)
                                    ?? reportsImportsLocalized("reportsParity.noRowsHint")
                            )
                        )
                    } else {
                        Table(model.rows) {
                            TableColumn(reportsImportsLocalized("field.name"), value: \.primary)
                                .width(min: 150, ideal: 220)
                            TableColumn(reportsImportsLocalized("field.type"), value: \.secondary)
                                .width(min: 100, ideal: 150)
                            TableColumn(reportsImportsLocalized("reportsParity.detail"), value: \.detail)
                        }
                    }
                }
                .frame(minWidth: 440)
            }
            if let message = model.errorMessage {
                Text(reportsImportsDisplayMessage(message))
                    .foregroundStyle(.red).font(.caption).padding(8)
            }
        }
        .navigationTitle(reportsImportsLocalized("nav.reports"))
        .task {
            await model.loadContext()
            await model.load()
        }
        .onChange(of: model.selection) { _, _ in Task { await model.load() } }
        .onChange(of: model.selectedYear) { _, _ in Task { await model.load() } }
    }

    private var reportToolbar: some View {
        HStack(spacing: 10) {
            Picker(reportsImportsLocalized("reportsParity.report"), selection: $model.selection) {
                ForEach(ReportKind.allCases) { kind in
                    Text(reportsImportsLocalized(kind.localizationKey)).tag(kind)
                }
            }
            .frame(width: 220)
            if model.selection == .capitalGains || model.selection == .taxSummary {
                Picker(reportsImportsLocalized("reportsParity.year"), selection: $model.selectedYear) {
                    ForEach(model.availableYears.isEmpty ? [model.selectedYear] : model.availableYears, id: \.self) {
                        Text(String($0)).tag($0)
                    }
                }
                .frame(width: 110)
            }
            if model.selection == .lightningProfitability {
                TextField(reportsImportsLocalized("reportsParity.lightningConnection"), text: $model.lightningConnection)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 220)
            }
            Button { Task { await model.load() } } label: {
                Label(reportsImportsLocalized("action.refresh"), systemImage: "arrow.clockwise")
            }
            Spacer()
            Menu(reportsImportsLocalized("reportsParity.reviewData")) {
                Button(reportsImportsLocalized("nav.journals")) { navigate(.journals) }
                Button(reportsImportsLocalized("nav.quarantine")) { navigate(.quarantine) }
                Button(reportsImportsLocalized("nav.transactions")) { navigate(.transactions) }
            }
            if let lastSavedURL {
                Button { NSWorkspace.shared.open(lastSavedURL) } label: {
                    Label(reportsImportsLocalized("reportsParity.openExport"), systemImage: "arrow.up.forward.app")
                }
            }
            Menu {
                ForEach(ReportsImportsExportKind.allCases) { kind in
                    Button(reportsImportsLocalized(kind.localizationKey)) { performExport(kind) }
                }
            } label: {
                Label(reportsImportsLocalized("action.export"), systemImage: "square.and.arrow.up")
            }
            .disabled(model.isExporting)
        }
        .padding(12)
    }

    private var reportIdentity: some View {
        GroupBox(reportsImportsLocalized("reportsParity.package")) {
            VStack(alignment: .leading, spacing: 7) {
                LabeledContent(reportsImportsLocalized("reportsParity.jurisdiction"), value: model.jurisdiction.isEmpty ? "—" : model.jurisdiction)
                LabeledContent(reportsImportsLocalized("reportsParity.method"), value: model.method.isEmpty ? "—" : model.method)
                LabeledContent(reportsImportsLocalized("reportsParity.rowCount"), value: String(model.rows.count))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var capitalGainsPackage: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                readinessPanel
                capitalMetricGrid
                if !model.kennzahlRows.isEmpty { kennzahlTable }
                lotAuditTable
                if !model.neutralSwapLots.isEmpty { neutralSwapTable }
            }
            .padding(16)
        }
    }

    private var readinessPanel: some View {
        GroupBox {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: readinessSymbol)
                    .foregroundStyle(readinessColor)
                VStack(alignment: .leading, spacing: 4) {
                    Text(reportsImportsLocalized(model.readiness.titleKey))
                        .font(.headline)
                    Text(readinessDetail)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if let action = model.readiness.action {
                    Button(reportsImportsLocalized(
                        action == .journals
                            ? "reportsParity.readiness.openLedger"
                            : "reportsParity.readiness.openQueue"
                    )) {
                        navigate(action == .journals ? .journals : .quarantine)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var readinessDetail: String {
        let base = reportsImportsLocalized(model.readiness.detailKey)
        guard model.readiness.count > 0 else { return base }
        return base.replacingOccurrences(of: "{count}", with: String(model.readiness.count))
    }

    private var readinessSymbol: String {
        switch model.readiness.tone {
        case .ready: "checkmark.circle"
        case .warning: "arrow.clockwise.circle"
        case .blocked: "exclamationmark.triangle"
        case .neutral: "info.circle"
        }
    }

    private var readinessColor: Color {
        switch model.readiness.tone {
        case .ready: .green
        case .warning: .orange
        case .blocked: .red
        case .neutral: .secondary
        }
    }

    private var capitalMetricGrid: some View {
        let cost = model.capitalLots.reduce(0) { $0 + $1.cost }
        let proceeds = model.capitalLots.reduce(0) { $0 + $1.proceeds }
        let gain = proceeds - cost
        return GroupBox(reportsImportsLocalized("reportsParity.metrics")) {
            Grid(alignment: .leading, horizontalSpacing: 24, verticalSpacing: 8) {
                GridRow {
                    metricLabel("reportsParity.lotCount", String(model.capitalLots.count))
                    metricLabel("reportsParity.totalSats", model.capitalLots.reduce(Int64(0)) { $0 + $1.sats }.formatted())
                }
                GridRow {
                    metricLabel("reportsParity.totalCost", money(cost))
                    metricLabel("reportsParity.totalProceeds", money(proceeds))
                }
                GridRow {
                    metricLabel("reportsParity.totalGain", money(gain))
                    metricLabel("reportsParity.neutralSwapCount", String(model.neutralSwapLots.count))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func metricLabel(_ key: String, _ value: String) -> some View {
        LabeledContent(reportsImportsLocalized(key)) {
            Text(value).monospacedDigit().kassiberSensitive()
        }
    }

    private var kennzahlTable: some View {
        GroupBox(reportsImportsLocalized("reportsParity.filingFields")) {
            Table(model.kennzahlRows) {
                TableColumn(reportsImportsLocalized("reportsParity.code")) { row in
                    Text(row.code).monospaced()
                }
                TableColumn(reportsImportsLocalized("field.name")) { row in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(row.label)
                        if !row.note.isEmpty { Text(row.note).font(.caption).foregroundStyle(.secondary) }
                    }
                }
                TableColumn(reportsImportsLocalized("reportsParity.form")) { row in
                    Text([row.form, row.formSection].filter { !$0.isEmpty }.joined(separator: " · "))
                }
                TableColumn(reportsImportsLocalized("reportsParity.amount")) { row in
                    Text(row.amount.map(money) ?? "—").monospacedDigit().kassiberSensitive()
                }
                TableColumn(reportsImportsLocalized("reportsParity.rowCount")) { row in
                    Text(String(row.rowCount)).monospacedDigit()
                }
            }
            .frame(minHeight: 150, idealHeight: 190)
        }
    }

    private var lotAuditTable: some View {
        GroupBox(reportsImportsLocalized("reportsParity.lotAudit")) {
            if model.capitalLots.isEmpty {
                ContentUnavailableView(
                    reportsImportsLocalized("reportsParity.noRows"),
                    systemImage: "tray"
                )
                .frame(minHeight: 130)
            } else {
                Table(model.capitalLots) {
                    TableColumn(reportsImportsLocalized("reportsParity.acquired")) { Text($0.acquired.isEmpty ? "—" : $0.acquired) }
                    TableColumn(reportsImportsLocalized("reportsParity.disposed")) { Text($0.disposed) }
                    TableColumn(reportsImportsLocalized("reportsParity.sats")) { Text($0.sats.formatted()).monospacedDigit().kassiberSensitive() }
                    TableColumn(reportsImportsLocalized("reportsParity.term")) { Text(AppLocalization.code($0.type)) }
                    TableColumn(reportsImportsLocalized("reportsParity.cost")) { Text(money($0.cost)).monospacedDigit().kassiberSensitive() }
                    TableColumn(reportsImportsLocalized("reportsParity.proceeds")) { Text(money($0.proceeds)).monospacedDigit().kassiberSensitive() }
                    TableColumn(reportsImportsLocalized("reportsParity.gain")) { Text(money($0.gain)).monospacedDigit().kassiberSensitive() }
                }
                .frame(minHeight: 220, idealHeight: 300)
            }
        }
    }

    private var neutralSwapTable: some View {
        GroupBox(reportsImportsLocalized("reportsParity.neutralSwapAudit")) {
            Table(model.neutralSwapLots) {
                TableColumn(reportsImportsLocalized("field.date"), value: \.date)
                TableColumn(reportsImportsLocalized("reportsParity.outLeg")) { row in
                    Text("\(row.outWallet) · \(row.outSats.formatted()) \(row.outAsset)")
                        .kassiberSensitive()
                }
                TableColumn(reportsImportsLocalized("reportsParity.inLeg")) { row in
                    Text("\(row.inWallet) · \(row.inSats.formatted()) \(row.inAsset)")
                        .kassiberSensitive()
                }
                TableColumn(reportsImportsLocalized("reportsParity.policy")) { row in
                    Text(AppLocalization.code(row.policy))
                }
                TableColumn(reportsImportsLocalized("reportsParity.fee")) { row in
                    Text("\(row.feeSats.formatted()) \(reportsImportsLocalized("reportsParity.satsUnit"))")
                        .monospacedDigit()
                        .kassiberSensitive()
                }
                TableColumn(reportsImportsLocalized("reportsParity.marketDelta")) { row in
                    Text(row.marketDelta.map(money) ?? "—").monospacedDigit().kassiberSensitive()
                }
            }
            .frame(minHeight: 180, idealHeight: 240)
        }
    }

    private func money(_ value: Double) -> String {
        value.formatted(.currency(code: "EUR"))
    }

    private var metricGrid: some View {
        GroupBox(reportsImportsLocalized("reportsParity.metrics")) {
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 6) {
                ForEach(Array(model.metrics.enumerated()), id: \.offset) { _, metric in
                    GridRow {
                        Text(AppLocalization.code(metric.0)).foregroundStyle(.secondary)
                        Text(metric.1).monospacedDigit().frame(maxWidth: .infinity, alignment: .trailing).kassiberSensitive()
                    }
                }
            }
        }
    }

    private var reportOptions: some View {
        GroupBox(reportsImportsLocalized("reportsParity.exportOptions")) {
            VStack(alignment: .leading, spacing: 9) {
                Picker(reportsImportsLocalized("reportsParity.walletScope"), selection: $model.exportWallet) {
                    Text(reportsImportsLocalized("reportsParity.allWallets")).tag("")
                    ForEach(model.wallets) { Text($0.label).tag($0.id) }
                }
                Toggle(reportsImportsLocalized("reportsParity.verifyWorkbook"), isOn: $model.xlsxVerify)
                Divider()
                Text(reportsImportsLocalized("reportsParity.summaryPdf")).font(.headline)
                Toggle(reportsImportsLocalized("reportsParity.includeSnapshot"), isOn: $model.summaryIncludeSnapshot)
                ForEach(model.wallets) { wallet in
                    Toggle(wallet.label, isOn: Binding(
                        get: { model.selectedSummaryWalletIDs.contains(wallet.id) },
                        set: { enabled in
                            if enabled { model.selectedSummaryWalletIDs.insert(wallet.id) }
                            else if model.selectedSummaryWalletIDs.count > 1 { model.selectedSummaryWalletIDs.remove(wallet.id) }
                        }
                    ))
                }
                Divider()
                Text(reportsImportsLocalized("reportsParity.auditPackage")).font(.headline)
                Picker(reportsImportsLocalized("reportsParity.sourceFundsCase"), selection: $model.auditSourceFundsCaseID) {
                    Text(reportsImportsLocalized("reportsParity.activeProfile")).tag("")
                    ForEach(model.sourceFundsCases) { Text($0.label).tag($0.id) }
                }
                Toggle(reportsImportsLocalized("reportsParity.copiedAttachments"), isOn: $model.auditIncludeCopiedAttachments)
                Toggle(reportsImportsLocalized("reportsParity.urlReferences"), isOn: $model.auditIncludeURLReferences)
                Toggle(reportsImportsLocalized("reportsParity.journalState"), isOn: $model.auditIncludeJournalState)
                Toggle(reportsImportsLocalized("reportsParity.reviewState"), isOn: $model.auditIncludeReviewState)
                Toggle(reportsImportsLocalized("reportsParity.editHistory"), isOn: $model.auditIncludeEditHistory)
                Divider()
                Text(reportsImportsLocalized("reportsParity.exitTaxOptions")).font(.headline)
                DatePicker(reportsImportsLocalized("exitTax.departure"), selection: $model.exitDepartureDate, displayedComponents: .date)
                Picker(reportsImportsLocalized("exitTax.destination"), selection: $model.exitDestination) {
                    Text(reportsImportsLocalized("exitTax.euEea")).tag("eu_eea")
                    Text(reportsImportsLocalized("exitTax.thirdCountry")).tag("third_country")
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func performExport(_ kind: ReportsImportsExportKind) {
        Task {
            await model.export(kind)
            guard let artifact = model.artifact else { return }
            lastSavedURL = reportsImportsSave(artifact, titleKey: "reportsParity.saveExport")
            model.clearArtifact()
        }
    }
}

// MARK: - Complete Source of Funds workstation

private enum ReportsImportsSourceFundsTab: String, CaseIterable, Identifiable {
    case report, sources, links, evidence, coverage, recipients, cases
    var id: String { rawValue }
}

private struct ReportsImportsDisclosureSectionRow: View {
    @Bindable var model: ReportsImportsSourceFundsViewModel
    let section: String

    var body: some View {
        HStack {
            Toggle(AppLocalization.code(section), isOn: includeBinding)
            Picker(reportsImportsLocalized("sourceFundsParity.override"), selection: overrideBinding) {
                Text(reportsImportsLocalized("sourceFundsParity.auto")).tag("auto")
                Text(reportsImportsLocalized("sourceFundsParity.show")).tag("show")
                Text(reportsImportsLocalized("sourceFundsParity.hide")).tag("hide")
            }
            .labelsHidden()
            .frame(width: 90)
        }
    }

    private var includeBinding: Binding<Bool> {
        Binding(
            get: { !model.omittedSections.contains(section) },
            set: { include in
                if include { model.omittedSections.remove(section) }
                else { model.omittedSections.insert(section) }
            }
        )
    }

    private var overrideBinding: Binding<String> {
        Binding(
            get: { model.revealOverrides[section] ?? "auto" },
            set: { value in
                if value == "auto" { model.revealOverrides.removeValue(forKey: section) }
                else { model.revealOverrides[section] = value }
            }
        )
    }
}

private struct ReportsImportsTargetPicker: View {
    @Bindable var model: ReportsImportsSourceFundsViewModel
    let openDetails: (TransactionRow) -> Void
    @Environment(\.locale) private var locale

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                Text(reportsImportsLocalized("sourceFundsParity.chooseTarget")).font(.headline)
                Text("\(model.filteredTransactions.count) / \(model.transactions.count)")
                    .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                Spacer()
                TextField(reportsImportsLocalized("sourceFundsParity.searchTransactions"), text: $model.transactionSearch)
                    .textFieldStyle(.roundedBorder).frame(maxWidth: 260)
            }
            HStack {
                filterPicker(reportsImportsLocalized("filter.flow"), selection: $model.transactionFlow, values: ["all", "incoming", "outgoing", "transfer"])
                filterPicker(reportsImportsLocalized("field.status"), selection: $model.transactionStatus, values: ["all", "confirmed", "pending", "review"])
                filterPicker(reportsImportsLocalized("field.network"), selection: $model.transactionNetwork, values: ["all"] + model.transactionNetworks)
                filterPicker(reportsImportsLocalized("field.asset"), selection: $model.transactionAsset, values: ["all"] + model.transactionAssets)
                filterPicker(reportsImportsLocalized("field.wallet"), selection: $model.transactionWallet, values: ["all"] + model.transactionWallets)
                Picker(reportsImportsLocalized("sourceFundsParity.dateRange"), selection: $model.transactionDays) {
                    Text(reportsImportsLocalized("sourceFundsParity.allDates")).tag(0)
                    Text(reportsImportsLocalized("activity.last7")).tag(7)
                    Text(reportsImportsLocalized("activity.last30")).tag(30)
                    Text(reportsImportsLocalized("activity.last365")).tag(365)
                }.frame(maxWidth: 140)
            }
            Table(model.filteredTransactions) {
                TableColumn(reportsImportsLocalized("field.date"), value: \.dateLabel).width(min: 100, ideal: 130)
                TableColumn(reportsImportsLocalized("field.wallet"), value: \.wallet).width(min: 100, ideal: 140)
                TableColumn(reportsImportsLocalized("field.type")) { row in Text(AppLocalization.code(row.type)) }.width(min: 80, ideal: 100)
                TableColumn(reportsImportsLocalized("field.amount")) { row in
                    KassiberAmountText(transaction: row).monospacedDigit().kassiberSensitive()
                }.width(min: 110, ideal: 140)
                TableColumn(reportsImportsLocalized("field.status")) { row in Text(AppLocalization.code(row.reviewStatus)) }.width(min: 80, ideal: 100)
                TableColumn("") { row in
                    HStack {
                        Button(reportsImportsLocalized("sourceFundsParity.useTarget")) { model.selectTarget(row) }
                            .buttonStyle(.link)
                        Button(reportsImportsLocalized("sourceFundsParity.details")) { openDetails(row) }
                            .buttonStyle(.link)
                    }
                }.width(120)
            }
            .overlay {
                if model.filteredTransactions.isEmpty {
                    ContentUnavailableView(reportsImportsLocalized("sourceFundsParity.noTransactions"), systemImage: "line.3.horizontal.decrease.circle")
                }
            }
        }
        .padding(12)
    }

    private func filterPicker(_ label: String, selection: Binding<String>, values: [String]) -> some View {
        Picker(label, selection: selection) {
            ForEach(values, id: \.self) { value in
                Text(value == "all" ? reportsImportsLocalized("filter.all") : AppLocalization.code(value)).tag(value)
            }
        }
        .frame(maxWidth: 150)
    }
}

struct ReportsImportsSourceFundsScreen: View {
    let daemon: any DaemonClient
    @State private var model: ReportsImportsSourceFundsViewModel
    @State private var tab: ReportsImportsSourceFundsTab = .report
    @State private var caseLabel = ""
    @State private var transactionDetail: TransactionRow?

    init(daemon: any DaemonClient) {
        self.daemon = daemon
        _model = State(initialValue: ReportsImportsSourceFundsViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            targetBar
            Divider()
            Picker("", selection: $tab) {
                ForEach(ReportsImportsSourceFundsTab.allCases) { item in
                    Text(reportsImportsLocalized("sourceFundsParity.tab.\(item.rawValue)")).tag(item)
                }
            }
            .pickerStyle(.segmented)
            .padding(12)
            tabContent
            if let summary = model.lastMutationSummary {
                Text(reportsImportsDisplayMessage(summary))
                    .font(.caption).foregroundStyle(.secondary).padding(6)
            }
            if let error = model.errorMessage {
                Text(reportsImportsDisplayMessage(error))
                    .foregroundStyle(.red).font(.caption).padding(6)
            }
        }
        .navigationTitle(reportsImportsLocalized("nav.sourceFunds"))
        .task { await model.loadInventory() }
        .sheet(item: $transactionDetail) { row in
            TransactionDetailSheet(daemon: daemon, transaction: row) {
                transactionDetail = nil
                Task { await model.loadInventory(); await model.preview() }
            }
        }
    }

    private var targetBar: some View {
        VStack(spacing: 8) {
            HStack {
                TextField(reportsImportsLocalized("sourceFunds.target"), text: $model.targetTransaction)
                    .textFieldStyle(.roundedBorder)
                TextField(reportsImportsLocalized("sourceFundsParity.targetAmount"), text: $model.targetAmount)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 150)
                Picker(reportsImportsLocalized("sourceFundsParity.purpose"), selection: $model.reportPurpose) {
                    Text(reportsImportsLocalized("sourceFundsParity.existingTransaction")).tag("existing_transaction")
                    Text(reportsImportsLocalized("sourceFundsParity.plannedSale")).tag("planned_exchange_sale")
                }
                .frame(width: 190)
                Button(reportsImportsLocalized("sourceFunds.preview")) { Task { await model.preview() } }
                    .disabled(model.targetTransaction.isEmpty || model.isLoading)
                Menu(reportsImportsLocalized("action.export")) {
                    Button("PDF") { export(.uiSourceFundsExportPdf) }
                    Button(reportsImportsLocalized("sourceFunds.bundle")) { export(.uiSourceFundsExportBundle) }
                }
                .disabled(!model.exportable || model.isMutating)
            }
            HStack {
                if model.reportPurpose == "planned_exchange_sale" {
                    TextField(reportsImportsLocalized("sourceFundsParity.destination"), text: $model.plannedDestination)
                    TextField(reportsImportsLocalized("sourceFundsParity.plannedNote"), text: $model.plannedNote)
                }
                Picker(reportsImportsLocalized("sourceFunds.reveal"), selection: $model.revealMode) {
                    ForEach(["labels_only", "minimal", "standard", "full"], id: \.self) { Text(AppLocalization.code($0)).tag($0) }
                }
                .frame(width: 160)
                Picker(reportsImportsLocalized("sourceFundsParity.recipient"), selection: $model.selectedRecipientID) {
                    Text(reportsImportsLocalized("sourceFundsParity.noRecipient")).tag("")
                    ForEach(model.recipients) { Text($0.label).tag($0.id) }
                }
                .frame(width: 180)
            }
        }
        .padding(12)
    }

    private var tabContent: AnyView {
        switch tab {
        case .report: AnyView(reportTab)
        case .sources: AnyView(sourcesTab)
        case .links: AnyView(linksTab)
        case .evidence: AnyView(evidenceTab)
        case .coverage: AnyView(coverageTab)
        case .recipients: AnyView(recipientsTab)
        case .cases: AnyView(casesTab)
        }
    }

    private var reportTab: some View {
        VStack(spacing: 0) {
            ReportsImportsTargetPicker(model: model) { transactionDetail = $0 }
                .frame(minHeight: 265, idealHeight: 320, maxHeight: 390)
            Divider()
            HSplitView {
                Form {
                Section(reportsImportsLocalized("sourceFundsParity.disclosure")) {
                    Picker(reportsImportsLocalized("sourceFundsParity.diagram"), selection: $model.diagramDetail) {
                        Text(reportsImportsLocalized("sourceFundsParity.summary")).tag("summary")
                        Text(reportsImportsLocalized("sourceFundsParity.detailed")).tag("detailed")
                    }
                    Picker(reportsImportsLocalized("sourceFundsParity.amountPrecision"), selection: $model.amountPrecision) {
                        Text("BTC").tag("btc")
                        Text("sats").tag("sats")
                    }
                    Toggle(reportsImportsLocalized("sourceFundsParity.maskRecipient"), isOn: $model.maskRecipient)
                    ForEach(["txids", "attachments", "wallets", "narrative"], id: \.self) { section in
                        ReportsImportsDisclosureSectionRow(model: model, section: section)
                    }
                }
                Section(reportsImportsLocalized("sourceFundsParity.readiness")) {
                    LabeledContent(reportsImportsLocalized("sourceFundsParity.target"), value: model.targetLabel.isEmpty ? "—" : model.targetLabel)
                    LabeledContent(reportsImportsLocalized("sourceFundsParity.blockers"), value: String(model.blockers))
                    LabeledContent(reportsImportsLocalized("sourceFundsParity.warnings"), value: String(model.warnings))
                    Label(
                        model.exportable ? reportsImportsLocalized("sourceFundsParity.exportable") : reportsImportsLocalized("sourceFundsParity.notExportable"),
                        systemImage: model.exportable ? "checkmark.shield" : "exclamationmark.triangle"
                    )
                }
            }
                .formStyle(.grouped)
                .frame(minWidth: 280, idealWidth: 340)
                List {
                if !model.narrative.isEmpty {
                    Section(reportsImportsLocalized("sourceFundsParity.narrative")) {
                        ForEach(Array(model.narrative.enumerated()), id: \.offset) { _, paragraph in Text(paragraph) }
                    }
                }
                Section(reportsImportsLocalized("sourceFunds.findings")) {
                    ForEach(model.findings) { finding in
                        VStack(alignment: .leading) {
                            Label(finding.message, systemImage: finding.severity == "error" ? "xmark.octagon" : "exclamationmark.triangle")
                            if !finding.reference.isEmpty { Text(finding.reference).font(.caption).foregroundStyle(.secondary) }
                            if let transaction = model.transactions.first(where: {
                                $0.id == finding.reference || $0.transactionID == finding.reference
                            }) {
                                Button(reportsImportsLocalized("sourceFundsParity.details")) { transactionDetail = transaction }
                                    .buttonStyle(.link)
                            }
                        }
                    }
                }
                }
            }
        }
    }

    private var sourcesTab: some View {
        HSplitView {
            Table(model.sources) {
                TableColumn(reportsImportsLocalized("field.name"), value: \.label)
                TableColumn(reportsImportsLocalized("field.type")) { row in Text(AppLocalization.code(row.type)) }
                TableColumn(reportsImportsLocalized("field.asset"), value: \.asset)
                TableColumn(reportsImportsLocalized("field.amount")) { row in Text(row.amount.map { String($0) } ?? "—").monospacedDigit().kassiberSensitive() }
                TableColumn(reportsImportsLocalized("sourceFundsParity.evidence")) { Text(String($0.attachmentCount)) }
            }
            .frame(minWidth: 430)
            Form {
                Section(reportsImportsLocalized("sourceFundsParity.addSource")) {
                    Picker(reportsImportsLocalized("field.type"), selection: $model.sourceType) {
                        ForEach(["fiat_purchase", "exchange_withdrawal", "mining", "income", "gift", "opening_balance_attestation", "missing_history", "unknown"], id: \.self) { Text(AppLocalization.code($0)).tag($0) }
                    }
                    TextField(reportsImportsLocalized("field.name"), text: $model.sourceLabel)
                    TextField(reportsImportsLocalized("field.asset"), text: $model.sourceAsset)
                    TextField(reportsImportsLocalized("field.amount"), text: $model.sourceAmount)
                    TextField(reportsImportsLocalized("sourceFundsParity.description"), text: $model.sourceDescription)
                    evidencePicker(selection: $model.selectedSourceAttachmentID)
                    Button(reportsImportsLocalized("sourceFundsParity.createSource")) { Task { await model.createSource() } }
                        .disabled(model.sourceLabel.isEmpty || model.sourceAmount.isEmpty || model.isMutating)
                }
                Section(reportsImportsLocalized("sourceFundsParity.attachExisting")) {
                    ForEach(model.sources) { source in
                        Button(source.label) { Task { await model.attachSelectedSource(source.id) } }
                            .disabled(model.selectedSourceAttachmentID.isEmpty)
                    }
                }
            }
            .formStyle(.grouped)
            .frame(minWidth: 300, idealWidth: 360)
        }
    }

    private var linksTab: some View {
        HSplitView {
            VStack(spacing: 8) {
                HStack {
                    Button(reportsImportsLocalized("sourceFundsParity.suggest")) { Task { await model.suggest() } }
                    Button(reportsImportsLocalized("sourceFundsParity.assemble")) { Task { await model.assemble() } }
                    Button(reportsImportsLocalized("sourceFundsParity.bulkReview")) { Task { await model.bulkReview() } }
                    Spacer()
                }
                .padding([.top, .horizontal], 10)
                List(model.links, selection: $model.selectedLinkID) { link in
                    Button { model.selectLink(link.id) } label: {
                        HStack {
                            VStack(alignment: .leading) {
                                Text(AppLocalization.code(link.type))
                                Text([link.fromSource.isEmpty ? link.fromTransaction : link.fromSource, link.toTransaction].filter { !$0.isEmpty }.joined(separator: " → "))
                                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                            Spacer()
                            Text(AppLocalization.code(link.state)).font(.caption)
                        }
                    }
                    .buttonStyle(.plain)
                    .tag(link.id)
                }
            }
            .frame(minWidth: 430)
            Form {
                Section(reportsImportsLocalized("sourceFundsParity.linkEditor")) {
                    TextField(reportsImportsLocalized("sourceFundsParity.fromTransaction"), text: $model.linkFromTransaction)
                    TextField(reportsImportsLocalized("sourceFundsParity.fromSource"), text: $model.linkFromSource)
                    TextField(reportsImportsLocalized("sourceFundsParity.toTransaction"), text: $model.linkToTransaction)
                    Picker(reportsImportsLocalized("field.type"), selection: $model.linkType) {
                        ForEach(["self_transfer", "exchange_transfer", "trade", "swap", "peg_in", "peg_out", "lightning_funding", "lightning_close", "lightning_routed", "lightning_swap", "coinjoin", "payjoin", "manual_source", "missing_history"], id: \.self) { Text(AppLocalization.code($0)).tag($0) }
                    }
                    Picker(reportsImportsLocalized("sourceFundsParity.confidence"), selection: $model.linkConfidence) {
                        ForEach(["exact", "strong", "weak", "unknown"], id: \.self) { Text(AppLocalization.code($0)).tag($0) }
                    }
                    TextField(reportsImportsLocalized("field.amount"), text: $model.linkAmount)
                    TextField(reportsImportsLocalized("sourceFundsParity.fromAmount"), text: $model.linkFromAmount)
                    TextField(reportsImportsLocalized("sourceFundsParity.explanation"), text: $model.linkExplanation)
                    evidencePicker(selection: $model.selectedLinkAttachmentID)
                    HStack {
                        Button(reportsImportsLocalized("sourceFundsParity.createLink")) { Task { await model.createLink() } }
                        Button(reportsImportsLocalized("sourceFundsParity.accept")) { Task { await model.reviewSelectedLink(state: "reviewed") } }
                        Button(reportsImportsLocalized("sourceFundsParity.reject"), role: .destructive) { Task { await model.reviewSelectedLink(state: "rejected") } }
                    }
                    Button(reportsImportsLocalized("sourceFundsParity.attachEvidence")) { Task { await model.attachSelectedLink() } }
                        .disabled(model.selectedLinkID.isEmpty || model.selectedLinkAttachmentID.isEmpty)
                }
            }
            .formStyle(.grouped)
            .frame(minWidth: 330, idealWidth: 390)
        }
    }

    private var evidenceTab: some View {
        Table(model.evidence) {
            TableColumn(reportsImportsLocalized("field.name"), value: \.label)
            TableColumn(reportsImportsLocalized("field.type")) { row in Text(AppLocalization.code(row.type)) }
            TableColumn(reportsImportsLocalized("nav.transactions"), value: \.transaction)
            TableColumn(reportsImportsLocalized("field.wallet"), value: \.wallet)
        }
    }

    private var coverageTab: some View {
        Table(model.coverage) {
            TableColumn(reportsImportsLocalized("field.name"), value: \.label)
            TableColumn(reportsImportsLocalized("sourceFundsParity.total")) { Text(String($0.total)).monospacedDigit().kassiberSensitive() }
            TableColumn(reportsImportsLocalized("sourceFundsParity.fullyTraced")) { Text(String($0.fullyTraced)).monospacedDigit().kassiberSensitive() }
            TableColumn(reportsImportsLocalized("sourceFundsParity.attested")) { Text(String($0.attested)).monospacedDigit().kassiberSensitive() }
            TableColumn(reportsImportsLocalized("sourceFundsParity.inReview")) { Text(String($0.inReview)).monospacedDigit().kassiberSensitive() }
            TableColumn(reportsImportsLocalized("sourceFundsParity.untraced")) { Text(String($0.untraced)).monospacedDigit().kassiberSensitive() }
        }
    }

    private var recipientsTab: some View {
        HSplitView {
            List(model.recipients) { recipient in
                Button { model.editRecipient(recipient) } label: {
                    HStack {
                        VStack(alignment: .leading) {
                            Text(recipient.label)
                            Text("\(AppLocalization.code(recipient.kind)) · \(AppLocalization.code(recipient.revealMode))").font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if !recipient.active { Image(systemName: "archivebox") }
                    }
                }
                .buttonStyle(.plain)
            }
            Form {
                Section(reportsImportsLocalized("sourceFundsParity.recipientEditor")) {
                    TextField(reportsImportsLocalized("field.name"), text: $model.recipientLabel)
                    TextField(reportsImportsLocalized("field.type"), text: $model.recipientKind)
                    Picker(reportsImportsLocalized("sourceFunds.reveal"), selection: $model.recipientRevealMode) {
                        ForEach(["labels_only", "minimal", "standard", "full"], id: \.self) { Text(AppLocalization.code($0)).tag($0) }
                    }
                    TextField(reportsImportsLocalized("sourceFundsParity.notes"), text: $model.recipientNotes)
                    HStack {
                        Button(reportsImportsLocalized("action.create")) { Task { await model.createRecipient() } }
                        Button(reportsImportsLocalized("action.save")) { Task { await model.updateRecipient(model.selectedRecipientID) } }
                            .disabled(model.selectedRecipientID.isEmpty)
                        Button(reportsImportsLocalized("action.delete"), role: .destructive) { Task { await model.deleteRecipient(model.selectedRecipientID) } }
                            .disabled(model.selectedRecipientID.isEmpty)
                    }
                }
            }
            .formStyle(.grouped)
            .frame(minWidth: 320, idealWidth: 380)
        }
    }

    private var casesTab: some View {
        VStack(spacing: 8) {
            HStack {
                TextField(reportsImportsLocalized("sourceFundsParity.caseLabel"), text: $caseLabel)
                    .textFieldStyle(.roundedBorder)
                Button(reportsImportsLocalized("sourceFundsParity.saveCase")) { Task { await model.saveCase(label: caseLabel) } }
                    .disabled(model.targetTransaction.isEmpty)
                Spacer()
            }
            .padding(.horizontal, 12)
            Table(model.cases) {
                TableColumn(reportsImportsLocalized("field.name"), value: \.label)
                TableColumn(reportsImportsLocalized("sourceFundsParity.target"), value: \.target)
                TableColumn(reportsImportsLocalized("field.status")) { row in Text(AppLocalization.code(row.status)) }
                TableColumn("") { row in
                    HStack {
                        Button("PDF") { export(.uiSourceFundsExportPdf, caseID: row.id) }.buttonStyle(.link)
                        Button("ZIP") { export(.uiSourceFundsExportBundle, caseID: row.id) }.buttonStyle(.link)
                    }
                }
                .width(100)
            }
        }
    }

    private func evidencePicker(selection: Binding<String>) -> some View {
        Picker(reportsImportsLocalized("sourceFundsParity.evidence"), selection: selection) {
            Text(reportsImportsLocalized("sourceFundsParity.noEvidence")).tag("")
            ForEach(model.evidence) { Text($0.label).tag($0.id) }
        }
    }

    private func export(_ kind: DaemonKind, caseID: String? = nil) {
        Task {
            if caseID == nil, model.savedCaseID == nil { await model.saveCase(label: caseLabel) }
            await model.export(kind, caseID: caseID)
            guard let artifact = model.artifact else { return }
            reportsImportsSave(artifact, titleKey: "sourceFundsParity.saveExport")
            model.clearArtifact()
        }
    }
}

// MARK: - Complete import and interchange workstation

private enum ReportsImportsImportTab: String, CaseIterable, Identifiable {
    case files, bip329, samourai, templates
    var id: String { rawValue }
}

struct ReportsImportsImportScreen: View {
    @State private var model: ReportsImportsImportViewModel
    @State private var tab: ReportsImportsImportTab = .files
    @Environment(\.kassiberNavigate) private var navigate

    init(daemon: any DaemonClient) {
        _model = State(initialValue: ReportsImportsImportViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            Picker("", selection: $tab) {
                ForEach(ReportsImportsImportTab.allCases) { item in
                    Text(reportsImportsLocalized("importsParity.tab.\(item.rawValue)")).tag(item)
                }
            }
            .pickerStyle(.segmented)
            .padding(12)
            Divider()
            switch tab {
            case .files: fileImportTab
            case .bip329: bip329Tab
            case .samourai: samouraiTab
            case .templates: templatesTab
            }
            if !model.resultMetrics.isEmpty { resultBar }
            if let error = model.errorMessage {
                Text(reportsImportsDisplayMessage(error))
                    .foregroundStyle(.red).font(.caption).padding(8)
            }
        }
        .navigationTitle(reportsImportsLocalized("nav.imports"))
        .toolbar {
            Button { navigate(.connections) } label: {
                Label(reportsImportsLocalized("importsParity.connections"), systemImage: "arrow.left")
            }
        }
        .task { consumePendingImportFormat() }
        .onReceive(NotificationCenter.default.publisher(for: KassiberHostNotification.openImportFormat)) { _ in
            consumePendingImportFormat()
        }
    }

    private func consumePendingImportFormat() {
        let defaults = UserDefaults.standard
        guard let raw = defaults.string(
            forKey: KassiberHostNotification.pendingImportFormatDefaultsKey
        ), let format = ReportsImportsFileFormat(rawValue: raw) else { return }
        defaults.removeObject(forKey: KassiberHostNotification.pendingImportFormatDefaultsKey)
        tab = .files
        model.format = format
    }

    private var fileImportTab: some View {
        HSplitView {
            fileImportForm
            filePreviewPane
        }
    }

    private var fileImportForm: some View {
        Form {
            Section(reportsImportsLocalized("importsParity.fileImport")) {
                Picker(reportsImportsLocalized("importsParity.format"), selection: $model.format) {
                    ForEach(ReportsImportsFileFormat.allCases) { format in
                        Text(reportsImportsLocalized(format.localizationKey)).tag(format)
                    }
                }
                HStack {
                    TextField(reportsImportsLocalized("importsParity.file"), text: $model.sourcePath)
                    Button(reportsImportsLocalized("importsParity.choose")) { chooseImportFile() }
                }
                Toggle(reportsImportsLocalized("importsParity.createWalletFirst"), isOn: $model.createWalletFirst)
                if model.createWalletFirst {
                    TextField(reportsImportsLocalized("importsParity.newWallet"), text: $model.newWalletLabel)
                    Toggle(reportsImportsLocalized("importsParity.importNow"), isOn: $model.syncAfterCreate)
                } else {
                    TextField(reportsImportsLocalized("field.wallet"), text: $model.wallet)
                }
                if [.bullBitcoin, .binanceSupplemental].contains(model.format) {
                    Picker(reportsImportsLocalized("importsParity.mode"), selection: $model.importMode) {
                        Text(reportsImportsLocalized("importsParity.relevant")).tag("relevant")
                        Text(reportsImportsLocalized("importsParity.full")).tag("full")
                    }
                }
                if model.format == .wasabiBundle { wasabiFields }
                Button(reportsImportsLocalized("importsParity.preview")) {
                    Task { await model.previewCurrentFile() }
                }
                .disabled(model.isWorking || (model.sourcePath.isEmpty && model.format != .wasabiBundle))
                Button(reportsImportsLocalized("importsParity.runImport")) { Task { await model.runImport() } }
                    .buttonStyle(.borderedProminent)
                    .disabled(model.isWorking || !model.canRunImport)
                Text(reportsImportsLocalized("importsParity.previewRequired"))
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 340, idealWidth: 420)
    }

    private var filePreviewPane: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(reportsImportsLocalized("importsParity.preview")).font(.headline)
                Spacer()
                Text("\(model.previewMapped)/\(model.previewRead)").monospacedDigit()
                Image(systemName: model.canRunImport ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                    .foregroundStyle(model.canRunImport ? .green : .orange)
            }
            if let summary = model.filePreviewSummary { filePreviewSummary(summary) }
            Table(model.previewRows) {
                TableColumn(reportsImportsLocalized("field.date"), value: \.date)
                TableColumn(reportsImportsLocalized("field.type")) { row in Text(AppLocalization.code(row.kind)) }
                TableColumn(reportsImportsLocalized("field.amount"), value: \.amount)
                TableColumn(reportsImportsLocalized("importsParity.value"), value: \.value)
            }
            if !model.previewProblems.isEmpty {
                List(model.previewProblems, id: \.self) {
                    Label(reportsImportsDisplayMessage($0), systemImage: "exclamationmark.triangle")
                }
                    .frame(height: 110)
            }
        }
        .padding(12)
        .frame(minWidth: 420)
    }

    private func filePreviewSummary(_ summary: ReportsImportsFilePreviewSummary) -> some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 5) {
                LabeledContent(
                    reportsImportsLocalized("importsParity.previewFile"),
                    value: reportsImportsDisplayMessage(summary.filename)
                )
                LabeledContent(
                    reportsImportsLocalized("importsParity.previewBytes"),
                    value: ByteCountFormatter.string(
                        fromByteCount: Int64(summary.byteCount), countStyle: .file
                    )
                )
                LabeledContent(
                    reportsImportsLocalized("importsParity.previewRecords"),
                    value: String(summary.estimatedRecords)
                )
                LabeledContent(
                    reportsImportsLocalized("importsParity.previewDetails"),
                    value: reportsImportsDisplayMessage(summary.detail)
                )
            }
        }
    }

    private var wasabiFields: some View {
        Group {
            TextField(reportsImportsLocalized("importsParity.wasabiHistory"), text: $model.wasabiHistoryJSON, axis: .vertical)
                .lineLimit(2...6)
            TextField(reportsImportsLocalized("importsParity.wasabiCoins"), text: $model.wasabiCoinsJSON, axis: .vertical)
                .lineLimit(2...6)
            TextField(reportsImportsLocalized("importsParity.wasabiWalletInfo"), text: $model.wasabiWalletInfoJSON, axis: .vertical)
                .lineLimit(2...6)
            TextField(reportsImportsLocalized("importsParity.wasabiAdditional"), text: $model.wasabiAdditionalJSON, axis: .vertical)
                .lineLimit(2...6)
        }
    }

    private var bip329Tab: some View {
        VStack(spacing: 10) {
            HStack {
                TextField(reportsImportsLocalized("importsParity.labelFile"), text: $model.bip329Path)
                    .textFieldStyle(.roundedBorder)
                Button(reportsImportsLocalized("importsParity.choose")) {
                    if let url = reportsImportsPickFile(types: [.json, .plainText]) { model.bip329Path = url.path }
                }
                Button(reportsImportsLocalized("importsParity.preview")) { Task { await model.previewBip329() } }
                Button(reportsImportsLocalized("importsParity.importLabels")) { Task { await model.importBip329() } }
                    .disabled(model.isWorking || !model.canImportBip329)
            }
            HStack {
                Picker(reportsImportsLocalized("importsParity.exportMode"), selection: $model.bip329ExportMode) {
                    Text(reportsImportsLocalized("importsParity.stored")).tag("stored")
                    Text(reportsImportsLocalized("importsParity.synthesized")).tag("synthesized")
                    Text(reportsImportsLocalized("importsParity.all")).tag("all")
                }
                TextField(reportsImportsLocalized("field.wallet"), text: $model.bip329ExportWallet)
                    .textFieldStyle(.roundedBorder)
                Button(reportsImportsLocalized("importsParity.exportLabels")) { Task { await exportBip329() } }
            }
            HStack {
                ForEach(Array(model.bip329Counts.enumerated()), id: \.offset) { _, metric in
                    LabeledContent(AppLocalization.code(metric.0), value: String(metric.1))
                }
            }
            .padding(.horizontal, 12)
            Table(model.bip329Rows) {
                TableColumn(reportsImportsLocalized("field.type")) { row in Text(AppLocalization.code(row.type)) }
                TableColumn(reportsImportsLocalized("importsParity.reference"), value: \.reference)
                TableColumn(reportsImportsLocalized("importsParity.label"), value: \.label)
                TableColumn(reportsImportsLocalized("field.status")) { row in Text(AppLocalization.code(row.status)) }
            }
        }
        .padding(12)
    }

    private var samouraiTab: some View {
        Form {
            Section(reportsImportsLocalized("importsParity.samourai")) {
                TextField(reportsImportsLocalized("field.name"), text: $model.samouraiLabel)
                TextField(reportsImportsLocalized("importsParity.backend"), text: $model.samouraiBackend)
                Picker(reportsImportsLocalized("field.network"), selection: $model.samouraiNetwork) {
                    Text("main").tag("main")
                    Text("testnet").tag("testnet")
                    Text("regtest").tag("regtest")
                }
                Stepper(reportsImportsLocalized("importsParity.gapLimit") + ": \(model.samouraiGapLimit)", value: $model.samouraiGapLimit, in: 1...10_000)
                Text(reportsImportsLocalized("importsParity.samouraiHint")).font(.caption).foregroundStyle(.secondary)
                TextField(reportsImportsLocalized("importsParity.samouraiDeposit"), text: $model.samouraiDeposit, axis: .vertical)
                    .lineLimit(2...5)
                TextField(reportsImportsLocalized("importsParity.samouraiBadbank"), text: $model.samouraiBadbank, axis: .vertical)
                    .lineLimit(2...5)
                TextField(reportsImportsLocalized("importsParity.samouraiPremix"), text: $model.samouraiPremix, axis: .vertical)
                    .lineLimit(2...5)
                TextField(reportsImportsLocalized("importsParity.samouraiPostmix"), text: $model.samouraiPostmix, axis: .vertical)
                    .lineLimit(2...5)
                DisclosureGroup(reportsImportsLocalized("importsParity.samouraiAdvanced")) {
                TextEditor(text: $model.samouraiSourceSetJSON)
                    .font(.system(.body, design: .monospaced))
                    .frame(minHeight: 220)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
                }
                Button(reportsImportsLocalized("importsParity.importSamourai")) { Task { await model.importSamourai() } }
                    .buttonStyle(.borderedProminent)
                    .disabled(
                        model.isWorking || (
                            model.samouraiSourceSetJSON.isEmpty && model.samouraiDeposit.isEmpty &&
                            model.samouraiBadbank.isEmpty && model.samouraiPremix.isEmpty && model.samouraiPostmix.isEmpty
                        )
                    )
            }
        }
        .formStyle(.grouped)
    }

    private var templatesTab: some View {
        ContentUnavailableView {
            Label(reportsImportsLocalized("importsParity.templates"), systemImage: "tablecells")
        } description: {
            Text(reportsImportsLocalized("importsParity.templatesHint"))
        } actions: {
            HStack {
                Button(reportsImportsLocalized("importsParity.xlsxTemplate")) { Task { await exportTemplate("xlsx") } }
                Button(reportsImportsLocalized("importsParity.csvTemplate")) { Task { await exportTemplate("csv") } }
            }
        }
    }

    private var resultBar: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 18) {
                ForEach(Array(model.resultMetrics.enumerated()), id: \.offset) { _, metric in
                    LabeledContent(AppLocalization.code(metric.0), value: metric.1)
                }
            }
            .padding(8)
        }
        .background(.quaternary.opacity(0.35))
    }

    private func chooseImportFile() {
        let types: [UTType] = model.format == .genericLedger ? [.commaSeparatedText, .tabSeparatedText, .spreadsheet] : [.data]
        if let url = reportsImportsPickFile(types: types) { model.sourcePath = url.path }
    }

    private func exportBip329() async {
        await model.exportBip329()
        guard let artifact = model.artifact else { return }
        reportsImportsSave(artifact, titleKey: "importsParity.saveLabels")
        model.clearArtifact()
    }

    private func exportTemplate(_ format: String) async {
        await model.exportLedgerTemplate(format: format)
        guard let artifact = model.artifact else { return }
        reportsImportsSave(artifact, titleKey: "importsParity.saveTemplate")
        model.clearArtifact()
    }
}
