import SwiftUI
import AppKit
import KassiberDaemonKit
import KassiberViewModels

private func localizedShellPresentation(_ value: String) -> String {
    let localized = AppLocalization.string(value)
    return localized == value ? value : localized
}

struct AppShellView: View {
    @Bindable var model: AppShellViewModel
    @State private var assistantPanelModel: AIChatViewModel
    @State private var booksModel: BooksViewModel
    @AppStorage("hideSensitive") private var hideSensitive = false
    @AppStorage("aiFeaturesEnabled") private var aiFeaturesEnabled = true
    @AppStorage("developerToolsEnabled") private var developerToolsEnabled = true
    // The Tauri client deliberately starts in dark mode. Keep the native
    // client on the same default while still allowing the user to opt into
    // System or Light from Settings/the toolbar.
    @AppStorage("appearance.theme") private var appearanceTheme = "dark"
    @AppStorage("appearance.currency") private var appearanceCurrency = "btc"
    @AppStorage("appearance.scale") private var appearanceScale = 0.9
    @AppStorage("assistant.panel.autoHide") private var assistantPanelAutoHide = true
    @AppStorage("assistant.panel.position") private var assistantPanelPosition = "right"
    @AppStorage("assistant.panel.startMinimized") private var assistantPanelStartMinimized = false
    @AppStorage("security.autoLockWhenIdle") private var autoLockWhenIdle = false
    @AppStorage("security.idleMinutes") private var idleMinutes = 5
    @AppStorage("security.lockOnWindowClose") private var lockOnWindowClose = false
    @AppStorage("security.lockOnScreenLock") private var lockOnScreenLock = true
    @AppStorage("security.touchIDUnlock") private var touchIDUnlock = false
    @AppStorage("security.requirePassphraseOnLaunch") private var requirePassphraseOnLaunch = false
    @AppStorage("connections.selectedWalletID") private var selectedConnectionID = ""
    @AppStorage("transactions.selectedID") private var selectedTransactionID = ""
    @AppStorage(KassiberHostNotification.settingsSectionDefaultsKey) private var selectedSettingsSection = "general"
    @AppStorage("birdsEye.workspaceID") private var selectedBirdsEyeWorkspaceID = ""
    @State private var activityMonitor: Any?
    @State private var createProjectOpen = false
    @State private var projectHostError: String?
    @State private var assistantPanelPresented = false
    @State private var launchRequiresManualPassphrase = false
    @State private var navigationHistory: NavigationHistory
    @State private var isRestoringHistory = false
    private let idleTimer = Timer.publish(every: 30, on: .main, in: .common).autoconnect()

    init(model: AppShellViewModel) {
        self.model = model
        _assistantPanelModel = State(initialValue: AIChatViewModel(daemon: model.daemon))
        _booksModel = State(initialValue: BooksViewModel(daemon: model.daemon))
        let defaults = UserDefaults.standard
        let initialLocation = NavigationLocation(
            screen: model.selection,
            connectionID: model.selection == .connections
                ? defaults.string(forKey: "connections.selectedWalletID") : nil,
            transactionID: model.selection == .transactions
                ? defaults.string(forKey: "transactions.selectedID") : nil,
            settingsSection: model.selection == .settings
                ? defaults.string(forKey: KassiberHostNotification.settingsSectionDefaultsKey) : nil,
            birdsEyeWorkspaceID: model.selection == .birdsEye
                ? defaults.string(forKey: "birdsEye.workspaceID") : nil
        )
        _navigationHistory = State(initialValue: NavigationHistory(current: initialLocation))
        _launchRequiresManualPassphrase = State(
            initialValue: UserDefaults.standard.bool(
                forKey: "security.requirePassphraseOnLaunch"
            )
        )
    }

