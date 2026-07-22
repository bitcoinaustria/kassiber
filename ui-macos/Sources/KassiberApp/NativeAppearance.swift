import SwiftUI
import KassiberViewModels

enum KassiberDisplayCurrency: String, CaseIterable {
    case bitcoin = "btc"
    case euro = "eur"
}

private struct KassiberDisplayCurrencyKey: EnvironmentKey {
    static let defaultValue = KassiberDisplayCurrency.bitcoin
}

extension EnvironmentValues {
    var kassiberDisplayCurrency: KassiberDisplayCurrency {
        get { self[KassiberDisplayCurrencyKey.self] }
        set { self[KassiberDisplayCurrencyKey.self] = newValue }
    }
}

/// Uses a recorded fiat value first and a supplied market rate second. When a
/// conversion is unavailable, it deliberately falls back to sats instead of
/// inventing a value or hiding the underlying accounting quantity.
func kassiberAmountString(
    sats: Int64,
    fiatValue: Double? = nil,
    rateEUR: Double? = nil,
    fiatCurrency: String = "EUR",
    mode: KassiberDisplayCurrency,
    locale: Locale
) -> String {
    if mode == .euro {
        let converted = fiatValue ?? rateEUR.map { Double(sats) / 100_000_000 * $0 }
        if let converted {
            return KassiberFormatting.fiat(converted, currency: fiatCurrency, locale: locale)
        }
    }
    return KassiberFormatting.sats(sats, locale: locale)
}

func kassiberBitcoinString(
    btc: Double,
    rateEUR: Double?,
    mode: KassiberDisplayCurrency,
    locale: Locale
) -> String {
    if mode == .euro, let rateEUR {
        return KassiberFormatting.fiat(btc * rateEUR, currency: "EUR", locale: locale)
    }
    return KassiberFormatting.btc(btc, locale: locale)
}

struct KassiberAmountText: View {
    let sats: Int64
    var fiatValue: Double?
    var rateEUR: Double?
    var fiatCurrency: String
    @Environment(\.kassiberDisplayCurrency) private var mode
    @Environment(\.locale) private var locale

    init(
        sats: Int64,
        fiatValue: Double? = nil,
        rateEUR: Double? = nil,
        fiatCurrency: String = "EUR"
    ) {
        self.sats = sats
        self.fiatValue = fiatValue
        self.rateEUR = rateEUR
        self.fiatCurrency = fiatCurrency
    }

    init(transaction: TransactionRow, rateEUR: Double? = nil) {
        sats = transaction.amountSats
        fiatValue = transaction.fiatValue
        self.rateEUR = transaction.rate ?? rateEUR
        fiatCurrency = transaction.fiatCurrency
    }

    var body: some View {
        Text(kassiberAmountString(
            sats: sats,
            fiatValue: fiatValue,
            rateEUR: rateEUR,
            fiatCurrency: fiatCurrency,
            mode: mode,
            locale: locale
        ))
    }
}
