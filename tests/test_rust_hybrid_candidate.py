"""The hybrid candidate serves each CLI call from Rust when Rust implements it.

`tests/rust_port/hybrid_cli.py` lets a Python test drive a whole CLI flow
while the Rust port is incomplete: commands Rust implements run in Rust, the
rest fall back to Python on the same book. These tests pin the routing, the
shared book, and that stdin and inherited descriptors reach either side.
"""

from __future__ import annotations

import json
import os
import re
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
IMPLEMENTED = set(re.findall(
    r'"([^"\n]+)"',
    (ROOT / "crates/cli/src/describe.rs").read_text().split(
        "pub const IMPLEMENTED: &[&str] = &[", 1
    )[1].split("];", 1)[0],
))


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
        owners = base / "owners"
        owners.mkdir(mode=0o700)
        environment = {
            cli_runner.CLI_ENV: json.dumps([sys.executable, str(HYBRID)]),
            "KASSIBER_HYBRID_RUST_BIN": str(Path(RUST_BIN).resolve()),
            "KASSIBER_HYBRID_LOG": str(self.log),
            "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
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
            ["projects", "create", "Native project"], ["projects", "list"],
            ["projects", "show", "native-project"], ["projects", "select", "native-project"],
            ["commands", "describe", "wallets"],
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
            {"command": command, "served_by": "rust" if command in IMPLEMENTED else "python"}
            for command in expected
        ])

    def test_wallet_lifecycle_has_no_python_fallbacks(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Lifecycle workspace"))
        self.ok(self.cli("profiles", "create", "Lifecycle book", "--workspace", "Lifecycle workspace"))
        self.ok(self.cli("context", "set", "--workspace", "Lifecycle workspace", "--profile", "Lifecycle book"))
        self.ok(self.cli("wallets", "create", "--label", "Local watch", "--kind", "address", "--address", "bc1qpublicplaceholder"))
        self.ok(self.cli("wallets", "list"))
        self.ok(self.cli("wallets", "get", "--wallet", "Local watch"))
        self.ok(self.cli("wallets", "update", "--wallet", "Local watch", "--label", "Archive"))
        self.ok(self.cli("wallets", "delete", "--wallet", "Archive"))
        lifecycle = [item for item in self.served() if item["command"].startswith("wallets.")]
        self.assertEqual(lifecycle, [
            {"command": "wallets." + command, "served_by": "rust"}
            for command in ("create", "list", "get", "update", "delete")
        ])

    def test_silent_payment_lifecycle_and_birthday_have_no_python_fallbacks(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "SP workspace"))
        self.ok(self.cli("profiles", "create", "SP book", "--workspace", "SP workspace"))
        self.ok(self.cli("context", "set", "--workspace", "SP workspace", "--profile", "SP book"))
        self.ok(self.cli("wallets", "create", "--label", "SP local", "--kind", "silent-payment", "--sp-descriptor", "sp(spscan1q" + "q"*40 + ")", "--sp-scan-start-height", "0", "--backend", "local", "--birthday", "2025-01-02Z"))
        self.ok(self.cli("wallets", "update", "--wallet", "SP local", "--birthday", "2025-01-02T12:00:00 +01:00"))
        self.ok(self.cli("wallets", "get", "--wallet", "SP local"))
        self.ok(self.cli("wallets", "list"))
        self.ok(self.cli("wallets", "delete", "--wallet", "SP local"))
        calls = [item for item in self.served() if item["command"].startswith("wallets.")]
        self.assertEqual(calls, [{"command":"wallets."+command,"served_by":"rust"} for command in ("create","update","get","list","delete")])

    def test_command_line_settings_survive_a_scrubbed_environment(self):
        candidate = [sys.executable, str(HYBRID), f"--hybrid-rust-bin={Path(RUST_BIN).resolve()}", f"--hybrid-log={self.log}"]
        environment = {key: value for key, value in os.environ.items() if not key.startswith(("KASSIBER_HYBRID_", cli_runner.CLI_ENV))}
        result = subprocess.run(
            [*candidate, "--machine", "--data-root", str(self.data_root), "init"],
            capture_output=True, text=True, timeout=120, stdin=subprocess.DEVNULL, env=environment, cwd=self.log.parent,
        )
        self.assertEqual(self.ok(result)["kind"], "init")
        self.assertEqual(self.served(), [{"command": "init", "served_by": "rust"}])

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
        # Encryption now runs in Rust. Keep a separate, unported database read
        # to prove the same inherited descriptor also reaches Python.
        from kassiber.cli.main import build_parser
        from kassiber.cli.command_registry import describe_command_catalog

        witness = next(command for command in describe_command_catalog(build_parser())["commands"]
                       if command["kind"] not in IMPLEMENTED
                       and command["effect"] == "read_only" and command["needs_database"]
                       and not any(arg["required"] for arg in command["arguments"]))
        self.ok(self.cli(*witness["path"], passphrase_flag="--db-passphrase-fd"))
        self.assertEqual(
            [entry["served_by"] for entry in self.served()],
            ["rust", "rust", "rust", "python"],
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