    var body: some View {
        Group {
            switch model.authenticationState {
            case .checking:
                ProgressView(AppLocalization.string("security.checkingDatabase"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            case .unlocked:
                authenticatedShell
            case let .locked(reason):
                DatabaseLockScreen(
                    model: model,
                    reason: reason,
                    touchIDUnlock: $touchIDUnlock,
                    onCreateProject: { createProjectOpen = true },
                    onImportProject: importProject
                )
            case let .failed(message):
                ContentUnavailableView {
                    Label(AppLocalization.string("security.startupFailed"), systemImage: "exclamationmark.triangle")
                } description: {
                    Text(AppLocalization.error(message))
                } actions: {
                    Button(AppLocalization.string("action.retry")) {
                        Task { await model.bootstrapAuthentication(attemptTouchID: false) }
                    }
                }
            }
        }
        .preferredColorScheme(preferredColorScheme)
        .dynamicTypeSize(dynamicTypeSize)
        .controlSize(nativeControlSize)
        .background(Color.kassiberCanvas.ignoresSafeArea())
        .task {
            // Establish the process-global AI gate before authentication can
            // reveal an Assistant route that immediately loads models.
            await model.setAIFeaturesEnabled(aiFeaturesEnabled)
            launchRequiresManualPassphrase = requirePassphraseOnLaunch
            await model.bootstrapAuthentication(
                attemptTouchID: touchIDUnlock && !launchRequiresManualPassphrase
            )
            if model.authenticationState.isUnlocked {
                launchRequiresManualPassphrase = false
            }
        }
        .task { await model.monitorAuthenticationEvents() }
        .onReceive(idleTimer) { date in
            Task { await model.lockIfIdle(enabled: autoLockWhenIdle, minutes: idleMinutes, now: date) }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            guard !launchRequiresManualPassphrase else { return }
            Task { await model.attemptTouchIDUnlockIfConfigured(enabled: touchIDUnlock) }
        }
        .onReceive(NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.sessionDidResignActiveNotification)) { _ in
            if lockOnScreenLock { Task { await model.lock(reason: AppLocalization.string("security.screenLocked")) } }
        }
        .onReceive(NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.screensDidSleepNotification)) { _ in
            if lockOnScreenLock { Task { await model.lock(reason: AppLocalization.string("security.screenLocked")) } }
        }
        .onAppear {
            installActivityMonitor()
            NSApplication.shared.activate(ignoringOtherApps: true)
        }
        .onDisappear {
            removeActivityMonitor()
            if lockOnWindowClose { Task { await model.lock(reason: AppLocalization.string("security.windowClosedLock")) } }
        }
        .onOpenURL { url in
            guard let intent = NativeHostIntent.parse(url) else { return }
            applyNativeHostIntent(
                intent,
                to: model,
                aiFeaturesEnabled: aiFeaturesEnabled,
                developerToolsEnabled: developerToolsEnabled
            )
        }
        .onReceive(NotificationCenter.default.publisher(for: KassiberHostNotification.navigateBack)) { _ in
            navigateBack()
        }
        .onReceive(NotificationCenter.default.publisher(for: KassiberHostNotification.navigateForward)) { _ in
            navigateForward()
        }
        .onChange(of: model.authenticationState) { _, state in
            if state.isUnlocked {
                launchRequiresManualPassphrase = false
            } else {
                assistantPanelPresented = false
                Task { await assistantPanelModel.clearForAppLock() }
            }
        }
        .sheet(isPresented: $createProjectOpen) {
            CreateProjectSheet(model: model, touchIDUnlock: $touchIDUnlock)
        }
        .alert(
            AppLocalization.string("projects.importFailed"),
            isPresented: Binding(
                get: { projectHostError != nil },
                set: { if !$0 { projectHostError = nil } }
            )
        ) {
            Button(AppLocalization.string("action.ok"), role: .cancel) {}
        } message: {
            Text(projectHostError ?? "")
        }
    }

