import Foundation

/// A complete browser-style location for the native shell. Detail selection
/// lives here instead of being inferred from the current sidebar row so Back
/// and Forward can restore the same workstation state.
public struct NavigationLocation: Equatable, Sendable {
    public var screen: AppScreen
    public var connectionID: String?
    public var transactionID: String?
    public var settingsSection: String?
    public var birdsEyeWorkspaceID: String?

    public init(
        screen: AppScreen,
        connectionID: String? = nil,
        transactionID: String? = nil,
        settingsSection: String? = nil,
        birdsEyeWorkspaceID: String? = nil
    ) {
        self.screen = screen
        self.connectionID = connectionID
        self.transactionID = transactionID
        self.settingsSection = settingsSection
        self.birdsEyeWorkspaceID = birdsEyeWorkspaceID
    }
}

/// Pure, deterministic history reducer used by AppShellView. Recording a
/// detail change on the same screen is intentional: selecting transaction B
/// after transaction A should make Back reopen A, just like the Tauri router.
public struct NavigationHistory: Equatable, Sendable {
    public private(set) var current: NavigationLocation
    public private(set) var backStack: [NavigationLocation] = []
    public private(set) var forwardStack: [NavigationLocation] = []
    public let capacity: Int

    public init(current: NavigationLocation, capacity: Int = 40) {
        self.current = current
        self.capacity = max(1, capacity)
    }

    public var canGoBack: Bool { !backStack.isEmpty }
    public var canGoForward: Bool { !forwardStack.isEmpty }

    public mutating func record(_ location: NavigationLocation) {
        guard location != current else { return }
        backStack.append(current)
        trimBackStack()
        current = location
        forwardStack.removeAll(keepingCapacity: true)
    }

    @discardableResult
    public mutating func goBack() -> NavigationLocation? {
        guard let destination = backStack.popLast() else { return nil }
        forwardStack.append(current)
        current = destination
        return destination
    }

    @discardableResult
    public mutating func goForward() -> NavigationLocation? {
        guard let destination = forwardStack.popLast() else { return nil }
        backStack.append(current)
        trimBackStack()
        current = destination
        return destination
    }

    /// Starts a fresh navigation epoch after a project or active-book switch.
    /// Locations from another database/profile must never be restored.
    public mutating func reset(to location: NavigationLocation) {
        current = location
        backStack.removeAll(keepingCapacity: true)
        forwardStack.removeAll(keepingCapacity: true)
    }

    private mutating func trimBackStack() {
        if backStack.count > capacity {
            backStack.removeFirst(backStack.count - capacity)
        }
    }
}

/// Stable identity for daemon-backed screen state. SwiftUI keys ScreenHost by
/// all three boundaries so a database or book switch reconstructs every child
/// view model instead of retaining rows from the previous scope.
public struct ScreenContentIdentity: Hashable, Sendable {
    public let projectID: String
    public let dataRoot: String
    public let profileID: String

    public init(projectID: String?, dataRoot: String?, profileID: String?) {
        self.projectID = projectID ?? ""
        self.dataRoot = dataRoot ?? ""
        self.profileID = profileID ?? ""
    }
}
