import SwiftUI
import AppKit
import KassiberViewModels

struct SecureNativeSettingsScene: View {
    @Bindable var shell: AppShellViewModel
    @State private var identity: SettingsSceneIdentityViewModel

    init(shell: AppShellViewModel) {
        self.shell = shell
        _identity = State(initialValue: SettingsSceneIdentityViewModel(daemon: shell.daemon))
    }

    var body: some View {
        Group {
            if shell.authenticationState.isUnlocked {
                FullLayeredSettingsScreen(daemon: shell.daemon)
                    .id(settingsIdentity)
            } else {
                ContentUnavailableView {
                    Label(AppLocalization.string("security.unlockKassiber"), systemImage: "lock.shield")
                } description: {
                    Text(AppLocalization.string("security.settingsLocked"))
                } actions: {
                    Button(AppLocalization.string("security.openMainWindow")) {
                        let settingsWindow = NSApplication.shared.keyWindow
                        settingsWindow?.orderBack(nil)
                        NSApplication.shared.windows.first(where: {
                            $0 !== settingsWindow && $0.isVisible && $0.canBecomeMain
                        })?.makeKeyAndOrderFront(nil)
                        NSApplication.shared.activate(ignoringOtherApps: true)
                    }
                }
            }
        }
        .safeAreaInset(edge: .bottom) {
            HStack {
                NativeBuildIdentityFooter()
                Spacer()
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .background(.bar)
        }
        .nativeKassiberSceneAppearance()
        .task(id: settingsLoadIdentity) {
            guard shell.authenticationState.isUnlocked else { return }
            await identity.load()
        }
        .task { await identity.monitorContextChanges() }
    }

    private var projectIdentity: String {
        [
            shell.selectedProjectID ?? "",
            shell.dataRoot ?? "",
            shell.importedProject?.dataRoot ?? "",
        ].joined(separator: "|")
    }

    private var settingsIdentity: String {
        "\(projectIdentity)|\(identity.identityToken)"
    }

    private var settingsLoadIdentity: String {
        "\(projectIdentity)|unlocked=\(shell.authenticationState.isUnlocked)"
    }
}
