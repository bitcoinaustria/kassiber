import unittest

from kassiber.core import ui_snapshot
from tests import test_transaction_graph as fixtures
from tests.custody_projection_fixtures import insert_reviewed_projection


class ExactTransactionDetailTests(unittest.TestCase):
    def test_exact_wallet_leg_keeps_principal_while_list_retains_net_pair(self):
        book = fixtures.TransactionGraphTest()
        book.setUp()
        self.addCleanup(book.tearDown)
        txid = "f" * 64
        book._tx("out", "wallet-a", "outbound", 600000000, txid, {}, fee_msat=10000000)
        book._tx("in", "wallet-b", "inbound", 600000000, txid, {})
        insert_reviewed_projection(
            book.conn, projection_id="a" * 64, workspace_id="ws-1", profile_id="profile-1",
            source_transaction_id="out", target_transaction_id="in",
            source_asset="BTC", target_asset="BTC", source_amount_msat=600000000,
            target_amount_msat=600000000, review_kind="manual_bridge", relation_kind="move",
            occurred_at=fixtures.NOW, target_occurred_at=fixtures.NOW,
        )
        book.conn.execute("UPDATE profiles SET last_processed_at=?, last_processed_tx_count=2, last_processed_input_version=journal_input_version WHERE id='profile-1'", (fixtures.NOW,))
        before = ui_snapshot.build_transactions_snapshot(book.conn)
        activity_before = ui_snapshot._activity_transactions(book.conn, "profile-1", has_book_state=True)
        self.assertEqual(len(before["txs"]), 1)
        self.assertEqual(before["txs"][0]["amountSat"], 0)
        changes = book.conn.total_changes

        incoming = ui_snapshot.build_transactions_resolve_snapshot(book.conn, {"query": "in"})["transaction"]
        outgoing = ui_snapshot.build_transactions_resolve_snapshot(book.conn, {"query": "out"})["transaction"]

        self.assertEqual(incoming["id"], "in")
        self.assertEqual(incoming["amountSat"], 600000)
        self.assertEqual(incoming["feeSat"], 0)
        self.assertEqual(incoming["account"], "Hot")
        self.assertNotIn("fee", incoming["counter"])
        self.assertEqual(incoming["pair"]["inAmountSat"], 600000)
        self.assertEqual(outgoing["id"], "out")
        self.assertEqual(outgoing["amountSat"], -600000)
        self.assertEqual(outgoing["feeSat"], 10000)
        self.assertEqual(outgoing["account"], "Cold")
        self.assertEqual(book.conn.total_changes, changes)
        self.assertEqual(ui_snapshot.build_transactions_snapshot(book.conn), before)
        self.assertEqual(ui_snapshot._activity_transactions(book.conn, "profile-1", has_book_state=True), activity_before)