    private var authenticatedShell: some View {
        let shell = NavigationSplitView {
            List(selection: $model.selection) {
                Section(AppLocalization.string("projects.bookSets")) {
                    ManagedProjectsMenu(
                        model: model,
                        onCreate: { createProjectOpen = true },
                        onImport: importProject
                    )
                }
                Section(AppLocalization.string("books.activeBook")) {
                    ActiveBookMenu(model: booksModel) { profileID in
                        Task { await booksModel.switchBook(profileID) }
                    }
                }
                Section(AppLocalization.string("section.main")) {
                    navigationRows([.dashboard, .transactions, .wallets, .reports])
                }
                Section(AppLocalization.string("section.review")) {
                    navigationRows([.journals, .quarantine, .swaps, .reconcile])
                }
                Section(AppLocalization.string("section.analysis")) {
                    navigationRows([.activity, .privacyMirror, .exitTax, .sourceFunds])
                }
                Section(AppLocalization.string("section.manage")) {
                    navigationRows([.books, .birdsEye, .connections, .imports, .egress])
                }
                Section {
                    navigationRows(utilityScreens)
                }
            }
            .navigationTitle("Kassiber")
            .navigationSplitViewColumnWidth(min: 220, ideal: 245, max: 290)
            .scrollContentBackground(.hidden)
            .background(.regularMaterial)
            .background(Color.kassiberSidebar)
            .safeAreaInset(edge: .bottom) {
                VStack(alignment: .leading, spacing: 8) {
                    if let error = model.actionableError {
                        Label(AppLocalization.error(error), systemImage: "exclamationmark.triangle")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(3)
                    }
                    NativeBuildIdentityFooter()
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.bar)
            }
        } detail: {
            assistantLayout {
                VStack(spacing: 0) {
                    NativeShellHeader(
                        booksModel: booksModel,
                        switchBook: { profileID in
                            Task { await booksModel.switchBook(profileID) }
                        },
                        canGoBack: navigationHistory.canGoBack,
                        canGoForward: navigationHistory.canGoForward,
                        navigateBack: navigateBack,
                        navigateForward: navigateForward,
                        openSearch: {
                            NotificationCenter.default.post(
                                name: KassiberHostNotification.showSearch,
                                object: nil
                            )
                        }
                    )
                    Divider()
                    ScreenHost(
                        screen: model.selection,
                        daemon: model.daemon,
                        assistantModel: assistantPanelModel
                    )
                    .id(screenContentIdentity)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
                .environment(\.kassiberHideSensitive, hideSensitive)
                .environment(\.kassiberDisplayCurrency, displayCurrency)
                .environment(\.kassiberNavigate, { model.selection = $0 })
        }
        .background(Color.kassiberCanvas.ignoresSafeArea())
        .toolbar {
            shellNavigationToolbar
            shellRefreshToolbar
            shellPrivacyToolbar
        }
        .overlay(alignment: .bottomTrailing) {
            VStack(alignment: .trailing, spacing: 8) {
                if let activity = model.primaryDaemonActivity {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(activity.detail.isEmpty ? activity.kind : activity.detail).font(.caption)
                            Text(activity.kind).font(.caption2.monospaced()).foregroundStyle(.secondary)
                        }
                    }
                    .padding(10)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
                    .shadow(radius: 4, y: 2)
                }
                if model.refreshCoordinator.mode != nil {
                    SyncProgressCard(
                        model: model.refreshCoordinator,
                        onNavigate: { model.selection = $0 }
                    )
                }
                NativeNotificationToastRail(store: model.notificationStore)
            }
            .padding(18)
        }
        return shell
        .task { await model.refreshReviewBadges() }
        .task {
            await booksModel.load()
            setRefreshBookIdentity()
        }
        .task {
            let events = await model.daemon.events()
            for await event in events {
                guard !Task.isCancelled else { return }
                await booksModel.handleHostEvent(event)
            }
        }
        .task { await model.routeFirstRunIfNeeded() }
        .task {
            await PreviewSnapshot.captureIfRequested { screen in
                model.selection = screen
            }
        }
        .onChange(of: aiFeaturesEnabled) { _, enabled in
            Task { await model.setAIFeaturesEnabled(enabled) }
            if !enabled {
                assistantPanelPresented = false
                if model.selection == .assistant { model.selection = .dashboard }
            }
        }
        .onChange(of: model.selection) { previous, selection in
            if !isRestoringHistory && previous != selection {
                navigationHistory.record(navigationLocation(for: selection))
            }
            if assistantPanelAutoHide && selection != .assistant
                && !assistantPanelModel.isStreaming && assistantPanelModel.pendingConsent == nil {
                assistantPanelPresented = false
            }
        }
        .onChange(of: selectedConnectionID) { _, _ in recordRouteStateChange(for: .connections) }
        .onChange(of: selectedTransactionID) { _, _ in recordRouteStateChange(for: .transactions) }
        .onChange(of: selectedSettingsSection) { _, _ in recordRouteStateChange(for: .settings) }
        .onChange(of: selectedBirdsEyeWorkspaceID) { _, _ in recordRouteStateChange(for: .birdsEye) }
        .onChange(of: screenContentIdentity) { _, _ in
            resetNavigationForContentScope()
            Task { await assistantPanelModel.clearForAppLock() }
        }
        .onChange(of: assistantPanelAutoHide) { _, autoHide in
            if !autoHide && !assistantPanelStartMinimized && model.selection != .assistant {
                assistantPanelPresented = true
            }
        }
        .onAppear {
            assistantPanelPresented = aiFeaturesEnabled
                && !assistantPanelAutoHide && !assistantPanelStartMinimized
        }
        .onChange(of: developerToolsEnabled) { _, enabled in
            if !enabled && model.selection == .logs { model.selection = .dashboard }
        }
        .modifier(GlobalSearchChromeModifier(
            shell: model,
            aiFeaturesEnabled: aiFeaturesEnabled,
            developerToolsEnabled: developerToolsEnabled,
            contentIdentity: screenContentIdentity
        ))
    }

