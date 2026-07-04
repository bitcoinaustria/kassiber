import sqlite3
import tempfile
import unittest
from pathlib import Path

from kassiber.core.ui_snapshot import build_workspace_overview_snapshot
from kassiber.db import get_setting, open_db, set_setting
from kassiber.errors import AppError
from kassiber.msat import btc_to_msat


NOW = "2026-06-06T10:00:00Z"


def _insert_workspace(conn: sqlite3.Connection, workspace_id: str, label: str) -> None:
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        (workspace_id, label, NOW),
    )


def _insert_profile(
    conn: sqlite3.Connection,
    profile_id: str,
    workspace_id: str,
    label: str,
    *,
    fiat_currency: str = "EUR",
    processed: bool = True,
    active_transactions: int = 1,
) -> None:
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            tax_long_term_days, gains_algorithm, journal_input_version,
            last_processed_input_version, last_processed_at,
            last_processed_tx_count, created_at
        ) VALUES(?, ?, ?, ?, 'generic', 365, 'FIFO', ?, ?, ?, ?, ?)
        """,
        (
            profile_id,
            workspace_id,
            label,
            fiat_currency,
            1,
            1 if processed else 0,
            "2026-06-06T09:30:00Z" if processed else None,
            active_transactions if processed else 0,
            NOW,
        ),
    )


def _insert_wallet(
    conn: sqlite3.Connection,
    wallet_id: str,
    workspace_id: str,
    profile_id: str,
    label: str,
) -> None:
    conn.execute(
        """
        INSERT INTO wallets(
            id, workspace_id, profile_id, label, kind, config_json, created_at
        ) VALUES(?, ?, ?, ?, 'address', '{}', ?)
        """,
        (wallet_id, workspace_id, profile_id, label, NOW),
    )


def _insert_transaction(
    conn: sqlite3.Connection,
    tx_id: str,
    workspace_id: str,
    profile_id: str,
    wallet_id: str,
    *,
    amount_btc: str,
    fiat_currency: str,
    fiat_rate: float,
    occurred_at: str,
    direction: str = "inbound",
) -> None:
    amount = btc_to_msat(amount_btc)
    sign = 1 if direction == "inbound" else -1
    conn.execute(
        """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, confirmed_at, direction, asset, amount, fee,
            fiat_currency, fiat_rate, fiat_value, fiat_price_source,
            kind, description, counterparty, note, excluded, raw_json, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, 'BTC', ?, 0, ?, ?, ?, 'manual',
                 'deposit', '', '', '', 0, '{}', ?)
        """,
        (
            tx_id,
            workspace_id,
            profile_id,
            wallet_id,
            f"{tx_id}-external",
            f"{tx_id}-fingerprint",
            occurred_at,
            occurred_at,
            direction,
            abs(amount),
            fiat_currency,
            fiat_rate,
            float(sign * amount) / 100_000_000_000 * fiat_rate,
            NOW,
        ),
    )


def _insert_journal_entry(
    conn: sqlite3.Connection,
    entry_id: str,
    workspace_id: str,
    profile_id: str,
    wallet_id: str,
    tx_id: str,
    *,
    quantity_btc: str,
    fiat_value: float,
    occurred_at: str | None = None,
) -> None:
    conn.execute(
        """
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
            occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
            cost_basis, proceeds, gain_loss, created_at
        ) VALUES(?, ?, ?, ?, ?, NULL, ?, 'acquisition', 'BTC', ?, ?, ?, ?, NULL, 0, ?)
        """,
        (
            entry_id,
            workspace_id,
            profile_id,
            tx_id,
            wallet_id,
            occurred_at or NOW,
            btc_to_msat(quantity_btc),
            fiat_value,
            fiat_value,
            fiat_value,
            NOW,
        ),
    )


def _insert_quarantine(
    conn: sqlite3.Connection,
    workspace_id: str,
    profile_id: str,
    tx_id: str,
) -> None:
    conn.execute(
        """
        INSERT INTO journal_quarantines(
            transaction_id, workspace_id, profile_id, reason, detail_json, created_at
        ) VALUES(?, ?, ?, 'missing_price', '{}', ?)
        """,
        (tx_id, workspace_id, profile_id, NOW),
    )


class WorkspaceOverviewSnapshotTest(unittest.TestCase):
    def _db(self) -> sqlite3.Connection:
        tmp = tempfile.TemporaryDirectory(prefix="kassiber-workspace-overview-")
        self.addCleanup(tmp.cleanup)
        conn = open_db(Path(tmp.name) / "data")
        self.addCleanup(conn.close)
        return conn

    def _seed_workspace(self, conn: sqlite3.Connection, *, mixed: bool = False) -> None:
        _insert_workspace(conn, "active-ws", "Active Set")
        _insert_profile(conn, "active-pf", "active-ws", "Active Book")
        _insert_workspace(conn, "ws", "Treasury Set")
        _insert_profile(conn, "pf-a", "ws", "Operating", fiat_currency="EUR")
        _insert_profile(
            conn,
            "pf-b",
            "ws",
            "Personal",
            fiat_currency="CHF" if mixed else "EUR",
            processed=False,
        )
        _insert_wallet(conn, "wal-a", "ws", "pf-a", "Operating Wallet")
        _insert_wallet(conn, "wal-b", "ws", "pf-b", "Personal Wallet")
        _insert_transaction(
            conn,
            "tx-a",
            "ws",
            "pf-a",
            "wal-a",
            amount_btc="1.0",
            fiat_currency="EUR",
            fiat_rate=50_000,
            occurred_at="2026-06-01T08:00:00Z",
        )
        _insert_transaction(
            conn,
            "tx-b",
            "ws",
            "pf-b",
            "wal-b",
            amount_btc="0.5",
            fiat_currency="CHF" if mixed else "EUR",
            fiat_rate=60_000,
            occurred_at="2026-06-02T08:00:00Z",
        )
        _insert_journal_entry(
            conn,
            "je-a",
            "ws",
            "pf-a",
            "wal-a",
            "tx-a",
            quantity_btc="1.0",
            fiat_value=50_000,
            occurred_at="2026-06-01T08:00:00Z",
        )
        _insert_quarantine(conn, "ws", "pf-b", "tx-b")
        set_setting(conn, "context_workspace", "active-ws")
        set_setting(conn, "context_profile", "active-pf")
        conn.commit()

    def test_multi_profile_workspace_aggregation_preserves_book_boundaries(self):
        conn = self._db()
        self._seed_workspace(conn)

        snapshot = build_workspace_overview_snapshot(conn, {"workspace_id": "ws"})

        self.assertEqual(snapshot["workspace"], {"id": "ws", "label": "Treasury Set"})
        self.assertEqual(snapshot["status"]["bookCount"], 2)
        self.assertEqual(snapshot["status"]["transactionCount"], 2)
        self.assertAlmostEqual(snapshot["fiat"]["btcBalance"], 1.5)
        self.assertEqual(snapshot["fiat"]["mode"], "single")
        self.assertEqual(snapshot["fiat"]["fiatCurrency"], "EUR")
        self.assertEqual(snapshot["fiat"]["eurBalance"], 80_000)
        self.assertEqual(
            {row["profileLabel"] for row in snapshot["fiat"]["books"]},
            {"Operating", "Personal"},
        )
        self.assertEqual(
            {connection["profileLabel"] for connection in snapshot["connections"]},
            {"Operating", "Personal"},
        )
        self.assertTrue(snapshot["status"]["needsJournals"])
        self.assertEqual(snapshot["status"]["quarantines"], 1)
        by_book = {book["profile"]["label"]: book for book in snapshot["books"]}
        self.assertTrue(by_book["Operating"]["readiness"]["ready"])
        self.assertFalse(by_book["Personal"]["readiness"]["ready"])
        self.assertIn("Run journal processing", by_book["Personal"]["readiness"]["hints"][0])
        series_by_date = {point["date"]: point for point in snapshot["portfolioSeries"]}
        self.assertAlmostEqual(series_by_date["2026-06-01"]["balanceBtc"], 1.0)
        self.assertAlmostEqual(series_by_date["2026-06-02"]["balanceBtc"], 1.5)
        self.assertEqual(series_by_date["2026-06-02"]["valueEur"], 80_000)
        self.assertEqual(
            {row["profileLabel"] for row in series_by_date["2026-06-02"]["books"]},
            {"Operating", "Personal"},
        )

    def test_mixed_currency_rollup_is_partial_and_keeps_per_book_rows(self):
        conn = self._db()
        self._seed_workspace(conn, mixed=True)

        snapshot = build_workspace_overview_snapshot(conn, {"workspace_id": "ws"})

        self.assertEqual(snapshot["fiat"]["mode"], "mixed")
        self.assertTrue(snapshot["fiat"]["mixed"])
        self.assertTrue(snapshot["fiat"]["partial"])
        self.assertIsNone(snapshot["fiat"]["eurBalance"])
        self.assertEqual(snapshot["fiat"]["currencies"], ["CHF", "EUR"])
        self.assertEqual(
            [(row["profileLabel"], row["fiatCurrency"]) for row in snapshot["fiat"]["books"]],
            [("Operating", "EUR"), ("Personal", "CHF")],
        )
        self.assertAlmostEqual(snapshot["fiat"]["btcBalance"], 1.5)
        latest = snapshot["portfolioSeries"][-1]
        self.assertAlmostEqual(latest["balanceBtc"], 1.5)
        self.assertNotIn("valueEur", latest)
        self.assertEqual(
            [(row["profileLabel"], row["fiatCurrency"]) for row in latest["books"]],
            [("Operating", "EUR"), ("Personal", "CHF")],
        )

    def test_empty_workspace_returns_empty_snapshot(self):
        conn = self._db()
        _insert_workspace(conn, "empty", "Empty Set")
        conn.commit()

        snapshot = build_workspace_overview_snapshot(conn, {"workspace_id": "empty"})

        self.assertEqual(snapshot["workspace"], {"id": "empty", "label": "Empty Set"})
        self.assertEqual(snapshot["books"], [])
        self.assertEqual(snapshot["status"]["bookCount"], 0)
        self.assertEqual(snapshot["fiat"]["mode"], "empty")

    def test_missing_workspace_raises_not_found(self):
        conn = self._db()

        with self.assertRaises(AppError) as caught:
            build_workspace_overview_snapshot(conn, {"workspace_id": "missing"})

        self.assertEqual(caught.exception.code, "not_found")

    def test_workspace_read_does_not_switch_active_context(self):
        conn = self._db()
        self._seed_workspace(conn)

        build_workspace_overview_snapshot(conn, {"workspace_id": "ws"})

        self.assertEqual(get_setting(conn, "context_workspace"), "active-ws")
        self.assertEqual(get_setting(conn, "context_profile"), "active-pf")


if __name__ == "__main__":
    unittest.main()
