"""The quarantine snapshot explains cause, root, blocking state and next step."""

import tempfile
import unittest

from kassiber.cli import handlers
from kassiber.core.custody_components import activate_component, create_component
from kassiber.core.ui_snapshot import build_journals_quarantine_snapshot
from kassiber.db import set_setting
from kassiber.errors import AppError
from tests.test_custody_quantity_handler import (
    BTC,
    SOURCE_AT,
    _book,
    _leg,
    _seed_transaction,
)


def _activate_residual_component(conn):
    component = create_component(
        conn,
        workspace_id="ws",
        profile_id="profile",
        component_type="manual_bridge",
        evidence_kind="manual_reconstruction",
        evidence_grade="reviewed",
        legs=[
            {
                **_leg("source", 10 * BTC, transaction_id="out", wallet_id="a", occurred_at=SOURCE_AT),
                "id": "source",
            },
            {
                **_leg(
                    "destination", 990_000_000_000,
                    transaction_id="in", wallet_id="c",
                    occurred_at="2025-01-01T00:00:00Z",
                ),
                "id": "destination",
            },
            {**_leg("suspense", 10_000_000_000, occurred_at=SOURCE_AT), "id": "suspense"},
        ],
        allocations=[
            {
                "source_leg_id": "source",
                "sink_leg_id": "destination",
                "source_amount_msat": 990_000_000_000,
                "sink_amount_msat": 990_000_000_000,
            },
            {
                "source_leg_id": "source",
                "sink_leg_id": "suspense",
                "source_amount_msat": 10_000_000_000,
                "sink_amount_msat": 10_000_000_000,
            },
        ],
    )
    activate_component(conn, component["id"])


