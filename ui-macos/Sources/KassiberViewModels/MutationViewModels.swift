import Foundation
import Observation
import KassiberDaemonKit

public enum BookRefreshMode: String, Sendable {
    case refresh
    case journals
    case fullRescan
}

public enum BookRefreshOutcome: String, Equatable, Sendable {
    case idle
    case running
    case clean
    case reviewRequired
    case partial
    case failed
}

public struct BookRefreshMilestone: Identifiable, Equatable, Sendable {
    public let phase: String
    public let localizationKey: String
    public let fraction: Double
    public var id: String { phase }

    public init(phase: String, localizationKey: String, fraction: Double) {
        self.phase = phase
        self.localizationKey = localizationKey
        self.fraction = fraction
    }
}

@MainActor
@Observable
public final class BookRefreshCoordinator {
    public static let milestones: [BookRefreshMilestone] = [
        BookRefreshMilestone(phase: "discovery", localizationKey: "sync.phase.discovery", fraction: 0.12),
        BookRefreshMilestone(phase: "backend_fetch", localizationKey: "sync.phase.backend_fetch", fraction: 0.46),
        BookRefreshMilestone(phase: "decode_enrich", localizationKey: "sync.phase.decode_enrich", fraction: 0.62),
        BookRefreshMilestone(phase: "import", localizationKey: "sync.phase.import", fraction: 0.78),
        BookRefreshMilestone(phase: "rate_coverage", localizationKey: "sync.phase.rate_coverage", fraction: 0.86),
        BookRefreshMilestone(phase: "auto_pair", localizationKey: "sync.phase.auto_pair", fraction: 0.91),
        BookRefreshMilestone(phase: "journal_refresh", localizationKey: "sync.phase.journal_refresh", fraction: 0.94),
    ]

    public private(set) var mode: BookRefreshMode?
    public private(set) var phase = ""
    public private(set) var source = ""
    public private(set) var progress = 0.0
    public private(set) var detail = ""
    public private(set) var progressDetails: [String] = []
    public private(set) var isRunning = false
    public private(set) var isMinimized = false
    public private(set) var isFirstSync = false
    public private(set) var outcome = BookRefreshOutcome.idle
    public private(set) var terminalTarget: AppScreen?
    public private(set) var quarantineCount = 0
    public private(set) var transferReviewCount = 0
    public private(set) var sourceProblemCount = 0
    public private(set) var errorMessage: String?
    public private(set) var completedMessage: String?

    public var activeMilestoneIndex: Int {
        guard isRunning else { return Self.milestones.count }
        if phase == "discovery" { return 0 }
        return Self.milestones.firstIndex(where: { progress <= $0.fraction })
            ?? Self.milestones.count
    }

    private let daemon: any DaemonClient
    private let notifications: NativeNotificationStore?
    private let defaults: UserDefaults
    private let firstSyncPersistenceKey: String
    private var currentBookKey: String?
    private var firstSyncDone: Set<String>

    public init(
        daemon: any DaemonClient,
        notifications: NativeNotificationStore? = nil,
        defaults: UserDefaults = .standard,
        firstSyncPersistenceKey: String = "native.firstSyncDone.v1"
    ) {
        self.daemon = daemon
        self.notifications = notifications
        self.defaults = defaults
        self.firstSyncPersistenceKey = firstSyncPersistenceKey
        firstSyncDone = Set(defaults.stringArray(forKey: firstSyncPersistenceKey) ?? [])
    }

    /// Select the active book with a stable profile/project identity. The raw
    /// value is reduced to an opaque fingerprint before it enters UserDefaults.
    public func setBookIdentity(_ identity: String?) {
        currentBookKey = identity.map(Self.opaqueBookKey)
    }

    public func hasCompletedFirstSync(for identity: String) -> Bool {
        firstSyncDone.contains(Self.opaqueBookKey(identity))
    }

