import Foundation
import Testing
@testable import KassiberDaemonKit

@Suite("Process daemon supervisor", .serialized)
struct ProcessDaemonSupervisorTests {
    private static let fakeDaemon = """
import json, sys
print(json.dumps({"kind":"daemon.ready","schema_version":1,"data":{"supported_kinds":[]}}), flush=True)
held_chats = set()
for line in sys.stdin:
    request = json.loads(line)
    kind = request["kind"]
    rid = request["request_id"]
    if kind == "ui.review.badges":
        print(json.dumps({"kind":"ui.freshness.worker","schema_version":1,"event":True,"data":{"state":"idle"}}), flush=True)
        print(json.dumps({"kind":"progress","schema_version":1,"request_id":rid,"data":{"step":1}}), flush=True)
        print(json.dumps({"kind":kind,"schema_version":1,"request_id":rid,"data":{"quarantine":2,"journals_needs_processing":True,"swaps":3}}), flush=True)
    elif kind == "ui.profiles.rename":
        print(json.dumps({"kind":"error","schema_version":1,"request_id":rid,"error":{"code":"rename_failed","message":"Could not rename the book; token=super-secret","retryable":False}}), flush=True)
    elif kind == "ai.chat":
        if request.get("args", {}).get("hold"):
            held_chats.add(rid)
        else:
            print(json.dumps({"kind":"ai.chat.delta","schema_version":1,"request_id":rid,"data":{"text":"Hi"}}), flush=True)
            print(json.dumps({"kind":kind,"schema_version":1,"request_id":rid,"data":{"text":"Hi"}}), flush=True)
    elif kind == "ai.chat.cancel":
        target = request.get("args", {}).get("target_request_id")
        print(json.dumps({"kind":kind,"schema_version":1,"request_id":rid,"data":{"cancelled":target in held_chats}}), flush=True)
        if target in held_chats:
            held_chats.remove(target)
            print(json.dumps({"kind":"ai.chat","schema_version":1,"request_id":target,"data":{"finish_reason":"cancelled"}}), flush=True)
    elif kind == "ui.transactions.list":
        payload = json.dumps({"kind":kind,"schema_version":1,"request_id":rid,"data":{"blob":"x" * 120000}}) + "\\n"
        for start in range(0, len(payload), 257):
            sys.stdout.write(payload[start:start + 257])
            sys.stdout.flush()
    elif kind == "ai.providers.set_api_key":
        service = "a" * 64
        print(json.dumps({"kind":"supervisor.ai_secret_store.request","schema_version":1,"request_id":"bridge-1","data":{"op":"set","provider_name":"remote","store_id":"macos_keychain","service":service,"account":"remote","secret":"test-secret"}}), flush=True)
        bridge = json.loads(sys.stdin.readline())
        print(json.dumps({"kind":kind,"schema_version":1,"request_id":rid,"data":{"bridge_enabled":request.get("args",{}).get("_desktop_secret_store_bridge"),"default_store":request.get("args",{}).get("_desktop_secret_store_default"),"bridge_kind":bridge.get("kind"),"bridge_state":bridge.get("data",{}).get("state")}}), flush=True)
    elif kind == "ai.providers.list":
        print(json.dumps({"kind":kind,"schema_version":1,"request_id":rid,"data":{"providers":[]}}), flush=True)
    elif kind == "status":
        print(json.dumps({"kind":kind,"schema_version":1,"request_id":rid,"data":{"argv":sys.argv[1:]}}), flush=True)
    else:
        print(json.dumps({"kind":kind,"schema_version":1,"request_id":rid,"data":{}}), flush=True)
"""

