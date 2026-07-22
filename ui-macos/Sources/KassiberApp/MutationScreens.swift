import SwiftUI
import UniformTypeIdentifiers
import KassiberDaemonKit
import KassiberViewModels

private func localized(_ key: String) -> String { AppLocalization.string(key) }

struct ConnectionsScreen: View {
    let daemon: any DaemonClient
    @State private var wallets: WalletsViewModel
    @State private var catalog: ConnectionSettingsViewModel
    @State private var mutation: WalletMutationViewModel
    @State private var showingAdd = false
    @State private var editing: WalletRow?

    init(daemon: any DaemonClient) {
        self.daemon = daemon
        _wallets = State(initialValue: WalletsViewModel(daemon: daemon))
        _catalog = State(initialValue: ConnectionSettingsViewModel(daemon: daemon))
        _mutation = State(initialValue: WalletMutationViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            Picker(localized("connections.layer"), selection: $catalog.layer) {
                ForEach(ConnectionLayer.allCases) { layer in
                    Text(localized("connections.layer.\(layer.rawValue)")).tag(layer)
                }
            }
            .pickerStyle(.segmented)
            .padding(12)
            Table(catalog.visibleEndpoints) {
                TableColumn(localized("connections.endpoint"), value: \.name)
                TableColumn(localized("field.type"), value: \.kind)
                TableColumn(localized("field.network"), value: \.network)
                TableColumn(localized("connections.trust")) { endpoint in
                    if endpoint.insecure {
                        Label(localized("connections.insecure"), systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                    } else if endpoint.usesCredentials {
                        Label(localized("connections.credentials"), systemImage: "lock")
                    } else {
                        Text("—")
                    }
                }
            }
            .frame(minHeight: 180, idealHeight: 250)
            Divider()
            Table(wallets.wallets.filter { wallet in
                switch catalog.layer {
                case .liquid: wallet.chain == "liquid"
                case .lightning: ["lnd", "cln", "nwc"].contains(wallet.kind)
                case .base: wallet.chain != "liquid" && !["lnd", "cln", "nwc"].contains(wallet.kind)
                }
            }) {
                TableColumn(localized("field.name"), value: \.label)
                TableColumn(localized("field.type"), value: \.kind)
                TableColumn(localized("field.network"), value: \.network)
                TableColumn(localized("field.status")) { wallet in
                    Text(wallet.deprecated ? localized("wallet.archivedShort") : wallet.syncStatus)
                }
                TableColumn("") { wallet in
                    Button(localized("action.edit")) {
                        mutation.configureForEdit(wallet)
                        editing = wallet
                    }
                    .buttonStyle(.link)
                }
                .width(55)
            }
        }
        .navigationTitle(localized("nav.connections"))
        .toolbar {
            Button {
                mutation = WalletMutationViewModel(daemon: daemon)
                showingAdd = true
            } label: {
                Label(localized("wallet.add"), systemImage: "plus")
            }
        }
        .sheet(isPresented: $showingAdd) {
            WalletAddSheet(model: mutation) {
                showingAdd = false
                Task { await wallets.load() }
            }
        }
        .sheet(item: $editing) { wallet in
            WalletEditSheet(model: mutation, wallet: wallet) {
                editing = nil
                Task { await wallets.load() }
            }
        }
        .task {
            if wallets.wallets.isEmpty { await wallets.load() }
            await catalog.load()
        }
    }
}

struct LayeredSettingsScreen: View {
    @State private var catalog: ConnectionSettingsViewModel
    @AppStorage("language") private var language = "en"

    init(daemon: any DaemonClient) {
        _catalog = State(initialValue: ConnectionSettingsViewModel(daemon: daemon))
    }

