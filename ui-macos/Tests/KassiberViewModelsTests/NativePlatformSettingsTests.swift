import Foundation
import Testing
@testable import KassiberViewModels

@Suite("Native platform settings")
struct NativePlatformSettingsTests {
    @Test("terminal launcher installs, repairs, and removes only managed files")
    func terminalLifecycle() throws {
        let root = FileManager.default.temporaryDirectory
            .appending(path: "kassiber-terminal-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let firstTarget = root.appending(path: "Kassiber One.app/Contents/MacOS/kassiber_native")
        let localBin = root.appending(path: ".local/bin", directoryHint: .isDirectory)
        let first = TerminalCommandManager(
            homeDirectory: root,
            targetExecutable: firstTarget,
            path: "\(localBin.path):/usr/bin"
        )

        #expect(try first.status().installed == false)
        let installed = try first.install()
        #expect(installed.installed)
        #expect(installed.managed)
        #expect(installed.pathOnPath)
        let script = try String(contentsOf: installed.commandURL, encoding: .utf8)
        #expect(script.contains(TerminalCommandManager.marker))
        #expect(script.contains("--cli \"$@\""))
        #expect(script.contains(firstTarget.path))
        let mode = try FileManager.default.attributesOfItem(atPath: installed.commandURL.path)[.posixPermissions] as? NSNumber
        #expect(mode?.intValue == 0o755)

        let nextTarget = root.appending(path: "Kassiber Two.app/Contents/MacOS/kassiber_native")
        let second = TerminalCommandManager(
            homeDirectory: root,
            targetExecutable: nextTarget,
            path: localBin.path
        )
        #expect(try second.status().needsRepair)
        let repaired = try second.install()
        #expect(repaired.installed)
        #expect(try String(contentsOf: repaired.commandURL, encoding: .utf8).contains(nextTarget.path))
        #expect(try second.remove().installed == false)
        #expect(!FileManager.default.fileExists(atPath: repaired.commandURL.path))
    }

    @Test("terminal launcher refuses to replace or remove a foreign command")
    func preservesConflict() throws {
        let root = FileManager.default.temporaryDirectory
            .appending(path: "kassiber-terminal-conflict-\(UUID().uuidString)", directoryHint: .isDirectory)
        let bin = root.appending(path: ".local/bin", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let command = bin.appending(path: "kassiber")
        try "#!/bin/sh\necho foreign\n".write(to: command, atomically: true, encoding: .utf8)
        let manager = TerminalCommandManager(
            homeDirectory: root,
            targetExecutable: root.appending(path: "kassiber_native"),
            path: bin.path
        )

        #expect(try manager.status().conflict)
        do {
            _ = try manager.install()
            Issue.record("install unexpectedly replaced a foreign command")
        } catch { /* expected */ }
        do {
            _ = try manager.remove()
            Issue.record("remove unexpectedly deleted a foreign command")
        } catch { /* expected */ }
        #expect(try String(contentsOf: command, encoding: .utf8).contains("foreign"))
    }
}
