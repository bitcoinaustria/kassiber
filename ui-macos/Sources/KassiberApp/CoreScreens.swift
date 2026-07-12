import SwiftUI
import Charts
import AppKit
import UniformTypeIdentifiers
import KassiberDaemonKit
import KassiberViewModels

private func localized(_ key: String) -> String {
    AppLocalization.string(key)
}

private struct ScreenErrorView: View {
    let message: String
    var retry: (() -> Void)?

    var body: some View {
        ContentUnavailableView {
            Label(localized("state.unavailable"), systemImage: "exclamationmark.triangle")
        } description: {
            Text(AppLocalization.error(message))
        } actions: {
            if let retry {
                Button(localized("action.retry"), action: retry)
            }
        }
    }
}

struct DashboardScreen: View {
    let daemon: any DaemonClient
    @State private var model: DashboardViewModel
    @State private var selectedTransaction: TransactionRow?
    @Environment(\.locale) private var locale
    @Environment(\.kassiberDisplayCurrency) private var displayCurrency

    init(daemon: any DaemonClient) {
        self.daemon = daemon
        _model = State(initialValue: DashboardViewModel(daemon: daemon))
    }

    var body: some View {
        Group {
            if model.isLoading && model.fiatBalance == nil {
                ProgressView(localized("state.loading"))
            } else if let error = model.errorMessage, model.fiatBalance == nil {
                ScreenErrorView(message: error) { Task { await model.load() } }
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 18) {
                        header
                        metrics
                        if !model.portfolio.isEmpty || !model.activityTransactions.isEmpty {
                            DashboardActivityChart(
                                points: model.portfolio,
                                transactions: model.activityTransactions,
                                fiatCurrency: model.fiatCurrency,
                                marketRate: {
                                    guard let balance = model.totalBTC, balance > 0, let fiat = model.fiatBalance else { return nil }
                                    return fiat / balance
                                }(),
                                onOpenTransaction: { selectedTransaction = $0 }
                            )
                            overviewVisuals
                        }
                        HStack(alignment: .top, spacing: 16) {
                            connections
                            recentTransactions
                        }
                    }
                    .padding(22)
                }
            }
        }
        .navigationTitle(localized("nav.dashboard"))
        .toolbar {
            ToolbarItemGroup {
                Button { Task { await model.refreshLatestRate() } } label: { Label(localized("dashboard.refreshPrice"), systemImage: "eurosign.arrow.circlepath") }
                Button { Task { await model.load() } } label: {
                    Label(localized("action.refresh"), systemImage: "arrow.clockwise")
                }
            }
        }
        .task { if model.fiatBalance == nil { await model.load() } }
        .sheet(item: $selectedTransaction) { row in
            TransactionDetailSheet(daemon: daemon, transaction: row) {
                selectedTransaction = nil
                Task { await model.load() }
            }
        }
    }

    private var overviewVisuals: some View {
        HStack(alignment: .top, spacing: 16) {
            GroupBox {
                Chart(model.holdings) { item in
                    SectorMark(
                        angle: .value(localized("dashboard.chart.holdings"), item.valueBTC),
                        innerRadius: .ratio(0.58), angularInset: 2
                    )
                    .foregroundStyle(by: .value(localized("field.name"), item.label))
                }
                .chartLegend(position: .bottom, alignment: .leading)
                .frame(height: 180)
            } label: {
                Label(localized("dashboard.chart.holdings"), systemImage: "chart.pie")
            }
            .frame(maxWidth: .infinity)

            GroupBox {
                Chart(model.drivers) { driver in
                    BarMark(
                        x: .value(localized("dashboard.chart.amount"), driver.valueBTC),
                        y: .value(localized("dashboard.chart.driver"), localized("dashboard.driver.\(driver.id)"))
                    )
                    .foregroundStyle(by: .value(localized("dashboard.chart.driver"), localized("dashboard.driver.\(driver.id)")))
                    .annotation(position: .trailing) {
                        Text(driver.count, format: .number).font(.caption2.monospacedDigit())
                    }
                }
                .chartLegend(.hidden)
                .frame(height: 180)
            } label: {
                Label(localized("dashboard.chart.drivers"), systemImage: "arrow.left.arrow.right")
            }
            .frame(maxWidth: .infinity)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(model.bookLabel.isEmpty ? localized("dashboard.title") : model.bookLabel)
                .font(.title2.weight(.semibold))
            if !model.workspaceLabel.isEmpty {
                Text(model.workspaceLabel)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            if model.needsJournals || model.quarantines > 0 {
                HStack {
                    if model.needsJournals {
                        Label(localized("dashboard.journalsNeeded"), systemImage: "books.vertical")
                    }
                    if model.quarantines > 0 {
                        Label(
                            String(format: localized("dashboard.quarantines"), model.quarantines),
                            systemImage: "exclamationmark.triangle"
                        )
                    }
                }
                .font(.callout)
                .foregroundStyle(.orange)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var metrics: some View {
        Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 10) {
            GridRow {
                metricCard(localized("dashboard.balance"), dashboardBalance)
                metricCard(localized("dashboard.marketValue"), model.fiatBalance.map { KassiberFormatting.fiat($0, currency: model.fiatCurrency, locale: locale) } ?? "—")
                metricCard(localized("dashboard.costBasis"), model.fiatCostBasis.map { KassiberFormatting.fiat($0, currency: model.fiatCurrency, locale: locale) } ?? "—")
                metricCard(localized("dashboard.unrealized"), model.fiatUnrealized.map { KassiberFormatting.fiat($0, currency: model.fiatCurrency, locale: locale) } ?? "—")
            }
        }
    }

    private func metricCard(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(label)
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
            Text(value)
                .font(.body.weight(.semibold).monospacedDigit())
                .lineLimit(1)
                .kassiberSensitive()
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .kassiberCardSurface(cornerRadius: 11)
    }

    private var portfolioChart: some View {
        GroupBox(localized("dashboard.portfolioHistory")) {
            Chart(model.portfolio) { point in
                AreaMark(
                    x: .value(localized("field.date"), point.date),
                    y: .value(localized("dashboard.marketValue"), point.fiatValue)
                )
                .foregroundStyle(.tint.opacity(0.18))
                LineMark(
                    x: .value(localized("field.date"), point.date),
                    y: .value(localized("dashboard.marketValue"), point.fiatValue)
                )
                .foregroundStyle(.tint)
            }
            .chartYAxis { AxisMarks(position: .leading) }
            .frame(height: 190)
            .padding(.top, 8)
        }
    }

    private var connections: some View {
        GroupBox(localized("nav.wallets")) {
            VStack(spacing: 0) {
                ForEach(model.connections.prefix(8)) { connection in
                    HStack {
                        Image(systemName: "wallet.bifold")
                        VStack(alignment: .leading) {
                            Text(connection.label)
                            Text(AppLocalization.code(connection.kind)).font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Text(kassiberBitcoinString(btc: connection.balanceBTC, rateEUR: dashboardRateEUR, mode: displayCurrency, locale: locale))
                            .monospacedDigit()
                            .kassiberSensitive()
                    }
                    .padding(.vertical, 7)
                    if connection.id != model.connections.prefix(8).last?.id { Divider() }
                }
            }
        }
        .frame(maxWidth: .infinity)
    }

    private var recentTransactions: some View {
        GroupBox(localized("dashboard.recentTransactions")) {
            VStack(spacing: 0) {
                ForEach(model.recentTransactions.prefix(8)) { row in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(row.counterparty.isEmpty ? AppLocalization.code(row.type) : row.counterparty).lineLimit(1)
                            Text(row.wallet).font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        KassiberAmountText(transaction: row, rateEUR: dashboardRateEUR)
                            .monospacedDigit()
                            .kassiberSensitive()
                    }
                    .padding(.vertical, 7)
                    if row.id != model.recentTransactions.prefix(8).last?.id { Divider() }
                }
            }
        }
        .frame(maxWidth: .infinity)
    }

    private var dashboardRateEUR: Double? {
        guard let btc = model.totalBTC, btc != 0, let fiat = model.fiatBalance else { return nil }
        return fiat / btc
    }

    private var dashboardBalance: String {
        guard let btc = model.totalBTC else { return "—" }
        return kassiberBitcoinString(btc: btc, rateEUR: dashboardRateEUR, mode: displayCurrency, locale: locale)
    }
}

struct TransactionsScreen: View {
    let daemon: any DaemonClient
    @State private var model: TransactionsViewModel
    @State private var selectedID: TransactionRow.ID?
    @State private var detail: TransactionRow?
    @State private var exporter: ReportExportViewModel
    @State private var showingNewDraft = false
    @State private var chartSelectionDate: Date?
    @State private var newDraft = NewTransactionDraft()
    @AppStorage("transactions.selectedID") private var requestedTransactionID = ""
    @Environment(\.locale) private var locale
    @Environment(\.kassiberDisplayCurrency) private var displayCurrency

    init(daemon: any DaemonClient) {
        self.daemon = daemon
        _model = State(initialValue: TransactionsViewModel(daemon: daemon))
        _exporter = State(initialValue: ReportExportViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            if let error = model.errorMessage, model.rows.isEmpty {
                ScreenErrorView(message: error) { Task { await model.reload() } }
            } else {
                filterBar
                if model.showWorkbench {
                    Divider()
                    transactionWorkbench
                }
                Divider()
                Table(model.visibleRows, selection: $selectedID) {
                    TableColumn(localized("field.date")) { row in
                        Text(row.occurredAt.map { KassiberFormatting.date($0, locale: locale) } ?? row.dateLabel)
                    }
                    .width(min: 130, ideal: 155)
                    TableColumn(localized("field.type")) { row in
                        Text(AppLocalization.code(row.type))
                    }
                        .width(min: 75, ideal: 100)
                    TableColumn(localized("field.wallet"), value: \.wallet)
                        .width(min: 110, ideal: 150)
                    TableColumn(localized("field.counterparty"), value: \.counterparty)
                        .width(min: 140, ideal: 190)
                    TableColumn(localized("field.amount")) { row in
                        KassiberAmountText(transaction: row)
                            .monospacedDigit()
                            .foregroundStyle(row.amountSats < 0 ? .secondary : .primary)
                            .kassiberSensitive()
                    }
                    .width(min: 120, ideal: 150)
                    TableColumn(localized("field.status")) { row in
                        Text(AppLocalization.code(row.reviewStatus))
                    }
                        .width(min: 90, ideal: 110)
                }
                .onChange(of: selectedID) { _, id in
                    guard let id, let row = model.rows.first(where: { $0.id == id }) else { return }
                    detail = row
                }
                if let cursor = model.nextCursor, !cursor.isEmpty {
                    Button(model.isLoading ? localized("state.loading") : localized("action.loadMore")) {
                        Task { await model.loadMore() }
                    }
                    .disabled(model.isLoading)
                    .padding(8)
                }
            }
        }
        .navigationTitle(localized("nav.transactions"))
        .searchable(text: $model.search, prompt: localized("action.search"))
        .toolbar {
            ToolbarItemGroup {
                Button {
                    if newDraft.wallet.isEmpty, let firstWallet = draftWallets.first {
                        newDraft = NewTransactionDraft(wallet: firstWallet)
                    }
                    showingNewDraft = true
                } label: {
                    Label(localized("transactionDraft.trigger"), systemImage: "plus")
                }
                Menu(localized("action.export")) {
                    Button("CSV") { exportTransactions(.uiTransactionsExportCsv) }
                    Button("XLSX") { exportTransactions(.uiTransactionsExportXlsx) }
                }
                Button {
                    model.showWorkbench.toggle()
                } label: {
                    Label(
                        localized(model.showWorkbench ? "transactions.hideWorkbench" : "transactions.showWorkbench"),
                        systemImage: model.showWorkbench ? "rectangle.compress.vertical" : "chart.bar.xaxis"
                    )
                }
                Button { Task { await model.reload() } } label: {
                    Label(localized("action.refresh"), systemImage: "arrow.clockwise")
                }
            }
        }
        .sheet(item: $detail) { row in
            TransactionDetailSheet(daemon: daemon, transaction: row) {
                detail = nil
                Task { await model.reload() }
            }
        }
        .sheet(isPresented: $showingNewDraft) {
            NewTransactionDraftSheet(draft: $newDraft, wallets: draftWallets) {}
        }
        .onChange(of: model.period) { _, _ in Task { await model.reload() } }
        .onChange(of: model.flow) { _, _ in Task { await model.reload() } }
        .onChange(of: model.reviewStatus) { _, _ in Task { await model.reload() } }
        .onChange(of: model.paymentMethod) { _, _ in Task { await model.reload() } }
        .onChange(of: model.feeOnly) { _, _ in Task { await model.reload() } }
        .onChange(of: model.quickFilter) { _, _ in Task { await model.reload() } }
        .onChange(of: model.sort) { _, _ in Task { await model.reload() } }
        .task {
            if model.rows.isEmpty { await model.load() }
            await openRequestedTransaction()
        }
        .onChange(of: requestedTransactionID) { _, reference in
            guard !reference.isEmpty else { return }
            Task { await openRequestedTransaction() }
        }
    }

    private func exportTransactions(_ kind: DaemonKind) {
        Task {
            var args: [String: JSONValue] = [:]
            if let wallet = model.walletScope { args["wallet"] = .string(wallet) }
            await exporter.export(kind, args: args.isEmpty ? nil : args)
            if let artifact = exporter.artifact {
                _ = try? saveExportArtifact(artifact, title: localized("export.saveReport"))
                exporter.clearArtifact()
            }
        }
    }

    private var draftWallets: [String] {
        var seen = Set<String>()
        let labels = model.rows.map(\.wallet)
            .filter { !$0.isEmpty && !$0.contains("→") && seen.insert($0).inserted }
        return labels.isEmpty ? [localized("transactionDraft.unassigned")] : labels
    }

    private func openRequestedTransaction() async {
        let reference = requestedTransactionID
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !reference.isEmpty else { return }
        defer { requestedTransactionID = "" }
        guard let transaction = await model.resolveTransaction(reference) else { return }
        selectedID = model.rows.contains(where: { $0.id == transaction.id }) ? transaction.id : nil
        detail = transaction
    }

    private var filterBar: some View {
        VStack(spacing: 8) {
            HStack {
                Picker(localized("transactions.period"), selection: $model.period) {
                    ForEach(TransactionPeriodFilter.allCases) { period in
                        Text(localized("period.\(period.rawValue)")).tag(period)
                    }
                }
                .frame(maxWidth: 190)
                Picker(localized("field.wallet"), selection: $model.walletScope) {
                    Text(localized("filter.allWallets")).tag(String?.none)
                    ForEach(model.wallets) { wallet in
                        Text(wallet.label).tag(String?.some(wallet.id))
                    }
                }
                .frame(maxWidth: 230)
                Picker(localized("filter.flow"), selection: $model.flow) {
                    ForEach(TransactionFlowFilter.allCases) { flow in
                        Text(localized("flow.\(flow.rawValue)")).tag(flow)
                    }
                }
                .frame(maxWidth: 170)
                Picker(localized("transactions.status"), selection: $model.reviewStatus) {
                    ForEach(["all", "completed", "pending", "review", "failed"], id: \.self) { status in
                        Text(localized("status.\(status)")).tag(status)
                    }
                }
                .frame(maxWidth: 150)
                Spacer()
            }
            HStack {
                Picker(localized("transactions.paymentMethod"), selection: $model.paymentMethod) {
                    ForEach(TransactionPaymentFilter.allCases) { method in
                        Text(localized("payment.\(method.id)")).tag(method)
                    }
                }
                .frame(maxWidth: 230)
                Picker(localized("transactions.quickFilter"), selection: $model.quickFilter) {
                    ForEach(TransactionQuickFilter.allCases) { quick in
                        Text(localized("quick.\(quick.rawValue)")).tag(quick)
                    }
                }
                .frame(maxWidth: 260)
                Toggle(localized("filter.fees"), isOn: $model.feeOnly)
                Picker(localized("action.sort"), selection: $model.sort) {
                    ForEach(TransactionSort.allCases) { sort in
                        Text(localized("sort.\(sort.rawValue)")).tag(sort)
                    }
                }
                .frame(maxWidth: 190)
                Spacer()
                Text(String(format: localized("transactions.count"), model.visibleRows.count))
                    .foregroundStyle(.secondary)
            }
        }
        .padding(10)
    }

    private var transactionWorkbench: some View {
        let metrics = model.metrics
        return VStack(spacing: 10) {
            HStack(spacing: 10) {
                workbenchMetric(localized("transactions.incoming"), aggregateAmount(.incoming, fallbackSats: metrics.incomingSats))
                workbenchMetric(localized("transactions.outgoing"), aggregateAmount(.outgoing, fallbackSats: metrics.outgoingSats))
                workbenchMetric(localized("transactions.internalFlows"), metrics.internalCount.formatted(.number.locale(locale)))
                workbenchMetric(localized("transactions.reviewQueue"), metrics.reviewCount.formatted(.number.locale(locale)), actionable: metrics.reviewCount > 0)
                if metrics.missingPriceCount > 0 {
                    workbenchMetric(localized("transactions.missingPrices"), metrics.missingPriceCount.formatted(.number.locale(locale)), actionable: true)
                }
            }
            HStack {
                Text(localized("transactions.flowChart")).font(.headline)
                Spacer()
                ForEach([TransactionFlowFilter.incoming, .outgoing, .transfer, .swap], id: \.self) { flow in
                    Button {
                        model.chartSegment = model.chartSegment == flow ? nil : flow
                    } label: {
                        Label(localized("flow.\(flow.rawValue)"), systemImage: model.chartSegment == flow ? "checkmark.circle.fill" : "circle")
                    }
                    .buttonStyle(.borderless)
                }
                Picker(localized("transactions.chartMetric"), selection: $model.chartMetric) {
                    ForEach(TransactionChartMetric.allCases) { metric in
                        Text(localized("chartMetric.\(metric.rawValue)")).tag(metric)
                    }
                }.pickerStyle(.segmented).frame(width: 270)
                Picker(localized("transactions.chartMode"), selection: $model.chartMode) {
                    ForEach(TransactionChartMode.allCases) { mode in
                        Text(localized("chartMode.\(mode.rawValue)")).tag(mode)
                    }
                }.pickerStyle(.segmented).frame(width: 270)
            }
            Chart(model.chartRows) { row in
                BarMark(
                    x: .value(localized("field.date"), row.date, unit: .month),
                    y: .value(localized("transactions.chartValue"), row.value)
                )
                .foregroundStyle(by: .value(localized("filter.flow"), localized("flow.\(row.flow.rawValue)")))
            }
            .chartXSelection(value: $chartSelectionDate)
            .onChange(of: chartSelectionDate) { _, date in
                model.chartBucket = date
            }
            .chartLegend(position: .bottom, alignment: .leading)
            .frame(height: 150)
            .kassiberSensitive()
            if model.chartBucket != nil || model.chartSegment != nil {
                Button(localized("transactions.clearChartFilter")) {
                    chartSelectionDate = nil
                    model.clearChartSelection()
                }
                .buttonStyle(.link)
                .font(.caption)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    private func aggregateAmount(_ flow: TransactionFlowFilter, fallbackSats: Int64) -> String {
        let matching = model.rows.filter { $0.flow == flow }
        let converted = matching.compactMap { row -> Double? in
            row.fiatValue ?? row.rate.map { Double(abs(row.amountSats)) / 100_000_000 * $0 }
        }
        if displayCurrency == .euro, converted.count == matching.count, !matching.isEmpty {
            return KassiberFormatting.fiat(converted.reduce(0) { $0 + abs($1) }, currency: matching.first?.fiatCurrency ?? "EUR", locale: locale)
        }
        return KassiberFormatting.sats(fallbackSats, locale: locale)
    }

    private func workbenchMetric(_ label: String, _ value: String, actionable: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label)
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
            Text(value)
                .font(.body.weight(.semibold).monospacedDigit())
                .foregroundStyle(actionable ? .orange : .primary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .kassiberCardSurface(cornerRadius: 10)
    }

}

struct TransactionDetailSheet: View {
    @State private var model: TransactionDetailViewModel
    @State private var tab = "details"
    @State private var showingFilePicker = false
    @State private var urlText = ""
    @State private var attachmentToRename: AttachmentRow?
    @State private var attachmentLabel = ""
    @State private var copySourceTransaction = ""
    @State private var copyAttachmentIDs = ""
    @State private var loanTransactions = ""
    @State private var loanID = ""
    @State private var payoutAsset = "BTC"
    @State private var payoutAmount = ""
    @State private var payoutCounterparty = ""
    let completed: () -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.locale) private var locale

    init(daemon: any DaemonClient, transaction: TransactionRow, completed: @escaping () -> Void) {
        _model = State(initialValue: TransactionDetailViewModel(daemon: daemon, transaction: transaction))
        self.completed = completed
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading) {
                    Text(model.transaction.counterparty.isEmpty ? AppLocalization.code(model.transaction.type) : model.transaction.counterparty).font(.title2)
                    Text(model.transaction.transactionID ?? model.transaction.id).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                }
                Spacer()
                KassiberAmountText(transaction: model.transaction).font(.title3.monospacedDigit()).kassiberSensitive()
            }.padding(16)
            HStack(spacing: 10) {
                Picker("", selection: $tab) {
                    Text(localized("transaction.details")).tag("details")
                    Text(localized("transaction.classify")).tag("classify")
                    Text(localized("transaction.pricing")).tag("pricing")
                    Text(localized("transaction.tax")).tag("tax")
                    Text(localized("transaction.linked")).tag("linked")
                }
                .pickerStyle(.segmented)
                Menu {
                    Button(localized("transaction.attachments")) { tab = "attachments" }
                    Button(localized("transaction.history")) { tab = "history" }
                    Button(localized("transaction.graph")) { tab = "graph" }
                    Button(localized("transaction.commercial")) { tab = "commercial" }
                } label: {
                    Label(auxiliaryTabLabel, systemImage: "ellipsis.circle")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 10)
            Divider()
            Group {
                if tab == "details" { details }
                else if tab == "classify" { classify }
                else if tab == "pricing" { pricing }
                else if tab == "tax" { tax }
                else if tab == "linked" { linked }
                else if tab == "attachments" { attachments }
                else if tab == "history" { history }
                else if tab == "graph" {
                    ScrollView {
                        TransactionFlowPresentation(
                            snapshot: model.graphSnapshot,
                            privacy: model.privacyContext,
                            privacyIsLoading: model.privacyIsLoading,
                            graphIsLoading: model.graphIsLoading,
                            expanded: true,
                            onSelectRouteLeg: { leg in Task { await model.selectGraphRouteLeg(leg) } }
                        )
                        .padding(16)
                    }
                }
                else { Table(model.commercialContext) { TableColumn(localized("field.field"), value: \.key); TableColumn(localized("field.value"), value: \.value) } }
            }
            if let error = model.errorMessage { Text(AppLocalization.error(error)).foregroundStyle(.red).font(.caption).padding(8) }
            Divider()
            HStack {
                Button(localized("action.cancel")) { dismiss() }
                Spacer()
                Button(localized("action.save")) {
                    Task { await model.save(); if model.didSave { completed() } }
                }
                .buttonStyle(.borderedProminent)
                .disabled(model.isWorking || !model.hasChanges)
            }
            .padding(12)
        }
        .frame(minWidth: 760, minHeight: 620)
        .task { await model.load() }
        .fileImporter(isPresented: $showingFilePicker, allowedContentTypes: [.data, .image, .pdf, .plainText], allowsMultipleSelection: false) { result in
            if case let .success(urls) = result, let url = urls.first { Task { await model.addFile(path: url.path) } }
        }
        .sheet(item: $attachmentToRename) { attachment in
            VStack(alignment: .leading, spacing: 14) { Text(localized("transaction.renameEvidence")).font(.title2); TextField(localized("field.name"), text: $attachmentLabel); HStack { Spacer(); Button(localized("action.cancel")) { attachmentToRename = nil }; Button(localized("action.save")) { Task { await model.rename(attachment, label: attachmentLabel); attachmentToRename = nil } }.buttonStyle(.borderedProminent).disabled(attachmentLabel.isEmpty) } }.padding(24).frame(width: 430)
        }
    }

    private var auxiliaryTabLabel: String {
        switch tab {
        case "attachments": localized("transaction.attachments")
        case "history": localized("transaction.history")
        case "graph": localized("transaction.graph")
        case "commercial": localized("transaction.commercial")
        default: localized("transaction.more")
        }
    }

    private var details: some View {
        Form {
            Section(localized("transaction.identity")) {
                LabeledContent(localized("field.date"), value: model.transaction.occurredAt.map { KassiberFormatting.date($0, locale: locale) } ?? model.transaction.dateLabel)
                LabeledContent(localized("field.wallet"), value: model.transaction.wallet.isEmpty ? "—" : model.transaction.wallet)
                LabeledContent(localized("field.counterparty"), value: model.transaction.counterparty.isEmpty ? "—" : model.transaction.counterparty)
                LabeledContent(localized("field.type"), value: AppLocalization.code(model.transaction.type))
                LabeledContent(localized("field.status"), value: AppLocalization.code(model.transaction.reviewStatus))
            }
            Section(localized("transaction.settlement")) {
                LabeledContent(localized("field.amount"), value: KassiberFormatting.btc(fromSats: model.transaction.amountSats, locale: locale))
                LabeledContent(localized("transaction.fee"), value: model.transaction.feeSats == 0 ? "—" : KassiberFormatting.btc(fromSats: model.transaction.feeSats, locale: locale))
                LabeledContent(localized("field.network"), value: [model.transaction.chain, model.transaction.network].filter { !$0.isEmpty }.map(AppLocalization.code).joined(separator: " · "))
                LabeledContent(localized("transaction.confirmations"), value: String(model.transaction.confirmations))
            }
            Section(localized("transaction.identifiers")) {
                LabeledContent(localized("transaction.internalID")) { Text(model.transaction.id).monospaced().textSelection(.enabled) }
                if let externalID = model.transaction.transactionID {
                    LabeledContent(localized("transaction.externalID")) { Text(externalID).monospaced().textSelection(.enabled) }
                }
                if let reason = model.transaction.quarantineReason {
                    LabeledContent(localized("transaction.quarantine"), value: AppLocalization.code(reason))
                }
            }
            Section(localized("transactionFlow.sectionTitle")) {
                TransactionFlowPresentation(
                    snapshot: model.graphSnapshot,
                    privacy: model.privacyContext,
                    privacyIsLoading: model.privacyIsLoading,
                    graphIsLoading: model.graphIsLoading,
                    onSelectRouteLeg: { leg in Task { await model.selectGraphRouteLeg(leg) } }
                )
                .padding(.vertical, 6)
            }
        }
        .formStyle(.grouped)
    }

    private var classify: some View {
        Form {
            Picker(localized("transaction.classification"), selection: $model.classification) {
                ForEach(TransactionDetailViewModel.classificationOptions, id: \.self) { value in
                    Text(classificationLabel(value)).tag(value)
                }
            }
            Picker(localized("transaction.reviewStatus"), selection: $model.reviewStatus) {
                ForEach(TransactionDetailViewModel.reviewStatuses, id: \.self) {
                    Text(AppLocalization.code($0)).tag($0)
                }
            }
            TextField(localized("transaction.tags"), text: $model.tags)
            Section(localized("transaction.tagSuggestions")) {
                ScrollView(.horizontal) {
                    HStack {
                        ForEach(model.availableTagSuggestions.prefix(7), id: \.self) { tag in
                            Button(tag) { model.addSuggestedTag(tag) }
                                .buttonStyle(.bordered)
                                .controlSize(.small)
                        }
                    }
                }
                .scrollIndicators(.hidden)
            }
            Section(localized("transaction.note")) {
                TextEditor(text: $model.note).frame(minHeight: 120)
            }
        }
        .formStyle(.grouped)
    }

    private var pricing: some View {
        Form {
            Section(localized("transaction.pricingSource")) {
                Picker(localized("transaction.pricingSource"), selection: Binding(
                    get: { model.pricingSelection },
                    set: { model.selectPricing($0) }
                )) {
                    ForEach(TransactionDetailViewModel.pricingOptions) { option in
                        Text(pricingOptionLabel(option.id)).tag(option.id)
                    }
                }
                .pickerStyle(.radioGroup)
            }
            if model.pricingSourceKind == "manual_override" {
                Section(localized("transaction.manualPricing")) {
                    Text(localized("transaction.manualPricingHint"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    TextField(localized("transaction.currency"), text: $model.manualCurrency)
                    TextField(localized("transaction.pricePerBTC"), text: Binding(
                        get: { model.manualPrice },
                        set: { model.updateManualPrice($0) }
                    ))
                    TextField(localized("transaction.totalValue"), text: Binding(
                        get: { model.manualValue },
                        set: { model.updateManualValue($0) }
                    ))
                    TextField(localized("transaction.evidenceSource"), text: $model.manualSource)
                }
            }
            Section(localized("transaction.currentPricing")) {
                LabeledContent(localized("transaction.pricingSource"), value: pricingSourceLabel)
                LabeledContent(localized("transaction.importedPrice"), value: model.transaction.rate.map { KassiberFormatting.fiat($0, currency: model.transaction.fiatCurrency, locale: locale) } ?? "—")
                LabeledContent(localized("transaction.totalValue"), value: model.transaction.fiatValue.map { KassiberFormatting.fiat($0, currency: model.transaction.fiatCurrency, locale: locale) } ?? "—")
                LabeledContent(localized("transaction.pricingQuality"), value: pricingQualityLabel(model.pricingQuality))
                if let evidence = model.transaction.pricingExternalRef, !evidence.isEmpty {
                    LabeledContent(localized("transaction.evidenceSource"), value: evidence)
                }
                if let provider = model.transaction.pricingProvider {
                    LabeledContent(localized("transaction.provider"), value: providerLabel(provider))
                }
                if model.pricingHasCacheProvenance {
                    LabeledContent(localized("transaction.pairGranularity"), value: [model.transaction.pricingPair, model.transaction.pricingGranularity].compactMap { $0 }.joined(separator: " · "))
                }
                if let moment = model.pricingMoment {
                    LabeledContent(model.pricingMomentIsTradingDay ? localized("transaction.tradingDay") : localized("transaction.priceTimestamp"), value: KassiberFormatting.date(moment, locale: locale))
                }
                if let method = model.transaction.pricingMethod {
                    LabeledContent(localized("transaction.pricingMethod"), value: method)
                }
                if let fetchedAt = model.pricingFetchedMoment {
                    LabeledContent(localized("transaction.fetchedAt"), value: KassiberFormatting.date(fetchedAt, locale: locale))
                }
            }
            if model.pricingQuality == "coarse_fallback" || model.pricingQuality == "provider_sample" {
                Section {
                    Label(localized(model.pricingQuality == "coarse_fallback" ? "transaction.coarsePricingWarning" : "transaction.samplePricingWarning"), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.orange)
                    Button(localized("transaction.useExactManualPrice")) { model.chooseExactManualPrice() }
                }
            } else if model.pricingIsMissing {
                Section {
                    Label(localized("transaction.missingPricingWarning"), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.orange)
                    Button(localized("transaction.useExactManualPrice")) { model.chooseExactManualPrice() }
                }
            }
        }
        .formStyle(.grouped)
    }

    private var tax: some View {
        Form {
            Section(localized("transaction.plainEnglish")) {
                Text(taxNarrative).foregroundStyle(.secondary)
                if model.isBasisQuarantine && !model.excluded {
                    Label(localized("transaction.basisWarning"), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.orange)
                }
            }
            Section(localized("transaction.austrianTreatment")) {
                Picker(localized("transaction.austrianTreatment"), selection: Binding(
                    get: { model.taxSelection },
                    set: { model.selectTaxTreatment($0) }
                )) {
                    ForEach(TransactionDetailViewModel.taxOptions) { option in
                        Text(taxTreatmentLabel(option.id)).tag(option.id)
                    }
                }
                Toggle(localized("transaction.taxable"), isOn: $model.taxable)
                Toggle(localized("transaction.excluded"), isOn: $model.excluded)
            }
            Section(localized("transaction.projectedEffect")) {
                LabeledContent(taxCostBasisLabel, value: taxCostBasisValue)
                LabeledContent(taxProceedsLabel, value: taxProceedsValue)
                LabeledContent(taxGainLossLabel, value: taxGainLossValue)
                if model.pricingSourceKind == "manual_override" {
                    LabeledContent(localized("transaction.priceEvidence"), value: model.manualSource.isEmpty ? localized("transaction.sourceMissing") : model.manualSource)
                }
                if model.journalNeedsProcessing {
                    Label(localized("transaction.journalsNeedProcessing"), systemImage: "arrow.triangle.2.circlepath")
                        .foregroundStyle(.orange)
                }
            }
        }
        .formStyle(.grouped)
    }

    private var attachments: some View {
        VStack(spacing: 0) {
            HStack { Button(localized("transaction.addFile")) { showingFilePicker = true }; TextField("https://…", text: $urlText); Button(localized("transaction.addURL")) { Task { await model.addURL(urlText); urlText = "" } }.disabled(urlText.isEmpty) }.padding(10)
            HStack { TextField(localized("transaction.copySource"), text: $copySourceTransaction); TextField(localized("transaction.copyIDs"), text: $copyAttachmentIDs); Button(localized("transaction.copyEvidence")) { Task { await model.copyAttachments(from: copySourceTransaction, attachmentIDs: copyAttachmentIDs.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }); copySourceTransaction = ""; copyAttachmentIDs = "" } }.disabled(copySourceTransaction.isEmpty || copyAttachmentIDs.isEmpty) }.padding(.horizontal, 10).padding(.bottom, 10)
            Table(model.attachments) {
                TableColumn(localized("field.name"), value: \.label)
                TableColumn(localized("field.type")) { row in Text(AppLocalization.code(row.type)) }
                TableColumn(localized("field.reference"), value: \.reference)
                TableColumn("") { row in HStack { Button(localized("action.open")) { Task { if let url = await model.open(row) { NSWorkspace.shared.open(url) } } }; Button(localized("action.edit")) { attachmentLabel = row.label; attachmentToRename = row }; Button(localized("action.remove"), role: .destructive) { Task { await model.remove(row) } } } }.width(220)
            }
        }
    }

    private var history: some View {
        Table(model.history) {
            TableColumn(localized("field.date")) { row in Text(row.changedAt.map { KassiberFormatting.date($0, locale: locale) } ?? "—") }
            TableColumn(localized("activity.source")) { row in Text(AppLocalization.code(row.source)) }
            TableColumn(localized("activity.change"), value: \.summary)
            TableColumn(localized("field.details"), value: \.detail)
            TableColumn("") { row in Button(localized("activity.revert"), role: .destructive) { Task { await model.revert(row) } } }.width(90)
        }
    }

    private var linked: some View {
        Form {
            Section(localized("transaction.pairedMovement")) {
                if let pair = model.pair {
                    LabeledContent(localized("transaction.outWallet"), value: pair.outWallet ?? localized("transaction.unknown"))
                    LabeledContent(localized("transaction.outAmount"), value: assetAmount(pair.outAmountSats, asset: pair.outAsset ?? "BTC"))
                    LabeledContent(localized("transaction.inWallet"), value: pair.inWallet ?? localized("transaction.unknown"))
                    LabeledContent(localized("transaction.inAmount"), value: assetAmount(pair.inAmountSats, asset: pair.inAsset ?? "BTC"))
                    LabeledContent(localized("transaction.pairFee"), value: pair.feeSats == 0 ? "—" : KassiberFormatting.btc(fromSats: abs(pair.feeSats), locale: locale))
                    if let policy = pair.policy { LabeledContent(localized("swaps.policy"), value: AppLocalization.code(policy)) }
                    if let kind = pair.kind { LabeledContent(localized("transaction.pairKind"), value: AppLocalization.code(kind)) }
                    if model.canUnpair {
                        Button(localized("swaps.unpair"), role: .destructive) { Task { await model.unpair() } }
                            .disabled(model.isUnpairing)
                    }
                } else {
                    Text(localized("transaction.noPairedMovement")).foregroundStyle(.secondary)
                }
            }
            Section(localized("transaction.loan")) {
                if let role = model.loanRole {
                    LabeledContent(localized("transaction.loanRole"), value: loanRoleLabel(role))
                    ForEach(model.linkedLoanMarks) { mark in
                        LabeledContent(loanRoleLabel(mark.role), value: mark.description.isEmpty ? mark.transactionID : mark.description)
                    }
                    if let current = model.currentLoanMark {
                        ForEach(model.loanLinkCandidates) { candidate in
                            HStack {
                                VStack(alignment: .leading) {
                                    Text(loanRoleLabel(candidate.role))
                                    Text(candidate.description.isEmpty ? candidate.transactionID : candidate.description)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer()
                                Button(localized("transaction.linkLoan")) {
                                    Task { await model.linkLoan(transactionIDs: [current.transactionID, candidate.transactionID]) }
                                }
                            }
                        }
                    }
                    Button(localized("transaction.unmarkLoan"), role: .destructive) { Task { await model.unmarkLoan() } }
                } else {
                    Text(localized("transaction.loanHelp")).foregroundStyle(.secondary)
                    ForEach(model.loanMarkOptions, id: \.self) { role in
                        Button(loanRoleLabel(role)) { Task { await model.markLoan(as: role) } }
                    }
                }
            }
            Section(localized("transaction.linkLoan")) {
                TextField(localized("transaction.loanTransactions"), text: $loanTransactions)
                TextField(localized("transaction.loanIDOptional"), text: $loanID)
                Button(localized("transaction.linkLoan")) { Task { await model.linkLoan(transactionIDs: loanTransactions.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }, loanID: loanID) } }.disabled(loanTransactions.isEmpty)
            }
            Section(localized("transaction.journalEntries")) {
                if model.journalEvents.isEmpty {
                    Text(localized("transaction.noJournalEntries")).foregroundStyle(.secondary)
                } else {
                    ForEach(model.journalEvents) { entry in
                        LabeledContent([AppLocalization.code(entry.entryType), entry.atCategory.map(AppLocalization.code)].compactMap { $0 }.joined(separator: " · ")) {
                            VStack(alignment: .trailing) {
                                Text(assetAmount(Int64((entry.quantity * 100_000_000).rounded()), asset: entry.asset))
                                Text(KassiberFormatting.fiat(entry.fiatValue, currency: "EUR", locale: locale)).foregroundStyle(.secondary)
                            }
                        }
                        if !entry.description.isEmpty { Text(entry.description).font(.caption).foregroundStyle(.secondary) }
                    }
                }
            }
            Section(localized("transaction.directPayout")) {
                TextField(localized("field.asset"), text: $payoutAsset)
                TextField(localized("field.amount"), text: $payoutAmount)
                TextField(localized("field.counterparty"), text: $payoutCounterparty)
                Button(localized("transaction.createPayout")) { Task { await model.createDirectPayout(asset: payoutAsset, amount: payoutAmount, counterparty: payoutCounterparty) } }.disabled(payoutAsset.isEmpty || payoutAmount.isEmpty)
            }
        }.formStyle(.grouped)
    }

    private func classificationLabel(_ value: String) -> String {
        let suffix = value.lowercased().replacingOccurrences(of: " ", with: "_")
        return localized("transaction.classification.\(suffix)")
    }

    private func pricingOptionLabel(_ value: String) -> String {
        localized("transaction.pricingOption.\(value)")
    }

    private func pricingQualityLabel(_ value: String) -> String {
        localized("transaction.pricingQuality.\(value)")
    }

    private var pricingSourceLabel: String {
        guard let source = model.pricingSourceKind else { return pricingOptionLabel("missing") }
        return TransactionDetailViewModel.pricingOptions.contains(where: { $0.id == source })
            ? pricingOptionLabel(source)
            : AppLocalization.code(source)
    }

    private func taxTreatmentLabel(_ value: String) -> String {
        localized("transaction.taxTreatment.\(value.replacingOccurrences(of: ":", with: "_"))")
    }

    private func loanRoleLabel(_ value: String) -> String {
        localized("transaction.loanRole.\(value)")
    }

    private func providerLabel(_ value: String) -> String {
        switch value.lowercased() {
        case "kraken-csv": "Kraken CSV"
        case "coinbase-exchange": "Coinbase Exchange"
        case "coingecko": "CoinGecko"
        case "manual": localized("transaction.providerManual")
        default: AppLocalization.code(value)
        }
    }

    private func assetAmount(_ sats: Int64, asset: String) -> String {
        KassiberFormatting.btc(fromSats: abs(sats), locale: locale)
            .replacingOccurrences(of: "BTC", with: asset)
    }

    private var taxNarrative: String {
        let action: String
        switch model.transaction.flow {
        case .incoming: action = localized("transaction.taxActionReceived")
        case .outgoing: action = localized("transaction.taxActionSent")
        default: action = localized("transaction.taxActionMoved")
        }
        let fiat = model.transaction.fiatValue.map { KassiberFormatting.fiat($0, currency: model.transaction.fiatCurrency, locale: locale) }
            ?? localized("transaction.noFiatYet")
        let treatment = model.excluded ? localized("transaction.excludedTreatment") : taxTreatmentLabel(model.taxSelection)
        return String(
            format: localized("transaction.taxNarrativeFormat"),
            action,
            KassiberFormatting.btc(fromSats: abs(model.transaction.amountSats), locale: locale),
            model.transaction.counterparty.isEmpty ? localized("transaction.theCounterparty") : model.transaction.counterparty,
            fiat,
            treatment
        )
    }

    private var taxCostBasisLabel: String {
        switch model.taxEffect.state {
        case .acquisition: localized("transaction.basisAdded")
        case .transfer: localized("transaction.basisTreatment")
        default: localized("transaction.costBasis")
        }
    }

    private var taxProceedsLabel: String {
        model.taxEffect.state == .income ? localized("transaction.incomeRecognized") : localized("transaction.proceeds")
    }

    private var taxGainLossLabel: String {
        model.taxEffect.state == .income ? localized("transaction.taxableIncome") : localized("transaction.gainLoss")
    }

    private var taxCostBasisValue: String {
        if let value = model.taxEffect.costBasis { return KassiberFormatting.fiat(value, currency: "EUR", locale: locale) }
        return switch model.taxEffect.state {
        case .transfer: localized("transaction.basisCarriedForward")
        default: localized("transaction.journalPending")
        }
    }

    private var taxProceedsValue: String {
        if let value = model.taxEffect.proceeds { return KassiberFormatting.fiat(value, currency: "EUR", locale: locale) }
        return switch model.taxEffect.state {
        case .transfer, .acquisition: localized("transaction.noDisposal")
        default: localized("transaction.journalPending")
        }
    }

    private var taxGainLossValue: String {
        if let value = model.taxEffect.gainLoss { return KassiberFormatting.fiat(value, currency: "EUR", locale: locale) }
        return switch model.taxEffect.state {
        case .transfer: localized("transaction.noRealization")
        case .acquisition: localized("transaction.notRealized")
        default: localized("transaction.journalPending")
        }
    }
}

struct WalletsScreen: View {
    let daemon: any DaemonClient
    @State private var model: ConnectionsParityViewModel
    @State private var detail: ConnectionDetailParityViewModel
    @AppStorage("connections.selectedWalletID") private var selectedWalletID: String?
    @State private var kindFilter = "all"
    @State private var statusFilter = "all"
    @State private var catalogOpen = false
    @State private var setupMode: ConnectionSetupMode?
    @State private var backendKindOverride: String?
    @Environment(\.locale) private var locale
    @Environment(\.kassiberDisplayCurrency) private var displayCurrency
    @Environment(\.kassiberNavigate) private var navigate

    init(daemon: any DaemonClient) {
        self.daemon = daemon
        _model = State(initialValue: ConnectionsParityViewModel(daemon: daemon))
        _detail = State(initialValue: ConnectionDetailParityViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .top, spacing: 14) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(localized("nav.wallets")).font(.largeTitle.bold())
                    Text(localized("connections.selectHint"))
                        .font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
                Button {
                    catalogOpen = true
                } label: {
                    Label(localized("connections.add"), systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)
                Button {
                    Task { await model.load() }
                } label: {
                    Label(localized("action.refresh"), systemImage: "arrow.clockwise")
                }
                .disabled(model.isWorking)
            }
            .padding(.horizontal, 20)
            .padding(.top, 18)
            .padding(.bottom, 14)

            metricStrip
                .padding(.horizontal, 20)
                .padding(.bottom, 14)

            HStack(spacing: 10) {
                Picker(localized("field.type"), selection: $kindFilter) {
                    Text(localized("filter.all")).tag("all")
                    Text(localized("connections.layer.base")).tag("base")
                    Text(localized("connections.layer.lightning")).tag("lightning")
                    Text(localized("connections.layer.liquid")).tag("liquid")
                }
                .pickerStyle(.menu)
                Picker(localized("field.status"), selection: $statusFilter) {
                    Text(localized("filter.all")).tag("all")
                    Text(localized("state.synced")).tag("synced")
                    Text(localized("state.syncing")).tag("syncing")
                    Text(localized("wallet.archivedShort")).tag("archived")
                }
                .pickerStyle(.menu)
                Spacer()
                Text("\(visibleWallets.count)/\(model.wallets.count)")
                    .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 10)

            Divider()

            HSplitView {
                Table(visibleWallets, selection: $selectedWalletID) {
                    TableColumn(localized("field.name"), value: \.label)
                    TableColumn(localized("field.type")) { wallet in
                        Text(AppLocalization.code(wallet.kind))
                    }
                    TableColumn(localized("field.balance")) { wallet in
                        Text(wallet.balanceBTC.map(balanceString) ?? "—")
                            .monospacedDigit().kassiberSensitive()
                    }
                    TableColumn(localized("connections.transactions")) { wallet in
                        Text(wallet.transactionCount, format: .number)
                            .monospacedDigit()
                    }
                    TableColumn(localized("field.status")) { wallet in
                        Text(statusLabel(for: wallet))
                            .foregroundStyle(wallet.deprecated ? .orange : .secondary)
                    }
                }
                .frame(minWidth: 470, idealWidth: 600)

                if let wallet = model.wallets.first(where: { $0.id == selectedWalletID }) {
                    NativeConnectionDetailView(
                        daemon: daemon,
                        wallet: wallet,
                        priceEUR: model.priceEUR,
                        model: detail,
                        didMutate: { Task { await model.load() } }
                    )
                    .id(wallet.id)
                    .task { await detail.load(walletRef: wallet.id) }
                    .frame(minWidth: 520)
                } else if model.isWorking {
                    ProgressView(localized("state.loading"))
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ContentUnavailableView(
                        localized("wallet.select"),
                        systemImage: "wallet.bifold",
                        description: Text(localized("connections.selectHint"))
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
        }
        .navigationTitle(localized("nav.wallets"))
        .sheet(isPresented: $catalogOpen) {
            ConnectionCatalogSheet(sources: model.catalog.sources) { source in
                openCatalogSource(source)
            }
        }
        .sheet(item: $setupMode) { mode in
            ConnectionSetupSheet(
                mode: mode, model: model, daemon: daemon,
                backendKindOverride: backendKindOverride
            ) {
                setupMode = nil
                backendKindOverride = nil
                Task { await model.load() }
            }
        }
        .task {
            await model.load()
            if selectedWalletID == nil || !model.wallets.contains(where: { $0.id == selectedWalletID }) {
                selectedWalletID = model.wallets.first?.id
            }
            consumePendingAddWallet()
        }
        .onReceive(NotificationCenter.default.publisher(for: KassiberHostNotification.addWallet)) { _ in
            consumePendingAddWallet()
        }
    }

    private var visibleWallets: [WalletRow] {
        model.wallets.filter { wallet in
            let isLightning = ["lnd", "cln", "coreln", "core-lightning", "nwc"].contains(wallet.kind)
            let kindMatches = switch kindFilter {
            case "base": wallet.chain != "liquid" && !isLightning
            case "lightning": isLightning
            case "liquid": wallet.chain == "liquid"
            default: true
            }
            let statusMatches = switch statusFilter {
            case "synced": !wallet.deprecated && wallet.syncStatus.lowercased() == "synced"
            case "syncing": wallet.syncStatus.lowercased() == "syncing"
            case "archived": wallet.deprecated
            default: true
            }
            return kindMatches && statusMatches
        }
    }

    private var metricStrip: some View {
        let totalBTC = model.wallets.compactMap(\.balanceBTC).reduce(0, +)
        let transactionCount = model.wallets.reduce(0) { $0 + $1.transactionCount }
        let syncing = model.wallets.filter { $0.syncStatus.lowercased() == "syncing" }.count
        let errors = model.wallets.filter { ["error", "failed"].contains($0.syncStatus.lowercased()) }.count
        return HStack(spacing: 0) {
            walletMetric(localized("field.balance"), balanceString(totalBTC), "bitcoinsign.circle")
            Divider().frame(height: 42)
            walletMetric(localized("connections.transactions"), transactionCount.formatted(), "list.bullet.rectangle")
            Divider().frame(height: 42)
            walletMetric(localized("state.syncing"), syncing.formatted(), "arrow.triangle.2.circlepath")
            Divider().frame(height: 42)
            walletMetric(localized("state.error"), errors.formatted(), "exclamationmark.triangle")
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 14)
        .kassiberCardSurface(cornerRadius: 11)
    }

    private func walletMetric(_ label: String, _ value: String, _ symbol: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: symbol).foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 2) {
                Text(label).font(.caption).foregroundStyle(.secondary)
                Text(value).font(.headline.monospacedDigit()).kassiberSensitive()
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func balanceString(_ btc: Double) -> String {
        kassiberBitcoinString(btc: btc, rateEUR: model.priceEUR, mode: displayCurrency, locale: locale)
    }

    private func statusLabel(for wallet: WalletRow) -> String {
        if wallet.deprecated { return localized("wallet.archivedShort") }
        guard !wallet.syncStatus.isEmpty else { return localized("state.unknown") }
        return AppLocalization.code(wallet.syncStatus)
    }

    private func consumePendingAddWallet() {
        guard UserDefaults.standard.bool(forKey: KassiberHostNotification.pendingAddWalletDefaultsKey) else { return }
        UserDefaults.standard.set(false, forKey: KassiberHostNotification.pendingAddWalletDefaultsKey)
        let raw = UserDefaults.standard.string(forKey: KassiberHostNotification.pendingConnectionSetupModeDefaultsKey)
        UserDefaults.standard.removeObject(forKey: KassiberHostNotification.pendingConnectionSetupModeDefaultsKey)
        guard let raw, let mode = ConnectionSetupMode.resolve(raw) else {
            catalogOpen = true
            return
        }
        setupMode = mode
    }

    private func openCatalogSource(_ source: ConnectionCatalogSource) {
        guard source.isEnabled else { return }
        catalogOpen = false
        switch source.route {
        case .descriptor: setupMode = .wallet
        case .addressList: setupMode = .addressList
        case .silentPayment: setupMode = .silentPayment
        case .liquidDescriptor: setupMode = .liquidWallet
        case let .backend(kind):
            backendKindOverride = kind
            setupMode = kind == "bitcoinrpc" ? .bitcoinCore : .backend
        case .btcpay: setupMode = .btcpay
        case .bullBitcoinWallet: setupMode = .bullbitcoin
        case .samourai: setupMode = .samourai
        case .bip329: setupMode = .bip329
        case let .fileImport(format):
            UserDefaults.standard.set(format, forKey: KassiberHostNotification.pendingImportFormatDefaultsKey)
            navigate(.imports)
            DispatchQueue.main.async {
                NotificationCenter.default.post(name: KassiberHostNotification.openImportFormat, object: nil)
            }
        case .planned: break
        }
    }
}

struct ReportsScreen: View {
    @State private var model: ReportsViewModel
    @State private var exporter: ReportExportViewModel
    @Environment(\.locale) private var locale

    init(daemon: any DaemonClient) {
        _model = State(initialValue: ReportsViewModel(daemon: daemon))
        _exporter = State(initialValue: ReportExportViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Picker(localized("nav.reports"), selection: $model.selection) {
                    ForEach(ReportKind.allCases) { report in
                        Text(localized(report.localizationKey)).tag(report)
                    }
                }
                .pickerStyle(.segmented)
                if !model.availableYears.isEmpty {
                    Picker(localized("field.year"), selection: $model.year) {
                        Text(localized("filter.allYears")).tag(Int?.none)
                        ForEach(model.availableYears, id: \.self) { year in
                            Text(String(year)).tag(Int?.some(year))
                        }
                    }
                    .frame(width: 130)
                }
                Menu(localized("action.export")) {
                    Section(localized("reports.fullReport")) {
                        Button("CSV") { export(.uiReportsExportCsv, annual: false) }
                        Button("XLSX") { export(.uiReportsExportXlsx, annual: false) }
                        Button("PDF") { export(.uiReportsExportPdf, annual: false) }
                        Button(localized("reports.summaryPDF")) { export(.uiReportsExportSummaryPdf, annual: false) }
                    }
                    Section(localized("reports.transactions")) {
                        Button("CSV") { export(.uiTransactionsExportCsv, annual: false) }
                        Button("XLSX") { export(.uiTransactionsExportXlsx, annual: false) }
                    }
                    Section(localized("reports.austrianE1kv")) {
                        Button("PDF") { export(.uiReportsExportAustrianE1kvPdf) }
                        Button("XLSX") { export(.uiReportsExportAustrianE1kvXlsx) }
                        Button("CSV") { export(.uiReportsExportAustrianE1kvCsv) }
                    }
                    Divider()
                    Button(localized("reports.auditPackage")) { export(.uiReportsExportAuditPackage, annual: false) }
                }
                .disabled(exporter.isExporting)
            }
            .padding(12)
            Divider()
            if let error = model.errorMessage, model.rows.isEmpty {
                ScreenErrorView(message: error) { Task { await model.load() } }
            } else {
                VStack(alignment: .leading, spacing: 12) {
                    if !model.titleDetail.isEmpty {
                        Text(model.titleDetail).font(.headline).foregroundStyle(.secondary)
                    }
                    if !model.rawSummary.isEmpty {
                        HStack {
                            ForEach(Array(model.rawSummary.prefix(5).enumerated()), id: \.offset) { _, item in
                                VStack(alignment: .leading) {
                                    Text(AppLocalization.code(item.0)).font(.caption).foregroundStyle(.secondary)
                                    Text(item.1).font(.title3.monospacedDigit())
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                        .padding(12)
                        .kassiberCardSurface(cornerRadius: 11)
                    }
                    Table(model.rows) {
                        TableColumn(localized("field.details"), value: \.primary)
                        TableColumn(localized("field.type"), value: \.secondary)
                        TableColumn(localized("field.quantity")) { row in
                            Text(row.quantitySats.map { KassiberFormatting.sats($0, locale: locale) } ?? "—")
                                .monospacedDigit()
                        }
                        TableColumn(localized("field.amount")) { row in
                            Text(row.amount.map { KassiberFormatting.fiat($0, currency: row.currency ?? "EUR", locale: locale) } ?? "—")
                                .monospacedDigit()
                        }
                    }
                }
                .padding(16)
            }
            if let error = exporter.errorMessage {
                Text(AppLocalization.error(error)).foregroundStyle(.red).font(.caption).padding(8)
            }
        }
        .navigationTitle(localized("nav.reports"))
        .toolbar {
            ToolbarItem {
                Button { Task { await model.load() } } label: {
                    Label(localized("action.refresh"), systemImage: "arrow.clockwise")
                }
            }
        }
        .task { if model.rows.isEmpty { await model.load() } }
    }

    private func export(_ kind: DaemonKind, annual: Bool = true) {
        Task {
            let args = annual ? model.year.map { ["year": JSONValue.integer(Int64($0))] } : nil
            await exporter.export(kind, args: args)
            if let artifact = exporter.artifact {
                _ = try? saveExportArtifact(artifact, title: localized("export.saveReport"))
                exporter.clearArtifact()
            }
        }
    }
}
