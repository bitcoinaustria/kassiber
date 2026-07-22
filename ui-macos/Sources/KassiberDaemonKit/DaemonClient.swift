import Foundation

public struct DaemonStreamSession: Sendable {
    public let requestID: String
    public let records: AsyncThrowingStream<DaemonRecord, Error>

    public init(requestID: String, records: AsyncThrowingStream<DaemonRecord, Error>) {
        self.requestID = requestID
        self.records = records
    }
}

public protocol DaemonClient: Sendable {
    func invoke(
        _ kind: DaemonKind,
        args: [String: JSONValue]?
    ) async throws -> DaemonEnvelope

    /// Includes intermediate records and the terminal record. A terminal
    /// record has the requested kind, `error`, or `auth_required`.
    func stream(
        _ kind: DaemonKind,
        args: [String: JSONValue]?
    ) async throws -> AsyncThrowingStream<DaemonRecord, Error>

    /// Starts a stream and exposes the request id used by daemon control
    /// messages such as ai.chat.cancel and ai.tool_call.consent.
    func streamSession(
        _ kind: DaemonKind,
        args: [String: JSONValue]?
    ) async throws -> DaemonStreamSession

    /// Unsolicited records carry `event: true` and never a request id.
    func events() async -> AsyncStream<DaemonRecord>
}

/// Optional desktop-host capability used by the native project-folder import
/// flow. It restarts the same supervised sidecar against a validated data root.
public protocol DaemonDataRootSwitching: Sendable {
    func activateDataRoot(_ dataRoot: String) async throws
    func clearActivatedDataRoot() async throws
}

/// Host-owned privacy gate for model-calling daemon kinds. Provider and chat
/// history management deliberately remain on the ordinary desktop surface.
public protocol DaemonRuntimeControlling: Sendable {
    func setAIFeaturesEnabled(_ enabled: Bool) async
    func areAIFeaturesEnabled() async -> Bool
}

/// Optional RAM-only diagnostics surface exposed by native process
/// supervisors. LogsViewModel folds this ring beside `ui.logs.snapshot`, just
/// like the Tauri bridge folds Rust supervisor lifecycle records.
public struct DaemonLifecycleRecord: Equatable, Sendable {
    public let id: Int64
    public let timestamp: Date
    public let event: String
    public let detail: String
    public let stderrTail: String
    public let source: String

    public init(
        id: Int64,
        timestamp: Date,
        event: String,
        detail: String,
        stderrTail: String = "",
        source: String = ""
    ) {
        self.id = id
        self.timestamp = timestamp
        self.event = event
        self.detail = detail
        self.stderrTail = stderrTail
        self.source = source
    }
}

public struct DaemonLifecycleSnapshot: Equatable, Sendable {
    public let records: [DaemonLifecycleRecord]
    public let lastID: Int64

    public init(records: [DaemonLifecycleRecord], lastID: Int64) {
        self.records = records
        self.lastID = lastID
    }
}

public protocol DaemonLifecycleSource: Sendable {
    func lifecycleSnapshot(afterID: Int64) async -> DaemonLifecycleSnapshot
}

public extension DaemonClient {
    func invoke(_ kind: DaemonKind) async throws -> DaemonEnvelope {
        try await invoke(kind, args: nil)
    }

    func stream(_ kind: DaemonKind) async throws -> AsyncThrowingStream<DaemonRecord, Error> {
        try await stream(kind, args: nil)
    }

    func streamSession(_ kind: DaemonKind) async throws -> DaemonStreamSession {
        try await streamSession(kind, args: nil)
    }
}

/// Lightweight deterministic transport used by view-model tests and previews.
public actor ScriptedDaemonClient: DaemonClient, DaemonDataRootSwitching {
    public struct Call: Equatable, Sendable {
        public let kind: DaemonKind
        public let args: [String: JSONValue]?
    }

    private var scripts: [DaemonKind: [DaemonRecord]]
    private var callsStorage: [Call] = []
    private var eventContinuations: [UUID: AsyncStream<DaemonRecord>.Continuation] = [:]
    private var activatedDataRootsStorage: [String] = []
    private var clearActivatedDataRootCountStorage = 0

    public init(scripts: [DaemonKind: [DaemonRecord]] = [:]) {
        self.scripts = scripts
    }

    public func set(_ records: [DaemonRecord], for kind: DaemonKind) {
        scripts[kind] = records
    }

    public func calls() -> [Call] { callsStorage }
    public func activatedDataRoots() -> [String] { activatedDataRootsStorage }
    public func clearActivatedDataRootCount() -> Int { clearActivatedDataRootCountStorage }

    public func activateDataRoot(_ dataRoot: String) async throws {
        activatedDataRootsStorage.append(dataRoot)
    }

    public func clearActivatedDataRoot() async throws {
        clearActivatedDataRootCountStorage += 1
    }

    public func invoke(
        _ kind: DaemonKind,
        args: [String: JSONValue]?
    ) async throws -> DaemonEnvelope {
        callsStorage.append(Call(kind: kind, args: args))
        guard let record = scripts[kind]?.last else {
            throw DaemonClientError.transport("no scripted response for \(kind.rawValue)")
        }
        return record
    }

    public func stream(
        _ kind: DaemonKind,
        args: [String: JSONValue]?
    ) async throws -> AsyncThrowingStream<DaemonRecord, Error> {
        callsStorage.append(Call(kind: kind, args: args))
        let records = scripts[kind] ?? []
        return AsyncThrowingStream { continuation in
            for record in records { continuation.yield(record) }
            continuation.finish()
        }
    }

    public func streamSession(
        _ kind: DaemonKind,
        args: [String: JSONValue]?
    ) async throws -> DaemonStreamSession {
        DaemonStreamSession(
            requestID: "scripted-\(callsStorage.count + 1)",
            records: try await stream(kind, args: args)
        )
    }

    public func events() async -> AsyncStream<DaemonRecord> {
        let id = UUID()
        let (stream, continuation) = AsyncStream<DaemonRecord>.makeStream()
        eventContinuations[id] = continuation
        continuation.onTermination = { [weak self] _ in
            Task { await self?.removeEventContinuation(id) }
        }
        return stream
    }

    public func emit(event: DaemonRecord) {
        for continuation in eventContinuations.values { continuation.yield(event) }
    }

    private func removeEventContinuation(_ id: UUID) {
        eventContinuations.removeValue(forKey: id)
    }
}
