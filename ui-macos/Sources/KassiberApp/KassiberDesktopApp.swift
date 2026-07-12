import SwiftUI
import AppKit
import Darwin
import KassiberDaemonKit
import KassiberViewModels

@main
struct KassiberDesktopApp: App {
    @State private var shell: AppShellViewModel
    @AppStorage(KassiberHostNotification.contentIdentityDefaultsKey)
    private var settingsContentIdentity = ""

    init() {
        let environment = ProcessInfo.processInfo.environment
        if environment["KASSIBER_PREVIEW_CAPTURE_BACKEND"] == "appkit" {
            // Backing-store verification is intentionally pinned to Aqua so
            // NSVisualEffectView materials have a deterministic opaque source
            // instead of inheriting the interactive user's system appearance.
            UserDefaults.standard.set("light", forKey: "appearance.theme")
        }
        let repoRoot: URL
        if let configured = environment["KASSIBER_REPO_ROOT"], !configured.isEmpty {
            repoRoot = URL(fileURLWithPath: configured, isDirectory: true)
        } else {
            repoRoot = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent() // KassiberApp
                .deletingLastPathComponent() // Sources
                .deletingLastPathComponent() // ui-macos
                .deletingLastPathComponent() // repository root
        }
        let resources = Bundle.main.resourceURL
        if let exitCode = NativeCLIForwarder.runIfRequested(
            resourceURL: resources,
            repositoryRoot: repoRoot
        ) {
            Darwin.exit(exitCode)
        }
        let bundledSidecar = resources?.appending(path: "kassiber-sidecar")
        let launchConfiguration: DaemonLaunchConfiguration
        if let resources, let bundledSidecar,
           FileManager.default.isExecutableFile(atPath: bundledSidecar.path) {
            launchConfiguration = .bundled(sidecar: bundledSidecar, resources: resources)
        } else {
            launchConfiguration = .repositoryDevelopment(repoRoot: repoRoot)
        }
        let daemon = ProcessDaemonSupervisor(
            configuration: launchConfiguration,
            nativeSecretStore: MacOSNativeServices.keychain
        )
        let shell = AppShellViewModel(
            daemon: daemon,
            touchIDManager: MacOSNativeServices.touchID,
            touchIDAccount: canonicalTouchIDAccount(for: nil),
            presentation: ShellPresentationStrings(
                unlockPrompt: AppLocalization.string("security.unlockPrompt"),
                unlockProjectFormat: AppLocalization.string("security.unlockProject %@"),
                touchIDReason: AppLocalization.string("security.touchIDReason"),
                touchIDMissingPassphrase: AppLocalization.string("security.touchIDMissingPassphrase"),
                locked: AppLocalization.string("security.locked"),
                idleLocked: AppLocalization.string("security.idleLocked"),
                authenticationFailed: AppLocalization.string("security.authenticationFailed"),
                touchIDFailed: AppLocalization.string("security.touchIDFailed"),
                lockFailed: AppLocalization.string("security.lockFailed"),
                forgetTouchIDFailed: AppLocalization.string("security.forgetTouchIDFailed"),
                importUnsupportedTransport: AppLocalization.string("projects.importUnsupportedTransport"),
                importedProjectReopenFailedFormat: AppLocalization.string("projects.importReopenFailed %@")
            )
        )
        if let previewScreen = environment["KASSIBER_PREVIEW_SCREEN"],
           let selection = AppScreen(rawValue: previewScreen) {
            shell.selection = selection
        }
        _shell = State(initialValue: shell)
        if let iconURL = Bundle.module.url(forResource: "AppIcon-1024", withExtension: "png") {
            NSApplication.shared.applicationIconImage = NSImage(contentsOf: iconURL)
        }
    }

    var body: some Scene {
        WindowGroup {
            AppShellView(model: shell)
                .tint(.kassiberAccent)
                .environment(\.locale, AppLocalization.locale)
                .frame(minWidth: 980, minHeight: 700)
        }
        .defaultSize(width: 1_760, height: 1_160)
        .defaultLaunchBehavior(.presented)
        .commands {
            KassiberCommands(shell: shell)
        }

        Settings {
            SecureNativeSettingsScene(shell: shell)
                .id(settingsContentIdentity)
                .frame(minWidth: 940, idealWidth: 1_040, minHeight: 650, idealHeight: 740)
        }
    }
}
