import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.core import custody_journal
from kassiber.core.ui_snapshot import (
    build_audit_changes_since_last_answer_snapshot,
    build_overview_snapshot,
    build_workspace_overview_snapshot,
)
from kassiber.db import get_setting, open_db, set_setting
from kassiber.errors import AppError
from kassiber.msat import btc_to_msat


NOW = "2026-06-06T10:00:00Z"


class JournalProjectionFreshnessTest(unittest.TestCase):
    def test_canonical_freshness_state_matrix(self):
        base = {
            "last_processed_at": NOW,
            "last_processed_tx_count": 2,
            "journal_input_version": 3,
            "last_processed_input_version": 3,
        }
        cases = (
            (base, 2, "current", True, False),
            ({**base, "last_processed_at": None}, 2, "not_processed", False, True),
            ({**base, "last_processed_tx_count": 1}, 2, "stale", False, True),
            ({**base, "last_processed_input_version": 2}, 2, "stale", False, True),
            ({**base, "last_processed_at": None}, 0, "no_transactions", False, False),
        )
        for profile, active_count, status, current, needs_processing in cases:
            with self.subTest(status=status, profile=profile):
                result = custody_journal.evaluate_projection_freshness(
                    profile, active_count
                )
                self.assertEqual(result["status"], status)
                self.assertEqual(result["is_current"], current)
                self.assertEqual(result["needs_processing"], needs_processing)


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
    entry_type: str = "acquisition",
) -> None:
    conn.execute(
        """
        INSERT INTO journal_entries(
            id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
            occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
            cost_basis, proceeds, gain_loss, created_at
        ) VALUES(?, ?, ?, ?, ?, NULL, ?, ?, 'BTC', ?, ?, ?, ?, NULL, 0, ?)
        """,
        (
            entry_id,
            workspace_id,
            profile_id,
            tx_id,
            wallet_id,
            occurred_at or NOW,
            entry_type,
            btc_to_msat(quantity_btc),
            fiat_value,
            fiat_value,
            fiat_value,
            NOW,
        ),
    )


