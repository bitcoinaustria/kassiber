"""Python refuses the desktop exchange connection kinds only Rust serves.

`ui.connections.exchange.{status,test,create,sync}` are Rust-only daemon
kinds (docs/reference/exchange-desktop.md). Python's daemon must refuse each
one with an error envelope, without writing the book and without contacting
anything (the test socket guard fails any egress), and must not advertise
them, so a desktop running the Python backend explains instead of failing.
"""

import hashlib
import json
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path

from kassiber.daemon import SUPPORTED_KINDS
from tests import cli_runner

KINDS = (
    "ui.connections.exchange.status",
    "ui.connections.exchange.test",
    "ui.connections.exchange.create",
    "ui.connections.exchange.sync",
)
ARGS = {
    "ui.connections.exchange.status": {},
    "ui.connections.exchange.test": {"exchange": "kraken", "api_key": "k", "api_secret": "s"},
    "ui.connections.exchange.create": {"exchange": "kraken", "api_key": "k", "api_secret": "s"},
    "ui.connections.exchange.sync": {"wallet": "Kraken"},
}


class PythonRefusesExchangeDesktopKindsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-python-exchange-desktop-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        # Children must not resolve the owner's real app-data root, and the
        # test socket guard fails any egress.
        self.env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
        self.env["KASSIBER_OFFLINE_PREFERENCE_FILE"] = str(Path(self.tmp.name) / "offline-mode.json")
        self.env["KASSIBER_TEST_NO_EGRESS"] = "1"
        for args in (("init",), ("workspaces", "create", "Main"),
                     ("profiles", "create", "Book", "--fiat-currency", "EUR")):
            result = self.run_cli(*args)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def run_cli(self, *args, wire=None):
        result = cli_runner.run(
            ["--data-root", str(self.data_root), "--machine", *args],
            reference=True,
            capture_output=True,
            text=True,
            env=self.env,
            input=wire,
            check=False,
        )
        self.assertNotIn("Traceback", result.stderr)
        return result

    def digest(self) -> str:
        with sqlite3.connect(self.data_root / "kassiber.sqlite3") as conn:
            rows = [
                (table, sorted(map(repr, conn.execute(f'SELECT * FROM "{table}"'))))
                for (table,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            ]
        return hashlib.sha256(repr(rows).encode()).hexdigest()

    def test_the_kinds_are_not_python_kinds(self):
        for kind in KINDS:
            self.assertNotIn(kind, SUPPORTED_KINDS)

    def test_the_daemon_refuses_each_kind_without_writing(self):
        before = self.digest()
        requests = [{"request_id": kind, "kind": kind, "args": ARGS[kind]} for kind in KINDS]
        requests.append({"request_id": "stop", "kind": "daemon.shutdown"})
        result = self.run_cli("daemon", wire="".join(json.dumps(r) + "\n" for r in requests))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        records = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(records[0]["kind"], "daemon.ready")
        for kind in KINDS:
            self.assertNotIn(kind, records[0]["data"]["supported_kinds"])
        replies = {record.get("request_id"): record for record in records[1:]}
        for kind in KINDS:
            reply = replies[kind]
            self.assertEqual(reply["kind"], "error", reply)
            # The desktop maps either code to "needs the Rust backend".
            self.assertIn(reply["error"]["code"], {"daemon_unavailable", "unsupported_kind"}, reply)
            self.assertEqual(reply["error"]["details"], {"kind": kind}, reply)
        self.assertEqual(self.digest(), before)


if __name__ == "__main__":
    unittest.main()
