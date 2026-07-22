import Foundation

public struct NewTransactionEvidence: Equatable, Sendable {
    public var transactionReference = ""
    public var btcpayInvoiceID = ""
    public var exchangeCSVRow = ""
    public var swapID = ""
    public var preimage = ""

    public init() {}

    public var primary: String? {
        [transactionReference, btcpayInvoiceID, exchangeCSVRow, swapID, preimage]
            .first { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }
}

public struct NewTransactionDraft: Equatable, Sendable {
    public static let networks = ["Bitcoin", "Lightning", "Liquid", "Ecash", "Exchange", "Other"]
    public static let flows = ["incoming", "outgoing", "transfer", "swap", "layer-transition"]
    public static let pricingSources = ["manual_override", "generic_import", "fmv_provider", "missing"]

    public var network = "Bitcoin"
    public var flow = "incoming"
    public var occurredAt: Date
    public var confirmedAt: Date?
    public var wallet = ""
    public var fromWallet = ""
    public var toWallet = ""
    public var fromExternal = ""
    public var toExternal = ""
    public var swapService = ""
    public var asset = "BTC"
    public var amountSats = ""
    public var sendAsset = "BTC"
    public var receiveAsset = "BTC"
    public var sendAmountSats = ""
    public var receiveAmountSats = ""
    public var feeSats = ""
    public var pricingSource = "manual_override"
    public var fiatCurrency = "EUR"
    public var pricePerBTC = ""
    public var totalValue = ""
    public var movementID = ""
    public var classification = "Income"
    public var taxTreatment = "neu:income_general"
    public var tags = ""
    public var note = ""
    public var evidence = NewTransactionEvidence()

    public init(now: Date = Date(), wallet: String = "") {
        occurredAt = now
        self.wallet = wallet
        fromWallet = wallet
        toWallet = wallet
    }

    public var isTwoLeg: Bool { flow == "swap" || flow == "layer-transition" }
    public var showsConfirmation: Bool { network == "Bitcoin" || network == "Liquid" }
    public var amountBTC: Double { Double(Self.parseSats(amountSats) ?? 0) / 100_000_000 }
    public var sendBTC: Double { Double(Self.parseSats(sendAmountSats) ?? 0) / 100_000_000 }
    public var receiveBTC: Double { Double(Self.parseSats(receiveAmountSats) ?? 0) / 100_000_000 }
    public var signedBTC: Double {
        if isTwoLeg { return receiveBTC - sendBTC }
        if flow == "outgoing" { return -amountBTC }
        if flow == "transfer" { return 0 }
        return amountBTC
    }
    public var fromDisplay: String {
        flow == "incoming" ? (fromExternal.isEmpty ? "External" : fromExternal) : (fromWallet.isEmpty ? "—" : fromWallet)
    }
    public var toDisplay: String {
        flow == "outgoing" ? (toExternal.isEmpty ? "External" : toExternal) : (toWallet.isEmpty ? "—" : toWallet)
    }

    public mutating func selectNetwork(_ value: String) {
        guard Self.networks.contains(value) else { return }
        network = value
        if value == "Liquid" {
            if asset == "BTC" { asset = "LBTC" }
            if receiveAsset == "BTC" { receiveAsset = "LBTC" }
        }
    }

    public mutating func selectFlow(_ value: String) {
        guard Self.flows.contains(value) else { return }
        flow = value
        switch value {
        case "incoming":
            classification = "Income"; taxTreatment = "neu:income_general"
        case "transfer":
            classification = "Transfer"; taxTreatment = "outside:none"
        case "swap":
            classification = "Swap"; taxTreatment = "neu:neu_swap"
        case "layer-transition":
            classification = "Transfer"; taxTreatment = "neu:neu_swap"
        default:
            classification = "Expense"; taxTreatment = "neu:neu_gain"
        }
    }

    public mutating func updateAmount(_ value: String, field: String = "amount") {
        switch field {
        case "send": sendAmountSats = value
        case "receive": receiveAmountSats = value
        default: amountSats = value
        }
        recalculate(changed: "amount", preferredAmount: value)
    }

    public mutating func updatePrice(_ value: String) {
        pricePerBTC = value
        recalculate(changed: "price", preferredAmount: nil)
    }

    public mutating func updateTotal(_ value: String) {
        totalValue = value
        recalculate(changed: "total", preferredAmount: nil)
    }

    public static func parseSats(_ value: String) -> Int64? {
        let normalized = value.components(separatedBy: CharacterSet(charactersIn: ",_ ")).joined()
        guard !normalized.isEmpty, let parsed = Double(normalized), parsed.isFinite else { return nil }
        return Int64(abs(parsed).rounded(.towardZero))
    }

    private mutating func recalculate(changed: String, preferredAmount: String?) {
        let amountText = preferredAmount ?? (isTwoLeg
            ? (receiveAmountSats.isEmpty ? sendAmountSats : receiveAmountSats)
            : amountSats)
        guard let sats = Self.parseSats(amountText), sats > 0 else { return }
        let btc = Double(sats) / 100_000_000
        let price = Self.parseDecimal(pricePerBTC)
        let total = Self.parseDecimal(totalValue)
        if (changed == "amount" || changed == "price"), let price {
            totalValue = String(format: "%.2f", btc * price)
        } else if (changed == "amount" || changed == "total"), let total {
            pricePerBTC = String(format: "%.2f", total / btc)
        }
    }

    private static func parseDecimal(_ value: String) -> Double? {
        let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: ",", with: ".")
        guard !normalized.isEmpty, let parsed = Double(normalized), parsed.isFinite else { return nil }
        return parsed
    }
}
