"""Repro: FIFO lot order among same-timestamp acquisitions followed random row ids.

Two books built from the same import file and processed the same way must
book the same gains. Custody composition ordered transactions by
`occurred_at, created_at, id` and the tax projection by row ids that commit to
random wallet ids, so rows imported in one second booked 100, 200 or 300 EUR
for one sale. Found by `sync-benchmark --runtimes python,python`. Both
runtimes now follow the source order (docs/reference/tax.md#same-time-order),
and the Rust binary (`KASSIBER_RUST_BIN`) books byte-identical journals.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import sqlite3
import tempfile
import unittest

from tests import cli_runner
from tests.rust_fixture_binary import checked
from tests.test_rust_shell_parity import _environment

ROWS = [
    ("lot-a", "2024-01-01T00:00:00Z", "inbound", "0.01", "10000"),
    ("lot-b", "2024-01-01T00:00:00Z", "inbound", "0.01", "20000"),
    ("lot-c", "2024-01-01T00:00:00Z", "inbound", "0.01", "30000"),
    ("sale", "2024-02-01T00:00:00Z", "outbound", "0.01", "40000"),
]
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
# Columns a build stamps with its own clock.
CLOCK_COLUMNS = {"created_at", "processed_at", "rebuilt_at", "updated_at"}
# Tables every build replaces whole, with fresh random row ids in both
# runtimes (docs/reference/r4-journal-pipeline.md); their content must match.
RANDOM_ID_TABLES = {"journal_account_holdings", "journal_tax_summary", "journal_wallet_holdings"}


def _env(base: Path, rust: str | None = None) -> dict[str, str]:
    env = _environment(base / "home")
    env.pop("KASSIBER_NO_EGRESS", None)
    env["KASSIBER_STATE_ROOT"] = str(base / "home" / "state")
    env["TZ"] = "UTC"
    if rust:
        env["KASSIBER_TEST_CLI"] = rust
    else:
        env.pop("KASSIBER_TEST_CLI", None)
    return env


def _cli(root: Path, env: dict[str, str], *args, rust: bool = False):
    result = cli_runner.run(["--machine", "--data-root", str(root), *args], reference=not rust, env=env,
                            capture_output=True, text=True, timeout=300)
    assert result.returncode == 0, result.stdout + result.stderr
    return json.loads(result.stdout)["data"]


def _imported(base: Path, name: str, env: dict[str, str], *, rust: bool = False) -> Path:
    root = base / name / "data"
    source = base / name / "rows.csv"
    source.parent.mkdir(parents=True)
    source.write_text("txid,occurred_at,direction,asset,amount,fee,fiat_rate\n" + "".join(
        f"{txid},{at},{direction},BTC,{amount},0,{rate}\n" for txid, at, direction, amount, rate in ROWS))
    _cli(root, env, "init", rust=rust)
    _cli(root, env, "workspaces", "create", "Main", rust=rust)
    _cli(root, env, "profiles", "create", "Book", "--workspace", "Main", "--fiat-currency", "EUR",
         "--tax-country", "generic", "--gains-algorithm", "FIFO", rust=rust)
    _cli(root, env, "wallets", "create", "--label", "Cold", "--kind", "custom", rust=rust)
    _cli(root, env, "wallets", "import-csv", "--wallet", "Cold", "--file", str(source), rust=rust)
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


class LotOrderDeterminismTest(unittest.TestCase):
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
        self.assertEqual(results, {(("sale", "disposal", "100.00", "300.00"),)})

    @unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the kassiber binary")
    def test_rust_books_byte_identical_journals(self):
        rust = str(checked(RUST_BIN, "cargo build -p kassiber --bin kassiber --offline --locked"))
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp).resolve()
            python_env, rust_env = _env(base), _env(base, rust)
            # One Python import, processed by each runtime.
            imported = _imported(base, "shared", python_env)
            native = base / "native" / "data"
            shutil.copytree(imported, native)
            _cli(imported, python_env, "journals", "process")
            _cli(native, rust_env, "journals", "process", rust=True)
            self.assertEqual(_journals(native), _journals(imported))
            # A Rust import records the same file positions, so it books the
            # same gains as a Python one.
            rust_book = _imported(base, "rust", rust_env, rust=True)
            _cli(rust_book, rust_env, "journals", "process", rust=True)
            self.assertEqual(_disposals(rust_book), _disposals(imported))
            positions = lambda root: sqlite3.connect(root / "kassiber.sqlite3").execute(
                "SELECT t.external_id, b.source_position FROM import_batch_transactions b "
                "JOIN transactions t ON t.id = b.transaction_id ORDER BY b.source_position").fetchall()
            self.assertEqual(positions(rust_book), positions(imported))
            self.assertEqual(positions(imported), [("lot-a", 1), ("lot-b", 2), ("lot-c", 3), ("sale", 4)])


if __name__ == "__main__":
    unittest.main()
