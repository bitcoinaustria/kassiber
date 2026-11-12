"""Whole-process parity for the registered Privacy Mirror report.

`reports privacy-mirror` and `ui.reports.privacy_mirror` run in the Rust
binary itself (an unregistered leaf would answer `not_implemented`) and in
the unchanged Python CLI, each on its own copy of one book. The index's
identities and clock come from the shared test seams
(`KASSIBER_RUST_TEST_ANALYSIS_*`, bound for Python by
``tests/rust_ai_tool_reference.py``). Every step compares stdout bytes and
all 113 tables, through cold, warm and incremental publications, with the
product offline switch off and on and the test socket guard armed: the
Mirror is local analysis and must not reach any network.
"""
from __future__ import annotations

from contextlib import closing
import importlib
import json
import os
import re
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest

from kassiber.db import open_db
from kassiber.secrets.sqlcipher import open_encrypted
from tests.test_rust_shell_parity import _environment
from tests.rust_port.book_schema import TABLES

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
REFERENCE = ROOT / "tests" / "rust_ai_tool_reference.py"
BOOK = ROOT / "crates/reports/tests/data/book.sql"


_HANDLE = re.compile(rb"ca-ref:[0-9a-f]{64}")


def _handles(stdout):
    names = {}
    return _HANDLE.sub(lambda match: names.setdefault(match.group(0), b"ca-ref:#%d" % len(names)), stdout)


