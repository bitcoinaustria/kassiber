import Testing
@testable import KassiberViewModels

@Suite("Structured native navigation history")
struct NavigationHistoryTests {
    @Test("back and forward restore route detail state")
    func restoresStructuredLocations() {
        var history = NavigationHistory(current: NavigationLocation(screen: .dashboard))
        history.record(NavigationLocation(screen: .connections, connectionID: "wallet-a"))
        history.record(NavigationLocation(screen: .transactions, transactionID: "tx-1"))
        history.record(NavigationLocation(screen: .settings, settingsSection: "security"))
        history.record(NavigationLocation(screen: .birdsEye, birdsEyeWorkspaceID: "workspace-2"))

        #expect(history.goBack() == NavigationLocation(screen: .settings, settingsSection: "security"))
        #expect(history.goBack() == NavigationLocation(screen: .transactions, transactionID: "tx-1"))
        #expect(history.goForward() == NavigationLocation(screen: .settings, settingsSection: "security"))
        #expect(history.goForward() == NavigationLocation(screen: .birdsEye, birdsEyeWorkspaceID: "workspace-2"))
    }

    @Test("same-screen detail changes are history entries")
    func sameScreenDetails() {
        var history = NavigationHistory(
            current: NavigationLocation(screen: .transactions, transactionID: "tx-1")
        )
        history.record(NavigationLocation(screen: .transactions, transactionID: "tx-2"))

        #expect(history.canGoBack)
        #expect(history.goBack()?.transactionID == "tx-1")
        #expect(history.goForward()?.transactionID == "tx-2")
    }

    @Test("new navigation clears forward state and scope reset clears both stacks")
    func branchAndScopeReset() {
        var history = NavigationHistory(current: NavigationLocation(screen: .dashboard), capacity: 2)
        history.record(NavigationLocation(screen: .reports))
        history.record(NavigationLocation(screen: .logs))
        history.record(NavigationLocation(screen: .settings, settingsSection: "privacy"))
        #expect(history.backStack.count == 2)
        _ = history.goBack()
        history.record(NavigationLocation(screen: .connections, connectionID: "wallet-b"))
        #expect(!history.canGoForward)

        history.reset(to: NavigationLocation(screen: .dashboard))
        #expect(!history.canGoBack)
        #expect(!history.canGoForward)
    }

    @Test("screen identity changes across each daemon data boundary")
    func screenContentIdentity() {
        let base = ScreenContentIdentity(projectID: "p1", dataRoot: "/a", profileID: "book-1")
        #expect(base != ScreenContentIdentity(projectID: "p2", dataRoot: "/a", profileID: "book-1"))
        #expect(base != ScreenContentIdentity(projectID: "p1", dataRoot: "/b", profileID: "book-1"))
        #expect(base != ScreenContentIdentity(projectID: "p1", dataRoot: "/a", profileID: "book-2"))
    }
}
