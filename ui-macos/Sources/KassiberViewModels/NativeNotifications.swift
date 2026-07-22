import Foundation
import Observation
import KassiberDaemonKit

public enum NativeNotificationTone: String, Codable, Equatable, Sendable {
    case info
    case success
    case warning
    case error
}

/// Language-independent notification routing. The raw values deliberately
/// match `AppScreen` so translated copy never participates in navigation.
public enum NativeNotificationTarget: String, Codable, Equatable, Sendable {
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

    public var screen: AppScreen? { AppScreen(rawValue: rawValue) }
}

public enum NativeNotificationAction: String, Codable, Equatable, Sendable {
    case open
    case processJournals
    case restoreBookRefresh
}

public enum NativeNotificationCategory: String, Codable, Equatable, Sendable {
    case mutation
    case maintenance
    case review
    case system
}

public struct NativeNotificationProgress: Equatable, Sendable {
    public var value: Double?
    public var indeterminate: Bool
    public var label: String

    public init(value: Double? = nil, indeterminate: Bool = false, label: String = "") {
        self.value = value
        self.indeterminate = indeterminate
        self.label = label
    }
}

/// A presentation-safe notification. Text is stored as localization keys;
/// potentially sensitive daemon detail is RAM-only and never encoded.
public struct NativeAppNotification: Identifiable, Equatable, Sendable {
    public var id: String
    public var titleKey: String
    public var bodyKey: String
    public var tone: NativeNotificationTone
    public var category: NativeNotificationCategory
    public var dedupeFingerprint: String?
    public var progress: NativeNotificationProgress?
    public var target: NativeNotificationTarget?
    public var action: NativeNotificationAction?
    public var count: Int?
    public var createdAt: Date
    public var ephemeralDetail: String?
    public var transient: Bool

    public init(
        id: String = UUID().uuidString,
        titleKey: String,
        bodyKey: String,
        tone: NativeNotificationTone,
        category: NativeNotificationCategory = .system,
        dedupeFingerprint: String? = nil,
        progress: NativeNotificationProgress? = nil,
        target: NativeNotificationTarget? = nil,
        action: NativeNotificationAction? = nil,
        count: Int? = nil,
        createdAt: Date = Date(),
        ephemeralDetail: String? = nil,
        transient: Bool = false
    ) {
        self.id = id
        self.titleKey = titleKey
        self.bodyKey = bodyKey
        self.tone = tone
        self.category = category
        self.dedupeFingerprint = dedupeFingerprint
        self.progress = progress
        self.target = target
        self.action = action
        self.count = count
        self.createdAt = createdAt
        self.ephemeralDetail = ephemeralDetail
        self.transient = transient
    }
}

public struct NativeNotificationReduction: Equatable, Sendable {
    public var consumed: Bool
    public var terminalMutation: Bool
    public var shouldRefreshReviewBadges: Bool

    public init(
        consumed: Bool = false,
        terminalMutation: Bool = false,
        shouldRefreshReviewBadges: Bool = false
    ) {
        self.consumed = consumed
        self.terminalMutation = terminalMutation
        self.shouldRefreshReviewBadges = shouldRefreshReviewBadges
    }
}

@MainActor
@Observable
public final class NativeNotificationStore {
    public private(set) var notifications: [NativeAppNotification] = []
    public private(set) var reviewBadges = ReviewBadges()

    public var reviewNotifications: [NativeAppNotification] {
        var rows: [NativeAppNotification] = []
        if reviewBadges.journalsNeedProcessing {
            rows.append(NativeAppNotification(
                id: "review-journals",
                titleKey: "notifications.review.journals.title",
                bodyKey: "notifications.review.journals.body",
                tone: .warning,
                category: .review,
                target: .journals,
                action: .processJournals
            ))
        }
        if reviewBadges.quarantine > 0 {
            rows.append(NativeAppNotification(
                id: "review-quarantine",
                titleKey: "notifications.review.quarantine.title",
                bodyKey: "notifications.review.quarantine.body %lld",
                tone: .warning,
                category: .review,
                target: .quarantine,
                action: .open,
                count: reviewBadges.quarantine
            ))
        }
        if let swaps = reviewBadges.swaps, swaps > 0 {
            rows.append(NativeAppNotification(
                id: "review-swaps",
                titleKey: "notifications.review.swaps.title",
                bodyKey: "notifications.review.swaps.body %lld",
                tone: .warning,
                category: .review,
                target: .swaps,
                action: .open,
                count: swaps
            ))
        }
        return rows
    }

    /// Review items are synthesized from live daemon state and intentionally
    /// remain separate from the clearable/persisted notification history.
    public var allNotifications: [NativeAppNotification] {
        notifications + reviewNotifications
    }

    public var count: Int { allNotifications.count }

    private let defaults: UserDefaults
    private let persistenceKey: String
    private let maximumCount: Int
    private let transientLifetime: Duration
    private var expiryTasks: [String: Task<Void, Never>] = [:]

