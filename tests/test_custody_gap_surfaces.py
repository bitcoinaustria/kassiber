from __future__ import annotations

import queue
import sqlite3
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

from kassiber.ai.tools import get_tool
from kassiber.ai.prompt import build_openai_tools
from kassiber.core.ui_snapshot import build_custody_lineage_snapshot
from kassiber.daemon import (
    AiToolRuntime,
    AI_TOOL_ONCE_ONLY_CONSENT,
    ParsedAiToolCall,
    SUPPORTED_KINDS,
    _execute_read_only_ai_tool,
    _execute_mutating_ai_tool,
    handle_request,
    _ui_custody_coverage_payload_from_conn,
    _ui_custody_gap_payload_from_conn,
)
from kassiber.errors import AppError


class CustodyGapSurfaceTest(unittest.TestCase):
    def test_read_kinds_are_supported_and_ai_read_only(self):
        for kind in (
            "ui.custody.coverage.snapshot",
            "ui.custody.lineage.snapshot",
            "ui.custody.gaps.list",
            "ui.custody.gaps.review_context",
            "ui.custody.gaps.history",
            "ui.custody.review.plan",
        ):
            self.assertIn(kind, SUPPORTED_KINDS)
            tool = get_tool(kind)
            self.assertIsNotNone(tool)
            self.assertEqual(tool.kind_class, "read_only")
            self.assertEqual(tool.daemon_kind, kind)

        for kind in ("ui.custody.review.apply",):
            self.assertIn(kind, SUPPORTED_KINDS)
            self.assertEqual(get_tool(kind).kind_class, "mutating")
            self.assertIn(kind, AI_TOOL_ONCE_ONLY_CONSENT)

        review = get_tool("ui.custody.gaps.review_context")
        self.assertEqual(review.parameters["required"], ["gap_id"])
        self.assertFalse(review.parameters["additionalProperties"])
        gap_list = get_tool("ui.custody.gaps.list")
        self.assertEqual(gap_list.parameters["properties"]["cursor"]["type"], "string")
        residual = get_tool("ui.custody.review.apply")
        self.assertEqual(
            residual.parameters["properties"]["classification"]["enum"],
            [
                "external_payment",
                "external_disposal",
                "external_gift",
                "external_loss",
                "retained_custody",
                "suspense_continuation",
            ],
        )
        coverage = get_tool("ui.custody.coverage.snapshot")
        self.assertEqual(coverage.parameters["properties"], {})
        self.assertFalse(coverage.parameters["additionalProperties"])
        lineage = get_tool("ui.custody.lineage.snapshot")
        self.assertEqual(lineage.parameters["properties"]["limit"]["maximum"], 500)
        self.assertEqual(
            lineage.parameters["properties"]["cursor"]["type"], "string"
        )
        self.assertEqual(
            lineage.parameters["properties"]["transaction_id"]["type"],
            "string",
        )
        self.assertFalse(lineage.parameters["additionalProperties"])

        advertised = {
            item["function"]["name"]
            for item in build_openai_tools(
                [
                    {
                        "role": "user",
                        "content": "Did this wallet roll pass through a missing wallet?",
                    }
                ]
            )
        }
        self.assertIn("ui_custody_gaps_list", advertised)
        self.assertIn("ui_custody_gaps_review_context", advertised)
        self.assertIn("ui_custody_gaps_history", advertised)
        self.assertIn("ui_custody_coverage_snapshot", advertised)
        self.assertIn("ui_custody_lineage_snapshot", advertised)

    def test_lineage_snapshot_keeps_custody_and_basis_states_separate(self):
        conn = sqlite3.connect(":memory:")
        result = {
            "records": [
                {
                    "source_transaction_id": "out-1",
                    "target_transaction_id": "in-1",
                    "source_wallet_id": "wallet-old",
                    "source_wallet_label": "Old vault",
                    "target_wallet_id": "wallet-new",
                    "target_wallet_label": "New vault",
                    "source_asset": "BTC",
                    "target_asset": "BTC",
                    "amount_msat": 10**20,
                    "custody_state": "internal_verified",
                    "basis_state": "blocked_by_prior_custody_basis",
                    "basis_barrier_at": "2019-01-01T00:00:00Z",
                    "reason": "recorded_exact_pair",
                    "atomic_group_id": None,
                    "component_id": None,
                    "occurred_at": "2020-01-01T00:00:00Z",
                    "target_occurred_at": "2020-01-02T00:00:00Z",
                    "created_at": "2026-01-01T00:00:00Z",
                }
            ],
            "count": 1,
            "returned": 1,
            "truncated": False,
            "next_cursor": "opaque-next-page",
            "observation_commitments_included": False,
            "replicated": False,
        }
        context = {
            "workspace_id": "workspace",
            "workspace_label": "Treasury",
            "profile_id": "profile",
            "profile_label": "Company",
        }
        with (
            patch(
                "kassiber.core.ui_snapshot._active_context_and_profile",
                return_value=(context, {"id": "profile"}),
            ),
            patch(
                "kassiber.core.ui_snapshot.core_custody_quantity_store.custody_decision_rows",
                return_value=result,
            ) as read_rows,
        ):
            payload = build_custody_lineage_snapshot(
                conn,
                {
                    "cursor": "opaque-current-page",
                    "limit": 25,
                    "transaction_id": " in-1 ",
                },
            )

        read_rows.assert_called_once_with(
            conn,
            "profile",
            limit=25,
            transaction_ids=["in-1"],
            cursor="opaque-current-page",
        )
        self.assertEqual(payload["items"][0]["amount_msat"], str(10**20))
        self.assertEqual(payload["items"][0]["out_transaction_id"], "out-1")
        self.assertEqual(payload["items"][0]["from_wallet_label"], "Old vault")
        self.assertEqual(payload["items"][0]["to_wallet_label"], "New vault")
        self.assertEqual(
            payload["items"][0]["evidence_reason"], "recorded_exact_pair"
        )
        self.assertEqual(
            payload["items"][0]["custody_state"], "internal_verified"
        )
        self.assertEqual(
            payload["items"][0]["basis_state"],
            "blocked_by_prior_custody_basis",
        )
        self.assertEqual(
            payload["summary"]["internal_verified"],
            1,
        )
        self.assertEqual(
            payload["summary"]["basis_blocked"],
            1,
        )
        self.assertIn(
            "Custody finality is separate",
            payload["summary"]["qualification"],
        )
        self.assertEqual(payload["next_cursor"], "opaque-next-page")
        self.assertFalse(payload["observation_commitments_included"])

        with self.assertRaises(AppError) as raised:
            build_custody_lineage_snapshot(conn, {"transaction_id": " "})
        self.assertEqual(raised.exception.code, "validation")

        with self.assertRaises(AppError) as raised:
            build_custody_lineage_snapshot(conn, {"descriptor": "private"})
        self.assertEqual(raised.exception.code, "validation")

        with self.assertRaises(AppError) as raised:
            build_custody_lineage_snapshot(conn, {"cursor": ""})
        self.assertEqual(raised.exception.code, "validation")

    def test_lineage_daemon_dispatch_forwards_bounded_args(self):
        conn = sqlite3.connect(":memory:")
        expected = {"items": [], "summary": {"total_count": 0}}
        with patch(
            "kassiber.daemon.build_custody_lineage_snapshot",
            return_value=expected,
        ) as snapshot:
            envelope, shutdown = handle_request(
                SimpleNamespace(conn=conn),
                {
                    "kind": "ui.custody.lineage.snapshot",
                    "request_id": "lineage-1",
                    "args": {"limit": 20},
                },
                Mock(),
            )

        snapshot.assert_called_once_with(conn, {"limit": 20})
        self.assertFalse(shutdown)
        self.assertEqual(envelope["kind"], "ui.custody.lineage.snapshot")
        self.assertEqual(envelope["request_id"], "lineage-1")
        self.assertEqual(envelope["data"], expected)

    def test_history_and_residual_dispatch_are_bounded_and_exact(self):
        conn = sqlite3.connect(":memory:")
        with (
            patch(
                "kassiber.daemon.resolve_scope",
                return_value=({"id": "workspace"}, {"id": "profile"}),
            ),
            patch(
                "kassiber.daemon.core_custody_gap_reviews.list_review_history",
                return_value={
                    "gap_id": "gap:1",
                    "count": 1,
                    "history": [{"revision": 1, "residual_msat": 10**20}],
                },
            ) as history,
        ):
            payload = _ui_custody_gap_payload_from_conn(
                conn,
                "ui.custody.gaps.history",
                {"gap_id": "gap:1", "limit": 20},
            )
        history.assert_called_once_with(conn, "profile", "gap:1", limit=20)
        self.assertEqual(payload["history"][0]["residual_msat"], str(10**20))

        with (
            patch(
                "kassiber.daemon.resolve_scope",
                return_value=({"id": "workspace"}, {"id": "profile"}),
            ),
            patch(
                "kassiber.daemon.core_custody_gap_reviews.apply_review",
                return_value={
                    "gap_id": "gap:1",
                    "review_id": "review",
                    "component_id": "component",
                    "residual_msat": 10**20,
                },
            ) as classify,
        ):
            payload = _ui_custody_gap_payload_from_conn(
                conn,
                "ui.custody.review.apply",
                {
                    "action": "classify_residual",
                    "gap_id": "gap:1",
                    "classification": "external_gift",
                    "expected_fingerprint": "a" * 64,
                    "reason": "reviewed evidence",
                },
                authored_source="ai_tool",
                commit=False,
            )
        classify.assert_called_once_with(
            conn,
            workspace_id="workspace",
            profile_id="profile",
            action="classify_residual",
            candidate=None,
            gap_id="gap:1",
            classification="external_gift",
            expected_fingerprint="a" * 64,
            reason="reviewed evidence",
            authored_source="ai_tool",
            commit=False,
        )
        self.assertEqual(payload["residual_msat"], str(10**20))

        with self.assertRaises(AppError) as raised:
            _ui_custody_gap_payload_from_conn(
                conn,
                "ui.custody.review.plan",
                {
                    "action": "classify_residual",
                    "gap_id": "gap:1",
                    "classification": "external_gift",
                    "component": {"raw": "not accepted"},
                },
            )
        self.assertEqual(raised.exception.code, "validation")

    def test_daemon_coverage_surface_preserves_unknown_ownership_universe(self):
        conn = sqlite3.connect(":memory:")
        expected = {
            "scope": "imported_policy_technical_coverage",
            "ownership_universe_known": False,
            "coverage_can_clear_custody_gaps": False,
            "wallets": [],
        }
        with (
            patch(
                "kassiber.daemon.resolve_scope",
                return_value=({"id": "workspace"}, {"id": "profile"}),
            ),
            patch(
                "kassiber.daemon.core_ownership_policy_epochs.technical_coverage_snapshot",
                return_value=expected,
            ) as snapshot,
        ):
            payload = _ui_custody_coverage_payload_from_conn(conn, {})

        snapshot.assert_called_once_with(conn, "profile")
        self.assertIs(payload, expected)
        self.assertFalse(payload["ownership_universe_known"])
        self.assertFalse(payload["coverage_can_clear_custody_gaps"])

        with self.assertRaises(AppError) as raised:
            _ui_custody_coverage_payload_from_conn(conn, {"descriptor": "private"})
        self.assertEqual(raised.exception.code, "validation")

    def test_daemon_projects_only_the_public_gap_contract(self):
        conn = sqlite3.connect(":memory:")
        raw = {
            "summary": {
                "total": 1,
                "needs_review": 1,
                "unresolved_msat": 10**20,
                "candidate_residual_msat": 10**19,
                "candidate_residual_by_asset": [
                    {"asset": "BTC", "amount_msat": 10**19},
                    {"asset": "LBTC", "amount_msat": 5 * 10**18},
                ],
                "canonical_unresolved_msat": 10**20,
                "canonical_issue_count": 2,
                "canonical_unresolved_by_asset": [
                    {"asset": "BTC", "amount_msat": 10**20, "issue_count": 1}
                ],
                "canonical_unquantified_issue_count": 1,
                "canonical_status": "known_custody_gaps",
                "canonical_status_text": "Known custody gaps require review",
                "derived_state_current": True,
                "qualification": "Current imported evidence only.",
                "search_complete": False,
                "search_status": "capacity_limited",
                "search_limit_kind": "candidate_count",
                "search_candidate_count": 501,
                "private_count": 9,
            },
            "gaps": [
                {
                    "gap_id": "gap:1",
                    "status": "needs_review",
                    "status_reason": "candidate_evidence_drift",
                    "asset": "BTC",
                    "source_wallet_label": "Old vault",
                    "destination_wallet_labels": ["New vault"],
                    "source_total_msat": 10**20,
                    "source_fee_msat": 1000,
                    "source_debit_msat": 10**20 + 1000,
                    "return_total_msat": 9 * 10**19,
                    "residual_msat": 10**19,
                    "started_at": "2020-01-01T00:00:00Z",
                    "ended_at": "2021-01-01T00:00:00Z",
                    "confidence": "strong",
                    "promotion_eligible": True,
                    "competitor_score_margin": 80,
                    "reason_codes": ["long_horizon"],
                    "downstream": {
                        "affected_disposals": 2,
                        "affected_years": [2022],
                        "transaction_ids": ["private-tx"],
                    },
                    "address": "bc1-private",
                    "descriptor": "wpkh(private)",
                    "source_ids": ["private-tx"],
                }
            ],
            "next_cursor": "100",
        }
        with (
            patch(
                "kassiber.daemon.resolve_scope",
                return_value=({"id": "workspace"}, {"id": "profile"}),
            ),
            patch(
                "kassiber.daemon.core_custody_gaps.build_gap_snapshot",
                return_value=raw,
            ) as build_snapshot,
        ):
            payload = _ui_custody_gap_payload_from_conn(
                conn,
                "ui.custody.gaps.list",
                {"limit": 100, "cursor": "50"},
            )

        build_snapshot.assert_called_once_with(
            conn,
            "profile",
            gap_id=None,
            limit=100,
            cursor="50",
        )
        self.assertEqual(payload["next_cursor"], "100")

        self.assertEqual(payload["summary"]["unresolved_msat"], str(10**20))
        self.assertEqual(
            payload["summary"]["canonical_unresolved_msat"],
            str(10**20),
        )
        self.assertEqual(
            payload["summary"]["candidate_residual_msat"],
            str(10**19),
        )
        self.assertEqual(
            payload["summary"]["candidate_residual_by_asset"],
            [
                {"asset": "BTC", "amount_msat": str(10**19)},
                {"asset": "LBTC", "amount_msat": str(5 * 10**18)},
            ],
        )
        self.assertEqual(payload["summary"]["canonical_issue_count"], 2)
        self.assertEqual(
            payload["summary"]["canonical_unresolved_by_asset"],
            [{"asset": "BTC", "amount_msat": str(10**20), "issue_count": 1}],
        )
        self.assertEqual(
            payload["summary"]["canonical_unquantified_issue_count"], 1
        )
        self.assertEqual(payload["summary"]["canonical_status"], "known_custody_gaps")
        self.assertTrue(payload["summary"]["derived_state_current"])
        self.assertEqual(
            payload["summary"]["qualification"], "Current imported evidence only."
        )
        self.assertFalse(payload["summary"]["search_complete"])
        self.assertEqual(payload["summary"]["search_status"], "capacity_limited")
        self.assertEqual(
            payload["summary"]["search_limit_kind"], "candidate_count"
        )
        self.assertEqual(payload["summary"]["search_candidate_count"], 501)
        self.assertNotIn("private_count", payload["summary"])
        gap = payload["gaps"][0]
        self.assertNotIn("address", gap)
        self.assertNotIn("descriptor", gap)
        self.assertNotIn("source_ids", gap)
        self.assertTrue(gap["promotion_eligible"])
        self.assertEqual(gap["status_reason"], "candidate_evidence_drift")
        self.assertEqual(gap["competitor_score_margin"], 80)
        self.assertEqual(gap["source_fee_msat"], 1000)
        self.assertEqual(gap["source_debit_msat"], str(10**20 + 1000))
        self.assertEqual(
            gap["downstream"],
            {"affected_disposals": 2, "affected_years": [2022]},
        )

    def test_review_context_requires_a_nonempty_gap_id(self):
        with self.assertRaisesRegex(AppError, "requires gap_id"):
            _ui_custody_gap_payload_from_conn(
                sqlite3.connect(":memory:"),
                "ui.custody.gaps.review_context",
                {"gap_id": ""},
            )

    def test_gap_list_rejects_invalid_cursor(self):
        with self.assertRaises(AppError) as raised:
            _ui_custody_gap_payload_from_conn(
                sqlite3.connect(":memory:"),
                "ui.custody.gaps.list",
                {"cursor": "not-an-offset"},
            )
        self.assertEqual(raised.exception.code, "validation")

    def test_remote_ai_provider_cannot_receive_cross_wallet_gap_linkage(self):
        result = _execute_read_only_ai_tool(
            ParsedAiToolCall(
                call_id="call-1",
                name="ui.custody.gaps.list",
                arguments={},
            ),
            AiToolRuntime(
                data_root="/tmp",
                runtime_config={},
                main_thread_tasks=queue.Queue(),
                maintenance_state={"provider_kind": "openai"},
            ),
        )
        self.assertEqual(result["ok"], False)
        self.assertEqual(result["reason"], "local_provider_required")

        coverage = _execute_read_only_ai_tool(
            ParsedAiToolCall(
                call_id="call-coverage",
                name="ui.custody.coverage.snapshot",
                arguments={},
            ),
            AiToolRuntime(
                data_root="/tmp",
                runtime_config={},
                main_thread_tasks=queue.Queue(),
                maintenance_state={"provider_kind": "openai"},
            ),
        )
        self.assertEqual(coverage["ok"], False)
        self.assertEqual(coverage["reason"], "local_provider_required")

        lineage = _execute_read_only_ai_tool(
            ParsedAiToolCall(
                call_id="call-lineage",
                name="ui.custody.lineage.snapshot",
                arguments={},
            ),
            AiToolRuntime(
                data_root="/tmp",
                runtime_config={},
                main_thread_tasks=queue.Queue(),
                maintenance_state={"provider_kind": "openai"},
            ),
        )
        self.assertEqual(lineage["ok"], False)
        self.assertEqual(lineage["reason"], "local_provider_required")

    def test_remote_ai_provider_cannot_preview_or_mutate_gap_linkage(self):
        runtime = AiToolRuntime(
            data_root="/tmp",
            runtime_config={},
            main_thread_tasks=queue.Queue(),
            maintenance_state={"provider_kind": "openai"},
        )
        preview = _execute_read_only_ai_tool(
            ParsedAiToolCall(
                call_id="call-preview",
                name="ui.custody.review.plan",
                arguments={"action": "create", "gap_id": "gap"},
            ),
            runtime,
        )
        create = _execute_mutating_ai_tool(
            ParsedAiToolCall(
                call_id="call-create",
                name="ui.custody.review.apply",
                arguments={
                    "action": "create",
                    "gap_id": "gap",
                    "expected_fingerprint": "0" * 64,
                },
            ),
            runtime,
        )
        self.assertEqual(preview["reason"], "local_provider_required")
        self.assertEqual(create["reason"], "local_provider_required")

    def test_bounded_search_is_an_ordinary_incomplete_queue_result(self):
        with (
            patch(
                "kassiber.daemon.resolve_scope",
                return_value=({"id": "workspace"}, {"id": "profile"}),
            ),
            patch(
                "kassiber.daemon.core_custody_gaps.build_gap_snapshot",
                return_value={
                    "summary": {
                        "search_complete": False,
                        "search_status": "capacity_limited",
                        "search_limit_kind": "candidate_population",
                        "search_candidate_count": 5_541,
                    },
                    "gaps": [],
                    "next_cursor": None,
                },
            ),
        ):
            payload = _ui_custody_gap_payload_from_conn(
                sqlite3.connect(":memory:"),
                "ui.custody.gaps.list",
                {},
            )
        self.assertFalse(payload["summary"]["search_complete"])
        self.assertEqual(payload["summary"]["search_status"], "capacity_limited")
        self.assertEqual(payload["summary"]["search_candidate_count"], 5_541)


if __name__ == "__main__":
    unittest.main()
