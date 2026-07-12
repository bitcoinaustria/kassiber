import Foundation

enum NativeCLIForwarder {
    static func forwardedArguments(_ arguments: [String] = CommandLine.arguments) -> [String]? {
        guard arguments.count > 1 else { return nil }
        let userArguments = arguments.dropFirst().filter { !$0.hasPrefix("-psn_") }
        guard let first = userArguments.first, first == "--cli" || first == "cli" else { return nil }
        return Array(userArguments.dropFirst())
    }

    static func runIfRequested(
        arguments: [String] = CommandLine.arguments,
        resourceURL: URL? = Bundle.main.resourceURL,
        repositoryRoot: URL? = nil
    ) -> Int32? {
        guard let forwarded = forwardedArguments(arguments) else { return nil }
        let process = Process()
        if let sidecar = resourceURL?.appending(path: "kassiber-sidecar"),
           FileManager.default.isExecutableFile(atPath: sidecar.path) {
            process.executableURL = sidecar
            process.arguments = forwarded
            process.currentDirectoryURL = resourceURL
        } else {
            let root = repositoryRoot ?? developmentRepositoryRoot()
            let python = root.appending(path: ".venv/bin/python")
            if FileManager.default.isExecutableFile(atPath: python.path) {
                process.executableURL = python
                process.arguments = ["-m", "kassiber"] + forwarded
            } else {
                process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
                process.arguments = ["uv", "run", "python", "-m", "kassiber"] + forwarded
            }
            process.currentDirectoryURL = root
        }
        do {
            try process.run()
            process.waitUntilExit()
            return process.terminationStatus
        } catch {
            FileHandle.standardError.write(Data("kassiber: could not start bundled CLI: \(error)\n".utf8))
            return 1
        }
    }

    private static func developmentRepositoryRoot() -> URL {
        if let configured = ProcessInfo.processInfo.environment["KASSIBER_REPO_ROOT"], !configured.isEmpty {
            return URL(fileURLWithPath: configured, isDirectory: true)
        }
        return URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
    }
}
