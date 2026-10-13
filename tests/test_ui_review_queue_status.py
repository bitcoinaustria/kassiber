import queue
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber import daemon as daemon_runtime
from kassiber.core import ui_snapshot
from kassiber.core.repo import current_context_snapshot
from kassiber.core.repo.context import invalidate_journals
from kassiber.daemon import (
    _ai_chat_args,
    _bind_screen_context_metrics,
    _screen_context_for_model,
)
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
    metrics = {"review_queue_pending": 11, "review_queue_review": 0}

    def setUp(self):
        self.book = fixtures.TransactionGraphTest()
        self.book.setUp()
        self.conn = self.book.conn

    def tearDown(self):
        self.book.tearDown()

    def _page_scope(self, **dashboard_args):
        """The scope the Transactions dashboard hands the desktop, on the wire."""

        scope = ui_snapshot.build_transactions_dashboard_snapshot(
            self.conn, {"period": "all", **dashboard_args}
        )["metricsScope"]
        return {
            "workspace_id": scope["workspaceId"],
            "profile_id": scope["profileId"],
            "input_version": scope["inputVersion"],
            "wallet": scope["wallet"],
        }

    def _screen_context(self, **overrides):
        return {
            "route": "/transactions",
            "filters": {"period": "1year"},
            "metrics": dict(self.metrics),
            "metrics_scope": self._page_scope(),
            "capabilities": ["transactions"],
            **overrides,
        }

    def _bound(self, screen_context):
        validated = _ai_chat_args({**self.base, "screen_context": screen_context})
        return _bind_screen_context_metrics(
            self.conn,
            validated["screen_context"],
            current_context_snapshot(self.conn),
        )

    def test_the_dashboard_names_the_book_state_its_counts_come_from(self):
        self.assertEqual(
            self._page_scope(),
            {
                "workspace_id": "ws-1",
                "profile_id": "profile-1",
                "input_version": 0,
                "wallet": None,
            },
        )
        # The page's filter is echoed as the one wallet it resolved to.
        self.assertEqual(self._page_scope(wallet="Cold")["wallet"], "wallet-a")

    def test_wallet_scoped_counts_name_their_wallet_to_the_model(self):
        context = self._bound(
            self._screen_context(metrics_scope=self._page_scope(wallet="cold"))
        )
        self.assertEqual(
            context["metrics_scope"],
            {"wallet": {"id": "wallet-a", "label": "Cold"}},
        )
        text = _screen_context_for_model(context)
        self.assertIn('"metrics_scope":{"wallet":{"id":"wallet-a","label":"Cold"}}', text)
        self.assertIn("metrics_scope.wallet", text)
        # The wallet's configuration never rides along.
        self.assertNotIn("config", text)

        book_wide = self._bound(self._screen_context())
        self.assertEqual(book_wide["metrics_scope"], {"wallet": None})

    def _seed_label_shadowing_another_wallets_id(self):
        # Wallet B's label is wallet A's id, so the filter "wallet-a" selects
        # both wallets on the page.
        self.conn.execute("UPDATE wallets SET label = 'wallet-a' WHERE id = 'wallet-b'")
        self.book._tx("pending-a", "wallet-a", "inbound", 1_000_000, "p-a", {})
        self.book._tx("pending-b", "wallet-b", "inbound", 1_000_000, "p-b", {})
        self.conn.commit()

    def test_an_ambiguous_wallet_filter_sends_no_counts(self):
        self._seed_label_shadowing_another_wallets_id()
        resolved = ui_snapshot.resolve_ui_wallet_filter(
            self.conn, "profile-1", "wallet-a"
        )
        self.assertEqual(sorted(w["id"] for w in resolved), ["wallet-a", "wallet-b"])

        dashboard = ui_snapshot.build_transactions_dashboard_snapshot(
            self.conn, {"period": "all", "wallet": "wallet-a"}
        )
        listed = ui_snapshot.build_transactions_snapshot(
            self.conn, {"wallet": "wallet-a", "quick": "review_queue"}
        )
        # The page counts what its own wallet filter selects, both wallets...
        self.assertEqual(dashboard["summary"]["pending"], 2)
        self.assertEqual(listed["count"], 2)
        # ...so it cannot name one wallet for the assistant.
        self.assertIsNone(dashboard["metricsScope"])

        # A scope naming wallet A does not bind while "wallet-a" also selects
        # wallet B; the counts are dropped instead of credited to wallet A.
        scope = {
            "workspace_id": "ws-1",
            "profile_id": "profile-1",
            "input_version": 0,
            "wallet": "wallet-a",
        }
        context = self._bound(self._screen_context(metrics_scope=scope))
        self.assertNotIn("metrics", context)

    def test_the_scope_names_a_wallet_by_its_resolved_id(self):
        scope = {**self._page_scope(), "wallet": "Cold"}
        context = self._bound(self._screen_context(metrics_scope=scope))
        self.assertNotIn("metrics", context)

    def test_a_wallet_outside_the_book_drops_the_counts(self):
        scope = self._page_scope()
        for wallet in ("Missing wallet", "wallet-z"):
            with self.subTest(wallet=wallet):
                context = self._bound(
                    self._screen_context(metrics_scope={**scope, "wallet": wallet})
                )
                self.assertNotIn("metrics", context)
                self.assertNotIn("metrics_scope", context)

    def test_current_page_metrics_reach_the_model_as_what_the_screen_showed(self):
        context = self._bound(self._screen_context())
        self.assertEqual(context["metrics"], self.metrics)
        self.assertEqual(context["metrics_scope"], {"wallet": None})
        text = _screen_context_for_model(context)
        self.assertIn('"review_queue_pending":11', text)
        self.assertIn("confirmation timestamp", text)
        self.assertNotIn("incoming_count", text)
        # Staleness is a legitimate explanation; a reconciliation is not owed.
        self.assertIn("the screen may be stale", text)
        self.assertIn("Never invent a reconciliation", text)
        self.assertNotIn("rather than calling the screen stale", text)
        # Binding ids are checked, not shown to the model.
        self.assertNotIn("profile-1", text)

    def test_metrics_from_another_book_or_older_inputs_are_dropped(self):
        scope = self._page_scope()
        stale_scopes = {
            "other profile": {**scope, "profile_id": "profile-2"},
            "other workspace": {**scope, "workspace_id": "ws-2"},
            "older inputs": {**scope, "input_version": scope["input_version"] + 1},
        }
        for name, stale in stale_scopes.items():
            with self.subTest(name):
                context = self._bound(self._screen_context(metrics_scope=stale))
                self.assertNotIn("metrics", context)
                self.assertNotIn("metrics_scope", context)
                self.assertEqual(context["route"], "/transactions")
                self.assertNotIn("Definitions", _screen_context_for_model(context))

        # A sync or edit after the page loaded bumps the input version.
        page_scope = self._page_scope()
        invalidate_journals(self.conn, "profile-1")
        self.conn.commit()
        context = self._bound(self._screen_context(metrics_scope=page_scope))
        self.assertNotIn("metrics", context)

    def _started_screen_context(self, screen_context, request_id="chat-1"):
        """Send ai.chat in-process and return the screen context the turn got."""

        started = {}

        class _CapturedThread:
            def __init__(self, *, target, args, **_kwargs):
                started["validated"] = args[2]

            def start(self):
                pass

        ctx = daemon_runtime.DaemonContext(
            conn=self.conn,
            data_root=str(Path(self.book.tmp.name) / "data"),
            runtime_config={},
            active_ai_chats=daemon_runtime.ActiveAiChats(),
            main_thread_tasks=queue.Queue(),
            auth_backoff=daemon_runtime.AuthAttemptBackoff(),
            input_lines=queue.Queue(),
            deferred_input_lines=[],
            out=None,
            freshness_stop_event=threading.Event(),
        )
        with patch.object(
            daemon_runtime, "resolve_ai_provider",
            return_value={"name": "local", "base_url": "http://127.0.0.1:1/v1",
                          "kind": "local"},
        ), patch.object(daemon_runtime, "require_ai_provider_contactable"), patch.object(
            daemon_runtime, "_resolve_ai_provider_api_key", return_value=None
        ), patch.object(daemon_runtime.threading, "Thread", _CapturedThread):
            daemon_runtime.handle_request(
                ctx,
                {
                    "request_id": request_id,
                    "kind": "ai.chat",
                    "args": {
                        **self.base,
                        "provider": "local",
                        "screen_context": screen_context,
                    },
                },
                None,
            )
        return started["validated"]["screen_context"]

    def test_the_chat_request_binds_metrics_before_the_turn_starts(self):
        stale = {**self._page_scope(), "input_version": 7}
        for scope, expect_metrics in ((self._page_scope(), True), (stale, False)):
            with self.subTest(expect_metrics=expect_metrics):
                context = self._started_screen_context(
                    self._screen_context(metrics_scope=scope),
                    request_id=f"chat-{expect_metrics}",
                )
                self.assertEqual("metrics" in context, expect_metrics)

    def test_a_pinned_transaction_from_another_book_is_not_named(self):
        self.book._tx("tx-here", "wallet-a", "inbound", 1_000_000, "a" * 64, {})
        self.conn.commit()
        pinned = {
            "route": "/transactions",
            "entity_type": "transaction",
            "capabilities": ["transactions"],
        }
        for entity_id, expect_kept in (
            ("tx-here", True),
            ("A" * 64, True),  # its txid, any case
            ("tx-previous-book", False),
        ):
            with self.subTest(entity_id=entity_id):
                context = self._started_screen_context(
                    {**pinned, "entity_id": entity_id}
                )
                self.assertEqual(context.get("entity_id") == entity_id, expect_kept)
                if not expect_kept:
                    self.assertNotIn("entity_type", context)
                    self.assertNotIn(
                        entity_id, _screen_context_for_model(context)
                    )
                    self.assertEqual(context["route"], "/transactions")

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
                    {**self.base, "screen_context": self._screen_context(metrics=metrics)}
                )
            self.assertEqual(raised.exception.code, "validation")

    def test_metrics_need_a_well_formed_scope(self):
        scope = self._page_scope()
        for name, context in {
            "missing scope": {"route": "/transactions", "metrics": dict(self.metrics)},
            "scope without metrics": {"route": "/transactions", "metrics_scope": scope},
            "missing version": self._screen_context(
                metrics_scope={k: v for k, v in scope.items() if k != "input_version"}
            ),
            "boolean version": self._screen_context(
                metrics_scope={**scope, "input_version": True}
            ),
            "path as id": self._screen_context(
                metrics_scope={**scope, "profile_id": "/home/user/book"}
            ),
            "extra field": self._screen_context(
                metrics_scope={**scope, "descriptor": "wpkh(xpub)"}
            ),
            "descriptor as wallet": self._screen_context(
                metrics_scope={
                    **scope,
                    "wallet": "wpkh([d34db33f/84h/0h/0h]xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz/0/*)",
                }
            ),
            "path as wallet": self._screen_context(
                metrics_scope={**scope, "wallet": "~/wallets/cold.json"}
            ),
            "non-string wallet": self._screen_context(
                metrics_scope={**scope, "wallet": 3}
            ),
        }.items():
            with self.subTest(name), self.assertRaises(AppError) as raised:
                _ai_chat_args({**self.base, "screen_context": context})
            self.assertEqual(raised.exception.code, "validation")


if __name__ == "__main__":
    unittest.main()
