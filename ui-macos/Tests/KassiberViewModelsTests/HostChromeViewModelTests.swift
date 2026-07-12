import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

private struct FixedNetworkPathSource: NativeNetworkPathSource {
    let state: NativeNetworkPathState

    func currentState() -> NativeNetworkPathState { state }

    func updates() -> AsyncStream<NativeNetworkPathState> {
        AsyncStream { continuation in continuation.finish() }
    }
}

private actor ScriptedEndpointChecker: NativeEndpointHealthChecking {
    let endpoints: [NativeEndpointSummary]
    var outcomes: [String: NativeEndpointProbeOutcome]
    private var probes: [String] = []

    init(
        endpoints: [NativeEndpointSummary],
        outcomes: [String: NativeEndpointProbeOutcome]
    ) {
        self.endpoints = endpoints
        self.outcomes = outcomes
    }

    func loadEndpoints() async throws -> [NativeEndpointSummary] { endpoints }

    func probe(endpointID: String) async throws -> NativeEndpointProbeOutcome {
        probes.append(endpointID)
        guard let outcome = outcomes[endpointID] else {
            throw NativeEndpointHealthError.probeFailed
        }
        return outcome
    }

    func setOutcome(_ outcome: NativeEndpointProbeOutcome, for id: String) {
        outcomes[id] = outcome
    }

    func probeCalls() -> [String] { probes }
}

@Suite("Native host chrome")
@MainActor
struct HostChromeViewModelTests {
    @Test("build identity reads injected bundle keys and bounds the commit")
    func buildIdentity() {
        let identity = NativeBuildIdentity(infoDictionary: [
            "CFBundleDisplayName": " kassiber_native ",
            "CFBundleShortVersionString": "0.22.99",
            "CFBundleVersion": "413",
            "KassiberBuildCommit": "abcdef1234567890",
            "KassiberSigningIdentityStrength": "production",
        ])

        #expect(identity.productName == "kassiber_native")
        #expect(identity.version == "0.22.99")
        #expect(identity.build == "413")
        #expect(identity.shortCommit == "abcdef1")
        #expect(identity.compactLabel == "0.22.99 · abcdef1")
        #expect(identity.signingStrength == "production")

        let dirty = NativeBuildIdentity(infoDictionary: [
            "CFBundleShortVersionString": "0.22.99",
            "KassiberBuildCommit": "abcdef1234567890",
            "KassiberBuildDirty": true,
        ])
        #expect(dirty.commit == "abcdef1234567890-dirty")
        #expect(dirty.shortCommit == "abcdef1-dirty")
        #expect(dirty.compactLabel == "0.22.99 · abcdef1-dirty")

        let fallback = NativeBuildIdentity(infoDictionary: [
            "KassiberBuildCommit": "unknown",
        ])
        #expect(fallback.productName == "kassiber_native")
        #expect(fallback.version == "dev")
        #expect(fallback.commit == nil)
        #expect(fallback.compactLabel == "dev")
    }

    @Test("network chrome disappears after all probes are healthy")
    func actionableNetworkState() async {
        let endpoints = [
            NativeEndpointSummary(
                id: "core", label: "Bitcoin Core", kind: "bitcoinrpc",
                probeKind: .bitcoinRPC, settingsSection: "bitcoin"
            ),
            NativeEndpointSummary(
                id: "node", label: "Lightning", kind: "lnd",
                probeKind: .lightning, settingsSection: "lightning"
            ),
        ]
        let checker = ScriptedEndpointChecker(
            endpoints: endpoints,
            outcomes: [
                "core": NativeEndpointProbeOutcome(ok: true),
                "node": NativeEndpointProbeOutcome(ok: false),
            ]
        )
        let model = GlobalNetworkHealthViewModel(
            pathSource: FixedNetworkPathSource(state: .online),
            checker: checker
        )

        await model.reloadAndProbe()
        await model.startMonitoring()

        #expect(model.pathState == .online)
        #expect(model.rows.first(where: { $0.id == "core" })?.state == .healthy)
        #expect(model.rows.first(where: { $0.id == "node" })?.state == .unhealthy)
        #expect(model.isActionable)
        #expect(model.issueCount == 1)

        await checker.setOutcome(NativeEndpointProbeOutcome(ok: true), for: "node")
        await model.probe("node")
        #expect(model.rows.allSatisfy { $0.state == .healthy })
        #expect(model.isActionable == false)
        #expect(await checker.probeCalls().contains("node"))
        model.stopMonitoring()
    }

