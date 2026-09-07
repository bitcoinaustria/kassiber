import unittest

from kassiber.core import ui_snapshot
from tests import test_transaction_graph as fixtures


class TransactionPaymentMethodTests(unittest.TestCase):
    def test_canonical_method_matches_filter_and_all_snapshot_surfaces(self):
        for kind, label, expected in (
            ("strike", "Strike", "Exchange"),
            ("strike", "Lightning savings", "Exchange"),
            ("strike", "Liquid exchange", "Exchange"),
            ("binance", "Savings", "Exchange"),
            ("21bitcoin", "Savings", "Exchange"),
            ("pocketbitcoin", "Savings", "Exchange"),
            ("lnd", "Exchange account", "Lightning"),
        ):
            with self.subTest(kind=kind, label=label):
                book = fixtures.TransactionGraphTest()
                book.setUp()
                try:
                    book.conn.execute("UPDATE wallets SET kind=?, label=? WHERE id='wallet-a'", (kind, label))
                    book._tx("sale", "wallet-a", "outbound", 1000000, "strike-trade", {}, kind="sell")
                    page = ui_snapshot.build_transactions_snapshot(book.conn, {"paymentMethod": expected})
                    self.assertEqual(page["count"], 1)
                    self.assertEqual(page["txs"][0]["paymentMethod"], expected)
                    resolved = ui_snapshot.build_transactions_resolve_snapshot(book.conn, {"query": "sale"})
                    self.assertEqual(resolved["transaction"]["paymentMethod"], expected)
                    self.assertEqual(ui_snapshot._transactions(book.conn, "profile-1")[0]["paymentMethod"], expected)
                    activity = ui_snapshot._activity_transactions(book.conn, "profile-1", has_book_state=False)
                    self.assertEqual(activity[0]["paymentMethod"], expected)
                    dashboard = ui_snapshot.build_transactions_dashboard_snapshot(book.conn, {"period": "all"})
                    self.assertEqual(dashboard["history"]["paymentMethods"], [expected])
                finally:
                    book.tearDown()
