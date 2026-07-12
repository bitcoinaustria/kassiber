import SwiftUI
import AppKit
import Charts
import UniformTypeIdentifiers
import KassiberDaemonKit
import KassiberViewModels

private func analysisLocalized(_ key: String) -> String { AppLocalization.string(key) }

private func activityFieldLabel(_ value: String) -> String {
    guard let separator = value.firstIndex(of: ":") else { return AppLocalization.code(value) }
    let field = String(value[..<separator])
    return AppLocalization.code(field) + String(value[separator...])
}

private func refreshDetailLabel(_ value: String) -> String {
    let parts = value.components(separatedBy: " · ")
    guard let phase = parts.last, parts.count > 1 else { return AppLocalization.code(value) }
    return (Array(parts.dropLast()) + [AppLocalization.code(phase)]).joined(separator: " · ")
}

struct ActivityScreen: View {
    @State private var model: ActivityViewModel
    @State private var selection: ActivityEventRow.ID?
    @Environment(\.locale) private var locale
    init(daemon: any DaemonClient) { _model = State(initialValue: ActivityViewModel(daemon: daemon)) }
    var body: some View {
        VStack(spacing: 0) {
            GroupBox(analysisLocalized("activity.filters")) {
                HStack {
                    Picker(analysisLocalized("activity.date"), selection: $model.dateDays) {
                        Text(analysisLocalized("activity.last7")).tag(7)
                        Text(analysisLocalized("activity.last30")).tag(30)
                        Text(analysisLocalized("activity.last365")).tag(365)
                        Text(analysisLocalized("activity.allTime")).tag(0)
                    }.frame(maxWidth: 180)
                    Picker(analysisLocalized("activity.source"), selection: $model.source) {
                        Text(analysisLocalized("filter.all")).tag("all")
                        Text(analysisLocalized("activity.desktop")).tag("gui")
                        Text("CLI").tag("cli")
                        Text(analysisLocalized("nav.assistant")).tag("ai_tool")
                    }.frame(maxWidth: 170)
                    Picker(analysisLocalized("activity.family"), selection: $model.family) {
                        Text(analysisLocalized("filter.all")).tag("all")
                        Text(analysisLocalized("activity.metadata")).tag("metadata")
                        Text(analysisLocalized("activity.pricing")).tag("pricing")
                        Text(analysisLocalized("activity.tax")).tag("tax")
                    }.frame(maxWidth: 170)
                    TextField(analysisLocalized("field.wallet"), text: $model.wallet)
                    TextField(
                        analysisLocalized("activity.transaction"),
                        text: Binding(get: { model.transaction }, set: { model.transaction = $0 })
                    )
                    Button(analysisLocalized("action.apply")) { Task { await model.load() } }
                }
                HStack {
                    Toggle(analysisLocalized("activity.pricingOnly"), isOn: $model.pricingOnly)
                    Toggle(analysisLocalized("activity.aiOnly"), isOn: $model.aiOnly)
                    Toggle(analysisLocalized("activity.staleOnly"), isOn: $model.staleOnly)
                    Spacer()
                    if model.staleCount > 0 {
                        Label(String(format: analysisLocalized("activity.staleCount %lld"), model.staleCount), systemImage: "exclamationmark.triangle")
                            .foregroundStyle(.orange)
                        Button(analysisLocalized("activity.process")) { Task { await model.processJournals() } }
                    }
                }
            }.padding(12)
            Divider()
            if let error = model.errorMessage, model.events.isEmpty {
                ContentUnavailableView(analysisLocalized("state.unavailable"), systemImage: "exclamationmark.triangle", description: Text(AppLocalization.error(error)))
            } else {
                Table(model.events, selection: $selection) {
                    TableColumn(analysisLocalized("field.date")) { row in Text(row.changedAt.map { KassiberFormatting.date($0, locale: locale) } ?? "—") }.width(min: 130, ideal: 160)
                    TableColumn(analysisLocalized("activity.change"), value: \.summary).width(min: 170, ideal: 250)
                    TableColumn(analysisLocalized("activity.transaction"), value: \.transactionReference).width(min: 130, ideal: 190)
                    TableColumn(analysisLocalized("field.wallet"), value: \.wallet)
                    TableColumn(analysisLocalized("activity.source")) { row in Text(AppLocalization.code(row.source)) }
                    TableColumn(analysisLocalized("field.details")) { row in
                        Text(row.fields.map(activityFieldLabel).joined(separator: "\n")).lineLimit(2)
                    }.width(min: 220, ideal: 360)
                }
                if let cursor = model.nextCursor, !cursor.isEmpty {
                    Divider()
                    HStack {
                        Spacer()
                        Button(model.isLoading ? analysisLocalized("activity.loadingMore") : analysisLocalized("activity.loadMore")) {
                            Task { await model.load(reset: false) }
                        }
                        .disabled(model.isLoading)
                        Spacer()
                    }
                    .padding(10)
                }
                if let selected = model.events.first(where: { $0.id == selection }) {
                    Divider()
                    HStack {
                        Text(selected.reason).foregroundStyle(.secondary)
                        Spacer()
                        Button(analysisLocalized("activity.revert"), role: .destructive) { Task { await model.revert(selected) } }
                    }.padding(10)
                }
            }
        }
        .navigationTitle(analysisLocalized("nav.activity"))
        .toolbar { Button { Task { await model.load() } } label: { Label(analysisLocalized("action.refresh"), systemImage: "arrow.clockwise") } }
        .task { if model.events.isEmpty { await model.load() } }
    }
}

