import AppKit
import SwiftUI
import KassiberViewModels

enum KassiberHostNotification {
    static let addWallet = Notification.Name("kassiber.native.add-wallet")
    static let pendingAddWalletDefaultsKey = "host.pendingAddWallet"
    static let pendingConnectionSetupModeDefaultsKey = "host.pendingConnectionSetupMode"
    static let pendingImportFormatDefaultsKey = "host.pendingImportFormat"
    static let openImportFormat = Notification.Name("kassiber.native.open-import-format")
    /// Settings scene can key its daemon-backed subtree by this value without
    /// sharing AppShellView's local BooksViewModel instance.
    static let contentIdentityDefaultsKey = "host.screenContentIdentity"
    static let settingsSectionDefaultsKey = "settings.selectedNativeSection"
    static let showSearch = Notification.Name("kassiber.native.show-search")
    static let navigateBack = Notification.Name("kassiber.native.navigate-back")
    static let navigateForward = Notification.Name("kassiber.native.navigate-forward")
}

@MainActor
func applyNativeHostIntent(
    _ intent: NativeHostIntent,
    to shell: AppShellViewModel,
    aiFeaturesEnabled: Bool,
    developerToolsEnabled: Bool
) {
    switch intent {
    case let .navigate(screen):
        if screen == .assistant, !aiFeaturesEnabled {
            openNativeSettings(section: "ai", shell: shell)
            return
        }
        if screen == .logs, !developerToolsEnabled {
            openNativeSettings(section: "developer", shell: shell)
            return
        }
        shell.selection = screen
        NSApplication.shared.activate(ignoringOtherApps: true)
    case let .openSettings(section):
        openNativeSettings(section: section, shell: shell)
    case .lockApp:
        guard shell.authenticationState.isUnlocked else { return }
        Task { await shell.lock() }
    case .addWallet:
        guard shell.authenticationState.isUnlocked else { return }
        UserDefaults.standard.set(true, forKey: KassiberHostNotification.pendingAddWalletDefaultsKey)
        shell.selection = .connections
        NSApplication.shared.activate(ignoringOtherApps: true)
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: KassiberHostNotification.addWallet, object: nil)
        }
    case .syncAllWallets:
        guard shell.authenticationState.isUnlocked else { return }
        Task { await shell.refreshCoordinator.run(.refresh) }
    case .processJournals:
        guard shell.authenticationState.isUnlocked else { return }
        Task { await shell.refreshCoordinator.run(.journals) }
    }
}

@MainActor
private func openNativeSettings(section: String?, shell: AppShellViewModel) {
    UserDefaults.standard.set(
        NativeHostIntent.nativeSettingsSection(for: section),
        forKey: KassiberHostNotification.settingsSectionDefaultsKey
    )
    shell.selection = .settings
    NSApplication.shared.activate(ignoringOtherApps: true)
    // SwiftUI owns the Settings scene; this is the documented AppKit responder
    // action used to surface that scene from native menu and deep-link intents.
    _ = NSApplication.shared.sendAction(
        Selector(("showSettingsWindow:")),
        to: nil,
        from: nil
    )
}

struct KassiberCommands: Commands {
    @Bindable var shell: AppShellViewModel
    @AppStorage("hideSensitive") private var hideSensitive = false
    @AppStorage("aiFeaturesEnabled") private var aiFeaturesEnabled = true
    @AppStorage("developerToolsEnabled") private var developerToolsEnabled = true
    @AppStorage("appearance.scale") private var appearanceScale = 0.9

