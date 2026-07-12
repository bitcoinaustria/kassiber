import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("App shell view model")
@MainActor
struct AppShellViewModelTests {
    @Test("loads actionable review badges from fake daemon")
    func loadsBadges() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiReviewBadges: [DaemonRecord(
                kind: "ui.review.badges",
                data: [
                    "quarantine": 4,
                    "journals_needs_processing": true,
                    "swaps": 2,
                ]
            )],
        ])
        let model = AppShellViewModel(daemon: daemon)
        await model.refreshReviewBadges()
        #expect(model.reviewBadges.count(for: .quarantine) == 4)
        #expect(model.reviewBadges.count(for: .journals) == 1)
        #expect(model.reviewBadges.count(for: .swaps) == 2)
        #expect(model.actionableError == nil)
    }

    @Test("suppresses zero-count reassurance badges")
    func noStandingGoodStatus() {
        let badges = ReviewBadges(quarantine: 0, journalsNeedProcessing: false, swaps: 0)
        #expect(badges.count(for: .quarantine) == nil)
        #expect(badges.count(for: .journals) == nil)
        #expect(badges.count(for: .swaps) == nil)
    }

    @Test("terminal mutations refresh review badges without badge-read recursion")
    func mutationActivityRefreshesBadges() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiReviewBadges: [DaemonRecord(
                kind: "ui.review.badges",
                data: ["quarantine": 3, "journals_needs_processing": false, "swaps": 1]
            )],
        ])
        let model = AppShellViewModel(daemon: daemon)
        await model.handleHostEvent(DaemonRecord(
            kind: "native.request.activity", event: true,
            data: [
                "request_id": "mutation-1", "kind": "ui.journals.process",
                "state": "finished",
            ]
        ))
        #expect(model.reviewBadges.count(for: .quarantine) == 3)
        #expect(model.reviewBadges.count(for: .swaps) == 1)
        #expect(await daemon.calls().filter { $0.kind == .uiReviewBadges }.count == 1)

        await model.handleHostEvent(DaemonRecord(
            kind: "native.request.activity", event: true,
            data: [
                "request_id": "badge-1", "kind": "ui.review.badges",
                "state": "finished",
            ]
        ))
        #expect(await daemon.calls().filter { $0.kind == .uiReviewBadges }.count == 1)
        #expect(!AppShellViewModel.reviewBadgeInvalidatingKinds.contains(.uiReviewBadges))
    }

    @Test("bootstrap can require manual passphrase even when Touch ID is configured")
    func manualLaunchGateSuppressesTouchID() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiProjectsList: [DaemonRecord(kind: "ui.projects.list", data: ["projects": []])],
            .status: [DaemonRecord(kind: "auth_required", data: ["label": "Unlock Kassiber"])],
        ])
        let touchID = RecordingTouchIDManager()
        let model = AppShellViewModel(daemon: daemon, touchIDManager: touchID)
        await model.bootstrapAuthentication(attemptTouchID: false)
        #expect(model.authenticationState == .locked("Unlock Kassiber"))
        #expect(await touchID.retrievalCount() == 0)
    }

    @Test("gates the whole shell while encrypted and unlocks the selected project")
    func encryptedStartupGate() async {
        let project: JSONValue = [
            "id": "private", "name": "Private books", "path": "/tmp/private",
            "data_root": "/tmp/private/data", "encrypted": true, "selected": true,
        ]
        let daemon = ScriptedDaemonClient(scripts: [
            .uiProjectsList: [DaemonRecord(kind: "ui.projects.list", data: [
                "selected_project_id": "private", "projects": .array([project]),
            ])],
            .status: [DaemonRecord(kind: "auth_required", data: [
                "scope": "unlock_database", "label": "Unlock Private books",
            ])],
            .uiProjectsSelect: [DaemonRecord(kind: "ui.projects.select", data: [
                "project": project,
                "status": [
                    "database_encrypted": true, "data_root": "/tmp/private/data",
                    "project_id": "private",
                ],
            ])],
        ])
        let model = AppShellViewModel(daemon: daemon)

        await model.bootstrapAuthentication(attemptTouchID: false)
        #expect(model.authenticationState == .locked("Enter the SQLCipher passphrase for Private books."))
        #expect(model.pendingProjectID == "private")
        await model.unlock(passphrase: "correct horse battery staple")
        #expect(model.authenticationState == .unlocked)
        #expect(model.databaseEncrypted)
        #expect(model.selectedProjectID == "private")
        let select = await daemon.calls().first { $0.kind == .uiProjectsSelect }
        #expect(select?.args?["project_id"]?.stringValue == "private")
        #expect(select?.args?["auth_response"]?["passphrase_secret"]?.stringValue == "correct horse battery staple")
    }

    @Test("persists, revalidates, restores, and clears an imported project selection")
    func persistedImportedProject() async throws {
        let root = FileManager.default.temporaryDirectory
            .appending(path: "kassiber-persisted-import-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try Data("encrypted-kassiber-database".utf8)
            .write(to: root.appending(path: "kassiber.sqlite3"))
        let selection = try ImportedProjectInspector.inspect(root)
        let suite = "AppShellViewModelTests.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defaults.removePersistentDomain(forName: suite)
        defer { defaults.removePersistentDomain(forName: suite) }
        let scripts: [DaemonKind: [DaemonRecord]] = [
            .uiProjectsList: [DaemonRecord(kind: "ui.projects.list", data: ["projects": []])],
            .status: [DaemonRecord(kind: "status", data: [
                "database_encrypted": true, "data_root": .string(selection.dataRoot),
            ])],
        ]

        let firstDaemon = ScriptedDaemonClient(scripts: scripts)
        let first = AppShellViewModel(daemon: firstDaemon, userDefaults: defaults)
        await first.activateImportedProject(selection)
        #expect(first.importedProject == selection)
        #expect(await firstDaemon.activatedDataRoots() == [selection.dataRoot])

        let relaunchedDaemon = ScriptedDaemonClient(scripts: scripts)
        let relaunched = AppShellViewModel(daemon: relaunchedDaemon, userDefaults: defaults)
        await relaunched.bootstrapAuthentication(attemptTouchID: false)
        #expect(relaunched.importedProject == selection)
        #expect(await relaunchedDaemon.activatedDataRoots() == [selection.dataRoot])

        await relaunched.clearImportedProject()
        #expect(relaunched.importedProject == nil)
        #expect(await relaunchedDaemon.clearActivatedDataRootCount() == 1)
        let afterClearDaemon = ScriptedDaemonClient(scripts: scripts)
        let afterClear = AppShellViewModel(daemon: afterClearDaemon, userDefaults: defaults)
        await afterClear.bootstrapAuthentication(attemptTouchID: false)
        #expect(await afterClearDaemon.activatedDataRoots().isEmpty)
    }
}

private actor RecordingTouchIDManager: TouchIDPassphraseManaging {
    private var retrievals = 0
    func status(account: String) async -> TouchIDPassphraseStatus {
        TouchIDPassphraseStatus(available: true, configured: true)
    }
    func store(passphrase: String, account: String) async throws {}
    func retrieveAfterAuthentication(account: String, reason: String) async throws -> String? {
        retrievals += 1
        return "remembered passphrase"
    }
    func delete(account: String) async throws {}
    func retrievalCount() -> Int { retrievals }
}
