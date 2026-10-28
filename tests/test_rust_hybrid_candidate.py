"""The hybrid candidate serves each CLI call from Rust when Rust implements it.

`tests/rust_port/hybrid_cli.py` lets a Python test drive a whole CLI flow
while the Rust port is incomplete: commands Rust implements run in Rust, the
rest fall back to Python on the same book. These tests pin the routing, the
shared book, and that stdin and inherited descriptors reach either side.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests import cli_runner

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
HYBRID = ROOT / "tests" / "rust_port" / "hybrid_cli.py"
PASSPHRASE = "hybrid candidate passphrase"


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to a `--features test-harness` build of crates/kassiber")
class HybridCandidateTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-hybrid-")
        self.addCleanup(directory.cleanup)
        base = Path(directory.name)
        self.data_root = base / "data"
        self.log = base / "served.jsonl"
        home = base / "home"
        home.mkdir()
        environment = {
            cli_runner.CLI_ENV: json.dumps([sys.executable, str(HYBRID)]),
            "KASSIBER_HYBRID_RUST_BIN": str(Path(RUST_BIN).resolve()),
            "KASSIBER_HYBRID_LOG": str(self.log),
            "HOME": str(home),
            "XDG_DATA_HOME": str(home / "xdg-data"),
            "XDG_CONFIG_HOME": str(home / "xdg-config"),
            "XDG_STATE_HOME": str(home / "xdg-state"),
            "KASSIBER_DISABLE_UPDATE_CHECK": "1",
            "KASSIBER_OFFLINE_PREFERENCE_FILE": str(home / "offline.json"),
            "KASSIBER_AGENT_ACCESS_FILE": str(home / "agents.json"),
            "KASSIBER_UPDATE_PREFERENCE_FILE": str(home / "updates.json"),
            "KASSIBER_UPDATE_CACHE_FILE": str(home / "update-cache.json"),
        }
        patcher = mock.patch.dict(os.environ, environment)
        patcher.start()
        self.addCleanup(patcher.stop)

    def cli(self, *args, passphrase_flag=None):
        argv = ["--machine", "--data-root", str(self.data_root)]
        kwargs = {}
        if passphrase_flag == "--db-passphrase-fd":
            argv += [passphrase_flag, cli_runner.InputFd(PASSPHRASE.encode())]
        argv += list(args)
        if passphrase_flag == "--new-passphrase-fd":
            argv += [passphrase_flag, cli_runner.InputFd(PASSPHRASE.encode())]
        return cli_runner.run(
            argv, capture_output=True, text=True, timeout=120,
            stdin=subprocess.DEVNULL, secret_input=passphrase_flag is not None, **kwargs,
        )

    def ok(self, result):
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        return json.loads(result.stdout)

    def served(self):
        return [json.loads(line) for line in self.log.read_text(encoding="utf-8").splitlines()]

    def test_each_call_runs_where_it_is_implemented_on_one_book(self):
        self.assertEqual(self.ok(self.cli("init"))["kind"], "init")
        self.assertEqual(self.ok(self.cli("workspaces", "create", "Main"))["kind"], "workspaces.create")
        self.assertEqual(self.ok(self.cli("status"))["kind"], "status")
        self.ok(self.cli("wallets", "kinds"))
        listed = self.ok(self.cli("workspaces", "list"))["data"]
        self.assertIn("Main", json.dumps(listed))
        setup_commands = [
            ["profiles", "create", "Book"], ["profiles", "list"], ["profiles", "get"],
            ["profiles", "set", "--gains-algorithm", "HIFO"],
            ["accounts", "create", "--code", "savings", "--label", "Savings", "--type", "asset"],
            ["accounts", "list"], ["context", "show"], ["context", "current"],
            ["context", "set", "--workspace", "Main", "--profile", "Book"],
            ["views", "create", "--surface", "swap", "--name", "Stored", "--filter", '{"memo": "é"}'],
            ["views", "list"],
        ]
        for command in setup_commands:
            self.ok(self.cli(*command))
        views = self.ok(self.cli("views", "list"))["data"]
        self.assertEqual(views[0]["filter"], {"memo": "é"})
        self.ok(self.cli("views", "delete", "--view-id", views[0]["id"]))
        expected = ["init", "workspaces.create", "status", "wallets.kinds", "workspaces.list"]
        expected += [".".join(command[:2]) for command in setup_commands]
        expected += ["views.list", "views.delete"]
        self.assertEqual(self.served(), [
            {"command": command, "served_by": "python" if command == "wallets.kinds" else "rust"}
            for command in expected
        ])

    def test_usage_errors_are_rusts(self):
        result = self.cli("bogus")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stdout)["error"]["code"], "usage_error")
        self.assertEqual(self.served(), [{"command": "<usage>", "served_by": "rust"}])

    def test_inherited_descriptors_reach_either_side(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("secrets", "init", passphrase_flag="--new-passphrase-fd"))
        status = self.ok(self.cli("status", passphrase_flag="--db-passphrase-fd"))["data"]
        self.assertTrue(status["database_encrypted"])
        self.assertEqual(
            [entry["served_by"] for entry in self.served()],
            ["rust", "python", "rust"],
        )

    def test_daemon_stays_in_python_unless_asked(self):
        process = cli_runner.popen(
            ["--machine", "--data-root", str(self.data_root), "daemon"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        stdout, _ = process.communicate(b"", timeout=120)
        self.assertEqual(json.loads(stdout.splitlines()[0])["kind"], "daemon.ready")
        self.assertEqual(self.served(), [{"command": "daemon", "served_by": "python"}])


if __name__ == "__main__":
    unittest.main()
