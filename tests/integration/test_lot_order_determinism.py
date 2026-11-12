"""Repro: FIFO lot order among same-timestamp acquisitions followed random row ids.

Two books built from the same import file and processed the same way must
book the same gains. Custody composition ordered transactions by
`occurred_at, created_at, id` and the tax projection by row ids that commit to
random wallet ids, so rows imported in one second booked 100, 200 or 300 EUR
for one sale. Found by `sync-benchmark --runtimes python,python`. Both
runtimes followed the source order (docs/reference/tax.md#same-time-order).

The Python half is frozen (docs/plan/22-python-removal.md, D5):
`goldens/lot_order_imported.sqlite3.gz` is the book a Python import left
before processing, and `goldens/lot_order_journals.json` every journal table
Python's `journals process` then wrote. The Rust binary must book
byte-identical journals from that book. `KASSIBER_RECORD_GOLDENS=1` rewrites
both from the binary under test after a reviewed change.
"""
from __future__ import annotations

import gzip
import json
import os
from pathlib import Path
import shutil
import sqlite3
import tempfile
import unittest

from tests.integration import cli
from tests.integration.runtime_parity import GOLDENS, child_env, jsonable, load_golden, write_golden

ROWS = [
    ("lot-a", "2024-01-01T00:00:00Z", "inbound", "0.01", "10000"),
    ("lot-b", "2024-01-01T00:00:00Z", "inbound", "0.01", "20000"),
    ("lot-c", "2024-01-01T00:00:00Z", "inbound", "0.01", "30000"),
    ("sale", "2024-02-01T00:00:00Z", "outbound", "0.01", "40000"),
]
RECORD = os.environ.get("KASSIBER_RECORD_GOLDENS") == "1"
IMPORTED_BOOK = "lot_order_imported.sqlite3.gz"
JOURNALS = "lot_order_journals.json"
# Columns a build stamps with its own clock.
CLOCK_COLUMNS = {"created_at", "processed_at", "rebuilt_at", "updated_at"}
# Tables every build replaces whole, with fresh random row ids
# (docs/reference/r4-journal-pipeline.md); their content must match.
RANDOM_ID_TABLES = {"journal_account_holdings", "journal_tax_summary", "journal_wallet_holdings"}


def _env(base: Path) -> dict[str, str]:
    return child_env(base / "home", rust_bin=cli.binary())


def _cli(root: Path, env: dict[str, str], *args):
    result = cli.run(["--machine", "--data-root", str(root), *args], env=env,
                     capture_output=True, text=True, timeout=300)
    assert result.returncode == 0, result.stdout + result.stderr
    return json.loads(result.stdout)["data"]


def _imported(base: Path, name: str, env: dict[str, str]) -> Path:
    root = base / name / "data"
    source = base / name / "rows.csv"
    source.parent.mkdir(parents=True)
    source.write_text("txid,occurred_at,direction,asset,amount,fee,fiat_rate\n" + "".join(
        f"{txid},{at},{direction},BTC,{amount},0,{rate}\n" for txid, at, direction, amount, rate in ROWS))
    _cli(root, env, "init")
    _cli(root, env, "workspaces", "create", "Main")
    _cli(root, env, "profiles", "create", "Book", "--workspace", "Main", "--fiat-currency", "EUR",
         "--tax-country", "generic", "--gains-algorithm", "FIFO")
    _cli(root, env, "wallets", "create", "--label", "Cold", "--kind", "custom")
    _cli(root, env, "wallets", "import-csv", "--wallet", "Cold", "--file", str(source))
    return root


def _disposals(root: Path) -> list[tuple]:
    conn = sqlite3.connect(root / "kassiber.sqlite3")
    try:
        return conn.execute(
            "SELECT t.external_id, j.entry_type, j.cost_basis_exact, j.gain_loss_exact "
            "FROM journal_entries j JOIN transactions t ON t.id = j.transaction_id "
            "WHERE j.entry_type = 'disposal' ORDER BY t.external_id").fetchall()
    finally:
        conn.close()


