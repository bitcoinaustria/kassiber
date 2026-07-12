import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("AI chat view model")
@MainActor
struct AIChatViewModelTests {
    @Test("stream reduces deltas and upserts tool cards")
    func streamReduction() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .aiChat: [
                DaemonRecord(kind: "ai.chat.status", data: ["phase": "waiting_for_model", "label": "Loading model"]),
                DaemonRecord(kind: "ai.chat.tool_call", data: [
                    "call_id": "call-1", "name": "ui.overview.snapshot",
                    "kind_class": "read_only", "needs_consent": false, "arguments": [:],
                ]),
                DaemonRecord(kind: "ai.chat.tool_result", data: [
                    "call_id": "call-1", "ok": true,
                    "envelope": ["kind": "ui.overview.snapshot", "data": ["ready": true]],
                ]),
                DaemonRecord(kind: "ai.chat.delta", data: ["delta": ["content": "<thi"]]),
                DaemonRecord(kind: "ai.chat.delta", data: ["delta": ["content": "nk>Checking the book"]]),
                DaemonRecord(kind: "ai.chat.delta", data: ["delta": ["content": "</think>Hello **book**"]]),
                DaemonRecord(kind: "ai.chat.status", data: ["phase": "thinking"]),
                DaemonRecord(kind: "ai.chat.delta", data: ["delta": ["reasoning": "Verifying totals"]]),
                DaemonRecord(kind: "ai.chat", data: ["finish_reason": "stop"]),
            ],
        ])
        let model = AIChatViewModel(daemon: client)
        model.model = "test-model"
        model.draft = "Hello"
        model.readToolsEnabled = true
        model.thinkingEffort = .high

        await model.send()

        let assistant = try #require(model.messages.last)
        #expect(assistant.content == "Hello **book**")
        #expect(assistant.status == .done)
        #expect(assistant.thinkingSegments.map(\.content) == ["Checking the book", "Verifying totals"])
        #expect(assistant.tools.count == 1)
        #expect(assistant.tools[0].status == "done")
        #expect(assistant.tools[0].result?["kind"]?.stringValue == "ui.overview.snapshot")
        let call = try #require(await client.calls().first)
        #expect(call.args?["tools_enabled"] == true)
        #expect(call.args?["persist"] == .string("auto"))
        #expect(call.args?["options"]?["reasoning_effort"] == .string("high"))
    }

    @Test("consent response targets the active chat request")
    func consentArguments() async throws {
        let client = PausableChatDaemon()
        let model = AIChatViewModel(daemon: client)
        model.model = "test-model"
        model.draft = "Run maintenance"
        let active = Task { await model.send() }
        while await client.streamCount() == 0 { await Task.yield() }
        await client.yieldFirst(DaemonRecord(kind: "ai.chat.tool_consent_required", data: [
            "call_id": "call-1", "name": "ui.maintenance.run", "summary": "Refresh",
            "arguments_preview": [:],
        ]))
        while model.pendingConsent == nil { await Task.yield() }

        await model.decideConsent("allow_once")
        let consent = await client.lastInvocation(.aiToolCallConsent)
        #expect(consent?["target_request_id"] == .string("queue-1"))
        #expect(consent?["call_id"] == .string("call-1"))
        await model.stop()
        let cancel = await client.lastInvocation(.aiChatCancel)
        #expect(cancel?["target_request_id"] == .string("queue-1"))
        await client.finishFirstStream(reason: "cancelled")
        await active.value
    }

    @Test("edit and retry replaces the branch and sends immediately")
    func editAndRetry() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .aiChat: [
                DaemonRecord(kind: "ai.chat.delta", data: ["delta": ["content": "Revised answer"]]),
                DaemonRecord(kind: "ai.chat", data: ["finish_reason": "stop"]),
            ],
        ])
        let model = AIChatViewModel(daemon: client)
        model.model = "test-model"
        model.loadPreviewConversation()
        let originalUser = try #require(model.messages.first)

        await model.editAndRetry(originalUser.id, content: "Revised question")

        #expect(model.messages.first?.content == "Revised question")
        #expect(model.messages.last?.content == "Revised answer")
        let call = try #require(await client.calls().first { $0.kind == .aiChat })
        #expect(call.args?["seed_history"] == true)
        #expect(call.args?["persist"] == .string("auto"))
    }

    @Test("incognito skips persistence only for a detached conversation")
    func incognitoPersistence() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .aiChat: [DaemonRecord(kind: "ai.chat", data: ["finish_reason": "stop"])],
        ])
        let model = AIChatViewModel(daemon: client)
        model.model = "test-model"
        model.incognito = true
        model.draft = "Private question"

        await model.send()

        let call = try #require(await client.calls().first { $0.kind == .aiChat })
        #expect(call.args?["persist"] == false)
        #expect(call.args?["seed_history"] == nil)
    }

    @Test("a queued prompt drains after the active stream")
    func queuedPrompt() async throws {
        let client = PausableChatDaemon()
        let model = AIChatViewModel(daemon: client)
        model.model = "test-model"
        model.draft = "First"
        let active = Task { await model.send() }
        while await client.streamCount() == 0 { await Task.yield() }

        model.draft = "Second"
        await model.send()
        #expect(model.queuedPrompts == ["Second"])

        await client.finishFirstStream()
        await active.value
        #expect(model.queuedPrompts.isEmpty)
        #expect(await client.streamCount() == 2)
        #expect(model.messages.filter { $0.role == .user }.map(\.content) == ["First", "Second"])
    }

    @Test("app lock clears every in-memory conversation artifact")
    func clearForLock() async {
        let client = ScriptedDaemonClient()
        let model = AIChatViewModel(daemon: client)
        model.draft = "private draft"
        model.loadPreviewConversation()
        await model.clearForAppLock()
        #expect(model.draft.isEmpty)
        #expect(model.messages.isEmpty)
        #expect(model.sessionID == nil)
        #expect(!model.isStreaming)
    }
}

