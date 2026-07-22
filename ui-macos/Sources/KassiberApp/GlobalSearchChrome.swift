import SwiftUI
import AppKit
import KassiberViewModels

struct GlobalSearchChromeModifier: ViewModifier {
    @Bindable var shell: AppShellViewModel
    let aiFeaturesEnabled: Bool
    let developerToolsEnabled: Bool
    let contentIdentity: ScreenContentIdentity
    @State private var model: GlobalSearchViewModel
    @State private var presented = false
    @FocusState private var searchFocused: Bool

    init(
        shell: AppShellViewModel,
        aiFeaturesEnabled: Bool,
        developerToolsEnabled: Bool,
        contentIdentity: ScreenContentIdentity
    ) {
        self.shell = shell
        self.aiFeaturesEnabled = aiFeaturesEnabled
        self.developerToolsEnabled = developerToolsEnabled
        self.contentIdentity = contentIdentity
        _model = State(initialValue: GlobalSearchViewModel(daemon: shell.daemon))
    }

    func body(content: Content) -> some View {
        content
            .sheet(isPresented: $presented) { searchPanel }
            .onReceive(NotificationCenter.default.publisher(for: KassiberHostNotification.showSearch)) { _ in
                guard shell.authenticationState.isUnlocked else { return }
                presented = true
            }
            .task {
                model.aiFeaturesEnabled = aiFeaturesEnabled
                model.developerToolsEnabled = developerToolsEnabled
                await model.load()
            }
            .task {
                let events = await shell.daemon.events()
                for await event in events {
                    guard !Task.isCancelled else { return }
                    await model.handleHostEvent(event)
                }
            }
            .task(id: model.query) {
                guard !model.query.isEmpty else { return }
                try? await Task.sleep(for: .milliseconds(180))
                guard !Task.isCancelled else { return }
                await model.resolveExactTransactionIfNeeded()
            }
            .onChange(of: contentIdentity) { _, _ in
                Task { await model.load() }
            }
            .onChange(of: aiFeaturesEnabled) { _, enabled in
                model.aiFeaturesEnabled = enabled
            }
            .onChange(of: developerToolsEnabled) { _, enabled in
                model.developerToolsEnabled = enabled
            }
    }

    private var searchPanel: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                TextField(AppLocalization.string("search.prompt"), text: $model.query)
                    .textFieldStyle(.plain)
                    .font(.title3)
                    .focused($searchFocused)
                    .onSubmit {
                        if let first = model.results.first { activate(first) }
                    }
                if model.isResolving { ProgressView().controlSize(.small) }
                Button {
                    presented = false
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .help(AppLocalization.string("action.close"))
            }
            .padding(16)
            Divider()
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                    suggestions
                }
                .padding(8)
            }
        }
        .frame(minWidth: 580, idealWidth: 640, minHeight: 390, idealHeight: 480)
        .onAppear { searchFocused = true }
        .onDisappear {
            searchFocused = false
            model.query = ""
        }
        .onExitCommand { presented = false }
    }

    @ViewBuilder
    private var suggestions: some View {
        if model.query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            Text(AppLocalization.string("search.title"))
        } else if model.results.isEmpty && !model.isResolving {
            Text(AppLocalization.string("search.noResults"))
        } else {
            ForEach(model.results) { result in
                Button { activate(result) } label: {
                    Label {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(title(result))
                            HStack(spacing: 6) {
                                Text(AppLocalization.string("search.category.\(result.category.rawValue)"))
                                if let subtitle = subtitle(result), !subtitle.isEmpty {
                                    Text("·")
                                    Text(subtitle).lineLimit(1)
                                }
                            }
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        }
                    } icon: {
                        Image(systemName: result.icon)
                    }
                }
                .buttonStyle(.plain)
                .padding(.horizontal, 8)
                .padding(.vertical, 7)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            if model.isResolving {
                HStack {
                    ProgressView().controlSize(.small)
                    Text(AppLocalization.string("state.loading"))
                }
            }
        }
    }

    private func title(_ result: GlobalSearchResult) -> String {
        result.titleKey.map(AppLocalization.string) ?? result.title ?? result.id
    }

    private func subtitle(_ result: GlobalSearchResult) -> String? {
        result.subtitleKey.map(AppLocalization.string) ?? result.subtitle
    }

    private func activate(_ result: GlobalSearchResult) {
        switch result.destination {
        case let .screen(screen):
            shell.selection = screen
        case let .connection(id):
            UserDefaults.standard.set(id, forKey: "connections.selectedWalletID")
            shell.selection = .connections
        case let .transaction(id):
            UserDefaults.standard.set(id, forKey: "transactions.selectedID")
            shell.selection = .transactions
        case let .settings(section):
            apply(.openSettings(section: section))
        case let .action(action):
            switch action {
            case .addWallet: apply(.addWallet)
            case .connectBTCPay: openConnectionSetup("btcpay")
            case .importBTCPay:
                UserDefaults.standard.set(
                    ReportsImportsFileFormat.btcpay.rawValue,
                    forKey: KassiberHostNotification.pendingImportFormatDefaultsKey
                )
                shell.selection = .imports
                DispatchQueue.main.async {
                    NotificationCenter.default.post(
                        name: KassiberHostNotification.openImportFormat,
                        object: nil
                    )
                }
            case .processJournals: apply(.processJournals)
            }
        }
        model.query = ""
        presented = false
    }

    private func openConnectionSetup(_ mode: String) {
        guard shell.authenticationState.isUnlocked else { return }
        let defaults = UserDefaults.standard
        defaults.set(true, forKey: KassiberHostNotification.pendingAddWalletDefaultsKey)
        defaults.set(mode, forKey: KassiberHostNotification.pendingConnectionSetupModeDefaultsKey)
        shell.selection = .connections
        NSApplication.shared.activate(ignoringOtherApps: true)
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: KassiberHostNotification.addWallet, object: nil)
        }
    }

    private func apply(_ intent: NativeHostIntent) {
        applyNativeHostIntent(
            intent,
            to: shell,
            aiFeaturesEnabled: aiFeaturesEnabled,
            developerToolsEnabled: developerToolsEnabled
        )
    }
}
