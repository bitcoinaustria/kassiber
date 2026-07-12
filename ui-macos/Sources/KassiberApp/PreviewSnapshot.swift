import AppKit
import CoreGraphics
import CryptoKit
import Foundation
import KassiberViewModels
import ScreenCaptureKit

@MainActor
private final class PreviewWindowCapture {
    static let appKitBackend = "AppKit.NSView.cacheDisplay-own-window-detail"
    static let screenCaptureKitBackend = "ScreenCaptureKit.SCScreenshotManager-own-window"

    private let filter: SCContentFilter?
    private let configuration: SCStreamConfiguration?
    private weak var window: NSWindow?
    private let cropSidebar: Bool
    let backend: String

    private init(
        filter: SCContentFilter?,
        configuration: SCStreamConfiguration?,
        window: NSWindow?,
        cropSidebar: Bool,
        backend: String
    ) {
        self.filter = filter
        self.configuration = configuration
        self.window = window
        self.cropSidebar = cropSidebar
        self.backend = backend
    }

    static func start(
        window: NSWindow,
        environment: [String: String]
    ) async throws -> PreviewWindowCapture {
        if environment["KASSIBER_PREVIEW_CAPTURE_BACKEND"] == "appkit" {
            return PreviewWindowCapture(
                filter: nil,
                configuration: nil,
                window: window,
                cropSidebar: environment["KASSIBER_PREVIEW_ONBOARDING"] != "1",
                backend: appKitBackend
            )
        }
        let content = try await SCShareableContent.excludingDesktopWindows(
            false,
            onScreenWindowsOnly: true
        )
        guard let shareableWindow = content.windows.first(where: {
            $0.windowID == CGWindowID(window.windowNumber)
        }) else {
            throw PreviewCaptureError.windowUnavailable
        }
        let scale = window.screen?.backingScaleFactor
            ?? NSScreen.main?.backingScaleFactor
            ?? 2
        let configuration = SCStreamConfiguration()
        configuration.width = max(1, Int(shareableWindow.frame.width * scale))
        configuration.height = max(1, Int(shareableWindow.frame.height * scale))
        configuration.scalesToFit = true
        configuration.showsCursor = false
        configuration.capturesAudio = false
        let filter = SCContentFilter(desktopIndependentWindow: shareableWindow)
        return PreviewWindowCapture(
            filter: filter,
            configuration: configuration,
            window: nil,
            cropSidebar: false,
            backend: screenCaptureKitBackend
        )
    }

    func image() async throws -> CGImage {
        if let filter, let configuration {
            return try await SCScreenshotManager.captureImage(
                contentFilter: filter,
                configuration: configuration
            )
        }
        guard let window,
              let view = window.contentView?.superview ?? window.contentView else {
            throw PreviewCaptureError.windowUnavailable
        }
        window.displayIfNeeded()
        view.layoutSubtreeIfNeeded()
        let bounds = view.bounds
        guard !bounds.isEmpty,
              let bitmap = view.bitmapImageRepForCachingDisplay(in: bounds) else {
            throw PreviewCaptureError.frameUnavailable("window backing store")
        }
        bitmap.size = bounds.size
        view.cacheDisplay(in: bounds, to: bitmap)
        guard let image = bitmap.cgImage else {
            throw PreviewCaptureError.frameUnavailable("window backing store")
        }
        guard cropSidebar else { return image }
        let boundary = navigationDetailBoundary(in: view)
            ?? min(252, view.bounds.width * 0.25)
        let scale = CGFloat(image.width) / max(view.bounds.width, 1)
        let pixelX = max(0, min(image.width - 1, Int((boundary * scale).rounded())))
        let crop = CGRect(x: pixelX, y: 0, width: image.width - pixelX, height: image.height)
        return image.cropping(to: crop) ?? image
    }

    private func navigationDetailBoundary(in root: NSView) -> CGFloat? {
        var candidates: [(area: CGFloat, boundary: CGFloat)] = []
        func inspect(_ parent: NSView) {
            if let split = parent as? NSSplitView,
               split.isVertical,
               split.arrangedSubviews.count >= 2 {
                let frame = split.convert(split.arrangedSubviews[1].frame, to: root)
                let splitFrame = split.convert(split.bounds, to: root)
                if frame.minX > root.bounds.minX,
                   frame.minX < root.bounds.maxX,
                   splitFrame.width > 700,
                   splitFrame.height > 500 {
                    candidates.append((splitFrame.width * splitFrame.height, frame.minX))
                }
            }
            parent.subviews.forEach(inspect)
        }
        inspect(root)
        return candidates.max(by: { $0.area < $1.area })?.boundary
    }
}

private enum PreviewCaptureError: Error {
    case windowUnavailable
    case screenUnavailable
    case frameUnavailable(String)
    case invalidPlan(String)
}

