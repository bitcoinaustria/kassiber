import unittest

from kassiber.core import ui_snapshot
from kassiber.daemon import _ai_chat_args, _screen_context_for_model
from kassiber.errors import AppError
from tests import test_transaction_graph as fixtures
from tests.custody_projection_fixtures import insert_reviewed_projection

CONFIRMED_AT = "2026-01-02T00:00:00Z"


class ReviewQueueStatusTests(unittest.TestCase):
    """The Review queue card, its filter, and the assistant share one definition."""

    def setUp(self):
        self.book = fixtures.TransactionGraphTest()
        self.book.setUp()
        self.conn = self.book.conn

    def tearDown(self):
        self.book.tearDown()

    def _seed(self):
        tx = self.book._tx
        # Imported without a confirmation time: pending, not a review item.
        tx("ledger-row", "wallet-a", "inbound", 100_000_000, "ledger-1", {})
        tx("confirmed", "wallet-a", "inbound", 200_000_000, "chain-1", {})
        tx("needs-review", "wallet-a", "outbound", 50_000_000, "chain-2", {})
        tx("quarantined", "wallet-a", "outbound", 10_000_000, "chain-3", {})
        tx("failed", "wallet-a", "inbound", 1_000_000, "chain-4", {})
        self.conn.execute(
            "UPDATE transactions SET confirmed_at = ? WHERE id <> 'ledger-row'",
            (CONFIRMED_AT,),
        )
        self.conn.execute(
            "UPDATE transactions SET review_status = 'needs-review' WHERE id = 'needs-review'"
        )
        self.conn.execute(
            "UPDATE transactions SET review_status = 'failed' WHERE id = 'failed'"
        )
        self.conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES('quarantined', 'ws-1', 'profile-1', 'missing_price', '{}', ?)
            """,
            (CONFIRMED_AT,),
        )
        self.conn.commit()

    def test_card_filter_and_health_count_the_same_rows(self):
        self._seed()
        dashboard = ui_snapshot.build_transactions_dashboard_snapshot(
            self.conn, {"period": "all"}
        )
        summary = dashboard["summary"]
        self.assertEqual(
            (summary["review"], summary["pending"], summary["failed"]), (2, 1, 1)
        )

        queue = ui_snapshot.transaction_review_queue_counts(self.conn, "profile-1")
        self.assertEqual(
            (queue["review"], queue["pending"], queue["failed"], queue["total"]),
            (2, 1, 1, 4),
        )
        self.assertIn("confirmation timestamp", queue["definitions"]["pending"])

        filtered = ui_snapshot.build_transactions_snapshot(
            self.conn, {"quick": "review_queue"}
        )
        self.assertEqual(
            sorted(row["id"] for row in filtered["txs"]),
            ["failed", "ledger-row", "needs-review", "quarantined"],
        )

        health = ui_snapshot.build_workspace_health_snapshot(self.conn)
        self.assertEqual(health["review_queue"]["pending"], 1)
        self.assertEqual(health["review_queue"]["review"], 2)

    def _seed_unconfirmed_reviewed_transfer(self):
        tx = self.book._tx
        # Both legs imported without a confirmation time, then reviewed as one
        # transfer from Cold to Hot.
        tx("transfer-out", "wallet-a", "outbound", 100_000_000, "move-1", {})
        tx("transfer-in", "wallet-b", "inbound", 100_000_000, "move-2", {})
        insert_reviewed_projection(
            self.conn,
            projection_id="f" * 64,
            workspace_id="ws-1",
            profile_id="profile-1",
            source_transaction_id="transfer-out",
            target_transaction_id="transfer-in",
            source_asset="BTC",
            target_asset="BTC",
            source_amount_msat=100_000_000,
            target_amount_msat=100_000_000,
            review_kind="transfer",
            occurred_at=fixtures.NOW,
            target_occurred_at=fixtures.NOW,
            relation_kind="move",
        )
        # Journals are current, so the reviewed pair is one item everywhere.
        self.conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = ?,
                last_processed_tx_count = 2,
                last_processed_input_version = journal_input_version
            WHERE id = 'profile-1'
            """,
            (CONFIRMED_AT,),
        )
        self.conn.commit()

    def test_a_reviewed_pair_counts_once_in_the_card_list_and_health(self):
        self._seed_unconfirmed_reviewed_transfer()

        dashboard = ui_snapshot.build_transactions_dashboard_snapshot(
            self.conn, {"period": "all"}
        )
        health = ui_snapshot.build_workspace_health_snapshot(self.conn)
        listed = ui_snapshot.build_transactions_snapshot(
            self.conn, {"quick": "review_queue"}
        )
        self.assertEqual(dashboard["summary"]["pending"], 1)
        self.assertEqual(health["review_queue"]["pending"], 1)
        self.assertEqual(health["review_queue"]["total"], 1)
        self.assertEqual(listed["count"], 1)

        # A wallet-scoped card counts the leg in that wallet, still once.
        for wallet in ("Cold", "Hot"):
            with self.subTest(wallet=wallet):
                scoped = ui_snapshot.build_transactions_dashboard_snapshot(
                    self.conn, {"period": "all", "wallet": wallet}
                )
                self.assertEqual(scoped["summary"]["pending"], 1)

    def test_a_pair_takes_its_most_urgent_leg_status(self):
        self._seed_unconfirmed_reviewed_transfer()
        self.conn.execute(
            "UPDATE transactions SET review_status = 'needs_review' WHERE id = 'transfer-out'"
        )
        self.conn.commit()

        queue = ui_snapshot.transaction_review_queue_counts(self.conn, "profile-1")
        self.assertEqual(
            (queue["review"], queue["pending"], queue["failed"], queue["total"]),
            (1, 0, 0, 1),
        )
        dashboard = ui_snapshot.build_transactions_dashboard_snapshot(
            self.conn, {"period": "all"}
        )
        self.assertEqual(
            (dashboard["summary"]["review"], dashboard["summary"]["pending"]), (1, 0)
        )
        listed = ui_snapshot.build_transactions_snapshot(
            self.conn, {"quick": "review_queue"}
        )
        self.assertEqual(listed["count"], 1)

    def test_python_status_matches_the_sql_definition(self):
        cases = {
            ("review", False, True): "review",
            ("blocked", False, False): "review",
            (None, True, True): "review",
            ("error", False, True): "failed",
            ("complete", False, False): "completed",
            ("pending", False, True): "completed",
            (None, False, False): "pending",
            (None, False, True): "completed",
        }
        for (status, quarantined, confirmed), expected in cases.items():
            with self.subTest(status=status, quarantined=quarantined, confirmed=confirmed):
                self.assertEqual(
                    ui_snapshot.ui_transaction_status(
                        status, quarantined=quarantined, confirmed=confirmed
                    ),
                    expected,
                )


