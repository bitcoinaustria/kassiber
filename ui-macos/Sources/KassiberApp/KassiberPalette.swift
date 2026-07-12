import AppKit
import SwiftUI

extension Color {
    /// Neutral graphite accent used by the native shell.  The native app is
    /// intentionally independent of the Bitcoin Austria brand palette.
    static let kassiberAccent = Color(nsColor: NSColor(name: nil) { appearance in
        let match = appearance.bestMatch(from: [.darkAqua, .aqua])
        if match == .darkAqua {
            return NSColor(srgbRed: 0.84, green: 0.87, blue: 0.91, alpha: 1) // #d6dee8
        }
        return NSColor(srgbRed: 0.24, green: 0.29, blue: 0.35, alpha: 1) // #3d4a59
    })

    /// Very dark graphite canvas for the dark appearance and a cool, light
    /// grey base that lets macOS materials read as glass in light mode.
    static let kassiberCanvas = Color(nsColor: NSColor(name: nil) { appearance in
        let match = appearance.bestMatch(from: [.darkAqua, .aqua])
        if match == .darkAqua {
            return NSColor(srgbRed: 0.025, green: 0.03, blue: 0.04, alpha: 1) // #070a0d
        }
        return NSColor(srgbRed: 0.93, green: 0.945, blue: 0.96, alpha: 1) // #edf1f5
    })

    static let kassiberSidebar = Color(nsColor: NSColor(name: nil) { appearance in
        let match = appearance.bestMatch(from: [.darkAqua, .aqua])
        if match == .darkAqua {
            return NSColor(srgbRed: 0.045, green: 0.055, blue: 0.07, alpha: 1) // #0b0e12
        }
        return NSColor(srgbRed: 0.88, green: 0.9, blue: 0.925, alpha: 1) // #e0e6ec
    })
}

private struct KassiberCardSurface: ViewModifier {
    let cornerRadius: CGFloat

    func body(content: Content) -> some View {
        content
            .background(
                .regularMaterial,
                in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .stroke(.separator.opacity(0.28), lineWidth: 0.75)
            }
    }
}

extension View {
    /// A restrained material surface for high-value content blocks. It keeps
    /// the light appearance glassy while remaining legible over the graphite
    /// dark canvas, without importing the old brand palette into the UI.
    func kassiberCardSurface(cornerRadius: CGFloat = 12) -> some View {
        modifier(KassiberCardSurface(cornerRadius: cornerRadius))
    }
}
