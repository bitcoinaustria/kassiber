"""UI capital-gains snapshot should include income like CLI report_capital_gains."""

from __future__ import annotations

import sqlite3
import unittest
from unittest.mock import patch

from kassiber.core import ui_snapshot


class CapitalGainsSnapshotParityTests(unittest.TestCase):
    def _conn(self, *, tax_country: str = "generic", gains_algorithm: str = "fifo"):
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.executescript(
            """
            CREATE TABLE profiles (
              id TEXT PRIMARY KEY, workspace_id TEXT, label TEXT,
              tax_country TEXT, fiat_currency TEXT, gains_algorithm TEXT,
              tax_long_term_days INTEGER,
              last_processed_at TEXT, last_processed_tx_count INTEGER,
              last_processed_input_version TEXT
            );
            CREATE TABLE workspaces (id TEXT PRIMARY KEY, label TEXT);
            CREATE TABLE wallets (
              id TEXT PRIMARY KEY, profile_id TEXT, label TEXT
            );
            CREATE TABLE transactions (
              id TEXT PRIMARY KEY, profile_id TEXT, wallet_id TEXT, excluded INTEGER,
              taxability_override INTEGER, occurred_at TEXT, asset TEXT, amount INTEGER
            );
            CREATE TABLE journal_entries (
              id TEXT PRIMARY KEY, profile_id TEXT, transaction_id TEXT,
              entry_type TEXT, occurred_at TEXT, quantity INTEGER,
              cost_basis REAL, proceeds REAL, gain_loss REAL,
              at_category TEXT, at_kennzahl INTEGER, capital_gains_type TEXT,
              created_at TEXT
            );
            CREATE TABLE journal_quarantines (profile_id TEXT);
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
            CREATE TABLE journal_custody_projection_relations (
              id TEXT, profile_id TEXT, relation_kind TEXT,
              out_transaction_id TEXT, in_transaction_id TEXT,
              kind TEXT, policy TEXT, swap_fee_msat INTEGER,
              swap_fee_kind TEXT, out_asset TEXT,
              out_amount INTEGER, in_asset TEXT,
              in_amount INTEGER, target_occurred_at TEXT
            );
            """
        )
        conn.execute("INSERT INTO workspaces VALUES ('ws1', 'Main')")
        conn.execute(
            "INSERT INTO profiles VALUES (?,?,?,?,?,?,?,?,?,?)",
            (
                "p1",
                "ws1",
                "Default",
                tax_country,
                "EUR",
                gains_algorithm,
                365,
                "now",
                1,
                "v1",
            ),
        )
        conn.execute("INSERT INTO settings VALUES ('context_workspace', 'ws1')")
        conn.execute("INSERT INTO settings VALUES ('context_profile', 'p1')")
        return conn

    def test_income_entries_appear_in_lots(self):
        conn = self._conn()
        conn.execute("INSERT INTO wallets VALUES ('w1', 'p1', 'Hot')")
        conn.execute(
            "INSERT INTO transactions VALUES (?,?,?,?,?,?,?,?)",
            ("t-income", "p1", "w1", 0, 1, "2026-03-01T00:00:00Z", "BTC", 100_000_000_000),
        )
        conn.execute(
            "INSERT INTO transactions VALUES (?,?,?,?,?,?,?,?)",
            ("t-sell", "p1", "w1", 0, 1, "2026-04-01T00:00:00Z", "BTC", 50_000_000_000),
        )
        conn.execute(
            "INSERT INTO journal_entries VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (
                "j-income",
                "p1",
                "t-income",
                "income",
                "2026-03-01T00:00:00Z",
                100_000_000_000,
                0.0,
                500.0,
                500.0,
                None,
                None,
                "short",
                "2026-03-01T00:00:00Z",
            ),
        )
        conn.execute(
            "INSERT INTO journal_entries VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (
                "j-sell",
                "p1",
                "t-sell",
                "disposal",
                "2026-04-01T00:00:00Z",
                -50_000_000_000,
                40.0,
                90.0,
                50.0,
                None,
                None,
                "short",
                "2026-04-01T00:00:00Z",
            ),
        )

        with patch.object(
            ui_snapshot,
            "_journal_freshness",
            return_value={"needs_processing": False},
        ):
            snapshot = ui_snapshot.build_capital_gains_snapshot(conn, tax_year=2026)

        self.assertEqual(len(snapshot["lots"]), 2)
        proceeds = sorted(lot["proceedsEur"] for lot in snapshot["lots"])
        self.assertEqual(proceeds, [90.0, 500.0])

    def test_neu_swap_only_year_is_not_selected_as_default(self):
        conn = self._conn(tax_country="at", gains_algorithm="moving_average_at")
        conn.execute("INSERT INTO wallets VALUES ('w1', 'p1', 'Hot')")
        conn.execute(
            "INSERT INTO transactions VALUES (?,?,?,?,?,?,?,?)",
            ("t-swap", "p1", "w1", 0, 1, "2025-06-01T00:00:00Z", "BTC", 100_000_000_000),
        )
        conn.execute(
            "INSERT INTO journal_entries VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (
                "j-swap",
                "p1",
                "t-swap",
                "disposal",
                "2025-06-01T00:00:00Z",
                -100_000_000_000,
                10.0,
                10.0,
                0.0,
                "neu_swap",
                None,
                "short",
                "2025-06-01T00:00:00Z",
            ),
        )

        with patch.object(
            ui_snapshot,
            "_journal_freshness",
            return_value={"needs_processing": False},
        ), patch.object(
            ui_snapshot,
            "_austrian_kennzahl_snapshot_rows",
            return_value=[],
        ):
            snapshot = ui_snapshot.build_capital_gains_snapshot(conn)

        self.assertEqual(snapshot["lots"], [])
        # Transaction year still appears via transaction years merge, but
        # primary taxable years must not treat neu_swap-only as reportable.
        primary = ui_snapshot._capital_gains_available_years(
            conn, "p1", primary_only=True, use_vienna_year=True
        )
        self.assertEqual(primary, [])

    def test_austrian_lots_display_vienna_local_date(self):
        conn = self._conn(tax_country="at", gains_algorithm="moving_average_at")
        conn.executemany(
            "INSERT INTO wallets VALUES (?, 'p1', ?)",
            [("w1", "Hot"), ("w2", "Cold")],
        )
        occurred_at = "2024-12-31T23:30:00Z"
        conn.executemany(
            "INSERT INTO transactions VALUES (?,?,?,?,?,?,?,?)",
            [
                ("t-sell", "p1", "w1", 0, 1, occurred_at, "BTC", 50_000_000_000),
                ("t-swap-out", "p1", "w1", 0, 1, occurred_at, "BTC", 25_000_000_000),
                ("t-swap-in", "p1", "w2", 0, 1, occurred_at, "LBTC", 24_900_000_000),
            ],
        )
        conn.executemany(
            "INSERT INTO journal_entries VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            [
                (
                    "j-sell",
                    "p1",
                    "t-sell",
                    "disposal",
                    occurred_at,
                    -50_000_000_000,
                    40.0,
                    90.0,
                    50.0,
                    "neu_gain",
                    174,
                    "short",
                    occurred_at,
                ),
                (
                    "j-swap",
                    "p1",
                    "t-swap-out",
                    "disposal",
                    occurred_at,
                    -25_000_000_000,
                    20.0,
                    20.0,
                    0.0,
                    "neu_swap",
                    None,
                    "short",
                    occurred_at,
                ),
            ],
        )
        conn.execute(
            "INSERT INTO journal_custody_projection_relations VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (
                "pair-1",
                "p1",
                "conversion",
                "t-swap-out",
                "t-swap-in",
                "peg-out",
                "carrying-value",
                100_000_000,
                "network",
                "BTC",
                25_000_000_000,
                "LBTC",
                24_900_000_000,
                occurred_at,
            ),
        )

        with patch.object(
            ui_snapshot,
            "_journal_freshness",
            return_value={"needs_processing": False},
        ), patch.object(
            ui_snapshot,
            "_austrian_kennzahl_snapshot_rows",
            return_value=[],
        ):
            snapshot = ui_snapshot.build_capital_gains_snapshot(conn, tax_year=2025)

        self.assertEqual(snapshot["lots"][0]["disposed"], "2025-01-01")
        self.assertEqual(snapshot["neutralSwapLots"][0]["date"], "2025-01-01")


if __name__ == "__main__":
    unittest.main()