    public func run(_ requestedMode: BookRefreshMode) async {
        guard !isRunning else {
            isMinimized = false
            return
        }
        let bookKey = currentBookKey ?? Self.opaqueBookKey("active-book")
        mode = requestedMode
        phase = requestedMode == .journals ? "journal_refresh" : "discovery"
        source = ""
        progress = 0.03
        detail = ""
        progressDetails = []
        isRunning = true
        isMinimized = false
        isFirstSync = requestedMode != .journals && !firstSyncDone.contains(bookKey)
        outcome = .running
        terminalTarget = nil
        quarantineCount = 0
        transferReviewCount = 0
        sourceProblemCount = 0
        errorMessage = nil
        completedMessage = nil
        defer { isRunning = false }

        notifications?.updateProgress(
            dedupeKey: requestedMode == .journals ? "journal-refresh" : "book-refresh",
            value: progress,
            indeterminate: phase == "discovery",
            label: phase,
            titleKey: requestedMode == .journals
                ? "notifications.journal.running.title"
                : "notifications.bookRefresh.running.title",
            bodyKey: requestedMode == .journals
                ? "notifications.journal.running.body"
                : "notifications.bookRefresh.running.body"
        )

        do {
            var terminalData: JSONValue?
            if requestedMode == .journals {
                let terminal = try await daemon.invoke(.uiJournalsProcess, args: nil)
                try acceptTerminal(terminal)
                terminalData = terminal.data
            } else {
                let records = try await daemon.stream(
                    .uiFreshnessRun,
                    args: [
                        "all": .bool(true),
                        "journals": .bool(true),
                        "auto_pair": .bool(true),
                        "run": .bool(true),
                        "force_full": .bool(requestedMode == .fullRescan),
                    ]
                )
                for try await record in records {
                    if record.kind == "ui.freshness.run.progress" {
                        applyProgress(record.data)
                    } else if record.kind == DaemonKind.uiFreshnessRun.rawValue || record.error != nil {
                        try acceptTerminal(record)
                        terminalData = record.data
                    }
                }
            }
            progress = 1
            classifyTerminal(terminalData, mode: requestedMode, bookKey: bookKey)
        } catch {
            phase = "error"
            progress = 1
            outcome = .failed
            terminalTarget = .logs
            errorMessage = String(describing: error)
            notifications?.post(
                titleKey: requestedMode == .journals
                    ? "notifications.journal.failed.title"
                    : "notifications.bookRefresh.failed.title",
                bodyKey: requestedMode == .journals
                    ? "notifications.journal.failed.body"
                    : "notifications.bookRefresh.failed.body",
                tone: .error,
                category: .maintenance,
                dedupeKey: requestedMode == .journals ? "journal-refresh" : "book-refresh",
                target: .logs,
                action: .open,
                ephemeralDetail: errorMessage
            )
        }
    }

    public func minimize() { isMinimized = true }
    public func restore() { isMinimized = false }
    public func dismiss() {
        guard !isRunning else { return }
        completedMessage = nil
        errorMessage = nil
        mode = nil
        outcome = .idle
        terminalTarget = nil
    }

    private func acceptTerminal(_ record: DaemonRecord) throws {
        if let error = record.error { throw error }
        if record.kind == "auth_required" {
            throw DaemonClientError.transport("Database authentication is required.")
        }
    }