    @ToolbarContentBuilder
    private var shellNavigationToolbar: some ToolbarContent {
        ToolbarItem(id: "kassiber.navigation.back", placement: .primaryAction) {
            Button(action: navigateBack) {
                Label(AppLocalization.string("menu.back"), systemImage: "chevron.left")
            }
            .disabled(!navigationHistory.canGoBack)
        }
        ToolbarItem(id: "kassiber.navigation.forward", placement: .primaryAction) {
            Button(action: navigateForward) {
                Label(AppLocalization.string("menu.forward"), systemImage: "chevron.right")
            }
            .disabled(!navigationHistory.canGoForward)
        }
        ToolbarItem(id: "kassiber.search", placement: .primaryAction) {
            Button {
                NotificationCenter.default.post(
                    name: KassiberHostNotification.showSearch,
                    object: nil
                )
            } label: {
                Label(AppLocalization.string("search.title"), systemImage: "magnifyingglass")
            }
            .help(AppLocalization.string("search.prompt"))
        }
        ToolbarItem(id: "kassiber.network-health", placement: .primaryAction) {
            GlobalNetworkHealthIndicator(
                daemon: model.daemon,
                identityEpoch: [model.dataRoot ?? "", booksModel.activeProfileID].joined(separator: "|")
            ) { section in
                applyNativeHostIntent(
                    .openSettings(section: section),
                    to: model,
                    aiFeaturesEnabled: aiFeaturesEnabled,
                    developerToolsEnabled: developerToolsEnabled
                )
            }
        }
    }

    @ToolbarContentBuilder
    private var shellRefreshToolbar: some ToolbarContent {
        ToolbarItem(id: "kassiber.refresh-control", placement: .primaryAction) {
            ControlGroup {
                Button {
                    Task { await model.refreshCoordinator.run(.refresh) }
                } label: {
                    Label(AppLocalization.string("sync.refreshBook"), systemImage: "arrow.clockwise")
                }
                Menu {
                    Button(AppLocalization.string("sync.reprocessJournals")) {
                        Task { await model.refreshCoordinator.run(.journals) }
                    }
                    Button(AppLocalization.string("sync.fullRescan")) {
                        Task { await model.refreshCoordinator.run(.fullRescan) }
                    }
                } label: {
                    Image(systemName: "chevron.down")
                }
                .menuStyle(.borderlessButton)
            }
            .disabled(model.refreshCoordinator.isRunning)
        }
        ToolbarItem(id: "kassiber.notifications", placement: .primaryAction) {
            NativeNotificationBell(
                store: model.notificationStore,
                onNavigate: { model.selection = $0 },
                onProcessJournals: {
                    Task { await model.refreshCoordinator.run(.journals) }
                },
                onRestoreBookRefresh: { model.refreshCoordinator.restore() }
            )
        }
        ToolbarItem(id: "kassiber.security.lock", placement: .primaryAction) {
            if model.databaseEncrypted {
                Button {
                    Task { await model.lock() }
                } label: {
                    Label(AppLocalization.string("security.lockNow"), systemImage: "lock")
                }
            }
        }
    }

    @ToolbarContentBuilder
    private var shellPrivacyToolbar: some ToolbarContent {
        ToolbarItem(id: "kassiber.privacy.sensitive", placement: .primaryAction) {
            Button {
                hideSensitive.toggle()
            } label: {
                Label(
                    hideSensitive ? AppLocalization.string("appearance.showSensitive") : AppLocalization.string("appearance.hideSensitive"),
                    systemImage: hideSensitive ? "eye" : "eye.slash"
                )
            }
        }
        ToolbarItem(id: "kassiber.appearance.theme", placement: .primaryAction) {
            Menu {
                Button(AppLocalization.string("appearance.theme.system")) { appearanceTheme = "system" }
                Button(AppLocalization.string("appearance.theme.light")) { appearanceTheme = "light" }
                Button(AppLocalization.string("appearance.theme.dark")) { appearanceTheme = "dark" }
            } label: {
                Label(
                    AppLocalization.string("appearance.theme.title"),
                    systemImage: appearanceTheme == "dark"
                        ? "moon"
                        : appearanceTheme == "light" ? "sun.max" : "circle.lefthalf.filled"
                )
            }
        }
        ToolbarItem(id: "kassiber.appearance.currency", placement: .primaryAction) {
            Button {
                appearanceCurrency = displayCurrency == .bitcoin ? "eur" : "btc"
            } label: {
                Text(displayCurrency == .bitcoin ? "₿" : "€").font(.headline)
            }
            .help(AppLocalization.string("appearance.denomination.title"))
        }
        ToolbarItem(id: "kassiber.assistant.panel", placement: .primaryAction) {
            if aiFeaturesEnabled && model.selection != .assistant {
                Button { assistantPanelPresented.toggle() } label: {
                    Label(
                        AppLocalization.string("assistantPanel.title"),
                        systemImage: assistantPanelPresented
                            ? "sparkles.rectangle.stack.fill" : "sparkles.rectangle.stack"
                    )
                }
                .help(
                    assistantPanelPresented
                        ? AppLocalization.string("assistantPanel.minimize")
                        : AppLocalization.string("assistantPanel.show")
                )
            }
        }
    }

