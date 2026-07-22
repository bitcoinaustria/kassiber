import Foundation

public enum KassiberFormatting {
    public static func sats(_ value: Int64, locale: Locale = .current) -> String {
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .decimal
        formatter.maximumFractionDigits = 0
        return (formatter.string(from: NSNumber(value: value)) ?? String(value)) + " sats"
    }

    public static func btc(fromSats value: Int64, locale: Locale = .current) -> String {
        let amount = Decimal(value) / Decimal(100_000_000)
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = 0
        formatter.maximumFractionDigits = 8
        return (formatter.string(from: amount as NSDecimalNumber) ?? "0") + " BTC"
    }

    public static func btc(_ value: Double, locale: Locale = .current) -> String {
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .decimal
        formatter.minimumFractionDigits = 0
        formatter.maximumFractionDigits = 8
        return (formatter.string(from: NSNumber(value: value)) ?? "0") + " BTC"
    }

    public static func fiat(
        _ value: Double,
        currency: String,
        locale: Locale = .current
    ) -> String {
        let formatter = NumberFormatter()
        formatter.locale = locale
        formatter.numberStyle = .currency
        formatter.currencyCode = currency.uppercased()
        formatter.minimumFractionDigits = 2
        formatter.maximumFractionDigits = 2
        return formatter.string(from: NSNumber(value: value)) ?? "\(value) \(currency.uppercased())"
    }

    public static func date(_ value: Date, locale: Locale = .current) -> String {
        value.formatted(
            .dateTime.locale(locale).year().month(.abbreviated).day().hour().minute()
        )
    }
}
