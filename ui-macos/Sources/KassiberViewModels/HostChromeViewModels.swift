import Foundation
import Observation
import KassiberDaemonKit

public enum NativeNetworkPathState: String, Equatable, Sendable {
    case unknown
    case online
    case offline
}

/// Foundation-only seam around the macOS Network framework. The app target
/// supplies an NWPathMonitor-backed implementation; tests inject a deterministic
/// source without importing Network or SwiftUI into the view-model package.
public protocol NativeNetworkPathSource: Sendable {
    func currentState() -> NativeNetworkPathState
    func updates() -> AsyncStream<NativeNetworkPathState>
}

public enum NativeEndpointProbeKind: String, Equatable, Sendable {
    case electrum
    case bitcoinRPC
    case lightning
    case btcpay
    case http
    case unsupported
}

/// Deliberately excludes exact URLs, proxy addresses, credential references,
/// and tokens. Only this summary crosses into observable UI state.
public struct NativeEndpointSummary: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String
    public let kind: String
    public let probeKind: NativeEndpointProbeKind
    public let settingsSection: String
    public let enabled: Bool
    public let checkable: Bool

    public init(
        id: String,
        label: String,
        kind: String,
        probeKind: NativeEndpointProbeKind,
        settingsSection: String,
        enabled: Bool = true,
        checkable: Bool = true
    ) {
        self.id = id
        self.label = label
        self.kind = kind
        self.probeKind = probeKind
        self.settingsSection = settingsSection
        self.enabled = enabled
        self.checkable = checkable
    }
}

public struct NativeEndpointProbeOutcome: Equatable, Sendable {
    public let ok: Bool

    public init(ok: Bool) {
        self.ok = ok
    }
}

public protocol NativeEndpointHealthChecking: Sendable {
    func loadEndpoints() async throws -> [NativeEndpointSummary]
    func probe(endpointID: String) async throws -> NativeEndpointProbeOutcome
}

public enum NativeEndpointHealthError: Error, Equatable, Sendable {
    case loadFailed
    case endpointUnavailable
    case probeFailed
}

