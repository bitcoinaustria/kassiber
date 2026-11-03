"""Shared harness for native desktop projections compared with the Python daemon.

Each comparison sends one JSONL session to the unchanged Python reference and
to the Rust candidate, requires identical response bytes, and checks that all
112 logical tables are unchanged by both. Served-by evidence is appended to
``KASSIBER_DESKTOP_COUNTS`` when set.
"""
import json
import os
import shutil
from pathlib import Path
import sqlite3
import subprocess
import tempfile

from kassiber.db import open_db
from kassiber.secrets.sqlcipher import open_encrypted
from tests import cli_runner
from tests.test_rust_shell_parity import _environment

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
DESKTOP_SQL = ROOT / "crates/transactions/tests/data/desktop.sql"


def run_candidate(args, *, env, input):
    """Run the Rust binary itself; ``cli_runner`` reads its switch from os.environ."""
    resolved, fds = [], []
    try:
        for arg in args:
            if isinstance(arg, cli_runner.InputFd):
                read_end, write_end = os.pipe()
                os.write(write_end, arg.data)
                os.close(write_end)
                fds.append(read_end)
                resolved.append(str(read_end))
            else:
                resolved.append(os.fspath(arg))
        return subprocess.run([RUST_BIN, *resolved], env=env, input=input, capture_output=True,
                              timeout=300, cwd=ROOT, pass_fds=fds)
    finally:
        for fd in fds:
            os.close(fd)


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

    def snapshot(self, root=None):
        path = (root or self.root) / "kassiber.sqlite3"
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

    def compare(self, args, *, kind=None, mutates=False, mask=None, tables=None):
        """Compare one session; ``mutates`` runs Rust on a copy taken before Python.

        ``mask`` normalizes both response lines and ``tables`` both snapshots when
        generated identifiers or clocks legitimately differ between the runs.
        """
        requests = self.requests(args, kind)
        requests.append(dict(request_id="stop", kind="daemon.shutdown"))
        wire = "".join(json.dumps(request) + "\n" for request in requests).encode()
        baseline = self.snapshot()
        candidate_root = self.root
        if mutates:
            candidate_root = self.base / f"candidate-{len(list(self.base.glob('candidate-*')))}" / "data"
            shutil.copytree(self.root, candidate_root)

        def argv(root):
            argv = ["--machine", "--data-root", str(root)]
            if self.passphrase:
                argv.extend(["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())])
            return [*argv, "daemon"]

        reference = cli_runner.run(argv(self.root), reference=True, env=self.env, input=wire,
                                   capture_output=True, timeout=300)
        self.assertEqual((reference.returncode, reference.stderr), (0, b""), reference.stdout)
        after_reference = self.snapshot()
        if not mutates:
            self.assertEqual(after_reference, baseline)
        candidate = run_candidate(argv(candidate_root), env=self.env, input=wire)
        self.assertEqual((candidate.returncode, candidate.stderr), (0, b""), candidate.stdout)
        actual, expected = candidate.stdout.splitlines(), reference.stdout.splitlines()
        ready = json.loads(actual[0])["data"]["supported_kinds"]
        self.assertEqual(ready[0], "status", "candidate must be the Rust daemon")
        self.assertIn(kind or self.KIND, ready)
        self.assertEqual(len(actual), len(expected))
        normalize = mask or (lambda line: line)
        for request, got, wanted in zip(requests, actual[1:], expected[1:]):
            self.assertEqual(normalize(got), normalize(wanted), request)
        if mutates:
            reshape = tables or (lambda snapshot: snapshot)
            self.assertEqual(reshape(self.snapshot(candidate_root)), reshape(after_reference))
        else:
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
