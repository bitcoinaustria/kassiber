import Foundation
import Observation
import KassiberDaemonKit

public enum AppScreen: String, CaseIterable, Hashable, Identifiable, Sendable {
    case dashboard
    case transactions
    case wallets
    case reports
    case journals
    case quarantine
    case swaps
    case reconcile
    case activity
    case privacyMirror
    case exitTax
    case sourceFunds
    case egress
    case books
    case birdsEye
    case connections
    case imports
    case assistant
    case logs
    case settings

    public var id: String { rawValue }
    public var localizationKey: String { "nav.\(rawValue)" }

    public var systemImage: String {
        switch self {
        case .dashboard: "chart.bar.xaxis"
        case .transactions: "list.bullet.rectangle"
        case .wallets: "wallet.bifold"
        case .reports: "doc.text"
        case .journals: "books.vertical"
        case .quarantine: "exclamationmark.triangle"
        case .swaps: "arrow.triangle.2.circlepath"
        case .reconcile: "checkmark.arrow.trianglehead.counterclockwise"
        case .activity: "clock.arrow.circlepath"
        case .privacyMirror: "hand.raised"
        case .exitTax: "airplane.departure"
        case .sourceFunds: "point.3.connected.trianglepath.dotted"
        case .egress: "network"
        case .books: "square.stack.3d.up"
        case .birdsEye: "rectangle.3.group"
        case .connections: "cable.connector"
        case .imports: "square.and.arrow.down"
        case .assistant: "sparkles"
        case .logs: "text.alignleft"
        case .settings: "gearshape"
        }
    }
}

public struct ReviewBadges: Equatable, Sendable {
    public var quarantine: Int
    public var journalsNeedProcessing: Bool
    public var swaps: Int?

    public init(quarantine: Int = 0, journalsNeedProcessing: Bool = false, swaps: Int? = nil) {
        self.quarantine = quarantine
        self.journalsNeedProcessing = journalsNeedProcessing
        self.swaps = swaps
    }

    public func count(for screen: AppScreen) -> Int? {
        switch screen {
        case .quarantine: quarantine > 0 ? quarantine : nil
        case .journals: journalsNeedProcessing ? 1 : nil
        case .swaps: (swaps ?? 0) > 0 ? swaps : nil
        default: nil
        }
    }
}

public enum ShellAuthenticationState: Equatable, Sendable {
    case checking
    case unlocked
    case locked(String)
    case failed(String)

    public var isUnlocked: Bool { self == .unlocked }
}

/// Localized host strings are injected by the app target so the
/// Foundation-only shell model never freezes English UI copy into auth state.
public struct ShellPresentationStrings: Equatable, Sendable {
    public let unlockPrompt: String
    public let unlockProjectFormat: String
    public let touchIDReason: String
    public let touchIDMissingPassphrase: String
    public let locked: String
    public let idleLocked: String
    public let authenticationFailed: String
    public let touchIDFailed: String
    public let lockFailed: String
    public let forgetTouchIDFailed: String
    public let importUnsupportedTransport: String
    public let importedProjectReopenFailedFormat: String

    public init(
        unlockPrompt: String,
        unlockProjectFormat: String,
        touchIDReason: String,
        touchIDMissingPassphrase: String,
        locked: String,
        idleLocked: String,
        authenticationFailed: String,
        touchIDFailed: String,
        lockFailed: String,
        forgetTouchIDFailed: String,
        importUnsupportedTransport: String,
        importedProjectReopenFailedFormat: String
    ) {
        self.unlockPrompt = unlockPrompt
        self.unlockProjectFormat = unlockProjectFormat
        self.touchIDReason = touchIDReason
        self.touchIDMissingPassphrase = touchIDMissingPassphrase
        self.locked = locked
        self.idleLocked = idleLocked
        self.authenticationFailed = authenticationFailed
        self.touchIDFailed = touchIDFailed
        self.lockFailed = lockFailed
        self.forgetTouchIDFailed = forgetTouchIDFailed
        self.importUnsupportedTransport = importUnsupportedTransport
        self.importedProjectReopenFailedFormat = importedProjectReopenFailedFormat
    }