/// Daemon-backed probe adapter. Exact endpoint material stays actor-private and
/// is sent only back to the existing allowlisted daemon test kinds.
public actor DaemonNativeEndpointHealthChecker: NativeEndpointHealthChecking {
    private struct RawEndpoint: Sendable {
        let name: String
        let url: String
        let trustSelfSigned: Bool
        let summary: NativeEndpointSummary
    }

    private let daemon: any DaemonClient
    private var endpointsByID: [String: RawEndpoint] = [:]

    public init(daemon: any DaemonClient) {
        self.daemon = daemon
    }

    public func loadEndpoints() async throws -> [NativeEndpointSummary] {
        do {
            let envelope = try await daemon.invoke(.uiBackendsSettingsList, args: nil)
            guard envelope.kind != "auth_required", envelope.error == nil else {
                throw NativeEndpointHealthError.loadFailed
            }
            let rows = envelope.data?.objectValue?.objects("backends") ?? []
            var next: [String: RawEndpoint] = [:]
            for row in rows {
                guard let name = row.string("name"), !name.isEmpty else { continue }
                let kind = row.string("kind") ?? ""
                let normalizedKind = kind.lowercased()
                let chain = row.string("chain")?.lowercased() ?? ""
                let url = row.string("url") ?? ""
                let enabled = row.bool("enabled", "on") ?? true
                let probeKind = Self.probeKind(for: normalizedKind, url: url)
                let backendResolved = [
                    NativeEndpointProbeKind.bitcoinRPC,
                    .lightning,
                    .btcpay,
                ].contains(probeKind)
                let checkable = enabled && probeKind != .unsupported
                    && (backendResolved || !url.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                let summary = NativeEndpointSummary(
                    id: name,
                    label: row.string("display_name", "label") ?? name,
                    kind: kind,
                    probeKind: probeKind,
                    settingsSection: Self.settingsSection(kind: normalizedKind, chain: chain),
                    enabled: enabled,
                    checkable: checkable
                )
                next[name] = RawEndpoint(
                    name: name,
                    url: url,
                    trustSelfSigned: row.bool("insecure", "trust_self_signed") ?? false,
                    summary: summary
                )
            }
            endpointsByID = next
            return next.values.map(\.summary).sorted {
                $0.label.localizedCaseInsensitiveCompare($1.label) == .orderedAscending
            }
        } catch is NativeEndpointHealthError {
            throw NativeEndpointHealthError.loadFailed
        } catch {
            // Never promote a transport error that may contain an endpoint.
            throw NativeEndpointHealthError.loadFailed
        }
    }

    public func probe(endpointID: String) async throws -> NativeEndpointProbeOutcome {
        guard let endpoint = endpointsByID[endpointID], endpoint.summary.checkable else {
            throw NativeEndpointHealthError.endpointUnavailable
        }
        let request: (DaemonKind, [String: JSONValue])
        switch endpoint.summary.probeKind {
        case .bitcoinRPC:
            request = (
                .uiBackendsBitcoinrpcTest,
                ["backend": .string(endpoint.name), "timeout": .integer(5)]
            )
        case .lightning:
            request = (
                .uiBackendsLightningTest,
                ["backend": .string(endpoint.name), "timeout": .integer(5)]
            )
        case .btcpay:
            request = (
                .uiConnectionsBtcpayDiscover,
                ["backend": .string(endpoint.name), "timeout": .integer(5)]
            )
        case .electrum:
            request = (
                .uiBackendsElectrumTest,
                [
                    "url": .string(endpoint.url),
                    "trust_self_signed": .bool(endpoint.trustSelfSigned),
                    "timeout": .integer(5),
                ]
            )
        case .http:
            request = (
                .uiBackendsHttpTest,
                ["url": .string(endpoint.url), "timeout": .integer(5)]
            )
        case .unsupported:
            throw NativeEndpointHealthError.endpointUnavailable
        }

        do {
            let envelope = try await daemon.invoke(request.0, args: request.1)
            guard envelope.kind != "auth_required", envelope.error == nil else {
                throw NativeEndpointHealthError.probeFailed
            }
            let data = envelope.data?.objectValue ?? [:]
            let ok: Bool
            switch endpoint.summary.probeKind {
            case .bitcoinRPC:
                ok = data.bool("reachable", "ok") ?? false
            case .btcpay:
                ok = data["stores"]?.arrayValue != nil || data.bool("ok", "reachable") == true
            default:
                ok = data.bool("ok", "reachable") ?? false
            }
            return NativeEndpointProbeOutcome(ok: ok)
        } catch is NativeEndpointHealthError {
            throw NativeEndpointHealthError.probeFailed
        } catch {
            // Keep exact URLs and daemon logs out of observable state.
            throw NativeEndpointHealthError.probeFailed
        }
    }

    private static func probeKind(for kind: String, url: String) -> NativeEndpointProbeKind {
        switch kind {
        case "bitcoinrpc":
            return .bitcoinRPC
        case "electrum", "fulcrum":
            return .electrum
        case "lnd", "cln", "coreln", "core-lightning", "nwc":
            return .lightning
        case "btcpay":
            return .btcpay
        case "esplora", "mempool", "liquid", "elements":
            return .http
        default:
            let scheme = URL(string: url)?.scheme?.lowercased()
            return scheme == "http" || scheme == "https" ? .http : .unsupported
        }
    }

    private static func settingsSection(kind: String, chain: String) -> String {
        if ["lnd", "cln", "coreln", "core-lightning", "nwc"].contains(kind) {
            return "lightning"
        }
        if chain == "liquid" || ["liquid", "elements"].contains(kind) {
            return "liquid"
        }
        return "bitcoin"
    }
}

public enum NativeEndpointHealthState: String, Equatable, Sendable {
    case unchecked
    case checking
    case healthy
    case unhealthy
}

public struct NativeEndpointHealthRow: Identifiable, Equatable, Sendable {
    public let endpoint: NativeEndpointSummary
    public var state: NativeEndpointHealthState

    public var id: String { endpoint.id }

    public init(endpoint: NativeEndpointSummary, state: NativeEndpointHealthState = .unchecked) {
        self.endpoint = endpoint
        self.state = state
    }
}

@MainActor
@Observable
public final class GlobalNetworkHealthViewModel {
    public private(set) var pathState: NativeNetworkPathState = .unknown
    public private(set) var rows: [NativeEndpointHealthRow] = []
    public private(set) var isLoading = false
    public private(set) var isChecking = false
    public private(set) var loadFailed = false

    public var actionableRows: [NativeEndpointHealthRow] {
        rows.filter {
            $0.endpoint.enabled && $0.endpoint.checkable && $0.state != .healthy
        }
    }

    /// The chrome stays absent when every tested endpoint is healthy. This is
    /// an exception surface, not a standing green status indicator.
    public var isActionable: Bool {
        pathState == .offline || loadFailed || isLoading || isChecking || !actionableRows.isEmpty
    }

    public var issueCount: Int {
        let endpointIssues = rows.filter { $0.state == .unhealthy }.count
        return max(pathState == .offline || loadFailed ? 1 : 0, endpointIssues)
    }

    public var canProbe: Bool {
        pathState == .online && !isLoading && !isChecking
            && rows.contains { $0.endpoint.enabled && $0.endpoint.checkable }
    }

    private let pathSource: any NativeNetworkPathSource
    private let checker: any NativeEndpointHealthChecking
    private var pathTask: Task<Void, Never>?
    private var reloadGeneration = 0

    public init(
        pathSource: any NativeNetworkPathSource,
        checker: any NativeEndpointHealthChecking
    ) {
        self.pathSource = pathSource
        self.checker = checker
    }

    public func startMonitoring() async {
        if pathTask == nil {
            let stream = pathSource.updates()
            pathTask = Task { @MainActor [weak self] in
                for await state in stream {
                    guard !Task.isCancelled else { return }
                    await self?.receivePathState(state)
                }
            }
        }
        await receivePathState(pathSource.currentState())
    }

    public func stopMonitoring() {
        pathTask?.cancel()
        pathTask = nil
    }

    public func reloadAndProbe() async {
        reloadGeneration &+= 1
        let generation = reloadGeneration
        isLoading = true
        do {
            let endpoints = try await checker.loadEndpoints()
            guard generation == reloadGeneration else { return }
            rows = endpoints.map { NativeEndpointHealthRow(endpoint: $0) }
            loadFailed = false
            isLoading = false
            if pathState == .online { await probeAll() }
        } catch {
            guard generation == reloadGeneration else { return }
            rows = []
            loadFailed = true
            isLoading = false
        }
    }

    public func receivePathState(_ state: NativeNetworkPathState) async {
        let previous = pathState
        pathState = state
        if state == .online, previous != .online, !rows.isEmpty {
            await probeAll()
        }
    }

    public func probeAll() async {
        guard canProbe else { return }
        isChecking = true
        let ids = rows.filter { $0.endpoint.enabled && $0.endpoint.checkable }.map(\.id)
        for id in ids {
            await probeWithoutChangingGlobalState(id)
        }
        isChecking = false
    }

    public func probe(_ endpointID: String) async {
        guard canProbe, rows.contains(where: { $0.id == endpointID }) else { return }
        isChecking = true
        await probeWithoutChangingGlobalState(endpointID)
        isChecking = false
    }

    private func probeWithoutChangingGlobalState(_ endpointID: String) async {
        guard let index = rows.firstIndex(where: { $0.id == endpointID }) else { return }
        rows[index].state = .checking
        do {
            let outcome = try await checker.probe(endpointID: endpointID)
            guard let current = rows.firstIndex(where: { $0.id == endpointID }) else { return }
            rows[current].state = outcome.ok ? .healthy : .unhealthy
        } catch {
            guard let current = rows.firstIndex(where: { $0.id == endpointID }) else { return }
            rows[current].state = .unhealthy
        }
    }
}

public struct NativeBuildIdentity: Equatable, Sendable {
    public let productName: String
    public let version: String
    public let build: String?
    public let commit: String?
    public let signingStrength: String?

    public var shortCommit: String? {
        commit.map { value in
            let dirty = value.hasSuffix("-dirty")
            let base = dirty ? String(value.dropLast("-dirty".count)) : value
            return String(base.prefix(7)) + (dirty ? "-dirty" : "")
        }
    }

    public var compactLabel: String {
        [version, shortCommit].compactMap { value in
            guard let value, !value.isEmpty else { return nil }
            return value
        }.joined(separator: " · ")
    }

    public init(infoDictionary: [String: Any]) {
        func value(_ key: String) -> String? {
            guard let raw = infoDictionary[key] as? String else { return nil }
            let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? nil : trimmed
        }
        productName = value("CFBundleDisplayName") ?? value("CFBundleName") ?? "kassiber_native"
        version = value("CFBundleShortVersionString") ?? "dev"
        build = value("CFBundleVersion")
        let candidateCommit = value("KassiberBuildCommit")
        if let candidateCommit, candidateCommit.lowercased() != "unknown" {
            let dirty = infoDictionary["KassiberBuildDirty"] as? Bool ?? false
            commit = dirty && !candidateCommit.hasSuffix("-dirty")
                ? "\(candidateCommit)-dirty" : candidateCommit
        } else {
            commit = nil
        }
        signingStrength = value("KassiberSigningIdentityStrength")
    }
}

@MainActor
@Observable
public final class SettingsSceneIdentityViewModel {
    public private(set) var activeWorkspaceID = ""
    public private(set) var activeProfileID = ""
    public private(set) var generation = 0

    public var identityToken: String {
        "\(activeWorkspaceID)|\(activeProfileID)|\(generation)"
    }

    private let daemon: any DaemonClient

    public init(daemon: any DaemonClient) {
        self.daemon = daemon
    }

    public func load() async {
        do {
            let envelope = try await daemon.invoke(.uiProfilesSnapshot, args: nil)
            guard envelope.kind != "auth_required", envelope.error == nil,
                  let data = envelope.data?.objectValue else { return }
            let workspace = data.string("activeWorkspaceId", "active_workspace_id") ?? ""
            let profile = data.string("activeProfileId", "active_profile_id")
                ?? Self.activeProfile(in: data)
                ?? ""
            if workspace != activeWorkspaceID || profile != activeProfileID {
                activeWorkspaceID = workspace
                activeProfileID = profile
                generation &+= 1
            }
        } catch {
            // Authentication state remains owned by AppShellViewModel. A
            // settings identity refresh never promotes transport details.
        }
    }

    public func monitorContextChanges() async {
        let stream = await daemon.events()
        for await event in stream {
            guard !Task.isCancelled else { return }
            await handleHostEvent(event)
        }
    }

    public func handleHostEvent(_ event: DaemonRecord) async {
        guard event.kind == "native.request.activity",
              let data = event.data?.objectValue,
              data.string("state") == "finished",
              let rawKind = data.string("kind"),
              Self.identityInvalidatingKinds.contains(rawKind) else { return }
        let priorGeneration = generation
        await load()
        if generation == priorGeneration { generation &+= 1 }
    }

    private static func activeProfile(in data: [String: JSONValue]) -> String? {
        for workspace in data.objects("workspaces") {
            if let profile = workspace.objects("profiles").first(where: {
                $0.bool("active", "selected") == true
            }) {
                return profile.string("id")
            }
        }
        return nil
    }

    private static let identityInvalidatingKinds: Set<String> = [
        DaemonKind.uiProfilesSwitch.rawValue,
        DaemonKind.uiProfilesCreate.rawValue,
        DaemonKind.uiProfilesRename.rawValue,
        DaemonKind.uiProfilesUpdate.rawValue,
        DaemonKind.uiProfilesResetData.rawValue,
        DaemonKind.uiWorkspaceCreate.rawValue,
        DaemonKind.uiWorkspaceRename.rawValue,
        DaemonKind.uiWorkspaceDelete.rawValue,
        DaemonKind.uiProjectsCreate.rawValue,
        DaemonKind.uiProjectsSelect.rawValue,
    ]
}
