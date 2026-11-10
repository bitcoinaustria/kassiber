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


def unported_read_witnesses(rust_bin: str, scratch: Path) -> list[list[str]]:
    """Read-only book commands without required arguments that this build
    still leaves to Python. `IMPLEMENTED` is the product build's list; a
    `test-harness` build also registers its feature-gated leaves (chat,
    chats, the MCP server), so the build's own probe decides."""
    from kassiber.cli.command_registry import describe_command_catalog
    from kassiber.cli.main import build_parser
    from tests.rust_port.hybrid_cli import PROBE_NOT_IMPLEMENTED

    env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
    env["KASSIBER_RUST_TEST_PROBE"] = "1"
    witnesses = []
    for entry in describe_command_catalog(build_parser())["commands"]:
        if (entry["kind"] in IMPLEMENTED or entry["effect"] != "read_only" or not entry["needs_database"]
                or any(arg["required"] for arg in entry["arguments"])):
            continue
        probe = subprocess.run([rust_bin, "--data-root", str(scratch / "probe-book"), *entry["path"]],
                               env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=120)
        if probe.returncode == PROBE_NOT_IMPLEMENTED:
            witnesses.append(entry["path"])
    return witnesses


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

    def test_json_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "JSON workspace"))
        self.ok(self.cli("profiles", "create", "JSON book", "--workspace", "JSON workspace"))
        self.ok(self.cli("context", "set", "--workspace", "JSON workspace", "--profile", "JSON book"))
        self.ok(self.cli("wallets", "create", "--label", "JSON source", "--kind", "custom"))
        source = self.data_root.parent / "input.json"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            for case in cases:
                if case["format"] != "json": continue
                with self.subTest(case=case["name"]):
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    result = self.cli("wallets", "import-json", "--wallet", "JSON source", "--file", str(source))
                    self.assertIn(result.returncode, (0,1), (result.stdout, result.stderr))
                    # The oracle fails the wrapper on any complete book-dump
                    # difference, including failed imports and unchanged runs.
                    self.assertNotIn("Traceback", result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-json"]
        self.assertEqual(calls, [{"command":"wallets.import-json","served_by":"rust"}] * sum(case["format"]=="json" for case in cases))

    def test_river_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "River workspace"))
        self.ok(self.cli("profiles", "create", "River book", "--workspace", "River workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "River workspace", "--profile", "River book"))
        self.ok(self.cli("wallets", "create", "--label", "River source", "--kind", "river"))
        source = self.data_root.parent / "river.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_exchange_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            for case in cases:
                with self.subTest(case=case["name"]):
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    result = self.cli("wallets", "import-river", "--wallet", "River source", "--file", str(source))
                    self.assertIn(result.returncode, (0,1), (result.stdout, result.stderr))
                    self.assertNotIn("Traceback", result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-river"]
        self.assertEqual(calls, [{"command":"wallets.import-river","served_by":"rust"}] * len(cases))

    def test_bull_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Bull workspace"))
        self.ok(self.cli("profiles", "create", "Bull book", "--workspace", "Bull workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Bull workspace", "--profile", "Bull book"))
        self.ok(self.cli("wallets", "create", "--label", "Bull source", "--kind", "bullbitcoin"))
        source = self.data_root.parent / "bull.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_bull_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            for mode in ("relevant", "full"):
                for case in cases:
                    with self.subTest(mode=mode, case=case["name"]):
                        source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                        result = self.cli("wallets", "import-bull", "--wallet", "Bull source", "--mode", mode, "--file", str(source))
                        self.assertIn(result.returncode, (0,1), (result.stdout, result.stderr))
                        self.assertNotIn("Traceback", result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-bull"]
        self.assertEqual(calls, [{"command":"wallets.import-bull","served_by":"rust"}] * (2 * len(cases)))

    def test_coinfinity_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Coinfinity workspace"))
        self.ok(self.cli("profiles", "create", "Coinfinity book", "--workspace", "Coinfinity workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Coinfinity workspace", "--profile", "Coinfinity book"))
        self.ok(self.cli("wallets", "create", "--label", "Coinfinity source", "--kind", "coinfinity"))
        source = self.data_root.parent / "coinfinity.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_coinfinity_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            for mode in ("relevant", "full"):
                for case in cases:
                    with self.subTest(mode=mode, case=case["name"]):
                        source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                        result = self.cli("wallets", "import-coinfinity", "--wallet", "Coinfinity source", "--mode", mode, "--file", str(source))
                        self.assertIn(result.returncode, (0,1), (result.stdout, result.stderr))
                        self.assertNotIn("Traceback", result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-coinfinity"]
        self.assertEqual(calls, [{"command":"wallets.import-coinfinity","served_by":"rust"}] * (2 * len(cases)))

    def test_twentyone_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "21bitcoin workspace"))
        self.ok(self.cli("profiles", "create", "21bitcoin book", "--workspace", "21bitcoin workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "21bitcoin workspace", "--profile", "21bitcoin book"))
        self.ok(self.cli("wallets", "create", "--label", "21bitcoin source", "--kind", "21bitcoin"))
        source = self.data_root.parent / "twentyone.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_twentyone_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            for case in cases:
                with self.subTest(case=case["name"]):
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    result = self.cli("wallets", "import-21bitcoin", "--wallet", "21bitcoin source", "--file", str(source))
                    self.assertIn(result.returncode, (0,1), (result.stdout, result.stderr))
                    self.assertNotIn("Traceback", result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-21bitcoin"]
        self.assertEqual(calls, [{"command":"wallets.import-21bitcoin","served_by":"rust"}] * len(cases))

    def test_bull_coordinator_default_wallet_alias_reimport_and_rollback_dump(self):
        from tests.test_cli_smoke import _BULLBITCOIN_EXISTING_CSV, _BULLBITCOIN_ORDERS_CSV
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Bull workspace"))
        self.ok(self.cli("profiles", "create", "Bull book", "--workspace", "Bull workspace", "--fiat-currency", "USD"))
        self.ok(self.cli("context", "set", "--workspace", "Bull workspace", "--profile", "Bull book"))
        source = self.data_root.parent / "bull.csv"
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            self.ok(self.cli("wallets", "create", "--label", "Observed", "--kind", "custom"))
            source.write_text(_BULLBITCOIN_EXISTING_CSV)
            self.ok(self.cli("wallets", "import-csv", "--wallet", "Observed", "--file", str(source)))
            source.write_text(_BULLBITCOIN_ORDERS_CSV)
            self.ok(self.cli("wallets", "import-bullbitcoin", "--file", str(source)))
            self.ok(self.cli("wallets", "import-bull", "--mode", "full", "--file", str(source)))
            repeated = self.ok(self.cli("wallets", "import-bullbitcoin", "--mode", "full", "--file", str(source)))["data"]
            self.assertEqual(repeated["imported"], 0)
            source.write_text(_BULLBITCOIN_ORDERS_CSV.replace("USD","EUR").replace("2026-04-16 09:50:00.000Z","invalid"))
            refused=self.cli("wallets", "import-bull", "--file", str(source))
            self.assertEqual(refused.returncode,1,refused.stderr)
            self.assertEqual(json.loads(refused.stdout)["error"]["message"],"Imported price currency EUR does not match profile fiat currency USD")
            source.write_text("bad,header\nx,y\n")
            refused = self.cli("wallets", "import-bull", "--mode", "full", "--wallet", "missing", "--file", str(source))
            self.assertEqual(refused.returncode, 1)
            source.write_text(_BULLBITCOIN_ORDERS_CSV)
            batch = self.ok(self.cli("imports", "list", "--wallet", "Bull Bitcoin"))["data"]["batches"][0]["id"]
            self.ok(self.cli("imports", "rollback", "--batch", batch, "--confirm"))
            self.ok(self.cli("wallets", "import-bullbitcoin", "--mode", "full", "--file", str(source)))
        calls=[item for item in self.served() if item["command"] in {"wallets.import-bull","wallets.import-bullbitcoin"}]
        self.assertEqual(len(calls),6)
        self.assertTrue(all(item["served_by"]=="rust" for item in calls))

    def test_coinfinity_coordinator_default_economics_reimport_and_rollback_dump(self):
        import csv
        import io
        from tests.test_cli_smoke import _COINFINITY_ORDERS_CSV
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Coinfinity workspace"))
        self.ok(self.cli("profiles", "create", "Coinfinity book", "--workspace", "Coinfinity workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Coinfinity workspace", "--profile", "Coinfinity book"))
        self.ok(self.cli("wallets", "create", "--label", "Observed", "--kind", "custom"))
        source = self.data_root.parent / "coinfinity.csv"
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            source.write_text("date,txid,amount,fee\n2025-01-02,observed,0.002,0.00001\n2025-01-02,ambiguous1,0.003,0\n2025-01-03,ambiguous2,0.003,0\n2025-01-05,outside,0.004,0\n")
            self.ok(self.cli("wallets", "import-csv", "--wallet", "Observed", "--file", str(source)))
            output=io.StringIO(); writer=csv.writer(output)
            writer.writerow(["Order ID","Type","Date","Amount EUR","Amount Crypto","Crypto","Rate EUR","Mining Fee Crypto","Total Fee EUR","Transaction"])
            for i,amount in enumerate(("0.002","0.003","0.004")):
                writer.writerow([str(i),"sell","2025-01-02","100",amount,"BTC","","0","1",""])
            source.write_text(output.getvalue())
            relevant=self.ok(self.cli("wallets", "import-coinfinity", "--file", str(source)))["data"]
            self.assertEqual((relevant["matched"],relevant["skipped_ambiguous"],relevant["skipped_unmatched"]),(1,1,1))
            self.ok(self.cli("wallets", "import-coinfinity", "--mode", "full", "--file", str(source)))
            repeated=self.ok(self.cli("wallets", "import-coinfinity", "--mode", "full", "--file", str(source)))["data"]
            self.assertEqual(repeated["imported"],0)
            batch=self.ok(self.cli("imports", "list", "--wallet", "Coinfinity"))["data"]["batches"][0]["id"]
            self.ok(self.cli("imports", "rollback", "--batch", batch, "--confirm"))
            self.ok(self.cli("wallets", "import-coinfinity", "--mode", "full", "--file", str(source)))
            source.write_text(_COINFINITY_ORDERS_CSV)
            self.ok(self.cli("wallets", "import-coinfinity", "--file", str(source)))
            for mode in ("bad"," "):
                self.assertEqual(self.cli("wallets", "import-coinfinity", "--mode", mode, "--file", str(source)).returncode,2)
            self.assertEqual(self.cli("wallets", "import-coinfinity", "--wallet", "missing", "--file", str(source)).returncode,1)
        calls=[item for item in self.served() if item["command"]=="wallets.import-coinfinity"]
        self.assertEqual(len(calls),6)
        self.assertTrue(all(item["served_by"]=="rust" for item in calls))

    def test_twentyone_default_ledger_reimport_legacy_flags_and_rollback_dump(self):
        import sqlite3
        from tests.test_cli_smoke import _TWENTYONEBITCOIN_TRANSACTIONS_CSV
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "21bitcoin workspace"))
        self.ok(self.cli("profiles", "create", "21bitcoin book", "--workspace", "21bitcoin workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "21bitcoin workspace", "--profile", "21bitcoin book"))
        source = self.data_root.parent / "twentyone.csv"
        source.write_text(_TWENTYONEBITCOIN_TRANSACTIONS_CSV)
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            result = self.ok(self.cli("wallets", "import-21bitcoin", "--file", str(source)))["data"]
            self.assertEqual((result["wallet"], result["mode"], result["imported"]), ("21bitcoin", "full", 2))
            repeated = self.ok(self.cli("wallets", "import-21bitcoin", "--file", str(source)))["data"]
            self.assertEqual((repeated["unchanged"],repeated["updated"]), (1,1))
            database = next(self.data_root.rglob("kassiber.sqlite3"))
            with sqlite3.connect(database) as conn:
                conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) SELECT 'legacy21',workspace_id,profile_id,'21bitcoin-matched','legacy','2025-01-01' FROM wallets LIMIT 1")
                conn.execute("INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'legacy21' FROM transactions WHERE external_id='21bitcoin:2'")
                conn.execute("UPDATE transactions SET excluded=1 WHERE external_id='21bitcoin:2'")
            reactivated = self.ok(self.cli("wallets", "import-21bitcoin", "--file", str(source)))["data"]
            self.assertEqual((reactivated["reconciliation_flags_cleared"],reactivated["reactivated"]),(1,1))
            source.write_text(_TWENTYONEBITCOIN_TRANSACTIONS_CSV.replace("Promotion Payout BTC Purchase","Updated purchase") + "new,21bitcoin,main,08.10.2022 16:31:20,BTC,0.001,,,BTC,0,deposit,receive,\n")
            changed = self.ok(self.cli("wallets", "import-21bitcoin", "--file", str(source)))["data"]
            self.assertEqual((changed["imported"],changed["updated"]),(1,1))
            self.ok(self.cli("imports", "rollback", "--batch", changed["import_batch_id"], "--confirm"))
            self.ok(self.cli("wallets", "import-21bitcoin", "--file", str(source)))
            self.assertEqual(self.cli("wallets", "import-21bitcoin", "--wallet", "missing", "--file", str(source)).returncode,1)
            self.assertEqual(self.cli("wallets", "import-21bitcoin", "--file", str(source.parent / "missing.csv")).returncode,1)
            self.assertEqual(self.cli("wallets", "import-21bitcoin", "--mode", "relevant", "--file", str(source)).returncode,2)
        calls=[item for item in self.served() if item["command"]=="wallets.import-21bitcoin"]
        self.assertEqual(calls,[{"command":"wallets.import-21bitcoin","served_by":"rust"}]*7)

    def test_pocket_aliases_have_no_fallback_and_match_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Pocket workspace"))
        self.ok(self.cli("profiles", "create", "Pocket book", "--workspace", "Pocket workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Pocket workspace", "--profile", "Pocket book"))
        self.ok(self.cli("wallets", "create", "--label", "Pocket source", "--kind", "pocketbitcoin"))
        source = self.data_root.parent / "pocket.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_pocket_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            for phase, mode in enumerate(("relevant", "full")):
                for index, case in enumerate(cases):
                    command=("import-pocket", "import-pocketbitcoin")[(phase+index)%2]
                    with self.subTest(mode=mode, case=case["name"], command=command):
                        source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                        result = self.cli("wallets", command, "--wallet", "Pocket source", "--mode", mode, "--file", str(source))
                        self.assertIn(result.returncode, (0,1), (result.stdout,result.stderr))
                        self.assertNotIn("Traceback",result.stderr)
        calls=[item for item in self.served() if item["command"] in {"wallets.import-pocket", "wallets.import-pocketbitcoin"}]
        self.assertEqual(len(calls),2*len(cases))
        self.assertTrue(all(item["served_by"]=="rust" for item in calls))
        for command in ("wallets.import-pocket", "wallets.import-pocketbitcoin"):
            self.assertEqual(sum(item["command"]==command for item in calls),len(cases))

    def test_pocket_default_alias_economics_reimport_and_rollback_dump(self):
        import csv
        import io
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Pocket workspace"))
        self.ok(self.cli("profiles", "create", "Pocket book", "--workspace", "Pocket workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Pocket workspace", "--profile", "Pocket book"))
        self.ok(self.cli("wallets", "create", "--label", "Observed", "--kind", "custom"))
        source = self.data_root.parent / "pocket.csv"
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            source.write_text("date,txid,amount,fee\n2025-01-02,observed,0.00199,0.00002\n2025-01-02,ambiguous1,0.003,0\n2025-01-03,ambiguous2,0.003,0\n2025-01-06,outside,0.004,0\n")
            self.ok(self.cli("wallets", "import-csv", "--wallet", "Observed", "--file", str(source)))
            output=io.StringIO(); writer=csv.writer(output)
            writer.writerow(["type","date","reference","price.currency","price.amount","cost.currency","cost.amount","fee.currency","fee.amount","value.currency","value.amount"])
            for i,amount in enumerate(("0.002","0.003","0.004")):
                writer.writerow(["exchange","2025-01-02",str(i),"EUR","","EUR","100","EUR","1.2","BTC",amount])
            writer.writerow(["withdrawal","2025-01-03","withdraw","EUR","","EUR","0","BTC","0.00001","BTC","0.00199"])
            source.write_text(output.getvalue())
            relevant=self.ok(self.cli("wallets", "import-pocket", "--file", str(source)))["data"]
            self.assertEqual((relevant["matched"],relevant["skipped_ambiguous"],relevant["skipped_unmatched"]),(1,1,1))
            full=self.ok(self.cli("wallets", "import-pocketbitcoin", "--mode", "full", "--file", str(source)))["data"]
            self.assertEqual((full["wallet"],full["imported"],full["pocketbitcoin_rows"]),("Pocket Bitcoin",3,3))
            repeated=self.ok(self.cli("wallets", "import-pocket", "--mode", "full", "--file", str(source)))["data"]
            self.assertEqual(repeated["imported"],0)
            source.write_text(output.getvalue().replace("BTC,0.002", "BTC,0.00201").replace("BTC,0.00001,BTC", "BTC,0.00002,BTC"))
            corrected=self.ok(self.cli("wallets", "import-pocketbitcoin", "--mode", "full", "--file", str(source)))["data"]
            self.assertEqual(corrected["imported"],0)
            self.ok(self.cli("imports", "rollback", "--batch", full["import_batch_id"], "--confirm"))
            self.ok(self.cli("wallets", "import-pocketbitcoin", "--mode", "full", "--file", str(source)))
            self.ok(self.cli("wallets", "import-pocketbitcoin", "--file", str(source)))
            self.assertEqual(self.cli("wallets", "import-pocket", "--wallet", "missing", "--file", str(source)).returncode,1)
            self.assertEqual(self.cli("wallets", "import-pocketbitcoin", "--file", str(source.parent/"missing.csv")).returncode,1)
        calls=[item for item in self.served() if item["command"] in {"wallets.import-pocket","wallets.import-pocketbitcoin"}]
        self.assertEqual(len(calls),8)
        self.assertTrue(all(item["served_by"]=="rust" for item in calls))

    def test_strike_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Strike workspace"))
        self.ok(self.cli("profiles", "create", "Strike book", "--workspace", "Strike workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Strike workspace", "--profile", "Strike book"))
        self.ok(self.cli("wallets", "create", "--label", "Strike source", "--kind", "strike"))
        source = self.data_root.parent / "strike.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_strike_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            for case in cases:
                with self.subTest(case=case["name"]):
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    result = self.cli("wallets", "import-strike", "--wallet", "Strike source", "--file", str(source))
                    self.assertIn(result.returncode, (0,1), (result.stdout,result.stderr))
                    self.assertNotIn("Traceback",result.stderr)
        calls=[item for item in self.served() if item["command"]=="wallets.import-strike"]
        self.assertEqual(calls,[{"command":"wallets.import-strike","served_by":"rust"}]*len(cases))

    def test_strike_default_ledger_reimport_updates_and_rollback_dump(self):
        from tests.test_cli_smoke import _STRIKE_CSV
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Strike workspace"))
        self.ok(self.cli("profiles", "create", "Strike book", "--workspace", "Strike workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Strike workspace", "--profile", "Strike book"))
        source = self.data_root.parent / "strike.csv"
        source.write_text(_STRIKE_CSV)
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            result=self.ok(self.cli("wallets", "import-strike", "--file", str(source)))["data"]
            self.assertEqual((result["wallet"],result["mode"],result["imported"]),("Strike","full",5))
            repeated=self.ok(self.cli("wallets", "import-strike", "--file", str(source)))["data"]
            self.assertEqual(repeated["imported"],0)
            source.write_text(_STRIKE_CSV.replace("BTC purchase","Updated purchase")+"new,Mar 10 2026 01:02:03,Buy,-10,0,0.0001,0,100000,10,balance,new,,note\n")
            changed=self.ok(self.cli("wallets", "import-strike", "--file", str(source)))["data"]
            self.assertEqual(changed["imported"],1)
            self.assertGreaterEqual(changed["updated"],1)
            self.ok(self.cli("imports", "rollback", "--batch", changed["import_batch_id"], "--confirm"))
            self.ok(self.cli("wallets", "import-strike", "--file", str(source)))
            self.assertEqual(self.cli("wallets", "import-strike", "--wallet", "missing", "--file", str(source)).returncode,1)
            self.assertEqual(self.cli("wallets", "import-strike", "--file", str(source.parent/"missing.csv")).returncode,1)
            self.assertEqual(self.cli("wallets", "import-strike", "--mode", "relevant", "--file", str(source)).returncode,2)
        calls=[item for item in self.served() if item["command"]=="wallets.import-strike"]
        self.assertEqual(calls,[{"command":"wallets.import-strike","served_by":"rust"}]*6)

    def test_ledgerlive_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Ledger workspace"))
        self.ok(self.cli("profiles", "create", "Ledger book", "--workspace", "Ledger workspace"))
        self.ok(self.cli("context", "set", "--workspace", "Ledger workspace", "--profile", "Ledger book"))
        self.ok(self.cli("wallets", "create", "--label", "Ledger source", "--kind", "ledgerlive"))
        source = self.data_root.parent / "ledgerlive.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_ledgerlive_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            for case in cases:
                with self.subTest(case=case["name"]):
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    result = self.cli("wallets", "import-ledger-live", "--wallet", "Ledger source", "--file", str(source))
                    self.assertIn(result.returncode, (0, 1), (result.stdout, result.stderr))
                    self.assertNotIn("Traceback", result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-ledger-live"]
        self.assertEqual(calls, [{"command": "wallets.import-ledger-live", "served_by": "rust"}] * len(cases))

    def test_ledgerlive_reimport_rollback_and_refusals_match_complete_dump(self):
        from tests.test_cli_smoke import _LEDGERLIVE_CSV
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Ledger workspace"))
        self.ok(self.cli("profiles", "create", "Ledger book", "--workspace", "Ledger workspace"))
        self.ok(self.cli("context", "set", "--workspace", "Ledger workspace", "--profile", "Ledger book"))
        self.ok(self.cli("wallets", "create", "--label", "Ledger source", "--kind", "ledgerlive"))
        source = self.data_root.parent / "ledgerlive.csv"
        source.write_text(_LEDGERLIVE_CSV)
        args = ("wallets", "import-ledger-live", "--wallet", "Ledger source", "--file", str(source))
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            result = self.ok(self.cli(*args))["data"]
            self.assertEqual(result["imported"], 2)
            repeated = self.ok(self.cli(*args))["data"]
            self.assertEqual((repeated["imported"], repeated["unchanged"]), (0, 2))
            database = next(self.data_root.rglob("kassiber.sqlite3"))
            import sqlite3
            with sqlite3.connect(database) as conn:
                conn.execute("UPDATE transactions SET description=NULL")
            enriched = self.ok(self.cli(*args))["data"]
            self.assertEqual((enriched["imported"], len(enriched["updated_records"])), (0, 2))
            self.assertTrue(enriched["journal_invalidated"])
            self.ok(self.cli("imports", "rollback", "--batch", result["import_batch_id"], "--confirm"))
            self.assertEqual(self.ok(self.cli(*args))["data"]["imported"], 2)
            self.assertEqual(self.cli("wallets", "import-ledger-live", "--wallet", "missing", "--file", str(source)).returncode, 1)
            self.assertEqual(self.cli("wallets", "import-ledger-live", "--wallet", "Ledger source", "--file", str(source.parent / "missing.csv")).returncode, 1)
            source.write_text(_LEDGERLIVE_CSV + "invalid,BTC,IN,1,0,later,,,,\n")
            self.assertEqual(self.cli(*args).returncode, 1)
        calls = [item for item in self.served() if item["command"] == "wallets.import-ledger-live"]
        self.assertEqual(calls, [{"command": "wallets.import-ledger-live", "served_by": "rust"}] * 7)

    def test_twentyone_unpriced_legacy_reactivation_matches_readiness_dump(self):
        import sqlite3
        from contextlib import closing
        from tests.test_twentyonebitcoin_import import HEADER

        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Legacy workspace"))
        self.ok(self.cli("profiles", "create", "Legacy book", "--workspace", "Legacy workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Legacy workspace", "--profile", "Legacy book"))
        source = self.data_root.parent / "legacy.csv"
        source.write_text(HEADER + "1,21bitcoin,main,10.01.2026 12:00:00,,,BTC,0.002,BTC,0.0001,withdrawal,Withdrawal,,\n")
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            self.ok(self.cli("wallets", "import-21bitcoin", "--file", str(source)))
            stable = self.ok(self.cli("wallets", "import-21bitcoin", "--file", str(source)))["data"]
            self.assertFalse(stable["journal_invalidated"])
            databases = list(self.data_root.rglob("kassiber.sqlite3"))
            self.assertEqual(len(databases), 1)
            with closing(sqlite3.connect(databases[0])) as conn:
                conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) SELECT 'legacy',workspace_id,profile_id,'21bitcoin-matched','Old','2025-01-01' FROM transactions LIMIT 1")
                conn.execute("INSERT INTO transaction_tags(transaction_id,tag_id) SELECT id,'legacy' FROM transactions")
                conn.execute("UPDATE transactions SET excluded=1")
                conn.execute("UPDATE profiles SET last_processed_at='2026-01-11T00:00:00Z',last_processed_tx_count=1,last_processed_input_version=journal_input_version,ownership_review_counts_json='{}'")
                before = conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0]
                conn.commit()
            result = self.ok(self.cli("wallets", "import-21bitcoin", "--file", str(source)))["data"]
            self.assertTrue(result["journal_invalidated"])
            self.assertEqual(result["reactivated"], 1)
            with closing(sqlite3.connect(databases[0])) as conn:
                self.assertEqual(conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0], before + 1)
        calls = [item for item in self.served() if item["command"] == "wallets.import-21bitcoin"]
        self.assertEqual(calls, [{"command": "wallets.import-21bitcoin", "served_by": "rust"}] * 3)


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
        # to prove the same inherited descriptor also reaches Python. A list
        # read answers on any book once it has a context.
        for setup in (("workspaces", "create", "Main"), ("profiles", "create", "Book", "--workspace", "Main"),
                      ("context", "set", "--workspace", "Main", "--profile", "Book")):
            self.ok(self.cli(*setup, passphrase_flag="--db-passphrase-fd"))
        # Once every database read is native there is no Python side left
        # to reach; the six Rust calls above still carry the descriptor.
        witness = next((path for path in unported_read_witnesses(RUST_BIN, self.data_root.parent)
                        if path[-1] == "list"), None)
        if witness is not None:
            self.ok(self.cli(*witness, passphrase_flag="--db-passphrase-fd"))
        self.assertEqual(
            [entry["served_by"] for entry in self.served()],
            ["rust"] * 6 + (["python"] if witness is not None else []),
        )

    def test_daemon_stays_in_python_unless_asked(self):
        process = cli_runner.popen(
            ["--machine", "--data-root", str(self.data_root), "daemon"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        stdout, _ = process.communicate(b"", timeout=120)
        self.assertEqual(json.loads(stdout.splitlines()[0])["kind"], "daemon.ready")
        self.assertEqual(self.served(), [{"command": "daemon", "served_by": "python"}])

    def test_cointracking_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Migration workspace"))
        self.ok(self.cli("profiles", "create", "Migration book", "--workspace", "Migration workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Migration workspace", "--profile", "Migration book"))
        self.ok(self.cli("wallets", "create", "--label", "CoinTracking source", "--kind", "cointracking"))
        source = self.data_root.parent / "cointracking.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_cointracking_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            for case in cases:
                with self.subTest(case=case["name"]):
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    result = self.cli("wallets", "import-cointracking", "--wallet", "CoinTracking source", "--file", str(source))
                    self.assertIn(result.returncode, (0, 1), (result.stdout, result.stderr))
                    self.assertNotIn("Traceback", result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-cointracking"]
        self.assertEqual(calls, [{"command": "wallets.import-cointracking", "served_by": "rust"}] * len(cases))

    def test_cointracking_reconciliation_reimport_identity_promotion_and_rollback_match_dump(self):
        import sqlite3
        from contextlib import closing
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Migration workspace"))
        self.ok(self.cli("profiles", "create", "Migration book", "--workspace", "Migration workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Migration workspace", "--profile", "Migration book"))
        for label, kind in (("CoinTracking source", "cointracking"), ("Observed", "custom")):
            self.ok(self.cli("wallets", "create", "--label", label, "--kind", kind))
        database = next(self.data_root.rglob("kassiber.sqlite3"))
        txid = "ab" * 32
        with closing(sqlite3.connect(database)) as conn:
            conn.execute("UPDATE wallets SET kind='descriptor' WHERE label='Observed'")
            for identity, external, stamp, amount in (("economic", "observed", "2025-01-02T00:00:00Z", 200000000), ("identity", txid, "2025-01-01T00:00:00Z", 1)):
                conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) SELECT ?,workspace_id,profile_id,id,?,?,?,'inbound','BTC',?,0,'2025-01-01' FROM wallets WHERE label='Observed'", (identity, external, identity, stamp, amount))
            conn.commit()
        source = self.data_root.parent / "cointracking.csv"
        header = "Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Date,Tx-ID,Comment\n"
        body = (header + "Deposit,0.002,BTC,,,,,2025-01-02,econ,one\n"
                + f"Deposit,0.003,BTC,,,,,2025-01-03,{txid},two\n"
                + "Deposit,0.004,BTC,,,,,2025-01-04,,three\nDeposit,0.004,BTC,,,,,2025-01-04,,four\n"
                + "Deposit,0.005,BTC,,,,,2025-01-05,active,five\n")
        source.write_text(body)
        args = ("wallets", "import-cointracking", "--wallet", "CoinTracking source", "--file", str(source))
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            first = self.ok(self.cli(*args))["data"]
            self.assertEqual((first["imported"], first["matched"], first["ambiguous"], first["excluded"]), (5, 1, 3, 4))
            repeated = self.ok(self.cli(*args))["data"]
            self.assertEqual((repeated["unchanged"], repeated["reconciliation_rows"]), (5, 0))
            self.assertFalse(repeated["journal_invalidated"])
            with closing(sqlite3.connect(database)) as conn:
                conn.execute("DELETE FROM transactions WHERE wallet_id=(SELECT id FROM wallets WHERE label='Observed')")
                conn.execute("UPDATE transactions SET description=NULL WHERE external_id IS NOT NULL")
                conn.commit()
            enriched = self.ok(self.cli(*args))["data"]
            self.assertEqual((enriched["reconciliation_flags_cleared"], enriched["reactivated"]), (2, 2))
            source.write_text(body.replace("2025-01-04,,three", f"2025-01-04,{txid},three"))
            promoted = self.ok(self.cli(*args))["data"]
            self.assertEqual((promoted["imported"], len(promoted["updated_records"])), (0, 1))
            self.ok(self.cli("imports", "rollback", "--batch", first["import_batch_id"], "--confirm"))
            self.assertEqual(self.ok(self.cli(*args))["data"]["imported"], 5)
            self.assertEqual(self.cli("wallets", "import-cointracking", "--wallet", "missing", "--file", str(source)).returncode, 1)
            self.assertEqual(self.cli("wallets", "import-cointracking", "--wallet", "Observed", "--file", str(source.parent / "missing.csv")).returncode, 1)
            self.assertEqual(self.cli("wallets", "import-cointracking", "--wallet", "CoinTracking source", "--file", str(source.parent / "missing.csv")).returncode, 1)
        calls = [item for item in self.served() if item["command"] == "wallets.import-cointracking"]
        self.assertEqual(calls, [{"command": "wallets.import-cointracking", "served_by": "rust"}] * 8)

    def test_blockpit_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Migration workspace"))
        self.ok(self.cli("profiles", "create", "Migration book", "--workspace", "Migration workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Migration workspace", "--profile", "Migration book"))
        self.ok(self.cli("wallets", "create", "--label", "Blockpit source", "--kind", "blockpit"))
        source = self.data_root.parent / "blockpit.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_blockpit_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            for case in cases:
                with self.subTest(case=case["name"]):
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    result = self.cli("wallets", "import-blockpit", "--wallet", "Blockpit source", "--file", str(source))
                    self.assertIn(result.returncode, (0, 1), (result.stdout, result.stderr))
                    self.assertNotIn("Traceback", result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-blockpit"]
        self.assertEqual(calls, [{"command": "wallets.import-blockpit", "served_by": "rust"}] * len(cases))

    def test_blockpit_reconciliation_reimport_identity_promotion_and_rollback_match_dump(self):
        import sqlite3
        from contextlib import closing
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Migration workspace"))
        self.ok(self.cli("profiles", "create", "Migration book", "--workspace", "Migration workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Migration workspace", "--profile", "Migration book"))
        for label, kind in (("Blockpit source", "blockpit"), ("Observed", "custom")):
            self.ok(self.cli("wallets", "create", "--label", label, "--kind", kind))
        database = next(self.data_root.rglob("kassiber.sqlite3"))
        txid = "ab" * 32
        with closing(sqlite3.connect(database)) as conn:
            conn.execute("UPDATE wallets SET kind='descriptor' WHERE label='Observed'")
            for identity, external, stamp, amount in (("economic", "observed", "2025-01-02T00:00:00Z", 200000000), ("identity", txid, "2025-01-01T00:00:00Z", 1)):
                conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) SELECT ?,workspace_id,profile_id,id,?,?,?,'inbound','BTC',?,0,'2025-01-01' FROM wallets WHERE label='Observed'", (identity, external, identity, stamp, amount))
            conn.commit()
        source = self.data_root.parent / "blockpit.csv"
        header = "Date (UTC),Integration Name,Label,Outgoing Asset,Outgoing Amount,Incoming Asset,Incoming Amount,Fee Asset,Fee Amount,Comments,Trx. ID\n"
        body = (header + "2025-01-02,Source,Deposit,,,BTC,0.002,,,one,econ\n"
                + f"2025-01-03,Source,Deposit,,,BTC,0.003,,,two,{txid}\n"
                + "2025-01-04,Source,Deposit,,,BTC,0.004,,,three,\n2025-01-04,Source,Deposit,,,BTC,0.004,,,four,\n"
                + "2025-01-05,Source,Deposit,,,BTC,0.005,,,five,active\n")
        source.write_text(body)
        args = ("wallets", "import-blockpit", "--wallet", "Blockpit source", "--file", str(source))
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            first = self.ok(self.cli(*args))["data"]
            self.assertEqual((first["imported"], first["matched"], first["ambiguous"], first["excluded"]), (5, 1, 3, 4))
            repeated = self.ok(self.cli(*args))["data"]
            self.assertEqual((repeated["unchanged"], repeated["reconciliation_rows"]), (5, 0))
            self.assertFalse(repeated["journal_invalidated"])
            with closing(sqlite3.connect(database)) as conn:
                conn.execute("DELETE FROM transactions WHERE wallet_id=(SELECT id FROM wallets WHERE label='Observed')")
                conn.execute("UPDATE transactions SET description=NULL WHERE external_id IS NOT NULL")
                conn.commit()
            enriched = self.ok(self.cli(*args))["data"]
            self.assertEqual((enriched["reconciliation_flags_cleared"], enriched["reactivated"]), (2, 2))
            source.write_text(body.replace("2025-01-04,Source,Deposit,,,BTC,0.004,,,three,", f"2025-01-04,Source,Deposit,,,BTC,0.004,,,three,{txid}"))
            promoted = self.ok(self.cli(*args))["data"]
            self.assertEqual((promoted["imported"], len(promoted["updated_records"])), (0, 1))
            self.ok(self.cli("imports", "rollback", "--batch", first["import_batch_id"], "--confirm"))
            self.assertEqual(self.ok(self.cli(*args))["data"]["imported"], 5)
            self.assertEqual(self.cli("wallets", "import-blockpit", "--wallet", "missing", "--file", str(source)).returncode, 1)
            self.assertEqual(self.cli("wallets", "import-blockpit", "--wallet", "Observed", "--file", str(source.parent / "missing.csv")).returncode, 1)
            self.assertEqual(self.cli("wallets", "import-blockpit", "--wallet", "Blockpit source", "--file", str(source.parent / "missing.csv")).returncode, 1)
        calls = [item for item in self.served() if item["command"] == "wallets.import-blockpit"]
        self.assertEqual(calls, [{"command": "wallets.import-blockpit", "served_by": "rust"}] * 8)

    def test_btcpay_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Workspace"))
        self.ok(self.cli("profiles", "create", "Book", "--workspace", "Workspace"))
        self.ok(self.cli("context", "set", "--workspace", "Workspace", "--profile", "Book"))
        self.ok(self.cli("wallets", "create", "--label", "BTCPay source", "--kind", "custom"))
        source = self.data_root.parent / "btcpay.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_btcpay_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            for case in cases:
                with self.subTest(case=case["name"]):
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    result = self.cli("wallets", "import-btcpay", "--wallet", "BTCPay source", "--file", str(source), "--format", case["format"].removeprefix("btcpay_"))
                    self.assertIn(result.returncode, (0, 1), (result.stdout, result.stderr))
                    self.assertNotIn("Traceback", result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-btcpay"]
        self.assertEqual(calls, [{"command":"wallets.import-btcpay", "served_by":"rust"}] * len(cases))

    def test_btcpay_coordinator_reimports_metadata_and_rollback_match_dump(self):
        from crates.wallets.scripts.generate_btcpay_vectors import BASE
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Workspace"))
        self.ok(self.cli("profiles", "create", "Book", "--workspace", "Workspace"))
        self.ok(self.cli("context", "set", "--workspace", "Workspace", "--profile", "Book"))
        self.ok(self.cli("wallets", "create", "--label", "BTCPay source", "--kind", "custom"))
        source = self.data_root.parent / "btcpay.json"
        args = ("wallets", "import-btcpay", "--wallet", "BTCPay source", "--file", str(source), "--input-format", "json")
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            source.write_text(json.dumps([BASE]))
            first = self.ok(self.cli(*args))["data"]
            self.assertEqual(first["btcpay_notes_set"], 1)
            self.assertFalse(self.ok(self.cli(*args))["data"]["journal_invalidated"])
            source.write_text(json.dumps([{**BASE, "Comment":"new", "Labels":"New label"}]))
            self.assertEqual(self.ok(self.cli(*args))["data"]["btcpay_tags_created"], 1)
            source.write_text(json.dumps([BASE, {**BASE, "Timestamp":"invalid"}]))
            self.assertEqual(self.cli(*args).returncode, 1)
            self.ok(self.cli("imports", "rollback", "--batch", first["import_batch_id"], "--confirm"))
            source.write_text(json.dumps([BASE]))
            self.assertEqual(self.ok(self.cli(*args))["data"]["imported"], 1)
            self.assertEqual(self.cli("wallets", "import-btcpay", "--wallet", "missing", "--file", str(source)).returncode, 1)
            self.assertEqual(self.cli("wallets", "import-btcpay", "--wallet", "BTCPay source", "--file", str(source.parent / "missing.csv")).returncode, 1)
        calls = [item for item in self.served() if item["command"] == "wallets.import-btcpay"]
        self.assertEqual(calls, [{"command":"wallets.import-btcpay", "served_by":"rust"}] * 7)

    def test_ledger_import_and_preview_have_no_fallback_and_match_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Workspace"))
        self.ok(self.cli("profiles", "create", "Book", "--workspace", "Workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Workspace", "--profile", "Book"))
        self.ok(self.cli("wallets", "create", "--label", "Ledger source", "--kind", "custom"))
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_ledger_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE":"1"}):
            for case in cases:
                with self.subTest(case=case["name"]):
                    source = self.data_root.parent / ("ledger" + case["extension"])
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    extra = ["--column-map",json.dumps(case["column_map"])] if case["column_map"] is not None else []
                    for flags in [[],["--dry-run","--limit",str(case["limit"])]]:
                        result = self.cli("wallets","import-ledger","--wallet","Ledger source","--file",str(source),*extra,*flags)
                        self.assertIn(result.returncode,(0,1),(result.stdout,result.stderr))
                        self.assertNotIn("Traceback",result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-ledger"]
        self.assertEqual(calls,[{"command":"wallets.import-ledger","served_by":"rust"}]*len(cases)*2)

    def test_ledger_mapped_coordinator_reimports_scope_failures_and_rollback_match_dump(self):
        import sqlite3
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Workspace"))
        self.ok(self.cli("profiles", "create", "Book", "--workspace", "Workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Workspace", "--profile", "Book"))
        self.ok(self.cli("wallets", "create", "--label", "Ledger source", "--kind", "custom"))
        source = self.data_root.parent / "mapped.csv"
        mapping = self.data_root.parent / "mapping.json"
        mapping.write_text(json.dumps({
            "date": "When", "amount": "Units", "type": "Side", "fiat_value": "Cash",
            "fiat_currency": "Currency", "txid": "Reference", "type_map": {"ACQUIRE": "Buy"},
        }))
        body = "When,Units,Side,Cash,Currency,Reference\n2025-01-02,0.002,ACQUIRE,190,EUR,mapped\n"
        source.write_text(body)
        args = ("wallets", "import-ledger", "--wallet", "Ledger source", "--file", str(source))
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            first = self.ok(self.cli(*args, "--column-map", "@" + str(mapping)))["data"]
            self.assertEqual(first["imported"], 1)
            repeated = self.ok(self.cli(*args, "--column-map", "@" + str(mapping)))["data"]
            self.assertEqual(repeated["imported"], 0)
            self.assertTrue(repeated["journal_invalidated"])
            source.write_text(body.replace("190", "191"))
            self.ok(self.cli(*args, "--column-map", mapping.read_text()))
            source.write_text(body + "invalid,0.003,ACQUIRE,200,EUR,later\n")
            self.assertEqual(self.cli(*args, "--column-map", "@" + str(mapping)).returncode, 1)
            self.assertEqual(self.cli(*args, "--column-map", mapping.read_text(), "--wallet", "missing").returncode, 1)
            self.assertEqual(self.cli(*args, "--column-map", mapping.read_text(), "--profile", "missing").returncode, 1)
            source.write_text(body)
            self.ok(self.cli(*args, "--column-map", "@" + str(mapping)))
        self.ok(self.cli("imports", "rollback", "--batch", first["import_batch_id"], "--confirm"))
        with sqlite3.connect(self.data_root / "kassiber.sqlite3") as conn:
            self.assertEqual(conn.execute("SELECT count(*) FROM transactions").fetchone()[0], 0)
            self.assertEqual(conn.execute("SELECT count(*) FROM import_batches").fetchone()[0], 0)
        calls = [item for item in self.served() if item["command"] == "wallets.import-ledger"]
        self.assertEqual(calls, [{"command": "wallets.import-ledger", "served_by": "rust"}] * 7)

    def test_binance_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Workspace"))
        self.ok(self.cli("profiles", "create", "Book", "--workspace", "Workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Workspace", "--profile", "Book"))
        self.ok(self.cli("wallets", "create", "--label", "Binance source", "--kind", "binance"))
        source = self.data_root.parent / "binance.csv"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_binance_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            for case in cases:
                with self.subTest(case=case["name"]):
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    result = self.cli("wallets", "import-binance-supplemental", "--wallet", "Binance source", "--file", str(source))
                    self.assertIn(result.returncode, (0, 1), (result.stdout, result.stderr))
                    self.assertNotIn("Traceback", result.stderr)
        calls = [item for item in self.served() if item["command"] == "wallets.import-binance-supplemental"]
        self.assertEqual(calls, [{"command": "wallets.import-binance-supplemental", "served_by": "rust"}] * len(cases))

    def test_binance_coordinator_reimports_enrichment_refusals_and_rollback_match_dump(self):
        from crates.wallets.scripts.generate_binance_vectors import inputs
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Workspace"))
        self.ok(self.cli("profiles", "create", "Book", "--workspace", "Workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Workspace", "--profile", "Book"))
        self.ok(self.cli("wallets", "create", "--label", "Binance source", "--kind", "binance"))
        source = self.data_root.parent / "binance.csv"
        args = ("wallets", "import-binance-supplemental", "--wallet", "Binance source", "--file", str(source))
        body = inputs()['autoinvest']
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            source.write_text(body)
            first = self.ok(self.cli(*args))["data"]
            self.assertEqual(first["imported"], 1)
            self.assertEqual(self.ok(self.cli(*args))["data"]["imported"], 0)
            self.assertEqual(self.ok(self.cli(*args, "--mode", "relevant"))["data"]["matched"], 1)
            # An ID-less autoinvest row's identity is its economic tuple
            # (9b4ce57c9), so a different quote amount is a different row.
            source.write_text(body.replace('100.00 EUR', '200.00 EUR'))
            self.assertEqual(self.ok(self.cli(*args))["data"]["imported"], 1)
            source.write_text(inputs()['partial-failure'])
            self.assertEqual(self.cli(*args).returncode, 1)
            self.ok(self.cli("imports", "rollback", "--batch", first["import_batch_id"], "--confirm"))
            source.write_text(body)
            self.assertEqual(self.ok(self.cli(*args))["data"]["imported"], 1)
            self.assertEqual(self.cli("wallets", "import-binance-supplemental", "--wallet", "missing", "--file", str(source)).returncode, 1)
            self.assertEqual(self.cli("wallets", "import-binance-supplemental", "--wallet", "Binance source", "--file", str(source.parent / "missing.csv")).returncode, 1)
        calls = [item for item in self.served() if item["command"] == "wallets.import-binance-supplemental"]
        self.assertEqual(calls, [{"command": "wallets.import-binance-supplemental", "served_by": "rust"}] * 8)

    def _checked_wasabi(self, *args):
        result = self.cli("wallets", "import-wasabi", *args)
        self.assertTrue(result.stdout, result.stderr)
        self.assertIsInstance(json.loads(result.stdout), dict)
        return result


    def test_wasabi_import_has_no_fallback_and_matches_every_reference_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Workspace"))
        self.ok(self.cli("profiles", "create", "Book", "--workspace", "Workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Workspace", "--profile", "Book"))
        self.ok(self.cli("wallets", "create", "--label", "Wasabi source", "--kind", "wasabi"))
        source = self.data_root.parent / "wasabi.json"
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_wasabi_imports.json").read_text())
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            for case in cases:
                with self.subTest(case=case["name"]):
                    source.write_bytes(bytes(case["bytes"]) if "bytes" in case else case["text"].encode())
                    result = self._checked_wasabi("--wallet", "Wasabi source", "--file", str(source))
                    self.assertIn(result.returncode, (0, 1), (result.stdout, result.stderr))
        calls = [item for item in self.served() if item["command"] == "wallets.import-wasabi"]
        self.assertEqual(calls, [{"command": "wallets.import-wasabi", "served_by": "rust"}] * len(cases))


    def test_wasabi_coercion_edges_have_no_fallback_and_match_book_dump(self):
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Workspace"))
        self.ok(self.cli("profiles", "create", "Book", "--workspace", "Workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Workspace", "--profile", "Book"))
        self.ok(self.cli("wallets", "create", "--label", "Wasabi source", "--kind", "wasabi"))
        source = self.data_root.parent / "wasabi.json"
        names = {"coin-distinct-outpoints", "account-digit-refusal", "account-nondigit", "last-alias-false", "surrogate-section", "metadata-large-float"}
        from crates.wallets.scripts.generate_wasabi_vectors import inputs
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            for name in sorted(names):
                with self.subTest(case=name):
                    source.write_text(inputs()[name])
                    self.assertIn(self._checked_wasabi("--wallet", "Wasabi source", "--file", str(source)).returncode, (0, 1))
            edges = json.loads((ROOT / 'crates/wallets/tests/vectors/python_wasabi_codepoints.json').read_text())
            for case in edges:
                with self.subTest(case=case['name']):
                    source.write_text(case['text'])
                    self.assertIn(self._checked_wasabi("--wallet", "Wasabi source", "--file", str(source)).returncode, (0, 1))
        calls = [item for item in self.served() if item["command"] == "wallets.import-wasabi"]
        self.assertEqual(calls, [{"command": "wallets.import-wasabi", "served_by": "rust"}] * (len(names) + len(edges)))


    def test_wasabi_coordinator_promotion_enrichment_reimports_refusals_and_rollback_match_dump(self):
        from contextlib import closing
        import sqlite3
        from crates.wallets.scripts.generate_wasabi_vectors import HISTORY, COIN
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Workspace"))
        self.ok(self.cli("profiles", "create", "Book", "--workspace", "Workspace", "--fiat-currency", "EUR"))
        self.ok(self.cli("context", "set", "--workspace", "Workspace", "--profile", "Book"))
        for label in ("Wasabi source", "Overlap"):
            self.ok(self.cli("wallets", "create", "--label", label, "--kind", "custom"))
        source = self.data_root.parent / "wasabi.json"
        args = ("--wallet", "Wasabi source", "--file", str(source))
        database = next(self.data_root.rglob("kassiber.sqlite3"))
        def run(value):
            source.write_text(json.dumps(value))
            return self.ok(self._checked_wasabi(*args))["data"]
        with mock.patch.dict(os.environ, {"KASSIBER_HYBRID_IMPORT_ORACLE": "1"}):
            first = run([{**HISTORY, "tx": None}])
            self.assertIn("import_batch_id", first)  # 06ddfc545
            promoted = run([HISTORY])
            self.assertEqual((promoted["imported"], len(promoted["updated_records"])), (1, 0))
            with closing(sqlite3.connect(database)) as conn:
                conn.execute("UPDATE transactions SET external_id_kind=NULL WHERE external_id=?", (HISTORY["tx"],))
                conn.commit()
            identity = run([HISTORY])
            self.assertEqual(len(identity["updated_records"]), 1)
            with closing(sqlite3.connect(database)) as conn:
                conn.execute("UPDATE transactions SET description=NULL,note='authored',review_status='completed'")
                conn.commit()
            run({"gethistory": [{**HISTORY, "islikelycoinjoin": True}], "listcoins": [COIN]})
            run({"gethistory": [HISTORY], "listcoins": []})
            source.write_text(json.dumps([HISTORY]))
            self.ok(self._checked_wasabi("--wallet", "Overlap", "--file", str(source)))
            self.assertEqual(self._checked_wasabi("--wallet", "missing", "--file", str(source)).returncode, 1)
            self.assertEqual(self._checked_wasabi("--wallet", "Wasabi source", "--file", str(source.parent / "missing.json")).returncode, 1)
            source.write_text(json.dumps({"gethistory": [HISTORY], "listcoins": [{**COIN, "txid": "bad"}]}))
            self.assertEqual(self._checked_wasabi(*args).returncode, 1)
        calls = [item for item in self.served() if item["command"] == "wallets.import-wasabi"]
        self.assertEqual(calls, [{"command": "wallets.import-wasabi", "served_by": "rust"}] * 9)



if __name__ == "__main__":
    unittest.main()