/// Test-only, opt-in capture of Kassiber's own visible native window.
///
/// Batch plans keep one app process, navigate each native route, and render a
/// fresh frame of that app's own window after the route settles. The default
/// path uses ScreenCaptureKit. Headless verification can explicitly select the
/// AppKit backing-store path, which stays process-local and never captures the
/// desktop or another application's pixels. The verification script decodes
/// each PNG, rejects blank or uniform frames, and binds the complete corpus to
/// the packaged executable.
@MainActor
enum PreviewSnapshot {
    private struct CaptureReceipt: Codable {
        let schemaVersion: Int
        let backend: String
        let file: String
        let screen: String
        let language: String
        let width: Int
        let height: Int
        let byteCount: Int
        let sha256: String
        let executableSHA256: String
        let product: String
        let bundleID: String
        let version: String
        let build: String
        let commit: String
        let capturedAt: String

        enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case backend, file, screen, language, width, height
            case byteCount = "byte_count"
            case sha256
            case executableSHA256 = "executable_sha256"
            case product
            case bundleID = "bundle_id"
            case version, build, commit
            case capturedAt = "captured_at"
        }
    }

    private struct PlanEntry {
        let screen: AppScreen
        let filename: String
    }

    private static var cachedExecutableSHA256: String?

    static func captureIfRequested(navigate: @escaping (AppScreen) -> Void) async {
        let environment = ProcessInfo.processInfo.environment
        let outputPath = environment["KASSIBER_PREVIEW_OUTPUT"]
        let rawPlan = environment["KASSIBER_PREVIEW_PLAN"]
        guard outputPath?.isEmpty == false || rawPlan?.isEmpty == false else { return }

        do {
            let window = try await preparedWindow(environment: environment)
            let session = try await PreviewWindowCapture.start(
                window: window,
                environment: environment
            )
            if let rawPlan, !rawPlan.isEmpty {
                try await capturePlan(
                    parsePlan(rawPlan),
                    session: session,
                    navigate: navigate,
                    environment: environment
                )
            } else if let outputPath, !outputPath.isEmpty {
                try? await Task.sleep(for: .seconds(routeDelay(environment)))
                try write(
                    try await session.image(),
                    to: URL(fileURLWithPath: outputPath),
                    screen: environment["KASSIBER_PREVIEW_ONBOARDING"] == "1"
                        ? "onboarding"
                        : environment["KASSIBER_PREVIEW_SCREEN"] ?? "unknown",
                    backend: session.backend,
                    environment: environment
                )
            }
            if let done = environment["KASSIBER_PREVIEW_DONE"], !done.isEmpty {
                try Data("ok\n".utf8).write(
                    to: URL(fileURLWithPath: done),
                    options: .atomic
                )
            }
        } catch {
            writeDiagnostic("native preview capture failed: \(error)")
            if let failure = environment["KASSIBER_PREVIEW_FAILED"], !failure.isEmpty {
                try? Data(String(describing: error).utf8).write(
                    to: URL(fileURLWithPath: failure),
                    options: .atomic
                )
            }
        }
    }

    private static func preparedWindow(environment: [String: String]) async throws -> NSWindow {
        var previewWindow: NSWindow?
        let onboarding = environment["KASSIBER_PREVIEW_ONBOARDING"] == "1"
        // A direct packaged-Mach-O launch can create the WindowGroup on a
        // non-active Space.  Activate the exact process before checking
        // visibility so the capture session never waits on an otherwise
        // perfectly rendered but off-Space native window.
        NSApplication.shared.activate(ignoringOtherApps: true)
        for _ in 0..<360 {
            let contentWindows = NSApplication.shared.windows.filter {
                $0.contentView != nil && $0.frame.width > 600 && $0.frame.height > 400
            }
            if onboarding {
                previewWindow = contentWindows.first(where: {
                    $0.isVisible && $0.frame.width < 1_200
                })
                    ?? contentWindows.first(where: \.isVisible)
                    ?? contentWindows.first(where: \.isKeyWindow)
                    ?? contentWindows.first
            } else {
                previewWindow = contentWindows.first(where: { $0.isVisible && $0.isKeyWindow })
                    ?? contentWindows.first(where: \.isVisible)
                    ?? contentWindows.first(where: \.isKeyWindow)
                    ?? contentWindows.first
            }
            if previewWindow != nil { break }
            try? await Task.sleep(for: .milliseconds(250))
        }
        guard let window = previewWindow else { throw PreviewCaptureError.windowUnavailable }
        if environment["KASSIBER_PREVIEW_CAPTURE_BACKEND"] == "appkit" {
            window.appearance = NSAppearance(named: .aqua)
        }
        let width = max(980, Int(environment["KASSIBER_PREVIEW_WIDTH"] ?? "") ?? 1_440)
        let height = max(640, Int(environment["KASSIBER_PREVIEW_HEIGHT"] ?? "") ?? 900)
        window.setContentSize(NSSize(width: width, height: height))
        guard let screen = NSScreen.main ?? window.screen else {
            throw PreviewCaptureError.screenUnavailable
        }
        window.setFrameOrigin(NSPoint(
            x: screen.frame.midX - window.frame.width / 2,
            y: screen.frame.midY - window.frame.height / 2
        ))
        window.sharingType = .readOnly
        window.makeKeyAndOrderFront(nil)
        window.contentView?.needsLayout = true
        window.contentView?.layoutSubtreeIfNeeded()
        return window
    }

    private static func capturePlan(
        _ plan: [PlanEntry],
        session: PreviewWindowCapture,
        navigate: @escaping (AppScreen) -> Void,
        environment: [String: String]
    ) async throws {
        guard let directoryPath = environment["KASSIBER_PREVIEW_OUTPUT_DIR"],
              !directoryPath.isEmpty else {
            throw PreviewCaptureError.invalidPlan("missing output directory")
        }
        let directory = URL(fileURLWithPath: directoryPath, isDirectory: true)
        for entry in plan {
            navigate(entry.screen)
            try? await Task.sleep(for: .seconds(routeDelay(environment)))
            try write(
                try await session.image(),
                to: directory.appending(path: entry.filename),
                screen: entry.screen.rawValue,
                backend: session.backend,
                environment: environment
            )
        }
    }

    private static func parsePlan(_ raw: String) throws -> [PlanEntry] {
        let entries = try raw.split(separator: ",").map { item -> PlanEntry in
            let fields = item.split(separator: "|", maxSplits: 1)
            let filename = fields.count == 2 ? String(fields[1]) : ""
            guard fields.count == 2,
                  let screen = AppScreen(rawValue: String(fields[0])),
                  validFilename(filename) else {
                throw PreviewCaptureError.invalidPlan(String(item))
            }
            return PlanEntry(screen: screen, filename: filename)
        }
        guard !entries.isEmpty else { throw PreviewCaptureError.invalidPlan("empty plan") }
        return entries
    }

    private static func validFilename(_ filename: String) -> Bool {
        !filename.isEmpty
            && filename.hasSuffix(".png")
            && URL(fileURLWithPath: filename).lastPathComponent == filename
            && !filename.contains("\t")
            && !filename.contains("\n")
    }

    private static func routeDelay(_ environment: [String: String]) -> Int {
        max(1, Int(environment["KASSIBER_PREVIEW_DELAY_SECONDS"] ?? "") ?? 5)
    }

    private static func write(
        _ image: CGImage,
        to output: URL,
        screen: String,
        backend: String,
        environment: [String: String]
    ) throws {
        let bitmap = NSBitmapImageRep(cgImage: image)
        guard let png = bitmap.representation(using: .png, properties: [:]) else {
            throw PreviewCaptureError.frameUnavailable(output.lastPathComponent)
        }
        try png.write(to: output, options: .atomic)
        try writeReceipt(
            png: png,
            image: image,
            output: output,
            screen: screen,
            backend: backend,
            environment: environment
        )
    }

    private static func writeReceipt(
        png: Data,
        image: CGImage,
        output: URL,
        screen: String,
        backend: String,
        environment: [String: String]
    ) throws {
        guard let receiptDirectory = environment["KASSIBER_PREVIEW_RECEIPT_DIR"],
              !receiptDirectory.isEmpty else { return }
        let directory = URL(fileURLWithPath: receiptDirectory, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let bundle = Bundle.main
        let rawLanguage = environment["KASSIBER_LANGUAGE"] ?? "en"
        let receipt = CaptureReceipt(
            schemaVersion: 1,
            backend: backend,
            file: output.lastPathComponent,
            screen: screen,
            language: rawLanguage == "de" ? "de-AT" : rawLanguage,
            width: image.width,
            height: image.height,
            byteCount: png.count,
            sha256: digest(png),
            executableSHA256: try executableDigest(),
            product: bundle.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "",
            bundleID: bundle.bundleIdentifier ?? "",
            version: bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "",
            build: bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "",
            commit: bundle.object(forInfoDictionaryKey: "KassiberBuildCommit") as? String ?? "",
            capturedAt: ISO8601DateFormatter().string(from: Date())
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        var payload = try encoder.encode(receipt)
        payload.append(0x0A)
        try payload.write(
            to: directory.appending(path: output.lastPathComponent + ".capture.json"),
            options: .atomic
        )
    }

    private static func executableDigest() throws -> String {
        if let cachedExecutableSHA256 { return cachedExecutableSHA256 }
        guard let executable = Bundle.main.executableURL else {
            throw PreviewCaptureError.frameUnavailable("missing executable")
        }
        let value = digest(try Data(contentsOf: executable, options: .mappedIfSafe))
        cachedExecutableSHA256 = value
        return value
    }

    private static func digest(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    private static func writeDiagnostic(_ message: String) {
        try? FileHandle.standardError.write(contentsOf: Data("\(message)\n".utf8))
    }
}