private struct PrivacyGraphRequest: Identifiable {
    let transactionID: String
    var id: String { transactionID }
}

private struct PrivacyCensusSlice: Identifiable {
    let severity: String
    let count: Int
    var id: String { severity }
}

private struct PrivacyDisclosureSection<Content: View>: View {
    let title: String
    let symbol: String
    let count: String?
    @State private var expanded: Bool
    @ViewBuilder let content: Content

    init(
        _ title: String,
        symbol: String,
        count: String? = nil,
        expanded: Bool = false,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.symbol = symbol
        self.count = count
        _expanded = State(initialValue: expanded)
        self.content = content()
    }

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            content.padding(.top, 12)
        } label: {
            HStack(spacing: 8) {
                Label(title, systemImage: symbol).font(.headline)
                Spacer()
                if let count { Text(count).font(.caption.monospacedDigit()).foregroundStyle(.secondary) }
            }
        }
        .padding(14)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(.separator.opacity(0.45)))
    }
}

struct PrivacyMirrorScreen: View {
    @State private var model: PrivacyMirrorViewModel
    @State private var recordsTab = "wallets"
    @State private var graphRequest: PrivacyGraphRequest?
    @Environment(\.kassiberNavigate) private var navigate
    @Environment(\.locale) private var locale

    init(daemon: any DaemonClient) {
        _model = State(initialValue: PrivacyMirrorViewModel(daemon: daemon))
    }

    var body: some View {
        Group {
            if model.isLoading && model.wallets.isEmpty && model.findings.isEmpty {
                ProgressView(analysisLocalized("privacy.loading"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let error = model.errorMessage, model.wallets.isEmpty && model.findings.isEmpty {
                ContentUnavailableView(
                    analysisLocalized("privacy.unavailable"),
                    systemImage: "exclamationmark.shield",
                    description: Text(AppLocalization.error(error))
                )
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 14) {
                        header
                        scoreOverview
                        worstRisk
                        findings
                        psbt
                        linkage
                        adversaries
                        evidence
                        records
                        heuristics
                    }
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .navigationTitle(analysisLocalized("nav.privacyMirror"))
        .sheet(item: $graphRequest) { request in
            NavigationStack {
                Table(model.transactionGraph) {
                    TableColumn(analysisLocalized("field.field"), value: \.key)
                    TableColumn(analysisLocalized("field.value"), value: \.value)
                    TableColumn("") { row in
                        Button { NativeAffordances.copy(row.value) } label: {
                            Image(systemName: "doc.on.doc")
                        }
                        .buttonStyle(.borderless)
                        .help(analysisLocalized("action.copy"))
                    }
                    .width(35)
                }
                .navigationTitle(analysisLocalized("privacy.flow.title"))
                .safeAreaInset(edge: .bottom) {
                    Text(request.transactionID)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .padding(10)
                        .frame(maxWidth: .infinity)
                        .background(.bar)
                }
            }
            .frame(minWidth: 680, minHeight: 460)
        }
        .task { await model.load() }
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                Text(analysisLocalized("nav.privacyMirror")).font(.title2.weight(.semibold))
                Text(analysisLocalized("privacy.subtitle")).font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
            evidenceBadge(model.evidenceLevel)
            if model.localOnly == false || model.advisoryOnly == false || model.coverageSummary.degraded {
                Label(analysisLocalized("privacy.guardrail.degraded"), systemImage: "exclamationmark.triangle.fill")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.orange)
                    .padding(.horizontal, 8).padding(.vertical, 4)
                    .background(.orange.opacity(0.1), in: Capsule())
            }
            Button { Task { await model.load() } } label: {
                Label(analysisLocalized("action.refresh"), systemImage: "arrow.clockwise")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .disabled(model.isLoading)
            .help(analysisLocalized("action.refresh"))
        }
    }

    private var scoreOverview: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .top, spacing: 12) {
                scoreHero
                    .frame(minWidth: 360, maxWidth: .infinity, maxHeight: .infinity)
                VStack(spacing: 12) {
                    severityCensusCard
                    scoreWaterfallCard
                }
                .frame(minWidth: 300, idealWidth: 380, maxWidth: 440)
            }
            VStack(spacing: 12) {
                scoreHero
                severityCensusCard
                scoreWaterfallCard
            }
        }
    }

