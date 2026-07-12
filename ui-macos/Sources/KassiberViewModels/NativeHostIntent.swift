import Foundation

/// The fixed public `kassiber://` contract shared with the Tauri host. Parsing
/// lives below SwiftUI/AppKit so cold-start and already-running deliveries use
/// exactly the same allowlisted behavior.
public enum NativeHostIntent: Equatable, Sendable {
    case navigate(AppScreen)
    case openSettings(section: String?)
    case lockApp
    case addWallet
    case syncAllWallets
    case processJournals

    private static let routes: [String: AppScreen] = [
        "overview": .dashboard,
        "transactions": .transactions,
        "connections": .connections,
        "books": .books,
        "reports": .reports,
        "source-of-funds": .sourceFunds,
        "journals": .journals,
        "tax-events": .journals,
        "quarantine": .quarantine,
        "assistant": .assistant,
        "logs": .logs,
        "diagnostics": .logs,
    ]

    public static let settingsSections: Set<String> = [
        "appearance", "privacy", "developer", "logs", "display",
        "explorer", "explorers", "bitcoin", "lightning", "liquid",
        "market", "desktop", "terminal", "security", "lock", "backends",
        "sync", "replication", "rates", "ai", "assistant", "data", "storage",
    ]

    public static func parse(_ url: URL) -> NativeHostIntent? {
        guard url.scheme?.lowercased() == "kassiber",
              let host = url.host?.lowercased() else { return nil }
        let segments = url.pathComponents
            .filter { $0 != "/" && !$0.isEmpty }
            .map { $0.lowercased() }
        let first = segments.first

        switch host {
        case "lock" where segments.isEmpty:
            return .lockApp
        case "settings":
            let section = first.flatMap { settingsSections.contains($0) ? $0 : nil }
            return .openSettings(section: section)
        case "workflow":
            switch first {
            case "add-wallet": return .addWallet
            case "sync-all", "sync": return .syncAllWallets
            case "process-journals": return .processJournals
            default: return nil
            }
        default:
            guard segments.isEmpty, let screen = routes[host] else { return nil }
            return .navigate(screen)
        }
    }

    /// Resolves the public Tauri settings slugs onto the native settings rail.
    /// Keep this mapping explicit: aliases are part of the external
    /// `kassiber://settings/<section>` contract once links ship in the wild.
    public static func nativeSettingsSection(for slug: String?) -> String {
        switch slug?.lowercased() {
        case "appearance", "display": "general"
        case "privacy": "privacy"
        case "developer", "logs": "general"
        case "security", "lock": "security"
        case "desktop", "terminal": "terminal"
        case "bitcoin", "backends", "explorer", "explorers": "bitcoin"
        case "lightning": "lightning"
        case "liquid": "liquid"
        case "market", "rates": "market"
        case "ai", "assistant": "assistant"
        case "sync", "replication": "replication"
        case "data", "storage": "data"
        default: "general"
        }
    }
}
