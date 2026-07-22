import Foundation
import Observation
import KassiberDaemonKit

public enum ConnectionLayer: String, CaseIterable, Identifiable, Sendable {
    case base
    case lightning
    case liquid
    public var id: String { rawValue }
}

public struct BackendEndpointRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let kind: String
    public let chain: String
    public let network: String
    public let hasURL: Bool
    public let isDefault: Bool
    public let usesCredentials: Bool
    public let insecure: Bool

    public var layer: ConnectionLayer {
        if chain == "liquid" || kind.contains("liquid") { return .liquid }
        if ["lnd", "cln", "core-lightning", "nwc"].contains(kind) { return .lightning }
        return .base
    }
}

@MainActor
@Observable
public final class ConnectionSettingsViewModel {
    public var layer: ConnectionLayer = .base
    public private(set) var endpoints: [BackendEndpointRow] = []
    public private(set) var workspace = ""
    public private(set) var profile = ""
    public private(set) var isLoading = false
    public private(set) var errorMessage: String?

    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public var visibleEndpoints: [BackendEndpointRow] { endpoints.filter { $0.layer == layer } }

    public func load() async {
        isLoading = true
        defer { isLoading = false }
        do {
            let envelope = try await daemon.invoke(.uiBackendsList, args: nil)
            if let error = envelope.error { throw error }
            let object = envelope.data?.objectValue ?? [:]
            let summary = object["summary"]?.objectValue ?? [:]
            workspace = summary.string("workspace") ?? ""
            profile = summary.string("profile") ?? ""
            endpoints = object.objects("backends").compactMap { row in
                guard let name = row.string("name") else { return nil }
                return BackendEndpointRow(
                    id: name, name: name, kind: row.string("kind") ?? "",
                    chain: row.string("chain") ?? "", network: row.string("network") ?? "",
                    hasURL: row.bool("has_url") ?? false,
                    isDefault: row.bool("is_default") ?? false,
                    usesCredentials: (row.bool("has_auth_header") ?? false)
                        || (row.bool("has_token") ?? false)
                        || (row.bool("has_cookiefile") ?? false)
                        || (row.bool("has_username") ?? false),
                    insecure: row.bool("insecure") ?? false
                )
            }
            errorMessage = nil
        } catch { errorMessage = String(describing: error) }
    }
}
