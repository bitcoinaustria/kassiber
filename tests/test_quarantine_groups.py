"""Quarantine rows roll up by root cause, and resolving a root reports what cleared."""

import json
import tempfile
import unittest

from kassiber import daemon
from kassiber.cli import handlers
from kassiber.core import quarantine_review
from kassiber.core.ui_snapshot import (
    build_journals_quarantine_groups_snapshot,
    build_journals_quarantine_snapshot,
)
from kassiber.db import set_setting
from kassiber.errors import AppError
from tests.test_custody_quantity_handler import BTC, SOURCE_AT, _book, _seed_transaction
from tests.test_quarantine_review import _activate_residual_component


class QuarantineGroupsTest(unittest.TestCase):
    def _open(self, root, *, component=True):
        conn = _book(root, "FIFO")
        self.addCleanup(conn.close)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", "profile")
        conn.commit()
        if component:
            _activate_residual_component(conn)
            handlers.process_journals(conn, "Books", "Book")
        return conn

    def _priced_root_with_a_dependent(self, conn):
        # A root held for a missing price and a row that waits on it. The
        # reprocess rebuilds the quarantine from the book itself.
        handlers.process_journals(conn, "Books", "Book")
        conn.execute("DELETE FROM journal_quarantines")
        conn.executemany(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, 'ws', 'profile', ?, ?, 'now')
            """,
            [
                ("buy-old", "missing_spot_price", "{}"),
                (
                    "buy-new",
                    "transfer_pair_dependency_blocked",
                    json.dumps({"blocked_by_transaction_ids": ["buy-old"]}),
                ),
            ],
        )
        conn.commit()

    def _add_waiting_rows(self, conn, count, prefix="wait"):
        # More rows that only wait on "out": the custody barrier names it.
        for index in range(count):
            tx_id = f"{prefix}-{index:02d}"
            _seed_transaction(
                conn, tx_id, "c", "outbound", BTC // 10, f"2026-02-{index + 1:02d}T00:00:00Z", 50_000
            )
            conn.execute(
                """
                INSERT INTO journal_quarantines(
                    transaction_id, workspace_id, profile_id, reason, detail_json, created_at
                ) VALUES(?, 'ws', 'profile', 'custody_basis_barrier', ?, 'now')
                """,
                (tx_id, json.dumps({"root_transaction_ids": ["out"]})),
            )
        conn.commit()

    def test_downstream_rows_roll_up_under_their_root_with_msat_totals(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            self._add_waiting_rows(conn, 3)

            overview = build_journals_quarantine_groups_snapshot(conn, {})
            listing = build_journals_quarantine_snapshot(conn, {"limit": 50})

        self.assertEqual(overview["count"], 5)
        self.assertEqual(overview["count"], listing["summary"]["count"])
        self.assertEqual(overview["waiting_count"], listing["summary"]["waiting_count"])
        self.assertEqual(overview["root_group_count"], 1)
        group = overview["by_root"][0]
        self.assertEqual(group["group_id"], "root:out")
        self.assertEqual(group["count"], 5)
        self.assertEqual(group["dependent_count"], 4)
        self.assertEqual(group["root"]["transaction_id"], "out")
        self.assertEqual(
            {item["reason"]: item["count"] for item in group["reasons"]},
            {"custody_basis_barrier": 4, "custody_quantity_unresolved": 1},
        )
        btc = group["amounts"][0]
        self.assertEqual(btc["asset"], "BTC")
        self.assertEqual(btc["outbound_msat"], 10 * BTC + BTC + 3 * (BTC // 10))
        reasons = {item["reason"]: item for item in overview["by_reason"]}
        self.assertEqual(reasons["custody_basis_barrier"]["count"], 4)
        self.assertEqual(reasons["custody_basis_barrier"]["group_id"], "reason:custody_basis_barrier")
        self.assertIn("by_root", overview["definitions"])

    def test_members_page_in_a_stable_order_and_a_changed_quarantine_stales_the_cursor(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            self._add_waiting_rows(conn, 3)

            first = build_journals_quarantine_groups_snapshot(conn, {"group_id": "root:out", "limit": 2})
            second = build_journals_quarantine_groups_snapshot(
                conn, {"group_id": "root:out", "limit": 2, "cursor": first["next_cursor"]}
            )
            third = build_journals_quarantine_groups_snapshot(
                conn, {"group_id": "root:out", "limit": 2, "cursor": second["next_cursor"]}
            )
            self._add_waiting_rows(conn, 4, prefix="more")
            with self.assertRaises(AppError) as stale:
                build_journals_quarantine_groups_snapshot(
                    conn, {"group_id": "root:out", "cursor": first["next_cursor"]}
                )
            with self.assertRaises(AppError) as unknown:
                build_journals_quarantine_groups_snapshot(conn, {"group_id": "root:nope"})
            with self.assertRaises(AppError) as malformed:
                build_journals_quarantine_groups_snapshot(
                    conn, {"group_id": "root:out", "cursor": "not-a-cursor"}
                )

        members = [m["transaction_id"] for page in (first, second, third) for m in page["members"]]
        self.assertEqual(members, ["out", "later-sale", "wait-00", "wait-01", "wait-02"])
        self.assertTrue(first["members"][0]["is_root"])
        self.assertIsNone(third["next_cursor"])
        self.assertEqual(stale.exception.code, "quarantine_cursor_stale")
        self.assertEqual(
            stale.exception.details["remediation"]["tool"], "ui.journals.quarantine.groups"
        )
        self.assertEqual(unknown.exception.code, "not_found")
        self.assertEqual(malformed.exception.code, "validation")

    def test_only_a_current_root_group_can_be_resolved(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            self.assertEqual(
                quarantine_review.root_transaction_for_group(conn, "profile", "root:out"), "out"
            )
            for group_id, code in (
                ("reason:custody_basis_barrier", "validation"),
                ("root:later-sale", "quarantine_group_stale"),
            ):
                with self.assertRaises(AppError) as raised:
                    quarantine_review.root_transaction_for_group(conn, "profile", group_id)
                self.assertEqual(raised.exception.code, code)

    def test_resolving_a_root_reports_the_waiting_rows_that_cleared(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root, component=False)
            self._priced_root_with_a_dependent(conn)
            groups = build_journals_quarantine_groups_snapshot(conn, {})

            payload = daemon._quarantine_resolution_payload(
                conn,
                {
                    "group_id": "root:buy-old",
                    "action": "price_override",
                    "fiat_rate": "10000",
                    "reason": "Price from the reviewed exchange statement",
                },
                default_source="ai_tool",
            )

        self.assertEqual(groups["by_root"][0]["group_id"], "root:buy-old")
        self.assertEqual(groups["by_root"][0]["dependent_count"], 1)
        self.assertEqual(payload["transaction_id"], "buy-old")
        self.assertEqual(payload["group_id"], "root:buy-old")
        self.assertTrue(payload["cleared"])
        self.assertEqual(
            payload["dependents_cleared"],
            {"count": 1, "transaction_ids": ["buy-new"], "truncated": False},
        )
        delta = payload["quarantine_delta"]
        self.assertEqual(delta["status"], "measured")
        self.assertEqual(delta["dependents_before"], 1)
        self.assertEqual(delta["count_before"], 2)
        self.assertEqual(delta["dependents_still_quarantined"]["count"], 0)
        self.assertEqual(delta["count_after"], delta["newly_quarantined"]["count"])

    def test_resolve_takes_exactly_one_target(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root, component=False)
            self._priced_root_with_a_dependent(conn)
            for args in (
                {"action": "exclude", "reason": "r"},
                {"action": "exclude", "reason": "r", "transaction": "buy-old", "group_id": "root:buy-old"},
            ):
                with self.assertRaises(AppError) as raised:
                    daemon._quarantine_resolution_payload(conn, args, default_source="ai_tool")
                self.assertEqual(raised.exception.code, "validation")
            with self.assertRaises(AppError) as stale:
                daemon._quarantine_resolution_payload(
                    conn,
                    {"group_id": "root:buy-new", "action": "exclude", "reason": "r"},
                    default_source="ai_tool",
                )
            self.assertEqual(stale.exception.code, "quarantine_group_stale")

    def test_without_a_reprocess_the_effect_is_unmeasured_not_zero(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root, component=False)
            self._priced_root_with_a_dependent(conn)

            payload = handlers.resolve_quarantine_with_effect(
                conn, "Books", "Book", action="exclude", group_id="root:buy-old"
            )

        self.assertIsNone(payload["dependents_cleared"])
        self.assertEqual(payload["quarantine_delta"]["status"], "not_measured")
        self.assertEqual(payload["quarantine_delta"]["dependents_before"], 1)
        self.assertEqual(payload["quarantine_delta"]["remediation"]["tool"], "ui.journals.process")

    def test_the_cli_reprocess_reports_the_same_effect(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root, component=False)
            self._priced_root_with_a_dependent(conn)

            payload = handlers.resolve_quarantine_with_effect(
                conn, "Books", "Book", action="price-override", tx_ref="buy-old",
                fiat_rate="10000", reprocess=True,
            )

        self.assertTrue(payload["reprocessed"])
        self.assertTrue(payload["cleared"])
        self.assertEqual(payload["dependents_cleared"]["transaction_ids"], ["buy-new"])

if __name__ == "__main__":
    unittest.main()