def _sources():
    scripts = str(ROOT / "crates/wallets/scripts")
    if scripts not in sys.path:
        sys.path.insert(0, scripts)
    return importlib.import_module("generate_analysis_sources")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustPrivacyMirrorProcessTest(unittest.TestCase):
    passphrase = None

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-mirror-process-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.env = _environment(self.base / "preferences")
        self.env.update({
            "HOME": os.environ["HOME"],
            "KASSIBER_STATE_ROOT": str(self.base / "state"),
            "KASSIBER_TEST_NO_EGRESS": "1",
            "KASSIBER_RUST_TEST_ANALYSIS_IDS": "sequential",
            "KASSIBER_RUST_TEST_ANALYSIS_TIME": "2026-01-01T00:00:00Z",
            "KASSIBER_RUST_TEST_OWNER_ROOT": str(self.base / "owners"),
        })
        (self.base / "owners").mkdir(mode=0o700)

    def book(self, root, sql):
        for script in (BOOK.read_text() + sql, ""):
            with closing(open_db(root, passphrase=self.passphrase)) as conn, conn:
                conn.executescript(script)

    def change(self, roots, sql):
        for root in roots:
            with closing(open_db(root, passphrase=self.passphrase)) as conn, conn:
                conn.executescript(sql)

    def snapshot(self, root):
        path = root / "kassiber.sqlite3"
        conn = open_encrypted(path, self.passphrase) if self.passphrase else sqlite3.connect(path)
        try:
            tables = sorted(row[0] for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"))
            self.assertEqual(tables, list(TABLES))
            return {table: sorted(conn.execute('SELECT * FROM "' + table + '"').fetchall(), key=repr)
                    for table in tables}
        finally:
            conn.close()

    def run_side(self, prefix, root, args, *, wire=None, offline):
        env = dict(self.env)
        if offline:
            env["KASSIBER_NO_EGRESS"] = "1"
        else:
            env.pop("KASSIBER_NO_EGRESS", None)
        argv = ["--data-root", str(root)]
        fds = []
        if self.passphrase:
            read_end, write_end = os.pipe()
            os.write(write_end, self.passphrase.encode())
            os.close(write_end)
            fds.append(read_end)
            argv += ["--db-passphrase-fd", str(read_end)]
        try:
            result = subprocess.run([*prefix, *argv, *args], env=env, input=wire, capture_output=True,
                                    timeout=300, cwd=ROOT, pass_fds=fds)
        finally:
            for fd in fds:
                os.close(fd)
        return result.returncode, result.stdout, result.stderr

    def compare(self, sql, steps, *, offline):
        """Run every step on both copies; ``steps`` are (sql, argv, wire)."""
        python_root, rust_root = self.base / f"python-{offline}", self.base / f"rust-{offline}"
        for root in (python_root, rust_root):
            shutil.rmtree(root, ignore_errors=True)
        # One book, copied: opening stamps migration audits with the clock.
        self.book(python_root, sql)
        shutil.copytree(python_root, rust_root)
        outcomes = []
        for change, args, wire in steps:
            if change:
                self.change((python_root, rust_root), change)
            expected = self.run_side([sys.executable, str(REFERENCE)], python_root, args, wire=wire,
                                     offline=offline)
            actual = self.run_side([str(Path(RUST_BIN).resolve())], rust_root, args, wire=wire,
                                   offline=offline)
            if wire is not None:
                # The ready line advertises each runtime's own kinds.
                expected = (expected[0], b"\n".join(expected[1].splitlines()[1:]), expected[2])
                ready = json.loads(actual[1].splitlines()[0])["data"]["supported_kinds"]
                self.assertIn("ui.reports.privacy_mirror", ready)
                actual = (actual[0], b"\n".join(actual[1].splitlines()[1:]), actual[2])
            # Redacted handles are keyed by a per-process secret: each side's
            # are renamed by first appearance, so one subject keeps one name.
            expected = (expected[0], _handles(expected[1]), expected[2])
            actual = (actual[0], _handles(actual[1]), actual[2])
            self.assertEqual(actual, expected, args)
            self.assertEqual(self.snapshot(rust_root), self.snapshot(python_root), args)
            outcomes.append(actual)
        return outcomes

    def steps(self, sources):
        cli = lambda *flags: (None, ["--machine", *flags, "reports", "privacy-mirror"], None)
        wire = b"".join(json.dumps(request).encode() + b"\n" for request in [
            {"request_id": "1", "kind": "ui.reports.privacy_mirror"},
            {"request_id": "2", "kind": "ui.reports.privacy_mirror", "args": None},
            {"request_id": "3", "kind": "ui.reports.privacy_mirror", "args": {}},
            {"request_id": "4", "kind": "ui.reports.privacy_mirror", "args": "x"},
            {"request_id": "5", "kind": "ui.reports.privacy_mirror", "args": {"wallet": "cold", "redacted": False}},
            {"request_id": "stop", "kind": "daemon.shutdown"},
        ])
        return [
            cli(),  # cold publication
            cli(),  # warm
            (None, ["--format", "table", "reports", "privacy-mirror"], None),
            (None, ["--format", "plain", "reports", "privacy-mirror"], None),
            (None, ["--format", "csv", "reports", "privacy-mirror"], None),
            (None, ["--machine", "reports", "privacy-mirror", "--workspace", "Main", "--profile", "Book"], None),
            (None, ["--machine", "reports", "privacy-mirror", "--profile", "absent"], None),
            (None, ["--machine", "daemon"], wire),
            ("UPDATE wallet_utxos SET amount=9000 WHERE id='utxo';", ["--machine", "reports", "privacy-mirror"], None),
            ("UPDATE chain_analysis_datasets SET status='revoked';", ["--machine", "daemon"], wire),
        ]

    def test_owned_book_with_claims_matches_python_offline_off_and_on(self):
        sources = _sources()
        sql = sources.stored() + sources.inventory() + sources.dataset()
        for offline in (False, True):
            with self.subTest(offline=offline):
                outcomes = self.compare(sql, self.steps(sources), offline=offline)
                self.assertEqual(outcomes[0][0], 0, outcomes[0][1][-2000:])
                report = json.loads(outcomes[0][1])
                self.assertEqual(report["kind"], "reports.privacy-mirror")
                # Not vacuous: dataset claims become findings, and the owned
                # spend is evaluated for entropy, behind redacted handles.
                self.assertGreaterEqual(report["data"]["summary"]["finding_count"], 1)
                self.assertGreaterEqual(report["data"]["entropy"]["evaluated"], 1)
                self.assertEqual(report["data"]["redaction"], "ai_export_safe")
                self.assertIn(b"ca-ref:#0", outcomes[0][1])

    def test_empty_and_stored_books_match_python(self):
        sources = _sources()
        for sql in ("", sources.stored()):
            with self.subTest(sql=bool(sql)):
                self.compare(sql, self.steps(sources)[:2] + self.steps(sources)[7:8], offline=True)

    def test_sqlcipher_book_matches_python(self):
        self.passphrase = "synthetic mirror key"
        sources = _sources()
        self.compare(sources.stored() + sources.inventory(), self.steps(sources)[:3], offline=False)


if __name__ == "__main__":
    unittest.main()
