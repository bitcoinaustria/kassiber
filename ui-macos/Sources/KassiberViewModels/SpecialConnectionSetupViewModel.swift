import Foundation
import Observation

public enum SpecialConnectionSetupMode: String, Equatable, Sendable {
    case btcpay
    case bullBitcoin
    case samourai
    case bip329
}

/// Owns the non-descriptor Add Connection workflows. The catalog model still
/// performs the daemon calls, while this coordinator owns form state,
/// cross-field validation, and the exact multi-stage submit routing.
@MainActor
@Observable
public final class SpecialConnectionSetupViewModel {
    public let mode: SpecialConnectionSetupMode
    public var label = ""
    public var selectedBackend = ""
    public var serverURL = ""
    public var apiKey = ""
    public var storeID = ""
    public var paymentMethods: Set<String> = ["BTC-CHAIN"]
    public var btcpayExistingWallets = false
    public var btcpayWalletRoutes: [String: String] = [:]
    public var btcpaySyncProvenance = true
    public var sourceFile = ""
    public var networks: Set<String> = ["bitcoin", "liquid", "lightning"]
    public var bullExistingWallets = false
    public var bullWalletRoutes: [String: String] = [:]
    public var network = "main"
    public var gapLimit = 20
    public var deposit = ""
    public var badbank = ""
    public var premix = ""
    public var postmix = ""
    public var ricochet = ""
    public var bip329ExportMode = "all"
    public var bip329ExportWallet = ""

    private let operations: ConnectionsParityViewModel

    public init(mode: SpecialConnectionSetupMode, operations: ConnectionsParityViewModel) {
        self.mode = mode
        self.operations = operations
    }

    public var availableBTCPayMethods: [BTCPayPaymentMethodRow] {
        operations.btcpayPaymentMethods.filter { storeID.isEmpty || $0.storeID == storeID }
    }

    public var selectedBTCPayMethodIDs: [String] {
        availableBTCPayMethods
            .filter { paymentMethods.contains($0.paymentMethodID) && $0.enabled && $0.syncSupported }
            .map(\.paymentMethodID)
    }

    public var isWorking: Bool { operations.isWorking }
    public var errorMessage: String? { operations.errorMessage }
    public var bip329Preview: BIP329PreviewSummary? { operations.bip329Preview }
    public var artifact: ConnectionOperationArtifact? { operations.artifact }

    public var canSubmit: Bool {
        guard !isWorking else { return false }
        switch mode {
        case .btcpay:
            if operations.btcpayStores.isEmpty {
                return !selectedBackend.isEmpty
                    || (!label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        && !serverURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        && !apiKey.isEmpty)
            }
            return !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && !storeID.isEmpty && !selectedBTCPayMethodIDs.isEmpty
                && (!btcpayExistingWallets || selectedBTCPayMethodIDs.allSatisfy {
                    !(btcpayWalletRoutes[$0] ?? "").isEmpty
                })
        case .bullBitcoin:
            return !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && !sourceFile.isEmpty && !networks.isEmpty
                && (!bullExistingWallets || networks.allSatisfy { !(bullWalletRoutes[$0] ?? "").isEmpty })
        case .samourai:
            let material = [deposit, badbank, premix, postmix, ricochet]
                .contains { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            let descriptorBackends = operations.safeBackendOptions.filter {
                ["esplora", "electrum", "bitcoinrpc"].contains($0.kind.lowercased())
                    && ($0.chain.isEmpty || $0.chain.lowercased() == "bitcoin")
            }
            return !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && material && (1...5_000).contains(gapLimit)
                && (descriptorBackends.isEmpty || !selectedBackend.isEmpty)
        case .bip329:
            return !sourceFile.isEmpty
        }
    }

    /// Returns true only when the modal should close. BTCPay discovery and
    /// BIP329 preview deliberately return false because they are staged flows.
    public func submit() async -> Bool {
        guard canSubmit else { return false }
        switch mode {
        case .btcpay:
            if operations.btcpayStores.isEmpty {
                await discoverBTCPay()
                return false
            }
            await operations.createBTCPay(
                label: label, savedBackend: selectedBackend.nilIfEmpty,
                backendLabel: label, serverURL: serverURL, apiKey: apiKey,
                storeID: storeID, paymentMethodIDs: selectedBTCPayMethodIDs,
                existingWalletRoutes: btcpayExistingWallets ? btcpayWalletRoutes : [:],
                syncProvenance: btcpaySyncProvenance
            )
            return operations.errorMessage == nil
        case .bullBitcoin:
            let routes = bullExistingWallets
                ? networks.sorted().compactMap { network -> (network: String, wallet: String)? in
                    guard let wallet = bullWalletRoutes[network], !wallet.isEmpty else { return nil }
                    return (network, wallet)
                }
                : []
            await operations.createBullBitcoinWallet(
                label: label, sourceFile: sourceFile, networks: networks.sorted(), existingRoutes: routes
            )
            return operations.errorMessage == nil
        case .samourai:
            await operations.importSamourai(
                label: label, backend: selectedBackend.nilIfEmpty, network: network, gapLimit: gapLimit,
                deposit: deposit, badbank: badbank, premix: premix, postmix: postmix, ricochet: ricochet
            )
            return operations.errorMessage == nil
        case .bip329:
            if !operations.canImportBIP329(file: sourceFile) {
                await operations.previewBIP329(file: sourceFile)
                return false
            }
            await operations.importBIP329(file: sourceFile)
            return operations.errorMessage == nil
        }
    }

    public func discoverBTCPay() async {
        await operations.discoverBTCPay(
            savedBackend: selectedBackend.nilIfEmpty, label: label,
            serverURL: serverURL, apiKey: apiKey
        )
        if storeID.isEmpty { storeID = operations.btcpayStores.first?.id ?? "" }
    }

    public func testBTCPay() async {
        guard let method = selectedBTCPayMethodIDs.first else { return }
        await operations.testBTCPay(
            savedBackend: selectedBackend.nilIfEmpty, label: label,
            serverURL: serverURL, apiKey: apiKey, storeID: storeID,
            paymentMethodID: method
        )
    }

    public func exportBIP329() async {
        await operations.exportBIP329(mode: bip329ExportMode, wallet: bip329ExportWallet.nilIfEmpty)
    }

    public func setBTCPayMethod(_ method: String, selected: Bool) {
        if selected { paymentMethods.insert(method) }
        else { paymentMethods.remove(method); btcpayWalletRoutes.removeValue(forKey: method) }
    }

    public func setBullNetwork(_ network: String, selected: Bool) {
        if selected { networks.insert(network) }
        else { networks.remove(network); bullWalletRoutes.removeValue(forKey: network) }
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