    private func makeSupervisor(
        secretStore: any DesktopNativeSecretStore = UnavailableNativeSecretStore()
    ) -> ProcessDaemonSupervisor {
        ProcessDaemonSupervisor(
            configuration: DaemonLaunchConfiguration(
                executable: URL(fileURLWithPath: "/usr/bin/env"),
                arguments: ["python3", "-u", "-c", Self.fakeDaemon],
                workingDirectory: URL(fileURLWithPath: NSTemporaryDirectory())
            ),
            nativeSecretStore: secretStore
        )
    }

    @Test("demultiplexes request terminal and unsolicited event")
    func invokeAndEvent() async throws {
        let supervisor = makeSupervisor()
        let events = await supervisor.events()
        let eventTask = Task { () -> DaemonRecord? in
            for await event in events where event.kind == "ui.freshness.worker" { return event }
            return nil
        }
        let response = try await supervisor.invoke(.uiReviewBadges, args: nil)
        #expect(response.kind == "ui.review.badges")
        #expect(response.data?["quarantine"]?.intValue == 2)
        let event = await eventTask.value
        #expect(event?.kind == "ui.freshness.worker")
        await supervisor.shutdown()
    }

    @Test("publishes central request activity for global progress UI")
    func requestActivity() async throws {
        let supervisor = makeSupervisor()
        let events = await supervisor.events()
        let activityTask = Task { () -> [String] in
            var states: [String] = []
            for await event in events where event.kind == "native.request.activity" {
                guard event.data?["kind"]?.stringValue == "ui.review.badges",
                      let state = event.data?["state"]?.stringValue else { continue }
                states.append(state)
                if state == "finished" { return states }
            }
            return states
        }
        _ = try await supervisor.invoke(.uiReviewBadges, args: nil)
        #expect(await activityTask.value == ["started", "progress", "finished"])
        await supervisor.shutdown()
    }

    @Test("terminal daemon errors publish one redacted failed activity outcome")
    func requestActivityFailure() async throws {
        let supervisor = makeSupervisor()
        let events = await supervisor.events()
        let activityTask = Task { () -> [DaemonRecord] in
            var matches: [DaemonRecord] = []
            for await event in events where event.kind == "native.request.activity" {
                guard event.data?["kind"]?.stringValue == "ui.profiles.rename" else { continue }
                matches.append(event)
                if event.data?["state"]?.stringValue == "failed" { return matches }
            }
            return matches
        }

        let response = try await supervisor.invoke(.uiProfilesRename, args: ["profile_id": "book", "label": "New"])
        #expect(response.error?.code == "rename_failed")
        let activities = await activityTask.value
        #expect(activities.map { $0.data?["state"]?.stringValue } == ["started", "failed"])
        let terminal = try #require(activities.last)
        #expect(terminal.data?["error_code"]?.stringValue == "rename_failed")
        #expect(terminal.data?["detail"]?.stringValue?.contains("super-secret") == false)
        #expect(terminal.data?["detail"]?.stringValue?.contains("[redacted]") == true)
        await supervisor.shutdown()
    }

    @Test("stream yields intermediates and terminal")
    func streaming() async throws {
        let supervisor = makeSupervisor()
        await supervisor.setAIFeaturesEnabled(true)
        let stream = try await supervisor.stream(.aiChat, args: ["prompt": "Hello"])
        var kinds: [String] = []
        for try await record in stream { kinds.append(record.kind) }
        #expect(kinds == ["ai.chat.delta", "ai.chat"])
        await supervisor.shutdown()
    }

    @Test("preserves ordering across a heavily fragmented large response")
    func fragmentedLargeResponse() async throws {
        let supervisor = makeSupervisor()
        let response = try await supervisor.invoke(.uiTransactionsList, args: ["limit": 100])
        #expect(response.data?["blob"]?.stringValue?.count == 120_000)
        await supervisor.shutdown()
    }