def _seed_directional_book(
    conn: sqlite3.Connection,
    prefix: str,
    *,
    journals: bool,
) -> None:
    """One book receiving 1.0 BTC, then sending 0.25 BTC back out."""
    _insert_workspace(conn, f"{prefix}-ws", "Net Set")
    _insert_profile(
        conn,
        f"{prefix}-pf",
        f"{prefix}-ws",
        "Trading",
        processed=journals,
        active_transactions=2,
    )
    _insert_wallet(
        conn,
        f"{prefix}-wal",
        f"{prefix}-ws",
        f"{prefix}-pf",
        "Trading Wallet",
    )
    for tx_id, amount_btc, rate, occurred_at, direction in (
        ("tx-in", "1.0", 50_000, "2026-06-01T08:00:00Z", "inbound"),
        ("tx-out", "0.25", 60_000, "2026-06-03T08:00:00Z", "outbound"),
    ):
        _insert_transaction(
            conn,
            f"{prefix}-{tx_id}",
            f"{prefix}-ws",
            f"{prefix}-pf",
            f"{prefix}-wal",
            amount_btc=amount_btc,
            fiat_currency="EUR",
            fiat_rate=rate,
            occurred_at=occurred_at,
            direction=direction,
        )
    if journals:
        for entry_id, tx_id, quantity_btc, fiat_value, occurred_at, entry_type in (
            ("je-in", "tx-in", "1.0", 50_000, "2026-06-01T08:00:00Z", "acquisition"),
            ("je-out", "tx-out", "-0.25", 15_000, "2026-06-03T08:00:00Z", "disposal"),
        ):
            _insert_journal_entry(
                conn,
                f"{prefix}-{entry_id}",
                f"{prefix}-ws",
                f"{prefix}-pf",
                f"{prefix}-wal",
                f"{prefix}-{tx_id}",
                quantity_btc=quantity_btc,
                fiat_value=fiat_value,
                occurred_at=occurred_at,
                entry_type=entry_type,
            )
    conn.commit()


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

    def test_outbound_transaction_reduces_rollup_and_portfolio_series(self):
        conn = self._db()
        _seed_directional_book(conn, "net", journals=True)

        snapshot = build_workspace_overview_snapshot(conn, {"workspace_id": "net-ws"})

        self.assertAlmostEqual(snapshot["fiat"]["btcBalance"], 0.75)
        self.assertAlmostEqual(snapshot["fiat"]["eurBalance"], 45_000)
        self.assertAlmostEqual(snapshot["fiat"]["eurCostBasis"], 35_000)
        series_by_date = {point["date"]: point for point in snapshot["portfolioSeries"]}
        self.assertAlmostEqual(series_by_date["2026-06-01"]["balanceBtc"], 1.0)
        self.assertAlmostEqual(series_by_date["2026-06-03"]["balanceBtc"], 0.75)

    def test_outbound_transaction_reduces_unprocessed_book_rollup(self):
        # No journals yet, so the rollup is derived straight from transaction
        # direction rather than from book state.
        conn = self._db()
        _seed_directional_book(conn, "raw", journals=False)

        snapshot = build_workspace_overview_snapshot(conn, {"workspace_id": "raw-ws"})

        self.assertAlmostEqual(snapshot["fiat"]["btcBalance"], 0.75)
        self.assertAlmostEqual(snapshot["fiat"]["eurBalance"], 45_000)
        series_by_date = {point["date"]: point for point in snapshot["portfolioSeries"]}
        self.assertAlmostEqual(series_by_date["2026-06-01"]["balanceBtc"], 1.0)
        self.assertAlmostEqual(series_by_date["2026-06-03"]["balanceBtc"], 0.75)

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

    def test_audit_snapshot_generated_at_uses_canonical_second_precision(self):
        conn = self._db()
        self._seed_workspace(conn)

        snapshot = build_audit_changes_since_last_answer_snapshot(conn)

        self.assertRegex(
            snapshot["current"]["generated_at"],
            r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$",
        )


def _insert_market_rate(conn: sqlite3.Connection, rate: float = 60_000) -> None:
    conn.execute(
        """
        INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at)
        VALUES('BTC-EUR', '2026-06-05T00:00:00Z', ?, 'manual', ?)
        """,
        (rate, NOW),
    )


def _set_processed_tx_count(conn: sqlite3.Connection, profile_id: str, count: int) -> None:
    conn.execute(
        "UPDATE profiles SET last_processed_tx_count = ? WHERE id = ?",
        (count, profile_id),
    )


def _insert_canonical_wallet_balance(
    conn: sqlite3.Connection,
    prefix: str,
    *,
    amount_btc: str,
) -> None:
    """Canonical custody quantity keeps quarantined rows, like production."""
    conn.execute(
        """
        INSERT INTO journal_quantity_postings(
            posting_id, workspace_id, profile_id, transaction_id, occurred_at,
            asset, location_kind, location_id, amount_msat, state, created_at
        ) VALUES(?, ?, ?, ?, '2026-06-01T08:00:00Z', 'BTC', 'wallet', ?, ?,
                 'observed', ?)
        """,
        (
            f"{prefix}-posting",
            f"{prefix}-ws",
            f"{prefix}-pf",
            f"{prefix}-tx-in",
            f"{prefix}-wal",
            btc_to_msat("1.0"),
            NOW,
        ),
    )
    conn.execute(
        """
        INSERT INTO journal_quantity_balances(
            workspace_id, profile_id, location_kind, location_id, asset,
            amount_msat, created_at
        ) VALUES(?, ?, 'wallet', ?, 'BTC', ?, ?)
        """,
        (f"{prefix}-ws", f"{prefix}-pf", f"{prefix}-wal", btc_to_msat(amount_btc), NOW),
    )