    public static let english = ShellPresentationStrings(
        unlockPrompt: "Enter the SQLCipher database passphrase to unlock Kassiber.",
        unlockProjectFormat: "Enter the SQLCipher passphrase for %@.",
        touchIDReason: "Unlock Kassiber with Touch ID",
        touchIDMissingPassphrase: "No database passphrase is stored for Touch ID.",
        locked: "Kassiber is locked.",
        idleLocked: "Kassiber locked after being idle.",
        authenticationFailed: "Kassiber could not authenticate the database.",
        touchIDFailed: "Touch ID could not unlock Kassiber.",
        lockFailed: "Kassiber could not lock the database.",
        forgetTouchIDFailed: "Kassiber could not forget the Touch ID credential.",
        importUnsupportedTransport: "This desktop transport cannot activate an imported project folder.",
        importedProjectReopenFailedFormat: "The previously imported project could not be reopened: %@"
    )
}

public struct ManagedProjectRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let path: String
    public let dataRoot: String
    public let encrypted: Bool
    public let selected: Bool

    init?(_ value: JSONValue) {
        guard let row = value.objectValue,
              let id = row.string("id"), !id.isEmpty else { return nil }
        self.id = id
        name = row.string("name") ?? id
        path = row.string("path") ?? ""
        dataRoot = row.string("data_root") ?? ""
        encrypted = row.bool("encrypted") ?? false
        selected = row.bool("selected") ?? false
    }
}

public struct DaemonRequestActivity: Identifiable, Equatable, Sendable {
    public let id: String
    public let kind: String
    public var detail: String
    public let startedAt: Date
}

@MainActor
@Observable
public final class AppShellViewModel {
    public var selection: AppScreen = .dashboard
    public private(set) var reviewBadges = ReviewBadges()
    public private(set) var actionableError: String?
    public private(set) var isLoadingBadges = false
    public private(set) var authenticationState: ShellAuthenticationState = .checking
    public private(set) var authenticationError: String?
    public private(set) var isAuthenticating = false
    public private(set) var databaseEncrypted = false
    public private(set) var dataRoot: String?
    public private(set) var touchIDStatus = TouchIDPassphraseStatus(
        available: false,
        configured: false
    )
    public private(set) var projects: [ManagedProjectRow] = []
    public private(set) var selectedProjectID: String?
    public private(set) var pendingProjectID: String?
    public private(set) var projectError: String?
    public private(set) var isLoadingProjects = false
    public private(set) var importedProject: ImportedProjectSelection?
    public private(set) var activeDaemonRequests: [String: DaemonRequestActivity] = [:]
    public var primaryDaemonActivity: DaemonRequestActivity? {
        activeDaemonRequests.values.sorted { $0.startedAt < $1.startedAt }.first
    }

    public let daemon: any DaemonClient
    public let notificationStore: NativeNotificationStore
    public let refreshCoordinator: BookRefreshCoordinator
    private let touchIDManager: any TouchIDPassphraseManaging
    private let presentation: ShellPresentationStrings
    private let userDefaults: UserDefaults
    private var touchIDAccount: String
    private var lastActivity = Date()
    private var didRestoreImportedProject = false
    private var badgeRefreshRequested = false