    @Test("keeps a bounded RAM-only lifecycle snapshot for the Logs bridge")
    func lifecycleSnapshot() async throws {
        let supervisor = makeSupervisor()
        _ = try await supervisor.invoke(.uiReviewBadges, args: nil)
        let first = await supervisor.lifecycleSnapshot(afterID: 0)
        #expect(first.records.contains { $0.event == "spawned" && $0.detail == "daemon ready" })
        #expect(first.records.allSatisfy { !$0.source.contains("/") })
        await supervisor.shutdown()
        let after = await supervisor.lifecycleSnapshot(afterID: first.lastID)
        #expect(after.records.count == 1)
        #expect(after.records.first?.event == "exited")
    }

    @Test("lifecycle stderr is secret-floored before entering RAM")
    func lifecycleRedaction() async {
        let seed = "abandon ability able about above absent absorb abstract absurd abuse access accident"
        let script = """
import json, sys, time
print(json.dumps({"kind":"daemon.ready","schema_version":1,"data":{}}), flush=True)
for line in sys.stdin:
    sys.stderr.write('{"api_key":"super-secret"} \(seed)\\n')
    sys.stderr.flush()
    time.sleep(0.05)
    print('not-json', flush=True)
    break
"""
        let supervisor = ProcessDaemonSupervisor(configuration: DaemonLaunchConfiguration(
            executable: URL(fileURLWithPath: "/usr/bin/env"),
            arguments: ["python3", "-u", "-c", script],
            workingDirectory: URL(fileURLWithPath: NSTemporaryDirectory())
        ))
        do {
            _ = try await supervisor.invoke(.uiReviewBadges, args: nil)
            Issue.record("Malformed daemon output unexpectedly succeeded")
        } catch {
            // Expected protocol failure.
        }
        let snapshot = await supervisor.lifecycleSnapshot(afterID: 0)
        let text = snapshot.records.map { "\($0.detail) \($0.stderrTail)" }.joined(separator: "\n")
        #expect(!text.contains("super-secret"))
        #expect(!text.contains(seed))
        #expect(text.contains("[redacted"))
        await supervisor.shutdown()
    }

    @Test("bridges AI provider secrets to an injected production Keychain store")
    func secretStoreBridge() async throws {
        let store = InMemoryNativeSecretStore()
        let supervisor = makeSupervisor(secretStore: store)
        let response = try await supervisor.invoke(
            .aiProvidersSetApiKey,
            args: ["name": "remote", "api_key": "provided-by-user"]
        )
        #expect(response.data?["bridge_enabled"]?.boolValue == true)
        #expect(response.data?["default_store"]?.stringValue == NativeSecretStoreIdentifiers.macOSKeychain)
        #expect(response.data?["bridge_kind"]?.stringValue == "supervisor.ai_secret_store.response")
        #expect(response.data?["bridge_state"]?.stringValue == "ok")
        #expect(try store.get(service: String(repeating: "a", count: 64), account: "remote") == Data("test-secret".utf8))

        let list = try await supervisor.invoke(.aiProvidersList, args: nil)
        let policy = list.data?["secret_store_policy"]?.objectValue
        #expect(policy?["platform"]?.stringValue == "macos")
        #expect(policy?["default"]?.objectValue?["store_id"]?.stringValue == NativeSecretStoreIdentifiers.macOSKeychain)
        await supervisor.shutdown()
    }

    @Test("restarts the sidecar against an imported data root and can restore the managed root")
    func dataRootRestart() async throws {
        let supervisor = makeSupervisor()
        try await supervisor.activateDataRoot("/tmp/imported-kassiber/data")
        let imported = try await supervisor.invoke(.status, args: nil)
        #expect(imported.data?["argv"]?.arrayValue?.compactMap(\.stringValue).suffix(2) == ["--data-root", "/tmp/imported-kassiber/data"])
        try await supervisor.clearActivatedDataRoot()
        let restored = try await supervisor.invoke(.status, args: nil)
        #expect(restored.data?["argv"]?.arrayValue?.compactMap(\.stringValue).contains("--data-root") == false)
        await supervisor.shutdown()
    }

