"""UI capital-gains snapshot should include income like CLI report_capital_gains."""

from __future__ import annotations

import sqlite3
import unittest
from unittest.mock import patch

from kassiber.core import ui_snapshot
from kassiber.db import DATABASE_INSTANCE_ID_SETTING


class CapitalGainsSnapshotParityTests(unittest.TestCase):
    def test_long_term_engine_label_is_presented_as_long_term(self):
        conn = self._conn()
        try:
            conn.execute("INSERT INTO wallets VALUES ('w1', 'p1', 'Hot')")
            conn.execute("INSERT INTO transactions VALUES ('t','p1','w1',0,1,'2026-01-01T00:00:00Z','BTC',1000)")
            conn.execute("INSERT INTO journal_entries VALUES ('j','p1','t','disposal','2026-01-01T00:00:00Z',-1000,1,2,1,NULL,NULL,'long_term','now')")
            with patch.object(ui_snapshot, "_journal_freshness", return_value={"needs_processing": False}):
                result = ui_snapshot.build_capital_gains_snapshot(conn, tax_year=2026)
            self.assertEqual(result["lots"][0]["type"], "LT")
        finally:
            conn.close()

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
              last_processed_input_version TEXT, journal_input_version INTEGER DEFAULT 1
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
              created_at TEXT, asset TEXT NOT NULL DEFAULT 'BTC'
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
            "INSERT INTO profiles VALUES (?,?,?,?,?,?,?,?,?,?,?)",
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
                "1",
                1,
            ),
        )
        conn.execute("INSERT INTO settings VALUES (?, ?)", (DATABASE_INSTANCE_ID_SETTING, "1" * 32))
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
            "INSERT INTO journal_entries (id, profile_id, transaction_id, entry_type, "
            "occurred_at, quantity, cost_basis, proceeds, gain_loss, at_category, "
            "at_kennzahl, capital_gains_type, created_at) "
            "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
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
            "INSERT INTO journal_entries (id, profile_id, transaction_id, entry_type, "
            "occurred_at, quantity, cost_basis, proceeds, gain_loss, at_category, "
            "at_kennzahl, capital_gains_type, created_at) "
            "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
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

    def _insert_entry(self, conn, entry_id, asset, entry_type, occurred_at, quantity,
                      cost_basis, proceeds, gain_loss):
        conn.execute(
            "INSERT INTO transactions VALUES (?,?,?,?,?,?,?,?)",
            (f"t-{entry_id}", "p1", "w1", 0, 1, occurred_at, asset, abs(quantity)),
        )
        conn.execute(
            "INSERT INTO journal_entries (id, profile_id, transaction_id, entry_type, "
            "occurred_at, quantity, cost_basis, proceeds, gain_loss, at_category, "
            "at_kennzahl, capital_gains_type, created_at, asset) "
            "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (entry_id, "p1", f"t-{entry_id}", entry_type, occurred_at, quantity,
             cost_basis, proceeds, gain_loss, None, None, "short", occurred_at, asset),
        )

    def test_rows_trace_to_transactions_and_totals_cover_the_year(self):
        from kassiber import daemon

        conn = self._conn()
        conn.execute("INSERT INTO wallets VALUES ('w1', 'p1', 'Hot')")
        self._insert_entry(conn, "j1", "BTC", "disposal", "2026-02-01T00:00:00Z",
                           -10_000_000_000, 100.0, 150.1, 50.1)
        self._insert_entry(conn, "j2", "BTC", "disposal", "2026-05-01T00:00:00Z",
                           -20_000_000_000, 300.0, 250.0, -50.0)
        self._insert_entry(conn, "j3", "LBTC", "income", "2026-06-01T00:00:00Z",
                           5_000_000_000, 0.0, 70.2, 70.2)

        with patch.object(
            ui_snapshot, "_journal_freshness", return_value={"needs_processing": False}
        ):
            snapshot = ui_snapshot.build_capital_gains_snapshot(conn, tax_year=2026)
            btc = ui_snapshot.build_capital_gains_snapshot(conn, tax_year=2026, asset="btc")
            ai = daemon._capital_gains_payload_for_ai(
                conn, {"tax_year": 2026, "limit": 1}
            )

        self.assertEqual(
            snapshot["totals"],
            {"rowCount": 3, "costEur": 400.0, "proceedsEur": 400.1,
             "incomeEur": 70.2, "gainLossEur": 70.3},
        )
        newest = snapshot["lots"][-1]
        self.assertEqual(newest["transactionId"], "t-j3")
        self.assertEqual(newest["asset"], "LBTC")
        self.assertEqual(newest["entryType"], "income")
        self.assertEqual(newest["quantityMsat"], 5_000_000_000)

        # A generic book states its own currency and holding period and gets
        # no Austrian estimate.
        self.assertEqual(snapshot["jurisdictionCode"], "GENERIC")
        self.assertEqual(snapshot["fiatCurrency"], "EUR")
        self.assertEqual(snapshot["longTermDays"], 365)
        self.assertIsNone(snapshot["indicativeTax"])

        self.assertEqual(btc["asset"], "BTC")
        self.assertEqual([lot["transactionId"] for lot in btc["lots"]], ["t-j1", "t-j2"])
        self.assertEqual(btc["totals"]["rowCount"], 2)

        # The model gets the newest rows, the year's totals, and a note that
        # rows are journal entries rather than acquisition lots.
        self.assertNotIn("lots", ai)
        self.assertEqual([row["transactionId"] for row in ai["rows"]], ["t-j3"])
        self.assertTrue(ai["rows_truncated"])
        self.assertEqual(ai["totals"]["rowCount"], 3)
        self.assertIn("not an acquisition lot", ai["row_note"])
        self.assertNotIn("indicativeTax", ai)
        for key in ("explanationReference", "acquired", "sats"):
            self.assertNotIn(key, ai["rows"][0])

    def test_breakdown_groups_by_holding_period_and_caps_its_links(self):
        from kassiber import daemon

        conn = self._conn()
        conn.execute("INSERT INTO wallets VALUES ('w1', 'p1', 'Hot')")
        self._insert_entry(conn, "j1", "BTC", "disposal", "2026-02-01T00:00:00Z",
                           -10_000_000_000, 100.0, 150.0, 50.0)
        self._insert_entry(conn, "j2", "BTC", "income", "2026-06-01T00:00:00Z",
                           5_000_000_000, 0.0, 70.2, 70.2)
        for index in range(ui_snapshot._BREAKDOWN_LINK_LIMIT + 1):
            self._insert_entry(conn, f"l{index:03d}", "BTC", "disposal",
                               "2026-03-01T00:00:00Z", -1_000_000, 1.0, 2.0, 1.0)
        conn.execute(
            "UPDATE journal_entries SET capital_gains_type = 'long' WHERE id LIKE 'l%'"
        )

        with patch.object(
            ui_snapshot, "_journal_freshness", return_value={"needs_processing": False}
        ):
            snapshot = ui_snapshot.build_capital_gains_snapshot(conn, tax_year=2026)
            ai = daemon._capital_gains_payload_for_ai(conn, {"tax_year": 2026})

        # A fixed order, whatever order the rows arrive in.
        self.assertEqual(
            [bucket["key"] for bucket in snapshot["breakdown"]],
            ["short_term", "long_term", "income"],
        )
        buckets = {bucket["key"]: bucket for bucket in snapshot["breakdown"]}
        self.assertEqual(
            buckets["short_term"],
            {"key": "short_term", "rowCount": 1, "transactionCount": 1,
             "transactionIds": ["t-j1"], "costEur": 100.0, "proceedsEur": 150.0,
             "incomeEur": 0.0, "gainLossEur": 50.0},
        )
        self.assertEqual(buckets["income"]["incomeEur"], 70.2)
        self.assertEqual(buckets["income"]["proceedsEur"], 0.0)
        # Past the Transactions screen's id cap the group links to the year.
        self.assertEqual(buckets["long_term"]["transactionCount"], 129)
        self.assertIsNone(buckets["long_term"]["transactionIds"])
        # The model gets the sums; the id lists only feed desktop links.
        for bucket in ai["breakdown"]:
            self.assertNotIn("transactionIds", bucket)

    def test_neu_swap_only_year_is_not_selected_as_default(self):
        conn = self._conn(tax_country="at", gains_algorithm="moving_average_at")
        conn.execute("INSERT INTO wallets VALUES ('w1', 'p1', 'Hot')")
        conn.execute(
            "INSERT INTO transactions VALUES (?,?,?,?,?,?,?,?)",
            ("t-swap", "p1", "w1", 0, 1, "2025-06-01T00:00:00Z", "BTC", 100_000_000_000),
        )
        conn.execute(
            "INSERT INTO journal_entries (id, profile_id, transaction_id, entry_type, "
            "occurred_at, quantity, cost_basis, proceeds, gain_loss, at_category, "
            "at_kennzahl, capital_gains_type, created_at) "
            "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
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
            "_austrian_old_rule_rows",
            return_value=0,
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
            "INSERT INTO journal_entries (id, profile_id, transaction_id, entry_type, "
            "occurred_at, quantity, cost_basis, proceeds, gain_loss, at_category, "
            "at_kennzahl, capital_gains_type, created_at) "
            "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
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
            "_austrian_old_rule_rows",
            return_value=0,
        ), patch.object(
            ui_snapshot,
            "_austrian_kennzahl_snapshot_rows",
            return_value=[],
        ):
            snapshot = ui_snapshot.build_capital_gains_snapshot(conn, tax_year=2025)
            liquid = ui_snapshot.build_capital_gains_snapshot(
                conn, tax_year=2025, asset="LBTC"
            )

        # An asset-scoped read keeps only the swaps that gave that asset up.
        self.assertEqual(liquid["neutralSwapLots"], [])
        self.assertNotIn("swaps", [bucket["key"] for bucket in liquid["breakdown"]])
        self.assertEqual(snapshot["lots"][0]["disposed"], "2025-01-01")
        self.assertEqual(snapshot["neutralSwapLots"][0]["date"], "2025-01-01")
        # Austrian rows group by field, with tax-neutral swaps beside them.
        self.assertEqual(
            [(bucket["key"], bucket["transactionIds"]) for bucket in snapshot["breakdown"]],
            [("174", ["t-sell"]), ("swaps", ["t-swap-out"])],
        )

    def test_austrian_breakdown_keeps_tax_free_and_unassigned_rows_apart(self):
        conn = self._conn(tax_country="at", gains_algorithm="moving_average_at")
        conn.execute("INSERT INTO wallets VALUES ('w1', 'p1', 'Hot')")
        cases = (
            ("j-neu", "neu_gain", 174, 40.0, 90.0, 50.0),
            ("j-alt", "alt_taxfree", None, 10.0, 310.0, 300.0),
            ("j-other", None, None, 5.0, 12.5, 7.5),
        )
        for entry_id, category, kennzahl, cost, proceeds, gain in cases:
            occurred_at = "2025-06-01T10:00:00Z"
            conn.execute(
                "INSERT INTO transactions VALUES (?,?,?,?,?,?,?,?)",
                (f"t-{entry_id}", "p1", "w1", 0, 1, occurred_at, "BTC", 1_000_000_000),
            )
            conn.execute(
                "INSERT INTO journal_entries (id, profile_id, transaction_id, entry_type, "
                "occurred_at, quantity, cost_basis, proceeds, gain_loss, at_category, "
                "at_kennzahl, capital_gains_type, created_at) "
                "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
                (entry_id, "p1", f"t-{entry_id}", "disposal", occurred_at,
                 -1_000_000_000, cost, proceeds, gain, category, kennzahl, "short",
                 occurred_at),
            )

        with patch.object(
            ui_snapshot, "_journal_freshness", return_value={"needs_processing": False}
        ), patch.object(
            ui_snapshot, "_austrian_old_rule_rows", return_value=0
        ), patch.object(
            ui_snapshot, "_austrian_kennzahl_snapshot_rows", return_value=[]
        ):
            snapshot = ui_snapshot.build_capital_gains_snapshot(conn, tax_year=2025)

        # Tax-free Altbestand and rows without a field stay out of the field
        # groups, in a fixed order after them.
        self.assertEqual(
            [(bucket["key"], bucket["transactionIds"], bucket["gainLossEur"])
             for bucket in snapshot["breakdown"]],
            [
                ("174", ["t-j-neu"], 50.0),
                ("tax_free", ["t-j-alt"], 300.0),
                ("unassigned", ["t-j-other"], 7.5),
            ],
        )
        # The year's total still sums every row; the desktop states the
        # tax-free part from its group.
        self.assertEqual(snapshot["totals"]["gainLossEur"], 357.5)

    def test_a_year_before_the_e1kv_states_no_e1kv_field_or_estimate(self):
        conn = self._conn(tax_country="at", gains_algorithm="moving_average_at")
        conn.execute("INSERT INTO wallets VALUES ('w1', 'p1', 'Hot')")
        cases = (
            ("j-neu", "neu_gain", 174, 40.0, 90.0, 50.0),
            ("j-spek", "alt_spekulation", 801, 10.0, 30.0, 20.0),
        )
        for entry_id, category, kennzahl, cost, proceeds, gain in cases:
            occurred_at = "2021-06-01T10:00:00Z"
            conn.execute(
                "INSERT INTO transactions VALUES (?,?,?,?,?,?,?,?)",
                (f"t-{entry_id}", "p1", "w1", 0, 1, occurred_at, "BTC", 1_000_000_000),
            )
            conn.execute(
                "INSERT INTO journal_entries (id, profile_id, transaction_id, entry_type, "
                "occurred_at, quantity, cost_basis, proceeds, gain_loss, at_category, "
                "at_kennzahl, capital_gains_type, created_at) "
                "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
                (entry_id, "p1", f"t-{entry_id}", "disposal", occurred_at,
                 -1_000_000_000, cost, proceeds, gain, category, kennzahl, "short",
                 occurred_at),
            )
        kennzahl_rows = [
            {"code": "801", "label": "KZ 801", "form": "E 1", "formSection": "",
             "amount": 20.0, "amountEurCents": 2000, "rowCount": 1, "source": "daemon"},
        ]

        with patch.object(
            ui_snapshot, "_journal_freshness", return_value={"needs_processing": False}
        ), patch.object(
            ui_snapshot, "_austrian_old_rule_rows", return_value=0
        ), patch.object(
            ui_snapshot, "_austrian_kennzahl_snapshot_rows", return_value=kennzahl_rows
        ):
            snapshot = ui_snapshot.build_capital_gains_snapshot(conn, tax_year=2021)

        self.assertFalse(snapshot["e1kvApplies"])
        self.assertIsNone(snapshot["indicativeTax"])
        # The E 1kv field did not exist in 2021: that row has no field; KZ 801
        # is an E 1 field and stays.
        self.assertEqual(
            [(bucket["key"], bucket["transactionIds"]) for bucket in snapshot["breakdown"]],
            [("801", ["t-j-spek"]), ("unassigned", ["t-j-neu"])],
        )
        self.assertEqual(
            sorted(lot["kennzahl"] for lot in snapshot["lots"] if lot["kennzahl"]), [801]
        )

    def test_swap_breakdown_counts_a_split_swap_once_and_scopes_by_entry_asset(self):
        conn = self._conn(tax_country="at", gains_algorithm="moving_average_at")
        conn.executemany(
            "INSERT INTO wallets VALUES (?, 'p1', ?)",
            [("w1", "Hot"), ("w2", "Cold")],
        )
        occurred_at = "2025-06-01T10:00:00Z"
        conn.executemany(
            "INSERT INTO transactions VALUES (?,?,?,?,?,?,?,?)",
            [
                ("t-swap-out", "p1", "w1", 0, 1, occurred_at, "BTC", 30_000_000_000),
                ("t-in-a", "p1", "w2", 0, 1, occurred_at, "LBTC", 10_000_000_000),
                ("t-in-b", "p1", "w2", 0, 1, occurred_at, "LBTC", 19_900_000_000),
            ],
        )
        conn.execute(
            "INSERT INTO journal_entries (id, profile_id, transaction_id, entry_type, "
            "occurred_at, quantity, cost_basis, proceeds, gain_loss, at_category, "
            "at_kennzahl, capital_gains_type, created_at, asset) "
            "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            (
                "j-swap", "p1", "t-swap-out", "disposal", occurred_at,
                -30_000_000_000, 24.0, 24.0, 0.0, "neu_swap", None, "short",
                occurred_at, "BTC",
            ),
        )
        # One swap split across two conversions; neither names its out asset.
        conn.executemany(
            "INSERT INTO journal_custody_projection_relations VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            [
                (
                    pair_id, "p1", "conversion", "t-swap-out", in_id, "peg-in",
                    "carrying-value", 0, "network", None, None, "LBTC", amount,
                    occurred_at,
                )
                for pair_id, in_id, amount in (
                    ("pair-a", "t-in-a", 10_000_000_000),
                    ("pair-b", "t-in-b", 19_900_000_000),
                )
            ],
        )

        with patch.object(
            ui_snapshot,
            "_journal_freshness",
            return_value={"needs_processing": False},
        ), patch.object(
            ui_snapshot,
            "_austrian_old_rule_rows",
            return_value=0,
        ), patch.object(
            ui_snapshot,
            "_austrian_kennzahl_snapshot_rows",
            return_value=[],
        ):
            snapshot = ui_snapshot.build_capital_gains_snapshot(conn, tax_year=2025)
            bitcoin = ui_snapshot.build_capital_gains_snapshot(
                conn, tax_year=2025, asset="BTC"
            )

        self.assertEqual(len(snapshot["neutralSwapLots"]), 2)
        swaps = next(b for b in snapshot["breakdown"] if b["key"] == "swaps")
        self.assertEqual(
            (swaps["rowCount"], swaps["transactionCount"], swaps["costEur"], swaps["proceedsEur"]),
            (1, 1, 24.0, 24.0),
        )
        # The journal entry's own asset scopes the swap, not the relation's.
        self.assertEqual(len(bitcoin["neutralSwapLots"]), 2)
        self.assertIn("swaps", [bucket["key"] for bucket in bitcoin["breakdown"]])


if __name__ == "__main__":
    unittest.main()