    /// Terminal mutations that can change the Review rail's actionable
    /// quarantine, journal-staleness, or swap counts. The review-badge read
    /// itself is deliberately absent so its request-activity events cannot
    /// recursively trigger another read.
    public static let reviewBadgeInvalidatingKinds: Set<DaemonKind> = [
        .uiJournalsProcess,
        .uiTransactionsResolve, .uiTransactionsMetadataUpdate,
        .uiTransactionsHistoryRevert, .uiTransactionsCommercialContext,
        .uiMetadataBip329Import, .uiBtcpayProvenanceReview,
        .uiLoansLink, .uiLoansMark, .uiLoansUnmark,
        .uiTransfersPair, .uiTransfersUnpair, .uiTransfersUpdate,
        .uiTransfersBulkPair, .uiTransfersDismiss,
        .uiTransfersPayoutsCreate, .uiTransfersPayoutsDelete,
        .uiTransfersRulesCreate, .uiTransfersRulesDelete,
        .uiTransfersRulesSetEnabled, .uiTransfersRulesApply,
        .uiFreshnessRun, .uiWorkspaceFreshnessRun, .uiWalletsSync,
        .uiMaintenanceRun, .uiRatesKrakenCsvImport, .uiRatesRebuild,
        .uiWalletsCreate, .uiWalletsImportFile, .uiWalletsImportSamourai,
        .uiWalletsUpdate, .uiWalletsDelete,
        .uiOnboardingComplete,
        .uiProfilesCreate, .uiProfilesRename, .uiProfilesUpdate,
        .uiProfilesSwitch, .uiProfilesResetData,
        .uiWorkspaceCreate, .uiWorkspaceRename, .uiWorkspaceDelete,
        .uiProjectsCreate, .uiProjectsSelect,
        .uiSyncPull, .uiSyncJoin, .uiSyncConflictsResolve,
    ]

    public init(
        daemon: any DaemonClient,
        touchIDManager: any TouchIDPassphraseManaging = UnavailableTouchIDPassphraseManager(),
        touchIDAccount: String = "default",
        userDefaults: UserDefaults = .standard,
        presentation: ShellPresentationStrings = .english
    ) {
        let notifications = NativeNotificationStore(defaults: userDefaults)
        self.daemon = daemon
        self.notificationStore = notifications
        self.refreshCoordinator = BookRefreshCoordinator(
            daemon: daemon,
            notifications: notifications,
            defaults: userDefaults
        )
        self.touchIDManager = touchIDManager
        self.presentation = presentation
        self.touchIDAccount = touchIDAccount
        self.userDefaults = userDefaults
    }

    public func bootstrapAuthentication(attemptTouchID: Bool = true) async {
        authenticationState = .checking
        authenticationError = nil
        if !didRestoreImportedProject {
            didRestoreImportedProject = true
            await restoreImportedProjectIfNeeded()
        }
        await loadProjects()
        do {
            let result = try await daemon.invoke(.status, args: nil)
            if result.kind == "auth_required" {
                adoptAuthProject(result.data)
                authenticationState = .locked(authLabel(result))
                databaseEncrypted = true
                await refreshTouchIDStatus()
                if attemptTouchID && touchIDStatus.configured {
                    await unlockWithTouchID()
                }
                return
            }
            if let error = result.error {
                authenticationState = .failed(authErrorMessage(error))
                authenticationError = authErrorMessage(error)
                return
            }
            parseStatus(result.data?.objectValue)
            pendingProjectID = nil
            authenticationState = .unlocked
            await refreshTouchIDStatus()
        } catch {
            authenticationState = .failed(presentation.authenticationFailed)
            authenticationError = presentation.authenticationFailed
        }
    }

    public func unlock(passphrase: String, rememberWithTouchID: Bool = false) async {
        guard !isAuthenticating else { return }
        isAuthenticating = true
        defer { isAuthenticating = false }
        do {
            let (kind, args) = projectAwareUnlock(passphrase: passphrase)
            let result = try await daemon.invoke(kind, args: args)
            if result.kind == "auth_required" {
                adoptAuthProject(result.data)
                authenticationState = .locked(authLabel(result))
                authenticationError = authLabel(result)
                return
            }
            if let error = result.error {
                authenticationState = .locked(authErrorMessage(error))
                authenticationError = authErrorMessage(error)
                return
            }
            let root = result.data?.objectValue
            parseStatus(root?["status"]?.objectValue ?? root)
            pendingProjectID = nil
            authenticationState = .unlocked
            authenticationError = nil
            noteActivity()
            if rememberWithTouchID {
                try await touchIDManager.store(passphrase: passphrase, account: touchIDAccount)
            }
            await refreshTouchIDStatus()
            await loadProjects()
        } catch {
            authenticationState = .locked(presentation.unlockPrompt)
            authenticationError = presentation.authenticationFailed
        }
    }