    private var scoreHero: some View {
        GroupBox {
            VStack(spacing: 12) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text(model.grade)
                        .font(.system(size: 48, weight: .bold, design: .rounded))
                        .foregroundStyle(gradeColor)
                    Text(model.score, format: .number)
                        .font(.system(size: 32, weight: .semibold, design: .monospaced))
                    Text(analysisLocalized("privacy.score.of"))
                        .font(.callout).foregroundStyle(.secondary)
                }
                Text(privacyLocalized("gradeHint", code: model.grade, fallback: ""))
                    .font(.callout.weight(.medium)).foregroundStyle(gradeColor)
                ProgressView(value: Double(model.score), total: 100).tint(gradeColor)
                Text(analysisLocalized("privacy.score.local"))
                    .font(.caption).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, minHeight: 190, maxHeight: .infinity)
        } label: {
            Label(analysisLocalized("privacy.score"), systemImage: "hand.raised.square")
        }
    }

    private var severityCensusCard: some View {
        GroupBox {
            VStack(spacing: 8) {
                if model.severityCensus.total == 0 {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 64)).foregroundStyle(.green)
                } else {
                    Chart(censusSlices) { slice in
                        SectorMark(
                            angle: .value(analysisLocalized("privacy.score.findings"), slice.count),
                            innerRadius: .ratio(0.62),
                            angularInset: 1.5
                        )
                        .foregroundStyle(severityColor(slice.severity))
                    }
                    .chartLegend(.hidden)
                    .frame(height: 120)
                }
                Text(String(format: analysisLocalized("privacy.score.findingCount %lld"), Int64(model.severityCensus.total)))
                    .font(.headline.monospacedDigit())
                HStack(spacing: 12) {
                    censusLegend("alert", model.severityCensus.alert)
                    censusLegend("warning", model.severityCensus.warning)
                    censusLegend("info", model.severityCensus.info)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 190)
        } label: {
            Label(analysisLocalized("privacy.score.census"), systemImage: "chart.pie")
        }
    }

    private var scoreWaterfallCard: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                if !model.scoreFactors.isEmpty {
                    Chart(model.scoreFactors) { factor in
                        BarMark(
                            x: .value(analysisLocalized("privacy.score.points"), factor.points),
                            y: .value(analysisLocalized("privacy.score.factor"), factorLabel(factor.key))
                        )
                        .foregroundStyle(factor.points < 0 ? .orange : .green)
                        .annotation(position: factor.points < 0 ? .leading : .trailing) {
                            Text(factor.points, format: .number.sign(strategy: .always()))
                                .font(.caption2.monospacedDigit())
                        }
                    }
                    .chartXAxis(.hidden)
                    .frame(height: max(70, CGFloat(model.scoreFactors.count) * 36))
                }
                waterfallRow(analysisLocalized("privacy.score.base"), value: model.scoreBase)
                ForEach(model.scoreFactors) { factor in
                    HStack {
                        Text(factorLabel(factor.key))
                        Text(factorDetail(factor)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                        Spacer()
                        Text(factor.points, format: .number.sign(strategy: .always()))
                            .monospacedDigit().foregroundStyle(factor.points < 0 ? .orange : .green)
                    }
                    .font(.callout)
                }
                Divider()
                waterfallRow(analysisLocalized("privacy.score.result"), value: model.score, bold: true)
                HStack {
                    Text(analysisLocalized("privacy.score.coverage"))
                    Spacer()
                    Text(model.coverage, format: .percent.precision(.fractionLength(0))).monospacedDigit()
                }
                .font(.caption).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, minHeight: 190, alignment: .topLeading)
        } label: {
            Label(analysisLocalized("privacy.score.waterfall"), systemImage: "chart.bar.xaxis")
        }
    }

    private var worstRisk: some View {
        GroupBox {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "exclamationmark.shield.fill")
                    .font(.title2).foregroundStyle(severityColor(model.worstRiskModel.severity))
                VStack(alignment: .leading, spacing: 5) {
                    Text(worstRiskTitle).font(.headline)
                    Text(worstRiskDescription)
                        .font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
                }
                Spacer()
                severityBadge(model.worstRiskModel.severity)
                evidenceBadge(model.worstRiskModel.evidence)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } label: {
            Label(analysisLocalized("privacy.primary.title"), systemImage: "wrench.and.screwdriver")
        }
    }

    private var findings: some View {
        PrivacyDisclosureSection(
            analysisLocalized("privacy.question.findings"),
            symbol: "list.bullet.clipboard",
            count: String(model.findings.count),
            expanded: true
        ) {
            if model.findings.isEmpty {
                ContentUnavailableView(
                    analysisLocalized("privacy.finding.empty"),
                    systemImage: "checkmark.shield"
                )
                .frame(minHeight: 130)
            } else {
                VStack(spacing: 8) {
                    ForEach(model.findings) { row in findingRow(row) }
                }
            }
        }
    }

    private var psbt: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                Text(analysisLocalized("privacy.psbtPrompt")).font(.callout).foregroundStyle(.secondary)
                TextEditor(text: $model.psbt)
                    .font(.body.monospaced())
                    .frame(minHeight: 110)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
                HStack {
                    Button(analysisLocalized("privacy.analyze")) { Task { await model.analyzePSBT() } }
                        .buttonStyle(.borderedProminent)
                        .disabled(model.psbt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    Spacer()
                    Label(analysisLocalized("privacy.psbt.local"), systemImage: "lock.fill")
                        .font(.caption).foregroundStyle(.secondary)
                }
                if !model.psbtResult.isEmpty {
                    Table(model.psbtResult) {
                        TableColumn(analysisLocalized("field.field"), value: \.key)
                        TableColumn(analysisLocalized("field.value"), value: \.value)
                    }
                    .frame(height: 220)
                }
            }
        } label: {
            Label(analysisLocalized("privacy.section.psbt"), systemImage: "shield.lefthalf.filled")
        }
    }

    private var linkage: some View {
        PrivacyDisclosureSection(
            analysisLocalized("privacy.linkage.title"),
            symbol: "point.3.connected.trianglepath.dotted",
            count: String(model.wallets.count),
            expanded: true
        ) {
            if model.wallets.isEmpty {
                Text(analysisLocalized("privacy.linkage.empty")).foregroundStyle(.secondary)
            } else {
                PrivacyLinkageCanvas(wallets: Array(model.wallets.prefix(8)))
                    .frame(height: max(150, CGFloat(min(model.wallets.count, 8)) * 46))
                HStack(spacing: 14) {
                    Label(analysisLocalized("privacy.linkage.linkable"), systemImage: "circle.fill").foregroundStyle(.orange)
                    Label(analysisLocalized("privacy.linkage.isolated"), systemImage: "circle.fill").foregroundStyle(.green)
                    Spacer()
                    evidenceBadge(model.evidenceLevel)
                }
                .font(.caption)
            }
        }
    }

    private var adversaries: some View {
        PrivacyDisclosureSection(
            analysisLocalized("privacy.question.infer"),
            symbol: "eye.trianglebadge.exclamationmark",
            count: String(model.adversaryCards.count)
        ) {
            if model.adversaryCards.isEmpty {
                Text(analysisLocalized("privacy.adversary.empty")).foregroundStyle(.secondary)
            } else {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 260), spacing: 10)], spacing: 10) {
                    ForEach(model.adversaryCards) { card in adversaryCard(card) }
                }
            }
        }
    }

    private var evidence: some View {
        PrivacyDisclosureSection(
            analysisLocalized("privacy.question.evidence"),
            symbol: "doc.text.magnifyingglass",
            count: String(model.evidenceDrilldowns.count + model.timeline.count)
        ) {
            VStack(alignment: .leading, spacing: 16) {
                GroupBox(analysisLocalized("privacy.section.evidenceItems")) {
                    if model.evidenceDrilldowns.isEmpty {
                        Text(analysisLocalized("privacy.table.empty")).foregroundStyle(.secondary)
                    } else {
                        VStack(spacing: 8) {
                            ForEach(model.evidenceDrilldowns) { row in
                                DisclosureGroup {
                                    if row.facts.isEmpty {
                                        Text(analysisLocalized("privacy.evidence.noFacts"))
                                            .font(.caption).foregroundStyle(.secondary)
                                    } else {
                                        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 5) {
                                            ForEach(row.facts) { fact in
                                                GridRow {
                                                    Text(AppLocalization.code(fact.key)).foregroundStyle(.secondary)
                                                    Text(fact.value).textSelection(.enabled)
                                                }
                                            }
                                        }
                                        .font(.caption)
                                    }
                                } label: {
                                    HStack {
                                        Text(AppLocalization.code(row.section))
                                        Text(shortID(row.id)).font(.caption.monospaced()).foregroundStyle(.secondary)
                                        Spacer()
                                        Text(tellLabel(row.kind)).font(.caption)
                                        evidenceBadge(row.evidenceLevel)
                                    }
                                }
                                .padding(8).background(.background, in: RoundedRectangle(cornerRadius: 7))
                            }
                        }
                    }
                }

                GroupBox(analysisLocalized("privacy.section.timeline")) {
                    Table(model.timeline) {
                        TableColumn(analysisLocalized("privacy.table.event")) { row in Text(tellLabel(row.kind)) }
                        TableColumn(analysisLocalized("privacy.table.category")) { row in Text(AppLocalization.code(row.category)) }
                        TableColumn(analysisLocalized("privacy.table.transaction")) { row in Text(shortID(row.transactionID)).font(.body.monospaced()) }
                        TableColumn(analysisLocalized("privacy.table.detail")) { row in
                            Text(row.newLinkage ? analysisLocalized("privacy.timeline.newLinkage") : tellLabel(row.detail))
                        }
                        TableColumn(analysisLocalized("privacy.evidence")) { row in evidenceBadge(row.evidence) }
                    }
                    .frame(height: max(150, min(300, CGFloat(model.timeline.count + 1) * 36)))
                }

                coverageCard
            }
        }
    }

    private var records: some View {
        PrivacyDisclosureSection(
            analysisLocalized("privacy.section.records"),
            symbol: "tablecells",
            count: String(model.wallets.count + model.transactions.count + model.utxos.count)
        ) {
            Picker(analysisLocalized("privacy.section.records"), selection: $recordsTab) {
                Text(analysisLocalized("nav.wallets")).tag("wallets")
                Text(analysisLocalized("nav.transactions")).tag("transactions")
                Text(analysisLocalized("privacy.tab.utxos")).tag("utxos")
            }
            .pickerStyle(.segmented)
            if recordsTab == "transactions" { transactionTable }
            else { privacyTable(recordsTab == "wallets" ? model.wallets : model.utxos) }
        }
    }

    private var heuristics: some View {
        PrivacyDisclosureSection(
            analysisLocalized("privacy.heuristics.title"),
            symbol: "checklist.checked",
            count: String(format: analysisLocalized("privacy.heuristics.count %lld %lld"), Int64(14), Int64(PrivacyMirrorViewModel.heuristics.count))
        ) {
            Text(analysisLocalized("privacy.heuristics.note"))
                .font(.caption).foregroundStyle(.secondary)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 210), spacing: 7)], spacing: 7) {
                ForEach(PrivacyMirrorViewModel.heuristics) { heuristic in
                    HStack(spacing: 7) {
                        Circle().fill(heuristicColor(heuristic.status)).frame(width: 8, height: 8)
                        Text(privacyLocalized("heuristic", code: heuristic.id, fallback: heuristic.name)).lineLimit(1)
                        Spacer(minLength: 4)
                        Text(privacyLocalized("heuristics.status", code: heuristic.status, fallback: AppLocalization.code(heuristic.status)))
                            .font(.caption2).foregroundStyle(.secondary)
                    }
                    .font(.caption)
                    .padding(7)
                    .background(.background, in: RoundedRectangle(cornerRadius: 6))
                }
            }
        }
    }

    private var coverageCard: some View {
        GroupBox(analysisLocalized("privacy.section.coverage")) {
            VStack(spacing: 7) {
                metricRow(analysisLocalized("privacy.coverage.known"), model.coverageSummary.knownCoinCount)
                metricRow(analysisLocalized("privacy.coverage.unknown"), model.coverageSummary.unknownCoinCount)
                metricRow(analysisLocalized("privacy.coverage.gaps"), model.coverageSummary.unknownCoverageCount)
                HStack {
                    Text(analysisLocalized("privacy.coverage.degraded"))
                    Spacer()
                    Text(model.coverageSummary.degraded ? analysisLocalized("privacy.yes") : analysisLocalized("privacy.bounded"))
                }
                if model.coverageSummary.degraded {
                    Button { navigate(.sourceFunds) } label: {
                        Label(analysisLocalized("privacy.action.reviewOrigins"), systemImage: "checkmark.seal")
                    }
                    .frame(maxWidth: .infinity, alignment: .trailing)
                }
            }
        }
    }

    @ViewBuilder
    private func findingRow(_ row: PrivacyFindingRow) -> some View {
        DisclosureGroup {
            VStack(alignment: .leading, spacing: 9) {
                Text(recommendation(row.kind)).font(.callout).foregroundStyle(.secondary)
                if !row.detail.isEmpty && row.detail != row.transactionID {
                    Text(row.detail).font(.caption).foregroundStyle(.secondary)
                }
                HStack {
                    if let transactionID = row.transactionID {
                        Button { openGraph(transactionID) } label: {
                            Label(analysisLocalized("privacy.flow.view"), systemImage: "point.3.connected.trianglepath.dotted")
                        }
                    }
                    if row.routesToSourceFunds {
                        Button { navigate(.sourceFunds) } label: {
                            Label(analysisLocalized("privacy.action.reviewOrigins"), systemImage: "checkmark.seal")
                        }
                    }
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
            }
            .padding(.top, 7)
        } label: {
            HStack(spacing: 9) {
                Circle().fill(severityColor(row.severity)).frame(width: 9, height: 9)
                VStack(alignment: .leading, spacing: 2) {
                    Text(tellLabel(row.kind)).font(.callout.weight(.medium))
                    Text(AppLocalization.code(row.severity)).font(.caption2.monospaced()).foregroundStyle(severityColor(row.severity))
                }
                Spacer()
                evidenceBadge(row.evidence)
            }
        }
        .padding(10)
        .background(.background, in: RoundedRectangle(cornerRadius: 8))
        .overlay(alignment: .leading) {
            Rectangle().fill(severityColor(row.severity)).frame(width: 3).clipShape(.rect(cornerRadius: 2))
        }
    }

    @ViewBuilder
    private func adversaryCard(_ card: PrivacyAdversaryCard) -> some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 9) {
                HStack {
                    metric(analysisLocalized("privacy.adversary.clusters"), card.exposedClusterCount)
                    Divider()
                    metric(analysisLocalized("privacy.adversary.wallets"), card.walletCount)
                }
                if !card.unknownCoverageStatus.isEmpty {
                    Label(AppLocalization.code(card.unknownCoverageStatus), systemImage: "questionmark.circle")
                        .font(.caption).foregroundStyle(.secondary)
                }
                ForEach(card.assumptions.prefix(3)) { assumption in
                    HStack(alignment: .top) {
                        Text(privacyLocalized("assumption", code: assumption.code, fallback: assumption.statement))
                            .font(.caption).fixedSize(horizontal: false, vertical: true)
                        Spacer()
                        evidenceBadge(assumption.evidence)
                    }
                }
            }
        } label: {
            HStack {
                Text(privacyLocalized("adversaryTier", code: card.tier, fallback: card.label.isEmpty ? AppLocalization.code(card.tier) : card.label))
                Spacer()
                evidenceBadge(card.evidence)
            }
        }
    }

    private func privacyTable(_ rows: [PrivacyTableRow]) -> some View {
        Table(rows) {
            TableColumn(analysisLocalized("field.reference")) { row in Text(shortID(row.primary)).font(.body.monospaced()) }
            TableColumn(analysisLocalized("field.wallet")) { row in Text(shortID(row.walletID ?? "—")) }
            TableColumn(analysisLocalized("field.amount")) { row in Text(amount(row.amountMSat)) }
            TableColumn(analysisLocalized("field.details")) { row in Text(privacyDetail(row)) }
            TableColumn(analysisLocalized("privacy.evidence")) { row in evidenceBadge(row.evidence) }
        }
        .frame(height: 280)
    }

    private var transactionTable: some View {
        Table(model.transactions) {
            TableColumn(analysisLocalized("field.reference")) { row in Text(shortID(row.primary)).font(.body.monospaced()) }
            TableColumn(analysisLocalized("field.details")) { row in Text(privacyDetail(row)) }
            TableColumn(analysisLocalized("privacy.table.penalties")) { row in Text(row.walletPenaltyCount, format: .number) }
            TableColumn(analysisLocalized("privacy.evidence")) { row in evidenceBadge(row.evidence) }
            TableColumn("") { row in
                Button { openGraph(row.primary) } label: { Label(analysisLocalized("privacy.flow.view"), systemImage: "point.3.connected.trianglepath.dotted") }
                    .buttonStyle(.borderless)
            }
            .width(min: 90, ideal: 110)
        }
        .frame(height: 280)
    }

    private func privacyDetail(_ row: PrivacyTableRow) -> String {
        switch row.detail {
        case let .wallet(coinCount, linkCount):
            return String(format: analysisLocalized("privacy.walletDetail %lld %lld"), Int64(coinCount), Int64(linkCount))
        case let .transaction(tellCount, tellKinds):
            let kinds = tellKinds.split(separator: ",")
                .map { tellLabel(String($0).trimmingCharacters(in: .whitespaces)) }
                .joined(separator: ", ")
            return String(format: analysisLocalized("privacy.transactionDetail %lld %@"), Int64(tellCount), kinds)
        case let .utxo(branchRole, sourceProximity):
            return "\(AppLocalization.code(branchRole)) · \(tellLabel(sourceProximity))"
        }
    }

    private var censusSlices: [PrivacyCensusSlice] {
        [
            .init(severity: "alert", count: model.severityCensus.alert),
            .init(severity: "warning", count: model.severityCensus.warning),
            .init(severity: "info", count: model.severityCensus.info),
        ].filter { $0.count > 0 }
    }

    private var gradeColor: Color {
        switch model.grade {
        case "A+": .green
        case "B": .mint
        case "C": .orange
        case "D": .orange
        default: .red
        }
    }

    private var worstRiskTitle: String {
        let modelTitle = model.worstRiskModel.title.isEmpty ? analysisLocalized("privacy.worst.fallback") : model.worstRiskModel.title
        return privacyLocalized("worstKind", code: model.worstRiskModel.kind, fallback: modelTitle)
    }

    private var worstRiskDescription: String {
        guard !model.worstRiskModel.kind.isEmpty else {
            return model.worstRisk.isEmpty ? analysisLocalized("privacy.noFindings") : model.worstRisk
        }
        let key = "privacy.reco.\(model.worstRiskModel.kind)"
        let value = analysisLocalized(key)
        if value != key { return value }
        return model.worstRisk.isEmpty ? analysisLocalized("privacy.worst.empty") : model.worstRisk
    }

    private func openGraph(_ transactionID: String) {
        graphRequest = PrivacyGraphRequest(transactionID: transactionID)
        Task { await model.loadGraph(transaction: transactionID) }
    }

    private func amount(_ msat: Int64?) -> String {
        guard let msat else { return "—" }
        return KassiberFormatting.sats(msat / 1000, locale: locale)
    }

    private func shortID(_ value: String) -> String {
        guard value.count > 24 else { return value.isEmpty ? "—" : value }
        return "\(value.prefix(12))…\(value.suffix(8))"
    }

    private func tellLabel(_ kind: String) -> String {
        privacyLocalized("tellKind", code: kind, fallback: AppLocalization.code(kind))
    }

    private func recommendation(_ kind: String) -> String {
        privacyLocalized("reco", code: kind, fallback: analysisLocalized("privacy.reco.fallback"))
    }

    private func factorLabel(_ key: String) -> String {
        privacyLocalized("score.factor", code: key, fallback: AppLocalization.code(key))
    }

    private func privacyLocalized(_ namespace: String, code: String, fallback: String) -> String {
        guard !code.isEmpty else { return fallback }
        let key = "privacy.\(namespace).\(code)"
        let value = analysisLocalized(key)
        return value == key ? fallback : value
    }

    private func factorDetail(_ factor: PrivacyScoreFactorRow) -> String {
        if factor.key == "wallet_linkage", let linked = factor.linked { return "\(linked)/\(factor.total ?? 0)" }
        if factor.key == "transaction_leaks", let leaking = factor.leaking { return "\(leaking)/\(factor.total ?? 0)" }
        return factor.total.map(String.init) ?? ""
    }

    private func severityColor(_ severity: String) -> Color {
        switch severity {
        case "alert", "critical": .red
        case "warning": .orange
        default: .blue
        }
    }

    private func heuristicColor(_ status: String) -> Color {
        switch status {
        case "computed": .green
        case "partial": .orange
        default: .secondary
        }
    }

    @ViewBuilder
    private func evidenceBadge(_ value: String) -> some View {
        Text(AppLocalization.code(value))
            .font(.caption2.weight(.medium))
            .padding(.horizontal, 7).padding(.vertical, 3)
            .background((value == "exact" ? Color.primary : value == "derived" ? .blue : .orange).opacity(0.1), in: Capsule())
            .foregroundStyle(value == "exact" ? Color.primary : value == "derived" ? .blue : .orange)
    }

    @ViewBuilder
    private func severityBadge(_ value: String) -> some View {
        Text(AppLocalization.code(value).uppercased())
            .font(.caption2.monospaced().weight(.semibold))
            .padding(.horizontal, 7).padding(.vertical, 3)
            .background(severityColor(value).opacity(0.1), in: Capsule())
            .foregroundStyle(severityColor(value))
    }

    @ViewBuilder
    private func censusLegend(_ severity: String, _ count: Int) -> some View {
        HStack(spacing: 4) {
            Circle().fill(severityColor(severity)).frame(width: 7, height: 7)
            Text(count, format: .number).monospacedDigit()
            Text(AppLocalization.code(severity))
        }
        .font(.caption2)
    }

    @ViewBuilder
    private func waterfallRow(_ label: String, value: Int, bold: Bool = false) -> some View {
        HStack {
            Text(label)
            Spacer()
            Text(value, format: .number).monospacedDigit()
        }
        .font(bold ? .callout.weight(.semibold) : .callout)
    }

    @ViewBuilder
    private func metric(_ label: String, _ value: Int) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption2).foregroundStyle(.secondary)
            Text(value, format: .number).font(.title3.monospacedDigit())
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func metricRow(_ label: String, _ value: Int) -> some View {
        HStack { Text(label); Spacer(); Text(value, format: .number).monospacedDigit() }
    }
}

