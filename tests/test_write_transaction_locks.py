"""Write transactions hold SQLite's writer lock before their first read.

A transaction begun deferred (a bare SAVEPOINT outside a transaction, or a
plain BEGIN) takes a shared lock with its reads. When another connection holds
the writer lock (a daemon worker such as the local-evidence watch worker,
another Kassiber process), the first write's upgrade fails with SQLITE_BUSY at
once: SQLite skips the busy handler because waiting could deadlock, so
busy_timeout never applies and the request ends as database_busy. That is how
an encrypted book's document import failed right after unlock.

Pinned here: `begin_write_savepoint` waits for a held writer and still commits
on an outermost RELEASE, while the bare form fails at once (both journal
modes, both drivers); and no product transaction opener begins deferred unless
it is a named read-only snapshot or a one-time open migration.
"""

from __future__ import annotations

import ast
import tempfile
import threading
import time
import unittest
from pathlib import Path

from kassiber.db import begin_write_savepoint, open_db

ROOT = Path(__file__).resolve().parent.parent

# Openers that may begin deferred, by (module, enclosing function). Read-only
# snapshots never upgrade, so they cannot hit the refused upgrade. One-time
# migrations run while the book opens, before the daemon starts its workers;
# a worker's own open then finds them done.
DEFERRED_ALLOWED = {
    ("kassiber/core/report_explanation.py", "explain_capital_gain"): "read-only snapshot",
    ("kassiber/core/source_funds_review.py", "_read_snapshot"): "read-only snapshot",
    ("kassiber/core/review_workflow.py", "inspect_cases"): "read-only snapshot",
    ("kassiber/core/review_workflow.py", "request_input"): "read-only snapshot, rolled back",
    ("kassiber/core/chain_observer/contract.py", "prepare_observer_update"): "read-only preparation",
    ("kassiber/core/privacy_linkage.py", "build_privacy_linkage_graph"): "read-only snapshot",
    ("kassiber/core/source_funds_coverage.py", "compute_coverage"): "read-only snapshot",
    ("kassiber/core/book_network.py", "read_network_snapshot"): "read-only snapshot",
    ("kassiber/core/chain_analysis/index.py", "build_index"): "read-only snapshot",
    ("kassiber/db.py", "begin_write_savepoint"): "the helper itself",
    ("kassiber/db.py", "_ensure_custody_gap_review_transaction_schema"): "one-time open migration",
    ("kassiber/db.py", "_ensure_transaction_page_index"): "open migration, read-only once applied",
    ("kassiber/db.py", "_ensure_custody_projection_table_shapes"): "one-time open migration",
    ("kassiber/chain_analysis_watches_schema.py", "ensure_inbox_sources"): "one-time open migration",
    ("kassiber/core/legacy_ledger.py", "_drop_all"): "one-time open migration",
}


def _opener(node):
    """The literal head of `<x>.execute("SAVEPOINT ..."/"BEGIN ...")`, or None."""
    if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
            and node.func.attr == "execute" and node.args):
        return None
    first = node.args[0]
    if isinstance(first, ast.JoinedStr) and first.values and isinstance(first.values[0], ast.Constant):
        first = first.values[0]
    if not (isinstance(first, ast.Constant) and isinstance(first.value, str)):
        return None
    words = first.value.split()
    if words and words[0].upper() in {"SAVEPOINT", "BEGIN"}:
        return " ".join(words[:2]).upper() if words[0].upper() == "BEGIN" else "SAVEPOINT"
    return None