    public func unlockWithTouchID() async {
        guard !isAuthenticating else { return }
        isAuthenticating = true
        defer { isAuthenticating = false }
        do {
            guard let passphrase = try await touchIDManager.retrieveAfterAuthentication(
                account: touchIDAccount,
                reason: presentation.touchIDReason
            ) else {
                authenticationError = presentation.touchIDMissingPassphrase
                await refreshTouchIDStatus()
                return
            }
            let (kind, args) = projectAwareUnlock(passphrase: passphrase)
            let result = try await daemon.invoke(kind, args: args)
            if let error = result.error {
                authenticationState = .locked(authErrorMessage(error))
                authenticationError = authErrorMessage(error)
                return
            }
            guard result.kind != "auth_required" else {
                authenticationState = .locked(authLabel(result))
                authenticationError = authLabel(result)
                return
            }
            let root = result.data?.objectValue
            parseStatus(root?["status"]?.objectValue ?? root)
            pendingProjectID = nil
            authenticationState = .unlocked
            authenticationError = nil
            noteActivity()
            await refreshTouchIDStatus()
            await loadProjects()
        } catch {
            authenticationError = presentation.touchIDFailed
        }
    }

    public func attemptTouchIDUnlockIfConfigured(enabled: Bool) async {
        guard enabled, case .locked = authenticationState else { return }
        await refreshTouchIDStatus()
        if touchIDStatus.configured { await unlockWithTouchID() }
    }

    public func lock(reason: String? = nil) async {
        guard databaseEncrypted, authenticationState.isUnlocked else { return }
        do {
            let result = try await daemon.invoke(.daemonLock, args: [:])
            if let error = result.error { throw error }
            authenticationState = .locked(reason ?? presentation.locked)
            authenticationError = nil
        } catch {
            authenticationError = presentation.lockFailed
        }
    }

    public func forgetTouchID() async {
        do {
            try await touchIDManager.delete(account: touchIDAccount)
            await refreshTouchIDStatus()
            authenticationError = nil
        } catch {
            authenticationError = presentation.forgetTouchIDFailed
        }
    }

    public func setAIFeaturesEnabled(_ enabled: Bool) async {
        guard let runtime = daemon as? any DaemonRuntimeControlling else { return }
        await runtime.setAIFeaturesEnabled(enabled)
    }

    public func monitorAuthenticationEvents() async {
        let stream = await daemon.events()
        for await event in stream {
            guard !Task.isCancelled else { return }
            await handleHostEvent(event)
        }
    }

    /// Reduces process-global supervisor events. Kept public so deterministic
    /// tests can exercise host behavior without racing stream subscription.
    public func handleHostEvent(_ event: DaemonRecord) async {
        switch event.kind {
            case "native.auth_required":
                databaseEncrypted = true
                adoptAuthProject(event.data)
                authenticationState = .locked(
                    authLabel(event)
                )
                await refreshTouchIDStatus()
            case "native.daemon_locked":
                if databaseEncrypted {
                    authenticationState = .locked(presentation.locked)
                    await refreshTouchIDStatus()
                }
            case "native.request.activity":
                _ = applyRequestActivity(event.data)
                let reduction = notificationStore.reduceHostEvent(event)
                if reduction.shouldRefreshReviewBadges {
                    await refreshReviewBadges()
                }
            default:
                break
        }
    }

    public func noteActivity() { lastActivity = Date() }

    /// Keeps first-sync state scoped to the active project/book without
    /// persisting a raw profile id or data-root path.
    public func setActiveBookIdentity(_ identity: String?) {
        refreshCoordinator.setBookIdentity(identity)
    }