private actor PausableChatDaemon: DaemonClient {
    private struct Invocation: Sendable {
        let kind: DaemonKind
        let args: [String: JSONValue]?
    }

    private var calls = 0
    private var firstContinuation: AsyncThrowingStream<DaemonRecord, Error>.Continuation?
    private var invocations: [Invocation] = []

    func streamCount() -> Int { calls }

    func yieldFirst(_ record: DaemonRecord) {
        firstContinuation?.yield(record)
    }

    func finishFirstStream(reason: String = "stop") {
        firstContinuation?.yield(DaemonRecord(kind: "ai.chat", data: ["finish_reason": .string(reason)]))
        firstContinuation?.finish()
        firstContinuation = nil
    }

    func lastInvocation(_ kind: DaemonKind) -> [String: JSONValue]? {
        invocations.last(where: { $0.kind == kind })?.args
    }

    func invoke(_ kind: DaemonKind, args: [String: JSONValue]?) async throws -> DaemonEnvelope {
        invocations.append(Invocation(kind: kind, args: args))
        return DaemonRecord(kind: kind.rawValue, data: [:])
    }

    func stream(_ kind: DaemonKind, args: [String: JSONValue]?) async throws -> AsyncThrowingStream<DaemonRecord, Error> {
        try await streamSession(kind, args: args).records
    }

    func streamSession(_ kind: DaemonKind, args: [String: JSONValue]?) async throws -> DaemonStreamSession {
        calls += 1
        let sequence = calls
        let (stream, continuation) = AsyncThrowingStream<DaemonRecord, Error>.makeStream()
        if sequence == 1 {
            firstContinuation = continuation
        } else {
            continuation.yield(DaemonRecord(kind: "ai.chat", data: ["finish_reason": "stop"]))
            continuation.finish()
        }
        return DaemonStreamSession(requestID: "queue-\(sequence)", records: stream)
    }

    func events() async -> AsyncStream<DaemonRecord> {
        AsyncStream { $0.finish() }
    }
}
