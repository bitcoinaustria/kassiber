import SwiftUI

private struct NativeSceneAppearanceModifier: ViewModifier {
    @AppStorage("appearance.theme") private var theme = "dark"
    @AppStorage("appearance.scale") private var scale = 0.9

    func body(content: Content) -> some View {
        content
            .preferredColorScheme(colorScheme)
            .dynamicTypeSize(typeSize)
            .controlSize(controlSize)
            .tint(.kassiberAccent)
            .background(Color.kassiberCanvas)
            .environment(\.locale, AppLocalization.locale)
    }

    private var colorScheme: ColorScheme? {
        switch theme {
        case "light": .light
        case "dark": .dark
        default: nil
        }
    }

    private var typeSize: DynamicTypeSize {
        switch scale {
        case ..<0.85: .small
        case ..<0.95: .medium
        case ..<1.1: .large
        default: .xLarge
        }
    }

    private var controlSize: ControlSize {
        if scale < 0.85 { return .small }
        if scale > 1.05 { return .large }
        return .regular
    }
}

extension View {
    func nativeKassiberSceneAppearance() -> some View {
        modifier(NativeSceneAppearanceModifier())
    }
}
