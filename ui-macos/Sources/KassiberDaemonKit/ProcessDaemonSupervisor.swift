import Foundation

public struct DaemonLaunchConfiguration: Equatable, Sendable {
    public let executable: URL
    public let arguments: [String]
    public let workingDirectory: URL
    public let environment: [String: String]?

    public init(
        executable: URL,
        arguments: [String],
        workingDirectory: URL,
        environment: [String: String]? = nil
    ) {
        self.executable = executable
        self.arguments = arguments
        self.workingDirectory = workingDirectory
        self.environment = environment
    }

    /// Uses the repository virtualenv when present and otherwise delegates to
    /// `uv`, matching the Tauri development bridge.
    public static func repositoryDevelopment(repoRoot: URL) -> Self {
        let environment = ProcessInfo.processInfo.environment
        let dataRootArguments: [String]
        if let dataRoot = environment["KASSIBER_DATA_ROOT"], !dataRoot.isEmpty {
            dataRootArguments = ["--data-root", dataRoot]
        } else {
            dataRootArguments = []
        }
        if let configured = environment["KASSIBER_PYTHON"], !configured.isEmpty {
            return Self(
                executable: URL(fileURLWithPath: configured),
                arguments: ["-m", "kassiber"] + dataRootArguments + ["daemon"],
                workingDirectory: repoRoot
            )
        }
        let virtualenvPython = repoRoot.appending(path: ".venv/bin/python")
        if FileManager.default.isExecutableFile(atPath: virtualenvPython.path) {
            return Self(
                executable: virtualenvPython,
                arguments: ["-m", "kassiber"] + dataRootArguments + ["daemon"],
                workingDirectory: repoRoot
            )
        }
        return Self(
            executable: URL(fileURLWithPath: "/usr/bin/env"),
            arguments: ["uv", "run", "python", "-m", "kassiber"] + dataRootArguments + ["daemon"],
            workingDirectory: repoRoot
        )
    }

    /// Bundles carry a self-contained sidecar in Resources.  PyInstaller's
    /// macOS one-file bootloader exits if its executable is replaced or moved
    /// while it is running (which can happen when an app update replaces the
    /// .app bundle).  Copy it to a per-launch stable runtime directory before
    /// spawning so an update cannot invalidate the daemon underneath the app.
    public static func bundled(sidecar: URL, resources: URL) -> Self {
        let environment = ProcessInfo.processInfo.environment
        let dataRootArguments: [String]
        if let dataRoot = environment["KASSIBER_DATA_ROOT"], !dataRoot.isEmpty {
            dataRootArguments = ["--data-root", dataRoot]
        } else {
            dataRootArguments = []
        }
        let runtime = stableBundledSidecar(sidecar)
        return Self(
            executable: runtime?.executable ?? sidecar,
            arguments: dataRootArguments + ["daemon"],
            workingDirectory: runtime?.workingDirectory ?? resources
        )
    }

    private static func stableBundledSidecar(_ sidecar: URL) -> (
        executable: URL,
        workingDirectory: URL
    )? {
        let fileManager = FileManager.default
        guard let applicationSupport = fileManager.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first else { return nil }
        let runtimeRoot = applicationSupport
            .appending(path: "kassiber_native", directoryHint: .isDirectory)
            .appending(path: "sidecars", directoryHint: .isDirectory)
            .appending(path: UUID().uuidString, directoryHint: .isDirectory)
        do {
            try fileManager.createDirectory(
                at: runtimeRoot,
                withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700]
            )
            let executable = runtimeRoot.appending(path: sidecar.lastPathComponent)
            // A hard link keeps the inode alive when an updater replaces the
            // app bundle, without duplicating the large one-file sidecar on
            // every launch.  Fall back to a copy when the app and support
            // directory are on different volumes.
            do {
                try fileManager.linkItem(at: sidecar, to: executable)
            } catch {
                try fileManager.copyItem(at: sidecar, to: executable)
            }
            try fileManager.setAttributes(
                [.posixPermissions: 0o700],
                ofItemAtPath: executable.path
            )
            return (executable, runtimeRoot)
        } catch {
            // The app can still run from Contents/Resources when the user's
            // Application Support volume is unavailable or read-only.
            return nil
        }
    }

    fileprivate func settingDataRoot(_ dataRoot: String) -> Self {
        var next = arguments
        while let index = next.firstIndex(of: "--data-root") {
            let end = min(index + 2, next.count)
            next.removeSubrange(index..<end)
        }
        let insertion = next.firstIndex(of: "daemon") ?? next.count
        next.insert(contentsOf: ["--data-root", dataRoot], at: insertion)
        return Self(
            executable: executable,
            arguments: next,
            workingDirectory: workingDirectory,
            environment: environment
        )
    }
}