private struct PrivacyLinkageCanvas: View {
    let wallets: [PrivacyTableRow]

    var body: some View {
        Canvas { context, size in
            let rowHeight = size.height / CGFloat(max(wallets.count, 1))
            let leftX = min(size.width * 0.38, 190)
            let observerX = max(leftX + 100, size.width - 115)
            let observerY = size.height / 2
            let maxEdges = max(1, wallets.map(linkCount).max() ?? 1)

            for (index, wallet) in wallets.enumerated() {
                let y = rowHeight * (CGFloat(index) + 0.5)
                let edges = linkCount(wallet)
                if edges > 0 {
                    var path = Path()
                    path.move(to: CGPoint(x: leftX, y: y))
                    path.addLine(to: CGPoint(x: observerX, y: observerY))
                    context.stroke(
                        path,
                        with: .color(.orange.opacity(0.55)),
                        lineWidth: 1.5 + CGFloat(edges) / CGFloat(maxEdges) * 4
                    )
                }
                let nodeColor: Color = edges > 0 ? .orange : .green
                let nodeRect = CGRect(x: 8, y: y - 15, width: leftX - 16, height: 30)
                context.fill(Path(roundedRect: nodeRect, cornerRadius: 6), with: .color(nodeColor.opacity(0.12)))
                context.stroke(Path(roundedRect: nodeRect, cornerRadius: 6), with: .color(nodeColor.opacity(0.8)))
                let label = context.resolve(Text(short(wallet.primary)).font(.caption.monospaced()))
                context.draw(label, at: CGPoint(x: nodeRect.minX + 7, y: y), anchor: .leading)
            }

            if wallets.contains(where: { linkCount($0) > 0 }) {
                context.fill(
                    Path(ellipseIn: CGRect(x: observerX - 24, y: observerY - 24, width: 48, height: 48)),
                    with: .color(.orange.opacity(0.14))
                )
                context.stroke(
                    Path(ellipseIn: CGRect(x: observerX - 24, y: observerY - 24, width: 48, height: 48)),
                    with: .color(.orange)
                )
                let observer = context.resolve(
                    Text(analysisLocalized("privacy.linkage.observer"))
                        .font(.caption2).foregroundStyle(.secondary)
                )
                context.draw(observer, at: CGPoint(x: observerX, y: observerY + 36), anchor: .center)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(analysisLocalized("privacy.linkage.title"))
        .accessibilityValue(
            String(format: analysisLocalized("privacy.linkage.accessibility %lld %lld"), Int64(wallets.count), Int64(wallets.filter { linkCount($0) > 0 }.count))
        )
    }

    private func linkCount(_ wallet: PrivacyTableRow) -> Int {
        guard case let .wallet(_, count) = wallet.detail else { return 0 }
        return count
    }

    private func short(_ value: String) -> String {
        guard value.count > 20 else { return value }
        return "\(value.prefix(10))…\(value.suffix(6))"
    }
}

struct EgressScreen: View {
    @State private var model: EgressViewModel
    @Environment(\.locale) private var locale
    init(daemon: any DaemonClient) { _model = State(initialValue: EgressViewModel(daemon: daemon)) }
    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 20) {
                metric(analysisLocalized("egress.unexpected"), model.unexpected, warning: model.unexpected > 0)
                metric(analysisLocalized("egress.updates"), model.updateRequests)
                VStack(alignment: .leading) { Text(analysisLocalized("egress.database")).font(.caption).foregroundStyle(.secondary); Text(AppLocalization.code(model.databaseClassification)); Text(model.databasePrefix).font(.caption.monospaced()).foregroundStyle(.secondary) }
                Spacer()
                Toggle(analysisLocalized("egress.actionableOnly"), isOn: $model.actionableOnly)
            }.padding(14)
            Divider()
            Table(model.visibleRecords) {
                TableColumn(analysisLocalized("field.date")) { row in Text(row.date.map { KassiberFormatting.date($0, locale: locale) } ?? "—") }
                TableColumn(analysisLocalized("egress.subsystem"), value: \.subsystem)
                TableColumn(analysisLocalized("egress.endpoint"), value: \.endpoint)
                TableColumn(analysisLocalized("egress.operation"), value: \.operation)
                TableColumn(analysisLocalized("egress.bytes")) { row in Text(row.bytesOut, format: .byteCount(style: .file)) }
                TableColumn(analysisLocalized("field.status")) { row in
                    Text(AppLocalization.code(row.status))
                }
            }
        }.navigationTitle(analysisLocalized("nav.egress"))
        .toolbar { Button { Task { await model.load() } } label: { Label(analysisLocalized("action.refresh"), systemImage: "arrow.clockwise") } }
        .task { await model.load() }
    }
    private func metric(_ label: String, _ value: Int, warning: Bool = false) -> some View {
        VStack(alignment: .leading) { Text(label).font(.caption).foregroundStyle(.secondary); Text(value, format: .number).font(.title2.monospacedDigit()).foregroundStyle(warning ? .red : .primary) }
    }
}

