import Foundation
import Observation
import KassiberDaemonKit

public enum LogRedactionMode: String, CaseIterable, Identifiable, Sendable {
    case highSignal = "high_signal"
    case publicSafe = "public_safe"

    public var id: String { rawValue }
}

public enum LogExportFormat: String, CaseIterable, Sendable {
    case markdown = "md"
    case plain = "log"
    case jsonl
}

public struct LogField: Equatable, Sendable {
    public let type: String
    public let value: JSONValue

    public init(type: String, value: JSONValue) {
        self.type = type
        self.value = value
    }
}

public enum LogRecordOrigin: String, Equatable, Sendable {
    case daemon
    case supervisor
    case local
}

public struct LogRecordRow: Identifiable, Equatable, Sendable {
    public let id: Int64
    public let timestamp: String
    public let date: Date?
    public let level: String
    public let module: String
    public let file: String
    public let line: Int64
    public let message: String
    public let fields: [String: LogField]
    public let origin: LogRecordOrigin

    public var location: String { "\(module):\(file):\(line)" }
}

public struct LogExportHeader: Equatable, Sendable {
    public let appVersion: String
    public let operatingSystem: String
    public let generatedAt: Date

    public init(appVersion: String, operatingSystem: String, generatedAt: Date = Date()) {
        self.appVersion = appVersion
        self.operatingSystem = operatingSystem
        self.generatedAt = generatedAt
    }
}

/// Native counterpart of `appLogs.ts`: a bounded, RAM-only client ring layered
/// on the daemon's own bounded ring. Nothing in this model writes to disk; only
/// explicit UI export actions receive an export string.
@MainActor
@Observable
public final class LogsViewModel {
    public static let maximumRecords = 10_000
    public static let maximumBytes: Int64 = 4 * 1024 * 1024
    public static let renderStep = 1_000
    public static let maximumRenderedLines = 8_000

    public var query = "" { didSet { resetRendering() } }
    public var useRegex = false { didSet { resetRendering() } }
    public var level = "all" { didSet { resetRendering() } }
    public var module = "all" { didSet { resetRendering() } }
    public var redacted = true { didSet { resetRendering() } }
    public var maskAmounts = false { didSet { resetRendering() } }
    public var redactionMode: LogRedactionMode = .publicSafe { didSet { resetRendering() } }
    public var autoscroll = true {
        didSet { if autoscroll { newWhilePaused = 0 } }
    }

    public private(set) var records: [LogRecordRow] = []
    public private(set) var bufferBytes: Int64 = 0
    public private(set) var maxBytes: Int64 = 1
    public private(set) var localBufferBytes: Int64 = 2
    public private(set) var gap = false
    public private(set) var startedAt: String?
    public private(set) var lastID: Int64 = 0
    public private(set) var renderLimit = renderStep
    public private(set) var newWhilePaused = 0
    public private(set) var rawUntil: Date?
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient
    private let amountSalt = UUID().uuidString
    private var lifecycleCursor: Int64 = 0
    private var nextLocalID: Int64 = 8_000_000_000_000_000
    private var bridgeFailing = false

    public init(daemon: any DaemonClient) {
        self.daemon = daemon
    }

    public static let levels = ["trace", "debug", "info", "warning", "error"]

    public var modules: [String] {
        Array(Set(records.map(\.module).filter { !$0.isEmpty })).sorted()
    }

    public var levelCounts: [String: Int] {
        Dictionary(grouping: records, by: \.level).mapValues(\.count)
    }

    public var moduleCounts: [String: Int] {
        Dictionary(grouping: records, by: \.module).mapValues(\.count)
    }

    public var regexError: String? {
        guard useRegex, !query.isEmpty else { return nil }
        do {
            _ = try NSRegularExpression(pattern: query, options: [.caseInsensitive])
            return nil
        } catch {
            return String(describing: error)
        }
    }

    public var filteredRecords: [LogRecordRow] {
        let expression: NSRegularExpression?
        if useRegex, !query.isEmpty {
            expression = try? NSRegularExpression(pattern: query, options: [.caseInsensitive])
            if expression == nil { return [] }
        } else {
            expression = nil
        }
        return records.filter { record in
            guard level == "all" || record.level == level else { return false }
            guard module == "all" || record.module == module else { return false }
            guard !query.isEmpty else { return true }
            let line = format(record)
            if let expression {
                return expression.firstMatch(
                    in: line,
                    range: NSRange(line.startIndex..<line.endIndex, in: line)
                ) != nil
            }
            return line.localizedCaseInsensitiveContains(query)
        }
    }

    /// The newest bounded slice, matching the web view's progressive renderer.
    public var visibleRecords: [LogRecordRow] {
        Array(filteredRecords.suffix(min(renderLimit, Self.maximumRenderedLines)))
    }

    public var bufferPercent: Int {
        guard maxBytes > 0 else { return 0 }
        return min(100, Int((Double(bufferBytes) / Double(maxBytes)) * 100))
    }

    public var activeFilterDescription: String {
        var parts: [String] = []
        if level != "all" { parts.append("level \(level)") }
        if module != "all" { parts.append("module \(module)") }
        if !query.isEmpty { parts.append("\(useRegex ? "regex" : "search") \"\(query)\"") }
        return parts.isEmpty ? "all captured records" : parts.joined(separator: ", ")
    }

    public var timeRangeDescription: String {
        guard let first = filteredRecords.first, let last = filteredRecords.last else { return "empty" }
        return "\(first.timestamp) – \(last.timestamp)"
    }