    @Test("offline path is actionable without inventing endpoint detail")
    func offlinePath() async {
        let checker = ScriptedEndpointChecker(endpoints: [], outcomes: [:])
        let model = GlobalNetworkHealthViewModel(
            pathSource: FixedNetworkPathSource(state: .offline),
            checker: checker
        )
        await model.reloadAndProbe()
        await model.startMonitoring()
        #expect(model.pathState == .offline)
        #expect(model.isActionable)
        #expect(model.issueCount == 1)
        #expect(model.rows.isEmpty)
        model.stopMonitoring()
    }

    @Test("daemon checker keeps exact endpoints and tokens out of UI summaries")
    func daemonEndpointPrivacy() async throws {
        let secretURL = "ssl://private.example.test:50002"
        let daemon = ScriptedDaemonClient(scripts: [
            .uiBackendsSettingsList: [DaemonRecord(
                kind: "ui.backends.settings.list",
                data: ["backends": [[
                    "name": "private-electrum", "display_name": "Private Electrum",
                    "kind": "electrum", "chain": "bitcoin", "url": .string(secretURL),
                    "has_token": true, "token": "must-never-surface", "insecure": true,
                ]]]
            )],
            .uiBackendsElectrumTest: [DaemonRecord(
                kind: "ui.backends.electrum.test", data: ["ok": true]
            )],
        ])
        let checker = DaemonNativeEndpointHealthChecker(daemon: daemon)

        let endpoints = try await checker.loadEndpoints()
        let endpoint = try #require(endpoints.first)
        #expect(endpoint.label == "Private Electrum")
        #expect(endpoint.probeKind == .electrum)
        #expect(String(describing: endpoints).contains(secretURL) == false)
        #expect(String(describing: endpoints).contains("must-never-surface") == false)

        #expect(try await checker.probe(endpointID: endpoint.id).ok)
        let testCall = await daemon.calls().last { $0.kind == .uiBackendsElectrumTest }
        #expect(testCall?.args?["url"] == .string(secretURL))
        #expect(String(describing: testCall?.args).contains("must-never-surface") == false)
    }

    @Test("settings scene identity advances on profile context changes")
    func settingsIdentity() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiProfilesSnapshot: [DaemonRecord(
                kind: "ui.profiles.snapshot",
                data: [
                    "activeWorkspaceId": "workspace-1",
                    "activeProfileId": "profile-1",
                    "workspaces": [],
                ]
            )],
        ])
        let model = SettingsSceneIdentityViewModel(daemon: daemon)
        await model.load()
        let firstGeneration = model.generation
        #expect(model.activeProfileID == "profile-1")

        await daemon.set([DaemonRecord(
            kind: "ui.profiles.snapshot",
            data: [
                "activeWorkspaceId": "workspace-1",
                "activeProfileId": "profile-2",
                "workspaces": [],
            ]
        )], for: .uiProfilesSnapshot)
        await model.handleHostEvent(DaemonRecord(
            kind: "native.request.activity",
            event: true,
            data: [
                "state": "finished",
                "kind": .string(DaemonKind.uiProfilesSwitch.rawValue),
            ]
        ))

        #expect(model.activeProfileID == "profile-2")
        #expect(model.generation > firstGeneration)
        #expect(model.identityToken.contains("profile-2"))
    }
}
