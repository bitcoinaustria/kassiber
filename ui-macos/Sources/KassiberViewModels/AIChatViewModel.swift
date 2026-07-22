import Foundation
import Observation
import KassiberDaemonKit

public enum ChatRole: String, Sendable { case user, assistant, system }
public enum ChatStatus: String, Sendable { case done, streaming, error, cancelled }
public enum ChatThinkingEffort: String, CaseIterable, Identifiable, Sendable {
    case auto, low, medium, high
    public var id: String { rawValue }
}

public struct ChatThinkingSegment: Identifiable, Equatable, Sendable {
    public let id: UUID
    public var content: String

    public init(id: UUID = UUID(), content: String = "") {
        self.id = id
        self.content = content
    }
}

public struct ChatToolCall: Identifiable, Equatable, Sendable {
    public let id: String
    public var name: String
    public var kindClass: String
    public var status: String
    public var summary: String
    public var arguments: JSONValue
    public var result: JSONValue?
}

public struct ChatMessage: Identifiable, Equatable, Sendable {
    public let id: UUID
    public let role: ChatRole
    public var content: String
    public var activity: String
    public var status: ChatStatus
    public var tools: [ChatToolCall]
    public var thinkingSegments: [ChatThinkingSegment]

    public init(
        id: UUID = UUID(), role: ChatRole, content: String,
        activity: String = "", status: ChatStatus = .done,
        tools: [ChatToolCall] = [], thinkingSegments: [ChatThinkingSegment] = []
    ) {
        self.id = id
        self.role = role
        self.content = content
        self.activity = activity
        self.status = status
        self.tools = tools
        self.thinkingSegments = thinkingSegments
    }
}

public struct ToolConsentRequest: Identifiable, Equatable, Sendable {
    public let targetRequestID: String
    public let callID: String
    public let name: String
    public let summary: String
    public let arguments: JSONValue
    public var id: String { callID }
}

public struct ChatSessionRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let updatedAt: Date?
    public let messageCount: Int
}

@MainActor
@Observable
public final class AIChatViewModel {
    public var draft = ""
    public var provider = ""
    public var model = ""
    public var readToolsEnabled = false
    public var incognito = false
    public var thinkingEffort: ChatThinkingEffort = .auto
    public private(set) var providers: [String] = []
    public private(set) var models: [String] = []
    public private(set) var messages: [ChatMessage] = []
    public private(set) var isStreaming = false
    public private(set) var pendingConsent: ToolConsentRequest?
    public private(set) var errorMessage: String?
    public private(set) var sessions: [ChatSessionRow] = []
    public private(set) var sessionID: String?
    public private(set) var historyEnabled = false
    public private(set) var queuedPrompts: [String] = []

