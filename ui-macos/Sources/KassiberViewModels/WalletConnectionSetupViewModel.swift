import Foundation
import Observation
import KassiberDaemonKit

public enum WalletConnectionSetupKind: String, CaseIterable, Equatable, Sendable {
    case descriptor
    case addressList
    case silentPayment
    case liquidDescriptor

    public var walletKind: String {
        switch self {
        case .descriptor, .liquidDescriptor: "descriptor"
        case .addressList: "address"
        case .silentPayment: "silent-payment"
        }
    }
    public var chain: String { self == .liquidDescriptor ? "liquid" : "bitcoin" }
    public var network: String { self == .liquidDescriptor ? "liquidv1" : "main" }
}

public enum SilentPaymentScanMode: String, CaseIterable, Equatable, Sendable {
    case localIndex = "local_index"
    case serverAssisted = "server_assisted"
}

public enum WalletConnectionValidationIssue: String, Equatable, Sendable {
    case labelRequired
    case backendRequired
    case walletMaterialRequired
    case gapLimitInvalid
    case birthdayInvalid
    case addressRequired
    case silentPaymentMaterialRequired
    case silentPaymentStartRequired
    case silentPaymentStartHeightInvalid
    case silentPaymentStartDateInvalid
    case silentPaymentFullHistoryAcknowledgementRequired
    case silentPaymentServerAcknowledgementRequired
}

@MainActor
@Observable
public final class WalletConnectionSetupViewModel {
    public let setupKind: WalletConnectionSetupKind
    public var label = ""
    public var backend = ""
    public var walletMaterial = ""
    public var scriptTypes: Set<String> = []
    public var gapLimit = 20
    public var birthday = ""
    public private(set) var addressInput = ""
    public private(set) var purgedPrivateKeys = 0
    public private(set) var purgedPublicKeys = 0
    public var syncAfterCreate = true
    public var silentPaymentMaterial = ""
    public var silentPaymentScanMode = SilentPaymentScanMode.localIndex
    public var silentPaymentStartHeight = ""
    public var silentPaymentStartDate = ""
    public var silentPaymentFullHistory = false
    public var acknowledgeFullHistory = false
    public var acknowledgeServerPrivacy = false
    public private(set) var backends: [SettingsBackendRow] = []
    public private(set) var preview: [DescriptorPreviewAddress] = []
    public private(set) var detectionMessage = ""
    public private(set) var isWorking = false
    public private(set) var didSave = false
    public private(set) var resultCode: String?
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient

    public init(kind: WalletConnectionSetupKind, daemon: any DaemonClient) {
        setupKind = kind
        self.daemon = daemon
    }

    public var addressSummary: AddressListParseResult { AddressListParser.parse(addressInput) }

    public var availableBackends: [SettingsBackendRow] {
        let supported = Set(["esplora", "liquid-esplora", "electrum", "bitcoinrpc"])
        return backends.filter { backend in
            guard supported.contains(backend.kind.lowercased()) else { return false }
            let backendChain = backend.chain.isEmpty ? "bitcoin" : backend.chain.lowercased()
            guard backendChain == setupKind.chain else { return false }
            if setupKind == .silentPayment {
                guard backend.silentPayments else { return false }
                if silentPaymentScanMode == .serverAssisted && backend.kind.lowercased() == "electrum" { return false }
            }
            return true
        }
    }