    private func navigateBack() {
        guard let destination = navigationHistory.goBack() else { return }
        restoreNavigationLocation(destination)
    }

    private func navigateForward() {
        guard let destination = navigationHistory.goForward() else { return }
        restoreNavigationLocation(destination)
    }

    private func recordRouteStateChange(for screen: AppScreen) {
        guard !isRestoringHistory, model.selection == screen else { return }
        navigationHistory.record(navigationLocation(for: screen))
    }

    private func navigationLocation(for screen: AppScreen) -> NavigationLocation {
        NavigationLocation(
            screen: screen,
            connectionID: screen == .connections ? selectedConnectionID.nilIfEmpty : nil,
            transactionID: screen == .transactions ? selectedTransactionID.nilIfEmpty : nil,
            settingsSection: screen == .settings ? selectedSettingsSection.nilIfEmpty : nil,
            birdsEyeWorkspaceID: screen == .birdsEye ? selectedBirdsEyeWorkspaceID.nilIfEmpty : nil
        )
    }

    private func restoreNavigationLocation(_ destination: NavigationLocation) {
        isRestoringHistory = true
        switch destination.screen {
        case .connections:
            selectedConnectionID = destination.connectionID ?? ""
        case .transactions:
            selectedTransactionID = destination.transactionID ?? ""
        case .settings:
            selectedSettingsSection = destination.settingsSection ?? "general"
        case .birdsEye:
            selectedBirdsEyeWorkspaceID = destination.birdsEyeWorkspaceID ?? ""
        default:
            break
        }
        model.selection = destination.screen
        DispatchQueue.main.async { isRestoringHistory = false }
    }

    private var screenContentIdentity: ScreenContentIdentity {
        ScreenContentIdentity(
            projectID: model.selectedProjectID,
            dataRoot: model.dataRoot,
            profileID: booksModel.activeProfileID
        )
    }

    private func resetNavigationForContentScope() {
        isRestoringHistory = true
        selectedConnectionID = ""
        selectedTransactionID = ""
        selectedBirdsEyeWorkspaceID = ""
        let identity = screenContentIdentity
        UserDefaults.standard.set(
            [identity.projectID, identity.dataRoot, identity.profileID].joined(separator: "\u{1f}"),
            forKey: KassiberHostNotification.contentIdentityDefaultsKey
        )
        setRefreshBookIdentity()
        navigationHistory.reset(to: navigationLocation(for: model.selection))
        DispatchQueue.main.async { isRestoringHistory = false }
    }

