import SwiftUI
import KassiberDaemonKit
import KassiberViewModels

enum NativeAssistantPanelPosition: String, CaseIterable {
    case left
    case center
    case right
}

/// macOS-native counterpart to the web assistant dock. Left and right use an
/// adjustable inspector split; center uses a resizable bottom utility pane.
/// The chat model is owned by AppShellView, so minimizing or moving the panel
/// never discards an in-progress conversation.
struct AssistantUtilityPanel: View {
    let daemon: any DaemonClient
    let model: AIChatViewModel
    let minimize: () -> Void
    @Environment(\.kassiberNavigate) private var navigate

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Label(AppLocalization.string("assistantPanel.title"), systemImage: "sparkles")
                    .font(.headline)
                Spacer()
                Button {
                    navigate(.assistant)
                    minimize()
                } label: {
                    Image(systemName: "arrow.up.left.and.arrow.down.right")
                }.help(AppLocalization.string("assistantPanel.openFull"))
                Button(action: minimize) {
                    Image(systemName: "minus")
                }.help(AppLocalization.string("assistantPanel.minimize"))
            }
            .padding(.horizontal, 10).padding(.vertical, 7)
            .background(.bar)
            Divider()
            AIChatScreen(daemon: daemon, sharedModel: model, compact: true)
        }
        .background(.background)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(AppLocalization.string("assistantPanel.title"))
    }
}