def _positions(root: Path) -> list[tuple]:
    conn = sqlite3.connect(root / "kassiber.sqlite3")
    try:
        return conn.execute(
            "SELECT t.external_id, b.source_position FROM import_batch_transactions b "
            "JOIN transactions t ON t.id = b.transaction_id ORDER BY b.source_position").fetchall()
    finally:
        conn.close()


def _journals(root: Path) -> dict[str, list[tuple]]:
    """Every journal table in a stable row order, without build clocks."""
    conn = sqlite3.connect(root / "kassiber.sqlite3")
    try:
        tables = [row[0] for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'journal\\_%' ESCAPE '\\' ORDER BY name")]
        dumped = {}
        for table in tables:
            columns = [row[1] for row in conn.execute(f'PRAGMA table_info("{table}")')
                       if row[1] not in CLOCK_COLUMNS and not (table in RANDOM_ID_TABLES and row[1] == "id")]
            dumped[table] = conn.execute(
                f'SELECT {",".join(chr(34) + c + chr(34) for c in columns)} FROM "{table}" '
                f'ORDER BY {",".join(str(n) for n in range(1, len(columns) + 1))}').fetchall()
        dumped["profiles"] = conn.execute(
            "SELECT journal_semantics_version, required_journal_semantics_version, last_processed_tx_count "
            "FROM profiles ORDER BY id").fetchall()
        return dumped
    finally:
        conn.close()


def _restore_book(target: Path) -> Path:
    target.mkdir(parents=True)
    (target / "kassiber.sqlite3").write_bytes(gzip.decompress((GOLDENS / IMPORTED_BOOK).read_bytes()))
    return target


@unittest.skipUnless(cli.configured(), cli.NO_BINARY)
class LotOrderDeterminismTest(unittest.TestCase):
    maxDiff = None

    def test_same_import_books_the_same_gains(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp).resolve()
            env = _env(base)
            # Each book used to pick one of three orderings; ten books all
            # agreeing by chance was about one run in 20,000.
            results = set()
            for index in range(10):
                root = _imported(base, f"book-{index}", env)
                _cli(root, env, "journals", "process")
                results.add(tuple(_disposals(root)))
                self.assertEqual(_positions(root), [("lot-a", 1), ("lot-b", 2), ("lot-c", 3), ("sale", 4)])
        self.assertEqual(results, {(("sale", "disposal", "100.00", "300.00"),)})

    def test_frozen_import_books_byte_identical_journals(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp).resolve()
            env = _env(base)
            if RECORD:
                imported = _imported(base, "shared", env)
                conn = sqlite3.connect(imported / "kassiber.sqlite3")
                conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
                conn.close()
                GOLDENS.mkdir(parents=True, exist_ok=True)
                (GOLDENS / IMPORTED_BOOK).write_bytes(
                    gzip.compress((imported / "kassiber.sqlite3").read_bytes(), mtime=0))
            book = _restore_book(base / "frozen" / "data")
            _cli(book, env, "journals", "process")
            journals = {"journals": _journals(book), "disposals": _disposals(book), "positions": _positions(book)}
            if RECORD:
                write_golden(JOURNALS, journals)
                self.skipTest(f"recorded {JOURNALS} from {cli.binary()}")
            expected = load_golden(JOURNALS)
            self.assertEqual(jsonable(journals), expected)
            self.assertEqual(expected["positions"], [["lot-a", 1], ["lot-b", 2], ["lot-c", 3], ["sale", 4]])
            # A fresh import records the same file positions, so it books the
            # same gains as the frozen one.
            fresh = _imported(base, "fresh", env)
            _cli(fresh, env, "journals", "process")
            self.assertEqual(jsonable(_disposals(fresh)), expected["disposals"])
            self.assertEqual(jsonable(_positions(fresh)), expected["positions"])
            shutil.rmtree(base / "frozen")


if __name__ == "__main__":
    unittest.main()
