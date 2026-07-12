import SwiftUI
import UniformTypeIdentifiers
import KassiberDaemonKit
import KassiberViewModels

private func reviewLocalized(_ key: String) -> String { AppLocalization.string(key) }

private let swapConfidenceCodes = ["exact", "strong"]
private let swapKindCodes = [
    "manual", "coinjoin", "whirlpool", "chain-swap", "peg-in", "peg-out",
    "reverse-submarine-swap", "submarine-swap", "swap-refund",
]
private let swapPolicyCodes = ["carrying-value", "taxable"]

private func reviewBranchLabel(_ value: String) -> String {
    let parts = value.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false)
    guard let first = parts.first else { return value }
    let label = AppLocalization.code(String(first).trimmingCharacters(in: .whitespaces))
    guard parts.count == 2 else { return label }
    return "\(label) #\(parts[1])"
}

struct JournalsScreen: View {
    @State private var model: JournalsViewModel
    @Environment(\.locale) private var locale
    @Environment(\.kassiberNavigate) private var navigate

    init(daemon: any DaemonClient) {
        _model = State(initialValue: JournalsViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            if let error = model.errorMessage, model.entries.isEmpty {
                ContentUnavailableView(reviewLocalized("state.unavailable"), systemImage: "exclamationmark.triangle", description: Text(AppLocalization.error(error)))
            } else {
                metrics
                Divider()
                HStack(spacing: 0) {
                    List(selection: $model.selectedType) {
                        Text(reviewLocalized("filter.allTypes"))
                            .tag(String?.none)
                        ForEach(model.entryTypes) { entry in
                            HStack {
                                Text(AppLocalization.code(entry.type))
                                Spacer()
                                Text(entry.count, format: .number).foregroundStyle(.secondary)
                            }
                            .tag(String?.some(entry.type))
                        }
                    }
                    .frame(width: 280)
                    Divider()
                    VStack(spacing: 0) {
                        journalHeader
                        Divider()
                        ScrollView {
                            LazyVStack(spacing: 0) {
                                ForEach(model.visibleEntries) { entry in
                                    journalRow(entry)
                                    Divider()
                                }
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle(reviewLocalized("nav.journals"))
        .toolbar {
            ToolbarItemGroup {
                Button(reviewLocalized("nav.quarantine")) { navigate(.quarantine) }
                    .disabled(model.quarantineCount == 0)
                Button(reviewLocalized("nav.reports")) { navigate(.reports) }
            }
            ToolbarItem {
                Button { Task { await model.load() } } label: {
                    Label(reviewLocalized("action.refresh"), systemImage: "arrow.clockwise")
                }
            }
        }
        .task { if model.entries.isEmpty { await model.load() } }
    }

    private var metrics: some View {
        HStack(spacing: 12) {
            metric(reviewLocalized("journal.transactions"), String(model.transactionCount))
            metric(reviewLocalized("journal.entries"), String(model.entryCount))
            metric(reviewLocalized("nav.quarantine"), String(model.quarantineCount))
            if model.needsProcessing {
                Label(reviewLocalized("dashboard.journalsNeeded"), systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
                    .padding()
            }
            Spacer()
        }
        .padding(12)
    }

    private var journalHeader: some View {
        HStack(spacing: 12) {
            journalColumnHeader(reviewLocalized("field.date"), width: 180)
            journalColumnHeader(reviewLocalized("field.type"), width: 120)
            journalColumnHeader(reviewLocalized("field.wallet"), flexible: true)
            journalColumnHeader(reviewLocalized("field.quantity"), width: 160, alignment: .trailing)
            journalColumnHeader(reviewLocalized("journal.gainLoss"), width: 145, alignment: .trailing)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(.bar)
    }

    private func journalRow(_ entry: JournalEntryRow) -> some View {
        HStack(spacing: 12) {
            Text(entry.occurredAt.map { KassiberFormatting.date($0, locale: locale) } ?? "—")
                .frame(width: 180, alignment: .leading)
            Text(AppLocalization.code(entry.type))
                .frame(width: 120, alignment: .leading)
            Text(entry.wallet)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            KassiberAmountText(
                sats: entry.quantityMSat / 1000,
                fiatValue: entry.fiatValue
            )
            .monospacedDigit()
            .kassiberSensitive()
            .frame(width: 160, alignment: .trailing)
            Text(KassiberFormatting.fiat(entry.gainLoss, currency: "EUR", locale: locale))
                .monospacedDigit()
                .kassiberSensitive()
                .frame(width: 145, alignment: .trailing)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .contentShape(Rectangle())
    }

    private func journalColumnHeader(
        _ title: String,
        width: CGFloat? = nil,
        flexible: Bool = false,
        alignment: Alignment = .leading
    ) -> some View {
        Text(title)
            .font(.caption.weight(.semibold))
            .foregroundStyle(.secondary)
            .frame(
                minWidth: flexible ? 80 : width,
                maxWidth: flexible ? .infinity : width,
                alignment: alignment
            )
    }

    private func metric(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading) {
            Text(label).font(.caption).foregroundStyle(.secondary)
            Text(value).font(.title2.monospacedDigit())
        }
        .frame(minWidth: 130, alignment: .leading)
    }
}

struct QuarantineScreen: View {
    let daemon: any DaemonClient
    @State private var model: QuarantineViewModel
    @State private var selection: QuarantineItemRow.ID?
    @State private var detailItem: QuarantineItemRow?
    @Environment(\.locale) private var locale
    @Environment(\.kassiberNavigate) private var navigate

    init(daemon: any DaemonClient) {
        self.daemon = daemon
        _model = State(initialValue: QuarantineViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Picker(reviewLocalized("quarantine.reason"), selection: $model.selectedReason) {
                    Text(reviewLocalized("filter.allReasons")).tag(String?.none)
                    ForEach(model.reasons) { reason in
                        Text("\(AppLocalization.code(reason.reason)) (\(reason.count))")
                            .tag(String?.some(reason.reason))
                    }
                }
                .frame(maxWidth: 280)
                Spacer()
                Text(String(format: reviewLocalized("quarantine.count"), model.visibleItems.count))
                    .foregroundStyle(.secondary)
            }
            .padding(12)
            Divider()
            if model.isLoading && model.items.isEmpty {
                ProgressView(reviewLocalized("state.loading"))
            } else if let error = model.errorMessage, model.items.isEmpty {
                ContentUnavailableView(reviewLocalized("state.unavailable"), systemImage: "exclamationmark.triangle", description: Text(AppLocalization.error(error)))
            } else if model.items.isEmpty {
                ContentUnavailableView(
                    reviewLocalized("quarantine.empty"),
                    systemImage: "tray",
                    description: Text(reviewLocalized("quarantine.emptyDetail"))
                )
            } else {
                Table(model.visibleItems, selection: $selection) {
                    TableColumn(reviewLocalized("field.date")) { item in
                        Text(item.occurredAt.map { KassiberFormatting.date($0, locale: locale) } ?? "—")
                    }.width(min: 135, ideal: 160)
                    TableColumn(reviewLocalized("field.wallet"), value: \.wallet)
                    TableColumn(reviewLocalized("field.direction")) { item in
                        Text(AppLocalization.code(item.direction))
                    }
                    TableColumn(reviewLocalized("field.amount")) { item in
                        KassiberAmountText(sats: item.amountMSat / 1000).monospacedDigit().kassiberSensitive()
                    }
                    TableColumn(reviewLocalized("quarantine.reason")) { item in
                        Text(AppLocalization.code(item.reason))
                    }
                    TableColumn(reviewLocalized("field.details"), value: \.detail)
                }
                .onChange(of: selection) { _, id in detailItem = model.items.first(where: { $0.id == id }) }
            }
        }
        .navigationTitle(reviewLocalized("nav.quarantine"))
        .toolbar {
            ToolbarItem { Button(reviewLocalized("nav.transactions")) { navigate(.transactions) } }
            ToolbarItem {
                Button { Task { await model.load() } } label: {
                    Label(reviewLocalized("action.refresh"), systemImage: "arrow.clockwise")
                }
            }
        }
        .task { if model.items.isEmpty { await model.load() } }
        .sheet(item: $detailItem) { item in
            QuarantineDetailHost(daemon: daemon, reference: item.id) {
                detailItem = nil
                Task { await model.load() }
            }
        }
    }
}

private struct QuarantineDetailHost: View {
    let daemon: any DaemonClient
    let reference: String
    let completed: () -> Void
    @State private var resolver: TransactionResolverViewModel
    init(daemon: any DaemonClient, reference: String, completed: @escaping () -> Void) {
        self.daemon = daemon; self.reference = reference; self.completed = completed
        _resolver = State(initialValue: TransactionResolverViewModel(daemon: daemon, reference: reference))
    }
    var body: some View {
        Group {
            if let transaction = resolver.transaction { TransactionDetailSheet(daemon: daemon, transaction: transaction, completed: completed) }
            else if let error = resolver.errorMessage { ContentUnavailableView(reviewLocalized("state.unavailable"), systemImage: "exclamationmark.triangle", description: Text(AppLocalization.error(error))).frame(width: 600, height: 400) }
            else { ProgressView(reviewLocalized("state.loading")).frame(width: 600, height: 400) }
        }.task { await resolver.load() }
    }
}

struct SwapsScreen: View {
    @State private var model: SwapsViewModel
    @State private var selectedCandidate: TransferCandidateRow.ID?
    @State private var selectedPair: PairedTransferRow.ID?
    @State private var editingPair: PairedTransferRow?
    @State private var editedPairKind = "manual"
    @State private var editedPairPolicy = "carrying-value"
    @State private var showingRule = false
    @State private var ruleName = ""
    @State private var ruleConfidence = "exact"
    @State private var ruleKind = "manual"
    @State private var rulePolicy = "carrying-value"
    @State private var savedViewName = ""
    @Environment(\.locale) private var locale

    init(daemon: any DaemonClient) {
        _model = State(initialValue: SwapsViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Picker(reviewLocalized("nav.swaps"), selection: $model.tab) {
                    Text(reviewLocalized("swaps.review")).tag(SwapsViewModel.Tab.review)
                    Text(reviewLocalized("swaps.paired")).tag(SwapsViewModel.Tab.paired)
                    Text(reviewLocalized("swaps.rules")).tag(SwapsViewModel.Tab.rules)
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 340)
                Spacer()
                if model.conflictCount > 0 {
                    Label(String(format: reviewLocalized("swaps.conflicts"), model.conflictCount), systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.orange)
                }
                if model.tab == .review && model.exactCount > 0 {
                    Button(reviewLocalized("swaps.pairExact")) { Task { await model.pairAllExact() } }
                }
                if model.tab == .rules {
                    Button(reviewLocalized("swaps.applyRules")) { Task { await model.applyRules() } }
                    Button { showingRule = true } label: { Label(reviewLocalized("swaps.newRule"), systemImage: "plus") }
                }
            }
            .padding(12)
            Divider()
            if model.isLoading && model.candidates.isEmpty && model.pairs.isEmpty {
                ProgressView(reviewLocalized("state.loading"))
            } else if let error = model.errorMessage {
                ContentUnavailableView(reviewLocalized("state.unavailable"), systemImage: "exclamationmark.triangle", description: Text(AppLocalization.error(error)))
            } else if model.tab == .review {
                candidateTable
            } else if model.tab == .paired {
                pairedTable
            } else {
                rulesView
            }
        }
        .navigationTitle(reviewLocalized("nav.swaps"))
        .toolbar {
            ToolbarItem {
                Button { Task { await model.load() } } label: {
                    Label(reviewLocalized("action.refresh"), systemImage: "arrow.clockwise")
                }
            }
        }
        .task { if model.candidates.isEmpty && model.pairs.isEmpty { await model.load() } }
        .sheet(isPresented: $showingRule) {
            VStack(alignment: .leading, spacing: 14) {
                Text(reviewLocalized("swaps.newRule")).font(.title2)
                TextField(reviewLocalized("field.name"), text: $ruleName)
                Picker(reviewLocalized("swaps.match"), selection: $ruleConfidence) {
                    ForEach(swapConfidenceCodes, id: \.self) { code in
                        Text(AppLocalization.code(code)).tag(code)
                    }
                }
                Picker(reviewLocalized("field.type"), selection: $ruleKind) {
                    ForEach(swapKindCodes, id: \.self) { code in
                        Text(AppLocalization.code(code)).tag(code)
                    }
                }
                Picker(reviewLocalized("swaps.policy"), selection: $rulePolicy) {
                    ForEach(swapPolicyCodes, id: \.self) { code in
                        Text(AppLocalization.code(code)).tag(code)
                    }
                }
                HStack { Spacer(); Button(reviewLocalized("action.cancel")) { showingRule = false }; Button(reviewLocalized("action.add")) { Task { await model.createRule(name: ruleName, confidence: ruleConfidence, kind: ruleKind, policy: rulePolicy); showingRule = false } }.buttonStyle(.borderedProminent).disabled(ruleName.isEmpty) }
            }.padding(24).frame(width: 450)
        }
        .sheet(item: $editingPair) { pair in
            VStack(alignment: .leading, spacing: 16) {
                Text(reviewLocalized("swaps.editPair")).font(.title2)
                LabeledContent(reviewLocalized("swaps.route")) {
                    Text("\(pair.outWallet) (\(pair.outAsset)) → \(pair.inWallet) (\(pair.inAsset))")
                        .foregroundStyle(.secondary)
                }
                Picker(reviewLocalized("field.type"), selection: $editedPairKind) {
                    ForEach(swapKindCodes, id: \.self) { code in
                        Text(AppLocalization.code(code)).tag(code)
                    }
                }
                Picker(reviewLocalized("swaps.policy"), selection: $editedPairPolicy) {
                    ForEach(swapPolicyCodes, id: \.self) { code in
                        Text(AppLocalization.code(code))
                            .tag(code)
                            .disabled(pair.outAsset == pair.inAsset && code == "taxable")
                    }
                }
                if pair.outAsset == pair.inAsset {
                    Text(reviewLocalized("swaps.sameAssetTaxableHint"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                HStack {
                    Spacer()
                    Button(reviewLocalized("action.cancel")) { editingPair = nil }
                    Button(reviewLocalized("action.save")) {
                        let kind = editedPairKind
                        let policy = editedPairPolicy
                        Task {
                            await model.update(pair, kind: kind, policy: policy)
                            if model.errorMessage == nil { editingPair = nil }
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(
                        model.isLoading
                            || (editedPairKind == pair.kind && editedPairPolicy == pair.policy)
                            || (pair.outAsset == pair.inAsset && editedPairPolicy == "taxable")
                    )
                }
            }
            .padding(24)
            .frame(width: 520)
        }
    }

    private var candidateTable: some View {
        Table(model.candidates, selection: $selectedCandidate) {
            TableColumn(reviewLocalized("swaps.outWallet"), value: \.outWallet)
            TableColumn(reviewLocalized("swaps.inWallet"), value: \.inWallet)
            TableColumn(reviewLocalized("swaps.route")) { row in Text("\(row.outAsset) → \(row.inAsset)") }
            TableColumn(reviewLocalized("field.amount")) { row in
                KassiberAmountText(sats: row.outAmountMSat / 1000).monospacedDigit().kassiberSensitive()
            }
            TableColumn(reviewLocalized("swaps.fee")) { row in
                KassiberAmountText(sats: row.feeMSat / 1000).monospacedDigit().kassiberSensitive()
            }
            TableColumn(reviewLocalized("swaps.match")) { row in
                Text("\(AppLocalization.code(row.confidence)) · \(AppLocalization.code(row.method))")
            }
            TableColumn(reviewLocalized("field.type")) { row in Text(AppLocalization.code(row.kind)) }
        }
        .safeAreaInset(edge: .bottom) {
            if let row = model.candidates.first(where: { $0.id == selectedCandidate }) {
                HStack {
                    Text("\(row.outWallet) → \(row.inWallet)").foregroundStyle(.secondary)
                    Spacer()
                    Button(reviewLocalized("swaps.dismiss"), role: .destructive) { Task { await model.dismiss(row) } }
                    Button(reviewLocalized("swaps.pair")) { Task { await model.pair(row) } }.buttonStyle(.borderedProminent)
                }.padding(10).background(.bar)
            }
        }
    }

    private var pairedTable: some View {
        Table(model.pairs, selection: $selectedPair) {
            TableColumn(reviewLocalized("swaps.outWallet"), value: \.outWallet)
            TableColumn(reviewLocalized("swaps.inWallet"), value: \.inWallet)
            TableColumn(reviewLocalized("swaps.route")) { row in Text("\(row.outAsset) → \(row.inAsset)") }
            TableColumn(reviewLocalized("swaps.fee")) { row in
                KassiberAmountText(sats: row.feeMSat / 1000).monospacedDigit().kassiberSensitive()
            }
            TableColumn(reviewLocalized("field.type")) { row in Text(AppLocalization.code(row.kind)) }
            TableColumn(reviewLocalized("swaps.policy")) { row in Text(AppLocalization.code(row.policy)) }
            TableColumn(reviewLocalized("swaps.source")) { row in Text(AppLocalization.code(row.source)) }
        }
        .safeAreaInset(edge: .bottom) {
            if let row = model.pairs.first(where: { $0.id == selectedPair }) {
                HStack {
                    Text("\(row.outWallet) → \(row.inWallet)").foregroundStyle(.secondary)
                    Spacer()
                    Button(reviewLocalized("swaps.editPair")) { beginEditing(row) }
                    Button(reviewLocalized("swaps.unpair"), role: .destructive) { Task { await model.unpair(row) } }
                }
                .padding(10)
                .background(.bar)
            }
        }
    }

    private func beginEditing(_ pair: PairedTransferRow) {
        editedPairKind = pair.kind
        editedPairPolicy = pair.policy
        editingPair = pair
    }

    private var rulesView: some View {
        HSplitView {
            VStack(spacing: 0) {
                HStack { TextField(reviewLocalized("swaps.savedViewName"), text: $savedViewName); Button(reviewLocalized("swaps.saveView")) { Task { await model.createSavedView(name: savedViewName); savedViewName = "" } }.disabled(savedViewName.isEmpty) }.padding(10)
                List(model.savedViews) { view in HStack { VStack(alignment: .leading) { Text(view.name); Text(view.filter).font(.caption).foregroundStyle(.secondary) }; Spacer(); Button(role: .destructive) { Task { await model.delete(view) } } label: { Image(systemName: "trash") } } }
            }.frame(minWidth: 260, idealWidth: 300)
            Table(model.rules) {
                TableColumn(reviewLocalized("field.name"), value: \.name)
                TableColumn(reviewLocalized("field.type")) { row in Text(AppLocalization.code(row.kind)) }
                TableColumn(reviewLocalized("swaps.policy")) { row in Text(AppLocalization.code(row.policy)) }
                TableColumn(reviewLocalized("field.details"), value: \.predicate)
                TableColumn(reviewLocalized("field.status")) { row in Toggle("", isOn: Binding(get: { row.enabled }, set: { _ in Task { await model.toggle(row) } })) }.width(60)
                TableColumn("") { row in Button(role: .destructive) { Task { await model.delete(row) } } label: { Image(systemName: "trash") } }.width(45)
            }
        }
    }
}

struct ReconcileScreen: View {
    @State private var model: ReconcileViewModel
    @State private var showingCSVPicker = false
    @State private var copied = false

    init(daemon: any DaemonClient) {
        let model = ReconcileViewModel(daemon: daemon)
        if let preview = ProcessInfo.processInfo.environment["KASSIBER_PREVIEW_RECONCILE"] {
            model.input = preview
        }
        _model = State(initialValue: model)
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .top, spacing: 12) {
                TextEditor(text: $model.input)
                    .font(.body.monospaced())
                    .frame(minHeight: 90, maxHeight: 130)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
                Button {
                    Task { await model.check() }
                } label: {
                    Label(reviewLocalized("reconcile.check"), systemImage: "magnifyingglass")
                }
                .buttonStyle(.borderedProminent)
                .disabled(model.input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.isLoading)
                Menu {
                    Button(reviewLocalized("reconcile.importCSV")) { showingCSVPicker = true }
                    Divider()
                    Button(reviewLocalized("reconcile.verifyOnchain")) { Task { await model.check(onchain: true) } }
                } label: { Image(systemName: "ellipsis.circle") }
            }
            .padding(12)
            if !model.results.isEmpty {
                HStack {
                    summary(reviewLocalized("reconcile.owned"), model.owned)
                    summary(reviewLocalized("reconcile.external"), model.external)
                    summary(reviewLocalized("reconcile.unknown"), model.unknown)
                    summary(reviewLocalized("reconcile.invalid"), model.invalid)
                    Spacer()
                    Text(String(format: reviewLocalized("reconcile.walletsScanned"), model.walletsScanned))
                        .foregroundStyle(.secondary)
                    Button {
                        NativeAffordances.copy(model.resultsCSV)
                        copied = true
                        Task { @MainActor in
                            try? await Task.sleep(for: .seconds(1.2))
                            copied = false
                        }
                    } label: {
                        Label(
                            reviewLocalized(copied ? "reconcile.copied" : "reconcile.copyCsv"),
                            systemImage: copied ? "checkmark" : "doc.on.clipboard"
                        )
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 8)
            }
            Divider()
            if let error = model.errorMessage {
                ContentUnavailableView(reviewLocalized("state.unavailable"), systemImage: "exclamationmark.triangle", description: Text(AppLocalization.error(error)))
            } else if model.results.isEmpty {
                ContentUnavailableView(
                    reviewLocalized("reconcile.empty"),
                    systemImage: "checkmark.arrow.trianglehead.counterclockwise",
                    description: Text(reviewLocalized("reconcile.emptyDetail"))
                )
            } else {
                Table(model.results) {
                    TableColumn(reviewLocalized("reconcile.input")) { row in
                        HStack {
                            Text(row.input).lineLimit(1).textSelection(.enabled)
                            Button { NativeAffordances.copy(row.input) } label: { Image(systemName: "doc.on.doc") }
                                .buttonStyle(.borderless)
                                .help(reviewLocalized("reconcile.copyInput"))
                        }
                    }
                    TableColumn(reviewLocalized("field.type")) { row in Text(AppLocalization.code(row.type)) }
                    TableColumn(reviewLocalized("field.status")) { row in Text(AppLocalization.code(row.status)) }
                    TableColumn(reviewLocalized("reconcile.classification")) { row in Text(AppLocalization.code(row.classification)) }
                    TableColumn(reviewLocalized("field.wallet")) { row in Text(row.wallets.joined(separator: ", ")) }
                    TableColumn(reviewLocalized("reconcile.branch")) { row in Text(reviewBranchLabel(row.branch)) }
                    TableColumn(reviewLocalized("field.details"), value: \.note)
                }
            }
        }
        .navigationTitle(reviewLocalized("nav.reconcile"))
        .fileImporter(isPresented: $showingCSVPicker, allowedContentTypes: [.commaSeparatedText, .tabSeparatedText], allowsMultipleSelection: false) { result in
            if case let .success(urls) = result, let url = urls.first {
                Task {
                    let scoped = url.startAccessingSecurityScopedResource()
                    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                    if let text = try? String(contentsOf: url, encoding: .utf8) {
                        model.loadCSV(name: url.lastPathComponent, text: text)
                        await model.check()
                    }
                }
            }
        }
        .task { if !model.input.isEmpty && model.results.isEmpty { await model.check() } }
    }

    private func summary(_ label: String, _ value: Int) -> some View {
        VStack(alignment: .leading) {
            Text(label).font(.caption).foregroundStyle(.secondary)
            Text(value, format: .number).font(.title3.monospacedDigit())
        }.frame(minWidth: 90, alignment: .leading)
    }
}