    public init(
        defaults: UserDefaults = .standard,
        persistenceKey: String = "native.notifications.v1",
        maximumCount: Int = 12,
        transientLifetime: Duration = .seconds(8)
    ) {
        self.defaults = defaults
        self.persistenceKey = persistenceKey
        self.maximumCount = max(1, maximumCount)
        self.transientLifetime = transientLifetime
        restore()
    }

    public func setReviewBadges(_ badges: ReviewBadges) {
        reviewBadges = badges
    }

    @discardableResult
    public func post(
        titleKey: String,
        bodyKey: String,
        tone: NativeNotificationTone,
        category: NativeNotificationCategory = .system,
        dedupeKey: String? = nil,
        progress: NativeNotificationProgress? = nil,
        target: NativeNotificationTarget? = nil,
        action: NativeNotificationAction? = nil,
        count: Int? = nil,
        ephemeralDetail: String? = nil,
        transient: Bool = false
    ) -> String {
        let fingerprint = dedupeKey.map(Self.fingerprint)
        let existing = fingerprint.flatMap { value in
            notifications.first(where: { $0.dedupeFingerprint == value })
        }
        let id = existing?.id ?? UUID().uuidString
        let item = NativeAppNotification(
            id: id,
            titleKey: titleKey,
            bodyKey: bodyKey,
            tone: tone,
            category: category,
            dedupeFingerprint: fingerprint,
            progress: progress,
            target: target,
            action: action,
            count: count,
            createdAt: Date(),
            ephemeralDetail: Self.boundedDetail(ephemeralDetail),
            transient: transient
        )
        notifications.removeAll { $0.id == id }
        notifications.insert(item, at: 0)
        trimAndPersist()
        if transient { scheduleExpiry(for: id) }
        return id
    }

    public func updateProgress(
        dedupeKey: String,
        value: Double? = nil,
        indeterminate: Bool = false,
        label: String,
        titleKey: String = "notifications.mutation.running.title",
        bodyKey: String = "notifications.mutation.running.body"
    ) {
        _ = post(
            titleKey: titleKey,
            bodyKey: bodyKey,
            tone: .info,
            category: .maintenance,
            dedupeKey: dedupeKey,
            progress: NativeNotificationProgress(
                value: value.map { min(max($0, 0), 1) },
                indeterminate: indeterminate,
                label: Self.boundedDetail(label) ?? ""
            ),
            action: dedupeKey == "book-refresh" ? .restoreBookRefresh : nil,
            ephemeralDetail: label
        )
    }

    public func remove(_ id: String) {
        expiryTasks.removeValue(forKey: id)?.cancel()
        notifications.removeAll { $0.id == id }
        persist()
    }

    /// Clearable history only. Live journal/quarantine/swap review items remain
    /// until their underlying daemon state is resolved.
    public func clearAll() {
        for task in expiryTasks.values { task.cancel() }
        expiryTasks.removeAll()
        notifications.removeAll()
        persist()
    }

    /// Central shell-callable reducer for `native.request.activity`. This is the
    /// single generic mutation/progress path; feature-specific coordinators can
    /// replace the same dedupe entry with richer terminal copy.
    @discardableResult
    public func reduceHostEvent(_ event: DaemonRecord) -> NativeNotificationReduction {
        guard event.kind == "native.request.activity",
              let row = event.data?.objectValue,
              let requestID = row.string("request_id"),
              let kind = row.string("kind"),
              let state = row.string("state") else { return NativeNotificationReduction() }
        let reviewInvalidating = DaemonKind(rawValue: kind)
            .map(AppShellViewModel.reviewBadgeInvalidatingKinds.contains) == true
        guard Self.isMutation(kind) || reviewInvalidating else {
            return NativeNotificationReduction(consumed: true)
        }

        let dedupe = Self.activityDedupeKey(kind: kind, requestID: requestID)
        let detail = row.string("detail", "error_code") ?? kind
        switch state {
        case "started":
            updateProgress(
                dedupeKey: dedupe,
                indeterminate: true,
                label: kind
            )
            return NativeNotificationReduction(consumed: true)
        case "progress":
            updateProgress(
                dedupeKey: dedupe,
                indeterminate: true,
                label: detail
            )
            return NativeNotificationReduction(consumed: true)
        case "finished":
            // A successful envelope is not necessarily a clean maintenance
            // result: freshness/sync terminals may contain partial source
            // errors, rate limits, quarantine, or report blockers. Their
            // feature coordinator owns terminal classification and replaces
            // this progress entry; the generic reducer must never call them a
            // success based only on the transport-level terminal state.
            if !Self.requiresTerminalInspection(kind) {
                _ = post(
                    titleKey: "notifications.mutation.finished.title",
                    bodyKey: "notifications.mutation.finished.body",
                    tone: .success,
                    category: .mutation,
                    dedupeKey: dedupe,
                    ephemeralDetail: kind,
                    transient: true
                )
            }
        case "failed":
            _ = post(
                titleKey: "notifications.mutation.failed.title",
                bodyKey: "notifications.mutation.failed.body",
                tone: .error,
                category: .mutation,
                dedupeKey: dedupe,
                target: .logs,
                action: .open,
                ephemeralDetail: detail,
                transient: true
            )
        case "cancelled":
            _ = post(
                titleKey: "notifications.mutation.cancelled.title",
                bodyKey: "notifications.mutation.cancelled.body",
                tone: .warning,
                category: .mutation,
                dedupeKey: dedupe,
                ephemeralDetail: kind,
                transient: true
            )
        default:
            return NativeNotificationReduction(consumed: true)
        }

        let terminal = ["finished", "failed", "cancelled"].contains(state)
        let invalidating = terminal && reviewInvalidating
        return NativeNotificationReduction(
            consumed: true,
            terminalMutation: terminal,
            shouldRefreshReviewBadges: invalidating
        )
    }