    private func applyProgress(_ data: JSONValue?) {
        guard let row = data?.objectValue else { return }
        phase = row.string("phase") ?? phase
        source = row.string("wallet", "source_label") ?? source
        let processed = row.double("processed")
        let total = row.double("total")
        let jobIndex = row.double("job_index")
        let jobTotal = row.double("job_total")
        let phaseFraction: [String: Double] = [
            "discovery": 0.12, "backend_fetch": 0.46, "decode_enrich": 0.62,
            "import": 0.78, "importing": 0.78, "rate_coverage": 0.86,
            "auto_pair": 0.91, "journal_refresh": 0.94, "done": 1,
        ]
        let withinJob: Double
        if phase == "rate_limited" {
            withinJob = progress
            if let attempt = row.int("retry_attempt"), let maximum = row.int("retry_max") {
                detail = "\(attempt) / \(maximum)"
            }
        } else if let processed, let total, total > 0 {
            withinJob = min(max(processed / total, 0), 1)
            detail = "\(Int(processed)) / \(Int(total))"
        } else {
            withinJob = phaseFraction[phase] ?? min(progress + 0.05, 0.9)
            detail = processed.map { String(Int($0)) } ?? ""
        }
        if let jobIndex, let jobTotal, jobTotal > 0 {
            progress = min(max((jobIndex - 1 + withinJob) / jobTotal, progress), 1)
        } else {
            progress = min(max(withinJob, progress), 1)
        }
        progressDetails = Self.progressDetailRows(row)
        notifications?.updateProgress(
            dedupeKey: mode == .journals ? "journal-refresh" : "book-refresh",
            value: progress,
            indeterminate: phase == "discovery",
            label: [source, phase, detail].filter { !$0.isEmpty }.joined(separator: " · "),
            titleKey: mode == .journals
                ? "notifications.journal.running.title"
                : "notifications.bookRefresh.running.title",
            bodyKey: mode == .journals
                ? "notifications.journal.running.body"
                : "notifications.bookRefresh.running.body"
        )
    }

    private func classifyTerminal(_ data: JSONValue?, mode: BookRefreshMode, bookKey: String) {
        let summary = Self.terminalSummary(data)
        quarantineCount = summary.quarantines
        transferReviewCount = summary.transfers
        sourceProblemCount = summary.problems
        progress = 1

        let dedupe = mode == .journals ? "journal-refresh" : "book-refresh"
        if summary.blocked {
            phase = "error"
            outcome = .failed
            terminalTarget = .logs
            completedMessage = "sync.outcome.blocked"
            notifications?.post(
                titleKey: mode == .journals
                    ? "notifications.journal.attention.title"
                    : "notifications.bookRefresh.attention.title",
                bodyKey: "notifications.bookRefresh.attention.body",
                tone: .error,
                category: .maintenance,
                dedupeKey: dedupe,
                target: .logs,
                action: .open,
                count: summary.problems
            )
            return
        }
        if summary.partial {
            phase = "error"
            outcome = .partial
            terminalTarget = .logs
            completedMessage = "sync.outcome.partial"
            notifications?.post(
                titleKey: "notifications.bookRefresh.partial.title",
                bodyKey: "notifications.bookRefresh.partial.body",
                tone: .warning,
                category: .maintenance,
                dedupeKey: dedupe,
                target: .logs,
                action: .open,
                count: max(1, summary.problems)
            )
            return
        }
        if summary.quarantines > 0 {
            phase = "done"
            outcome = .reviewRequired
            terminalTarget = .quarantine
            completedMessage = "sync.outcome.review"
            notifications?.post(
                titleKey: "notifications.bookRefresh.quarantine.title",
                bodyKey: "notifications.review.quarantine.body %lld",
                tone: .warning,
                category: .maintenance,
                dedupeKey: dedupe,
                target: .quarantine,
                action: .open,
                count: summary.quarantines
            )
            return
        }

        if mode != .journals {
            firstSyncDone.insert(bookKey)
            defaults.set(firstSyncDone.sorted(), forKey: firstSyncPersistenceKey)
        }
        phase = "done"
        if summary.transfers > 0 {
            outcome = .reviewRequired
            terminalTarget = .swaps
            completedMessage = "sync.outcome.review"
            notifications?.post(
                titleKey: "notifications.bookRefresh.swaps.title",
                bodyKey: "notifications.review.swaps.body %lld",
                tone: .warning,
                category: .maintenance,
                dedupeKey: dedupe,
                target: .swaps,
                action: .open,
                count: summary.transfers
            )
        } else {
            outcome = .clean
            terminalTarget = mode == .journals ? .journals : nil
            completedMessage = mode == .journals ? "sync.outcome.journalsDone" : "sync.outcome.clean"
            notifications?.post(
                titleKey: mode == .journals
                    ? "notifications.journal.finished.title"
                    : "notifications.bookRefresh.finished.title",
                bodyKey: mode == .journals
                    ? "notifications.journal.finished.body"
                    : "notifications.bookRefresh.finished.body",
                tone: .success,
                category: .maintenance,
                dedupeKey: dedupe,
                target: mode == .journals ? .journals : nil,
                action: mode == .journals ? .open : nil
            )
        }
    }

