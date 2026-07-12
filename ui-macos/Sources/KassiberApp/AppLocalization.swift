import Foundation
import SwiftUI

enum AppLocalization {
    static var languageCode: String {
        if let override = ProcessInfo.processInfo.environment["KASSIBER_LANGUAGE"],
           override.lowercased().hasPrefix("de") {
            return "de"
        }
        if let saved = UserDefaults.standard.string(forKey: "language"),
           saved.lowercased().hasPrefix("de") {
            return "de"
        }
        if Locale.preferredLanguages.first?.lowercased().hasPrefix("de") == true {
            return "de"
        }
        return "en"
    }

    static var locale: Locale {
        Locale(identifier: languageCode == "de" ? "de_AT" : "en_US")
    }

    static func string(_ key: String) -> String {
        let localization = languageCode == "de" ? "de" : "en"
        let bundle = Bundle.module.path(forResource: localization, ofType: "lproj")
            .flatMap(Bundle.init(path:))
            ?? .module
        return NSLocalizedString(key, bundle: bundle, value: key, comment: "")
    }

    /// Maps stable daemon/status codes at the presentation boundary. Unknown
    /// codes remain readable without translating daemon payloads themselves.
    static func code(_ value: String) -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return trimmed }
        let canonical = trimmed
            .lowercased()
            .replacingOccurrences(of: "-", with: "_")
            .replacingOccurrences(of: " ", with: "_")
        let key = "code.\(canonical)"
        let translated = string(key)
        if translated != key { return translated }
        let letters = trimmed.filter(\.isLetter)
        if !letters.isEmpty, letters == letters.uppercased() { return trimmed }
        return trimmed
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: "-", with: " ")
            .localizedCapitalized
    }

    /// Localizes validation keys and stable daemon codes without leaking an
    /// English daemon exception into the German UI. Unknown transport details
    /// remain available in the RAM-only Logs surface; user-facing screens use
    /// a safe localized fallback.
    static func error(_ value: String) -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return string("error.operationFailed") }

        let direct = string(trimmed)
        if direct != trimmed { return direct }

        let literalAliases: [String: String] = [
            "The overview response had an unexpected format.": "error.unexpectedOverview",
            "The transaction response had an unexpected format.": "error.unexpectedTransactions",
            "The report response had an unexpected format.": "reportsParity.error.unexpectedReport",
            "Transaction not found.": "error.transactionNotFound",
        ]
        if let key = literalAliases[trimmed] { return string(key) }

        let canonical = trimmed
            .lowercased()
            .replacingOccurrences(of: "-", with: "_")
            .replacingOccurrences(of: " ", with: "_")
        for key in [
            "error.\(canonical)",
            "onboarding.error.\(canonical)",
            "code.\(canonical)",
        ] {
            let translated = string(key)
            if translated != key { return translated }
        }
        return string("error.operationFailed")
    }
}
