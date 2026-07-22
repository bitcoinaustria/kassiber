import Foundation
import Sparkle

/// Sparkle is intentionally dormant until a signed distribution supplies
/// SUFeedURL and SUPublicEDKey in its Info.plist. Development builds never
/// contact an update server.
@MainActor
final class UpdaterScaffold {
    static let shared = UpdaterScaffold()
    private let controller: SPUStandardUpdaterController?

    private init() {
        guard Bundle.main.object(forInfoDictionaryKey: "SUFeedURL") != nil,
              Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") != nil else {
            controller = nil
            return
        }
        controller = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: nil,
            userDriverDelegate: nil
        )
    }

    var isConfigured: Bool { controller != nil }

    func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }
}
