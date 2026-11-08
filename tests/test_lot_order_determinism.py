"""Lots that share a timestamp are consumed in an order the data decides.

Two books built from the same import must book the same gains. Rows imported
in one second share `occurred_at` and `created_at`, so composition used to
fall back to the random row id, and a disposal consumed whichever same-time
acquisition happened to sort first (100, 200 or 300 EUR of gain for one sale).
These tests give the rows adversarial ids and require one answer.
"""
from __future__ import annotations

import contextlib
import io
import itertools
import json
from pathlib import Path
import sqlite3
import tempfile
from typing import Any
import unittest
from unittest import mock
import uuid

from kassiber.cli import handlers
from kassiber.cli.main import main
from kassiber.core import accounts as core_accounts
from kassiber.core import custody_journal
from kassiber.core.journal_calculation import stored_calculation
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, SAME_TIME_ORDER_SEMANTICS_MIGRATION, open_db


# Same-second acquisitions at three prices, then sales that consume part of
# them. File order is c, a, b: neither the external ids nor the prices sort
# the same way, so a FIFO/LIFO answer shows which key decided.
ROWS = [
    ("lot-c", "2024-01-01T00:00:00Z", "inbound", "0.01", "30000"),
    ("lot-a", "2024-01-01T00:00:00Z", "inbound", "0.01", "10000"),
    ("lot-b", "2024-01-01T00:00:00Z", "inbound", "0.01", "20000"),
    ("sale-1", "2024-02-01T00:00:00Z", "outbound", "0.01", "40000"),
    ("sale-2", "2024-03-01T00:00:00Z", "outbound", "0.005", "40000"),
]
JOURNAL_COLUMNS = (
    "t.external_id", "j.entry_type", "j.quantity", "j.cost_basis_exact",
    "j.proceeds_exact", "j.gain_loss_exact", "j.at_category",
)


class _Ids:
    """`uuid.uuid4` for imported rows: ids in a chosen rank order."""

    def __init__(self, ranks):
        self._ids = iter(uuid.UUID(int=(rank + 1) << 64) for rank in ranks)

    def uuid4(self):
        return next(self._ids)


