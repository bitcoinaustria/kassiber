"""The Rust shell answers what the Python CLI and daemon answer.

Plan 21 R1 builds the Rust binary's shell (docs/reference/rust-shell.md):
argv, global flags, the machine envelope, usage errors, `commands
describe`, and the daemon's JSONL protocol. When `KASSIBER_RUST_BIN` names
a Rust binary, these tests run the same argv or daemon input through
`python -m kassiber` and the binary, each on its own disposable root, and
compare exit codes and raw stdout/stderr bytes. JSONL records keep their
terminators. No nondeterministic fields are normalized in these scenarios.
Only the documented R1 catalog/kind subsets are projected from Python.
Without the binary the process cases skip; CI and the local gate supply it.

Build the binary with the test harness, because the suite arms
`KASSIBER_TEST_NO_EGRESS` (a build without the attempt observer exits 78)
and the daemon cases need the in-memory store until the storage crates land:

    cargo build -p kassiber --features test-harness
    KASSIBER_RUST_BIN=target/debug/kassiber \\
        uv run --locked python -m pytest tests/test_rust_shell_parity.py -q

The drift checks at the end need no binary: they fail when the Python
grammar, capability tables, or Unicode tables move without the Rust copies.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from contextlib import contextmanager
from pathlib import Path
from unittest import mock

from tests import cli_runner

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
RUST_SKIP = "set KASSIBER_RUST_BIN to a `--features test-harness` build of crates/kassiber"
TIMEOUT = 120


def _environment(home: Path, columns: str = "80") -> dict[str, str]:
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("KASSIBER_OPERATOR_", "KASSIBER_RUST_", "SATBOOKS_"))
        and key not in {"COLUMNS", "LINES", "KASSIBER_DATA_ROOT", "KASSIBER_STATE_ROOT"}
    }
    env.update(
        {
            "HOME": str(home),
            "USERPROFILE": str(home),
            "LOCALAPPDATA": str(home),
            "XDG_DATA_HOME": str(home / "xdg-data"),
            "XDG_CONFIG_HOME": str(home / "xdg-config"),
            "XDG_STATE_HOME": str(home / "xdg-state"),
            "COLUMNS": columns,
            "LINES": "24",
            "PYTHONIOENCODING": "utf-8",
            "KASSIBER_DISABLE_UPDATE_CHECK": "1",
            "KASSIBER_OFFLINE_PREFERENCE_FILE": str(home / "offline.json"),
            "KASSIBER_AGENT_ACCESS_FILE": str(home / "agents.json"),
            "KASSIBER_UPDATE_PREFERENCE_FILE": str(home / "updates.json"),
            "KASSIBER_UPDATE_CACHE_FILE": str(home / "update-cache.json"),
        }
    )
    return env


def _rust(args: list[str]) -> list[str]:
    return [str(Path(RUST_BIN).resolve()), *args]


class _Roots(unittest.TestCase):
    """Separate disposable homes for the two implementations."""

    def setUp(self):
        self._dir = tempfile.TemporaryDirectory(prefix="kassiber-rust-parity-")
        base = Path(self._dir.name)
        self.python_home = base / "python"
        self.rust_home = base / "rust"
        self.python_home.mkdir()
        self.rust_home.mkdir()

    def tearDown(self):
        self._dir.cleanup()

    @contextmanager
    def mutated_rust(self, mutation):
        wrapper = Path(self._dir.name) / "mutate_wire.py"
        wrapper.write_text(
            "import json, subprocess, sys\n"
            "binary, mutation, *args = sys.argv[1:]\n"
            "result = subprocess.run([binary, *args], input=sys.stdin.buffer.read(), capture_output=True)\n"
            "def mutate(raw):\n"
            "    if not raw or mutation == 'identity': return raw\n"
            "    records = raw.splitlines() if 'daemon' in args else [raw]\n"
            "    output = []\n"
            "    for record in records:\n"
            "        value = json.loads(record)\n"
            "        if mutation == 'key-order': value = dict(reversed(list(value.items())))\n"
            "        options = {'separators': (',', ':')} if 'daemon' in args else {'indent': 2}\n"
            "        if mutation == 'indentation': options = {'indent': 4}\n"
            "        if mutation == 'separators': options['separators'] = (', ', ': ')\n"
            "        output.append(json.dumps(value, **options).encode() + b'\\n')\n"
            "    return b''.join(output)\n"
            "sys.stdout.buffer.write(mutate(result.stdout))\n"
            "sys.stderr.buffer.write(mutate(result.stderr))\n"
            "sys.exit(result.returncode)\n",
            encoding="utf-8",
        )
        with mock.patch(__name__ + "._rust", side_effect=lambda args: [
            sys.executable, str(wrapper), str(Path(RUST_BIN).resolve()), mutation, *args,
        ]):
            yield

    def both(self, args: list[str], columns: str = "80", *, disposable_cwd: bool = False):
        python = cli_runner.run(
            args, reference=True, cwd=self.python_home if disposable_cwd else ROOT,
            env=_environment(self.python_home, columns),
            capture_output=True, timeout=TIMEOUT, stdin=subprocess.DEVNULL,
        )
        rust = subprocess.run(
            _rust(args), cwd=self.rust_home if disposable_cwd else ROOT,
            env=_environment(self.rust_home, columns),
            capture_output=True, timeout=TIMEOUT, stdin=subprocess.DEVNULL,
        )
        self.assertNotEqual(
            rust.returncode, 78,
            f"{RUST_BIN} refused to start under KASSIBER_TEST_NO_EGRESS; build it "
            "with --features test-harness",
        )
        return python, rust

    def assert_same(self, args: list[str], *, columns: str = "80", json_stdout: bool):
        python, rust = self.both(args, columns)
        self.assertEqual(rust.returncode, python.returncode, (python.stderr, rust.stderr))
        if json_stdout:
            json.loads(python.stdout)
            json.loads(rust.stdout)
        self.assertEqual(rust.stdout, python.stdout, f"stdout bytes: {args!r}")
        self.assertEqual(rust.stderr, python.stderr, f"stderr bytes: {args!r}")
        return python, rust


@unittest.skipUnless(RUST_BIN, RUST_SKIP)
class RustCliParityTest(_Roots):
    def test_saved_json_matches_stdout_bytes(self):
        args = ["--machine", "commands", "describe", "status"]
        reference, _ = self.assert_same(args, json_stdout=True)
        for target in ["~/exports/current.json", "./~/exports/dotted.json", "exports/deep/./report.json"]:
            with self.subTest(target=target):
                python, rust = self.both(
                    ["--machine", "--output", target, *args[1:]], disposable_cwd=True,
                )
                self.assertEqual((python.returncode, rust.returncode), (0, 0))
                self.assertEqual((python.stdout, rust.stdout, python.stderr, rust.stderr), (b"",) * 4)
                for home in (self.python_home, self.rust_home):
                    relative = target.removeprefix("./").removeprefix("~/")
                    self.assertEqual((home / relative).read_bytes(), reference.stdout)
                    self.assertFalse((home / "~").exists())

    def test_cli_oracle_rejects_formatting_discrepancies_from_a_child(self):
        scenarios = [
            (["--machine", "commands", "describe", "status"], True),
            (["--machine", "bogus"], True),
            (["--machine", "--output", ".", "commands", "describe", "status"], False),
        ]
        with self.mutated_rust("identity"):
            for args, json_stdout in scenarios:
                self.assert_same(args, json_stdout=json_stdout)
        for mutation in ["indentation", "key-order", "separators"]:
            with self.subTest(mutation=mutation), self.mutated_rust(mutation):
                for args, json_stdout in scenarios:
                    python, rust = self.both(args)
                    self.assertEqual(rust.returncode, python.returncode)
                    stream = "stdout" if json_stdout else "stderr"
                    self.assertEqual(json.loads(getattr(rust, stream)), json.loads(getattr(python, stream)))
                    with self.assertRaises(AssertionError):
                        self.assert_same(args, json_stdout=json_stdout)

    def test_output_failures_preserve_python_error_and_filename(self):
        directories = [".", "./.", "quoted'folder", 'double"folder', "café\u0345"]
        if os.name != "nt":
            directories.append("both'\"\\\nfolder")
        for home in (self.python_home, self.rust_home):
            for directory in directories:
                (home / directory).mkdir(exist_ok=True)
            (home / "block").write_text("file", encoding="utf-8")
        targets = [*directories, "block/report.json", "block/child/report.json"]
        if os.name != "nt":
            for home in (self.python_home, self.rust_home):
                (home / "dangling").symlink_to(home / "missing", target_is_directory=True)
            targets.append("dangling/child/report.json")
        for target in targets:
            with self.subTest(target=target):
                python, rust = self.both(
                    ["--machine", "--output", target, "commands", "describe", "status"],
                    disposable_cwd=True,
                )
                self.assertEqual((python.returncode, rust.returncode), (1, 1))
                self.assertEqual(rust.stdout, python.stdout)
                self.assertEqual(rust.stderr, python.stderr)

    def test_unresolved_output_user_fails_before_creating_directories(self):
        directory = "~kassiber-review-no-such-user"
        for prefix in ["", "./", "././"]:
            target = f"{prefix}{directory}/report.json"
            with self.subTest(target=target):
                python, rust = self.both(
                    ["--machine", "--output", target, "commands", "describe", "status"],
                    disposable_cwd=True,
                )
                self.assertEqual(python.returncode, 1)
                self.assertEqual(rust.returncode, python.returncode)
                self.assertEqual(rust.stdout, python.stdout)
                self.assertEqual(rust.stderr, python.stderr)
                self.assertEqual(json.loads(python.stderr)["error"]["message"],
                                 "Could not determine home directory.")
                for home in (self.python_home, self.rust_home):
                    self.assertFalse((home / directory).exists())

    def test_version(self):
        # The oracle reference must ignore the general candidate switch.
        with mock.patch.dict(os.environ, {"KASSIBER_TEST_CLI": "/does-not-exist/reference-must-ignore"}):
            self.assert_same(["--version"], json_stdout=False)

    def test_machine_usage_errors(self):
        for args in (
            ["--machine", "bogus"],
            ["status", "--machine"],
            ["--machine", "wallets"],
            ["--machine", "--format", "xml", "status"],
            ["--machine", "status", "--bogus"],
            ["--machine", "--d", "status"],
            ["--machine", "--db-passphrase-fd", "x", "status"],
            ["--machine", "wallets", "sync", "--bogus"],
            ["--machine", "reports", "lightning-profitability", "--window-days", "0"],
            ["--machine", "attachments", "add"],
            ["--machine", "update", "--status", "--enable-checks"],
        ):
            python, _ = self.assert_same(args, json_stdout=True)
            self.assertEqual(python.returncode, 2)
            self.assertEqual(json.loads(python.stdout)["error"]["code"], "usage_error")
        self.assert_same(["--machine", "bogus"], columns="40", json_stdout=True)

    def test_plain_usage_errors_and_help(self):
        # `--machine=1` is not the `--machine` token: plain usage on stderr.
        for args in (
            ["bogus"], ["--mach", "bogus"], ["--machine=1", "status"], ["wallets"],
            ["status", "--x"], ["-h"], ["status", "--help"],
        ):
            self.assert_same(args, json_stdout=False)
        self.assert_same(["wallets", "-h"], columns="200", json_stdout=False)

    def test_flag_combination_errors(self):
        for args in (
            ["--machine", "--format", "csv", "status"],
            ["--machine", "--format", "table", "commands", "describe"],
        ):
            python, _ = self.assert_same(args, json_stdout=False)
            self.assertEqual(python.returncode, 1)
            self.assertTrue(python.stderr.startswith(b"error: --machine requires --format json"))
            self.assertEqual(python.stdout, b"")

    def test_unknown_commands(self):
        self.assert_same(["--machine", "nope"], json_stdout=True)
        python, _ = self.assert_same(["--machine", "commands", "describe", "nope"], json_stdout=True)
        self.assertEqual(json.loads(python.stdout)["error"]["code"], "unknown_command")

    def test_commands_describe_for_implemented_leaves(self):
        for path in (["status"], ["daemon"], ["commands"], ["commands", "describe"]):
            self.assert_same(["--machine", "commands", "describe", *path], json_stdout=True)
        python, rust = self.both(["--machine", "commands", "describe"])
        self.assertEqual((python.returncode, rust.returncode), (0, 0))
        python_catalog = json.loads(python.stdout)["data"]
        rust_catalog = json.loads(rust.stdout)["data"]
        by_kind = {entry["kind"]: entry for entry in python_catalog["commands"]}
        self.assertGreater(rust_catalog["count"], 0)
        self.assertEqual(rust_catalog["count"], len(rust_catalog["commands"]))
        self.assertEqual(rust_catalog["global"], python_catalog["global"])
        for entry in rust_catalog["commands"]:
            with self.subTest(kind=entry["kind"]):
                self.assertEqual(entry, by_kind[entry["kind"]])
        # R1 intentionally advertises a smaller catalog. Project that field
        # from Python; compare the resulting wire document, including order.
        projected = json.loads(python.stdout)
        projected["data"]["commands"] = [entry for entry in python_catalog["commands"]
                                             if entry["kind"] in {"status", "daemon", "commands.describe"}]
        projected["data"]["count"] = len(projected["data"]["commands"])
        self.assertEqual(rust.stdout, (json.dumps(projected, indent=2) + "\n").encode())
        unported = self.both(["--machine", "commands", "describe", "wallets"])[1]
        self.assertEqual(unported.returncode, 1)
        self.assertEqual(json.loads(unported.stdout)["error"]["code"], "not_implemented")


def _daemon(args: list[str], env: dict[str, str], lines: list[str], *, reference=False) -> tuple[int, list[bytes]]:
    options = dict(cwd=ROOT, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    process = (cli_runner.popen(args, reference=True, **options) if reference
               else subprocess.Popen(_rust(args), **options))
    stdout, stderr = process.communicate("".join(lines).encode(), timeout=TIMEOUT)
    if stderr:
        raise AssertionError(f"daemon stderr: {stderr!r}")
    return process.returncode, stdout.splitlines(keepends=True)


@unittest.skipUnless(RUST_BIN, RUST_SKIP)
class RustDaemonParityTest(_Roots):
    def run_both(self, lines: list[str]):
        python_root = self.python_home / "data"
        rust_root = self.rust_home / "data"
        python = _daemon(
            ["--data-root", str(python_root), "daemon"],
            _environment(self.python_home), lines, reference=True,
        )
        rust_env = _environment(self.rust_home)
        rust_env["KASSIBER_RUST_TEST_FAKE_STORE"] = json.dumps({"data_root": str(rust_root)})
        rust = _daemon(["--data-root", str(rust_root), "daemon"], rust_env, lines)
        return python, rust

    def assert_daemon_same(self, python, rust):
        self.assertEqual(len(rust), len(python))
        self.assertTrue(python)
        python_ready, rust_ready = json.loads(python[0]), json.loads(rust[0])
        for ready in (python_ready, rust_ready):
            self.assertEqual(ready["kind"], "daemon.ready")
            self.assertNotIn("request_id", ready)
            self.assertEqual(ready["schema_version"], 1)
        kinds = ["status", "daemon.lock", "daemon.unlock", "daemon.shutdown"]
        self.assertTrue(set(kinds) <= set(python_ready["data"]["supported_kinds"]))
        # This is the documented R1 capability projection, not a field mask.
        python_ready["data"]["supported_kinds"] = [
            *kinds, "test.stream", "test.emit_event", "test.late", "test.secret_store",
        ]
        self.assertEqual(rust[0], (json.dumps(python_ready, separators=(",", ":")) + "\n").encode())
        for index, (expected, actual) in enumerate(zip(python[1:], rust[1:]), 1):
            self.assertEqual(actual, expected, f"JSONL record {index} bytes")

    def test_daemon_oracle_rejects_formatting_discrepancies_from_a_child(self):
        lines = ['{"request_id":"e","kind":"token=topsecret"}\n', "not json\n",
                 '{"request_id":"q","kind":"daemon.shutdown"}\n']
        with self.mutated_rust("identity"):
            (python_code, python), (rust_code, rust) = self.run_both(lines)
            self.assertEqual((python_code, rust_code), (0, 0))
            self.assert_daemon_same(python, rust)
        for mutation in ["key-order", "separators"]:
            with self.subTest(mutation=mutation), self.mutated_rust(mutation):
                (python_code, python), (rust_code, rust) = self.run_both(lines)
                self.assertEqual((python_code, rust_code), (0, 0))
                self.assertEqual([json.loads(record) for record in rust[1:]],
                                 [json.loads(record) for record in python[1:]])
                with self.assertRaises(AssertionError):
                    self.assert_daemon_same(python, rust)

    def test_unicode_secret_redaction_matches_python_generated_inputs(self):
        from crates.envelope.scripts.gen_golden import unicode_secret_inputs

        kinds = ["mnemonıc=abandon ability able", "blİndİng=topsecret",
                 "\u0345token=topsecret", "\U0001e4d0" + "xpub" + "A" * 40,
                 *unicode_secret_inputs()]
        lines = [json.dumps({"request_id": i, "kind": kind}) + "\n"
                 for i, kind in enumerate(kinds)]
        lines.append('{"request_id":"q","kind":"daemon.shutdown"}\n')
        (python_code, python), (rust_code, rust) = self.run_both(lines)
        self.assertEqual((python_code, rust_code), (0, 0))
        self.assertEqual(len(python), len(lines) + 1)
        self.assert_daemon_same(python, rust)

    def test_handshake_protocol_errors_and_shutdown(self):
        lines = [
            '{"request_id":"u1","kind":"nope.kind"}\n',
            '{"request_id":"u2","kind":"token=abc"}\n',
            '{"request_id":3,"kind":"x\\ud800"}\n',
            "not json\n",
            '{"a": 1,}\n',
            "[1, 2]\n",
            "\n",
            '{"args":{}}\n',
            '{"request_id":"e","kind":""}\n',
            "x" * 1_000_001 + "\n",
            "x" * 999_999 + "\n",
            # Outside Windows stdin splits at "\n" only; "\r" is a character.
            '{"request_id":"r1","kind":"status"}\r{"request_id":"r2","kind":"x"}\n',
            " " * 999_999 + "\r\n",
            '{"request_id":"z","kind":"supervisor.ai_secret_store.response"}\n',
            '{"request_id":"q","kind":"daemon.shutdown"}\n',
            '{"request_id":"never","kind":"nope"}\n',
        ]
        (python_code, python), (rust_code, rust) = self.run_both(lines)
        self.assertEqual((rust_code, python_code), (0, 0))
        self.assert_daemon_same(python, rust)
        self.assertEqual(json.loads(python[-1])["kind"], "daemon.shutdown")

    def test_a_carriage_return_inside_a_request_is_whitespace(self):
        (python_code, python), (rust_code, rust) = self.run_both(
            ['{"kind":\r"daemon.shutdown","request_id":9}\n', '{"request_id":"after","kind":"nope"}\n']
        )
        self.assertEqual((python_code, rust_code), (0, 0))
        self.assert_daemon_same(python, rust)
        self.assertEqual([json.loads(r)["kind"] for r in python[1:]], ["daemon.shutdown"])

    def test_end_of_input_stops_both(self):
        (python_code, python), (rust_code, rust) = self.run_both([])
        self.assertEqual((python_code, rust_code), (0, 0))
        self.assertEqual([json.loads(r)["kind"] for r in python], ["daemon.ready"])
        self.assertEqual([json.loads(r)["kind"] for r in rust], ["daemon.ready"])
        self.assert_daemon_same(python, rust)


class RustShellDriftTest(unittest.TestCase):
    """Python changes that the Rust copies must follow (no binary needed)."""

    def check(self, script: str) -> None:
        result = subprocess.run(
            [sys.executable, str(ROOT / script), "--check"], cwd=ROOT,
            capture_output=True, text=True, timeout=TIMEOUT,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_generated_grammar_matches_build_parser(self):
        self.check("crates/cli/scripts/gen_parser_spec.py")

    def test_unicode_tables_match_this_interpreter(self):
        self.check("crates/pytext/scripts/gen_unicode_tables.py")

    def test_capability_tables_match(self):
        from kassiber.command_capabilities import CLI_CAPABILITIES, DAEMON_CAPABILITIES

        recorded = json.loads(
            (ROOT / "crates/runtime/tests/data/capabilities.json").read_text(encoding="utf-8")
        )
        self.assertEqual(
            recorded,
            {
                "cli": {path: cap.value for path, cap in CLI_CAPABILITIES.items()},
                "daemon": {kind: cap.value for kind, cap in DAEMON_CAPABILITIES.items()},
            },
            "rerun crates/runtime/scripts/gen_capabilities.py and update "
            "crates/runtime/src/capabilities.rs",
        )


if __name__ == "__main__":
    unittest.main()