    @ViewBuilder
    private func assistantLayout<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        let position = NativeAssistantPanelPosition(rawValue: assistantPanelPosition) ?? .right
        if aiFeaturesEnabled && model.selection != .assistant && assistantPanelPresented {
            switch position {
            case .left:
                HSplitView {
                    assistantPanel.frame(minWidth: 340, idealWidth: 420, maxWidth: 540)
                    content()
                }
            case .center:
                VSplitView {
                    content()
                    assistantPanel.frame(minHeight: 260, idealHeight: 340, maxHeight: 500)
                }
            case .right:
                HSplitView {
                    content()
                    assistantPanel.frame(minWidth: 340, idealWidth: 420, maxWidth: 540)
                }
            }
        } else {
            content()
        }
    }

    private var assistantPanel: some View {
        AssistantUtilityPanel(
            daemon: model.daemon,
            model: assistantPanelModel,
            minimize: { assistantPanelPresented = false }
        )
    }

    private var displayCurrency: KassiberDisplayCurrency {
        KassiberDisplayCurrency(rawValue: appearanceCurrency) ?? .bitcoin
    }

    private func setRefreshBookIdentity() {
        let identity = screenContentIdentity
        model.setActiveBookIdentity(
            identity.profileID.nilIfEmpty
                ?? identity.projectID.nilIfEmpty
                ?? identity.dataRoot.nilIfEmpty
        )
    }

    private var preferredColorScheme: ColorScheme? {
        switch appearanceTheme {
        case "light": .light
        case "dark": .dark
        default: nil
        }
    }

    private var dynamicTypeSize: DynamicTypeSize {
        switch appearanceScale {
        case ..<0.85: .small
        case ..<0.95: .medium
        case ..<1.1: .large
        default: .xLarge
        }
    }

    private var nativeControlSize: ControlSize {
        if appearanceScale < 0.85 { return .small }
        if appearanceScale > 1.05 { return .large }
        return .regular
    }

    private func installActivityMonitor() {
        guard activityMonitor == nil else { return }
        activityMonitor = NSEvent.addLocalMonitorForEvents(
            matching: [.keyDown, .leftMouseDown, .rightMouseDown, .otherMouseDown, .mouseMoved, .scrollWheel]
        ) { event in
            model.noteActivity()
            return event
        }
    }

    private func removeActivityMonitor() {
        guard let activityMonitor else { return }
        NSEvent.removeMonitor(activityMonitor)
        self.activityMonitor = nil
    }

    private func importProject() {
        guard let folder = NativeAffordances.chooseDirectory() else { return }
        do {
            let selection = try ImportedProjectInspector.inspect(folder)
            Task { await model.activateImportedProject(selection) }
        } catch {
            projectHostError = String(describing: error)
        }
    }

    private var utilityScreens: [AppScreen] {
        (aiFeaturesEnabled ? [.assistant] : [])
            + (developerToolsEnabled ? [.logs] : [])
            + [.settings]
    }

    @ViewBuilder
    private func navigationRows(_ screens: [AppScreen]) -> some View {
        ForEach(screens) { screen in
            HStack {
                Label {
                    Text(AppLocalization.string(screen.localizationKey))
                } icon: {
                    Image(systemName: screen.systemImage)
                }
                Spacer()
                if let count = model.reviewBadges.count(for: screen) {
                    Text(count, format: .number)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 1)
                        .background(.quaternary, in: Capsule())
                        .accessibilityLabel(
                            String(format: AppLocalization.string("badge.unresolved %lld"), count)
                        )
                }
            }
            .tag(screen)
        }
    }
}

/// The Tauri client keeps its navigation/search/book context in a persistent
/// content header. Native macOS still owns the title bar, but this companion
/// header keeps the two clients' working layout and affordances aligned.
private struct NativeShellHeader: View {
    @Bindable var booksModel: BooksViewModel
    let switchBook: (String) -> Void
    let canGoBack: Bool
    let canGoForward: Bool
    let navigateBack: () -> Void
    let navigateForward: () -> Void
    let openSearch: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Label {
                VStack(alignment: .leading, spacing: 1) {
                    Text("Kassiber").font(.headline)
                    if let activeBook = booksModel.activeBook {
                        Text(activeBook.name).font(.caption).foregroundStyle(.secondary)
                    }
                }
            } icon: {
                Image(systemName: "bitcoinsign.circle.fill")
                    .foregroundStyle(.tint)
            }
            .frame(minWidth: 150, alignment: .leading)

            Divider().frame(height: 24)

            Button(action: navigateBack) {
                Image(systemName: "chevron.left")
            }
            .buttonStyle(.borderless)
            .disabled(!canGoBack)
            .help(AppLocalization.string("menu.back"))
            Button(action: navigateForward) {
                Image(systemName: "chevron.right")
            }
            .buttonStyle(.borderless)
            .disabled(!canGoForward)
            .help(AppLocalization.string("menu.forward"))

            ActiveBookMenu(model: booksModel, switchBook: switchBook)

            Button(action: openSearch) {
                HStack(spacing: 7) {
                    Image(systemName: "magnifyingglass")
                    Text(AppLocalization.string("search.prompt"))
                        .foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Text("⌘K")
                        .font(.caption.monospaced())
                        .foregroundStyle(.tertiary)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .frame(maxWidth: 420)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .stroke(.separator.opacity(0.3), lineWidth: 0.75)
                }
            }
            .buttonStyle(.plain)

            Spacer(minLength: 8)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(.bar)
    }
}

private struct ActiveBookMenu: View {
    @Bindable var model: BooksViewModel
    let switchBook: (String) -> Void

