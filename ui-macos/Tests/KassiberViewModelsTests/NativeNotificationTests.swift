import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Native notifications and first sync", .serialized)
@MainActor
struct NativeNotificationTests {
    @Test("dedupe upserts and persistence strips transient detail and progress")
    func safeBoundedPersistence() throws {
        let suite = "kassiber-native-notifications-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let key = "test.notifications"
        var store: NativeNotificationStore? = NativeNotificationStore(
            defaults: defaults,
            persistenceKey: key,
            maximumCount: 2,
            transientLifetime: .seconds(3_600)
        )

        store?.updateProgress(dedupeKey: "book:private-path", value: 0.2, label: "Wallet Alice")
        store?.updateProgress(dedupeKey: "book:private-path", value: 0.7, label: "Wallet Alice 70")
        #expect(store?.notifications.count == 1)
        #expect(store?.notifications.first?.progress?.value == 0.7)
        store?.post(
            titleKey: "notifications.bookRefresh.finished.title",
            bodyKey: "notifications.bookRefresh.finished.body",
            tone: .success,
            dedupeKey: "book:private-path",
            progress: NativeNotificationProgress(value: 1, label: "Sensitive source label"),
            ephemeralDetail: "Sensitive source label"
        )
        store?.post(
            titleKey: "notifications.mutation.finished.title",
            bodyKey: "notifications.mutation.finished.body",
            tone: .success,
            dedupeKey: "second"
        )
        store?.post(
            titleKey: "notifications.mutation.failed.title",
            bodyKey: "notifications.mutation.failed.body",
            tone: .error,
            dedupeKey: "transient",
            ephemeralDetail: "token=should-not-persist",
            transient: true
        )
        #expect(store?.notifications.count == 2)
        store = nil

        let restored = NativeNotificationStore(
            defaults: defaults,
            persistenceKey: key,
            maximumCount: 2,
            transientLifetime: .seconds(3_600)
        )
        #expect(restored.notifications.count == 1)
        #expect(restored.notifications.first?.progress == nil)
        #expect(restored.notifications.first?.ephemeralDetail == nil)
        #expect(defaults.data(forKey: key).map { String(decoding: $0, as: UTF8.self) }?.contains("private-path") == false)
        #expect(defaults.data(forKey: key).map { String(decoding: $0, as: UTF8.self) }?.contains("should-not-persist") == false)
    }

    @Test("review notifications remain synthesized and clear-all leaves actions")
    func synthesizedReviewItems() throws {
        let defaults = try #require(UserDefaults(suiteName: "kassiber-native-review-\(UUID().uuidString)"))
        let store = NativeNotificationStore(defaults: defaults)
        store.post(
            titleKey: "notifications.mutation.finished.title",
            bodyKey: "notifications.mutation.finished.body",
            tone: .success
        )
        store.setReviewBadges(ReviewBadges(quarantine: 2, journalsNeedProcessing: true, swaps: 3))

        #expect(store.reviewNotifications.map(\.id) == ["review-journals", "review-quarantine", "review-swaps"])
        #expect(store.reviewNotifications.first?.action == .processJournals)
        store.clearAll()
        #expect(store.notifications.isEmpty)
        #expect(store.reviewNotifications.count == 3)
    }

    @Test("central request reducer distinguishes success and redacted failure outcomes")
    func hostRequestReducer() throws {
        let defaults = try #require(UserDefaults(suiteName: "kassiber-native-reducer-\(UUID().uuidString)"))
        let store = NativeNotificationStore(defaults: defaults, transientLifetime: .seconds(3_600))
        let started = DaemonRecord(kind: "native.request.activity", event: true, data: [
            "request_id": "macos-1", "kind": "ui.profiles.update", "state": "started",
        ])
        let failed = DaemonRecord(kind: "native.request.activity", event: true, data: [
            "request_id": "macos-1", "kind": "ui.profiles.update", "state": "failed",
            "error_code": "update_failed", "detail": "Public-safe failure",
        ])

        #expect(store.reduceHostEvent(started).consumed)
        let reduction = store.reduceHostEvent(failed)
        #expect(reduction.terminalMutation)
        #expect(reduction.shouldRefreshReviewBadges)
        #expect(store.notifications.count == 1)
        #expect(store.notifications.first?.tone == .error)
        #expect(store.notifications.first?.target == .logs)
        #expect(store.notifications.first?.progress == nil)
    }