class ScreenContextMetricsTests(unittest.TestCase):
    base = {
        "model": "local-model",
        "messages": [{"role": "user", "content": "Why 11 pending?"}],
        "tools_enabled": True,
    }

    def test_known_page_metrics_reach_the_model_with_definitions(self):
        validated = _ai_chat_args(
            {
                **self.base,
                "screen_context": {
                    "route": "/transactions",
                    "filters": {"period": "1year"},
                    "metrics": {"review_queue_pending": 11, "review_queue_review": 0},
                    "capabilities": ["transactions"],
                },
            }
        )
        context = validated["screen_context"]
        self.assertEqual(
            context["metrics"], {"review_queue_pending": 11, "review_queue_review": 0}
        )
        text = _screen_context_for_model(context)
        self.assertIn('"review_queue_pending":11', text)
        self.assertIn("confirmation timestamp", text)
        self.assertNotIn("incoming_count", text)

    def test_unknown_or_non_count_metrics_are_rejected(self):
        for metrics in (
            {"wallet_balance_btc": 1},
            {"review_queue_pending": True},
            {"review_queue_pending": -1},
            {"review_queue_pending": "11"},
            {"review_queue_pending": 1.5},
            ["review_queue_pending"],
        ):
            with self.subTest(metrics=metrics), self.assertRaises(AppError) as raised:
                _ai_chat_args(
                    {
                        **self.base,
                        "screen_context": {"route": "/transactions", "metrics": metrics},
                    }
                )
            self.assertEqual(raised.exception.code, "validation")


if __name__ == "__main__":
    unittest.main()
