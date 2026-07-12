import AppKit
import SwiftUI
import UniformTypeIdentifiers
import KassiberViewModels

typealias KassiberNavigateAction = @MainActor @Sendable (AppScreen) -> Void

private struct KassiberNavigateKey: EnvironmentKey {
    static let defaultValue: KassiberNavigateAction = { _ in }
}

extension EnvironmentValues {
    var kassiberNavigate: KassiberNavigateAction {
        get { self[KassiberNavigateKey.self] }
        set { self[KassiberNavigateKey.self] = newValue }
    }
}

/// Native equivalents of Tauri's file-picker and clipboard helpers. Sensitive
/// values are cleared only when the clipboard still contains the value copied
/// by Kassiber, so newer user content is never overwritten.
@MainActor
enum NativeAffordances {
    static func copy(_ value: String) {
        guard !value.isEmpty else { return }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(value, forType: .string)
        let copiedChangeCount = pasteboard.changeCount
        guard clearClipboardEnabled else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 30) {
            let current = NSPasteboard.general
            guard current.changeCount == copiedChangeCount,
                  current.string(forType: .string) == value else { return }
            current.clearContents()
        }
    }

    static func chooseFile(types: [UTType] = [.data], title: String? = nil) -> URL? {
        let panel = NSOpenPanel()
        panel.title = title ?? ""
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = types
        return panel.runModal() == .OK ? panel.url : nil
    }

    static func chooseDirectory(title: String? = nil) -> URL? {
        let panel = NSOpenPanel()
        panel.title = title ?? ""
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        return panel.runModal() == .OK ? panel.url : nil
    }

    static func saveCopy(
        source: URL,
        suggestedFilename: String,
        types: [UTType] = [.data],
        title: String? = nil
    ) -> URL? {
        let panel = NSSavePanel()
        panel.title = title ?? ""
        panel.nameFieldStringValue = suggestedFilename
        panel.allowedContentTypes = types
        guard panel.runModal() == .OK, let destination = panel.url else { return nil }
        do {
            if FileManager.default.fileExists(atPath: destination.path) {
                try FileManager.default.removeItem(at: destination)
            }
            try FileManager.default.copyItem(at: source, to: destination)
            return destination
        } catch { return nil }
    }

    private static var clearClipboardEnabled: Bool {
        let defaults = UserDefaults.standard
        guard defaults.object(forKey: "clearClipboard") != nil else { return true }
        return defaults.bool(forKey: "clearClipboard")
    }
}
