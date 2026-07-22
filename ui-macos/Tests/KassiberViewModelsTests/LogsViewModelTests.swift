import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Native Logs parity")
@MainActor
struct LogsViewModelTests {
    private let txid = String(repeating: "a", count: 64)

    @Test("Redaction tiers preserve correlation, mask amounts, and time-box raw reveal")
    func redactionAndRawReveal() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiLogsSnapshot: [snapshot(records: [[
                "id": 1,
                "ts": "2026-07-10T12:00:00.000Z",
                "level": "warning",
                "module": "wallet.sync",
                "file": "sync.py",
                "line": 42,
                "msg": .string("sent 2500 sats tx \(txid) from /Users/alice/book.csv to alice@example.com at BTC/EUR 64000.12 Bearer secret-token"),
                "fields": [
                    "transaction": ["type": "txid", "value": .string(txid)],
                    "amount": ["type": "amount", "value": "2500 sats"],
                    "source": ["type": "path", "value": "/Users/alice/book.csv"],
                ],
            ]])],
        ])
        let model = LogsViewModel(daemon: daemon)
        await model.load()

        model.redactionMode = .highSignal
        let highSignal = model.format(model.records[0])
        #expect(highSignal.contains("txid#"))
        #expect(highSignal.contains("amount#"))
        #expect(highSignal.contains("(~1000 sats)"))
        #expect(highSignal.contains("/Users/alice/book.csv"))
        #expect(highSignal.contains("BTC/EUR 64000.12"))
        #expect(!highSignal.contains(txid))
        #expect(!highSignal.contains("secret-token"))

        model.redactionMode = .publicSafe
        model.maskAmounts = true
        let publicSafe = model.format(model.records[0])
        #expect(publicSafe.contains("[redacted-path]"))
        #expect(publicSafe.contains("[redacted-email]"))
        #expect(publicSafe.contains("[redacted-rate]"))
        #expect(!publicSafe.contains("(~"))

        let now = Date(timeIntervalSince1970: 1_700_000_000)
        model.revealRaw(now: now)
        let raw = model.format(model.records[0])
        #expect(raw.contains(txid))
        #expect(raw.contains("2500 sats"))
        #expect(raw.contains("Bearer [redacted]"))
        model.expireRawIfNeeded(now: now.addingTimeInterval(301))
        #expect(model.redacted)
        #expect(model.rawUntil == nil)
    }

    @Test("Level, module, text, and regex filters use the rendered line")
    func filters() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiLogsSnapshot: [snapshot(records: [
                record(id: 1, level: "info", module: "rates", message: "Rate refresh complete"),
                record(id: 2, level: "warning", module: "sync", message: "Review wallet gap"),
                record(id: 3, level: "error", module: "sync", message: "Backend failed"),
            ])],
        ])
        let model = LogsViewModel(daemon: daemon)
        await model.load()
        model.level = "warning"
        model.module = "sync"
        model.query = "review"
        #expect(model.filteredRecords.map(\.id) == [2])

        model.query = #"Backend\s+failed"#
        model.level = "error"
        model.useRegex = true
        #expect(model.filteredRecords.map(\.id) == [3])

        model.query = "[invalid"
        #expect(model.regexError != nil)
        #expect(model.filteredRecords.isEmpty)
        model.clearFilters()
        #expect(model.filteredRecords.count == 3)
        #expect(!model.useRegex)
    }

    @Test("Incremental polling detects daemon restarts, gaps, and paused arrivals")
    func pollingAndRestart() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiLogsSnapshot: [snapshot(records: [record(id: 10, message: "old daemon")], lastID: 10, startedAt: "start-a")],
        ])
        let model = LogsViewModel(daemon: daemon)
        await model.poll()
        #expect(model.records.map(\.id) == [10])
        model.autoscroll = false

        await daemon.set([
            snapshot(records: [record(id: 1, message: "new daemon")], lastID: 1, startedAt: "start-b", gap: true),
        ], for: .uiLogsSnapshot)
        await model.poll()
        #expect(model.records.filter { $0.origin == .daemon }.map(\.id) == [1])
        #expect(model.records.contains { $0.module == "daemon:bridge" && $0.message.contains("evicted") })
        #expect(model.startedAt == "start-b")
        #expect(model.gap)
        #expect(model.newWhilePaused == 2)

        let calls = await daemon.calls().filter { $0.kind == .uiLogsSnapshot }
        #expect(calls.contains { $0.args?["after_id"]?.intValue == 10 })
        #expect(calls.contains { $0.args?["after_id"]?.intValue == 0 })
    }

    @Test("Progressive rendering, copy excerpt, and the RAM ring stay bounded")
    func progressiveAndBounded() async {
        let rows = (1...1_200).map { record(id: Int64($0), message: "row \($0)") }
        let daemon = ScriptedDaemonClient(scripts: [
            .uiLogsSnapshot: [snapshot(records: rows, lastID: 1_200)],
        ])
        let model = LogsViewModel(daemon: daemon)
        await model.load()
        #expect(model.visibleRecords.count == 1_000)
        #expect(model.visibleRecords.first?.id == 201)
        model.loadOlder()
        #expect(model.visibleRecords.count == 1_200)
        let copied = model.copyLast200Text().split(separator: "\n")
        #expect(copied.count == 200)
        #expect(copied.first?.contains("row 1001") == true)

        let manyRows = (1...10_050).map { record(id: Int64($0), message: "bounded \($0)") }
        await daemon.set([snapshot(records: manyRows, lastID: 10_050, startedAt: "large")], for: .uiLogsSnapshot)
        await model.poll()
        #expect(model.records.count <= LogsViewModel.maximumRecords)
        #expect(model.localBufferBytes <= LogsViewModel.maximumBytes)
    }

    @Test("Markdown, plain, and JSONL exports match redaction and watermark contracts")
    func exports() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiLogsSnapshot: [snapshot(records: [record(id: 1, message: "transaction \(txid) paid 900 sats")])],
        ])
        let model = LogsViewModel(daemon: daemon)
        await model.load()
        let generated = Date(timeIntervalSince1970: 1_700_000_000)
        let header = LogExportHeader(appVersion: "1.2.3", operatingSystem: "macOS test", generatedAt: generated)

        let markdown = model.exportText(format: .markdown, header: header)
        #expect(markdown.contains("# Kassiber log snapshot"))
        #expect(markdown.contains("- App version: 1.2.3"))
        #expect(markdown.contains("txid#"))
        #expect(!markdown.contains(txid))
        let jsonl = model.exportText(format: .jsonl, header: header)
        #expect(jsonl.contains("\"fields\""))
        #expect(!jsonl.contains("RAW EXPORT"))

        model.revealRaw(now: generated)
        let raw = model.exportText(format: .plain, header: header)
        #expect(raw.hasPrefix("RAW EXPORT - may contain wallet material"))
        #expect(raw.contains(txid))
        #expect(model.exportFilename(format: .plain, date: generated).hasSuffix("-raw.log"))
    }

    @Test("Support bundle requires a description and includes reports, failures, and AI provenance")
    func supportBundle() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiLogsSnapshot: [snapshot(records: [
                record(id: 1, level: "info", module: "ai.runtime", message: "AI chat started", fields: ["kind": ["type": "text", "value": "ai.chat"]]),
                record(id: 2, level: "error", module: "sync", message: "Backend failed for \(txid)", fields: ["request_id": ["type": "text", "value": "req-1"]]),
                record(id: 3, level: "info", module: "sync", message: "cleanup", fields: ["request_id": ["type": "text", "value": "req-1"]]),
            ])],
        ])
        let model = LogsViewModel(daemon: daemon)
        await model.load()
        let header = LogExportHeader(appVersion: "dev", operatingSystem: "macOS", generatedAt: Date(timeIntervalSince1970: 1_700_000_000))
        #expect(model.supportBundle(issueDescription: "  ", mode: .publicSafe, header: header) == nil)

        let bundle = model.supportBundle(
            issueDescription: "Failure for \(txid) in /Users/alice/book",
            mode: .publicSafe,
            header: header
        )
        #expect(bundle?.contains("kassiber.support_bundle.manifest") == true)
        #expect(bundle?.contains("kassiber.support_bundle.redaction_report") == true)
        #expect(bundle?.contains("kassiber.support_bundle.last_failure") == true)
        #expect(bundle?.contains("kassiber.support_bundle.ai_provenance") == true)
        #expect(bundle?.contains("txid#") == true)
        #expect(bundle?.contains(txid) == false)
        #expect(bundle?.contains("/Users/alice/book") == false)
        let preview = model.supportBundlePreview(issueDescription: "Failure", mode: .publicSafe, header: header)
        #expect(preview.split(separator: "\n").count <= 30)
    }

    @Test("Native supervisor lifecycle and local UI events join the same RAM ring")
    func supervisorAndLocalRing() async {
        let script = #"""
import json, sys
print(json.dumps({"kind":"daemon.ready","schema_version":1,"data":{}}), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    print(json.dumps({"kind":request["kind"],"schema_version":1,"request_id":request["request_id"],"data":{"records":[],"last_id":0,"gap":False,"started_at":"native-test","buffer_bytes":0,"max_bytes":4194304}}), flush=True)
"""#
        let supervisor = ProcessDaemonSupervisor(configuration: DaemonLaunchConfiguration(
            executable: URL(fileURLWithPath: "/usr/bin/env"),
            arguments: ["python3", "-u", "-c", script],
            workingDirectory: URL(fileURLWithPath: NSTemporaryDirectory())
        ))
        let model = LogsViewModel(daemon: supervisor)
        model.emitLocal(
            level: "error",
            module: "ui.error",
            file: "LogsScreen.swift",
            message: "UI failed Bearer local-secret"
        )
        let seedWords = "abandon ability able about above absent absorb abstract absurd abuse access accident"
        model.emitLocal(level: "error", module: "ui.error", file: "LogsScreen.swift", message: seedWords)
        await model.poll()
        #expect(model.records.contains { $0.origin == .local && $0.message.contains("Bearer [redacted]") })
        #expect(model.records.contains { $0.origin == .supervisor && $0.message.contains("daemon ready") })
        #expect(model.records.allSatisfy { !$0.message.contains("local-secret") })
        #expect(model.records.allSatisfy { !$0.message.contains(seedWords) })
        await supervisor.shutdown()
    }

    private func snapshot(
        records: [[String: JSONValue]],
        lastID: Int64? = nil,
        startedAt: String = "start-a",
        gap: Bool = false
    ) -> DaemonRecord {
        let recordIDs: [Int64] = records.compactMap { row in row["id"]?.intValue }
        let resolvedLastID: Int64 = lastID ?? recordIDs.max() ?? 0
        let encodedRecords: [JSONValue] = records.map { row in .object(row) }
        let payload: [String: JSONValue] = [
            "records": .array(encodedRecords),
            "last_id": .integer(resolvedLastID),
            "started_at": .string(startedAt),
            "gap": .bool(gap),
            "buffer_bytes": 1_024,
            "max_bytes": .integer(4 * 1_024 * 1_024),
        ]
        return DaemonRecord(kind: "ui.logs.snapshot", data: .object(payload))
    }

    private func record(
        id: Int64,
        level: String = "info",
        module: String = "daemon",
        message: String,
        fields: [String: JSONValue] = [:]
    ) -> [String: JSONValue] {
        [
            "id": .integer(id),
            "ts": .string("2026-07-10T12:00:00.000Z"),
            "level": .string(level),
            "module": .string(module),
            "file": .string("daemon.py"),
            "line": 10,
            "msg": .string(message),
            "fields": .object(fields),
        ]
    }
}