    private struct TerminalSummary {
        var blocked = false
        var partial = false
        var quarantines = 0
        var transfers = 0
        var problems = 0
    }

    private static func terminalSummary(_ data: JSONValue?) -> TerminalSummary {
        guard let root = data?.objectValue else { return TerminalSummary() }
        var summary = TerminalSummary()
        let completed = root.objects("completed")
        let sources = root.objects("sources")
        let aggregate = root["summary"]?.objectValue ?? [:]
        let failed = Int(aggregate.int("failed") ?? 0)
        let blocking = Int(aggregate.int("blocking_reports") ?? 0)
        let rateLimited = Int(aggregate.int("rate_limited") ?? 0)
        summary.blocked = failed > 0 || blocking > 0
        summary.partial = rateLimited > 0
        summary.problems = failed + blocking + rateLimited

        for source in sources {
            let status = source.string("status") ?? ""
            if status == "failed" || status == "blocking_reports" || source.bool("blocking_reports") == true {
                summary.blocked = true
                summary.problems += 1
            } else if ["partially_stale", "rate_limited"].contains(status) {
                summary.partial = true
                summary.problems += 1
            }
        }
        for job in completed {
            let status = job.string("status") ?? ""
            if ["error", "cancelled", "failed"].contains(status) {
                summary.blocked = true
                summary.problems += 1
            } else if ["rate_limited", "partially_stale"].contains(status) {
                summary.partial = true
                summary.problems += 1
            }
            guard job.string("job_type") == "journal_refresh",
                  let result = job["result"]?.objectValue else { continue }
            summary.quarantines += Int(max(
                result.int("quarantined") ?? 0,
                result.int("quarantine_count") ?? 0
            ))
            if let autoPair = result["auto_pair"]?.objectValue {
                if autoPair.bool("skipped") == true || autoPair["error"]?.objectValue != nil {
                    summary.blocked = true
                    summary.problems += 1
                }
                if let remaining = autoPair["remaining"]?.objectValue {
                    summary.transfers += Int(max(remaining.int("total") ?? 0, 0))
                }
            }
        }
        return summary
    }

    private static func progressDetailRows(_ row: [String: JSONValue]) -> [String] {
        var rows: [String] = []
        if let index = row.int("job_index"), let total = row.int("job_total"), total > 0 {
            rows.append("\(index) / \(total)")
        }
        if let imported = row.int("imported"), let skipped = row.int("skipped") {
            rows.append("+\(imported) · =\(skipped)")
        }
        if let retained = row.int("retained_targets") { rows.append("\(retained) targets") }
        if let unused = row.int("unused_streak"), let gap = row.int("gap_limit") {
            rows.append("\(unused) / \(gap) gap")
        }
        return rows
    }

    private static func opaqueBookKey(_ identity: String) -> String {
        var hash: UInt64 = 14_695_981_039_346_656_037
        for byte in identity.utf8 {
            hash ^= UInt64(byte)
            hash &*= 1_099_511_628_211
        }
        return String(hash, radix: 16)
    }
}

public struct DescriptorPreviewAddress: Identifiable, Equatable, Sendable {
    public let branch: String
    public let index: Int
    public let address: String
    public let derivationPath: String
    public var id: String { "\(branch):\(index):\(address)" }
}