    @Test("clean first sync is remembered per opaque book identity")
    func firstSyncIdentity() async throws {
        let suite = "kassiber-native-first-sync-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let terminal = DaemonRecord(kind: "ui.freshness.run", data: [
            "completed": [["job_type": "journal_refresh", "status": "done", "result": [:]]],
            "summary": ["failed": 0, "blocking_reports": 0, "rate_limited": 0],
        ])
        let firstClient = ScriptedDaemonClient(scripts: [.uiFreshnessRun: [terminal]])
        let first = BookRefreshCoordinator(
            daemon: firstClient,
            defaults: defaults,
            firstSyncPersistenceKey: "test.firstSync"
        )
        first.setBookIdentity("profile-1:/private/path")
        await first.run(.refresh)
        #expect(first.isFirstSync)
        #expect(first.outcome == .clean)
        #expect(first.hasCompletedFirstSync(for: "profile-1:/private/path"))
        #expect(defaults.stringArray(forKey: "test.firstSync")?.first?.contains("private") == false)

        let again = BookRefreshCoordinator(
            daemon: ScriptedDaemonClient(scripts: [.uiFreshnessRun: [terminal]]),
            defaults: defaults,
            firstSyncPersistenceKey: "test.firstSync"
        )
        again.setBookIdentity("profile-1:/private/path")
        await again.run(.refresh)
        #expect(!again.isFirstSync)
        #expect(again.outcome == .clean)
    }

    @Test("partial sources never produce success or complete first sync")
    func partialFailure() async throws {
        let defaults = try #require(UserDefaults(suiteName: "kassiber-native-partial-\(UUID().uuidString)"))
        let notices = NativeNotificationStore(defaults: defaults, transientLifetime: .seconds(3_600))
        let terminal = DaemonRecord(kind: "ui.freshness.run", data: [
            "completed": [[
                "job_type": "btcpay_provenance", "source_label": "Store", "status": "rate_limited",
            ]],
            "sources": [[
                "source_key": "btcpay:1", "source_type": "btcpay_provenance",
                "source_label": "Store", "status": "partially_stale",
            ]],
            "summary": ["failed": 0, "blocking_reports": 0, "rate_limited": 1],
        ])
        let model = BookRefreshCoordinator(
            daemon: ScriptedDaemonClient(scripts: [.uiFreshnessRun: [terminal]]),
            notifications: notices,
            defaults: defaults,
            firstSyncPersistenceKey: "test.firstSync"
        )
        model.setBookIdentity("profile-partial")

        await model.run(.refresh)

        #expect(model.outcome == .partial)
        #expect(model.terminalTarget == .logs)
        #expect(!model.hasCompletedFirstSync(for: "profile-partial"))
        #expect(notices.notifications.first?.tone == .warning)
        #expect(notices.notifications.first?.titleKey == "notifications.bookRefresh.partial.title")
    }

    @Test("quarantine blocks setup completion while swap review does not")
    func terminalReviewRouting() async throws {
        let defaults = try #require(UserDefaults(suiteName: "kassiber-native-review-route-\(UUID().uuidString)"))
        let quarantineTerminal = DaemonRecord(kind: "ui.freshness.run", data: [
            "completed": [[
                "job_type": "journal_refresh", "status": "done", "result": ["quarantined": 2],
            ]],
        ])
        let quarantined = BookRefreshCoordinator(
            daemon: ScriptedDaemonClient(scripts: [.uiFreshnessRun: [quarantineTerminal]]),
            defaults: defaults,
            firstSyncPersistenceKey: "test.firstSync"
        )
        quarantined.setBookIdentity("profile-review")
        await quarantined.run(.refresh)
        #expect(quarantined.outcome == .reviewRequired)
        #expect(quarantined.terminalTarget == .quarantine)
        #expect(!quarantined.hasCompletedFirstSync(for: "profile-review"))

        let swapTerminal = DaemonRecord(kind: "ui.freshness.run", data: [
            "completed": [[
                "job_type": "journal_refresh", "status": "done",
                "result": ["auto_pair": ["remaining": ["total": 3]]],
            ]],
        ])
        let swaps = BookRefreshCoordinator(
            daemon: ScriptedDaemonClient(scripts: [.uiFreshnessRun: [swapTerminal]]),
            defaults: defaults,
            firstSyncPersistenceKey: "test.firstSync"
        )
        swaps.setBookIdentity("profile-swap")
        await swaps.run(.refresh)
        #expect(swaps.terminalTarget == .swaps)
        #expect(swaps.hasCompletedFirstSync(for: "profile-swap"))
    }
}
