import Foundation
import Observation
import KassiberDaemonKit

public struct TerminalCommandStatus: Equatable, Sendable {
    public let available: Bool
    public let installed: Bool
    public let managed: Bool
    public let needsRepair: Bool
    public let conflict: Bool
    public let pathOnPath: Bool
    public let command: String
    public let binDirectory: URL
    public let commandURL: URL
    public let targetURL: URL
    public let pathHint: String
    public let message: String
}

public enum TerminalCommandError: Error, Equatable, Sendable, CustomStringConvertible {
    case operation(String)
    public var description: String { switch self { case let .operation(message): message } }
}

public struct TerminalCommandManager {
    public static let marker = "Kassiber desktop CLI launcher. Managed by Kassiber Settings."
    private let homeDirectory: URL
    private let targetExecutable: URL
    private let pathEntries: [URL]
    private let fileManager: FileManager

    public init(
        homeDirectory: URL = FileManager.default.homeDirectoryForCurrentUser,
        targetExecutable: URL? = Bundle.main.executableURL,
        path: String? = ProcessInfo.processInfo.environment["PATH"],
        fileManager: FileManager = .default
    ) {
        self.homeDirectory = homeDirectory
        self.targetExecutable = targetExecutable ?? URL(fileURLWithPath: CommandLine.arguments.first ?? "kassiber_native")
        self.pathEntries = (path ?? "").split(separator: ":").map { URL(fileURLWithPath: String($0), isDirectory: true).standardizedFileURL }
        self.fileManager = fileManager
    }

    public func status() throws -> TerminalCommandStatus {
        let paths = commandPaths()
        let state = try inspect(commandURL: paths.command, targetURL: paths.target)
        let pathOnPath = pathEntries.contains(paths.bin.standardizedFileURL)
        let installed = state == .current
        let managed = state == .current || state == .managedStale
        let needsRepair = state == .managedStale
        let conflict = state == .conflict
        let message: String
        if conflict { message = "A different command already exists at the install path." }
        else if needsRepair { message = "The terminal command is managed by Kassiber but points at an older app path." }
        else if installed && pathOnPath { message = "The terminal command is installed and appears on PATH." }
        else if installed { message = "The terminal command is installed; add its folder to PATH if your shell cannot find it." }
        else { message = "Install the user-local terminal command to run kassiber from a shell." }
        return TerminalCommandStatus(
            available: true, installed: installed, managed: managed,
            needsRepair: needsRepair, conflict: conflict, pathOnPath: pathOnPath,
            command: "kassiber", binDirectory: paths.bin, commandURL: paths.command,
            targetURL: paths.target, pathHint: pathHint(paths.bin), message: message
        )
    }

    public func install() throws -> TerminalCommandStatus {
        let paths = commandPaths()
        switch try inspect(commandURL: paths.command, targetURL: paths.target) {
        case .missing: break
        case .current: return try status()
        case .managedStale: try fileManager.removeItem(at: paths.command)
        case .conflict:
            throw TerminalCommandError.operation(
                "\(paths.command.path) already exists and is not managed by Kassiber. Move it aside first."
            )
        }
        try fileManager.createDirectory(at: paths.bin, withIntermediateDirectories: true)
        try launcherContents(targetURL: paths.target).write(to: paths.command, atomically: true, encoding: .utf8)
        try fileManager.setAttributes([.posixPermissions: 0o755], ofItemAtPath: paths.command.path)
        return try status()
    }

    public func remove() throws -> TerminalCommandStatus {
        let paths = commandPaths()
        switch try inspect(commandURL: paths.command, targetURL: paths.target) {
        case .missing: break
        case .current, .managedStale: try fileManager.removeItem(at: paths.command)
        case .conflict:
            throw TerminalCommandError.operation(
                "\(paths.command.path) is not managed by Kassiber, so it was left untouched."
            )
        }
        return try status()
    }

    public func launcherContents(targetURL: URL? = nil) -> String {
        let target = (targetURL ?? targetExecutable).path
        let quoted = "'\(target.replacingOccurrences(of: "'", with: "'\"'\"'"))'"
        return "#!/bin/sh\n# \(Self.marker)\n# target: \(target)\nexec \(quoted) --cli \"$@\"\n"
    }

    private enum FileState { case missing, current, managedStale, conflict }

    private func commandPaths() -> (bin: URL, command: URL, target: URL) {
        let local = homeDirectory.appending(path: ".local/bin", directoryHint: .isDirectory)
        let legacy = homeDirectory.appending(path: "bin", directoryHint: .isDirectory)
        let bin = [local, legacy].first { pathEntries.contains($0.standardizedFileURL) } ?? local
        return (bin, bin.appending(path: "kassiber"), targetExecutable.standardizedFileURL)
    }

    private func inspect(commandURL: URL, targetURL: URL) throws -> FileState {
        var isDirectory: ObjCBool = false
        guard fileManager.fileExists(atPath: commandURL.path, isDirectory: &isDirectory) else { return .missing }
        let attributes = try fileManager.attributesOfItem(atPath: commandURL.path)
        if attributes[.type] as? FileAttributeType == .typeSymbolicLink {
            let destination = try fileManager.destinationOfSymbolicLink(atPath: commandURL.path)
            let destinationURL = URL(fileURLWithPath: destination, relativeTo: commandURL.deletingLastPathComponent()).standardizedFileURL
            return destinationURL == targetURL.standardizedFileURL ? .managedStale : .conflict
        }
        let contents = try String(contentsOf: commandURL, encoding: .utf8)
        if contents == launcherContents(targetURL: targetURL) { return .current }
        return contents.contains(Self.marker) ? .managedStale : .conflict
    }

    private func pathHint(_ bin: URL) -> String {
        let home = homeDirectory.standardizedFileURL.path
        let display = bin.standardizedFileURL.path.hasPrefix(home + "/")
            ? "$HOME/" + String(bin.standardizedFileURL.path.dropFirst(home.count + 1))
            : bin.path
        return "export PATH=\"\(display):$PATH\""
    }
}

@MainActor
@Observable
public final class NativePlatformSettingsViewModel {
    public private(set) var terminalStatus: TerminalCommandStatus?
    public private(set) var terminalError: String?
    public private(set) var isWorking = false
    private let terminal: TerminalCommandManager

    public init(terminal: TerminalCommandManager = TerminalCommandManager()) { self.terminal = terminal }

    public func loadTerminalStatus() { operate { try terminal.status() } }
    public func installTerminalCommand() { operate { try terminal.install() } }
    public func removeTerminalCommand() { operate { try terminal.remove() } }

    private func operate(_ operation: () throws -> TerminalCommandStatus) {
        isWorking = true; defer { isWorking = false }
        do { terminalStatus = try operation(); terminalError = nil }
        catch { terminalError = String(describing: error) }
    }
}