    @Test("denies non-renderer kinds while keeping audited desktop kinds available")
    func rendererAllowlist() async throws {
        let supervisor = makeSupervisor()
        let reveal = try await supervisor.invoke(
            .backendsRevealToken,
            args: ["name": "secret-backend"]
        )
        #expect(reveal.kind == "error")
        #expect(reveal.error?.code == "kind_not_allowed")
        #expect(reveal.error?.details?["kind"]?.stringValue == "backends.reveal_token")

        let aiOnlyRead = try await supervisor.invoke(.uiReportBlockers, args: nil)
        #expect(aiOnlyRead.kind == "error")
        #expect(aiOnlyRead.error?.code == "kind_not_allowed")

        let allowed = try await supervisor.invoke(.status, args: nil)
        #expect(allowed.kind == "status")
        await supervisor.shutdown()
    }

    @Test("AI runtime defaults off and disabling it cancels an in-flight chat")
    func aiRuntimeGateAndCancellation() async throws {
        let supervisor = makeSupervisor()
        let blocked = try await supervisor.invoke(
            .aiListModels,
            args: ["provider": "local"]
        )
        #expect(blocked.kind == "error")
        #expect(blocked.error?.code == "ai_features_disabled")
        let providers = try await supervisor.invoke(.aiProvidersList, args: nil)
        #expect(providers.kind == "ai.providers.list")

        await supervisor.setAIFeaturesEnabled(true)
        let session = try await supervisor.streamSession(.aiChat, args: ["hold": true])
        await supervisor.setAIFeaturesEnabled(false)
        var records: [DaemonRecord] = []
        for try await record in session.records { records.append(record) }
        #expect(records.last?.kind == "ai.chat")
        #expect(records.last?.data?["finish_reason"]?.stringValue == "cancelled")
        #expect(records.last?.data?["reason"]?.stringValue == "ai_features_disabled")
        #expect(await supervisor.areAIFeaturesEnabled() == false)
        let blockedAgain = try await supervisor.invoke(
            .aiTestConnection,
            args: ["provider": "local"]
        )
        #expect(blockedAgain.error?.code == "ai_features_disabled")
        await supervisor.shutdown()
    }

    @Test("shutdown releases callers waiting for daemon.ready")
    func shutdownDuringStartup() async throws {
        let supervisor = ProcessDaemonSupervisor(
            configuration: DaemonLaunchConfiguration(
                executable: URL(fileURLWithPath: "/usr/bin/env"),
                arguments: ["python3", "-u", "-c", "import time; time.sleep(60)"],
                workingDirectory: URL(fileURLWithPath: NSTemporaryDirectory())
            )
        )
        let pending = Task {
            try await supervisor.invoke(.uiReviewBadges, args: nil)
        }
        try await Task.sleep(for: .milliseconds(60))
        await supervisor.shutdown()
        do {
            _ = try await pending.value
            Issue.record("startup waiter unexpectedly completed")
        } catch let error as DaemonClientError {
            #expect(error == .daemonExited("daemon was shut down"))
        }
    }
}

private final class InMemoryNativeSecretStore: DesktopNativeSecretStore, @unchecked Sendable {
    let availability = NativeSecretStoreAvailability(state: .available, identityStrength: "production")
    private let lock = NSLock()
    private var values: [String: Data] = [:]

    func get(service: String, account: String) throws -> Data? {
        lock.lock(); defer { lock.unlock() }
        return values["\(service):\(account)"]
    }

    func exists(service: String, account: String) throws -> Bool {
        try get(service: service, account: account) != nil
    }

    func set(service: String, account: String, secret: Data) throws {
        lock.lock(); defer { lock.unlock() }
        values["\(service):\(account)"] = secret
    }

    func delete(service: String, account: String) throws {
        lock.lock(); defer { lock.unlock() }
        values.removeValue(forKey: "\(service):\(account)")
    }
}