struct BirdsEyeScreen: View {
    let daemon: any DaemonClient
    @State private var model: BirdsEyeViewModel
    @AppStorage("birdsEye.workspaceID") private var requestedWorkspaceID = ""
    @State private var selectedTransaction: TransactionRow?
    @Environment(\.kassiberNavigate) private var navigate
    init(daemon: any DaemonClient) {
        self.daemon = daemon
        _model = State(initialValue: BirdsEyeViewModel(daemon: daemon))
    }
    var body: some View {
        VStack(spacing: 0) {
            HStack { VStack(alignment: .leading) { Text(analysisLocalized("birdsEye.bookSet")).font(.caption).foregroundStyle(.secondary); Text(model.workspaceLabel).font(.title2) }; Spacer(); Text(String(format: analysisLocalized("birdsEye.books %lld"), model.profiles.count)).foregroundStyle(.secondary) }.padding(14)
            if model.isRefreshing { HStack { ProgressView(value: model.refreshProgress); Text(refreshDetailLabel(model.refreshDetail)).font(.caption).foregroundStyle(.secondary) }.padding(.horizontal, 14).padding(.bottom, 10) }
            if !model.chartPoints.isEmpty {
                DashboardActivityChart(
                    points: model.chartPoints,
                    transactions: model.chartTransactions,
                    fiatCurrency: model.chartFiatCurrency,
                    marketRate: model.chartMarketRate,
                    onOpenTransaction: { selectedTransaction = $0 }
                )
                .padding(.horizontal, 14)
                .padding(.bottom, 10)
            }
            Divider()
            Table(model.profiles) {
                TableColumn(analysisLocalized("books.book"), value: \.label)
                TableColumn(analysisLocalized("books.currency"), value: \.currency)
                TableColumn(analysisLocalized("nav.transactions")) { row in Text(row.transactionCount, format: .number) }
                TableColumn(analysisLocalized("nav.wallets")) { row in Text(row.walletCount, format: .number) }
                TableColumn(analysisLocalized("nav.quarantine")) { row in Text(row.quarantines, format: .number) }
                TableColumn(analysisLocalized("field.status")) { row in if !row.ready { Label(analysisLocalized("birdsEye.needsAttention"), systemImage: "exclamationmark.triangle").foregroundStyle(.orange) } }
                TableColumn("") { row in Button(analysisLocalized("books.open")) { Task { await model.switchBook(row.id); if model.errorMessage == nil { navigate(.dashboard) } } } }.width(80)
            }
        }
        .navigationTitle(analysisLocalized("nav.birdsEye"))
        .toolbar { Button { Task { await model.refreshWorkspace() } } label: { Label(analysisLocalized("birdsEye.refreshAll"), systemImage: "arrow.triangle.2.circlepath") }.disabled(model.isRefreshing); Button { Task { await model.load() } } label: { Image(systemName: "arrow.clockwise") } }
        .task {
            await model.load(workspaceID: requestedWorkspaceID.isEmpty ? nil : requestedWorkspaceID)
        }
        .sheet(item: $selectedTransaction) { row in
            TransactionDetailSheet(daemon: daemon, transaction: row) {
                selectedTransaction = nil
                Task { await model.load(workspaceID: model.workspaceID) }
            }
        }
    }
}
