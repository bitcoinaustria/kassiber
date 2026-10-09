import unittest

from kassiber.core import ui_snapshot
from kassiber.daemon import _ai_chat_args, _screen_context_for_model
from kassiber.errors import AppError
from tests import test_transaction_graph as fixtures

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
