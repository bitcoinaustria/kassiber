import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Native parity view models")
@MainActor
struct NativeParityViewModelTests {
    @Test("Activity maps stale provenance and can revert")
    func activity() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiActivityHistory: [DaemonRecord(kind: "ui.activity.history", data: ["events": [[
                "id": "e1", "transaction_id": "tx1", "transaction_external_id": "abc", "wallet_label": "Cold",
                "changed_at": "2026-07-10T12:00:00Z", "source": "gui", "summary": "Note updated",
                "families": ["metadata"], "fields": [["field": "note", "label": "Note", "before_label": "", "after_label": "Reviewed"]],
                "report_anchor": ["stale_for_reports": true],
            ]], "next_cursor": "page-2"])],
            .uiActivityStale: [DaemonRecord(kind: "ui.activity.stale", data: ["edit_count": 1])],
        ])
        let model = ActivityViewModel(daemon: daemon)
        await model.load()
        #expect(model.events.first?.summary == "Note updated")
        #expect(model.events.first?.source == "gui")
        #expect(model.events.first?.stale == true)
        #expect(model.staleCount == 1)
        await model.load(reset: false)
        let pageCall = await daemon.calls().last { $0.kind == .uiActivityHistory }
        #expect(pageCall?.args?["cursor"] == .string("page-2"))
    }

    @Test("Privacy mirror parses every typed parity section and derives ranked findings")
    func privacy() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiReportsPrivacyMirror: [DaemonRecord(kind: "ui.reports.privacy_mirror", data: [
                "local_only": true, "read_only": true, "advisory_only": true,
                "summary": [
                    "evidence_level": "derived",
                    "privacy_score": [
                        "value": 78, "base": 100, "coverage_ratio": .number(0.75),
                        "factors": [["key": "wallet_linkage", "linked": 1, "total": 2, "weight": .number(0.55), "points": -22]],
                    ],
                    "worst_risk": ["kind": "common_input", "severity": "warning", "answer": "Address reuse", "evidence_level": "exact"],
                    "linkage_score": 2, "linkable_cluster_count": 1, "adversary_view_count": 1,
                    "wallet_count": 1, "transaction_tell_count": 2, "utxo_count": 1, "unknown_count": 2,
                ],
                "adversary_cards": [[
                    "tier": "passive_chain_watcher", "label": "Watcher", "evidence_level": "derived",
                    "summary": [
                        "exposed_cluster_count": 1, "wallet_count": 1, "observer_entity_count": 1,
                        "unknown_coverage": ["status": "partial_model_coverage", "node_count": 1],
                    ],
                    "model_assumptions": [["code": "bitcoin_graph_facts_only", "statement": "Local facts", "evidence_level": "derived"]],
                ]],
                "wallet_view": [[
                    "wallet_id": "w1", "coin_count": 2, "amount_msat": 5000,
                    "linkage_edge_count": 1, "cluster_count": 1,
                    "unknown_role_coin_count": 1, "evidence_level": "exact",
                ]],
                "transaction_view": [[
                    "txid": "tx1", "tell_count": 2,
                    "tell_kinds": ["sender_common_input", "fee_fingerprint"],
                    "wallet_penalty_count": 1, "evidence_level": "exact",
                ]],
                "utxo_view": [[
                    "coin_id": "coin1", "wallet_id": "w1", "amount_msat": 5000,
                    "branch_role": "receive", "source_proximity": "unknown_provenance",
                    "evidence_level": "unknown",
                ]],
                "timeline": [[
                    "id": "edge1", "kind": "common_input", "category": "linkage",
                    "txid": "tx1", "detail": "common_input", "new_linkage": true,
                    "evidence_level": "exact",
                ]],
                "coverage": [
                    "evidence_level": "unknown", "source_proximity_known_coin_count": 1,
                    "source_proximity_unknown_coin_count": 1, "unknown_coverage_count": 1,
                    "degraded": true,
                ],
                "unknowns": [[
                    "source": "linkage_graph", "code": "source_proximity_coverage_gaps",
                    "title": "Origins incomplete", "evidence_level": "unknown",
                ]],
                "evidence_drilldowns": [[
                    "section": "edges", "id": "edge1", "kind": "common_input",
                    "evidence_level": "exact", "evidence": ["new_cluster_merges": 1],
                ]],
            ])],
        ])
        let model = PrivacyMirrorViewModel(daemon: daemon)
        await model.load()
        #expect(model.score == 78)
        #expect(model.scoreBase == 100)
        #expect(model.scoreIsGrounded)
        #expect(model.scoreFactors.first?.linked == 1)
        #expect(model.coverage == 0.75)
        #expect(model.coverageSummary.degraded)
        #expect(!model.guardrailsNominal)
        #expect(model.severityCensus.warning == 1)
        #expect(model.severityCensus.info == 2)
        #expect(model.findings.map(\.kind) == ["sender_common_input", "source_proximity_coverage_gaps", "coverage_degraded"])
        #expect(model.findings[1].routesToSourceFunds)
        #expect(model.findings[2].routesToSourceFunds)
        #expect(model.wallets.first?.id == "w1")
        #expect(model.wallets.first?.detail == .wallet(coinCount: 2, linkCount: 1))
        #expect(model.wallets.first?.amountMSat == 5000)
        #expect(model.transactions.first?.walletPenaltyCount == 1)
        #expect(model.utxos.first?.walletID == "w1")
        #expect(model.adversaryCards.first?.assumptions.first?.code == "bitcoin_graph_facts_only")
        #expect(model.timeline.first?.newLinkage == true)
        #expect(model.evidenceDrilldowns.first?.facts.first?.value == "1")
        #expect(model.linkageScore == 2)
        #expect(PrivacyMirrorViewModel.heuristics.count == 34)
        #expect(PrivacyMirrorViewModel.heuristics.count { $0.status == "computed" } == 14)
    }

    @Test("Privacy mirror fallback score is deterministic and nominal guardrails stay separate")
    func privacyFallback() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiReportsPrivacyMirror: [DaemonRecord(kind: "ui.reports.privacy_mirror", data: [
                "local_only": true, "read_only": true, "advisory_only": true,
                "summary": ["worst_risk": ["kind": "bounded_local_model", "severity": "info"]],
                "transaction_view": [["txid": "tx-clean", "tell_count": 0, "wallet_penalty_count": 0]],
                "coverage": ["degraded": false], "unknowns": [],
            ])],
        ])
        let model = PrivacyMirrorViewModel(daemon: daemon)
        await model.load()
        #expect(!model.scoreIsGrounded)
        #expect(model.scoreBase == 70)
        #expect(model.score == 67)
        #expect(model.grade == "C")
        #expect(model.scoreFactors == [
            PrivacyScoreFactorRow(key: "info", linked: nil, leaking: nil, total: 1, weight: nil, points: -3),
        ])
        #expect(model.guardrailsNominal)
        #expect(model.findings.first?.transactionID == "tx-clean")
    }

    @Test("Egress and logs parse only bounded RAM snapshots")
    func ramSnapshots() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiEgressSnapshot: [DaemonRecord(kind: "ui.egress.snapshot", data: [
                "summary": ["unexpected": 1, "update": 0], "allowlist_complete": true,
                "db_header": ["classification": "encrypted"],
                "records": [["id": 1, "ts": "2026-07-10T12:00:00Z", "subsystem": "pricing", "host": "example.com", "port": 443, "operation": "http.request", "bytes_out": 10, "allowlist_status": "unexpected"]],
            ])],
            .uiLogsSnapshot: [DaemonRecord(kind: "ui.logs.snapshot", data: [
                "buffer_bytes": 100, "max_bytes": 1000, "records": [["id": 1, "ts": "2026-07-10T12:00:00Z", "level": "warning", "module": "daemon", "file": "daemon.py", "line": 2, "msg": "Review", "fields": [:]]],
            ])],
        ])
        let egress = EgressViewModel(daemon: daemon); await egress.load(); egress.actionableOnly = true
        let logs = LogsViewModel(daemon: daemon); await logs.load(); logs.level = "warning"
        #expect(egress.visibleRecords.count == 1)
        #expect(logs.visibleRecords.first?.message == "Review")
    }

    @Test("Books use explicit profile switch")
    func books() async {
        let snapshot: JSONValue = ["activeWorkspaceId": "ws", "activeProfileId": "p1", "workspaces": [["id": "ws", "name": "Set", "profiles": [["id": "p1", "name": "Book", "active": true]]]]]
        let daemon = ScriptedDaemonClient(scripts: [
            .uiProfilesSnapshot: [DaemonRecord(kind: "ui.profiles.snapshot", data: snapshot)],
            .uiProfilesSwitch: [DaemonRecord(kind: "ui.profiles.switch", data: ["activeProfileId": "p1"])],
            .uiWorkspaceRename: [DaemonRecord(kind: "ui.workspace.rename", data: [:])],
        ])
        let model = BooksViewModel(daemon: daemon)
        await model.load()
        await model.switchBook("p1")
        await model.renameWorkspace("ws", label: "Renamed Set")
        #expect(model.workspaces.first?.books.first?.name == "Book")
        #expect(await daemon.calls().contains { $0.kind == .uiProfilesSwitch })
        #expect(await daemon.calls().contains {
            $0.kind == .uiWorkspaceRename
                && $0.args?["workspace_id"] == .string("ws")
                && $0.args?["label"] == .string("Renamed Set")
        })
    }

    @Test("Books parse daemon snake-case identities for the global switcher")
    func booksSnakeCaseIdentity() async {
        let snapshot: JSONValue = [
            "active_workspace_id": "ws", "active_profile_id": "p2",
            "workspaces": [[
                "id": "ws", "label": "Company",
                "profiles": [[
                    "id": "p2", "label": "Austria", "fiat_currency": "EUR",
                    "tax_country": "AT", "gains_algorithm": "fifo",
                ]],
            ]],
        ]
        let daemon = ScriptedDaemonClient(scripts: [
            .uiProfilesSnapshot: [DaemonRecord(kind: "ui.profiles.snapshot", data: snapshot)],
        ])
        let model = BooksViewModel(daemon: daemon)
        await model.load()
        #expect(model.activeProfileID == "p2")
        #expect(model.activeBook?.name == "Austria")
        #expect(model.activeBook?.fiatCurrency == "EUR")
    }

    @Test("Birds-eye book switching uses the daemon profile_id contract")
    func birdsEyeSwitchArgument() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiWorkspaceOverviewSnapshot: [DaemonRecord(
                kind: "ui.workspace.overview.snapshot",
                data: ["workspace": ["label": "Set"], "books": []]
            )],
            .uiProfilesSwitch: [DaemonRecord(kind: "ui.profiles.switch", data: [:])],
        ])
        let model = BirdsEyeViewModel(daemon: daemon)
        await model.load(workspaceID: "ws")
        await model.switchBook("profile-2")
        let call = await daemon.calls().last { $0.kind == .uiProfilesSwitch }
        #expect(call?.args?["profile_id"] == .string("profile-2"))
        #expect(call?.args?["profile"] == nil)
    }

    @Test("Stored chat sessions resume and branch without mutating the original")
    func chatSessions() async {
        let daemon = ScriptedDaemonClient(scripts: [
            .uiChatSessionsList: [DaemonRecord(kind: "ui.chat.sessions.list", data: ["history_enabled": true, "sessions": [["id": "s1", "title": "Tax", "updated_at": "2026-07-10T12:00:00Z", "message_count": 2]]])],
            .uiChatSessionsGet: [DaemonRecord(kind: "ui.chat.sessions.get", data: ["id": "s1", "messages": [["role": "user", "content": "Question"], ["role": "assistant", "content": "Answer"]]])],
        ])
        let model = AIChatViewModel(daemon: daemon)
        model.incognito = true
        await model.loadSessions(); await model.resume(model.sessions[0])
        #expect(model.sessionID == "s1")
        #expect(!model.incognito)
        model.branch(from: model.messages[0].id)
        #expect(model.sessionID == nil)
        #expect(model.messages.count == 1)
    }
}