    var body: some View {
        Form {
            Section(localized("settings.bookScope")) {
                LabeledContent(localized("book.setName"), value: catalog.workspace)
                LabeledContent(localized("book.name"), value: catalog.profile)
            }
            Section(localized("settings.connectionLayers")) {
                Picker(localized("connections.layer"), selection: $catalog.layer) {
                    ForEach(ConnectionLayer.allCases) { layer in
                        Text(localized("connections.layer.\(layer.rawValue)")).tag(layer)
                    }
                }
                ForEach(catalog.visibleEndpoints) { endpoint in
                    LabeledContent(endpoint.name) {
                        HStack {
                            Text([endpoint.kind, endpoint.network].filter { !$0.isEmpty }.joined(separator: " · "))
                            if endpoint.insecure {
                                Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange)
                            }
                            if endpoint.usesCredentials { Image(systemName: "lock") }
                        }
                    }
                }
                Text(localized("settings.privacyBoundary")).font(.caption).foregroundStyle(.secondary)
            }
            Section(localized("settings.language")) {
                Picker(localized("settings.language"), selection: $language) {
                    Text(localized("language.english")).tag("en")
                    Text(localized("language.germanAT")).tag("de-AT")
                }
                Text(localized("settings.languageRestart")).font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .navigationTitle(localized("nav.settings"))
        .task { await catalog.load() }
    }
}

private struct WalletAddSheet: View {
    @Bindable var model: WalletMutationViewModel
    let didSave: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            Form {
                Section(localized("wallet.watchOnly")) {
                    TextField(localized("field.name"), text: $model.label)
                    Picker(localized("field.network"), selection: $model.network) {
                        Text("main").tag("main")
                        Text("testnet").tag("testnet")
                        Text("regtest").tag("regtest")
                    }
                    TextField(localized("wallet.backendOptional"), text: $model.backend)
                    Stepper(value: $model.gapLimit, in: 1...10_000) {
                        LabeledContent(localized("wallet.gapLimit"), value: String(model.gapLimit))
                    }
                    TextEditor(text: $model.walletMaterial)
                        .font(.system(.body, design: .monospaced))
                        .frame(minHeight: 90)
                        .overlay(alignment: .topLeading) {
                            if model.walletMaterial.isEmpty {
                                Text(localized("wallet.materialPrompt"))
                                    .foregroundStyle(.tertiary)
                                    .padding(.top, 8)
                                    .padding(.leading, 5)
                                    .allowsHitTesting(false)
                            }
                        }
                    HStack {
                        Button(localized("wallet.detectPreview")) { Task { await model.detectAndPreview() } }
                            .disabled(model.walletMaterial.isEmpty || model.isWorking)
                        if model.isWorking { ProgressView().controlSize(.small) }
                        if !model.detectionMessage.isEmpty {
                            Text(model.detectionMessage).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
                if !model.preview.isEmpty {
                    Section(localized("wallet.previewAddresses")) {
                        ForEach(model.preview.prefix(8)) { item in
                            VStack(alignment: .leading, spacing: 2) {
                                Text(item.address).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                                Text("\(item.branch) · \(item.derivationPath)").font(.caption2).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                if let error = model.errorMessage {
                    Text(AppLocalization.error(error)).foregroundStyle(.red).font(.caption)
                }
            }
            .formStyle(.grouped)
            Divider()
            HStack {
                Button(localized("action.cancel")) { dismiss() }
                Spacer()
                Button(localized("action.add")) { Task { await model.create() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.label.isEmpty || model.preview.isEmpty || model.isWorking)
            }
            .padding()
        }
        .frame(width: 680, height: 640)
        .onChange(of: model.didSave) { _, saved in if saved { didSave() } }
    }
}

private struct WalletEditSheet: View {
    @Bindable var model: WalletMutationViewModel
    let wallet: WalletRow
    let didSave: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            Form {
                TextField(localized("field.name"), text: $model.label)
                Toggle(localized("wallet.archive"), isOn: $model.archived)
                Text(localized("wallet.archiveHint")).font(.caption).foregroundStyle(.secondary)
                SecureField(localized("wallet.passphraseOptional"), text: $model.passphrase)
                Text(localized("wallet.passphraseHint")).font(.caption).foregroundStyle(.secondary)
                if let error = model.errorMessage { Text(AppLocalization.error(error)).foregroundStyle(.red).font(.caption) }
            }
            .formStyle(.grouped)
            Divider()
            HStack {
                Button(localized("action.cancel")) { dismiss() }
                Spacer()
                Button(localized("action.save")) { Task { await model.update(wallet: wallet) } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.label.isEmpty || model.isWorking)
            }
            .padding()
        }
        .frame(width: 480, height: 280)
        .onChange(of: model.didSave) { _, saved in if saved { didSave() } }
    }
}

struct ImportsScreen: View {
    @State private var model: LedgerImportViewModel
    @State private var wallets: WalletsViewModel
    @State private var showingPicker = false
    @State private var sourceFormat = "generic_ledger"
    @State private var wallet: String?

    init(daemon: any DaemonClient) {
        _model = State(initialValue: LedgerImportViewModel(daemon: daemon))
        _wallets = State(initialValue: WalletsViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Picker(localized("import.format"), selection: $sourceFormat) {
                    Text(localized("import.genericLedger")).tag("generic_ledger")
                    Text("BTCPay CSV").tag("btcpay_csv")
                    Text("Phoenix CSV").tag("phoenix_csv")
                    Text("River CSV").tag("river_csv")
                    Text("Ledger Live CSV").tag("ledgerlive_csv")
                }
                .frame(width: 220)
                Picker(localized("field.wallet"), selection: $wallet) {
                    Text(localized("wallet.select")).tag(String?.none)
                    ForEach(wallets.wallets) { item in Text(item.label).tag(String?.some(item.id)) }
                }
                .frame(width: 220)
                Button(localized("import.chooseFile")) { showingPicker = true }
            }
            GroupBox(localized("import.preview")) {
                if model.isWorking {
                    ProgressView().padding()
                } else if let file = model.fileURL {
                    VStack(alignment: .leading, spacing: 8) {
                        Label(file.lastPathComponent, systemImage: "doc")
                        HStack {
                            LabeledContent(localized("import.mapped"), value: String(model.mapped))
                            LabeledContent(localized("import.errors"), value: String(model.errors))
                        }
                        Table(model.rows) {
                            TableColumn(localized("import.previewRows"), value: \.values)
                        }
                        .frame(minHeight: 250)
                    }
                    .padding(8)
                } else {
                    ContentUnavailableView(localized("import.noPreview"), systemImage: "doc.badge.plus")
                        .frame(maxWidth: .infinity, minHeight: 280)
                }
            }
            if let error = model.errorMessage { Text(AppLocalization.error(error)).foregroundStyle(.red).font(.caption) }
            HStack {
                Spacer()
                Button(localized("import.confirm")) {
                    Task { await model.importFile(sourceFormat: sourceFormat, wallet: wallet) }
                }
                .buttonStyle(.borderedProminent)
                .disabled(!model.canImport || wallet == nil || model.isWorking)
            }
        }
        .padding(20)
        .navigationTitle(localized("nav.imports"))
        .fileImporter(
            isPresented: $showingPicker,
            allowedContentTypes: [.commaSeparatedText, .tabSeparatedText, .spreadsheet],
            allowsMultipleSelection: false
        ) { result in
            if case let .success(urls) = result, let url = urls.first {
                Task { await model.preview(url) }
            }
        }
        .task {
            await wallets.load()
            if let path = ProcessInfo.processInfo.environment["KASSIBER_PREVIEW_IMPORT_FILE"],
               model.fileURL == nil {
                await model.preview(URL(fileURLWithPath: path))
            }
        }
        .alert(localized("import.complete"), isPresented: Binding(
            get: { model.didImport },
            set: { if !$0 { model.acknowledgeImport() } }
        )) { Button(localized("action.ok")) {} }
    }
}

struct BooksScreen: View {
    let daemon: any DaemonClient
    @Environment(\.kassiberNavigate) private var navigate
    @State private var model: BooksViewModel
    @State private var showingNewWorkspace = false
    @State private var showingNewBook: WorkspaceRow?
    @State private var renamingWorkspace: WorkspaceRow?
    @State private var editingBook: BookRow?
    @State private var workspaceName = ""
    @State private var bookName = ""
    @State private var country = "generic"
    @State private var algorithm = "fifo"
    @State private var showingOnboarding = false

    init(daemon: any DaemonClient) {
        self.daemon = daemon
        _model = State(initialValue: BooksViewModel(daemon: daemon))
    }

    var body: some View {
        Group {
        if model.workspaces.isEmpty && !model.isWorking {
            ContentUnavailableView {
                Label(localized("books.noBooks"), systemImage: "books.vertical")
            } description: { Text(localized("books.noBooksHint")) } actions: {
                Button(localized("books.startSetup")) { showingOnboarding = true }.buttonStyle(.borderedProminent)
            }
        } else { List {
            ForEach(model.workspaces) { workspace in
                Section {
                    ForEach(workspace.books) { book in
                        HStack {
                            VStack(alignment: .leading, spacing: 3) {
                                HStack { Text(book.name).font(.headline); if book.active { Text(localized("books.active")).font(.caption).foregroundStyle(.secondary) } }
                                Text("\(book.taxCountry) · \(book.gainsAlgorithm.uppercased()) · \(book.fiatCurrency) · \(book.wallets) \(localized("nav.wallets").lowercased())")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button(localized("action.edit")) { bookName = book.name; country = book.taxCountry; algorithm = book.gainsAlgorithm; editingBook = book }
                            Button(localized("books.open")) { Task { await model.switchBook(book.id) } }.disabled(book.active)
                        }.padding(.vertical, 4)
                    }
                } header: {
                    HStack {
                        Text(workspace.name)
                        Text([workspace.currency, workspace.jurisdiction].filter { !$0.isEmpty }.joined(separator: " · ")).foregroundStyle(.secondary)
                        Spacer()
                        Button {
                            UserDefaults.standard.set(workspace.id, forKey: "birdsEye.workspaceID")
                            navigate(.birdsEye)
                        } label: {
                            Label(localized("books.overview"), systemImage: "rectangle.3.group")
                        }
                        Button {
                            workspaceName = workspace.name
                            renamingWorkspace = workspace
                        } label: {
                            Label(localized("books.renameSet"), systemImage: "pencil")
                        }
                        Button { bookName = ""; country = "generic"; algorithm = "fifo"; showingNewBook = workspace } label: { Label(localized("books.addBook"), systemImage: "plus") }
                    }
                }
            }
        } }
        }
        .navigationTitle(localized("nav.books"))
        .toolbar { Button { workspaceName = ""; showingNewWorkspace = true } label: { Label(localized("books.addSet"), systemImage: "plus") }; Button { Task { await model.load() } } label: { Image(systemName: "arrow.clockwise") } }
        .task { await model.load() }
        .onAppear {
            if ProcessInfo.processInfo.environment["KASSIBER_PREVIEW_ONBOARDING"] == "1" {
                showingOnboarding = true
            }
        }
        .sheet(isPresented: $showingNewWorkspace) {
            VStack(alignment: .leading, spacing: 16) { Text(localized("books.addSet")).font(.title2); TextField(localized("book.setName"), text: $workspaceName); HStack { Spacer(); Button(localized("action.cancel")) { showingNewWorkspace = false }; Button(localized("action.add")) { Task { await model.createWorkspace(label: workspaceName); showingNewWorkspace = false } }.buttonStyle(.borderedProminent).disabled(workspaceName.isEmpty) } }.padding(24).frame(width: 440)
        }
        .sheet(item: $renamingWorkspace) { workspace in
            VStack(alignment: .leading, spacing: 16) {
                Text(localized("books.renameSet")).font(.title2)
                TextField(localized("book.setName"), text: $workspaceName)
                HStack {
                    Spacer()
                    Button(localized("action.cancel")) { renamingWorkspace = nil }
                    Button(localized("action.save")) {
                        Task {
                            await model.renameWorkspace(workspace.id, label: workspaceName)
                            if model.errorMessage == nil { renamingWorkspace = nil }
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(workspaceName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.isWorking)
                }
            }
            .padding(24)
            .frame(width: 440)
        }
        .sheet(item: $showingNewBook) { workspace in bookForm(title: localized("books.addBook")) { Task { await model.createBook(workspaceID: workspace.id, label: bookName, country: country, algorithm: algorithm); showingNewBook = nil } } }
        .sheet(item: $editingBook) { book in bookForm(title: localized("books.editBook")) { Task { await model.updateBook(book.id, label: bookName, country: country, algorithm: algorithm); editingBook = nil } } }
        .sheet(isPresented: $showingOnboarding) { FullOnboardingScreen(daemon: daemon) { showingOnboarding = false; Task { await model.load() } } }
    }

    private func bookForm(title: String, save: @escaping () -> Void) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(title).font(.title2)
            TextField(localized("book.name"), text: $bookName)
            Picker(localized("book.taxCountry"), selection: $country) { Text(localized("book.generic")).tag("generic"); Text(localized("book.austria")).tag("AT") }
            Picker(localized("book.gains"), selection: $algorithm) {
                if country == "AT" { Text(localized("book.movingAverageAT")).tag("moving_average_at") }
                else { ForEach(["fifo", "lifo", "hifo", "lofo"], id: \.self) { Text($0.uppercased()).tag($0) } }
            }
            Text(localized("books.policyWarning")).font(.caption).foregroundStyle(.secondary)
            HStack { Spacer(); Button(localized("action.cancel")) { showingNewBook = nil; editingBook = nil }; Button(localized("action.save"), action: save).buttonStyle(.borderedProminent).disabled(bookName.isEmpty) }
        }.padding(24).frame(width: 480)
    }
}

struct SyncProgressCard: View {
    @Bindable var model: BookRefreshCoordinator
    var onNavigate: ((AppScreen) -> Void)?

    init(model: BookRefreshCoordinator, onNavigate: ((AppScreen) -> Void)? = nil) {
        self.model = model
        self.onNavigate = onNavigate
    }

    var body: some View {
        if model.isMinimized {
            Button { model.restore() } label: {
                HStack(spacing: 8) {
                    if model.isRunning { ProgressView().controlSize(.small) }
                    Label(localized("sync.showProgress"), systemImage: "arrow.clockwise")
                }
            }
            .buttonStyle(.borderedProminent)
        } else {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Label(
                        model.isFirstSync
                            ? localized("sync.first.title")
                            : localized("sync.incremental.title"),
                        systemImage: model.isRunning
                            ? "arrow.triangle.2.circlepath" : terminalSymbol
                    )
                        .font(.headline)
                    Spacer()
                    if model.isRunning {
                        Button { model.minimize() } label: { Image(systemName: "minus") }
                            .buttonStyle(.plain)
                    } else {
                        Button { model.dismiss() } label: { Image(systemName: "xmark") }
                            .buttonStyle(.plain)
                    }
                }
                Text(model.isFirstSync ? localized("sync.first.body") : localized("sync.incremental.body"))
                    .font(.caption)
                    .foregroundStyle(.secondary)

                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(localized("sync.phase.\(model.phase)"))
                            .font(.caption.weight(.medium))
                            .foregroundStyle(model.outcome == .failed ? .red : .primary)
                        if !model.source.isEmpty {
                            Text(model.source).font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    if model.isRunning {
                        Text(model.progress, format: .percent.precision(.fractionLength(0)))
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                }
                ProgressView(value: model.progress)
                if !model.detail.isEmpty {
                    Text(model.detail).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
                if !model.progressDetails.isEmpty {
                    Text(model.progressDetails.joined(separator: " · "))
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(.tertiary)
                }

                if model.isRunning && model.mode != .journals {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(Array(BookRefreshCoordinator.milestones.enumerated()), id: \.element.id) { index, milestone in
                            HStack(spacing: 8) {
                                Image(systemName: milestoneSymbol(index))
                                    .frame(width: 14)
                                    .foregroundStyle(milestoneColor(index))
                                Text(localized(milestone.localizationKey))
                                    .font(.caption2)
                                    .foregroundStyle(index == model.activeMilestoneIndex ? .primary : .secondary)
                                    .strikethrough(index < model.activeMilestoneIndex, color: .secondary)
                            }
                        }
                    }
                    .padding(.vertical, 2)
                }

                if let key = model.completedMessage {
                    Text(localized(key))
                        .font(.caption)
                        .foregroundStyle(model.outcome == .clean ? .green : .orange)
                }
                if let error = model.errorMessage {
                    Text(AppLocalization.error(error)).font(.caption).foregroundStyle(.red).lineLimit(3)
                }

                if model.isRunning {
                    HStack {
                        Text(localized("sync.keepUsing"))
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                        Spacer()
                        Button(localized("sync.continueBackground")) { model.minimize() }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                    }
                } else if let target = model.terminalTarget, let onNavigate {
                    HStack {
                        Spacer()
                        Button(localized(reviewActionKey(target))) { onNavigate(target) }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                    }
                }
            }
            .padding(16)
            .frame(width: 420)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 18))
            .overlay(RoundedRectangle(cornerRadius: 18).stroke(.separator))
            .shadow(radius: 14, y: 6)
        }
    }

    private var terminalSymbol: String {
        switch model.outcome {
        case .clean: "checkmark.circle.fill"
        case .reviewRequired, .partial: "exclamationmark.triangle.fill"
        case .failed: "xmark.octagon.fill"
        default: "arrow.triangle.2.circlepath"
        }
    }

    private func milestoneSymbol(_ index: Int) -> String {
        if index < model.activeMilestoneIndex { return "checkmark.circle.fill" }
        if index == model.activeMilestoneIndex { return "arrow.triangle.2.circlepath.circle.fill" }
        return "circle"
    }

    private func milestoneColor(_ index: Int) -> Color {
        if index < model.activeMilestoneIndex { return .green }
        if index == model.activeMilestoneIndex { return .accentColor }
        return .secondary.opacity(0.45)
    }

    private func reviewActionKey(_ target: AppScreen) -> String {
        switch target {
        case .quarantine: "sync.reviewQuarantine"
        case .swaps: "sync.reviewSwaps"
        case .journals: "sync.openJournals"
        default: "sync.openLogs"
        }
    }
}