    public func lockIfIdle(enabled: Bool, minutes: Int, now: Date = Date()) async {
        guard enabled, databaseEncrypted, authenticationState.isUnlocked else { return }
        let threshold = TimeInterval(max(1, minutes) * 60)
        if now.timeIntervalSince(lastActivity) >= threshold {
            await lock(reason: presentation.idleLocked)
        }
    }

    public func refreshTouchIDStatus() async {
        touchIDStatus = await touchIDManager.status(account: touchIDAccount)
    }

    public func loadProjects() async {
        isLoadingProjects = true
        defer { isLoadingProjects = false }
        do {
            let result = try await daemon.invoke(.uiProjectsList, args: nil)
            if let error = result.error { throw error }
            let data = result.data?.objectValue ?? [:]
            selectedProjectID = data.string("selected_project_id")
            projects = (data["projects"]?.arrayValue ?? []).compactMap(ManagedProjectRow.init)
            if importedProject == nil,
               let selected = projects.first(where: { $0.id == selectedProjectID || $0.selected }) {
                dataRoot = selected.dataRoot
                databaseEncrypted = selected.encrypted
                updateTouchIDAccount(selected.dataRoot)
                if !authenticationState.isUnlocked { pendingProjectID = selected.id }
            }
            projectError = nil
        } catch {
            projectError = String(describing: error)
        }
    }

    public func createProject(
        name: String,
        passphrase: String?,
        rememberWithTouchID: Bool = false
    ) async {
        guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        isLoadingProjects = true
        defer { isLoadingProjects = false }
        var args: [String: JSONValue] = ["name": .string(name), "select": true]
        if let passphrase, !passphrase.isEmpty {
            args["auth_response"] = ["passphrase_secret": .string(passphrase)]
        }
        do {
            if importedProject != nil,
               let switcher = daemon as? any DaemonDataRootSwitching {
                try await switcher.clearActivatedDataRoot()
                importedProject = nil
                clearPersistedImportedProject()
            }
            let result = try await daemon.invoke(.uiProjectsCreate, args: args)
            if let error = result.error { throw error }
            guard result.kind != "auth_required" else {
                adoptAuthProject(result.data)
                authenticationState = .locked(authLabel(result))
                return
            }
            if let project = result.data?.objectValue?["project"],
               let row = ManagedProjectRow(project) {
                selectedProjectID = row.id
                dataRoot = row.dataRoot
                databaseEncrypted = row.encrypted
                updateTouchIDAccount(row.dataRoot)
                if rememberWithTouchID, let passphrase, !passphrase.isEmpty {
                    try await touchIDManager.store(passphrase: passphrase, account: touchIDAccount)
                }
            }
            pendingProjectID = nil
            authenticationState = .unlocked
            authenticationError = nil
            projectError = nil
            await loadProjects()
        } catch {
            projectError = String(describing: error)
            authenticationState = .unlocked
        }
    }

    public func selectProject(_ project: ManagedProjectRow) async {
        guard project.id != selectedProjectID || !authenticationState.isUnlocked else { return }
        pendingProjectID = project.id
        databaseEncrypted = project.encrypted
        updateTouchIDAccount(project.dataRoot)
        authenticationState = .checking
        do {
            if importedProject != nil,
               let switcher = daemon as? any DaemonDataRootSwitching {
                try await switcher.clearActivatedDataRoot()
                importedProject = nil
                clearPersistedImportedProject()
            }
            let result = try await daemon.invoke(.uiProjectsSelect, args: ["project_id": .string(project.id)])
            if result.kind == "auth_required" {
                adoptAuthProject(result.data)
                authenticationState = .locked(authLabel(result))
                await refreshTouchIDStatus()
                return
            }
            if let error = result.error { throw error }
            let root = result.data?.objectValue
            parseStatus(root?["status"]?.objectValue ?? root)
            selectedProjectID = project.id
            pendingProjectID = nil
            authenticationState = .unlocked
            authenticationError = nil
            projectError = nil
            await refreshTouchIDStatus()
            await loadProjects()
        } catch {
            projectError = String(describing: error)
            authenticationState = .failed(presentation.authenticationFailed)
        }
    }