    public var validationIssue: WalletConnectionValidationIssue? {
        if label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return .labelRequired }
        if backend.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return .backendRequired }
        switch setupKind {
        case .descriptor, .liquidDescriptor:
            if walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return .walletMaterialRequired }
            if !(1...5_000).contains(gapLimit) { return .gapLimitInvalid }
            if !birthday.isEmpty && !Self.validBirthday(birthday) { return .birthdayInvalid }
        case .addressList:
            if addressSummary.valid.isEmpty { return .addressRequired }
        case .silentPayment:
            if silentPaymentMaterial.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                return .silentPaymentMaterialRequired
            }
            let height = silentPaymentStartHeight.trimmingCharacters(in: .whitespacesAndNewlines)
            let date = silentPaymentStartDate.trimmingCharacters(in: .whitespacesAndNewlines)
            if !height.isEmpty && (Int(height) == nil || Int(height)! < 0 || String(Int(height)!) != height) {
                return .silentPaymentStartHeightInvalid
            }
            if !date.isEmpty && !Self.validISODate(date) { return .silentPaymentStartDateInvalid }
            if !silentPaymentFullHistory && height.isEmpty && date.isEmpty {
                return .silentPaymentStartRequired
            }
            if silentPaymentFullHistory && !acknowledgeFullHistory {
                return .silentPaymentFullHistoryAcknowledgementRequired
            }
            if silentPaymentScanMode == .serverAssisted && !acknowledgeServerPrivacy {
                return .silentPaymentServerAcknowledgementRequired
            }
        }
        return nil
    }

    public var canSubmit: Bool { !isWorking && validationIssue == nil }

    public func configure(backends: [SettingsBackendRow]) {
        self.backends = backends
        if !availableBackends.contains(where: { $0.name == backend }) {
            backend = availableBackends.first(where: \.isDefault)?.name ?? availableBackends.first?.name ?? ""
        }
    }

    public func setAddressInput(_ text: String) {
        let scrubbed = AddressListParser.scrubKeyMaterial(text)
        let removed = scrubbed.privateKeys + scrubbed.publicKeys > 0
        addressInput = removed ? scrubbed.text : text
        if removed {
            purgedPrivateKeys += scrubbed.privateKeys
            purgedPublicKeys += scrubbed.publicKeys
        } else if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            purgedPrivateKeys = 0
            purgedPublicKeys = 0
        }
    }

    public func appendAddressFile(_ text: String) {
        let combined = addressInput.isEmpty ? text : "\(addressInput)\n\(text)"
        setAddressInput(combined)
    }

    public func reportAddressFileReadFailure() { errorMessage = "address_file_read_failed" }

    public func detectAndPreview() async {
        guard setupKind == .descriptor || setupKind == .liquidDescriptor,
              !walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        isWorking = true
        defer { isWorking = false }
        do {
            let resolved = try await resolvedScriptTypes()
            var args = baseArguments()
            args["wallet_material"] = .string(walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines))
            if !resolved.isEmpty { args["script_types"] = .array(resolved.sorted().map(JSONValue.string)) }
            args["count"] = .integer(5)
            let envelope = try await daemon.invoke(.uiWalletsPreviewDescriptor, args: args)
            try BackendSettingsViewModel.requireSuccess(envelope)
            preview = (envelope.data?.objectValue ?? [:]).objects("addresses").compactMap { row in
                guard let address = row.string("address") else { return nil }
                return DescriptorPreviewAddress(
                    branch: row.string("branch") ?? "receive", index: Int(row.int("index") ?? 0),
                    address: address, derivationPath: row.string("derivation_path") ?? ""
                )
            }
            errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    public func create() async {
        guard validationIssue == nil else { return }
        isWorking = true
        didSave = false
        defer { isWorking = false }
        do {
            var args = baseArguments()
            args["label"] = .string(label.trimmingCharacters(in: .whitespacesAndNewlines))
            args["kind"] = .string(setupKind.walletKind)
            switch setupKind {
            case .descriptor, .liquidDescriptor:
                args["wallet_material"] = .string(walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines))
                let resolved = try await resolvedScriptTypes()
                if !resolved.isEmpty { args["script_types"] = .array(resolved.sorted().map(JSONValue.string)) }
                args["gap_limit"] = .integer(Int64(gapLimit))
                if !birthday.isEmpty { args["birthday"] = .string(birthday) }
            case .addressList:
                args["addresses"] = .array(addressSummary.valid.map(JSONValue.string))
            case .silentPayment:
                args["sp_descriptor"] = .string(silentPaymentMaterial.trimmingCharacters(in: .whitespacesAndNewlines))
                args["sp_scan_mode"] = .string(silentPaymentScanMode.rawValue)
                args["sp_full_history"] = .bool(silentPaymentFullHistory)
                args["sp_acknowledge_full_history_warning"] = .bool(acknowledgeFullHistory)
                args["sp_acknowledge_server_warning"] = .bool(acknowledgeServerPrivacy)
                if !silentPaymentFullHistory {
                    if let height = Int(silentPaymentStartHeight) { args["sp_scan_start_height"] = .integer(Int64(height)) }
                    if !silentPaymentStartDate.isEmpty { args["sp_scan_start_date"] = .string(silentPaymentStartDate) }
                }
            }
            try BackendSettingsViewModel.requireSuccess(try await daemon.invoke(.uiWalletsCreate, args: args))
            if syncAfterCreate || setupKind == .silentPayment {
                let records = try await daemon.stream(.uiWalletsSync, args: ["wallet": .string(label.trimmingCharacters(in: .whitespacesAndNewlines))])
                for try await record in records {
                    if let error = record.error { throw error }
                    if record.kind == "auth_required" {
                        throw DaemonClientError.transport("database_passphrase_required")
                    }
                }
            }
            didSave = true
            resultCode = "wallet_created"
            errorMessage = nil
        } catch { errorMessage = BackendSettingsViewModel.message(error) }
    }

    private func baseArguments() -> [String: JSONValue] {
        var args: [String: JSONValue] = ["chain": .string(setupKind.chain), "network": .string(setupKind.network)]
        if !backend.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            args["backend"] = .string(backend.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return args
    }

    private func resolvedScriptTypes() async throws -> Set<String> {
        let material = walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines)
        guard material.hasPrefix("xpub") || material.hasPrefix("tpub") else { return scriptTypes }
        if !scriptTypes.isEmpty { return scriptTypes }
        var args = baseArguments()
        args["wallet_material"] = .string(material)
        let envelope = try await daemon.invoke(.uiWalletsDetectScriptTypes, args: args)
        try BackendSettingsViewModel.requireSuccess(envelope)
        let data = envelope.data?.objectValue ?? [:]
        guard data.bool("probed") == true else {
            throw DaemonClientError.transport(data.string("reason") ?? "script_type_detection_unavailable")
        }
        let active = Set(data["active"]?.arrayValue?.compactMap(\.stringValue) ?? [])
        let resolved = active.isEmpty ? Set(["p2wpkh"]) : active
        scriptTypes = resolved
        detectionMessage = resolved.sorted().joined(separator: ", ")
        return resolved
    }

    private static func validBirthday(_ value: String) -> Bool {
        if let height = Int(value) { return height >= 0 }
        return validISODate(value)
    }

    private static func validISODate(_ value: String) -> Bool {
        let pattern = #"^\d{4}-\d{2}-\d{2}$"#
        guard value.range(of: pattern, options: .regularExpression) != nil else { return false }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyy-MM-dd"
        formatter.isLenient = false
        return formatter.date(from: value) != nil
    }
}
