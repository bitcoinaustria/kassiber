from __future__ import annotations

import queue
import sqlite3
import unittest
from unittest.mock import patch

from kassiber.ai.tools import get_tool
from kassiber.ai.prompt import build_openai_tools
from kassiber.core.custody_gaps import CustodyGapSearchLimitError
from kassiber.daemon import (
    AiToolRuntime,
    AI_TOOL_ONCE_ONLY_CONSENT,
    ParsedAiToolCall,
    SUPPORTED_KINDS,
    _execute_read_only_ai_tool,
    _execute_mutating_ai_tool,
    _ui_custody_coverage_payload_from_conn,
    _ui_custody_gap_payload_from_conn,
)
from kassiber.errors import AppError


class CustodyGapSurfaceTest(unittest.TestCase):
    def test_read_kinds_are_supported_and_ai_read_only(self):
        for kind in (
            "ui.custody.coverage.snapshot",
            "ui.custody.gaps.list",
            "ui.custody.gaps.review_context",
            "ui.custody.gaps.history",
            "ui.custody.gaps.bridge.preview",
            "ui.custody.gaps.reopen.preview",
            "ui.custody.gaps.revise.preview",
            "ui.custody.gaps.residual.preview",
        ):
            self.assertIn(kind, SUPPORTED_KINDS)
            tool = get_tool(kind)
            self.assertIsNotNone(tool)
            self.assertEqual(tool.kind_class, "read_only")
            self.assertEqual(tool.daemon_kind, kind)

        for kind in (
            "ui.custody.gaps.bridge.create",
            "ui.custody.gaps.dismiss",
            "ui.custody.gaps.reopen",
            "ui.custody.gaps.revise",
            "ui.custody.gaps.residual.classify",
        ):
            self.assertIn(kind, SUPPORTED_KINDS)
            self.assertEqual(get_tool(kind).kind_class, "mutating")
            self.assertIn(kind, AI_TOOL_ONCE_ONLY_CONSENT)

        review = get_tool("ui.custody.gaps.review_context")
        self.assertEqual(review.parameters["required"], ["gap_id"])
        self.assertFalse(review.parameters["additionalProperties"])
        gap_list = get_tool("ui.custody.gaps.list")
        self.assertEqual(gap_list.parameters["properties"]["cursor"]["type"], "string")
        residual = get_tool("ui.custody.gaps.residual.classify")
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
                "kassiber.daemon.core_custody_gap_reviews.classify_residual",
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
                "ui.custody.gaps.residual.classify",
                {
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
                "ui.custody.gaps.residual.preview",
                {
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
                name="ui.custody.gaps.bridge.preview",
                arguments={"gap_id": "gap"},
            ),
            runtime,
        )
        create = _execute_mutating_ai_tool(
            ParsedAiToolCall(
                call_id="call-create",
                name="ui.custody.gaps.bridge.create",
                arguments={
                    "gap_id": "gap",
                    "expected_fingerprint": "0" * 64,
                },
            ),
            runtime,
        )
        self.assertEqual(preview["reason"], "local_provider_required")
        self.assertEqual(create["reason"], "local_provider_required")

    def test_bounded_search_failure_does_not_claim_an_empty_queue(self):
        with (
            patch(
                "kassiber.daemon.resolve_scope",
                return_value=({"id": "workspace"}, {"id": "profile"}),
            ),
            patch(
                "kassiber.daemon.core_custody_gaps.build_gap_snapshot",
                side_effect=CustodyGapSearchLimitError("too many rows"),
            ),
            self.assertRaises(AppError) as raised,
        ):
            _ui_custody_gap_payload_from_conn(
                sqlite3.connect(":memory:"),
                "ui.custody.gaps.list",
                {},
            )
        self.assertEqual(raised.exception.code, "custody_gap_search_limit")


if __name__ == "__main__":
    unittest.main()
