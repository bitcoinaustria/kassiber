import sqlite3
import unittest

from kassiber.core.ui_snapshot import _wallet_month_balances

BTC_MSAT = 100_000_000_000


def _conn(rows):
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.execute(
        "CREATE TABLE transactions ("
        " id TEXT PRIMARY KEY, profile_id TEXT, wallet_id TEXT, asset TEXT,"
        " excluded INTEGER NOT NULL DEFAULT 0, occurred_at TEXT, created_at TEXT,"
        " direction TEXT, amount INTEGER, fee INTEGER)"
    )
    conn.executemany(
        "INSERT INTO transactions VALUES (?, 'pf-1', ?, ?, ?, ?, ?, ?, ?, ?)",
        rows,
    )
    return conn


class WalletMonthBalancesTest(unittest.TestCase):
    def test_month_end_balances_for_months_a_wallet_moved(self):
        conn = _conn(
            [
                ("t1", "w-1", "BTC", 0, "2024-05-01T00:00:00Z", "a", "inbound", 2 * BTC_MSAT, 0),
                ("t2", "w-1", "BTC", 0, "2024-05-20T00:00:00Z", "b", "inbound", BTC_MSAT, 0),
                ("t3", "w-1", "BTC", 0, "2026-09-10T00:00:00Z", "c", "outbound", BTC_MSAT // 2, 1_000_000),
                ("t4", "w-2", "LBTC", 0, "2026-11-02T00:00:00Z", "d", "inbound", BTC_MSAT // 4, 0),
                ("t5", "w-2", "BTC", 1, "2026-11-03T00:00:00Z", "e", "inbound", BTC_MSAT, 0),
                ("t6", "w-3", "USDT", 0, "2026-11-03T00:00:00Z", "f", "inbound", BTC_MSAT, 0),
                ("t7", "w-4", "BTC", 0, "not a date", "g", "inbound", BTC_MSAT, 0),
            ]
        )
        balances = _wallet_month_balances(conn, "pf-1")
        self.assertEqual(
            balances["w-1"],
            [
                {"month": "2024-05", "balanceBtc": 3.0},
                {"month": "2026-09", "balanceBtc": 2.49999},
            ],
        )
        # Liquid bitcoin counts; excluded rows and other assets do not.
        self.assertEqual(balances["w-2"], [{"month": "2026-11", "balanceBtc": 0.25}])
        self.assertNotIn("w-3", balances)
        self.assertNotIn("w-4", balances)

    def test_no_transactions_means_no_points(self):
        self.assertEqual(_wallet_month_balances(_conn([]), "pf-1"), {})


if __name__ == "__main__":
    unittest.main()