@MainActor
@Observable
public final class WalletMutationViewModel {
    public var label = ""
    public var walletMaterial = ""
    public var backend = ""
    public var chain = "bitcoin"
    public var network = "main"
    public var gapLimit = 20
    public var scriptTypes: Set<String> = []
    public var archived = false
    public var passphrase = ""
    public private(set) var preview: [DescriptorPreviewAddress] = []
    public private(set) var detectionMessage = ""
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?
    public private(set) var didSave = false

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func detectAndPreview() async {
        guard !walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        isWorking = true
        defer { isWorking = false }
        do {
            if walletMaterial.hasPrefix("xpub") || walletMaterial.hasPrefix("tpub") {
                let detected = try await daemon.invoke(
                    .uiWalletsDetectScriptTypes,
                    args: walletArgs(includeMaterial: true, includeScriptTypes: false)
                )
                if let error = detected.error { throw error }
                let object = detected.data?.objectValue ?? [:]
                let active = object["active"]?.arrayValue?.compactMap(\.stringValue) ?? []
                scriptTypes = Set(active)
                detectionMessage = object.bool("probed") == true
                    ? active.joined(separator: ", ")
                    : (object.string("reason") ?? "p2wpkh")
            }
            let envelope = try await daemon.invoke(
                .uiWalletsPreviewDescriptor,
                args: walletArgs(includeMaterial: true, includeScriptTypes: true).merging(["count": .integer(5)]) { _, new in new }
            )
            if let error = envelope.error { throw error }
            let object = envelope.data?.objectValue ?? [:]
            preview = object.objects("addresses").compactMap { row in
                guard let address = row.string("address") else { return nil }
                return DescriptorPreviewAddress(
                    branch: row.string("branch") ?? "receive",
                    index: Int(row.int("index") ?? 0),
                    address: address,
                    derivationPath: row.string("derivation_path") ?? ""
                )
            }
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    public func create() async {
        isWorking = true
        defer { isWorking = false }
        do {
            var args = walletArgs(includeMaterial: true, includeScriptTypes: true)
            args["label"] = .string(label.trimmingCharacters(in: .whitespacesAndNewlines))
            args["kind"] = .string("descriptor")
            args["gap_limit"] = .integer(Int64(gapLimit))
            let envelope = try await daemon.invoke(.uiWalletsCreate, args: args)
            if let error = envelope.error { throw error }
            didSave = true
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    public func update(wallet: WalletRow) async {
        isWorking = true
        defer { isWorking = false }
        do {
            let authentication: JSONValue = passphrase.isEmpty
                ? .object(["plaintext_change_ack": .string("CHANGE LOCAL DATA")])
                : .object(["passphrase_secret": .string(passphrase)])
            let envelope = try await daemon.invoke(
                .uiWalletsUpdate,
                args: [
                    "wallet": .string(wallet.id),
                    "label": .string(label.trimmingCharacters(in: .whitespacesAndNewlines)),
                    "deprecated": .bool(archived),
                    "auth_response": authentication,
                ]
            )
            if let error = envelope.error { throw error }
            if envelope.kind == "auth_required" {
                throw DaemonClientError.transport("Enter the database passphrase to save this change.")
            }
            didSave = true
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    public func configureForEdit(_ wallet: WalletRow) {
        label = wallet.label
        archived = wallet.deprecated
    }

    private func walletArgs(includeMaterial: Bool, includeScriptTypes: Bool) -> [String: JSONValue] {
        var args: [String: JSONValue] = ["chain": .string(chain), "network": .string(network)]
        if includeMaterial { args["wallet_material"] = .string(walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines)) }
        if includeScriptTypes, !scriptTypes.isEmpty {
            args["script_types"] = .array(scriptTypes.sorted().map(JSONValue.string))
        }
        if !backend.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            args["backend"] = .string(backend.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return args
    }
}

public struct LedgerPreviewRow: Identifiable, Equatable, Sendable {
    public let id: Int
    public let values: String
}

@MainActor
@Observable
public final class LedgerImportViewModel {
    public private(set) var fileURL: URL?
    public private(set) var rows: [LedgerPreviewRow] = []
    public private(set) var mapped = 0
    public private(set) var errors = 0
    public private(set) var confident = false
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?
    public private(set) var didImport = false

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public var canImport: Bool { fileURL != nil && confident && mapped > 0 && errors == 0 }

    public func acknowledgeImport() { didImport = false }

    public func preview(_ url: URL) async {
        isWorking = true
        defer { isWorking = false }
        do {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            let bytes = try Data(contentsOf: url)
            let envelope = try await daemon.invoke(
                .uiWalletsLedgerPreview,
                args: [
                    "filename": .string(url.lastPathComponent),
                    "source_bytes_base64": .string(bytes.base64EncodedString()),
                ]
            )
            if let error = envelope.error { throw error }
            let object = envelope.data?.objectValue ?? [:]
            fileURL = url
            mapped = Int(object.int("mapped") ?? 0)
            errors = Int(object.int("errors") ?? 0)
            confident = object.bool("confident") ?? false
            rows = (object["preview"]?.arrayValue ?? []).prefix(8).enumerated().map { index, value in
                LedgerPreviewRow(id: index, values: Self.compactDescription(value))
            }
            errorMessage = nil
        } catch {
            fileURL = nil
            rows = []
            errorMessage = String(describing: error)
        }
    }

    public func importFile(sourceFormat: String, wallet: String?) async {
        guard let fileURL, canImport else { return }
        isWorking = true
        defer { isWorking = false }
        do {
            var args: [String: JSONValue] = [
                "source_file": .string(fileURL.path),
                "source_format": .string(sourceFormat),
            ]
            if let wallet, !wallet.isEmpty { args["wallet"] = .string(wallet) }
            let envelope = try await daemon.invoke(.uiWalletsImportFile, args: args)
            if let error = envelope.error { throw error }
            if envelope.kind == "auth_required" {
                throw DaemonClientError.transport("Database authentication is required.")
            }
            didImport = true
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }

    private static func compactDescription(_ value: JSONValue) -> String {
        guard let object = value.objectValue else { return String(describing: value) }
        return object.sorted { $0.key < $1.key }.compactMap { key, value in
            switch value {
            case let .string(text): "\(key): \(text)"
            case let .integer(number): "\(key): \(number)"
            case let .unsignedInteger(number): "\(key): \(number)"
            case let .number(number): "\(key): \(number)"
            case let .bool(flag): "\(key): \(flag)"
            default: nil
            }
        }.joined(separator: " · ")
    }
}

@MainActor
@Observable
public final class BookSetupViewModel {
    public var workspaceLabel = "My Books"
    public var profileLabel = "Private"
    public var taxCountry = "generic"
    public var fiatCurrency = "EUR"
    public var taxLongTermDays = 365
    public var gainsAlgorithm = "fifo"
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?
    public private(set) var didSave = false

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func save() async {
        isWorking = true
        defer { isWorking = false }
        do {
            let envelope = try await daemon.invoke(
                .uiOnboardingComplete,
                args: [
                    "workspace_label": .string(workspaceLabel),
                    "profile_label": .string(profileLabel),
                    "tax_country": .string(taxCountry),
                    "fiat_currency": .string(fiatCurrency),
                    "tax_long_term_days": .integer(Int64(taxLongTermDays)),
                    "gains_algorithm": .string(gainsAlgorithm),
                ]
            )
            if let error = envelope.error { throw error }
            if envelope.kind == "auth_required" {
                throw DaemonClientError.transport("Database authentication is required.")
            }
            didSave = true
            errorMessage = nil
        } catch {
            errorMessage = String(describing: error)
        }
    }
}
