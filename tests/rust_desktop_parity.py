"""Shared harness for native desktop projections compared with the Python daemon.

Each comparison sends one JSONL session to the unchanged Python reference and
to the Rust candidate, requires identical response bytes, and checks that all
112 logical tables are unchanged by both. Served-by evidence is appended to
``KASSIBER_DESKTOP_COUNTS`` when set.
"""
import json
import os
from pathlib import Path
import sqlite3
import tempfile

from kassiber.db import open_db
from kassiber.secrets.sqlcipher import open_encrypted
from tests import cli_runner
from tests.test_rust_shell_parity import _environment

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
DESKTOP_SQL = ROOT / "crates/transactions/tests/data/desktop.sql"


class DesktopParity:
    """Mixin for ``unittest.TestCase``; set ``KIND`` and optionally ``SEED``."""

    KIND = ""
    SEED = DESKTOP_SQL
    passphrase = None

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-r5-desktop-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / "data"
        self.env = _environment(self.base / "preferences")
        self.env["HOME"] = os.environ["HOME"]
        self.env["KASSIBER_STATE_ROOT"] = str(self.base / "state")
        self.env["KASSIBER_TEST_NO_EGRESS"] = "1"
        self.env["KASSIBER_NO_EGRESS"] = "1"
        self.seed()

    def seed(self):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            if self.SEED is not None:
                conn.executescript(Path(self.SEED).read_text())
        self.change("")

    def encrypt(self, passphrase):
        self.root = self.base / "encrypted" / "data"
        self.passphrase = passphrase
        self.seed()

    def change(self, sql):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(sql)

    def snapshot(self):
        path = self.root / "kassiber.sqlite3"
        conn = open_encrypted(path, self.passphrase) if self.passphrase else sqlite3.connect(path)
        try:
            tables = sorted(row[0] for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"))
            self.assertEqual(len(tables), 112)
            return {table: sorted(conn.execute('SELECT * FROM "' + table + '"').fetchall(), key=repr)
                    for table in tables}
        finally:
            conn.close()

    def requests(self, args, kind=None):
        requests = []
        for index, arg in enumerate(args):
            request = dict(request_id=str(index), kind=kind or self.KIND)
            if arg is not ...:
                request["args"] = arg
            requests.append(request)
        return requests

    def compare(self, args, *, kind=None, mutates=False):
        requests = self.requests(args, kind)
        requests.append(dict(request_id="stop", kind="daemon.shutdown"))
        wire = "".join(json.dumps(request) + "\n" for request in requests).encode()
        argv = ["--machine", "--data-root", str(self.root)]
        if self.passphrase:
            argv.extend(["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())])
        argv.append("daemon")
        baseline = self.snapshot()
        reference = cli_runner.run(argv, reference=True, env=self.env, input=wire,
                                   capture_output=True, timeout=300)
        self.assertEqual((reference.returncode, reference.stderr), (0, b""), reference.stdout)
        after_reference = self.snapshot()
        if not mutates:
            self.assertEqual(after_reference, baseline)
        candidate = cli_runner.run(
            argv, env={**self.env, cli_runner.CLI_ENV: RUST_BIN}, input=wire,
            capture_output=True, timeout=300)
        self.assertEqual((candidate.returncode, candidate.stderr), (0, b""), candidate.stdout)
        actual, expected = candidate.stdout.splitlines(), reference.stdout.splitlines()
        self.assertIn(kind or self.KIND, json.loads(actual[0])["data"]["supported_kinds"])
        self.assertEqual(len(actual), len(expected))
        for request, got, wanted in zip(requests, actual[1:], expected[1:]):
            self.assertEqual(got, wanted, request)
        if not mutates:
            self.assertEqual(self.snapshot(), baseline)
        responses = [json.loads(line) for line in actual[1:-1]]
        if path := os.environ.get("KASSIBER_DESKTOP_COUNTS"):
            with open(path, "a", encoding="utf-8") as handle:
                for request, response in zip(requests, responses):
                    handle.write(json.dumps(dict(
                        kind=request["kind"], served_by="rust", request_id=response["request_id"],
                        ok=response["kind"] != "error",
                        error_code=response.get("error", {}).get("code"))) + "\n")
        return responses
