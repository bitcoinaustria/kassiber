import Foundation
import KassiberDaemonKit

extension Dictionary where Key == String, Value == JSONValue {
    func string(_ keys: String...) -> String? {
        for key in keys {
            if let value = self[key]?.stringValue { return value }
        }
        return nil
    }

    func int(_ keys: String...) -> Int64? {
        for key in keys {
            if let value = self[key]?.intValue { return value }
        }
        return nil
    }

    func double(_ keys: String...) -> Double? {
        for key in keys {
            if let value = self[key]?.doubleValue { return value }
        }
        return nil
    }

    func bool(_ keys: String...) -> Bool? {
        for key in keys {
            if let value = self[key]?.boolValue { return value }
        }
        return nil
    }

    func objects(_ keys: String...) -> [[String: JSONValue]] {
        for key in keys {
            if let values = self[key]?.arrayValue {
                return values.compactMap(\.objectValue)
            }
        }
        return []
    }
}

enum DaemonValueParser {
    static func date(_ value: String?) -> Date? {
        guard let value, !value.isEmpty else { return nil }
        if let parsed = ISO8601DateFormatter().date(from: value) { return parsed }
        let formats = ["yyyy-MM-dd HH:mm", "yyyy-MM-dd"]
        for format in formats {
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.dateFormat = format
            formatter.timeZone = TimeZone(secondsFromGMT: 0)
            if let parsed = formatter.date(from: value) { return parsed }
        }
        return nil
    }
}
