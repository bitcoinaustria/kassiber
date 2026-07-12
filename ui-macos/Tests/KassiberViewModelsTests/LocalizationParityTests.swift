import Foundation
import Testing

@Suite("Localization")
struct LocalizationParityTests {
    @Test("English, Austrian German, presentation codes, and String Catalog stay in lockstep")
    func catalogParity() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let process = Process()
        let output = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["python3", "Scripts/sync_string_catalog.py", "--check"]
        process.currentDirectoryURL = packageRoot
        process.standardOutput = output
        process.standardError = output
        try process.run()
        process.waitUntilExit()
        let diagnostics = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        #expect(process.terminationStatus == 0, Comment(rawValue: diagnostics))
    }
}