/// Foundation-only JSONL sidecar supervisor. It mirrors the Tauri supervisor's
/// routing rules: one long-lived process, request-id demultiplexing, strict
/// startup ordering, interleaved stream records, and separate unsolicited
/// event delivery.
public actor ProcessDaemonSupervisor: DaemonClient, DaemonLifecycleSource, DaemonDataRootSwitching, DaemonRuntimeControlling {
    private struct PendingRequest {
        let kind: DaemonKind
        let streaming: Bool
        let continuation: AsyncThrowingStream<DaemonRecord, Error>.Continuation
        var timeout: Task<Void, Never>
    }

    private var configuration: DaemonLaunchConfiguration
    private let initialConfiguration: DaemonLaunchConfiguration
    private let nativeSecretStore: any DesktopNativeSecretStore
    private var process: Process?
    private var stdin: FileHandle?
    private var stdout: FileHandle?
    private var stderr: FileHandle?
    private var stdoutPump: Task<Void, Never>?
    private var stderrPump: Task<Void, Never>?
    private var stdoutFramer = JSONLFramer()
    private var nextRequestID: UInt64 = 1
    private var pending: [String: PendingRequest] = [:]
    private var ready = false
    private var readyWaiters: [UUID: CheckedContinuation<Void, Error>] = [:]
    private var startupFailure: Error?
    private var eventContinuations: [UUID: AsyncStream<DaemonRecord>.Continuation] = [:]
    private var stderrTail = Data()
    private let stderrTailLimit = 16 * 1024
    private var lifecycleRing: [DaemonLifecycleRecord] = []
    private var nextLifecycleID: Int64 = 1
    private let lifecycleCapacity = 64
    private var aiFeaturesEnabled = false

    public init(
        configuration: DaemonLaunchConfiguration,
        nativeSecretStore: any DesktopNativeSecretStore = UnavailableNativeSecretStore()
    ) {
        self.configuration = configuration
        self.initialConfiguration = configuration
        self.nativeSecretStore = nativeSecretStore
    }

    public func invoke(
        _ kind: DaemonKind,
        args: [String: JSONValue]?
    ) async throws -> DaemonEnvelope {
        let session = try await openStream(
            kind,
            args: args,
            streaming: Self.longRunningKinds.contains(kind)
        )
        for try await record in session.records {
            if isTerminal(record, for: kind) { return record }
        }
        throw DaemonClientError.daemonExited("daemon stream ended without a terminal envelope")
    }

    public func stream(
        _ kind: DaemonKind,
        args: [String: JSONValue]?
    ) async throws -> AsyncThrowingStream<DaemonRecord, Error> {
        try await openStream(kind, args: args, streaming: true).records
    }

    public func streamSession(
        _ kind: DaemonKind,
        args: [String: JSONValue]?
    ) async throws -> DaemonStreamSession {
        try await openStream(kind, args: args, streaming: true)
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

    public func redactedStderrSnapshot() -> String {
        String(decoding: stderrTail, as: UTF8.self)
    }

    public func lifecycleSnapshot(afterID: Int64) -> DaemonLifecycleSnapshot {
        DaemonLifecycleSnapshot(
            records: lifecycleRing.filter { $0.id > afterID },
            lastID: lifecycleRing.last?.id ?? 0
        )
    }

    public func setAIFeaturesEnabled(_ enabled: Bool) async {
        aiFeaturesEnabled = enabled
        if !enabled { cancelActiveAIChats() }
        broadcastHostEvent(kind: "native.ai_runtime_state", data: ["enabled": .bool(enabled)])
    }

    public func areAIFeaturesEnabled() async -> Bool { aiFeaturesEnabled }

    public func shutdown(recordExit: Bool = true) {
        if recordExit {
            recordLifecycle(event: "exited", detail: "status: 0 native supervisor shut down")
        }
        stdout?.readabilityHandler = nil
        stderr?.readabilityHandler = nil
        stdoutPump?.cancel()
        stderrPump?.cancel()
        stdoutPump = nil
        stderrPump = nil
        process?.terminate()
        process = nil
        stdin = nil
        stdout = nil
        stderr = nil
        ready = false
        let error = DaemonClientError.daemonExited("daemon was shut down")
        let waiters = Array(readyWaiters.values)
        readyWaiters.removeAll()
        for waiter in waiters { waiter.resume(throwing: error) }
        failAll(error)
    }

    public func activateDataRoot(_ dataRoot: String) async throws {
        let normalized = URL(fileURLWithPath: dataRoot, isDirectory: true)
            .resolvingSymlinksInPath().standardizedFileURL.path
        guard !normalized.isEmpty else {
            throw DaemonClientError.transport("imported project data root is empty")
        }
        restart(using: configuration.settingDataRoot(normalized))
    }

    public func clearActivatedDataRoot() async throws {
        restart(using: initialConfiguration)
    }

    private func restart(using nextConfiguration: DaemonLaunchConfiguration) {
        shutdown(recordExit: false)
        recordLifecycle(event: "replaced", detail: "data root changed")
        configuration = nextConfiguration
        startupFailure = nil
        stdoutFramer = JSONLFramer()
        nextRequestID = 1
    }

    private func openStream(
        _ kind: DaemonKind,
        args: [String: JSONValue]?,
        streaming: Bool
    ) async throws -> DaemonStreamSession {
        if let denial = DesktopDaemonAccessPolicy.denial(
            for: kind,
            aiFeaturesEnabled: aiFeaturesEnabled
        ) {
            return deniedSession(kind: kind, denial: denial)
        }
        try await ensureProcess()
        let requestID = "macos-\(nextRequestID)"
        nextRequestID &+= 1
        guard pending[requestID] == nil else {
            throw DaemonClientError.requestConflict(requestID)
        }

        let (stream, continuation) = AsyncThrowingStream<DaemonRecord, Error>.makeStream()
        let timeoutSeconds: UInt64 = streaming || Self.longRunningKinds.contains(kind) ? 90 : 15
        let timeout = Task { [weak self] in
            try? await Task.sleep(nanoseconds: timeoutSeconds * 1_000_000_000)
            guard !Task.isCancelled else { return }
            await self?.expire(requestID)
        }
        pending[requestID] = PendingRequest(
            kind: kind,
            streaming: streaming,
            continuation: continuation,
            timeout: timeout
        )
        broadcastRequestActivity(requestID: requestID, kind: kind, state: "started")
        continuation.onTermination = { [weak self] _ in
            Task { await self?.abandon(requestID) }
        }

        do {
            let request = DaemonRequest(
                kind: kind,
                requestID: requestID,
                args: augmentAIProviderSecretArguments(kind: kind, args: args)
            )
            try write(JSONLCodec.encode(request))
        } catch {
            finish(requestID, throwing: error)
            throw error
        }
        return DaemonStreamSession(requestID: requestID, records: stream)
    }

    private func deniedSession(kind: DaemonKind, denial: DaemonRecord) -> DaemonStreamSession {
        let requestID = "macos-denied-\(nextRequestID)"
        nextRequestID &+= 1
        let record = DaemonRecord(
            kind: denial.kind,
            schemaVersion: denial.schemaVersion,
            requestID: .string(requestID),
            data: denial.data,
            error: denial.error
        )
        let stream = AsyncThrowingStream<DaemonRecord, Error> { continuation in
            continuation.yield(record)
            continuation.finish()
        }
        return DaemonStreamSession(requestID: requestID, records: stream)
    }

    private func ensureProcess() async throws {
        if ready { return }
        if let startupFailure { throw startupFailure }
        if process == nil { try launch() }
        if ready { return }
        try await withThrowingTaskGroup(of: Void.self) { group in
            group.addTask { [weak self] in
                guard let self else { throw CancellationError() }
                try await self.waitUntilReady()
            }
            group.addTask {
                try await Task.sleep(for: .seconds(30))
                await self.recordLifecycle(event: "spawn_failed", detail: "daemon did not become ready within 30 seconds")
                throw DaemonClientError.daemonNotReady("daemon did not become ready within 30 seconds")
            }
            _ = try await group.next()
            group.cancelAll()
        }
    }

    private func waitUntilReady() async throws {
        let id = UUID()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                appendReadyWaiter(id, continuation)
            }
        } onCancel: {
            Task { await self.cancelReadyWaiter(id) }
        }
    }

    private func appendReadyWaiter(
        _ id: UUID,
        _ continuation: CheckedContinuation<Void, Error>
    ) {
        if ready {
            continuation.resume()
        } else if let startupFailure {
            continuation.resume(throwing: startupFailure)
        } else {
            readyWaiters[id] = continuation
        }
    }

    private func cancelReadyWaiter(_ id: UUID) {
        readyWaiters.removeValue(forKey: id)?.resume(throwing: CancellationError())
    }

    private func launch() throws {
        let process = Process()
        let stdinPipe = Pipe()
        let stdoutPipe = Pipe()
        let stderrPipe = Pipe()
        process.executableURL = configuration.executable
        process.arguments = configuration.arguments
        process.currentDirectoryURL = configuration.workingDirectory
        if let environment = configuration.environment { process.environment = environment }
        process.standardInput = stdinPipe
        process.standardOutput = stdoutPipe
        process.standardError = stderrPipe
        do {
            try process.run()
        } catch {
            recordLifecycle(event: "spawn_failed", detail: "could not spawn daemon: \(error)", source: configuration.executable.lastPathComponent)
            throw DaemonClientError.daemonNotReady("could not spawn daemon: \(error)")
        }
        recordLifecycle(event: "spawned", detail: "daemon process started", source: configuration.executable.lastPathComponent)
        self.process = process
        stdin = stdinPipe.fileHandleForWriting

        let stdout = stdoutPipe.fileHandleForReading
        self.stdout = stdout
        let (stdoutChunks, stdoutContinuation) = AsyncStream<Data>.makeStream()
        stdout.readabilityHandler = { handle in
            let data = handle.availableData
            if data.isEmpty {
                handle.readabilityHandler = nil
                stdoutContinuation.finish()
            } else {
                stdoutContinuation.yield(data)
            }
        }
        stdoutPump = Task { [weak self] in
            for await data in stdoutChunks {
                guard !Task.isCancelled else { return }
                await self?.receiveStdout(data)
            }
            guard !Task.isCancelled else { return }
            await self?.stdoutEnded()
        }
        let stderr = stderrPipe.fileHandleForReading
        self.stderr = stderr
        let (stderrChunks, stderrContinuation) = AsyncStream<Data>.makeStream()
        stderr.readabilityHandler = { handle in
            let data = handle.availableData
            if data.isEmpty {
                handle.readabilityHandler = nil
                stderrContinuation.finish()
            } else {
                stderrContinuation.yield(data)
            }
        }
        stderrPump = Task { [weak self] in
            for await data in stderrChunks {
                guard !Task.isCancelled else { return }
                await self?.receiveStderr(data)
            }
        }
    }

    private func receiveStdout(_ data: Data) {
        for line in stdoutFramer.append(data) {
            do {
                try route(JSONLCodec.decodeRecord(line))
            } catch {
                protocolFailure(error)
                return
            }
        }
    }

    private func route(_ originalRecord: DaemonRecord) throws {
        var record = originalRecord
        if record.kind == "daemon.ready", record.requestID == nil {
            guard !ready else {
                throw DaemonClientError.protocolError("daemon emitted daemon.ready twice")
            }
            ready = true
            recordLifecycle(event: "spawned", detail: "daemon ready", source: configuration.executable.lastPathComponent)
            for waiter in readyWaiters.values { waiter.resume() }
            readyWaiters.removeAll()
            return
        }
        guard ready else {
            throw DaemonClientError.protocolError("daemon response arrived before daemon.ready")
        }
        if record.kind == "supervisor.ai_secret_store.request" {
            try replyToSecretStoreRequest(record)
            return
        }
        if record.kind == DaemonKind.aiProvidersList.rawValue {
            record = attachingSecretStorePolicy(to: record)
        }
        if record.kind == "auth_required" {
            broadcastHostEvent(kind: "native.auth_required", data: record.data)
        } else if record.kind == DaemonKind.daemonLock.rawValue {
            broadcastHostEvent(kind: "native.daemon_locked", data: record.data)
        }
        if record.event == true {
            guard record.requestID == nil, !record.kind.isEmpty else {
                throw DaemonClientError.protocolError("malformed unsolicited daemon event")
            }
            for continuation in eventContinuations.values { continuation.yield(record) }
            return
        }
        guard let requestID = record.requestIDString else {
            throw DaemonClientError.protocolError("daemon response is missing request_id")
        }
        // Late records after a non-fatal timeout are expected and discarded.
        guard let request = pending[requestID] else { return }
        if isTerminal(record, for: request.kind) {
            request.continuation.yield(record)
            finish(requestID, terminal: record)
            return
        }
        broadcastRequestActivity(
            requestID: requestID,
            kind: request.kind,
            state: "progress",
            detail: activityDetail(record)
        )
        if request.streaming {
            resetInactivityTimeout(requestID)
            request.continuation.yield(record)
        } else if record.kind != "progress" {
            throw DaemonClientError.protocolError(
                "non-streaming \(request.kind.rawValue) emitted \(record.kind)"
            )
        }
    }

    private func replyToSecretStoreRequest(_ record: DaemonRecord) throws {
        let result: Result<JSONValue, Error> = Result { try handleSecretStoreOperation(record.data) }
        let response: JSONValue
        switch result {
        case let .success(data):
            response = .object([
                "kind": .string("supervisor.ai_secret_store.response"),
                "schema_version": .integer(1), "request_id": record.requestID ?? .null,
                "data": data,
            ])
        case let .failure(error):
            response = .object([
                "kind": .string("supervisor.ai_secret_store.response"),
                "schema_version": .integer(1), "request_id": record.requestID ?? .null,
                "error": .object([
                    "code": .string("secret_store_bridge_error"),
                    "message": .string(Self.redact(String(describing: error))),
                    "retryable": .bool(true),
                ]),
            ])
        }
        try write(JSONLCodec.encode(response))
    }

    private func handleSecretStoreOperation(_ value: JSONValue?) throws -> JSONValue {
        guard let data = value?.objectValue else {
            throw NativeSecretStoreError.operation("secret-store bridge request missing data")
        }
        guard let provider = data["provider_name"]?.stringValue, !provider.trimmingCharacters(in: .whitespaces).isEmpty else {
            throw NativeSecretStoreError.operation("secret-store bridge request missing provider_name")
        }
        guard let operation = data["op"]?.stringValue else {
            throw NativeSecretStoreError.operation("secret-store bridge request missing op")
        }
        let storeID = data["store_id"]?.stringValue ?? NativeSecretStorePolicy.inlineStoreID
        guard storeID != NativeSecretStorePolicy.inlineStoreID else {
            throw NativeSecretStoreError.operation("sqlcipher_inline refs are handled by the Python daemon")
        }
        guard storeID == NativeSecretStoreIdentifiers.macOSKeychain else {
            throw NativeSecretStoreError.operation("requested AI provider secret store is not available on macOS")
        }
        if operation == "availability" {
            return .object([
                "provider_name": .string(provider),
                "availability": nativeSecretStore.availability.jsonValue,
            ])
        }
        let (service, account) = try validateSecretReference(data, provider: provider)
        switch operation {
        case "get":
            guard let bytes = try nativeSecretStore.get(service: service, account: account) else {
                return ["provider_name": .string(provider), "state": "missing", "secret": .null]
            }
            guard let secret = String(data: bytes, encoding: .utf8) else {
                throw NativeSecretStoreError.operation("stored provider API key is not UTF-8")
            }
            return ["provider_name": .string(provider), "state": "ok", "secret": .string(secret)]
        case "exists":
            let exists = try nativeSecretStore.exists(service: service, account: account)
            return ["provider_name": .string(provider), "state": .string(exists ? "ok" : "missing")]
        case "set":
            guard let secret = data["secret"]?.stringValue else {
                throw NativeSecretStoreError.operation("secret-store set request missing secret")
            }
            try nativeSecretStore.set(service: service, account: account, secret: Data(secret.utf8))
            return ["provider_name": .string(provider), "state": "ok"]
        case "delete":
            try nativeSecretStore.delete(service: service, account: account)
            return ["provider_name": .string(provider), "state": "missing"]
        default:
            throw NativeSecretStoreError.operation("unsupported secret-store bridge operation")
        }
    }

    private func validateSecretReference(
        _ data: [String: JSONValue], provider: String
    ) throws -> (service: String, account: String) {
        guard let service = data["service"]?.stringValue,
              service.count == 64,
              service.utf8.allSatisfy({ byte in
                  (48...57).contains(byte) || (65...70).contains(byte) || (97...102).contains(byte)
              }) else {
            throw NativeSecretStoreError.operation("secret-store service is outside the Kassiber namespace")
        }
        guard let account = data["account"]?.stringValue, account == provider else {
            throw NativeSecretStoreError.operation("secret-store account must match the AI provider name")
        }
        return (service, account)
    }

    private func augmentAIProviderSecretArguments(
        kind: DaemonKind, args: [String: JSONValue]?
    ) -> [String: JSONValue]? {
        guard Self.secretBridgeKinds.contains(kind) else { return args }
        var result = args ?? [:]
        result["_desktop_secret_store_bridge"] = true
        if kind == .aiProvidersSetApiKey || kind == .aiProvidersMoveApiKey {
            let requested = result["store_id"]?.stringValue.flatMap {
                $0 == NativeSecretStorePolicy.inlineStoreID ? nil : $0
            }
            let selection = NativeSecretStorePolicy.select(
                availability: nativeSecretStore.availability,
                requested: requested
            )
            result["_desktop_secret_store_default"] = .string(selection.storeID)
            result["_desktop_secret_store_policy"] = selection.jsonValue
        }
        return result
    }

    private func attachingSecretStorePolicy(to record: DaemonRecord) -> DaemonRecord {
        guard var data = record.data?.objectValue else { return record }
        let availability = nativeSecretStore.availability
        let defaultSelection = NativeSecretStorePolicy.select(availability: availability, requested: nil)
        data["secret_store_policy"] = .object([
            "platform": "macos",
            "availability": availability.jsonValue,
            "default": defaultSelection.jsonValue,
            "policy": .object([
                "native_store": .string(NativeSecretStoreIdentifiers.macOSKeychain),
                "sqlcipher_store": .string(NativeSecretStorePolicy.inlineStoreID),
            ]),
        ])
        return DaemonRecord(
            kind: record.kind, schemaVersion: record.schemaVersion,
            requestID: record.requestID, event: record.event,
            data: .object(data), error: record.error
        )
    }

    private func receiveStderr(_ data: Data) {
        let redacted = Data(Self.redact(String(decoding: data, as: UTF8.self)).utf8)
        stderrTail.append(redacted)
        if stderrTail.count > stderrTailLimit {
            stderrTail.removeFirst(stderrTail.count - stderrTailLimit)
        }
    }

    private func stdoutEnded() {
        guard process != nil else { return }
        protocolFailure(DaemonClientError.daemonExited("Python daemon closed stdout"))
    }

    private func protocolFailure(_ error: Error) {
        recordLifecycle(
            event: "killed",
            detail: String(describing: error),
            stderrTail: String(decoding: stderrTail, as: UTF8.self),
            source: configuration.executable.lastPathComponent
        )
        startupFailure = error
        ready = false
        process?.terminate()
        process = nil
        stdin = nil
        stdout?.readabilityHandler = nil
        stderr?.readabilityHandler = nil
        stdout = nil
        stderr = nil
        for waiter in readyWaiters.values { waiter.resume(throwing: error) }
        readyWaiters.removeAll()
        failAll(error)
    }

    private func write(_ data: Data) throws {
        guard let stdin else {
            throw DaemonClientError.daemonExited("daemon stdin is unavailable")
        }
        do {
            try stdin.write(contentsOf: data)
        } catch {
            throw DaemonClientError.transport("could not write daemon request: \(error)")
        }
    }

    private func finish(
        _ requestID: String,
        throwing error: Error? = nil,
        terminal: DaemonRecord? = nil
    ) {
        guard let request = pending.removeValue(forKey: requestID) else { return }
        request.timeout.cancel()
        let terminalError = terminal?.error
        let authenticationFailure = terminal?.kind == "auth_required"
        let failed = error != nil || terminalError != nil || authenticationFailure
        broadcastRequestActivity(
            requestID: requestID,
            kind: request.kind,
            state: failed ? "failed" : "finished",
            detail: terminalError?.message
                ?? (authenticationFailure ? "Authentication is required." : terminal.map { _ in request.kind.rawValue })
                ?? error.map { String(describing: $0) },
            errorCode: terminalError?.code ?? (authenticationFailure ? "auth_required" : nil)
        )
        if let error {
            request.continuation.finish(throwing: error)
        } else {
            request.continuation.finish()
        }
    }

    private func abandon(_ requestID: String) {
        guard let request = pending.removeValue(forKey: requestID) else { return }
        request.timeout.cancel()
        broadcastRequestActivity(requestID: requestID, kind: request.kind, state: "cancelled")
    }

    private func expire(_ requestID: String) {
        guard let request = pending[requestID] else { return }
        recordLifecycle(
            event: "request_timeout",
            detail: "\(request.kind.rawValue) timed out",
            source: configuration.executable.lastPathComponent
        )
        finish(
            requestID,
            throwing: DaemonClientError.requestTimedOut(
                kind: request.kind.rawValue,
                retryable: !Self.mutatingKinds.contains(request.kind)
            )
        )
    }

    private func resetInactivityTimeout(_ requestID: String) {
        guard pending[requestID]?.streaming == true else { return }
        let timeout = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 90 * 1_000_000_000)
            guard !Task.isCancelled else { return }
            await self?.expire(requestID)
        }
        pending[requestID]?.timeout.cancel()
        pending[requestID]?.timeout = timeout
    }

    private func failAll(_ error: Error) {
        let requestIDs = Array(pending.keys)
        for requestID in requestIDs { finish(requestID, throwing: error) }
    }

    private func removeEventContinuation(_ id: UUID) {
        eventContinuations.removeValue(forKey: id)
    }

    private func recordLifecycle(
        event: String,
        detail: String,
        stderrTail: String = "",
        source: String = ""
    ) {
        lifecycleRing.append(DaemonLifecycleRecord(
            id: nextLifecycleID,
            timestamp: Date(),
            event: event,
            detail: Self.redact(detail),
            stderrTail: Self.redact(stderrTail),
            source: source
        ))
        nextLifecycleID += 1
        if lifecycleRing.count > lifecycleCapacity {
            lifecycleRing.removeFirst(lifecycleRing.count - lifecycleCapacity)
        }
    }

    /// Authentication is a process-global desktop concern. Promote terminal
    /// auth records to the supervisor event stream so a request made by any
    /// screen can immediately replace the entire app with the lock screen.
    private func broadcastHostEvent(kind: String, data: JSONValue?) {
        let event = DaemonRecord(kind: kind, event: true, data: data)
        for continuation in eventContinuations.values { continuation.yield(event) }
    }

    private func cancelActiveAIChats() {
        let chatRequestIDs = pending.compactMap { requestID, request in
            request.kind == .aiChat ? requestID : nil
        }
        for targetRequestID in chatRequestIDs {
            if ready, stdin != nil {
                let controlID = "macos-ai-disable-\(nextRequestID)"
                nextRequestID &+= 1
                let control = DaemonRequest(
                    kind: .aiChatCancel,
                    requestID: controlID,
                    args: ["target_request_id": .string(targetRequestID)]
                )
                try? write(JSONLCodec.encode(control))
            }
            guard let request = pending[targetRequestID] else { continue }
            request.continuation.yield(DaemonRecord(
                kind: DaemonKind.aiChat.rawValue,
                requestID: .string(targetRequestID),
                data: ["finish_reason": "cancelled", "reason": "ai_features_disabled"]
            ))
            finish(targetRequestID)
        }
    }

    private func broadcastRequestActivity(
        requestID: String,
        kind: DaemonKind,
        state: String,
        detail: String? = nil,
        errorCode: String? = nil
    ) {
        var data: [String: JSONValue] = [
            "request_id": .string(requestID),
            "kind": .string(kind.rawValue),
            "state": .string(state),
        ]
        if let detail, !detail.isEmpty { data["detail"] = .string(Self.redact(detail)) }
        if let errorCode, !errorCode.isEmpty {
            data["error_code"] = .string(String(Self.redact(errorCode).prefix(80)))
        }
        broadcastHostEvent(kind: "native.request.activity", data: .object(data))
    }

    private func activityDetail(_ record: DaemonRecord) -> String {
        let data = record.data?.objectValue ?? [:]
        return data["label"]?.stringValue
            ?? data["phase"]?.stringValue
            ?? data["state"]?.stringValue
            ?? record.kind
    }

    private func isTerminal(_ record: DaemonRecord, for kind: DaemonKind) -> Bool {
        record.kind == kind.rawValue || record.kind == "error" || record.kind == "auth_required"
    }

    private static let longRunningKinds: Set<DaemonKind> = [
        .aiChat,
        .uiWalletsSync,
        .uiFreshnessRun,
        .uiWorkspaceFreshnessRun,
        .uiJournalsProcess,
        .uiRatesRebuild,
        .uiSyncPush,
        .uiSyncPull,
        .uiSyncJoin,
    ]

    private static let secretBridgeKinds: Set<DaemonKind> = [
        .aiProvidersList, .aiProvidersGet, .aiProvidersSetApiKey,
        .aiProvidersMoveApiKey, .aiProvidersDelete, .aiListModels,
        .aiTestConnection, .aiChat,
    ]

    private static let mutatingKinds: Set<DaemonKind> = Set(
        DaemonKind.allCases.filter { kind in
            let value = kind.rawValue
            return value.contains(".create")
                || value.contains(".update")
                || value.contains(".delete")
                || value.contains(".import")
                || value.contains(".export")
                || value.contains(".sync")
                || value.contains(".run")
                || value.contains(".process")
                || value.contains(".pair")
                || value.contains(".resolve")
                || value.contains(".configure")
                || value.hasSuffix(".set_api_key")
                || value.hasSuffix(".consent")
        }
    )

    private static func redact(_ input: String) -> String {
        var output = input
        let patterns = [
            #"(?i)(api[_-]?key|token|password|passphrase|secret|cookie|descriptor|mnemonic|seed)\s*[:=]\s*[^\s,}]+"#,
            #"(?i)\"(?:api[_-]?key|token|password|passphrase|secret|cookie|descriptor|mnemonic|seed(?:[_-]?(?:phrase|words))?|xprv)\"\s*:\s*\"[^\"]*\""#,
            #"(?i)bearer\s+[^\s,}]+"#,
            #"(?i)\b(?:https?|tcp|ssl)://[^\s/:@]+:[^@\s/]+@"#,
            #"\b(?:xpub|ypub|zpub|tpub|upub|vpub|xprv|yprv|zprv|tprv|uprv|vprv)[A-Za-z0-9]{20,}\b"#,
            #"(?i)\b(?:wpkh|sh|wsh|tr|pkh|combo|sp)\([^\n]{16,800}\)(?:#[a-z0-9]{8})?"#,
        ]
        for pattern in patterns {
            guard let regex = try? NSRegularExpression(pattern: pattern) else { continue }
            let range = NSRange(output.startIndex..., in: output)
            output = regex.stringByReplacingMatches(
                in: output,
                range: range,
                withTemplate: "[redacted]"
            )
        }
        for wordCount in [24, 21, 18, 15, 12] {
            guard let regex = try? NSRegularExpression(
                pattern: "\\b(?:[a-z]{3,8}\\s+){\(wordCount - 1)}[a-z]{3,8}\\b"
            ) else { continue }
            output = regex.stringByReplacingMatches(
                in: output,
                range: NSRange(output.startIndex..., in: output),
                withTemplate: "[redacted-seed-phrase]"
            )
        }
        return output
    }
}