def deferred_openers():
    """(module, function, opener) for each transaction opener that may begin deferred."""
    found = []
    for path in sorted((ROOT / "kassiber").rglob("*.py")):
        module = path.relative_to(ROOT).as_posix()
        tree = ast.parse(path.read_text(encoding="utf-8"))
        for function in ast.walk(tree):
            if not isinstance(function, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            nested = {id(node) for inner in ast.walk(function) if inner is not function
                      and isinstance(inner, (ast.FunctionDef, ast.AsyncFunctionDef))
                      for node in ast.walk(inner)}
            calls = [node for node in ast.walk(function) if id(node) not in nested]
            openers = [_opener(node) for node in calls]
            immediate = "BEGIN IMMEDIATE" in openers
            for opener in openers:
                if opener == "BEGIN IMMEDIATE" or opener is None:
                    continue
                # A savepoint behind this function's own BEGIN IMMEDIATE (when
                # no caller transaction is open) already holds the writer.
                if opener == "SAVEPOINT" and immediate:
                    continue
                found.append((module, function.name, opener))
    return found


class WriteSavepointTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="write-savepoint-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)

    def hold_writer(self, data_root, passphrase, seconds):
        held = threading.Event()

        def run():
            conn = open_db(data_root, passphrase=passphrase)
            try:
                conn.execute("BEGIN IMMEDIATE")
                held.set()
                time.sleep(seconds)
                conn.commit()
            finally:
                conn.close()

        thread = threading.Thread(target=run)
        thread.start()
        self.addCleanup(thread.join, 60)
        self.assertTrue(held.wait(60))

    def test_a_write_savepoint_waits_for_the_writer_and_the_bare_one_does_not(self):
        for passphrase in (None, "synthetic key"):
            with self.subTest(encrypted=passphrase is not None):
                data_root = self.base / ("encrypted" if passphrase else "plaintext")
                conn = open_db(data_root, passphrase=passphrase)
                self.addCleanup(conn.close)

                self.hold_writer(data_root, passphrase, 1.0)
                conn.execute("SAVEPOINT bare")
                conn.execute("SELECT COUNT(*) FROM settings").fetchone()
                started = time.monotonic()
                with self.assertRaisesRegex(Exception, "locked|busy"):
                    conn.execute("INSERT INTO settings(key, value) VALUES('probe', 'bare')")
                self.assertLess(time.monotonic() - started, 0.5)
                conn.execute("ROLLBACK TO SAVEPOINT bare")
                conn.execute("RELEASE SAVEPOINT bare")
                time.sleep(1.2)

                self.hold_writer(data_root, passphrase, 1.0)
                started = time.monotonic()
                begin_write_savepoint(conn, "reserved")
                self.assertGreaterEqual(time.monotonic() - started, 0.5)
                conn.execute("SELECT COUNT(*) FROM settings").fetchone()
                conn.execute("INSERT INTO settings(key, value) VALUES('probe', 'reserved')")
                conn.execute("RELEASE SAVEPOINT reserved")
                self.assertFalse(conn.in_transaction)
                with open_db(data_root, passphrase=passphrase) as other:
                    self.assertEqual(
                        other.execute("SELECT value FROM settings WHERE key = 'probe'").fetchone()[0],
                        "reserved",
                    )

    def test_a_refused_reservation_leaves_no_savepoint_open(self):
        data_root = self.base / "plaintext"
        conn = open_db(data_root)
        self.addCleanup(conn.close)
        conn.execute("PRAGMA busy_timeout = 0")
        self.hold_writer(data_root, None, 1.0)
        with self.assertRaisesRegex(Exception, "locked|busy"):
            begin_write_savepoint(conn, "refused")
        self.assertFalse(conn.in_transaction)


class TransactionOpenerTest(unittest.TestCase):
    def test_writes_never_begin_deferred(self):
        unexpected = sorted({(module, function, opener) for module, function, opener in deferred_openers()
                             if (module, function) not in DEFERRED_ALLOWED})
        self.assertEqual(unexpected, [], "use begin_write_savepoint or BEGIN IMMEDIATE for a write transaction")

    def test_the_allowlist_names_live_openers(self):
        live = {(module, function) for module, function, _ in deferred_openers()}
        self.assertEqual(sorted(set(DEFERRED_ALLOWED) - live), [])


if __name__ == "__main__":
    unittest.main()
