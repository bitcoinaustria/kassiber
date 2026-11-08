import sqlite3
import unittest
from datetime import date

from kassiber.core.ui_snapshot import _wallet_balance_trends

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


class WalletBalanceTrendTest(unittest.TestCase):
    def test_month_end_balances_carry_history_before_the_window(self):
        conn = _conn(
            [
                ("t1", "w-1", "BTC", 0, "2024-05-01T00:00:00Z", "a", "inbound", 2 * BTC_MSAT, 0),
                ("t2", "w-1", "BTC", 0, "2026-09-10T00:00:00Z", "b", "outbound", BTC_MSAT // 2, 1_000_000),
                ("t3", "w-2", "LBTC", 0, "2026-11-02T00:00:00Z", "c", "inbound", BTC_MSAT // 4, 0),
                ("t4", "w-2", "BTC", 1, "2026-11-03T00:00:00Z", "d", "inbound", BTC_MSAT, 0),
                ("t5", "w-3", "USDT", 0, "2026-11-03T00:00:00Z", "e", "inbound", BTC_MSAT, 0),
            ]
        )
        trends = _wallet_balance_trends(conn, "pf-1", months=3, today=date(2026, 11, 7))
        self.assertEqual(
            [point["month"] for point in trends["w-1"]],
            ["2026-09", "2026-10", "2026-11"],
        )
        self.assertEqual(
            [point["balanceBtc"] for point in trends["w-1"]],
            [1.49999, 1.49999, 1.49999],
        )
        self.assertEqual(
            [point["balanceBtc"] for point in trends["w-2"]],
            [0.0, 0.0, 0.25],
        )
        self.assertNotIn("w-3", trends)

    def test_no_transactions_means_no_trend(self):
        self.assertEqual(_wallet_balance_trends(_conn([]), "pf-1"), {})


if __name__ == "__main__":
    unittest.main()