    var body: some View {
        Menu {
            ForEach(model.workspaces) { workspace in
                Section(workspace.name) {
                    ForEach(workspace.books) { book in
                        Button {
                            switchBook(book.id)
                        } label: {
                            HStack {
                                Label(book.name, systemImage: "book.closed")
                                if book.id == model.activeProfileID || book.active {
                                    Image(systemName: "checkmark")
                                }
                            }
                        }
                        .disabled(book.id == model.activeProfileID || model.isWorking)
                    }
                }
            }
            Divider()
            Button {
                Task { await model.load() }
            } label: {
                Label(AppLocalization.string("action.refresh"), systemImage: "arrow.clockwise")
            }
        } label: {
            Label {
                VStack(alignment: .leading, spacing: 1) {
                    Text(model.activeBook?.name ?? AppLocalization.string("books.switchBook"))
                    if let workspace = model.workspaces.first(where: { $0.id == model.activeWorkspaceID }) {
                        Text(workspace.name).font(.caption).foregroundStyle(.secondary)
                    }
                }
            } icon: {
                Image(systemName: "book.closed.fill")
            }
        }
        .menuStyle(.borderlessButton)
        .disabled(model.workspaces.isEmpty || model.isWorking)
        .help(model.errorMessage.map(AppLocalization.error) ?? AppLocalization.string("books.switchBook"))
    }
}

private struct DatabaseLockScreen: View {
    @Bindable var model: AppShellViewModel
    let reason: String
    @Binding var touchIDUnlock: Bool
    let onCreateProject: () -> Void
    let onImportProject: () -> Void
    @State private var passphrase = ""
    @State private var rememberWithTouchID = false

    var body: some View {
        VStack(spacing: 18) {
            Image(systemName: "lock.shield.fill")
                .font(.system(size: 48, weight: .semibold))
                .foregroundStyle(.tint)
                .accessibilityHidden(true)
            Text(AppLocalization.string("security.unlockKassiber"))
                .font(.largeTitle.bold())
            Text(localizedShellPresentation(reason))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            ManagedProjectsMenu(
                model: model,
                onCreate: onCreateProject,
                onImport: onImportProject
            ).frame(width: 340)
            SecureField(AppLocalization.string("security.passphrase"), text: $passphrase)
                .textFieldStyle(.roundedBorder)
                .frame(width: 340)
                .onSubmit { unlock() }
            if model.touchIDStatus.available {
                Toggle(AppLocalization.string("security.rememberTouchID"), isOn: $rememberWithTouchID)
                    .disabled(model.touchIDStatus.configured)
                    .frame(width: 340, alignment: .leading)
            }
            HStack {
                if model.touchIDStatus.configured && touchIDUnlock {
                    Button {
                        Task { await model.unlockWithTouchID() }
                    } label: {
                        Label(AppLocalization.string("security.unlockTouchID"), systemImage: "touchid")
                    }
                }
                Button(AppLocalization.string("security.unlock")) { unlock() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(passphrase.isEmpty || model.isAuthenticating)
            }
            if model.isAuthenticating { ProgressView().controlSize(.small) }
            if let error = model.authenticationError {
                Text(localizedShellPresentation(error)).font(.caption).foregroundStyle(.red).multilineTextAlignment(.center)
            } else if let reason = model.touchIDStatus.reason, !model.touchIDStatus.available {
                Text(reason).font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.center)
            }
        }
        .padding(48)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.kassiberCanvas)
    }

    private func unlock() {
        let submitted = passphrase
        Task {
            await model.unlock(passphrase: submitted, rememberWithTouchID: rememberWithTouchID)
            if model.authenticationState.isUnlocked {
                passphrase = ""
                if rememberWithTouchID { touchIDUnlock = true }
            }
        }
    }
}

private struct ManagedProjectsMenu: View {
    @Bindable var model: AppShellViewModel
    let onCreate: () -> Void
    let onImport: () -> Void

    var body: some View {
        Menu {
            ForEach(model.projects) { project in
                Button {
                    Task { await model.selectProject(project) }
                } label: {
                    Label(project.name, systemImage: project.encrypted ? "lock" : "folder")
                }
                .disabled(project.id == model.selectedProjectID && model.importedProject == nil)
            }
            if !model.projects.isEmpty { Divider() }
            Button(AppLocalization.string("projects.create"), action: onCreate)
            Button(AppLocalization.string("projects.importFolder"), action: onImport)
            if model.importedProject != nil {
                Divider()
                Button(AppLocalization.string("projects.returnManaged")) {
                    Task { await model.clearImportedProject() }
                }
            }
        } label: {
            HStack {
                Label(currentLabel, systemImage: model.importedProject == nil ? "square.stack.3d.up" : "externaldrive")
                Spacer()
                if model.isLoadingProjects { ProgressView().controlSize(.small) }
                Image(systemName: "chevron.up.chevron.down").font(.caption)
            }
        }
        .menuStyle(.borderlessButton)
        if let error = model.projectError {
            Text(AppLocalization.error(error)).font(.caption).foregroundStyle(.red).lineLimit(2)
        }
    }