def build_book(base: Path, rows, *, algorithm="FIFO", country="generic",
               ranks=None, files=None, inspect=None) -> list[tuple]:
    """Import `rows` (one file, or `files` as lists of row indexes) and process.

    `ranks` gives imported rows ids in that rank order; None keeps fresh random
    ids. Returns every realized entry, keyed by the row's external id, or what
    `inspect(conn, profile_id)` returns from the processed book.
    """
    root = base / "data"

    def cli(*args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = main(["--machine", "--data-root", str(root), *args])
        payload = json.loads(out.getvalue())
        assert code == 0, json.dumps(payload)[:800]
        return payload["data"]

    cli("init")
    cli("workspaces", "create", "Main")
    cli("profiles", "create", "Book", "--workspace", "Main", "--fiat-currency", "EUR",
        "--tax-country", country, "--gains-algorithm", algorithm)
    cli("wallets", "create", "--label", "Cold", "--kind", "custom")
    groups = files or [list(range(len(rows)))]
    ids = mock.patch("kassiber.core.imports.uuid", _Ids(ranks)) if ranks is not None else contextlib.nullcontext()
    with ids:
        for number, group in enumerate(groups):
            source = base / f"rows-{number}.csv"
            source.write_text("txid,occurred_at,direction,asset,amount,fee,fiat_rate\n" + "".join(
                f"{txid},{at},{direction},BTC,{amount},0,{rate}\n"
                for txid, at, direction, amount, rate in (rows[index] for index in group)))
            cli("wallets", "import-csv", "--wallet", "Cold", "--file", str(source))
    cli("journals", "process")
    conn = open_db(root)
    if inspect is not None:
        try:
            return inspect(conn, conn.execute("SELECT id FROM profiles").fetchone()[0])
        finally:
            conn.close()
    try:
        external = dict(conn.execute("SELECT id, external_id FROM transactions").fetchall())
        entries = conn.execute(
            f"SELECT {', '.join(JOURNAL_COLUMNS)}, j.calculation_json, j.calculation_compact "
            "FROM journal_entries j JOIN transactions t ON t.id = j.transaction_id "
            "WHERE j.entry_type NOT IN ('acquisition', 'transfer_in', 'transfer_out') "
            "ORDER BY t.external_id, j.entry_type, j.quantity, j.cost_basis_exact").fetchall()
    finally:
        conn.close()
    # Each realized entry with the lots it consumed, so equal-gain answers
    # that consumed different lots still differ.
    return [
        (*tuple(entry)[:len(JOURNAL_COLUMNS)], tuple(
            (external.get((fragment.get("lot") or {}).get("transaction_id")), fragment.get("quantity_msat"))
            for fragment in (stored_calculation(entry) or {}).get("fragments") or ()))
        for entry in entries
    ]


def books_by_id_order(rows, **options) -> dict[Any, list[tuple]]:
    """One book per rank order of the lots' ids, by its result."""
    lots = sum(1 for row in rows if row[2] == "inbound")
    results: dict[tuple, list[tuple]] = {}
    for lot_ranks in itertools.permutations(range(lots)):
        ranks = (*lot_ranks, *range(lots, len(rows)))
        with tempfile.TemporaryDirectory(prefix="kassiber-lot-order-") as tmp:
            result = build_book(Path(tmp).resolve(), rows, ranks=ranks, **options)
            result = tuple(result) if isinstance(result, list) else result
        results.setdefault(result, []).append(ranks)
    return results


class LotOrderDeterminismTest(unittest.TestCase):
    def assert_one_answer(self, rows, **options):
        results = books_by_id_order(rows, **options)
        self.assertEqual(len(results), 1, json.dumps([list(result) for result in results], indent=1))
        return next(iter(results))

    def test_fifo_follows_file_order_whatever_the_row_ids(self):
        result = self.assert_one_answer(ROWS[:4])
        # File order: lot-c (30000) is the first lot.
        self.assertIn(("sale-1", "disposal", -1_000_000_000, "300.00", "400.00", "100.00", None,
                       (("lot-c", 1_000_000_000),)), result)

    def test_lifo_follows_file_order_whatever_the_row_ids(self):
        result = self.assert_one_answer(ROWS[:4], algorithm="LIFO")
        # The last same-time lot in file order is lot-b (20000).
        self.assertIn(("sale-1", "disposal", -1_000_000_000, "200.00", "400.00", "200.00", None,
                       (("lot-b", 1_000_000_000),)), result)

    def test_hifo_and_lofo_rank_same_cost_lots_by_file_order(self):
        # Two lots at one price: the price ties, so the rank falls to the row.
        rows = [
            ("lot-y", "2024-01-01T00:00:00Z", "inbound", "0.02", "10000"),
            ("lot-x", "2024-01-01T00:00:00Z", "inbound", "0.01", "10000"),
            ("sale-1", "2024-02-01T00:00:00Z", "outbound", "0.015", "40000"),
        ]
        for algorithm in ("HIFO", "LOFO"):
            with self.subTest(algorithm=algorithm):
                result = self.assert_one_answer(rows, algorithm=algorithm)
                self.assertEqual([entry[-1] for entry in result if entry[1] == "disposal"],
                                 [(("lot-y", 1_500_000_000),)])

    def test_austrian_alt_lots_follow_file_order(self):
        # Pre-2021 lots are Alt and leave FIFO in file order.
        rows = [
            ("lot-c", "2020-01-01T00:00:00Z", "inbound", "0.01", "3000"),
            ("lot-a", "2020-01-01T00:00:00Z", "inbound", "0.01", "1000"),
            ("lot-b", "2020-01-01T00:00:00Z", "inbound", "0.01", "2000"),
            ("sale-1", "2020-06-01T00:00:00Z", "outbound", "0.01", "40000"),
        ]
        result = self.assert_one_answer(rows, country="at")
        self.assertEqual([(entry[3], entry[-1]) for entry in result if entry[1] == "disposal"],
                         [("30.00", (("lot-c", 1_000_000_000),))])

    def test_rows_of_two_files_order_by_content_not_import_order(self):
        # lot-a and lot-b come from separate files, each at position 1, so
        # their position ties and the external id decides, not which file
        # came first. The second imported lot gets the smaller id each time.
        rows = ROWS[1:4]
        results = set()
        for files in ([[0], [1, 2]], [[1], [0, 2]]):
            with tempfile.TemporaryDirectory(prefix="kassiber-lot-order-") as tmp:
                results.add(tuple(build_book(Path(tmp).resolve(), rows, files=files, ranks=(1, 0, 2))))
        self.assertEqual(len(results), 1, results)
        self.assertEqual([entry[-1] for entry in next(iter(results)) if entry[1] == "disposal"],
                         [(("lot-a", 1_000_000_000),)])

    def test_latest_transaction_rate_is_the_last_same_time_row_in_file_order(self):
        # The latest time holds two prices; the file's last row supplies the
        # fallback rate, whatever the ids or external ids say.
        rows = [
            ("lot-a", "2024-01-01T00:00:00Z", "inbound", "0.01", "10000"),
            ("lot-z", "2024-05-01T00:00:00Z", "inbound", "0.01", "30000"),
            ("lot-y", "2024-05-01T00:00:00Z", "inbound", "0.01", "20000"),
        ]

        def rates(conn: sqlite3.Connection, profile_id: str):
            return (
                str(custody_journal.latest_transaction_rates_for_profile(conn, profile_id)["BTC"]),
                str(custody_journal.transaction_rate_quotes_for_profile(conn, profile_id)["BTC"]["rate"]),
                str(custody_journal.transaction_rate_quotes_for_profile(
                    conn, profile_id, at_or_before="2024-06-01T00:00:00Z")["BTC"]["rate"]),
            )

        self.assertEqual(self.assert_one_answer(rows, inspect=rates), ("20000", "20000", "20000"))

    def test_processed_books_need_a_rebuild_once(self):
        # A book built before the source order: its journals may hold a
        # random lot order. The order arrived with the current version.
        with tempfile.TemporaryDirectory(prefix="kassiber-lot-order-") as tmp:
            base = Path(tmp).resolve()
            build_book(base, ROWS[:4])
            conn = open_db(base / "data")
            unprocessed = core_accounts.create_profile(
                conn, conn.execute("SELECT id FROM workspaces").fetchone()[0],
                "Untouched", "EUR", "FIFO", "generic", 365)
            conn.execute("DELETE FROM schema_migration_audits WHERE migration_name = ?",
                         (SAME_TIME_ORDER_SEMANTICS_MIGRATION,))
            conn.execute("UPDATE profiles SET required_journal_semantics_version = 1, "
                         "journal_semantics_version = ? WHERE last_processed_at IS NOT NULL",
                         (JOURNAL_CALCULATION_SEMANTICS_VERSION - 1,))
            conn.commit()
            before = dict(conn.execute("SELECT * FROM profiles WHERE label = 'Book'").fetchone())
            entries = conn.execute("SELECT * FROM journal_entries ORDER BY id").fetchall()
            conn.close()
            for _ in range(2):
                conn = open_db(base / "data")
                try:
                    after = dict(conn.execute("SELECT * FROM profiles WHERE label = 'Book'").fetchone())
                    self.assertEqual(after, {**before, "required_journal_semantics_version": JOURNAL_CALCULATION_SEMANTICS_VERSION})
                    freshness = custody_journal.projection_freshness(conn, after["id"])
                    self.assertEqual((freshness["status"], freshness["reason"]), (
                        "stale", "journal calculation semantics changed; rebuild with current Kassiber"))
                    self.assertEqual(conn.execute(
                        "SELECT required_journal_semantics_version FROM profiles WHERE id = ?",
                        (unprocessed["id"],)).fetchone()[0], 0)
                    self.assertEqual(conn.execute("SELECT * FROM journal_entries ORDER BY id").fetchall(), entries)
                    impacts = conn.execute(
                        "SELECT impact_json FROM schema_migration_audits WHERE migration_name = ?",
                        (SAME_TIME_ORDER_SEMANTICS_MIGRATION,)).fetchall()
                    self.assertEqual([json.loads(row[0]) for row in impacts], [{
                        "affected_profile_count": 1, "migration": SAME_TIME_ORDER_SEMANTICS_MIGRATION,
                        "required_journal_semantics_version": JOURNAL_CALCULATION_SEMANTICS_VERSION, "schema_version": 1}])
                finally:
                    conn.close()
            conn = open_db(base / "data")
            try:
                handlers.process_journals(conn, "Main", "Book")
                profile = conn.execute("SELECT * FROM profiles WHERE label = 'Book'").fetchone()
                self.assertEqual(profile["journal_semantics_version"], JOURNAL_CALCULATION_SEMANTICS_VERSION)
                self.assertTrue(custody_journal.projection_freshness(conn, profile["id"])["is_current"])
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
