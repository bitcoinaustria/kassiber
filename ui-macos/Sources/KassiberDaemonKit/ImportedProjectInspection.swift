import Foundation

public struct ImportedProjectSelection: Equatable, Sendable {
    public let stateRoot: String
    public let dataRoot: String
    public let database: String
    public let encrypted: Bool
}

public enum ImportedProjectInspectionError: Error, Equatable, Sendable, CustomStringConvertible {
    case invalid(String)
    public var description: String { switch self { case let .invalid(message): message } }
}

/// Strictly inspects a user-picked project folder without opening its database.
/// The rules mirror the Tauri host: legacy filenames are accepted, symlinks and
/// empty files are rejected, and plaintext SQLite must contain Kassiber schema
/// markers rather than merely sharing the `.sqlite3` extension.
public enum ImportedProjectInspector {
    private static let databaseNames = ["kassiber.sqlite3", "satbooks.sqlite3"]

    public static func inspect(_ pickedURL: URL, fileManager: FileManager = .default) throws -> ImportedProjectSelection {
        let picked = pickedURL.standardizedFileURL
        let attributes = try fileManager.attributesOfItem(atPath: picked.path)
        guard attributes[.type] as? FileAttributeType == .typeDirectory else {
            throw ImportedProjectInspectionError.invalid("Choose a Kassiber project or data folder.")
        }
        let canonical = picked.resolvingSymlinksInPath().standardizedFileURL
        let direct = try inspectDataRoot(canonical, fileManager: fileManager)
        let nestedURL = canonical.appending(path: "data", directoryHint: .isDirectory)
        let nested = try inspectDataRoot(nestedURL, fileManager: fileManager)

        let selection: (URL, URL, Bool)
        switch (direct, nested) {
        case let (_, nested?) where isManagedStateRoot(canonical, fileManager: fileManager):
            selection = nested
        case (_?, _?):
            throw ImportedProjectInspectionError.invalid(
                "Selected folder contains Kassiber databases both directly and under data/. Choose the exact data folder."
            )
        case let (direct?, nil): selection = direct
        case let (nil, nested?): selection = nested
        case (nil, nil):
            throw ImportedProjectInspectionError.invalid(
                "Choose a Kassiber project folder containing data/kassiber.sqlite3, or choose the data folder itself."
            )
        }
        let (dataRoot, database, encrypted) = selection
        let stateRoot = dataRoot.lastPathComponent == "data"
            ? dataRoot.deletingLastPathComponent()
            : dataRoot
        return ImportedProjectSelection(
            stateRoot: stateRoot.path,
            dataRoot: dataRoot.path,
            database: database.path,
            encrypted: encrypted
        )
    }

    private static func inspectDataRoot(
        _ root: URL,
        fileManager: FileManager
    ) throws -> (URL, URL, Bool)? {
        var isDirectory: ObjCBool = false
        guard fileManager.fileExists(atPath: root.path, isDirectory: &isDirectory) else { return nil }
        let rootAttributes = try fileManager.attributesOfItem(atPath: root.path)
        guard rootAttributes[.type] as? FileAttributeType != .typeSymbolicLink else {
            throw ImportedProjectInspectionError.invalid("Kassiber data folders must not be symlinks.")
        }
        guard isDirectory.boolValue else { return nil }
        let canonical = root.resolvingSymlinksInPath().standardizedFileURL
        for name in databaseNames {
            let database = canonical.appending(path: name)
            if let encrypted = try inspectDatabase(database, fileManager: fileManager) {
                return (canonical, database, encrypted)
            }
        }
        return nil
    }

    private static func inspectDatabase(_ url: URL, fileManager: FileManager) throws -> Bool? {
        guard fileManager.fileExists(atPath: url.path) else { return nil }
        let attributes = try fileManager.attributesOfItem(atPath: url.path)
        guard attributes[.type] as? FileAttributeType != .typeSymbolicLink else {
            throw ImportedProjectInspectionError.invalid("Kassiber database files must not be symlinks.")
        }
        guard attributes[.type] as? FileAttributeType == .typeRegular else { return nil }
        guard (attributes[.size] as? NSNumber)?.uint64Value ?? 0 > 0 else {
            throw ImportedProjectInspectionError.invalid("Kassiber database file is empty.")
        }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        let header = try handle.read(upToCount: 16) ?? Data()
        let encrypted = header.count < 16 || header != Data("SQLite format 3\0".utf8)
        if !encrypted {
            try handle.seek(toOffset: 0)
            let prefix = try handle.read(upToCount: 1_048_576) ?? Data()
            let lowered = String(decoding: prefix, as: UTF8.self).lowercased()
            let markers = ["create table", "settings", "workspaces", "profiles", "workspace_id", "fiat_currency"]
            guard markers.allSatisfy(lowered.contains) else {
                throw ImportedProjectInspectionError.invalid(
                    "Selected SQLite file does not contain Kassiber workspace/profile tables."
                )
            }
        }
        return encrypted
    }

    private static func isManagedStateRoot(_ root: URL, fileManager: FileManager) -> Bool {
        root.lastPathComponent == ".kassiber"
            || fileManager.fileExists(atPath: root.appending(path: "config/settings.json").path)
    }
}
