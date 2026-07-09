"""Tax-summary non-reportable / neu_swap exclusion key matching."""

from __future__ import annotations

import sqlite3
import unittest

from kassiber.core.reports import _exclude_non_reportable_tax_summary_rows


def _conn_with_non_reportable_disposal(*, journal_cgt: str | None = "short"):
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(
        """
        CREATE TABLE transactions (
          id TEXT PRIMARY KEY,
          profile_id TEXT,
          taxability_override INTEGER,
          kind TEXT
        );
        CREATE TABLE journal_entries (
          id TEXT PRIMARY KEY,
          profile_id TEXT,
          transaction_id TEXT,
          entry_type TEXT,
          asset TEXT,
          quantity INTEGER,
          proceeds REAL,
          cost_basis REAL,
          gain_loss REAL,
          capital_gains_type TEXT,
          occurred_at TEXT,
          at_category TEXT
        );
        """
    )
    conn.execute(
        "INSERT INTO transactions VALUES (?, ?, ?, ?)",
        ("t1", "p1", 0, "sell"),
    )
    conn.execute(
        "INSERT INTO journal_entries VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        (
            "j1",
            "p1",
            "t1",
            "disposal",
            "BTC",
            100_000_000_000,
            100.0,
            50.0,
            50.0,
            journal_cgt,
            "2026-03-01T00:00:00Z",
            None,
        ),
    )
    return conn


class TaxSummaryExclusionTests(unittest.TestCase):
    def test_null_summary_capital_gains_type_still_excludes_non_reportable(self):
        conn = _conn_with_non_reportable_disposal(journal_cgt="short")
        summary_rows = [
            {
                "year": 2026,
                "asset": "BTC",
                "transaction_type": "sell",
                "capital_gains_type": None,
                "quantity": 1.0,
                "quantity_msat": 100_000_000_000,
                "proceeds": 100.0,
                "cost_basis": 50.0,
                "gain_loss": 50.0,
            }
        ]
        out = _exclude_non_reportable_tax_summary_rows(conn, "p1", summary_rows)
        self.assertEqual(out, [])

    def test_blank_summary_capital_gains_type_still_excludes_non_reportable(self):
        conn = _conn_with_non_reportable_disposal(journal_cgt=None)
        summary_rows = [
            {
                "year": 2026,
                "asset": "BTC",
                "transaction_type": "sell",
                "capital_gains_type": "",
                "quantity": 1.0,
                "quantity_msat": 100_000_000_000,
                "proceeds": 100.0,
                "cost_basis": 50.0,
                "gain_loss": 50.0,
            }
        ]
        out = _exclude_non_reportable_tax_summary_rows(conn, "p1", summary_rows)
        self.assertEqual(out, [])

    def test_matching_short_excludes_non_reportable(self):
        conn = _conn_with_non_reportable_disposal(journal_cgt="short")
        summary_rows = [
            {
                "year": 2026,
                "asset": "BTC",
                "transaction_type": "sell",
                "capital_gains_type": "short",
                "quantity": 1.0,
                "quantity_msat": 100_000_000_000,
                "proceeds": 100.0,
                "cost_basis": 50.0,
                "gain_loss": 50.0,
            }
        ]
        out = _exclude_non_reportable_tax_summary_rows(conn, "p1", summary_rows)
        self.assertEqual(out, [])


if __name__ == "__main__":
    unittest.main()
