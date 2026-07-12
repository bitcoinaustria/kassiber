import Foundation
import Testing
@testable import KassiberDaemonKit

@Suite("Generated daemon kind contract")
struct GeneratedKindLockstepTests {
    @Test("generated Swift and manifest match daemon.py")
    func generatorCheck() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let process = Process()
        let output = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["python3", "Scripts/generate_daemon_kinds.py", "--check"]
        process.currentDirectoryURL = packageRoot
        process.standardOutput = output
        process.standardError = output
        try process.run()
        process.waitUntilExit()
        let diagnostics = String(
            decoding: output.fileHandleForReading.readDataToEndOfFile(),
            as: UTF8.self
        )
        #expect(process.terminationStatus == 0, Comment(rawValue: diagnostics))

        let manifestURL = packageRoot.appending(path: "Generated/DaemonKinds.generated.json")
        let manifest = try JSONDecoder().decode(
            [String].self,
            from: Data(contentsOf: manifestURL)
        )
        #expect(manifest == DaemonKind.allCases.map(\.rawValue))
    }

    @Test("all supervisor classifications are allowlisted")
    func knownClassificationsAreGenerated() {
        let streaming: Set<DaemonKind> = [
            .aiChat, .uiWalletsSync, .uiFreshnessRun, .uiWorkspaceFreshnessRun,
            .uiJournalsProcess, .uiRatesRebuild, .uiSyncPush, .uiSyncPull, .uiSyncJoin,
        ]
        #expect(streaming.isSubset(of: Set(DaemonKind.allCases)))
    }

    @Test("native frontend maps every Tauri route to scoped native owners")
    func frontendParityCheck() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let process = Process()
        let output = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["python3", "Scripts/check_frontend_parity.py"]
        process.currentDirectoryURL = packageRoot
        process.standardOutput = output; process.standardError = output
        try process.run(); process.waitUntilExit()
        let diagnostics = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        #expect(process.terminationStatus == 0, Comment(rawValue: diagnostics))
        #expect(diagnostics.contains("22 native route-to-screen contracts cover 384 route/kind memberships"))
        #expect(diagnostics.contains("15 screen presentation/action contracts"))
    }

    @Test("native renderer and AI runtime gates exactly match Tauri")
    func desktopAccessPolicyMatchesTauri() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let rust = try String(
            contentsOf: packageRoot
                .deletingLastPathComponent()
                .appending(path: "ui-tauri/src-tauri/src/lib.rs"),
            encoding: .utf8
        )
        let rendererNames = try stringArrayConstant("ALLOWED_DAEMON_KINDS", in: rust)
        let runtimeNames = try stringArrayConstant("AI_RUNTIME_KINDS", in: rust)
        #expect(
            Set(DesktopDaemonAccessPolicy.rendererAllowedKinds.map(\.rawValue))
                == rendererNames
        )
        #expect(
            Set(DesktopDaemonAccessPolicy.aiRuntimeKinds.map(\.rawValue))
                == runtimeNames
        )
        #expect(DesktopDaemonAccessPolicy.aiRuntimeKinds.isSubset(of: DesktopDaemonAccessPolicy.rendererAllowedKinds))
        #expect(
            DesktopDaemonAccessPolicy.aiOnlyToolResultKinds
                .isSubset(of: DesktopDaemonAccessPolicy.aiToolResultPresentationKinds)
        )
        #expect(
            DesktopDaemonAccessPolicy.aiOnlyToolResultKinds
                .isDisjoint(with: DesktopDaemonAccessPolicy.rendererAllowedKinds)
        )
        #expect(
            Set(DesktopDaemonAccessPolicy.aiOnlyToolResultKinds.map(\.rawValue)) == Set([
                "ui.audit.changes_since_last_answer",
                "ui.report.blockers",
                "ui.transactions.extremes",
                "ui.transactions.search",
            ])
        )
    }

    @Test("native UI direct daemon calls stay inside the renderer allowlist")
    func nativeDirectCallsitesAreAllowlisted() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let generated = try String(
            contentsOf: packageRoot.appending(path: "Sources/KassiberDaemonKit/Generated/DaemonKind.generated.swift"),
            encoding: .utf8
        )
        let declaration = try NSRegularExpression(pattern: #"case\s+(\w+)\s*=\s*\"([^\"]+)\""#)
        var rawValueByCase: [String: String] = [:]
        for match in declaration.matches(in: generated, range: NSRange(generated.startIndex..., in: generated)) {
            guard let caseRange = Range(match.range(at: 1), in: generated),
                  let rawRange = Range(match.range(at: 2), in: generated) else { continue }
            rawValueByCase[String(generated[caseRange])] = String(generated[rawRange])
        }
        let directCall = try NSRegularExpression(
            pattern: #"\.(?:invoke|stream|streamSession)\s*\(\s*\.(\w+)"#
        )
        let allowed = Set(DesktopDaemonAccessPolicy.rendererAllowedKinds.map(\.rawValue))
        let sourceRoot = packageRoot.appending(path: "Sources")
        let enumerator = try #require(
            FileManager.default.enumerator(
                at: sourceRoot,
                includingPropertiesForKeys: [.isRegularFileKey]
            )
        )
        var violations: [String] = []
        for case let file as URL in enumerator where file.pathExtension == "swift" {
            let source = try String(contentsOf: file, encoding: .utf8)
            for match in directCall.matches(in: source, range: NSRange(source.startIndex..., in: source)) {
                guard let caseRange = Range(match.range(at: 1), in: source) else { continue }
                let caseName = String(source[caseRange])
                guard let rawValue = rawValueByCase[caseName], !allowed.contains(rawValue) else { continue }
                let line = source[..<caseRange.lowerBound].reduce(into: 1) { count, character in
                    if character == "\n" { count += 1 }
                }
                violations.append("\(file.lastPathComponent):\(line): \(rawValue)")
            }
        }
        #expect(
            violations.isEmpty,
            Comment(rawValue: "Direct native daemon calls bypass the renderer allowlist:\n\(violations.joined(separator: "\n"))")
        )
    }

    @Test("global command palette does not duplicate a screen toolbar search")
    func globalSearchAvoidsDuplicateToolbarItem() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let source = try String(
            contentsOf: packageRoot.appending(path: "Sources/KassiberApp/GlobalSearchChrome.swift"),
            encoding: .utf8
        )
        #expect(!source.contains(".searchable("))
        #expect(source.contains(".sheet(isPresented: $presented)"))
    }

    @Test("global macOS toolbar items keep stable AppKit identities")
    func globalToolbarItemsHaveStableIdentifiers() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let source = try String(
            contentsOf: packageRoot.appending(path: "Sources/KassiberApp/AppShellView.swift"),
            encoding: .utf8
        )
        let declaration = try NSRegularExpression(pattern: #"ToolbarItem\(id: \"([^\"]+)\""#)
        let ids = declaration.matches(in: source, range: NSRange(source.startIndex..., in: source)).compactMap { match in
            Range(match.range(at: 1), in: source).map { String(source[$0]) }
        }
        #expect(ids.count >= 11)
        #expect(Set(ids).count == ids.count)
        #expect(source.contains("shellNavigationToolbar"))
        #expect(source.contains("shellRefreshToolbar"))
        #expect(source.contains("shellPrivacyToolbar"))
        #expect(!source.contains("ToolbarItemGroup(placement: .primaryAction)"))
    }

    @Test("journal ledger avoids the release NSTableView reentrancy path")
    func journalLedgerUsesLazySwiftUIRows() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let source = try String(
            contentsOf: packageRoot.appending(path: "Sources/KassiberApp/ReviewScreens.swift"),
            encoding: .utf8
        )
        let start = try #require(source.range(of: "struct JournalsScreen"))
        let end = try #require(source.range(of: "struct QuarantineScreen", range: start.upperBound..<source.endIndex))
        let journalSource = source[start.lowerBound..<end.lowerBound]
        #expect(!journalSource.contains("Table("))
        #expect(journalSource.contains("LazyVStack"))
    }

    private func stringArrayConstant(_ name: String, in source: String) throws -> Set<String> {
        let pattern = #"const\s+"# + NSRegularExpression.escapedPattern(for: name)
            + #"\s*:\s*&\[&str\]\s*=\s*&\[(.*?)\];"#
        let regex = try NSRegularExpression(pattern: pattern, options: [.dotMatchesLineSeparators])
        let sourceRange = NSRange(source.startIndex..., in: source)
        let match = try #require(regex.firstMatch(in: source, range: sourceRange))
        let bodyRange = try #require(Range(match.range(at: 1), in: source))
        let body = String(source[bodyRange])
        let strings = try NSRegularExpression(pattern: #"\"([^\"]+)\""#)
        return Set(strings.matches(in: body, range: NSRange(body.startIndex..., in: body)).compactMap { match in
            guard let range = Range(match.range(at: 1), in: body) else { return nil }
            return String(body[range])
        })
    }
}