    public func activateImportedProject(_ selection: ImportedProjectSelection) async {
        guard let switcher = daemon as? any DaemonDataRootSwitching else {
            projectError = presentation.importUnsupportedTransport
            return
        }
        authenticationState = .checking
        pendingProjectID = nil
        dataRoot = selection.dataRoot
        databaseEncrypted = selection.encrypted
        updateTouchIDAccount(selection.dataRoot)
        do {
            try await switcher.activateDataRoot(selection.dataRoot)
            importedProject = selection
            didRestoreImportedProject = true
            persistImportedProject(selection)
            await bootstrapAuthentication(attemptTouchID: false)
            projectError = nil
        } catch {
            projectError = String(describing: error)
            authenticationState = .failed(presentation.authenticationFailed)
        }
    }

    public func clearImportedProject() async {
        guard let switcher = daemon as? any DaemonDataRootSwitching else { return }
        authenticationState = .checking
        do {
            try await switcher.clearActivatedDataRoot()
            importedProject = nil
            clearPersistedImportedProject()
            pendingProjectID = nil
            await bootstrapAuthentication(attemptTouchID: false)
            projectError = nil
        } catch {
            projectError = String(describing: error)
            authenticationState = .failed(presentation.authenticationFailed)
        }
    }

    private func parseStatus(_ data: [String: JSONValue]?) {
        guard let data else { return }
        databaseEncrypted = data.bool("database_encrypted") ?? databaseEncrypted
        if let root = data.string("data_root"), !root.isEmpty {
            dataRoot = root
            updateTouchIDAccount(root)
        }
        selectedProjectID = data.string("project_id") ?? selectedProjectID
    }

    private func projectAwareUnlock(passphrase: String) -> (DaemonKind, [String: JSONValue]) {
        var args: [String: JSONValue] = [
            "auth_response": ["passphrase_secret": .string(passphrase)],
        ]
        if let pendingProjectID {
            args["project_id"] = .string(pendingProjectID)
            return (.uiProjectsSelect, args)
        }
        return (.daemonUnlock, args)
    }

    private func adoptAuthProject(_ data: JSONValue?) {
        guard let project = data?.objectValue?["project"],
              let row = ManagedProjectRow(project) else { return }
        pendingProjectID = row.id
        databaseEncrypted = row.encrypted
        dataRoot = row.dataRoot
        updateTouchIDAccount(row.dataRoot)
    }

    private func updateTouchIDAccount(_ root: String) {
        guard !root.isEmpty else { return }
        touchIDAccount = URL(fileURLWithPath: root, isDirectory: true)
            .resolvingSymlinksInPath().standardizedFileURL.path
    }

    private func authLabel(_ record: DaemonRecord) -> String {
        if let pendingProjectID,
           let project = projects.first(where: { $0.id == pendingProjectID }) {
            return String(format: presentation.unlockProjectFormat, project.name)
        }
        if let projectValue = record.data?.objectValue?["project"],
           let project = ManagedProjectRow(projectValue) {
            return String(format: presentation.unlockProjectFormat, project.name)
        }
        guard let label = record.data?.objectValue?.string("label"), !label.isEmpty else {
            return presentation.unlockPrompt
        }
        if label.localizedCaseInsensitiveContains("SQLCipher"),
           label.localizedCaseInsensitiveContains("passphrase") {
            return presentation.unlockPrompt
        }
        return label
    }

    private func authErrorMessage(_ error: DaemonErrorPayload) -> String {
        _ = error
        return presentation.authenticationFailed
    }