    public var isRawVisible: Bool { !redacted }

    /// Compatibility entry point used by previews and existing tests.
    public func load() async {
        await poll()
    }

    /// Incrementally folds up to four daemon pages into the local RAM ring.
    public func poll() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }

        do {
            var page = 0
            var cursor = lastID
            while page < 4 {
                let result = try await daemon.invoke(.uiLogsSnapshot, args: [
                    "after_id": .integer(cursor),
                    "limit": .integer(500),
                ])
                if let error = result.error {
                    errorMessage = error.message
                    return
                }

                let object = result.data?.objectValue ?? [:]
                let snapshotStartedAt = object.string("started_at")
                if let current = startedAt,
                   let snapshotStartedAt,
                   current != snapshotStartedAt {
                    records.removeAll { $0.origin == .daemon }
                    enforceLocalBounds()
                    lastID = 0
                    cursor = 0
                    startedAt = snapshotStartedAt
                    page += 1
                    continue
                }
                if startedAt == nil { startedAt = snapshotStartedAt }

                bufferBytes = object.int("buffer_bytes") ?? bufferBytes
                maxBytes = max(1, object.int("max_bytes") ?? maxBytes)
                let snapshotGap = object.bool("gap") ?? false
                if snapshotGap, !gap {
                    emitLocal(
                        level: "info",
                        module: "daemon:bridge",
                        file: "LogsViewModel.swift",
                        message: "Daemon log records were evicted before the bridge caught up",
                        fields: [
                            "after_id": LogField(type: "number", value: .integer(cursor)),
                            "last_id": LogField(type: "number", value: .integer(object.int("last_id") ?? cursor)),
                        ]
                    )
                }
                gap = snapshotGap
                let parsed = object.objects("records").compactMap(Self.parseRecord)
                merge(parsed)

                let responseLastID = object.int("last_id") ?? parsed.last?.id ?? cursor
                if let newest = parsed.last?.id {
                    cursor = max(cursor, newest)
                    lastID = max(lastID, newest)
                }
                if parsed.count < 500 || cursor >= responseLastID {
                    lastID = max(lastID, responseLastID)
                    break
                }
                page += 1
            }
            await pollLifecycle()
            bridgeFailing = false
            errorMessage = nil
        } catch is CancellationError {
            return
        } catch {
            errorMessage = String(describing: error)
            if !bridgeFailing {
                emitLocal(
                    level: "warning",
                    module: "daemon:bridge-poll",
                    file: "LogsViewModel.swift",
                    message: "Daemon log poll failed; backing off",
                    fields: ["error_message": LogField(type: "text", value: .string(String(describing: error)))]
                )
            }
            bridgeFailing = true
            await pollLifecycle()
        }
    }

    public func pollContinuously(interval: Duration = .seconds(4)) async {
        while !Task.isCancelled {
            await poll()
            do {
                try await Task.sleep(for: bridgeFailing ? .seconds(30) : interval)
            } catch {
                return
            }
        }
    }

    public func clearView() {
        records.removeAll(keepingCapacity: true)
        localBufferBytes = 2
        renderLimit = Self.renderStep
        newWhilePaused = 0
        // Keep `lastID`: clear is client-view-only and must not replay the
        // daemon's existing ring on the next incremental poll.
    }

    public func clearFilters() {
        query = ""
        useRegex = false
        level = "all"
        module = "all"
    }

    public func loadOlder() {
        renderLimit = min(Self.maximumRenderedLines, renderLimit + Self.renderStep)
    }

    public func jumpToLatest() {
        autoscroll = true
        newWhilePaused = 0
    }

    /// Captures native UI/runtime events in the same bounded RAM ring. This is
    /// the native equivalent of frontend `emitAppLog`; callers never receive a
    /// filesystem sink.
    public func emitLocal(
        level: String,
        module: String,
        file: String,
        line: Int64 = 0,
        message: String,
        fields: [String: LogField] = [:]
    ) {
        let timestamp = Self.isoString(Date())
        let safeFields = fields.mapValues { field -> LogField in
            guard let text = field.value.stringValue else { return field }
            return LogField(type: field.type, value: .string(LogRedactor.secretFloor(text)))
        }
        let record = LogRecordRow(
            id: nextLocalID,
            timestamp: timestamp,
            date: Self.parseDate(timestamp),
            level: Self.levels.contains(level) ? level : "info",
            module: module,
            file: file,
            line: line,
            message: LogRedactor.secretFloor(message),
            fields: safeFields,
            origin: .local
        )
        nextLocalID += 1
        merge([record])
    }

    public func revealRaw(now: Date = Date(), duration: TimeInterval = 5 * 60) {
        redacted = false
        rawUntil = now.addingTimeInterval(duration)
    }

    public func hideRaw() {
        redacted = true
        rawUntil = nil
    }

    public func expireRawIfNeeded(now: Date = Date()) {
        if let rawUntil, now >= rawUntil { hideRaw() }
    }

    public func renderedRecord(
        _ record: LogRecordRow,
        forceMode: LogRedactionMode? = nil,
        forceMaskAmounts: Bool? = nil
    ) -> LogRecordRow {
        let shouldRedact = forceMode != nil || redacted
        guard shouldRedact else { return record }
        let mode = forceMode ?? redactionMode
        let hideScale = forceMaskAmounts ?? maskAmounts
        var usedNames: [String: Int] = [:]
        var outputFields: [String: LogField] = [:]
        for (name, field) in record.fields.sorted(by: { $0.key < $1.key }) {
            let base = Self.redactedFieldName(name: name, type: field.type)
            let seen = usedNames[base, default: 0]
            usedNames[base] = seen + 1
            let renderedName = seen == 0 ? base : "\(base)_\(seen + 1)"
            outputFields[renderedName] = renderedField(field, mode: mode, maskAmounts: hideScale)
        }
        return LogRecordRow(
            id: record.id,
            timestamp: record.timestamp,
            date: record.date,
            level: record.level,
            module: record.module,
            file: record.file,
            line: record.line,
            message: redactText(record.message, mode: mode, maskAmounts: hideScale),
            fields: outputFields,
            origin: record.origin
        )
    }

    public func format(_ record: LogRecordRow) -> String {
        let rendered = renderedRecord(record)
        let levelText = rendered.level.uppercased().padding(toLength: 7, withPad: " ", startingAt: 0)
        let fieldText = rendered.fields.sorted(by: { $0.key < $1.key }).map {
            "\($0.key)=\(Self.stringify($0.value.value))"
        }.joined(separator: " ")
        return [rendered.timestamp, levelText, rendered.location, rendered.message, fieldText]
            .filter { !$0.isEmpty }
            .joined(separator: " ")
    }

    public func expandedJSON(_ record: LogRecordRow) -> String {
        Self.jsonString(recordObject(renderedRecord(record)), pretty: true)
    }

    public func fieldSummary(_ record: LogRecordRow) -> String {
        renderedRecord(record).fields.sorted(by: { $0.key < $1.key }).map {
            "\($0.key)=\(Self.stringify($0.value.value))"
        }.joined(separator: " ")
    }

    public func copyLast200Text() -> String {
        filteredRecords.suffix(200).map(format).joined(separator: "\n")
    }

    public func exportText(format: LogExportFormat, header: LogExportHeader) -> String {
        let source = filteredRecords
        let rendered = source.map { renderedRecord($0) }
        let rawWatermark = redacted ? nil : "RAW EXPORT - may contain wallet material"
        switch format {
        case .plain:
            let body = source.map(self.format).joined(separator: "\n")
            return [rawWatermark, body].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: "\n") + "\n"
        case .jsonl:
            var lines: [String] = []
            if let rawWatermark {
                lines.append(Self.jsonString([
                    "kind": .string("kassiber.log_export_watermark"),
                    "ts": .string(Self.isoString(header.generatedAt)),
                    "msg": .string(rawWatermark),
                ]))
            }
            lines += rendered.map { Self.jsonString(recordObject($0)) }
            return lines.joined(separator: "\n") + "\n"
        case .markdown:
            let filter = redacted
                ? redactText(activeFilterDescription, mode: redactionMode, maskAmounts: maskAmounts)
                : activeFilterDescription
            var lines = ["# Kassiber log snapshot", ""]
            if let rawWatermark { lines += ["> \(rawWatermark)", ""] }
            lines += [
                "- App version: \(header.appVersion)",
                "- OS: \(header.operatingSystem)",
                "- Generated: \(Self.isoString(header.generatedAt))",
                "- Time range: \(timeRangeDescription)",
                "- Active filter: \(filter)",
                "- Redaction: \(redactionLabel)",
                "", "```log",
            ]
            lines += source.map(self.format)
            lines += ["```", ""]
            return lines.joined(separator: "\n")
        }
    }

    public func exportFilename(format: LogExportFormat, date: Date = Date()) -> String {
        let stamp = Self.filenameStamp(date)
        return "kassiber-\(stamp)Z-\(redactionLabel).\(format.rawValue)"
    }

    public func supportBundleFilename(date: Date = Date()) -> String {
        "kassiber-support-\(Self.filenameStamp(date))Z.support.jsonl"
    }

    public func supportBundle(
        issueDescription: String,
        mode: LogRedactionMode,
        header: LogExportHeader,
        maxEvents: Int = 1_000,
        contextRadius: Int = 12,
        includeAIProvenance: Bool = true
    ) -> String? {
        buildSupportBundle(
            issueDescription: issueDescription,
            mode: mode,
            header: header,
            maxEvents: maxEvents,
            contextRadius: contextRadius,
            includeAIProvenance: includeAIProvenance,
            allowEmptyDescription: false
        )
    }

    private func buildSupportBundle(
        issueDescription: String,
        mode: LogRedactionMode,
        header: LogExportHeader,
        maxEvents: Int = 1_000,
        contextRadius: Int = 12,
        includeAIProvenance: Bool = true,
        allowEmptyDescription: Bool
    ) -> String? {
        let issue = issueDescription.trimmingCharacters(in: .whitespacesAndNewlines)
        guard allowEmptyDescription || !issue.isEmpty else { return nil }

        let exported = Array(records.suffix(max(0, maxEvents)))
        let rendered = exported.map { renderedRecord($0, forceMode: mode, forceMaskAmounts: mode == .publicSafe) }
        let failureIndexes = rendered.indices.filter { Self.isFailure(rendered[$0]) }
        var context = Set<Int>()
        for failureIndex in failureIndexes {
            let traceID = Self.fieldString(rendered[failureIndex], "trace_id")
            let requestID = Self.fieldString(rendered[failureIndex], "request_id")
            for index in rendered.indices {
                if !traceID.isEmpty, Self.fieldString(rendered[index], "trace_id") == traceID { context.insert(index) }
                if !requestID.isEmpty, Self.fieldString(rendered[index], "request_id") == requestID { context.insert(index) }
            }
            let lower = max(0, failureIndex - max(0, contextRadius))
            let upper = min(rendered.count - 1, failureIndex + max(0, contextRadius))
            if lower <= upper { context.formUnion(lower...upper) }
        }
        let aiIndexes = includeAIProvenance
            ? rendered.indices.filter { Self.isAIProvenance(rendered[$0]) }
            : []
        let omitted = max(0, records.count - exported.count)
        let modeOptionsMask = mode == .publicSafe
        let bundleTimeRange = Self.timeRange(exported)
        let bundleFilter = "support_bundle=all_records, ui_filter=(\(activeFilterDescription))"

        var lines: [[String: JSONValue]] = []
        lines.append([
            "kind": "kassiber.support_bundle.manifest",
            "schema_version": 1,
            "generated_at": .string(Self.isoString(header.generatedAt)),
            "app_version": .string(header.appVersion),
            "os": .string(header.operatingSystem),
            "time_range": .string(bundleTimeRange),
            "active_filter": .string(redactText(bundleFilter, mode: mode, maskAmounts: modeOptionsMask)),
            "redaction": .string(mode.rawValue),
            "public_safe": .bool(mode == .publicSafe),
            "format_note": .string(Self.supportFormatNote(mode)),
            "sections": .object([
                "issue": 1,
                "redaction_report": 1,
                "events": .integer(Int64(rendered.count)),
                "last_failures": .integer(Int64(context.count)),
                "ai_provenance": .integer(Int64(aiIndexes.count)),
                "diagnostics": 1,
            ]),
        ])
        lines.append([
            "kind": "kassiber.support_bundle.issue",
            "schema_version": 1,
            "description": .string(redactText(issue, mode: mode, maskAmounts: modeOptionsMask)),
        ])
        lines.append(redactionReport(exported, omitted: omitted, mode: mode))
        lines.append([
            "kind": "kassiber.support_bundle.diagnostics",
            "schema_version": 1,
            "summary": .object([
                "events_included": .integer(Int64(rendered.count)),
                "events_omitted_from_start": .integer(Int64(omitted)),
                "failures_detected": .integer(Int64(failureIndexes.count)),
                "ai_provenance_records": .integer(Int64(aiIndexes.count)),
                "buffer_time_range": .string(bundleTimeRange),
            ]),
        ])
        for index in rendered.indices {
            lines.append([
                "kind": "kassiber.support_bundle.event",
                "schema_version": 1,
                "index": .integer(Int64(index)),
                "record": .object(recordObject(rendered[index])),
            ])
        }
        for index in context.sorted() {
            lines.append([
                "kind": "kassiber.support_bundle.last_failure",
                "schema_version": 1,
                "index": .integer(Int64(index)),
                "record": .object(recordObject(rendered[index])),
            ])
        }
        for index in aiIndexes {
            lines.append([
                "kind": "kassiber.support_bundle.ai_provenance",
                "schema_version": 1,
                "index": .integer(Int64(index)),
                "record": .object(recordObject(rendered[index])),
            ])
        }
        return lines.map { Self.jsonString($0) }.joined(separator: "\n") + "\n"
    }

    public func supportBundlePreview(
        issueDescription: String,
        mode: LogRedactionMode,
        header: LogExportHeader,
        lineLimit: Int = 30
    ) -> String {
        guard let bundle = buildSupportBundle(
            issueDescription: issueDescription,
            mode: mode,
            header: header,
            allowEmptyDescription: true
        ) else { return "" }
        return bundle.split(separator: "\n", omittingEmptySubsequences: false)
            .prefix(max(0, lineLimit))
            .joined(separator: "\n")
    }

    private var redactionLabel: String {
        if !redacted { return "raw" }
        return maskAmounts ? "redacted-amounts" : "redacted"
    }

    private func resetRendering() {
        renderLimit = Self.renderStep
    }

    private func merge(_ incoming: [LogRecordRow]) {
        guard !incoming.isEmpty else { return }
        let existing = Set(records.map(\.id))
        let additions = incoming.filter { !existing.contains($0.id) }
        guard !additions.isEmpty else { return }
        records.append(contentsOf: additions)
        records.sort { lhs, rhs in
            if let left = lhs.date, let right = rhs.date, left != right { return left < right }
            if lhs.timestamp != rhs.timestamp { return lhs.timestamp < rhs.timestamp }
            return lhs.id < rhs.id
        }
        if !autoscroll { newWhilePaused += additions.count }
        enforceLocalBounds()
    }

    private func enforceLocalBounds() {
        if records.count > Self.maximumRecords {
            records.removeFirst(records.count - Self.maximumRecords)
        }
        localBufferBytes = records.reduce(2) { $0 + Self.estimatedBytes($1) }
        while !records.isEmpty, localBufferBytes > Self.maximumBytes {
            let dropCount = max(1, Int(ceil(Double(records.count) * 0.1)))
            for record in records.prefix(dropCount) {
                localBufferBytes -= Self.estimatedBytes(record)
            }
            records.removeFirst(min(dropCount, records.count))
        }
        localBufferBytes = max(2, localBufferBytes)
    }

    private static func parseRecord(_ row: [String: JSONValue]) -> LogRecordRow? {
        guard let id = row.int("id") else { return nil }
        let timestamp = row.string("ts") ?? ""
        var fields: [String: LogField] = [:]
        for (name, encoded) in row["fields"]?.objectValue ?? [:] {
            let object = encoded.objectValue
            let type = object?.string("type") ?? "text"
            let rawValue = object?["value"] ?? encoded
            let value: JSONValue
            if let text = rawValue.stringValue {
                // The daemon already floors these values. Repeating the floor
                // at this trust boundary keeps malformed preview/test records
                // from putting credentials in the native RAM ring.
                value = .string(LogRedactor.secretFloor(text))
            } else {
                value = rawValue
            }
            fields[name] = LogField(type: type, value: value)
        }
        return LogRecordRow(
            id: id,
            timestamp: timestamp,
            date: Self.parseDate(timestamp),
            level: row.string("level") ?? "info",
            module: row.string("module") ?? "",
            file: row.string("file") ?? "",
            line: row.int("line") ?? 0,
            message: LogRedactor.secretFloor(row.string("msg") ?? ""),
            fields: fields,
            origin: .daemon
        )
    }

    private func pollLifecycle() async {
        guard let source = daemon as? any DaemonLifecycleSource else { return }
        let snapshot = await source.lifecycleSnapshot(afterID: lifecycleCursor)
        let mapped = snapshot.records.map { record in
            var fields: [String: LogField] = [
                "source": LogField(type: "text", value: .string(LogRedactor.secretFloor(record.source))),
            ]
            if !record.stderrTail.isEmpty {
                fields["stderr_tail"] = LogField(type: "text", value: .string(LogRedactor.secretFloor(record.stderrTail)))
            }
            let level: String
            if record.event == "spawned" {
                level = "info"
            } else if record.event == "exited",
                      record.detail.range(of: #"\bstatus:?\s*0\b"#, options: [.regularExpression, .caseInsensitive]) != nil {
                level = "info"
            } else if record.event == "request_timeout" {
                level = "warning"
            } else {
                level = "error"
            }
            let timestamp = Self.isoString(record.timestamp)
            return LogRecordRow(
                id: 9_000_000_000_000_000 + record.id,
                timestamp: timestamp,
                date: record.timestamp,
                level: level,
                module: "supervisor",
                file: "ProcessDaemonSupervisor.swift",
                line: 0,
                message: record.detail.isEmpty ? record.event : "\(record.event): \(record.detail)",
                fields: fields,
                origin: .supervisor
            )
        }
        merge(mapped)
        lifecycleCursor = snapshot.lastID
    }

    private func renderedField(_ field: LogField, mode: LogRedactionMode, maskAmounts: Bool) -> LogField {
        let value = Self.stringify(field.value)
        switch field.type {
        case "api_key": return LogField(type: "text", value: .string("[redacted-api-key]"))
        case "xpriv": return LogField(type: "text", value: .string("[redacted-private-key]"))
        case "xpub": return LogField(type: "text", value: .string("xpub#\(Self.stableHash(value))"))
        case "descriptor": return LogField(type: "text", value: .string(LogRedactor.maskDescriptor(value)))
        case "txid": return LogField(type: "text", value: .string(Self.pseudoTxID(value)))
        case "amount":
            let parsed = Self.parseAmount(value)
            return LogField(type: "text", value: .string(pseudoAmount(parsed.number, unit: parsed.unit, showScale: !maskAmounts)))
        case "address":
            return mode == .publicSafe
                ? LogField(type: "text", value: .string(Self.keepShort(value, head: 5, tail: 4)))
                : LogField(type: field.type, value: .string(redactText(value, mode: mode, maskAmounts: maskAmounts)))
        case "url":
            return mode == .publicSafe
                ? LogField(type: "text", value: .string("url#\(Self.stableHash(value))"))
                : LogField(type: field.type, value: .string(redactText(value, mode: mode, maskAmounts: maskAmounts)))
        case "path":
            return mode == .publicSafe
                ? LogField(type: "text", value: .string(LogRedactor.maskPath(value)))
                : LogField(type: field.type, value: .string(redactText(value, mode: mode, maskAmounts: maskAmounts)))
        case "label":
            return mode == .publicSafe
                ? LogField(type: "text", value: .string("wallet#\(Self.stableHash(value))"))
                : LogField(type: field.type, value: .string(redactText(value, mode: mode, maskAmounts: maskAmounts)))
        case "onion": return mode == .publicSafe ? LogField(type: "text", value: .string("onion#\(Self.stableHash(value))")) : field
        case "email": return mode == .publicSafe ? LogField(type: "text", value: .string("email#\(Self.stableHash(value))")) : field
        case "ip": return mode == .publicSafe ? LogField(type: "text", value: .string("ip#\(Self.stableHash(value))")) : field
        case "text": return LogField(type: field.type, value: .string(redactText(value, mode: mode, maskAmounts: maskAmounts)))
        default:
            if case .string = field.value {
                return LogField(type: field.type, value: .string(redactText(value, mode: mode, maskAmounts: maskAmounts)))
            }
            return field
        }
    }

    private func redactText(_ value: String, mode: LogRedactionMode, maskAmounts: Bool) -> String {
        var output = LogRedactor.secretFloor(value)
        if mode == .publicSafe {
            output = LogRedactor.publicSafeOperational(output)
        }
        output = LogRedactor.replace(pattern: #"\b[0-9a-fA-F]{64,}\b"#, in: output) {
            Self.pseudoTxID($0)
        }
        output = LogRedactor.replaceAmounts(in: output) { number, unit in
            self.pseudoAmount(number, unit: unit, showScale: !maskAmounts)
        }
        return output
    }

    private func pseudoAmount(_ number: String, unit: String, showScale: Bool) -> String {
        let normalized = number.replacingOccurrences(of: #"[,_ ]"#, with: "", options: .regularExpression)
        let token = "amount#\(Self.stableHash("\(amountSalt)|\(normalized)|\(unit.lowercased())"))"
        guard showScale else { return token }
        let magnitude = Self.amountMagnitude(normalized)
        return unit.isEmpty ? "\(token) (\(magnitude))" : "\(token) (\(magnitude) \(unit))"
    }

    private func recordObject(_ record: LogRecordRow) -> [String: JSONValue] {
        var fields: [String: JSONValue] = [:]
        for (name, field) in record.fields {
            fields[name] = .object(["type": .string(field.type), "value": field.value])
        }
        return [
            "id": .integer(record.id),
            "ts": .string(record.timestamp),
            "level": .string(record.level),
            "module": .string(record.module),
            "file": .string(record.file),
            "line": .integer(record.line),
            "msg": .string(record.message),
            "fields": .object(fields),
        ]
    }

    private func redactionReport(_ source: [LogRecordRow], omitted: Int, mode: LogRedactionMode) -> [String: JSONValue] {
        let secretTypes = Set(["api_key", "descriptor", "xpriv", "xpub"])
        let operationalTypes = Set(["address", "email", "ip", "label", "onion", "path", "txid", "url", "amount"])
        var secretCounts: [String: Int64] = [:]
        var operationalCounts: [String: Int64] = [:]
        var secretTextHits: Int64 = 0
        var publicTextHits: Int64 = 0
        for record in source {
            if LogRedactor.secretFloor(record.message) != record.message { secretTextHits += 1 }
            if mode == .publicSafe, LogRedactor.publicSafeOperational(record.message) != record.message { publicTextHits += 1 }
            for field in record.fields.values {
                if secretTypes.contains(field.type) { secretCounts[field.type, default: 0] += 1 }
                if operationalTypes.contains(field.type) { operationalCounts[field.type, default: 0] += 1 }
            }
        }
        return [
            "kind": "kassiber.support_bundle.redaction_report",
            "schema_version": 1,
            "mode": .string(mode.rawValue),
            "txids": "pseudonymized",
            "amounts": .string(mode == .publicSafe ? "pseudonymized" : "pseudonymized-with-magnitude"),
            "omitted_events_from_start": .integer(Int64(omitted)),
            "secret_floor_field_counts": .object(secretCounts.mapValues(JSONValue.integer)),
            "operational_field_counts": .object(operationalCounts.mapValues(JSONValue.integer)),
            "secret_floor_text_hits": .integer(secretTextHits),
            "public_safe_text_hits": .integer(publicTextHits),
            "excluded_material": .array([
                "raw daemon arguments", "raw imported rows", "raw AI prompts", "database files",
                "descriptors", "xpubs", "private keys", "mnemonics", "backend URLs", "API keys",
                "local filesystem paths", "stack locals",
            ].map(JSONValue.string)),
        ]
    }

    private static func redactedFieldName(name: String, type: String) -> String {
        if type == "amount" { return name }
        if ["xpub", "xpriv", "descriptor"].contains(type) { return "wallet_material" }
        if ["api_key", "address", "email", "ip", "label", "onion", "path", "txid", "url"].contains(type) { return type }
        return name
    }

    private static func isFailure(_ record: LogRecordRow) -> Bool {
        record.level == "error" || record.fields["error_code"] != nil ||
            record.message.range(of: #"(?:failed|threw|error)"#, options: [.regularExpression, .caseInsensitive]) != nil
    }

    private static func isAIProvenance(_ record: LogRecordRow) -> Bool {
        fieldString(record, "kind").hasPrefix("ai.chat") ||
            record.module.localizedCaseInsensitiveContains("ai") ||
            record.message.localizedCaseInsensitiveContains("AI chat")
    }

    private static func fieldString(_ record: LogRecordRow, _ name: String) -> String {
        guard let field = record.fields[name] else { return "" }
        return stringify(field.value)
    }

    private static func supportFormatNote(_ mode: LogRedactionMode) -> String {
        if mode == .publicSafe {
            return "Each JSONL row is independently redacted for public support: wallet and credential material is stripped, txids and amounts use stable pseudonyms, and operational fields are masked."
        }
        return "High-signal bundles keep operational debugging data readable. Txids and amounts use stable pseudonyms, amounts retain only a coarse magnitude, and wallet or credential material is stripped."
    }

    private static func estimatedBytes(_ record: LogRecordRow) -> Int64 {
        Int64(jsonString([
            "id": .integer(record.id), "ts": .string(record.timestamp), "level": .string(record.level),
            "module": .string(record.module), "file": .string(record.file), "line": .integer(record.line),
            "msg": .string(record.message),
        ]).utf8.count + record.fields.reduce(0) { $0 + $1.key.utf8.count + stringify($1.value.value).utf8.count } + 1)
    }

    private static func stringify(_ value: JSONValue) -> String {
        switch value {
        case let .string(value): return value
        case let .integer(value): return String(value)
        case let .unsignedInteger(value): return String(value)
        case let .number(value): return String(value)
        case let .bool(value): return String(value)
        case .null: return "null"
        case .array, .object:
            guard let data = try? JSONEncoder().encode(value) else { return "" }
            return String(decoding: data, as: UTF8.self)
        }
    }

    private static func jsonString(_ object: [String: JSONValue], pretty: Bool = false) -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = pretty ? [.prettyPrinted, .sortedKeys] : [.sortedKeys]
        guard let data = try? encoder.encode(JSONValue.object(object)) else { return "{}" }
        return String(decoding: data, as: UTF8.self)
    }

    private static func stableHash(_ value: String) -> String {
        var hash: UInt32 = 0x811c9dc5
        for codeUnit in value.utf16 {
            hash ^= UInt32(codeUnit)
            hash = hash &* 0x01000193
        }
        return String(format: "%08x", hash)
    }

    private static func pseudoTxID(_ value: String) -> String {
        "txid#\(stableHash(value.lowercased()))"
    }

    private static func keepShort(_ value: String, head: Int, tail: Int) -> String {
        guard value.count > head + tail + 1 else { return value }
        return "\(value.prefix(head))...\(value.suffix(tail))"
    }

    private static func parseAmount(_ value: String) -> (number: String, unit: String) {
        if let match = LogRedactor.firstMatch(pattern: #"^([€$£¥₿])\s*([+-]?[\d.,_ ]*\d)"#, in: value), match.count >= 3 {
            return (match[2], match[1])
        }
        if let match = LogRedactor.firstMatch(pattern: #"^([+-]?[\d.,_ ]*\d)\s*([A-Za-z]{1,6}|[€$£¥₿])?"#, in: value), match.count >= 2 {
            return (match[1], match.count >= 3 ? match[2] : "")
        }
        return (value, "")
    }

    private static func amountMagnitude(_ value: String) -> String {
        guard let number = Double(value), number.isFinite, number != 0 else { return "~0" }
        let exponent = Int(floor(log10(abs(number))))
        let sign = number < 0 ? "-" : ""
        let body: String
        if exponent < 0 {
            body = String(format: "%.*f", -exponent, pow(10, Double(exponent)))
        } else if exponent <= 4 {
            body = String(Int(pow(10, Double(exponent))))
        } else {
            body = "1e\(exponent)"
        }
        return "~\(sign)\(body)"
    }

    private static func parseDate(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: value) ?? ISO8601DateFormatter().date(from: value)
    }

    private static func isoString(_ date: Date) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: date)
    }

    private static func timeRange(_ records: [LogRecordRow]) -> String {
        guard let first = records.first, let last = records.last else { return "empty" }
        return "\(first.timestamp) – \(last.timestamp)"
    }

    private static func filenameStamp(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd'T'HH-mm"
        return formatter.string(from: date)
    }
}