    var body: some Commands {
        CommandGroup(replacing: .appSettings) {
            Button(AppLocalization.string("menu.settings")) {
                openSettings(nil)
            }
            .keyboardShortcut(",", modifiers: .command)
        }

        CommandGroup(after: .appInfo) {
            Button(AppLocalization.string("menu.checkUpdates")) {
                UpdaterScaffold.shared.checkForUpdates()
            }
            .disabled(!UpdaterScaffold.shared.isConfigured)
        }

        CommandGroup(after: .newItem) {
            Button(AppLocalization.string("menu.lock")) {
                apply(.lockApp)
            }
            .keyboardShortcut("l", modifiers: .command)
            .disabled(!workspaceEnabled || !shell.databaseEncrypted)
        }

        CommandGroup(after: .sidebar) {
            Button(AppLocalization.string("search.title")) {
                NotificationCenter.default.post(name: KassiberHostNotification.showSearch, object: nil)
            }
            .keyboardShortcut("k", modifiers: .command)
            .disabled(!workspaceEnabled)
            Divider()
            navigationButton(.dashboard, shortcut: "1")
            navigationButton(.transactions, shortcut: "2")
            navigationButton(.connections, shortcut: "3")
            navigationButton(.books, shortcut: "4")
            navigationButton(.reports, shortcut: "5")
            navigationButton(.sourceFunds, shortcut: "6")
            navigationButton(.journals, shortcut: "7")
            navigationButton(.quarantine, shortcut: "8")
            navigationButton(.assistant, shortcut: "9")
                .disabled(!workspaceEnabled || !aiFeaturesEnabled)
            Divider()
            Button(AppLocalization.string("menu.toggleSensitive")) {
                hideSensitive.toggle()
            }
            Divider()
            Button(AppLocalization.string("menu.smallerUI")) {
                appearanceScale = max(0.8, appearanceScale - 0.05)
            }
            .keyboardShortcut("-", modifiers: .command)
            .disabled(appearanceScale <= 0.8)
            Button(AppLocalization.string("menu.largerUI")) {
                appearanceScale = min(1.2, appearanceScale + 0.05)
            }
            .keyboardShortcut("=", modifiers: .command)
            .disabled(appearanceScale >= 1.2)
            Button(AppLocalization.string("menu.defaultUIScale")) {
                appearanceScale = 0.9
            }
            .keyboardShortcut("0", modifiers: .command)
            .disabled(abs(appearanceScale - 0.9) < 0.001)
        }

        CommandGroup(after: .pasteboard) {
            Button(AppLocalization.string("menu.back")) {
                NotificationCenter.default.post(name: KassiberHostNotification.navigateBack, object: nil)
            }
            .keyboardShortcut("[", modifiers: .command)
            .disabled(!workspaceEnabled)
            Button(AppLocalization.string("menu.forward")) {
                NotificationCenter.default.post(name: KassiberHostNotification.navigateForward, object: nil)
            }
            .keyboardShortcut("]", modifiers: .command)
            .disabled(!workspaceEnabled)
        }

        CommandMenu(AppLocalization.string("menu.workflows")) {
            Button(AppLocalization.string("menu.addWallet")) { apply(.addWallet) }
                .keyboardShortcut("a", modifiers: [.command, .shift])
                .disabled(!workspaceEnabled)
            Divider()
            Button(AppLocalization.string("menu.syncAll")) { apply(.syncAllWallets) }
                .keyboardShortcut("r", modifiers: .command)
                .disabled(!workspaceEnabled || shell.refreshCoordinator.isRunning)
            Button(AppLocalization.string("menu.processJournals")) { apply(.processJournals) }
                .keyboardShortcut("j", modifiers: [.command, .shift])
                .disabled(!workspaceEnabled || shell.refreshCoordinator.isRunning)
            Divider()
            Button(AppLocalization.string("menu.reportsExport")) { apply(.navigate(.reports)) }
                .keyboardShortcut("e", modifiers: [.command, .shift])
                .disabled(!workspaceEnabled)
            Button(AppLocalization.string("menu.connectionsImports")) { apply(.navigate(.connections)) }
                .disabled(!workspaceEnabled)
            Button(AppLocalization.string("menu.localDataBackup")) { openSettings("data") }
        }

        CommandMenu(AppLocalization.string("menu.settingsSections")) {
            Button(AppLocalization.string("settings.parity.general")) { openSettings(nil) }
            Divider()
            Button(AppLocalization.string("settings.parity.privacy")) { openSettings("privacy") }
            Button(AppLocalization.string("menu.display")) { openSettings("display") }
            Button(AppLocalization.string("settings.parity.security")) { openSettings("security") }
            Button(AppLocalization.string("settings.parity.bitcoin")) { openSettings("backends") }
            Button(AppLocalization.string("settings.parity.assistant")) { openSettings("ai") }
                .disabled(!aiFeaturesEnabled)
            Button(AppLocalization.string("settings.parity.data")) { openSettings("data") }
        }

        CommandGroup(replacing: .help) {
            Button(AppLocalization.string("menu.documentation")) {
                openURL("https://github.com/bitcoinaustria/kassiber#readme")
            }
            Button(AppLocalization.string("nav.logs")) { apply(.navigate(.logs)) }
                .disabled(!workspaceEnabled || !developerToolsEnabled)
            Divider()
            Button(AppLocalization.string("menu.reportIssue")) {
                openURL("https://github.com/bitcoinaustria/kassiber/issues")
            }
        }
    }

    private var workspaceEnabled: Bool { shell.authenticationState.isUnlocked }

    @ViewBuilder
    private func navigationButton(_ screen: AppScreen, shortcut: KeyEquivalent) -> some View {
        Button(AppLocalization.string(screen.localizationKey)) {
            apply(.navigate(screen))
        }
        .keyboardShortcut(shortcut, modifiers: .command)
        .disabled(!workspaceEnabled)
    }

    private func apply(_ intent: NativeHostIntent) {
        applyNativeHostIntent(
            intent,
            to: shell,
            aiFeaturesEnabled: aiFeaturesEnabled,
            developerToolsEnabled: developerToolsEnabled
        )
    }

    private func openSettings(_ section: String?) {
        apply(.openSettings(section: section))
    }

    private func openURL(_ value: String) {
        guard let url = URL(string: value) else { return }
        NSWorkspace.shared.open(url)
    }
}