    private func applyRequestActivity(_ value: JSONValue?) -> Bool {
        guard let data = value?.objectValue,
              let requestID = data.string("request_id"),
              let kind = data.string("kind"),
              let state = data.string("state") else { return false }
        switch state {
        case "started":
            activeDaemonRequests[requestID] = DaemonRequestActivity(
                id: requestID,
                kind: kind,
                detail: data.string("detail") ?? kind,
                startedAt: Date()
            )
        case "progress":
            guard var activity = activeDaemonRequests[requestID] else { return false }
            activity.detail = data.string("detail") ?? activity.detail
            activeDaemonRequests[requestID] = activity
        default:
            activeDaemonRequests.removeValue(forKey: requestID)
        }
        return state == "finished"
            && DaemonKind(rawValue: kind).map(Self.reviewBadgeInvalidatingKinds.contains) == true
    }

    private func restoreImportedProjectIfNeeded() async {
        guard let dataRoot = userDefaults.string(forKey: Self.importedDataRootKey),
              !dataRoot.isEmpty else { return }
        guard let switcher = daemon as? any DaemonDataRootSwitching else {
            clearPersistedImportedProject()
            return
        }
        do {
            let revalidated = try ImportedProjectInspector.inspect(
                URL(fileURLWithPath: dataRoot, isDirectory: true)
            )
            try await switcher.activateDataRoot(revalidated.dataRoot)
            importedProject = revalidated
            persistImportedProject(revalidated)
            self.dataRoot = revalidated.dataRoot
            databaseEncrypted = revalidated.encrypted
            pendingProjectID = nil
            updateTouchIDAccount(revalidated.dataRoot)
        } catch {
            clearPersistedImportedProject()
            importedProject = nil
            projectError = String(
                format: presentation.importedProjectReopenFailedFormat,
                String(describing: error)
            )
        }
    }

    private func persistImportedProject(_ selection: ImportedProjectSelection) {
        userDefaults.set(selection.stateRoot, forKey: Self.importedStateRootKey)
        userDefaults.set(selection.dataRoot, forKey: Self.importedDataRootKey)
        userDefaults.set(selection.database, forKey: Self.importedDatabaseKey)
        userDefaults.set(selection.encrypted, forKey: Self.importedEncryptedKey)
    }

    private func clearPersistedImportedProject() {
        for key in [
            Self.importedStateRootKey, Self.importedDataRootKey,
            Self.importedDatabaseKey, Self.importedEncryptedKey,
        ] { userDefaults.removeObject(forKey: key) }
    }

    private static let importedStateRootKey = "projects.imported.stateRoot"
    private static let importedDataRootKey = "projects.imported.dataRoot"
    private static let importedDatabaseKey = "projects.imported.database"
    private static let importedEncryptedKey = "projects.imported.encrypted"

    public func refreshReviewBadges() async {
        badgeRefreshRequested = true
        guard !isLoadingBadges else { return }
        isLoadingBadges = true
        defer { isLoadingBadges = false }
        while badgeRefreshRequested {
            badgeRefreshRequested = false
            do {
                let envelope = try await daemon.invoke(.uiReviewBadges, args: nil)
                if let error = envelope.error {
                    actionableError = error.message
                    continue
                }
                guard let data = envelope.data?.objectValue else {
                    actionableError = "The review summary had an unexpected format."
                    continue
                }
                reviewBadges = ReviewBadges(
                    quarantine: Int(data["quarantine"]?.intValue ?? 0),
                    journalsNeedProcessing: data["journals_needs_processing"]?.boolValue ?? false,
                    swaps: data["swaps"]?.intValue.map(Int.init)
                )
                notificationStore.setReviewBadges(reviewBadges)
                actionableError = nil
            } catch {
                actionableError = String(describing: error)
            }
        }
    }

    public func routeFirstRunIfNeeded() async {
        do {
            let result = try await daemon.invoke(.uiProfilesSnapshot, args: nil)
            guard result.error == nil else { return }
            if result.data?.objectValue?.objects("workspaces").isEmpty == true { selection = .books }
        } catch { /* Startup/auth routing remains owned by the daemon envelope. */ }
    }
}
