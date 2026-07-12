import SwiftUI

private struct KassiberHideSensitiveKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    var kassiberHideSensitive: Bool {
        get { self[KassiberHideSensitiveKey.self] }
        set { self[KassiberHideSensitiveKey.self] = newValue }
    }
}

private struct KassiberSensitiveModifier: ViewModifier {
    @Environment(\.kassiberHideSensitive) private var hidden

    @ViewBuilder
    func body(content: Content) -> some View {
        if hidden {
            content
                .blur(radius: 7)
                .textSelection(.disabled)
                .allowsHitTesting(false)
                .privacySensitive()
                .accessibilityHidden(true)
        } else {
            content
        }
    }
}

extension View {
    /// Applies the same on-screen privacy posture as Tauri's sensitive-value
    /// class while leaving surrounding labels and controls readable.
    func kassiberSensitive() -> some View { modifier(KassiberSensitiveModifier()) }
}
