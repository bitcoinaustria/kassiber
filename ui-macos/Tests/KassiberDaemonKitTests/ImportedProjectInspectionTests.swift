import Foundation
import Testing
@testable import KassiberDaemonKit

@Suite("Imported project inspection")
struct ImportedProjectInspectionTests {
    @Test("accepts a managed project root with a Kassiber plaintext database")
    func managedPlaintext() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let data = root.appending(path: "data", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: data, withIntermediateDirectories: true)
        let markers = "SQLite format 3\0 create table settings workspaces profiles workspace_id fiat_currency"
        try Data(markers.utf8).write(to: data.appending(path: "kassiber.sqlite3"))

        let result = try ImportedProjectInspector.inspect(root)
        #expect(result.stateRoot == root.path)
        #expect(result.dataRoot == data.path)
        #expect(result.encrypted == false)
    }

    @Test("accepts a direct encrypted legacy database")
    func encryptedLegacy() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        try Data("ciphertext".utf8).write(to: root.appending(path: "satbooks.sqlite3"))
        let result = try ImportedProjectInspector.inspect(root)
        #expect(result.dataRoot == root.path)
        #expect(result.encrypted)
    }

    @Test("rejects ambiguous ad-hoc roots")
    func ambiguous() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let data = root.appending(path: "data", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: data, withIntermediateDirectories: true)
        try Data("cipher-direct".utf8).write(to: root.appending(path: "kassiber.sqlite3"))
        try Data("cipher-nested".utf8).write(to: data.appending(path: "kassiber.sqlite3"))
        #expect(throws: ImportedProjectInspectionError.self) {
            try ImportedProjectInspector.inspect(root)
        }
    }

    private func temporaryDirectory() throws -> URL {
        let root = FileManager.default.temporaryDirectory
            .appending(path: "kassiber-project-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return root.resolvingSymlinksInPath().standardizedFileURL
    }
}
