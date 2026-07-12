import Foundation
import Testing
@testable import KassiberViewModels

@Suite("Native host intents")
struct NativeHostIntentTests {
    @Test("deep links mirror the fixed Tauri route aliases")
    func routes() throws {
        let expectations: [(String, NativeHostIntent)] = [
            ("kassiber://overview", .navigate(.dashboard)),
            ("kassiber://transactions", .navigate(.transactions)),
            ("kassiber://source-of-funds", .navigate(.sourceFunds)),
            ("kassiber://tax-events", .navigate(.journals)),
            ("kassiber://diagnostics", .navigate(.logs)),
            ("kassiber://assistant", .navigate(.assistant)),
        ]
        for (raw, expected) in expectations {
            #expect(NativeHostIntent.parse(try #require(URL(string: raw))) == expected)
        }
    }

    @Test("settings aliases and workflow actions are case insensitive")
    func actions() throws {
        #expect(
            NativeHostIntent.parse(try #require(URL(string: "KASSIBER://SETTINGS/Privacy")))
                == .openSettings(section: "privacy")
        )
        #expect(
            NativeHostIntent.parse(try #require(URL(string: "kassiber://settings/not-allowed")))
                == .openSettings(section: nil)
        )
        #expect(
            NativeHostIntent.parse(try #require(URL(string: "kassiber://workflow/sync")))
                == .syncAllWallets
        )
        #expect(
            NativeHostIntent.parse(try #require(URL(string: "kassiber://workflow/process-journals")))
                == .processJournals
        )
        #expect(NativeHostIntent.parse(try #require(URL(string: "kassiber://lock"))) == .lockApp)
    }

    @Test("unknown schemes, hosts, paths, and workflows are ignored")
    func rejectsUnknownLinks() throws {
        let links = [
            "https://overview",
            "kassiber://unknown",
            "kassiber://overview/extra",
            "kassiber://lock/extra",
            "kassiber://workflow/delete-everything",
        ]
        for raw in links {
            #expect(NativeHostIntent.parse(try #require(URL(string: raw))) == nil)
        }
    }

    @Test("all public settings aliases resolve onto the native settings rail")
    func settingsRailMapping() {
        let expectations: [String: String] = [
            "appearance": "general", "display": "general",
            "privacy": "privacy", "developer": "general", "logs": "general",
            "explorer": "bitcoin", "explorers": "bitcoin",
            "bitcoin": "bitcoin", "backends": "bitcoin",
            "lightning": "lightning", "liquid": "liquid",
            "market": "market", "rates": "market",
            "security": "security", "lock": "security",
            "ai": "assistant", "assistant": "assistant",
            "sync": "replication", "replication": "replication",
            "data": "data", "storage": "data",
            "desktop": "terminal", "terminal": "terminal",
        ]
        #expect(Set(expectations.keys) == NativeHostIntent.settingsSections)
        for (slug, section) in expectations {
            #expect(NativeHostIntent.nativeSettingsSection(for: slug) == section)
        }
        #expect(NativeHostIntent.nativeSettingsSection(for: nil) == "general")
    }
}
