"""The Rust shell keeps and re-binds reset decisions exactly as Python does.

One book holding every decision type is built with the Python reference
(`tests/test_book_reset_keeps_decisions.py`), then copied. Each copy is reset
and resynced by one runtime, with one transaction missing and one changed, and
journals are rebuilt. The reset and sync envelopes, the kept reset records and
every decision, transaction and journal row must match after masking only
generated identifiers and the clock.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import sqlite3
from pathlib import Path

import pytest

from tests import cli_runner
from tests import test_book_reset_keeps_decisions as scenario
from tests.test_rust_shell_parity import RUST_BIN, RUST_SKIP

pytestmark = pytest.mark.skipif(not RUST_BIN, reason=RUST_SKIP)

UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
# Every date the scenario writes itself is before 2026; later ones are clocks.
CLOCK = re.compile(r"20(?:2[6-9]|[3-9]\d)-\d\d-\d\dT[0-9:.]+(?:Z|\+00:00)")
TABLES = (
    "book_reset_transactions",
    "book_reset_parked_rows",
    "transactions",
    "journal_entries",
    "journal_quarantines",
    *scenario.DECISION_TABLES,
)


def _normalize(text: str, names: dict[str, str]) -> str:
    for transaction_id, name in names.items():
        text = text.replace(transaction_id, name)
    return CLOCK.sub("<clock>", UUID.sub("<uuid>", text))


def _dump(data_root: Path) -> dict:
    conn = sqlite3.connect(data_root / "kassiber.sqlite3")
    try:
        names = {
            str(row[0]): f"tx:{row[1]}"
            for row in conn.execute(
                "SELECT id, external_id FROM transactions "
                "UNION SELECT transaction_id, external_id FROM book_reset_transactions"
            )
        }
        out = {}
        for table in TABLES:
            cursor = conn.execute(f"SELECT * FROM {table}")
            columns = [item[0] for item in cursor.description]
            rows = [
                _normalize(json.dumps(dict(zip(columns, row)), sort_keys=True), names)
                for row in cursor.fetchall()
            ]
            out[table] = sorted(rows)
        return {"tables": out, "names": names}
    finally:
        conn.close()


def _run(book: scenario.BookResetKeepsDecisionsTest, data_root: Path, rust: bool) -> dict:
    env = dict(book.env)
    if rust:
        env[cli_runner.CLI_ENV] = str(Path(RUST_BIN).resolve())
    else:
        env.pop(cli_runner.CLI_ENV, None)
    book.data_root, book.env = data_root, env
    summary = book.reset()
    reset_tables = _dump(data_root)
    scenario._write_csv(book.b_csv, [row for row in scenario.B_ROWS if row[1] != "b-in"])
    scenario._write_csv(
        book.a_csv,
        [(*row[:3], "0.25000000", *row[4:]) if row[1] == "a-loan" else row for row in scenario.A_ROWS],
    )
    synced = book.cli("wallets", "sync", "--all")
    book.cli("journals", "process")
    final = _dump(data_root)
    names = final["names"]
    scenario._write_csv(book.a_csv, scenario.A_ROWS)
    scenario._write_csv(book.b_csv, scenario.B_ROWS)
    wipe = book.reset(wipe_everything=True)
    return {
        "reset": _normalize(json.dumps(summary, sort_keys=True), reset_tables["names"]),
        "after_reset": reset_tables["tables"],
        "sync": [
            _normalize(json.dumps({k: item.get(k) for k in ("wallet", "imported", "reset_decisions")}, sort_keys=True), names)
            for item in synced
        ],
        "after_sync": final["tables"],
        "wipe": _normalize(json.dumps(wipe, sort_keys=True), names),
        "after_wipe": _dump(data_root)["tables"],
    }


def test_rust_reset_and_rebind_match_python(monkeypatch):
    # The seed book is built by the Python reference, so only the reset,
    # resync and journal steps differ between the two copies.
    monkeypatch.delenv(cli_runner.CLI_ENV, raising=False)
    # The scenario's own setUp builds the book; none of its tests runs here.
    book = scenario.BookResetKeepsDecisionsTest("test_wipe_everything_still_deletes_decisions")
    book.setUp()
    try:
        seed = book.root / "seed"
        python_root = book.root / "python"
        rust_root = book.root / "rust"
        shutil.copytree(book.data_root, seed)
        for root in (python_root, rust_root):
            shutil.copytree(seed, root)
        python = _run(book, python_root, rust=False)
        rust = _run(book, rust_root, rust=True)
    finally:
        book.doCleanups()
    assert python["reset"] == rust["reset"]
    assert python["after_reset"] == rust["after_reset"]
    assert python["sync"] == rust["sync"]
    assert python["after_sync"] == rust["after_sync"]
    assert python["wipe"] == rust["wipe"]
    assert python["after_wipe"] == rust["after_wipe"]
    assert len(python["after_reset"]["book_reset_parked_rows"]) > 10
    assert len(python["after_reset"]["book_reset_transactions"]) == 7
    # The scenario exercises a missing and a changed transaction.
    holds = [row for row in python["after_sync"]["journal_quarantines"] if "book_reset_decision_held" in row]
    assert len(holds) == 2, holds