private enum LogRedactor {
    static func secretFloor(_ input: String) -> String {
        var value = input
        for count in [24, 21, 18, 15, 12] {
            let pattern = "\\b(?:[a-z]{3,8}\\s+){\(count - 1)}[a-z]{3,8}\\b"
            value = replace(pattern: pattern, in: value) { _ in "[redacted-seed-phrase]" }
        }
        value = replace(pattern: #"(?im)\b(mnemonic|recovery[_-]?phrase|seed(?:[_-]?phrase)?)\b(\s*[:=]\s*)(.+)$"#, in: value) { match in
            guard let pieces = firstMatch(pattern: #"(?im)^(.+?\b(?:mnemonic|recovery[_-]?phrase|seed(?:[_-]?phrase)?)\b\s*[:=]\s*).+$"#, in: match), pieces.count > 1 else { return "[redacted]" }
            return pieces[1] + "[redacted]"
        }
        value = replace(pattern: #"\b((?:https?|tcp|ssl)://)([^/\s:@]+):([^@\s/]+)@"#, in: value, options: [.caseInsensitive]) { match in
            let scheme = match.split(separator: ":", maxSplits: 1).first.map(String.init) ?? "https"
            return "\(scheme)://[redacted-credentials]@"
        }
        value = replace(pattern: #"\b(?:xprv|tprv|yprv|zprv|uprv|vprv)[1-9A-HJ-NP-Za-km-z]{20,}\b"#, in: value) { _ in "[redacted-private-key]" }
        value = replace(pattern: #"\b(?:xpub|tpub|ypub|zpub|upub|vpub)[1-9A-HJ-NP-Za-km-z]{20,}\b"#, in: value) { _ in "[redacted-extended-key]" }
        value = replace(pattern: #"\b(?:wpkh|sh|wsh|tr|pkh|combo|sp)\([^\n]{16,800}\)(?:#[a-z0-9]{8})?"#, in: value, options: [.caseInsensitive]) { maskDescriptor($0) }
        value = replace(pattern: #"\bBearer\s+[A-Za-z0-9._~+/-]+=*"#, in: value, options: [.caseInsensitive]) { _ in "Bearer [redacted]" }
        value = replace(pattern: #"(?i)\b(api[_-]?key|auth[_-]?header|cookie|descriptor|passphrase|password|secret|token)\b(\s*[:=]\s*)([^\s,;\"']+)"#, in: value) { match in
            guard let pieces = firstMatch(pattern: #"(?i)^([^:=]+)(\s*[:=]\s*).+$"#, in: match), pieces.count >= 3 else { return "[redacted]" }
            return pieces[1] + pieces[2] + "[redacted]"
        }
        return value
    }

    static func publicSafeOperational(_ input: String) -> String {
        var value = input
        let units = "BTC|XBT|LBTC|sats?|msats?|EUR|USD|CHF|GBP|JPY|CAD|AUD|NZD|SEK|NOK|DKK|PLN|CZK|HUF"
        let number = #"[+-]?(?:(?:\d{1,3}(?:[,_ .]\d{3})+)|\d+)(?:[.,]\d+)?"#
        value = replace(pattern: "(?i)\\b(?:\(units))[/-](?:\(units))\\s*(?::|=|at|rate)?\\s*\(number)\\b", in: value) { _ in "[redacted-rate]" }
        value = replace(pattern: #"\b(?:https?|tcp|ssl)://[^\s,;\"')\]}]+"#, in: value, options: [.caseInsensitive]) { _ in "[redacted-url]" }
        value = replace(pattern: #"\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b"#, in: value, options: [.caseInsensitive]) { _ in "[redacted-email]" }
        value = replace(pattern: #"\b(?:bc1|tb1|bcrt1|lq1|ex1)[023456789acdefghjklmnpqrstuvwxyz]{20,90}\b"#, in: value, options: [.caseInsensitive]) { _ in "[redacted-address]" }
        value = replace(pattern: #"\b[13][1-9A-HJ-NP-Za-km-z]{25,34}\b"#, in: value) { _ in "[redacted-address]" }
        value = replace(pattern: #"\b[A-Za-z0-9.-]{16,}\.onion\b"#, in: value, options: [.caseInsensitive]) { _ in "[redacted-onion]" }
        value = replace(pattern: #"(?:^|[\s\"'(])(?:/Users|/home|/var|/private|/tmp)/[^\s,;\"')\]}]+"#, in: value) { match in
            (match.first?.isWhitespace == true ? " " : "") + "[redacted-path]"
        }
        return value
    }

    static func replaceAmounts(
        in input: String,
        replacement: (_ number: String, _ unit: String) -> String
    ) -> String {
        let units = "BTC|XBT|LBTC|sats?|msats?|EUR|USD|CHF|GBP|JPY|CAD|AUD|NZD|SEK|NOK|DKK|PLN|CZK|HUF"
        let number = #"[+-]?(?:(?:\d{1,3}(?:[,_ .]\d{3})+)|\d+)(?:[.,]\d+)?"#
        var protectedRates: [String: String] = [:]
        let protectedInput = replace(
            pattern: "(?i)\\b(?:\(units))[/-](?:\(units))\\s*(?::|=|at|rate)?\\s*\(number)\\b",
            in: input
        ) { rate in
            let key = "[kassiber-market-rate-\(protectedRates.count)]"
            protectedRates[key] = rate
            return key
        }
        let pattern = "(?i)(?:\\b(?:\(units))\\s*\(number)\\b|(?<![A-Za-z0-9])\(number)\\s*(?:\(units))\\b|[€$£¥₿]\\s*\(number)\\b|(?<![A-Za-z0-9])\(number)\\s*[€$£¥₿])"
        var output = replace(pattern: pattern, in: protectedInput) { token in
            let parsed = parseAmountToken(token)
            return replacement(parsed.number, parsed.unit)
        }
        let keyed = #"(?i)\b([A-Za-z][A-Za-z0-9_]*(?:msats?|sats?))\b(['\"]?\s*[:=]\s*['\"]?)([+-]?\d[\d.,_]*\d|[+-]?\d)"#
        output = replace(pattern: keyed, in: output) { token in
            guard let pieces = firstMatch(pattern: keyed, in: token), pieces.count >= 4 else { return token }
            let unit = pieces[1].lowercased().contains("msat") ? "msat" : "sat"
            return pieces[1] + pieces[2] + replacement(pieces[3], unit)
        }
        for (key, rate) in protectedRates { output = output.replacingOccurrences(of: key, with: rate) }
        return output
    }

    static func maskDescriptor(_ value: String) -> String {
        guard let parts = firstMatch(pattern: #"(?i)\b(wpkh|sh|wsh|tr|pkh|combo|sp)\("#, in: value), parts.count > 1 else {
            return "[redacted-descriptor]"
        }
        let origin = firstMatch(pattern: #"\[[^\]\n]{1,120}\]"#, in: value)?.first ?? ""
        return "\(parts[1])(\(origin)[redacted-key])"
    }

    static func maskPath(_ value: String) -> String {
        let normalized = value.replacingOccurrences(of: "\\", with: "/")
        return "~/.../\(normalized.split(separator: "/").last.map(String.init) ?? "path")"
    }

    static func replace(
        pattern: String,
        in value: String,
        options: NSRegularExpression.Options = [],
        replacement: (String) -> String
    ) -> String {
        guard let expression = try? NSRegularExpression(pattern: pattern, options: options) else { return value }
        let source = value as NSString
        let matches = expression.matches(in: value, range: NSRange(location: 0, length: source.length))
        guard !matches.isEmpty else { return value }
        let mutable = NSMutableString(string: value)
        for match in matches.reversed() {
            let token = source.substring(with: match.range)
            mutable.replaceCharacters(in: match.range, with: replacement(token))
        }
        return mutable as String
    }

    static func firstMatch(pattern: String, in value: String) -> [String]? {
        guard let expression = try? NSRegularExpression(pattern: pattern),
              let match = expression.firstMatch(in: value, range: NSRange(value.startIndex..<value.endIndex, in: value)) else { return nil }
        let source = value as NSString
        return (0..<match.numberOfRanges).map { index in
            let range = match.range(at: index)
            return range.location == NSNotFound ? "" : source.substring(with: range)
        }
    }

    private static func parseAmountToken(_ token: String) -> (number: String, unit: String) {
        let numericPattern = #"[+-]?\d[\d.,_ ]*\d|[+-]?\d"#
        guard let expression = try? NSRegularExpression(pattern: numericPattern),
              let match = expression.firstMatch(in: token, range: NSRange(token.startIndex..<token.endIndex, in: token)) else {
            return (token, "")
        }
        let source = token as NSString
        let number = source.substring(with: match.range)
        let unit = token.replacingOccurrences(of: number, with: "").replacingOccurrences(of: " ", with: "")
        return (number, unit)
    }
}