    private func trimAndPersist() {
        if notifications.count > maximumCount {
            let removed = notifications.suffix(from: maximumCount).map(\.id)
            for id in removed { expiryTasks.removeValue(forKey: id)?.cancel() }
            notifications.removeLast(notifications.count - maximumCount)
        }
        persist()
    }

    private func scheduleExpiry(for id: String) {
        expiryTasks.removeValue(forKey: id)?.cancel()
        let lifetime = transientLifetime
        expiryTasks[id] = Task { @MainActor [weak self] in
            do { try await Task.sleep(for: lifetime) } catch { return }
            guard !Task.isCancelled else { return }
            self?.remove(id)
        }
    }

    private struct PersistedNotification: Codable {
        let id: String
        let titleKey: String
        let bodyKey: String
        let tone: NativeNotificationTone
        let category: NativeNotificationCategory
        let dedupeFingerprint: String?
        let target: NativeNotificationTarget?
        let action: NativeNotificationAction?
        let count: Int?
        let createdAt: Date
    }

    private func persist() {
        let safe = notifications.filter { !$0.transient }.prefix(maximumCount).map {
            PersistedNotification(
                id: $0.id,
                titleKey: $0.titleKey,
                bodyKey: $0.bodyKey,
                tone: $0.tone,
                category: $0.category,
                dedupeFingerprint: $0.dedupeFingerprint,
                target: $0.target,
                action: $0.action,
                count: $0.count,
                createdAt: $0.createdAt
            )
        }
        guard let data = try? JSONEncoder().encode(Array(safe)) else { return }
        defaults.set(data, forKey: persistenceKey)
    }

    private func restore() {
        guard let data = defaults.data(forKey: persistenceKey),
              let rows = try? JSONDecoder().decode([PersistedNotification].self, from: data) else { return }
        notifications = rows.prefix(maximumCount).map {
            NativeAppNotification(
                id: $0.id,
                titleKey: $0.titleKey,
                bodyKey: $0.bodyKey,
                tone: $0.tone,
                category: $0.category,
                dedupeFingerprint: $0.dedupeFingerprint,
                target: $0.target,
                action: $0.action,
                count: $0.count,
                createdAt: $0.createdAt
            )
        }
    }

    private static func activityDedupeKey(kind: String, requestID: String) -> String {
        switch kind {
        case DaemonKind.uiFreshnessRun.rawValue: "book-refresh"
        case DaemonKind.uiWorkspaceFreshnessRun.rawValue: "workspace-refresh"
        case DaemonKind.uiJournalsProcess.rawValue: "journal-refresh"
        case DaemonKind.uiRatesRebuild.rawValue: "rates-refresh"
        case DaemonKind.uiWalletsSync.rawValue: "wallet-refresh"
        default: "mutation:\(requestID)"
        }
    }

    private static func isMutation(_ kind: String) -> Bool {
        kind.contains(".create")
            || kind.contains(".update")
            || kind.contains(".delete")
            || kind.contains(".import")
            || kind.contains(".export")
            || kind.contains(".sync")
            || kind.contains(".run")
            || kind.contains(".process")
            || kind.contains(".pair")
            || kind.contains(".resolve")
            || kind.contains(".configure")
            || kind.hasSuffix(".set_api_key")
            || kind.hasSuffix(".consent")
    }

    private static func requiresTerminalInspection(_ kind: String) -> Bool {
        [
            DaemonKind.uiFreshnessRun.rawValue,
            DaemonKind.uiWorkspaceFreshnessRun.rawValue,
            DaemonKind.uiWalletsSync.rawValue,
            DaemonKind.uiJournalsProcess.rawValue,
            DaemonKind.uiMaintenanceRun.rawValue,
            DaemonKind.uiRatesRebuild.rawValue,
        ].contains(kind)
    }

    private static func boundedDetail(_ detail: String?) -> String? {
        guard let detail else { return nil }
        let normalized = detail.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return nil }
        return String(normalized.prefix(240))
    }

    /// FNV-1a keeps arbitrary producer dedupe strings (book ids, source ids,
    /// paths) out of UserDefaults while still supporting stable coalescing.
    private static func fingerprint(_ value: String) -> String {
        var hash: UInt64 = 14_695_981_039_346_656_037
        for byte in value.utf8 {
            hash ^= UInt64(byte)
            hash &*= 1_099_511_628_211
        }
        return String(hash, radix: 16)
    }
}