    private var currentLabel: String {
        if let imported = model.importedProject {
            return URL(fileURLWithPath: imported.stateRoot).lastPathComponent
        }
        return model.projects.first(where: { $0.id == model.selectedProjectID })?.name
            ?? AppLocalization.string("projects.choose")
    }
}

private struct CreateProjectSheet: View {
    @Bindable var model: AppShellViewModel
    @Binding var touchIDUnlock: Bool
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var encrypted = true
    @State private var passphrase = ""
    @State private var confirmation = ""
    @State private var rememberWithTouchID = true

    var body: some View {
        NavigationStack {
            Form {
                TextField(AppLocalization.string("projects.name"), text: $name)
                Toggle(AppLocalization.string("security.enableEncryption"), isOn: $encrypted)
                if encrypted {
                    SecureField(AppLocalization.string("security.newPassphrase"), text: $passphrase)
                    SecureField(AppLocalization.string("security.confirmPassphrase"), text: $confirmation)
                    if model.touchIDStatus.available {
                        Toggle(AppLocalization.string("security.rememberTouchID"), isOn: $rememberWithTouchID)
                    }
                }
                Text(AppLocalization.string("projects.localOnlyHint"))
                    .font(.caption).foregroundStyle(.secondary)
                if let error = model.projectError { Text(AppLocalization.error(error)).foregroundStyle(.red) }
            }
            .formStyle(.grouped)
            .navigationTitle(AppLocalization.string("projects.create"))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(AppLocalization.string("action.cancel")) { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(AppLocalization.string("projects.create")) {
                        Task {
                            await model.createProject(
                                name: name,
                                passphrase: encrypted ? passphrase : nil,
                                rememberWithTouchID: encrypted && rememberWithTouchID
                            )
                            if model.projectError == nil {
                                if rememberWithTouchID { touchIDUnlock = true }
                                dismiss()
                            }
                        }
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(
                        name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            || (encrypted && (passphrase.count < 10 || passphrase != confirmation))
                            || model.isLoadingProjects
                    )
                }
            }
        }.frame(width: 520, height: 420)
    }
}

private struct ScreenHost: View {
    let screen: AppScreen
    let daemon: any KassiberDaemonKit.DaemonClient
    let assistantModel: AIChatViewModel

    @ViewBuilder
    var body: some View {
        switch screen {
        case .dashboard:
            DashboardScreen(daemon: daemon)
        case .transactions:
            TransactionsScreen(daemon: daemon)
        case .wallets:
            WalletsScreen(daemon: daemon)
        case .reports:
            ReportsImportsReportsScreen(daemon: daemon)
        case .journals:
            JournalsScreen(daemon: daemon)
        case .quarantine:
            QuarantineScreen(daemon: daemon)
        case .swaps:
            SwapsScreen(daemon: daemon)
        case .reconcile:
            ReconcileScreen(daemon: daemon)
        case .exitTax:
            ExitTaxScreen(daemon: daemon)
        case .sourceFunds:
            ReportsImportsSourceFundsScreen(daemon: daemon)
        case .activity:
            ActivityScreen(daemon: daemon)
        case .privacyMirror:
            PrivacyMirrorScreen(daemon: daemon)
        case .egress:
            EgressScreen(daemon: daemon)
        case .books:
            BooksScreen(daemon: daemon)
        case .birdsEye:
            BirdsEyeScreen(daemon: daemon)
        case .connections:
            FullConnectionsScreen(daemon: daemon)
        case .imports:
            ReportsImportsImportScreen(daemon: daemon)
        case .assistant:
            AIChatScreen(daemon: daemon, sharedModel: assistantModel)
        case .logs:
            LogsScreen(daemon: daemon)
        case .settings:
            FullLayeredSettingsScreen(daemon: daemon)
        }
    }
}

struct NativeSettingsView: View {
    @AppStorage("language") private var language = "en"

    var body: some View {
        Form {
            Picker(selection: $language) {
                Text(AppLocalization.string("language.english")).tag("en")
                Text(AppLocalization.string("language.germanAT")).tag("de-AT")
            } label: {
                Text(AppLocalization.string("settings.language"))
            }
            Text(AppLocalization.string("settings.languageRestart"))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
        .padding()
        .navigationTitle(AppLocalization.string("nav.settings"))
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
