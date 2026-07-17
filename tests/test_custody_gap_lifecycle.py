from __future__ import annotations

from dataclasses import replace
import tempfile
import unittest
import io
import json
import sqlite3
from contextlib import redirect_stdout
from unittest.mock import patch

from kassiber.cli.main import build_parser, dispatch
from kassiber.cli import handlers
from kassiber.core import (
    custody_components,
    custody_filed_reports,
    custody_gap_reviews,
    custody_gaps,
)
from kassiber.db import open_db
from kassiber.daemon import _ui_custody_gap_payload_from_conn
from kassiber.errors import AppError


BTC = 100_000_000_000


def _review_action(conn, kwargs):
    if kwargs.get("classification") is not None:
        return "classify_residual"
    candidate = kwargs.get("candidate")
    if candidate is None:
        return "reopen"
    latest = custody_gap_reviews.latest_reviews(conn, kwargs["profile_id"]).get(
        candidate.gap_id
    )
    return (
        "revise"
        if latest and latest.get("event_kind") == "bridge_reopened"
        else "create"
    )


def _preview_review(conn, *, action=None, **kwargs):
    action = action or _review_action(conn, kwargs)
    plan = custody_gap_reviews.plan_review(conn, action=action, **kwargs)
    public = custody_gap_reviews.public_review_plan(plan)
    candidate = plan.get("candidate")
    public["candidate_fingerprint"] = (
        custody_gap_reviews.candidate_fingerprint(candidate) if candidate else None
    )
    public["authored_claim_fingerprint"] = plan.get("authored_claim_fingerprint")
    public["expected_input_version"] = plan["input_version"]
    return public


def _apply_review(conn, *, action=None, **kwargs):
    action = action or _review_action(conn, kwargs)
    if "expected_input_version" not in kwargs:
        plan_args = {
            key: value
            for key, value in kwargs.items()
            if key != "commit"
        }
        kwargs["expected_input_version"] = custody_gap_reviews.plan_review(
            conn, action=action, **plan_args
        )["input_version"]
    return custody_gap_reviews.apply_review(conn, action=action, **kwargs)


def _append_dismissal(conn, **kwargs):
    commit = kwargs.pop("commit", True)
    plan = custody_gap_reviews.plan_review(conn, action="dismiss", **kwargs)
    return custody_gap_reviews.apply_review(
        conn,
        action="dismiss",
        expected_input_version=plan["input_version"],
        commit=commit,
        **kwargs,
    )


class CustodyGapLifecycleTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.TemporaryDirectory()
        self.conn = open_db(self.root.name)
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', 'now')"
        )
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, created_at) "
            "VALUES('profile', 'ws', 'Book', 'now')"
        )
        for wallet_id, label in (("a", "Old vault"), ("c", "New vault")):
            self.conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, label, kind, config_json, created_at
                ) VALUES(?, 'ws', 'profile', ?, 'descriptor',
                         '{"chain":"bitcoin","network":"main"}', 'now')
                """,
                (wallet_id, label),
            )
        self._transaction(
            "out", "a", "outbound", 10 * BTC, "2020-01-01T00:00:00Z",
            fee=10_000_000, privacy_boundary="coinjoin",
        )
        self._transaction(
            "return", "c", "inbound", 99 * BTC // 10, "2021-01-01T00:00:00Z"
        )
        self.conn.commit()

    def tearDown(self):
        self.conn.close()
        self.root.cleanup()

    def _transaction(
        self,
        tx_id,
        wallet_id,
        direction,
        amount,
        occurred_at,
        *,
        fee=0,
        privacy_boundary=None,
    ):
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, direction, asset, amount, fee,
                privacy_boundary, raw_json, created_at
            ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, 'BTC', ?, ?, ?, '{}', ?)
            """,
            (
                tx_id, wallet_id, tx_id, f"fp-{tx_id}", occurred_at,
                direction, amount, fee, privacy_boundary, occurred_at,
            ),
        )

    def _candidate(self):
        return custody_gaps.find_gap_candidate(self.conn, "profile", self._gap_id())

    def _gap_id(self):
        result, _ = custody_gaps.load_gap_search_result(self.conn, "profile")
        candidates = list(result.candidates)
        self.assertEqual(len(candidates), 1)
        return candidates[0].gap_id

    def _mark_journal_current(self, *, version: int = 7):
        active_count = self.conn.execute(
            "SELECT COUNT(*) FROM transactions WHERE profile_id = 'profile' "
            "AND excluded = 0"
        ).fetchone()[0]
        self.conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = '2022-01-01T00:00:00Z',
                last_processed_tx_count = ?,
                journal_input_version = ?,
                last_processed_input_version = ?,
                ownership_review_counts_json = '{"reviewed": 2}'
            WHERE id = 'profile'
            """,
            (active_count, version, version),
        )
        self.conn.commit()

    def _create_bridge(self):
        candidate = self._candidate()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )
        created = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
            expected_input_version=preview["expected_input_version"],
        )
        return candidate, created

    def _assert_reviewed_non_sale_residual(self, classification):
        self._transaction(
            "fund",
            "a",
            "inbound",
            10 * BTC + 10_000_000,
            "2019-01-01T00:00:00Z",
        )
        self.conn.execute(
            "UPDATE transactions SET fiat_currency = 'EUR', fiat_rate = 1000, "
            "fiat_rate_exact = '1000' WHERE id = 'fund'"
        )
        self.conn.commit()
        candidate, _created = self._create_bridge()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification=classification,
        )
        resolved = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification=classification,
            expected_input_version=preview["expected_input_version"],
        )
        self.conn.execute(
            "UPDATE transactions SET fiat_currency = 'EUR', fiat_rate = 10000, "
            "fiat_rate_exact = '10000' WHERE id = 'out'"
        )
        self.conn.commit()
        processed = handlers.process_journals(self.conn, "Books", "Book")
        self.assertFalse(processed["custody_quantity"]["blocked"])
        reasons = {
            row["reason"]
            for row in self.conn.execute(
                "SELECT reason FROM journal_quarantines "
                "WHERE profile_id = 'profile' AND transaction_id = 'out'"
            )
        }
        self.assertIn("non_sale_disposal_kind", reasons)
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM journal_entries "
                "WHERE transaction_id = 'out' AND entry_type = 'disposal'"
            ).fetchone()[0],
            0,
        )
        component = custody_components.get_component(
            self.conn, resolved["component_id"]
        )
        self.assertEqual(
            component["evidence"]["residual_classification"]["classification"],
            classification,
        )

    def test_current_fingerprint_dismisses_and_changed_evidence_reopens(self):
        self._mark_journal_current()
        candidate = self._candidate()
        fingerprint = custody_gap_reviews.candidate_fingerprint(candidate)
        _append_dismissal(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
            reason="not ours",
        )

        freshness = self.conn.execute(
            """
            SELECT last_processed_at, last_processed_tx_count,
                   journal_input_version, last_processed_input_version,
                   ownership_review_counts_json
            FROM profiles WHERE id = 'profile'
            """
        ).fetchone()
        self.assertIsNone(freshness["last_processed_at"])
        self.assertEqual(freshness["last_processed_tx_count"], 0)
        self.assertEqual(freshness["journal_input_version"], 8)
        self.assertEqual(freshness["last_processed_input_version"], 7)
        self.assertIsNone(freshness["ownership_review_counts_json"])

        dismissed = custody_gaps.build_gap_snapshot(self.conn, "profile")
        self.assertEqual(dismissed["gaps"][0]["status"], "dismissed")
        self.assertEqual(
            custody_gap_reviews.latest_dismissed_fingerprints(self.conn, "profile"),
            {candidate.gap_id: fingerprint},
        )

        self.conn.execute(
            "UPDATE transactions SET amount = ? WHERE id = 'return'",
            (98 * BTC // 10,),
        )
        self.conn.commit()
        reopened = custody_gaps.build_gap_snapshot(self.conn, "profile")
        self.assertEqual(reopened["gaps"][0]["status"], "needs_review")
        self.assertNotEqual(reopened["gaps"][0]["candidate_fingerprint"], fingerprint)

    def test_dismissal_transaction_relations_are_atomic_and_survive_retraction(self):
        candidate = self._candidate()
        fingerprint = custody_gap_reviews.candidate_fingerprint(candidate)
        with patch(
            "kassiber.core.custody_gap_reviews._append_review_transaction_relations",
            side_effect=RuntimeError("relation write failed"),
        ):
            with self.assertRaisesRegex(RuntimeError, "relation write failed"):
                _append_dismissal(
                    self.conn,
                    workspace_id="ws",
                    profile_id="profile",
                    candidate=candidate,
                )
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_gap_reviews").fetchone()[0],
            0,
        )
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_gap_review_transactions"
            ).fetchone()[0],
            0,
        )

        review = _append_dismissal(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )
        review_id = self.conn.execute(
            "SELECT id FROM custody_gap_reviews WHERE profile_id = 'profile' AND gap_id = ?",
            (review["gap_id"],),
        ).fetchone()[0]
        relations = [
            (row["role"], row["transaction_id"])
            for row in self.conn.execute(
                """
                SELECT role, transaction_id
                FROM custody_gap_review_transactions
                WHERE review_id = ? ORDER BY role, transaction_id
                """,
                (review_id,),
            ).fetchall()
        ]
        self.assertEqual(
            relations,
            [("return", "return"), ("source", "out")],
        )

        self.conn.execute("DELETE FROM transactions WHERE id = 'out'")
        self.conn.commit()
        self.assertEqual(
            self.conn.execute(
                "SELECT transaction_id FROM custody_gap_review_transactions "
                "WHERE review_id = ? AND role = 'source'",
                (review_id,),
            ).fetchone()[0],
            "out",
        )

    def test_legacy_resolved_review_relations_backfill_from_component_anchors(self):
        candidate, created = self._create_bridge()
        self.conn.execute("DROP TABLE custody_gap_review_transactions")
        self.conn.commit()
        self.conn.close()
        self.conn = open_db(self.root.name)

        relations = [
            (row["role"], row["transaction_id"])
            for row in self.conn.execute(
                """
                SELECT role, transaction_id
                FROM custody_gap_review_transactions
                WHERE review_id = ? ORDER BY role, transaction_id
                """,
                (created["review_id"],),
            ).fetchall()
        ]
        self.assertEqual(
            relations,
            [("return", "return"), ("source", "out")],
        )
        self.assertEqual(candidate.source_ids, ("out",))

    def test_dismissal_rolls_back_if_journal_invalidation_fails(self):
        self._mark_journal_current()
        candidate = self._candidate()
        fingerprint = custody_gap_reviews.candidate_fingerprint(candidate)

        with (
            patch(
                "kassiber.core.custody_gap_reviews._invalidate_journals",
                side_effect=sqlite3.OperationalError("invalidation failed"),
            ),
            self.assertRaises(sqlite3.OperationalError),
        ):
            _append_dismissal(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                candidate=candidate,
            )

        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_gap_reviews").fetchone()[0],
            0,
        )
        freshness = self.conn.execute(
            "SELECT last_processed_at, journal_input_version FROM profiles "
            "WHERE id = 'profile'"
        ).fetchone()
        self.assertEqual(freshness["last_processed_at"], "2022-01-01T00:00:00Z")
        self.assertEqual(freshness["journal_input_version"], 7)

    def test_fingerprint_and_guided_bridge_bind_protocol_scope(self):
        candidate = self._candidate()
        fingerprint = custody_gap_reviews.candidate_fingerprint(candidate)
        self.conn.execute(
            "UPDATE wallets SET config_json = '{\"chain\":\"bitcoin\",\"network\":\"test\"}' "
            "WHERE id IN ('a', 'c')"
        )
        self.conn.commit()

        changed = self._candidate()

        self.assertEqual(changed.gap_id, candidate.gap_id)
        self.assertEqual(changed.network, "test")
        self.assertNotEqual(
            custody_gap_reviews.candidate_fingerprint(changed), fingerprint
        )
        with self.assertRaises(AppError) as raised:
            _apply_review(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                candidate=candidate,
            )
        self.assertEqual(raised.exception.code, "custody_gap_stale")

    def test_review_actions_rederive_candidate_after_transaction_change(self):
        candidate = self._candidate()
        fingerprint = custody_gap_reviews.candidate_fingerprint(candidate)
        self.conn.execute(
            "UPDATE transactions SET amount = ? WHERE id = 'return'",
            (98 * BTC // 10,),
        )
        self.conn.commit()

        for action in ("dismiss", "bridge"):
            with self.subTest(action=action), self.assertRaises(AppError) as raised:
                if action == "dismiss":
                    _append_dismissal(
                        self.conn,
                        workspace_id="ws",
                        profile_id="profile",
                        candidate=candidate,
                    )
                else:
                    _apply_review(
                        self.conn,
                        workspace_id="ws",
                        profile_id="profile",
                        candidate=candidate,
                    )
            self.assertEqual(raised.exception.code, "custody_gap_stale")

        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_gap_reviews").fetchone()[0],
            0,
        )
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0],
            0,
        )

    def test_guided_bridge_rejects_unknown_scope_after_preview(self):
        candidate = self._candidate()
        fingerprint = custody_gap_reviews.candidate_fingerprint(candidate)
        self.conn.execute(
            "UPDATE wallets SET config_json = "
            "'{\"chain\":\"future-layer\",\"network\":\"main\"}' "
            "WHERE id IN ('a', 'c')"
        )
        self.conn.commit()

        with self.assertRaises(AppError) as raised:
            _apply_review(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                candidate=candidate,
            )

        self.assertEqual(raised.exception.code, "custody_gap_stale")
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0],
            0,
        )

    def test_guided_bridge_preserves_lightning_protocol_scope(self):
        self.conn.execute(
            "UPDATE wallets SET kind = 'lnd', config_json = '{\"network\":\"main\"}' "
            "WHERE id IN ('a', 'c')"
        )
        self.conn.commit()
        candidate = self._candidate()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )

        created = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
            expected_input_version=preview["expected_input_version"],
        )

        self.assertEqual(candidate.protocol_chain, "lightning")
        scopes = {
            (row["rail"], row["chain"], row["network"])
            for row in self.conn.execute(
                "SELECT rail, chain, network FROM custody_component_legs "
                "WHERE component_id = ? AND role != 'suspense'",
                (created["component_id"],),
            )
        }
        self.assertEqual(scopes, {("lightning", "bitcoin", "main")})

    def test_preview_then_create_authors_exact_active_bridge_without_fee_leakage(self):
        candidate = self._candidate()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
            authored_source="cli",
        )
        self.assertTrue(preview["dry_run"])
        self.assertTrue(preview["activatable"])
        self.assertNotEqual(
            preview["authored_claim_fingerprint"],
            preview["candidate_fingerprint"],
        )
        self.assertEqual(preview["retained_msat"], 99 * BTC // 10)
        self.assertEqual(preview["residual_msat"], BTC // 10)
        self.assertEqual(preview["fee_msat"], 10_000_000)
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0],
            0,
        )

        result = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
            expected_input_version=preview["expected_input_version"],
            authored_source="cli",
        )
        self.assertEqual(result["status"], "resolved")
        component = self.conn.execute(
            "SELECT state, authored_source FROM custody_components WHERE id = ?",
            (result["component_id"],),
        ).fetchone()
        self.assertEqual((component["state"], component["authored_source"]), ("active", "cli"))
        roles = {
            row["role"]: int(row["total"])
            for row in self.conn.execute(
                "SELECT role, SUM(amount_msat) AS total FROM custody_component_legs "
                "WHERE component_id = ? GROUP BY role",
                (result["component_id"],),
            )
        }
        self.assertEqual(roles["destination"], 99 * BTC // 10)
        self.assertEqual(roles["suspense"], BTC // 10)
        self.assertEqual(roles["fee"], 10_000_000)
        history = custody_gaps.build_gap_snapshot(
            self.conn, "profile", gap_id=candidate.gap_id
        )
        self.assertEqual(history["gaps"][0]["status"], "resolved")
        self.assertEqual(history["summary"]["resolved"], 0)
        self.assertEqual(
            history["gaps"][0]["correction"],
            {
                "component_id": result["component_id"],
                "strategy": "create_revision_then_activate",
            },
        )

    def test_review_plans_are_read_only_and_apply_exact_component_rows(self):
        candidate = self._candidate()
        self.conn.execute("DELETE FROM custody_gap_candidate_projections")
        self.conn.commit()
        before = self.conn.total_changes
        plan = custody_gap_reviews.plan_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            action="create",
            candidate=candidate,
            authored_source="cli",
        )
        self.assertEqual(self.conn.total_changes, before)
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_gap_candidate_projections"
            ).fetchone()[0],
            0,
        )
        self.assertTrue(plan["activatable"])
        repeated = custody_gap_reviews.plan_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            action="create",
            candidate=candidate,
            authored_source="cli",
        )
        self.assertEqual(repeated["input_version"], plan["input_version"])
        self.assertEqual(repeated["component_plan"], plan["component_plan"])

        created = custody_gap_reviews.apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            action="create",
            candidate=candidate,
            expected_input_version=plan["input_version"],
            authored_source="cli",
        )
        self.assertTrue(created["review_id"])
        persisted = custody_components.get_component(
            self.conn, created["component_id"]
        )
        leg_fields = tuple(plan["component_plan"]["legs"][0])
        allocation_fields = tuple(plan["component_plan"]["allocations"][0])
        self.assertEqual(
            [{key: row[key] for key in leg_fields} for row in persisted["legs"]],
            plan["component_plan"]["legs"],
        )
        self.assertEqual(
            [
                {key: row[key] for key in allocation_fields}
                for row in persisted["allocations"]
            ],
            plan["component_plan"]["allocations"],
        )

        for preview_call in (
            lambda: _preview_review(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                gap_id=candidate.gap_id,
                classification="external_payment",
            ),
            lambda: _preview_review(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                gap_id=candidate.gap_id,
            ),
        ):
            before = self.conn.total_changes
            preview_call()
            self.assertEqual(self.conn.total_changes, before)

    def test_guided_bridge_previews_and_retains_filed_report_amendment_history(self):
        filed = custody_filed_reports.create_filed_report_snapshot(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            report_kind="capital-gains",
            report_state="filed",
            period_start_year=2021,
            period_end_year=2021,
            content_sha256="ab" * 32,
            classification_summary={
                "external_presumed": {
                    "count": 1,
                    "amount_msat": 99 * BTC // 10,
                }
            },
            gain_summary={
                "fiat_currency": "EUR",
                "proceeds_exact": "99000.00",
                "cost_basis_exact": "90000.00",
                "gain_loss_exact": "9000.00",
                "status": "final",
            },
            created_at="2022-04-01T00:00:00Z",
        )
        custody_filed_reports.create_filed_report_snapshot(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            report_kind="capital-gains",
            report_state="filed",
            period_start_year=2019,
            period_end_year=2019,
            content_sha256="cd" * 32,
        )
        candidate = self._candidate()

        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )

        self.assertEqual(len(preview["filed_report_impacts"]), 1)
        impact_preview = preview["filed_report_impacts"][0]
        self.assertEqual(impact_preview["filed_report_snapshot_id"], filed["id"])
        self.assertEqual(impact_preview["affected_period_start_year"], 2021)
        self.assertEqual(
            impact_preview["after_gain_summary"],
            {"status": "pending_journal_rebuild"},
        )
        self.assertIn("amended filing", impact_preview["amendment_warning"])
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_filed_report_impacts"
            ).fetchone()[0],
            0,
        )

        created = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
            expected_input_version=preview["expected_input_version"],
        )

        self.assertEqual(len(created["filed_report_impacts"]), 1)
        impacts = custody_filed_reports.list_custody_impacts(self.conn, "profile")
        self.assertEqual(len(impacts), 1)
        self.assertEqual(impacts[0]["component_id"], created["component_id"])
        self.assertEqual(
            impacts[0]["before_gain_summary"]["gain_loss_exact"], "9000.00"
        )
        self.assertEqual(
            impacts[0]["after_classification_summary"],
            {
                "custody_suspense": {"count": 1, "amount_msat": BTC // 10},
                "internal_retained": {
                    "count": 1,
                    "amount_msat": 99 * BTC // 10,
                },
                "network_fee": {"count": 1, "amount_msat": 10_000_000},
            },
        )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "UPDATE custody_filed_report_impacts SET amendment_warning = 'rewrite'"
            )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "UPDATE filed_report_snapshots SET content_sha256 = ? WHERE id = ?",
                ("ef" * 32, filed["id"]),
            )

    def test_filed_report_impact_follows_profile_asset_pool_across_wallets(self):
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(
                'other', 'ws', 'profile', 'Unrelated wallet', 'descriptor',
                '{"chain":"bitcoin","network":"main"}', 'now'
            )
            """
        )
        self._transaction(
            "other-wallet-sale",
            "other",
            "outbound",
            BTC,
            "2023-06-01T00:00:00Z",
        )
        filed = custody_filed_reports.create_filed_report_snapshot(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            report_kind="capital-gains",
            report_state="filed",
            period_start_year=2023,
            period_end_year=2023,
            content_sha256="fa" * 32,
            created_at="2024-04-01T00:00:00Z",
        )
        candidate = self._candidate()
        snapshot = custody_gap_reviews._candidate_snapshot(self.conn, candidate)

        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )

        self.assertEqual(snapshot["downstream"]["affected_disposals"], 1)
        self.assertEqual(snapshot["downstream"]["affected_years"], [2023])
        self.assertEqual(
            [
                impact["filed_report_snapshot_id"]
                for impact in preview["filed_report_impacts"]
            ],
            [filed["id"]],
        )

    def test_filed_report_impact_failure_rolls_back_bridge_and_review_atomically(self):
        custody_filed_reports.create_filed_report_snapshot(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            report_kind="capital-gains",
            report_state="filed",
            period_start_year=2021,
            period_end_year=2021,
            content_sha256="ab" * 32,
        )
        candidate = self._candidate()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )

        with (
            patch(
                "kassiber.core.custody_gap_reviews.custody_filed_reports.append_custody_impacts",
                side_effect=sqlite3.OperationalError("impact write failed"),
            ),
            self.assertRaises(sqlite3.OperationalError),
        ):
            _apply_review(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                candidate=candidate,
                expected_input_version=preview["expected_input_version"],
            )

        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0],
            0,
        )
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_gap_reviews").fetchone()[0],
            0,
        )
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_filed_report_impacts"
            ).fetchone()[0],
            0,
        )

    def test_amount_time_only_hint_can_be_explicitly_reviewed(self):
        self.conn.execute(
            "UPDATE transactions SET privacy_boundary = NULL WHERE id = 'out'"
        )
        self.conn.commit()
        candidate = self._candidate()
        self.assertFalse(candidate.promotion_eligible)
        with self.assertRaises(AppError) as not_previewed:
            custody_gap_reviews.apply_review(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                action="create",
                candidate=candidate,
                expected_input_version=custody_gap_reviews.candidate_fingerprint(
                    candidate
                ),
            )
        self.assertEqual(
            not_previewed.exception.code, "custody_review_plan_invalid"
        )

        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )

        self.assertEqual(preview["review_mode"], "manual_weak_hint")
        self.assertTrue(preview["requires_explicit_confirmation"])
        self.assertIn("weak_advisory_evidence", preview["warnings"])
        self.assertEqual(
            preview["candidate_fingerprint"],
            custody_gap_reviews.candidate_fingerprint(candidate),
        )
        created = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
            expected_input_version=preview["expected_input_version"],
        )
        self.assertEqual(created["status"], "resolved")

    def test_ranking_changes_do_not_invalidate_authored_bridge_commitment(self):
        candidate = self._candidate()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )
        _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
            expected_input_version=preview["expected_input_version"],
        )
        review = custody_gap_reviews.latest_reviews(self.conn, "profile")[
            candidate.gap_id
        ]
        changed_ranking = replace(
            candidate,
            promotion_eligible=False,
            competitor_score_margin=0,
            conflict_set_id="new-competitor-cluster",
            conflict_size=2,
            reason_codes=(*candidate.reason_codes, "competitor_margin_insufficient"),
        )

        self.assertNotEqual(
            custody_gap_reviews.candidate_fingerprint(candidate),
            custody_gap_reviews.candidate_fingerprint(changed_ranking),
        )
        self.assertEqual(
            custody_gap_reviews.authored_claim_fingerprint(candidate),
            custody_gap_reviews.authored_claim_fingerprint(changed_ranking),
        )
        self.assertEqual(
            custody_gap_reviews.review_status(self.conn, changed_ranking, review),
            "resolved",
        )

    def test_excess_return_is_warned_and_cannot_be_hidden_in_the_bridge(self):
        self.conn.execute(
            "UPDATE transactions SET amount = ? WHERE id = 'return'",
            (101 * BTC // 10,),
        )
        self.conn.commit()
        candidate = self._candidate()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )

        self.assertEqual(candidate.excess_msat, BTC // 10)
        self.assertIn("excess_return_unclassified", preview["warnings"])
        self.assertFalse(preview["activatable"])
        with self.assertRaises(AppError) as raised:
            _apply_review(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                candidate=candidate,
                expected_input_version=preview["expected_input_version"],
            )
        self.assertEqual(raised.exception.code, "custody_gap_bridge_excess_return")
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0],
            0,
        )

    def test_resolved_history_stays_visible_and_conflicts_after_evidence_drift(self):
        candidate = self._candidate()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )
        _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
            expected_input_version=preview["expected_input_version"],
        )
        self.conn.execute(
            "UPDATE transactions SET kind = 'evidence-changed' WHERE id = 'return'"
        )
        self.conn.commit()

        snapshot = custody_gaps.build_gap_snapshot(
            self.conn, "profile", gap_id=candidate.gap_id
        )

        self.assertEqual(snapshot["summary"]["total"], 0)
        self.assertEqual(snapshot["gaps"][0]["gap_id"], candidate.gap_id)
        self.assertEqual(snapshot["gaps"][0]["status"], "conflicting")
        self.assertIn("status_reason", snapshot["gaps"][0])

    def test_resolved_history_stays_visible_when_component_is_superseded(self):
        candidate = self._candidate()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )
        created = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
            expected_input_version=preview["expected_input_version"],
        )
        custody_components.supersede_component(
            self.conn, created["component_id"], reason="review correction"
        )
        self.conn.commit()

        snapshot = custody_gaps.build_gap_snapshot(
            self.conn, "profile", gap_id=candidate.gap_id
        )

        self.assertEqual(snapshot["summary"]["total"], 1)
        self.assertEqual(snapshot["gaps"][0]["gap_id"], candidate.gap_id)
        self.assertEqual(snapshot["gaps"][0]["status"], "conflicting")
        self.assertEqual(
            snapshot["gaps"][0]["status_reason"], "component_not_effective"
        )

    def test_cli_list_review_and_review_plan_need_no_component_json(self):
        gap_id = self._gap_id()
        for command, expected_kind in (
            (["list"], "transfers.gaps.list"),
            (["review", "--gap-id", gap_id], "transfers.gaps.review"),
            (
                ["plan", "--action", "create", "--gap-id", gap_id],
                "transfers.gaps.plan",
            ),
        ):
            args = build_parser().parse_args(
                [
                    "--data-root", self.root.name, "--machine",
                    "transfers", "gaps", *command,
                    "--workspace", "Books", "--profile", "Book",
                ]
            )
            args.format = "json"
            args.non_interactive = True
            output = io.StringIO()
            with redirect_stdout(output):
                dispatch(self.conn, args)
            envelope = json.loads(output.getvalue())
            self.assertEqual(envelope["kind"], expected_kind)
            self.assertNotIn("source_ids", json.dumps(envelope))

    def test_external_residual_classification_finalizes_exact_point_one(self):
        candidate, created = self._create_bridge()
        before = handlers.process_journals(self.conn, "Books", "Book")
        self.assertTrue(before["custody_quantity"]["blocked"])

        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification="external_payment",
        )
        self.assertEqual(preview["custody_state"], "external_confirmed")
        resolved = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification="external_payment",
            expected_input_version=preview["expected_input_version"],
        )

        self.assertEqual(resolved["component_revision"], 2)
        self.assertTrue(resolved["review_id"])
        old = custody_components.get_component(self.conn, created["component_id"])
        new = custody_components.get_component(self.conn, resolved["component_id"])
        self.assertEqual(old["state"], "superseded")
        self.assertEqual(new["effective_state"], "active")
        self.assertEqual(
            sum(
                int(leg["amount_msat"])
                for leg in new["legs"]
                if leg["role"] == "external"
            ),
            BTC // 10,
        )
        after = handlers.process_journals(self.conn, "Books", "Book")
        self.assertFalse(after["custody_quantity"]["blocked"])
        posting = self.conn.execute(
            """
            SELECT state, location_kind, amount_msat
            FROM journal_quantity_postings
            WHERE transaction_id = 'out' AND location_kind = 'external'
            """
        ).fetchone()
        self.assertEqual(
            (posting["state"], posting["location_kind"], posting["amount_msat"]),
            ("external_confirmed", "external", BTC // 10),
        )
        history = custody_gap_reviews.list_review_history(
            self.conn, "profile", candidate.gap_id
        )
        self.assertEqual(
            [row["event_kind"] for row in history["history"]],
            ["bridge_created", "residual_classified"],
        )
        latest_only = custody_gap_reviews.list_review_history(
            self.conn, "profile", candidate.gap_id, limit=1
        )
        self.assertEqual(
            [row["event_kind"] for row in latest_only["history"]],
            ["residual_classified"],
        )
        self.assertNotIn('"out"', json.dumps(history))
        self.assertNotIn('"return"', json.dumps(history))
        snapshot = custody_gaps.build_gap_snapshot(
            self.conn, "profile", gap_id=candidate.gap_id
        )
        self.assertEqual(snapshot["gaps"][0]["status"], "resolved")

    def test_retained_residual_is_internal_reviewed_without_fake_observation(self):
        candidate, _created = self._create_bridge()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification="retained_custody",
        )
        self.assertEqual(preview["custody_state"], "internal_reviewed")
        resolved = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification="retained_custody",
            expected_input_version=preview["expected_input_version"],
        )
        component = custody_components.get_component(
            self.conn, resolved["component_id"]
        )
        retained = [leg for leg in component["legs"] if leg["role"] == "retained"]
        self.assertEqual(len(retained), 1)
        self.assertEqual(retained[0]["amount_msat"], BTC // 10)
        self.assertIsNone(retained[0]["transaction_id"])
        self.assertTrue(retained[0]["location_ref"])

        processed = handlers.process_journals(self.conn, "Books", "Book")
        self.assertFalse(processed["custody_quantity"]["blocked"])
        posting = self.conn.execute(
            """
            SELECT state, location_kind, amount_msat
            FROM journal_quantity_postings
            WHERE transaction_id = 'out' AND location_kind = 'retained_custody'
            """
        ).fetchone()
        self.assertEqual(
            (posting["state"], posting["amount_msat"]),
            ("internal_reviewed", BTC // 10),
        )
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM journal_entries "
                "WHERE transaction_id = 'out' AND entry_type = 'disposal'"
            ).fetchone()[0],
            0,
        )

    def test_suspense_continuation_remains_an_explicit_custody_blocker(self):
        candidate, _created = self._create_bridge()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification="suspense_continuation",
        )
        self.assertEqual(preview["custody_state"], "custody_suspense")
        resolved = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification="suspense_continuation",
            expected_input_version=preview["expected_input_version"],
        )
        self.assertEqual(resolved["custody_state"], "custody_suspense")
        processed = handlers.process_journals(self.conn, "Books", "Book")
        self.assertTrue(processed["custody_quantity"]["blocked"])

    def test_reviewed_gift_never_degrades_to_a_market_sale(self):
        self._assert_reviewed_non_sale_residual("external_gift")

    def test_reviewed_loss_never_degrades_to_a_market_sale(self):
        self._assert_reviewed_non_sale_residual("external_loss")

    def test_reopen_then_revise_preserves_every_revision_and_history_event(self):
        candidate, created = self._create_bridge()
        changes_before_preview = self.conn.total_changes
        reopen_preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            reason="correct bridge",
        )
        self.assertEqual(self.conn.total_changes, changes_before_preview)
        reopened = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            expected_input_version=reopen_preview["expected_input_version"],
            reason="correct bridge",
        )
        self.assertEqual(reopened["status"], "needs_review")
        self.assertTrue(reopened["review_id"])
        self.assertEqual(
            custody_components.get_component(self.conn, created["component_id"])[
                "state"
            ],
            "superseded",
        )

        current = self._candidate()
        changes_before_preview = self.conn.total_changes
        revision_preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=current,
            reason="correct bridge",
        )
        self.assertEqual(self.conn.total_changes, changes_before_preview)
        revised = _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=current,
            expected_input_version=revision_preview["expected_input_version"],
            reason="correct bridge",
        )
        self.assertEqual(
            revised["component_revision"],
            revision_preview["new_component_revision"],
        )
        self.assertEqual(revised["component_revision"], 2)
        self.assertTrue(revised["review_id"])
        self.assertEqual(
            custody_components.get_component(self.conn, revised["component_id"])[
                "effective_state"
            ],
            "active",
        )
        history = custody_gap_reviews.list_review_history(
            self.conn, "profile", candidate.gap_id
        )
        self.assertEqual(
            [row["event_kind"] for row in history["history"]],
            ["bridge_created", "bridge_reopened", "bridge_revised"],
        )
        self.assertEqual(
            [row["component_revision"] for row in history["history"]],
            [1, 1, 2],
        )

    def test_residual_confirmation_fails_closed_after_evidence_change(self):
        candidate, _created = self._create_bridge()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification="external_disposal",
        )
        self.conn.execute(
            "UPDATE transactions SET kind = 'changed-after-preview' WHERE id = 'return'"
        )
        self.conn.commit()
        with self.assertRaises(AppError) as raised:
            _apply_review(
                self.conn,
                workspace_id="ws",
                profile_id="profile",
                gap_id=candidate.gap_id,
                classification="external_disposal",
                expected_input_version=preview["expected_input_version"],
            )
        self.assertEqual(raised.exception.code, "custody_gap_stale")
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_gap_reviews "
                "WHERE event_kind = 'residual_classified'"
            ).fetchone()[0],
            0,
        )

    def test_residual_write_can_join_an_outer_audit_transaction(self):
        candidate, created = self._create_bridge()
        preview = _preview_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification="external_payment",
        )
        self.conn.execute("BEGIN")
        _apply_review(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            gap_id=candidate.gap_id,
            classification="external_payment",
            expected_input_version=preview["expected_input_version"],
            commit=False,
        )
        self.assertTrue(self.conn.in_transaction)
        self.conn.rollback()
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM custody_gap_reviews "
                "WHERE event_kind = 'residual_classified'"
            ).fetchone()[0],
            0,
        )
        self.assertEqual(
            custody_components.get_component(self.conn, created["component_id"])[
                "effective_state"
            ],
            "active",
        )

    def test_cli_correction_previews_and_history_are_redacted(self):
        candidate, _created = self._create_bridge()
        for command, expected_kind in (
            (
                ["plan", "--action", "classify_residual", "--gap-id",
                 candidate.gap_id, "--classification", "external_payment"],
                "transfers.gaps.plan",
            ),
            (
                ["history", "--gap-id", candidate.gap_id],
                "transfers.gaps.history",
            ),
        ):
            args = build_parser().parse_args(
                [
                    "--data-root", self.root.name, "--machine",
                    "transfers", "gaps", *command,
                    "--workspace", "Books", "--profile", "Book",
                ]
            )
            args.format = "json"
            args.non_interactive = True
            output = io.StringIO()
            with redirect_stdout(output):
                dispatch(self.conn, args)
            envelope = json.loads(output.getvalue())
            self.assertEqual(envelope["kind"], expected_kind)
            encoded = json.dumps(envelope)
            self.assertNotIn('"out"', encoded)
            self.assertNotIn('"return"', encoded)
            self.assertNotIn("source_ids", encoded)

    def test_daemon_preview_and_create_accept_only_gap_identity_and_return_no_raw_txids(self):
        listed = _ui_custody_gap_payload_from_conn(
            self.conn, "ui.custody.gaps.list", {"workspace": "Books", "profile": "Book"}
        )
        gap = listed["gaps"][0]
        preview = _ui_custody_gap_payload_from_conn(
            self.conn,
            "ui.custody.review.plan",
            {
                "workspace": "Books",
                "profile": "Book",
                "action": "create",
                "gap_id": gap["gap_id"],
            },
        )
        self.assertNotIn('"out"', json.dumps(preview))
        created = _ui_custody_gap_payload_from_conn(
            self.conn,
            "ui.custody.review.apply",
            {
                "workspace": "Books",
                "profile": "Book",
                "action": "create",
                "gap_id": gap["gap_id"],
                "expected_input_version": preview["input_version"],
            },
        )
        self.assertEqual(created["status"], "resolved")
        self.assertNotIn("transaction", json.dumps(created))

    def test_concurrent_latest_reviews_conflict_instead_of_silently_winning(self):
        candidate = self._candidate()
        fingerprint = custody_gap_reviews.candidate_fingerprint(candidate)
        _append_dismissal(
            self.conn,
            workspace_id="ws",
            profile_id="profile",
            candidate=candidate,
        )
        row = self.conn.execute(
            "SELECT * FROM custody_gap_reviews WHERE gap_id = ?", (candidate.gap_id,)
        ).fetchone()
        self.conn.execute(
            """
            INSERT INTO custody_gap_reviews(
                id, workspace_id, profile_id, gap_id, revision,
                candidate_fingerprint, action, authored_source, snapshot_json, created_at
            ) VALUES('concurrent', 'ws', 'profile', ?, ?, ?, 'resolved',
                     'gui', ?, 'later')
            """,
            (candidate.gap_id, row["revision"], fingerprint, row["snapshot_json"]),
        )
        self.conn.commit()

        snapshot = custody_gaps.build_gap_snapshot(self.conn, "profile")
        self.assertEqual(snapshot["gaps"][0]["status"], "conflicting")
        self.assertEqual(
            custody_gap_reviews.latest_dismissed_fingerprints(self.conn, "profile"),
            {},
        )
        with self.assertRaises(sqlite3.IntegrityError):
            self.conn.execute(
                "UPDATE custody_gap_reviews SET reason = 'rewrite' WHERE id = ?",
                (row["id"],),
            )


if __name__ == "__main__":
    unittest.main()