    private let daemon: any DaemonClient
    private var currentRequestID: String?
    private var activeAssistantID: UUID?
    private var conversationEpoch: UInt64 = 0
    private var seedHistoryPending = false
    private var thinkParser = ChatThinkParser()

    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public func loadProviders() async {
        do {
            let envelope = try await daemon.invoke(.aiProvidersList, args: nil)
            if let error = envelope.error { throw error }
            let object = envelope.data?.objectValue ?? [:]
            let rows = object.objects("providers")
            providers = rows.compactMap { $0.string("name") }
            if provider.isEmpty { provider = object.string("default") ?? providers.first ?? "" }
            if model.isEmpty,
               let row = rows.first(where: { $0.string("name") == provider }) {
                model = row.string("default_model") ?? ""
            }
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }

    public func loadModels() async {
        guard !provider.isEmpty else { return }
        do {
            let envelope = try await daemon.invoke(
                .aiListModels,
                args: ["provider": .string(provider)]
            )
            if let error = envelope.error { throw error }
            let values = envelope.data?.objectValue?["models"]?.arrayValue ?? []
            models = values.compactMap { value in
                if let name = value.stringValue { return name }
                return value.objectValue?.string("id", "name", "model")
            }
            if model.isEmpty { model = models.first ?? "" }
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }

    public func send() async {
        let prompt = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !prompt.isEmpty else { return }
        draft = ""
        if isStreaming {
            queuedPrompts.append(prompt)
            return
        }
        await runTurn(prompt)
        while !queuedPrompts.isEmpty, !isStreaming {
            let queued = queuedPrompts.removeFirst()
            await runTurn(queued)
        }
    }

    private func runTurn(_ prompt: String) async {
        let epoch = conversationEpoch
        errorMessage = nil
        messages.append(ChatMessage(role: .user, content: prompt))
        let assistant = ChatMessage(role: .assistant, content: "", activity: "preparing", status: .streaming)
        messages.append(assistant)
        activeAssistantID = assistant.id
        thinkParser = ChatThinkParser()
        isStreaming = true
        defer {
            isStreaming = false
            pendingConsent = nil
            currentRequestID = nil
            activeAssistantID = nil
        }

        do {
            let session = try await daemon.streamSession(.aiChat, args: chatArgs())
            guard epoch == conversationEpoch else {
                _ = try? await daemon.invoke(
                    .aiChatCancel,
                    args: ["target_request_id": .string(session.requestID)]
                )
                return
            }
            currentRequestID = session.requestID
            for try await record in session.records {
                guard epoch == conversationEpoch else { break }
                apply(record, targetRequestID: session.requestID)
            }
        } catch {
            errorMessage = String(describing: error)
            updateAssistant { $0.status = .error }
        }
    }

    public func loadSessions() async {
        do {
            let result = try await daemon.invoke(.uiChatSessionsList, args: ["limit": .integer(50)])
            if let error = result.error { throw error }
            let object = result.data?.objectValue ?? [:]
            historyEnabled = object.bool("history_enabled") ?? false
            sessions = object.objects("sessions").compactMap { row in
                guard let id = row.string("id") else { return nil }
                return ChatSessionRow(id: id, title: row.string("title") ?? id,
                    updatedAt: DaemonValueParser.date(row.string("updated_at")), messageCount: Int(row.int("message_count") ?? 0))
            }
        } catch { errorMessage = String(describing: error) }
    }

    public func resume(_ session: ChatSessionRow) async {
        guard !isStreaming else { return }
        do {
            let result = try await daemon.invoke(.uiChatSessionsGet, args: ["session_id": .string(session.id)])
            if let error = result.error { throw error }
            let object = result.data?.objectValue ?? [:]
            messages = object.objects("messages").compactMap { row in
                guard let roleString = row.string("role"), let role = ChatRole(rawValue: roleString), let content = row.string("content"), !content.isEmpty else { return nil }
                return ChatMessage(role: role, content: content)
            }
            sessionID = object.string("id") ?? session.id
            incognito = false
            queuedPrompts = []
            seedHistoryPending = false
        } catch { errorMessage = String(describing: error) }
    }

    public func delete(_ session: ChatSessionRow) async {
        do {
            let result = try await daemon.invoke(.uiChatSessionsDelete, args: ["session_id": .string(session.id)])
            if let error = result.error { throw error }
            if sessionID == session.id { reset() }
            await loadSessions()
        } catch { errorMessage = String(describing: error) }
    }

    public func clearSessions() async {
        do {
            let result = try await daemon.invoke(.uiChatSessionsClear, args: nil)
            if let error = result.error { throw error }
            reset(); await loadSessions()
        } catch { errorMessage = String(describing: error) }
    }

    public func branch(from messageID: UUID) {
        guard !isStreaming, let index = messages.firstIndex(where: { $0.id == messageID }) else { return }
        messages = Array(messages.prefix(index + 1))
        sessionID = nil
        queuedPrompts = []
        seedHistoryPending = true
    }

    public func edit(_ messageID: UUID, content: String) {
        guard !isStreaming, let index = messages.firstIndex(where: { $0.id == messageID }), messages[index].role == .user else { return }
        messages = Array(messages.prefix(index))
        draft = content
        sessionID = nil
        queuedPrompts = []
        seedHistoryPending = true
    }

    public func editAndRetry(_ messageID: UUID, content: String) async {
        guard !isStreaming,
              messages.contains(where: { $0.id == messageID && $0.role == .user }) else { return }
        edit(messageID, content: content)
        await send()
    }

    public var markdownExport: String {
        messages.map { message in
            let heading = message.role == .user ? "## You" : "## Kassiber"
            return "\(heading)\n\n\(message.content)"
        }.joined(separator: "\n\n---\n\n") + "\n"
    }

    public func stop() async {
        guard let currentRequestID else { return }
        do {
            _ = try await daemon.invoke(
                .aiChatCancel,
                args: ["target_request_id": .string(currentRequestID)]
            )
            updateAssistant { $0.status = .cancelled }
        } catch { errorMessage = String(describing: error) }
    }

    public func decideConsent(_ decision: String) async {
        guard let request = pendingConsent else { return }
        pendingConsent = nil
        do {
            let envelope = try await daemon.invoke(
                .aiToolCallConsent,
                args: [
                    "target_request_id": .string(request.targetRequestID),
                    "call_id": .string(request.callID),
                    "decision": .string(decision),
                ]
            )
            if let error = envelope.error { throw error }
            updateTool(callID: request.callID) { tool in
                tool.status = decision == "deny" ? "denied" : "running"
            }
        } catch { errorMessage = String(describing: error) }
    }

    public func reset() {
        guard !isStreaming else { return }
        messages = []
        errorMessage = nil
        sessionID = nil
        queuedPrompts = []
        seedHistoryPending = false
    }

    /// Drops every in-memory chat artifact at the app-lock boundary and
    /// invalidates a suspended stream before it can repopulate the transcript.
    public func clearForAppLock() async {
        conversationEpoch &+= 1
        if let targetRequestID = currentRequestID {
            _ = try? await daemon.invoke(
                .aiChatCancel,
                args: ["target_request_id": .string(targetRequestID)]
            )
        }
        draft = ""
        messages = []
        queuedPrompts = []
        pendingConsent = nil
        errorMessage = nil
        sessionID = nil
        currentRequestID = nil
        activeAssistantID = nil
        isStreaming = false
        seedHistoryPending = false
        thinkParser = ChatThinkParser()
    }

    public func loadPreviewConversation() {
        guard messages.isEmpty else { return }
        messages = [
            ChatMessage(role: .user, content: "Summarize my book readiness."),
            ChatMessage(
                role: .assistant,
                content: "## Book readiness\n\nYour journals are current. **Three transfer candidates** still need review before the final handoff.",
                tools: [ChatToolCall(
                    id: "preview-tool", name: "ui.overview.snapshot", kindClass: "read_only",
                    status: "done", summary: "", arguments: .object([:]), result: nil
                )]
            ),
        ]
    }

    private func chatArgs() -> [String: JSONValue] {
        let transcript: [JSONValue] = messages.filter { $0.role != .assistant || !$0.content.isEmpty }.map { message in
            .object(["role": .string(message.role.rawValue), "content": .string(message.content)])
        }
        var args: [String: JSONValue] = [
            "messages": .array(transcript),
            "tools_enabled": .bool(readToolsEnabled),
            "system_prompt_kind": .string("kassiber"),
            "persist": incognito && sessionID == nil ? .bool(false) : .string("auto"),
        ]
        if thinkingEffort != .auto {
            args["options"] = .object(["reasoning_effort": .string(thinkingEffort.rawValue)])
        }
        if !provider.isEmpty { args["provider"] = .string(provider) }
        if !model.isEmpty { args["model"] = .string(model) }
        if let sessionID { args["session_id"] = .string(sessionID) }
        if seedHistoryPending, sessionID == nil {
            args["seed_history"] = .bool(true)
        }
        seedHistoryPending = false
        return args
    }

    private func apply(_ record: DaemonRecord, targetRequestID: String) {
        if let error = record.error {
            errorMessage = error.message
            updateAssistant { $0.status = .error }
            return
        }
        let data = record.data?.objectValue ?? [:]
        switch record.kind {
        case "ai.chat.delta":
            let delta = data["delta"]?.objectValue ?? [:]
            var visible = ""
            var thinking = ""
            if let content = delta.string("content") {
                let split = thinkParser.feed(content)
                visible += split.content
                thinking += split.thinking
            }
            thinking += delta.string("reasoning") ?? ""
            if !visible.isEmpty { updateAssistant { $0.content += visible } }
            if !thinking.isEmpty { appendThinking(thinking) }
        case "ai.chat.status":
            // Preserve the stable phase for presentation-boundary localization.
            // The daemon's English label remains a fallback for legacy records.
            let phase = data.string("phase") ?? ""
            if phase == "waiting_for_model" || phase == "thinking" {
                beginThinkingSegment()
            }
            updateAssistant { $0.activity = phase.isEmpty ? (data.string("label") ?? "") : phase }
        case "ai.chat.tool_call":
            guard let callID = data.string("call_id"), let name = data.string("name") else { return }
            upsertTool(ChatToolCall(
                id: callID, name: name, kindClass: data.string("kind_class") ?? "unknown",
                status: data.bool("needs_consent") == true ? "pending" : "running", summary: "",
                arguments: data["arguments"] ?? .object([:]), result: nil
            ))
        case "ai.chat.tool_consent_required":
            guard let callID = data.string("call_id"), let name = data.string("name") else { return }
            let summary = data.string("summary") ?? name
            let arguments = data["arguments_preview"] ?? .object([:])
            upsertTool(ChatToolCall(
                id: callID, name: name, kindClass: "mutating", status: "awaiting_consent",
                summary: summary, arguments: arguments, result: nil
            ))
            pendingConsent = ToolConsentRequest(
                targetRequestID: targetRequestID, callID: callID, name: name,
                summary: summary, arguments: arguments
            )
        case "ai.chat.tool_result":
            guard let callID = data.string("call_id") else { return }
            updateTool(callID: callID) { tool in
                tool.status = data.bool("ok") == true ? "done" : (data.string("reason") == "user_denied" ? "denied" : "error")
                tool.result = data["envelope"] ?? data["message"]
            }
        case DaemonKind.aiChat.rawValue:
            let tail = thinkParser.flush()
            if !tail.content.isEmpty { updateAssistant { $0.content += tail.content } }
            if !tail.thinking.isEmpty { appendThinking(tail.thinking) }
            let finish = data.string("finish_reason")
            if let stored = data.string("session_id") { sessionID = stored }
            updateAssistant { $0.status = finish == "cancelled" ? .cancelled : .done; $0.activity = "" }
        default: break
        }
    }

    private func updateAssistant(_ change: (inout ChatMessage) -> Void) {
        guard let id = activeAssistantID, let index = messages.firstIndex(where: { $0.id == id }) else { return }
        change(&messages[index])
    }

    private func beginThinkingSegment() {
        updateAssistant { message in
            if message.thinkingSegments.last?.content.isEmpty == true { return }
            message.thinkingSegments.append(ChatThinkingSegment())
        }
    }

    private func appendThinking(_ content: String) {
        guard !content.isEmpty else { return }
        updateAssistant { message in
            if message.thinkingSegments.isEmpty {
                message.thinkingSegments.append(ChatThinkingSegment(content: content))
            } else {
                message.thinkingSegments[message.thinkingSegments.count - 1].content += content
            }
        }
    }

    private func upsertTool(_ tool: ChatToolCall) {
        updateAssistant { message in
            if let index = message.tools.firstIndex(where: { $0.id == tool.id }) {
                message.tools[index] = tool
            } else { message.tools.append(tool) }
        }
    }

    private func updateTool(callID: String, _ change: (inout ChatToolCall) -> Void) {
        updateAssistant { message in
            guard let index = message.tools.firstIndex(where: { $0.id == callID }) else { return }
            change(&message.tools[index])
        }
    }
}

private struct ChatThinkSplit {
    var content = ""
    var thinking = ""
}

/// Incremental splitter for local models that stream reasoning inside
/// `<think>...</think>` rather than the structured `reasoning` delta field.
private struct ChatThinkParser {
    private var insideThinking = false
    private var pending = ""

    mutating func feed(_ chunk: String) -> ChatThinkSplit {
        var source = pending + chunk
        pending = ""
        var output = ChatThinkSplit()
        while !source.isEmpty {
            let tag = insideThinking ? "</think>" : "<think>"
            if let range = source.range(of: tag) {
                append(String(source[..<range.lowerBound]), to: &output)
                source = String(source[range.upperBound...])
                insideThinking.toggle()
                continue
            }
            var prefixLength = min(source.count, max(0, tag.count - 1))
            while prefixLength > 0, !tag.hasPrefix(String(source.suffix(prefixLength))) {
                prefixLength -= 1
            }
            let flushCount = source.count - prefixLength
            if flushCount > 0 { append(String(source.prefix(flushCount)), to: &output) }
            pending = prefixLength > 0 ? String(source.suffix(prefixLength)) : ""
            source = ""
        }
        return output
    }

    mutating func flush() -> ChatThinkSplit {
        var output = ChatThinkSplit()
        append(pending, to: &output)
        pending = ""
        return output
    }

    private func append(_ value: String, to output: inout ChatThinkSplit) {
        if insideThinking { output.thinking += value } else { output.content += value }
    }
}