def _activate_book(conn: sqlite3.Connection, prefix: str) -> None:
    set_setting(conn, "context_workspace", f"{prefix}-ws")
    set_setting(conn, "context_profile", f"{prefix}-pf")
    conn.commit()


class OverviewFiatCompletenessTest(unittest.TestCase):
    def _db(self) -> sqlite3.Connection:
        tmp = tempfile.TemporaryDirectory(prefix="kassiber-overview-completeness-")
        self.addCleanup(tmp.cleanup)
        conn = open_db(Path(tmp.name) / "data")
        self.addCleanup(conn.close)
        return conn

    def _add_quarantined_transaction(
        self,
        conn: sqlite3.Connection,
        prefix: str,
        *,
        amount_btc: str,
        direction: str,
        occurred_at: str,
    ) -> None:
        tx_id = f"{prefix}-tx-quarantined"
        _insert_transaction(
            conn,
            tx_id,
            f"{prefix}-ws",
            f"{prefix}-pf",
            f"{prefix}-wal",
            amount_btc=amount_btc,
            fiat_currency="EUR",
            fiat_rate=55_000,
            occurred_at=occurred_at,
            direction=direction,
        )
        _insert_quarantine(conn, f"{prefix}-ws", f"{prefix}-pf", tx_id)
        _set_processed_tx_count(conn, f"{prefix}-pf", 3)

    def test_current_book_with_market_rate_claims_complete_basis(self):
        conn = self._db()
        _seed_directional_book(conn, "ok", journals=True)
        _insert_market_rate(conn)
        _activate_book(conn, "ok")

        overview = build_overview_snapshot(conn)
        completeness = overview["fiat"]["completeness"]

        self.assertEqual(completeness["state"], "complete")
        self.assertTrue(completeness["costBasisComplete"])
        self.assertEqual(completeness["reasons"], [])
        self.assertEqual(completeness["quarantineCount"], 0)
        self.assertEqual(completeness["basisCoveredMsat"], btc_to_msat("0.75"))
        self.assertEqual(completeness["basisUncoveredMsat"], 0)
        self.assertIsNone(completeness["earliestIncompleteAt"])
        self.assertEqual(completeness["missingPriceCount"], 0)
        self.assertFalse(completeness["marketRateMissing"])
        self.assertAlmostEqual(overview["fiat"]["eurCostBasis"], 35_000)

    def test_quarantined_inbound_leaves_displayed_btc_without_basis(self):
        conn = self._db()
        _seed_directional_book(conn, "qin", journals=True)
        _insert_market_rate(conn)
        self._add_quarantined_transaction(
            conn,
            "qin",
            amount_btc="0.12",
            direction="inbound",
            occurred_at="2026-06-04T08:00:00Z",
        )
        _insert_canonical_wallet_balance(conn, "qin", amount_btc="0.87")
        _activate_book(conn, "qin")

        overview = build_overview_snapshot(conn)
        completeness = overview["fiat"]["completeness"]

        self.assertAlmostEqual(overview["balanceSummary"]["totalBtc"], 0.87)
        self.assertEqual(completeness["state"], "incomplete")
        self.assertFalse(completeness["costBasisComplete"])
        self.assertEqual(completeness["reasons"], ["quarantines"])
        self.assertEqual(completeness["quarantineCount"], 1)
        self.assertEqual(completeness["quarantinedInboundMsat"], btc_to_msat("0.12"))
        self.assertEqual(completeness["quarantinedOutboundMsat"], 0)
        self.assertEqual(completeness["basisCoveredMsat"], btc_to_msat("0.75"))
        self.assertEqual(completeness["basisUncoveredMsat"], btc_to_msat("0.12"))
        self.assertEqual(completeness["earliestIncompleteAt"], "2026-06-04T08:00:00Z")
        # The basis only reflects journaled rows, which is why the completeness
        # block must travel with it.
        self.assertAlmostEqual(overview["fiat"]["eurCostBasis"], 35_000)

    def test_quarantined_outbound_marks_basis_incomplete_without_uncovered_btc(self):
        conn = self._db()
        _seed_directional_book(conn, "qout", journals=True)
        _insert_market_rate(conn)
        self._add_quarantined_transaction(
            conn,
            "qout",
            amount_btc="0.1",
            direction="outbound",
            occurred_at="2026-06-04T09:00:00Z",
        )
        _insert_canonical_wallet_balance(conn, "qout", amount_btc="0.65")
        _activate_book(conn, "qout")

        completeness = build_overview_snapshot(conn)["fiat"]["completeness"]

        self.assertEqual(completeness["state"], "incomplete")
        self.assertFalse(completeness["costBasisComplete"])
        self.assertEqual(completeness["quarantinedInboundMsat"], 0)
        self.assertEqual(completeness["quarantinedOutboundMsat"], btc_to_msat("0.1"))
        # Journaled basis still carries coins that already left, so basis is
        # overstated: nothing is uncovered, yet the block stays incomplete.
        self.assertEqual(completeness["basisUncoveredMsat"], 0)
        self.assertEqual(completeness["earliestIncompleteAt"], "2026-06-04T09:00:00Z")

    def test_custody_blocker_without_a_start_keeps_every_point_incomplete(self):
        conn = self._db()
        _seed_directional_book(conn, "nostart", journals=True)
        _insert_market_rate(conn)
        self._add_quarantined_transaction(
            conn,
            "nostart",
            amount_btc="0.1",
            direction="outbound",
            occurred_at="2026-06-04T09:00:00Z",
        )
        _activate_book(conn, "nostart")

        with patch(
            "kassiber.core.custody_quantity_store.custody_quantity_readiness_summary",
            side_effect=AppError(
                "Custody state unavailable", code="custody_quantity_state_unavailable"
            ),
        ):
            completeness = build_overview_snapshot(conn)["fiat"]["completeness"]

        self.assertIn("custody_unresolved", completeness["reasons"])
        # The quarantine's date must not hide that custody coverage is unknown.
        self.assertIsNone(completeness["earliestIncompleteAt"])

    def test_stale_journals_do_not_claim_coverage(self):
        conn = self._db()
        _seed_directional_book(conn, "stale", journals=True)
        _insert_market_rate(conn)
        _set_processed_tx_count(conn, "stale-pf", 1)
        _activate_book(conn, "stale")

        completeness = build_overview_snapshot(conn)["fiat"]["completeness"]

        self.assertEqual(completeness["state"], "stale")
        self.assertFalse(completeness["costBasisComplete"])
        self.assertEqual(completeness["reasons"], ["journals_stale"])
        self.assertIsNone(completeness["basisCoveredMsat"])
        self.assertIsNone(completeness["basisUncoveredMsat"])
        self.assertIsNone(completeness["earliestIncompleteAt"])

    def test_missing_market_rate_is_flagged_instead_of_trusted(self):
        conn = self._db()
        _seed_directional_book(conn, "norate", journals=True)
        _activate_book(conn, "norate")

        overview = build_overview_snapshot(conn)
        completeness = overview["fiat"]["completeness"]

        self.assertIsNone(overview["marketRate"]["rate"])
        self.assertEqual(completeness["state"], "unavailable")
        self.assertTrue(completeness["costBasisComplete"])
        self.assertEqual(completeness["reasons"], ["market_rate_missing"])
        self.assertTrue(completeness["marketRateMissing"])

    def test_custody_gap_and_missing_price_block_basis_from_earliest_date(self):
        conn = self._db()
        _seed_directional_book(conn, "gap", journals=True)
        _insert_market_rate(conn)
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source,
                kind, description, counterparty, note, excluded, raw_json, created_at
            ) VALUES('gap-tx-unpriced', 'gap-ws', 'gap-pf', 'gap-wal', 'unpriced',
                     'unpriced-fp', '2026-06-05T08:00:00Z', '2026-06-05T08:00:00Z',
                     'inbound', 'BTC', ?, 0, 'EUR', NULL, NULL, NULL, 'deposit',
                     '', '', '', 0, '{}', ?)
            """,
            (btc_to_msat("0.01"), NOW),
        )
        _set_processed_tx_count(conn, "gap-pf", 3)
        conn.execute(
            """
            INSERT INTO journal_quantity_issues(
                issue_id, workspace_id, profile_id, issue_type, state, asset,
                amount_msat, occurred_at, transaction_ids_json, reason,
                detail_json, blocks_from, created_at
            ) VALUES('gap-issue', 'gap-ws', 'gap-pf', 'unresolved_quantity',
                     'custody_suspense', 'BTC', ?, '2026-06-02T08:00:00Z',
                     '["gap-tx-out"]', 'missing_wallet', '{}',
                     '2026-06-02T08:00:00Z', ?)
            """,
            (btc_to_msat("0.05"), NOW),
        )
        _activate_book(conn, "gap")

        completeness = build_overview_snapshot(conn)["fiat"]["completeness"]

        self.assertEqual(completeness["state"], "incomplete")
        self.assertEqual(completeness["reasons"], ["custody_unresolved", "missing_prices"])
        self.assertEqual(completeness["missingPriceCount"], 1)
        self.assertEqual(completeness["earliestIncompleteAt"], "2026-06-02T08:00:00Z")

    def test_workspace_rollup_carries_book_and_total_completeness(self):
        conn = self._db()
        _seed_directional_book(conn, "roll", journals=True)
        _insert_market_rate(conn)
        _insert_profile(
            conn,
            "roll-pf-b",
            "roll-ws",
            "Savings",
            processed=True,
            active_transactions=1,
        )
        _insert_wallet(conn, "roll-wal-b", "roll-ws", "roll-pf-b", "Savings Wallet")
        _insert_transaction(
            conn,
            "roll-b-tx",
            "roll-ws",
            "roll-pf-b",
            "roll-wal-b",
            amount_btc="0.2",
            fiat_currency="EUR",
            fiat_rate=40_000,
            occurred_at="2026-06-02T08:00:00Z",
        )
        _insert_quarantine(conn, "roll-ws", "roll-pf-b", "roll-b-tx")
        conn.commit()

        snapshot = build_workspace_overview_snapshot(conn, {"workspace_id": "roll-ws"})

        rows = {row["profileLabel"]: row for row in snapshot["fiat"]["books"]}
        self.assertEqual(rows["Trading"]["completeness"]["state"], "complete")
        self.assertEqual(rows["Savings"]["completeness"]["state"], "incomplete")
        rollup = snapshot["fiat"]["completeness"]
        self.assertEqual(rollup["state"], "incomplete")
        self.assertFalse(rollup["costBasisComplete"])
        self.assertEqual(rollup["reasons"], ["quarantines"])
        self.assertEqual(rollup["quarantineCount"], 1)
        self.assertEqual(rollup["quarantinedInboundMsat"], btc_to_msat("0.2"))
        self.assertEqual(rollup["earliestIncompleteAt"], "2026-06-02T08:00:00Z")
        books = {book["profile"]["label"]: book for book in snapshot["books"]}
        self.assertIn("balanceSummary", books["Savings"])
        self.assertIn("taxFreeBalance", books["Savings"])
        self.assertEqual(
            books["Savings"]["fiat"]["completeness"],
            rows["Savings"]["completeness"],
        )

    def test_empty_overview_does_not_claim_complete_basis(self):
        conn = self._db()

        completeness = build_overview_snapshot(conn)["fiat"]["completeness"]

        self.assertEqual(completeness["state"], "unavailable")
        self.assertFalse(completeness["costBasisComplete"])


if __name__ == "__main__":
    unittest.main()