class QuarantineReviewTest(unittest.TestCase):
    def _open(self, root):
        conn = _book(root, "FIFO")
        self.addCleanup(conn.close)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", "profile")
        conn.commit()
        return conn

    def test_downstream_rows_group_under_the_custody_problem_that_blocks_them(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            _activate_residual_component(conn)
            handlers.process_journals(conn, "Books", "Book")

            snapshot = build_journals_quarantine_snapshot(conn, {"limit": 50})

        items = {item["transaction_id"]: item for item in snapshot["items"]}
        root_item = items["out"]
        self.assertEqual(root_item["reason"], "custody_quantity_unresolved")
        self.assertFalse(root_item["is_downstream"])
        self.assertTrue(root_item["blocks_reports"])
        self.assertEqual(root_item["reasons"], ["custody_quantity_unresolved", "custody_basis_barrier"])
        self.assertEqual(root_item["evidence"]["blocker_code"], "reviewed_residual_suspense")

        later = items["later-sale"]
        self.assertEqual(later["reason"], "custody_basis_barrier")
        self.assertEqual(later["category"], "downstream")
        self.assertTrue(later["is_downstream"])
        self.assertFalse(later["blocks_reports"])
        self.assertEqual(later["root"]["transaction_id"], "out")
        self.assertEqual(later["root"]["reason"], "custody_quantity_unresolved")
        self.assertEqual(later["actions"], [{"kind": "resolve_root", "transaction_id": "out"}])

        # Roots come first; the queue reads from the fix to its consequences.
        self.assertEqual(snapshot["items"][0]["transaction_id"], "out")
        summary = snapshot["summary"]
        self.assertTrue(summary["reports_blocked"])
        self.assertEqual(summary["blocking_count"], 1)
        self.assertFalse(summary["freshness"]["needs_processing"])
        self.assertIsNone(summary["freshness"]["last_error"])
        group = summary["groups"][0]
        self.assertEqual(group["root_transaction_id"], "out")
        self.assertEqual(group["count"], 2)
        self.assertEqual(group["downstream_count"], 1)
        self.assertTrue(group["blocks_reports"])
        self.assertEqual(summary["group_count"], 1)

    def test_missing_acquisition_history_names_amounts_and_wallet(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            conn.execute("DELETE FROM transactions WHERE id IN ('out', 'in', 'later-sale')")
            _seed_transaction(
                conn, "oversell", "a", "outbound", 12 * BTC, "2024-06-01T00:00:00Z", 30_000
            )
            handlers.process_journals(conn, "Books", "Book")

            snapshot = build_journals_quarantine_snapshot(conn, {"limit": 50})

        item = next(item for item in snapshot["items"] if item["transaction_id"] == "oversell")
        self.assertEqual(item["reason"], "insufficient_lots")
        self.assertEqual(item["category"], "missing_acquisition_history")
        self.assertEqual(item["evidence"]["wallet_label"], "A")
        self.assertEqual(item["evidence"]["required_msat"], 12 * BTC)
        self.assertEqual(item["evidence"]["available_msat"], 10 * BTC)
        self.assertEqual(
            [action["kind"] for action in item["actions"]],
            ["import_history", "connect_wallet"],
        )
        self.assertEqual(item["actions"][0]["wallet_label"], "A")

    def test_presumed_disposals_and_unclassified_receipts_are_listed_as_assumptions(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            conn.execute("DELETE FROM transactions WHERE id IN ('out', 'in', 'later-sale')")
            conn.execute("UPDATE transactions SET kind = 'buy' WHERE id = 'buy-old'")
            _seed_transaction(
                conn, "payment", "a", "outbound", BTC, "2024-06-01T00:00:00Z", 30_000
            )
            _seed_transaction(
                conn, "sale", "a", "outbound", BTC, "2024-07-01T00:00:00Z", 30_000
            )
            conn.execute("UPDATE transactions SET kind = 'sell' WHERE id = 'sale'")
            handlers.process_journals(conn, "Books", "Book")

            summary = build_journals_quarantine_snapshot(conn, {"limit": 5})["summary"]

        outbound = summary["assumptions"]["presumed_external_outbound"]
        self.assertEqual([item["transaction_id"] for item in outbound["items"]], ["payment"])
        self.assertEqual(outbound["amount_msat"], BTC)
        inbound = summary["assumptions"]["unclassified_inbound"]
        # The explicit purchase is evidence; the kind-less receipt is presumed.
        self.assertEqual([item["transaction_id"] for item in inbound["items"]], ["buy-new"])

    def test_stale_projection_and_paging_are_reported(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            _activate_residual_component(conn)
            handlers.process_journals(conn, "Books", "Book")
            conn.execute(
                "UPDATE profiles SET journal_input_version = journal_input_version + 1"
            )
            conn.commit()

            page = build_journals_quarantine_snapshot(conn, {"limit": 1, "offset": 1})
            with self.assertRaises(AppError):
                build_journals_quarantine_snapshot(conn, {"offset": -1})

        self.assertTrue(page["summary"]["freshness"]["needs_processing"])
        self.assertEqual(page["summary"]["offset"], 1)
        self.assertEqual([item["transaction_id"] for item in page["items"]], ["later-sale"])



class QuarantineEvidenceAssetTest(unittest.TestCase):
    def test_non_bitcoin_quantities_are_not_rendered_as_msat(self):
        from kassiber.core.quarantine_review import _evidence

        row = {"wallet": "Hot", "asset": "USDT", "transaction_id": "t"}
        evidence = _evidence(
            "insufficient_lots", {"asset": "USDT", "required": 5.0, "available": 1.0}, row, {}
        )
        self.assertNotIn("required_msat", evidence)
        btc = _evidence(
            "insufficient_lots", {"asset": "BTC", "required": 0.5, "available": 0.1},
            {**row, "asset": "BTC"}, {},
        )
        self.assertEqual(btc["required_msat"], 50_000_000_000)



class QuarantineChainedRootTest(unittest.TestCase):
    def test_dependency_chains_resolve_to_their_ultimate_root(self):
        import json

        with tempfile.TemporaryDirectory() as root:
            conn = _book(root, "FIFO")
            self.addCleanup(conn.close)
            set_setting(conn, "context_workspace", "ws")
            set_setting(conn, "context_profile", "profile")
            for tx_id, reason, detail in (
                ("out", "pending_onchain_confirmation", {}),
                ("in", "transfer_pair_dependency_blocked", {"blocked_by_transaction_ids": ["out"]}),
                ("later-sale", "transfer_pair_dependency_blocked", {"blocked_by_transaction_ids": ["in"]}),
            ):
                conn.execute(
                    "INSERT INTO journal_quarantines(transaction_id, workspace_id, profile_id, "
                    "reason, detail_json, created_at) VALUES(?, 'ws', 'profile', ?, ?, 'now')",
                    (tx_id, reason, json.dumps(detail)),
                )
            conn.commit()
            snapshot = build_journals_quarantine_snapshot(conn, {"limit": 10})

        items = {item["transaction_id"]: item for item in snapshot["items"]}
        self.assertEqual(items["in"]["root"]["transaction_id"], "out")
        self.assertEqual(items["later-sale"]["root"]["transaction_id"], "out")
        self.assertNotIn("wallet_id", items["out"])
        self.assertEqual(snapshot["summary"]["group_count"], 1)


if __name__ == "__main__":
    unittest.main()
