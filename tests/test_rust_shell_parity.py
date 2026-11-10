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
`KASSIBER_TEST_NO_EGRESS` (a build without the attempt observer exits 78).
CLI and daemon cases use real disposable books through the storage adapter:

    cargo build -p kassiber --features test-harness
    KASSIBER_RUST_BIN=target/debug/kassiber \\
        uv run --locked python -m pytest tests/test_rust_shell_parity.py -q

The drift checks at the end need no binary: they fail when the Python
grammar, capability tables, or Unicode tables move without the Rust copies.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import closing, contextmanager
from pathlib import Path
from unittest import mock

import pytest

from tests import cli_runner
from tests.rust_port.rust_only import python_daemon_kinds, rust_only_daemon_kinds, without_rust_only_kinds

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
RUST_SKIP = "set KASSIBER_RUST_BIN to a `--features test-harness` build of crates/kassiber"
TIMEOUT = 120
if os.environ.get("KASSIBER_CI") == "1" and not RUST_BIN:
    raise RuntimeError("CI must set KASSIBER_RUST_BIN; the subprocess parity oracle cannot skip")


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
        self.assertEqual(_python_choices(rust.stdout), python.stdout, f"stdout bytes: {args!r}")
        self.assertEqual(_python_choices(rust.stderr), python.stderr, f"stderr bytes: {args!r}")
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

    def test_commands_describe_full_catalog(self):
        # The Rust tax-country registry writes two flags' help and the pool
        # scope's default (docs/reference/rust-shell.md#tax-countries);
        # every other byte of the catalog matches.
        python, rust = self.both(["--machine", "commands", "describe"])
        self.assertEqual(rust.returncode, python.returncode, (python.stderr, rust.stderr))
        self.assertEqual(rust.stderr, python.stderr)
        self.assertEqual(_registry_neutral(_python_leaves(rust.stdout)), _registry_neutral(python.stdout))
        rust_flags = _registry_flags(json.loads(rust.stdout))
        self.assertIn("us (preview)", rust_flags[("profiles create", "--tax-country")]["help"])
        # The Rust-only CLI proxy provider kind (rust-shell.md#cli-proxy-providers).
        for command in ("ai providers create", "ai providers update"):
            kind = rust_flags[(command, "--kind")]
            self.assertEqual(kind["choices"], ["local", "remote", "tee", "cli_proxy"])
            self.assertIn("cli_proxy", kind["help"])
        self.assertIn("us: global or wallet, default wallet", rust_flags[("profiles create", "--cost-basis-pool-scope")]["help"])
        catalog = json.loads(python.stdout)["data"]
        self.assertEqual(catalog["count"], 317)
        # Metadata for an unported family and leaf must have the same wire bytes.
        for path in (["init"], ["status"], ["daemon"], ["commands"],
                     ["commands", "describe"], ["wallets", "sync"], ["imports"]):
            self.assert_same(["--machine", "commands", "describe", *path], json_stdout=True)
        # The wallets family adds the Rust-only exchange sync; the rest of its
        # bytes match.
        python, rust = self.both(["--machine", "commands", "describe", "wallets"])
        self.assertEqual(_python_leaves(rust.stdout), json.dumps(json.loads(python.stdout), sort_keys=True).encode())
        bitpanda = [c for c in json.loads(rust.stdout)["data"]["commands"] if c["kind"] in RUST_ONLY_LEAVES]
        self.assertEqual([c["capability"] for c in bitpanda], ["operator"])



REGISTRY_FLAGS = ("--tax-country", "--cost-basis-pool-scope")
# Flags whose choices and help name a Rust-only value: the CLI proxy kind.
RUST_ONLY_FLAGS = {("ai providers create", "--kind"), ("ai providers update", "--kind")}

# Leaves only the Rust backend serves (crates/cli/src/spec.rs::RUST_ONLY_LEAVES,
# docs/reference/rust-shell.md#rust-only-commands).
RUST_ONLY_LEAVES = {"wallets.sync-bitpanda"}
RUST_ONLY_BACKEND_KINDS = {"bitpanda"}


def _python_choices(output: bytes) -> bytes:
    """Rust usage text without the Rust-only choices (`sync-bitpanda` in
    `wallets`, `bitpanda` in `backends --kind`); they never wrap a line, so
    every other byte must be Python's."""
    for name in (*(leaf.rsplit(".", 1)[1] for leaf in RUST_ONLY_LEAVES), *RUST_ONLY_BACKEND_KINDS):
        token = name.encode()
        output = output.replace(b"," + token + b",", b",").replace(b"," + token + b"}", b"}")
    return output


def _python_leaves(stdout: bytes) -> bytes:
    """A Rust catalog without the Rust-only leaves and backend kinds, as
    sorted JSON bytes."""
    payload = json.loads(stdout)
    commands = payload["data"]["commands"]
    kept = [c for c in commands if c["kind"] not in RUST_ONLY_LEAVES]
    for command in kept:
        if command["kind"] in {"backends.create", "backends.update"}:
            for argument in command["arguments"]:
                if argument["name"] == "kind":
                    argument["choices"] = [c for c in argument["choices"] if c not in RUST_ONLY_BACKEND_KINDS]
    payload["data"]["count"] -= len(commands) - len(kept)
    payload["data"]["commands"] = kept
    return json.dumps(payload, sort_keys=True).encode()


def _registry_flags(payload):
    """`(command, flag)` -> the argument's description, for the registry's
    flags and the Rust-only provider kind."""
    found = {}
    for command in payload["data"]["commands"]:
        for argument in command.get("arguments", []):
            for name in argument["flags"]:
                if name in REGISTRY_FLAGS or (command["command"], name) in RUST_ONLY_FLAGS:
                    found[(command["command"], name)] = argument
    return found


def _registry_neutral(stdout: bytes) -> str:
    """The catalog with the registry's help, and the Rust-only kind's help
    and extra choice, replaced."""
    payload = json.loads(stdout)
    for key, argument in _registry_flags(payload).items():
        argument["help"] = "<registry help>"
        if key in RUST_ONLY_FLAGS:
            argument["choices"] = [c for c in argument["choices"] if c != "cli_proxy"]
    return json.dumps(payload, sort_keys=True)


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
        kinds = python_daemon_kinds(rust_ready["data"]["supported_kinds"])
        self.assertTrue(set(kinds) <= set(python_ready["data"]["supported_kinds"]))
        # This is the documented R1 capability projection, not a field mask.
        served = set(rust_ready["data"]["supported_kinds"])
        python_ready["data"]["supported_kinds"] = [
            *[kind for kind in python_ready["data"]["supported_kinds"] if kind in served],
            *rust_only_daemon_kinds(rust_ready["data"]["supported_kinds"]),
            "test.stream", "test.journals.refresh", "test.emit_event", "test.late", "test.secret_store", "test.freshness.plan", "test.freshness.select", "test.freshness.invoke",
            "test.freshness.execute", "test.freshness.product.execute",
            "test.freshness.cancel", "test.freshness.pause", "test.freshness.resume",
            "test.freshness.request.cancel", "test.freshness.request.revoke",
            "test.freshness.graph.pending",
            "test.freshness.graph.start",
            "test.ai_tool", "test.ai_auto_read", "test.ai_consent_preview", "test.ai_custody_record",
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



@unittest.skipUnless(RUST_BIN, RUST_SKIP)
class RustRealStoreParityTest(_Roots):
    """Real binary, real codecs and cross-language locks; all state is disposable."""

    PASSPHRASE = "R3 shell synthetic '🔑"
    # Set by the sliced reference-file tests below: (part, count) selects
    # scenarios[part::count], and `subtest` reports each scenario.
    scenario_slice = None
    subtest = None

    def setUp(self):
        super().setUp()
        self.home = self.rust_home
        self.owners = Path(self._dir.name) / "owners"
        self.owners.mkdir(mode=0o700)
        self.root = self.home / "book" / "data"
        self.env = _environment(self.home)
        self.env.update({
            "KASSIBER_RUST_TEST_OWNER_ROOT": str(self.owners),
            "KASSIBER_TEST_NO_EGRESS": "strict",
            "KASSIBER_NO_EGRESS": "1",
        })

    @contextmanager
    def reference(self):
        # Production ownership deliberately ignores HOME. Inject the namespace
        # in the Python reference launcher, never through a product env switch.
        original = cli_runner.prefix
        script = (
            "import sys; from pathlib import Path; "
            "from kassiber.operator import project; "
            "root=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:root; "
            "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
        )
        def prefix(*, reference=False, python_options=()):
            if reference:
                return (sys.executable, "-c", script, str(self.owners))
            return original(reference=reference, python_options=python_options)
        with mock.patch.object(cli_runner, "prefix", side_effect=prefix):
            yield

    def call(self, rust, args, *, root=None, secret=None, new_secret=False, default=False, timeout=TIMEOUT):
        args = ["--machine", *([] if default else ["--data-root", str(root or self.root)]), *args]
        with tempfile.TemporaryFile() as fd:
            if secret is not None:
                fd.write(secret.encode())
                fd.seek(0)
                if new_secret:
                    args += ["--new-passphrase-fd", str(fd.fileno())]
                else:
                    args[1:1] = ["--db-passphrase-fd", str(fd.fileno())]
            kwargs = dict(cwd=ROOT, env=self.env, stdin=subprocess.DEVNULL,
                          capture_output=True, timeout=timeout,
                          pass_fds=(fd.fileno(),) if secret is not None else ())
            with self.reference():
                return (subprocess.run(_rust(args), **kwargs) if rust
                        else cli_runner.run(args, reference=True, **kwargs))

    def ok(self, result):
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        return json.loads(result.stdout)["data"]

    def same_status(self, root=None, secret=None, default=False):
        rust = self.call(True, ["status"], root=root, secret=secret, default=default)
        python = self.call(False, ["status"], root=root, secret=secret, default=default)
        self.ok(rust)
        self.ok(python)
        self.assertEqual(rust.stdout, python.stdout)
        self.assertEqual(rust.stderr, python.stderr)
        return json.loads(rust.stdout)["data"]

    def project_trace(self, calls, *, candidate):
        env = self.env | {
            cli_runner.CLI_ENV: json.dumps([sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py")]),
            "KASSIBER_HYBRID_RUST_BIN": str(Path(RUST_BIN).resolve()),
        }
        records = []
        catalog = self.home / "xdg-data/kassiber/config/projects.json"
        (self.home / "blocked").write_text("existing local file", encoding="utf-8")
        for args, secret in calls:
            argv = ["--machine", *args]
            if secret is not None:
                argv += ["--new-passphrase-fd", cli_runner.InputFd(secret.encode())]
            with mock.patch.dict(os.environ, env), self.reference():
                result = cli_runner.run(argv, reference=not candidate, env=env,
                                        capture_output=True, timeout=TIMEOUT, secret_input=secret is not None)
            records.append((result.returncode, result.stdout, result.stderr,
                            catalog.read_bytes() if catalog.exists() else None))
        return records

    def test_project_catalog_wire_and_state_match_through_hybrid(self):
        external = self.home / "external"
        calls = [(args, None) for args in [
            ["projects", "list"], ["projects", "show", "missing"],
            ["projects", "create", "Alpha"],
            ["projects", "create", "Beta", "--no-select", "--path", str(external)],
            ["projects", "show", " BETA "], ["projects", "list"],
            ["--project", "beta", "projects", "list"],
            ["projects", "select", "BETA"], ["projects", "list"],
            ["projects", "create", "Replacement", "--project-id", "beta"],
            ["projects", "create", "..."], ["projects", "select", "absent"],
            ["projects", "create", "Blocked", "--path", str(self.home / "blocked")],
            # projects dispatch precedes book selection and ignores these flags.
            ["--project", "ignored", "--data-root", str(self.root), "projects", "list"],
        ]]
        expected = self.project_trace(calls, candidate=False)
        shutil.rmtree(self.home)
        self.home.mkdir()
        actual = self.project_trace(calls, candidate=True)
        self.assertEqual(actual, expected)
        self.assertFalse((external / "data/kassiber.sqlite3").exists())

    def test_project_encrypted_creation_descriptor_matches_python(self):
        calls = [
            (["projects", "create", "Too short"], "elevenchars"),
            (["projects", "create", "Alpha", "--no-select"], self.PASSPHRASE),
            (["projects", "show", "alpha"], None),
            (["projects", "list"], None),
        ]
        expected = self.project_trace(calls, candidate=False)
        self.assertEqual([r[0] for r in expected], [1, 0, 0, 0])
        shutil.rmtree(self.home)
        self.home.mkdir()
        self.assertEqual(self.project_trace(calls, candidate=True), expected)
        data = self.home / "xdg-data/kassiber/projects/alpha/data"
        self.ok(self.call(True, ["init"], root=data, secret=self.PASSPHRASE))
        self.same_status(data, secret=self.PASSPHRASE)

    def test_init_and_status_cross_language_in_both_directions(self):
        for creator in (True, False):
            with self.subTest(rust_creator=creator):
                root = self.home / str(creator) / "data"
                self.ok(self.call(creator, ["init"], root=root))
                self.same_status(root)
                # Repeated init from the other implementation has the same
                # public result and neither duplicates nor removes backends.
                rust = self.call(True, ["init"], root=root)
                python = self.call(False, ["init"], root=root)
                self.ok(rust)
                self.ok(python)
                self.assertEqual(rust.stdout, python.stdout)
                self.same_status(root)

    def test_python_plaintext_book_with_selected_context_and_backend(self):
        self.ok(self.call(False, ["init"]))
        self.ok(self.call(False, ["workspaces", "create", "Shell workspace"]))
        self.ok(self.call(False, ["profiles", "create", "Shell profile", "--workspace", "Shell workspace"]))
        self.ok(self.call(False, ["context", "set", "--workspace", "Shell workspace", "--profile", "Shell profile"]))
        self.ok(self.call(False, ["backends", "create", "private", "--kind", "electrum", "--url", "ssl://example.invalid:50002"]))
        self.ok(self.call(False, ["backends", "set-default", "private"]))
        status = self.same_status()
        self.assertEqual(status["current_workspace"], "Shell workspace")
        self.assertEqual(status["current_profile"], "Shell profile")
        self.assertEqual(status["default_backend"], "private")

    def test_offline_preference_through_hybrid_and_locked_daemon(self):
        preference = Path(self.env["KASSIBER_OFFLINE_PREFERENCE_FILE"])
        log = self.home / "offline-served.jsonl"
        env = {**self.env, cli_runner.CLI_ENV: json.dumps([sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py")]),
               "KASSIBER_HYBRID_RUST_BIN": str(Path(RUST_BIN).resolve()), "KASSIBER_HYBRID_LOG": str(log)}
        def same(action):
            expected = self.call(False, ["offline", action])
            with mock.patch.dict(os.environ, env), self.reference():
                actual = cli_runner.run(["--machine", "--data-root", str(self.root), "offline", action],
                                        env=env, capture_output=True, timeout=TIMEOUT)
            self.assertEqual((actual.returncode, actual.stdout, actual.stderr),
                             (expected.returncode, expected.stdout, expected.stderr))
            return self.ok(actual)
        for action in ("status", "on", "status", "off"):
            same(action)
        self.assertFalse((self.root / "kassiber.sqlite3").exists())
        self.assertEqual(preference.read_bytes(), b'{"enabled": false, "schema_version": 1}\n')
        for raw in (b"not json", b"[]", b"\xff", b'{"schema_version":2,"enabled":false}',
                    b'{"schema_version":1,"enabled":0}', b'{"schema_version":true,"enabled":false}',
                    b'{"schema_version":1.0,"enabled":false}', b" " * 4097):
            preference.write_bytes(raw)
            same("status")
        same("off")
        self.assertFalse((self.root / "kassiber.sqlite3").exists())
        records = [json.loads(line) for line in log.read_text().splitlines()]
        self.assertEqual({r["command"] for r in records}, {"offline.status", "offline.on", "offline.off"})
        self.assertTrue(all(r["served_by"] == "rust" for r in records))
        if os.environ.get("KASSIBER_HYBRID_LOG"):
            with Path(os.environ["KASSIBER_HYBRID_LOG"]).open("a") as witness:
                for record in records:
                    witness.write(json.dumps(record) + "\n")

        self.encrypted_book()
        database = self.root / "kassiber.sqlite3"
        before = database.read_bytes()
        requests = [("ui.network.offline", {}), ("ui.network.offline.set", []),
                    ("ui.network.offline.set", {}), ("ui.network.offline.set", {"enabled": 1}),
                    ("ui.network.offline.set", {"enabled": True, "extra": 1}),
                    ("ui.network.offline.set", {"enabled": True}), ("ui.network.offline", {}),
                    ("ui.network.offline.set", {"enabled": False})]
        lines = [json.dumps({"kind": kind, "args": args, "request_id": index}) + "\n"
                 for index, (kind, args) in enumerate(requests)]
        with self.reference():
            expected = _daemon(["--data-root", str(self.root), "daemon"], self.env, lines, reference=True)
            actual = _daemon(["--data-root", str(self.root), "daemon"], self.env, lines)
        self.assertEqual((expected[0], actual[0]), (0, 0))
        self.assertEqual(actual[1][1:], expected[1][1:])
        self.assertEqual(database.read_bytes(), before)

    def test_network_configuration_and_partition_export_through_hybrid(self):
        from kassiber.db import open_db, set_setting
        from kassiber.backup.pack import import_backup
        vectors = json.loads((ROOT / "crates/setup/fixtures/python_networks.json").read_text())
        def seed(root, index):
            connection = open_db(root)
            connection.execute("PRAGMA foreign_keys=OFF")
            for sql in vectors["scenarios"][index]["seed"]:
                connection.executescript(sql)
            set_setting(connection, "context_workspace", "ws-1")
            set_setting(connection, "context_profile", "profile-1")
            connection.commit()
            connection.close()
        seed(self.root, 1)
        partition_root = self.home / "partition/data"
        seed(partition_root, 5)
        log = self.home / "networks-served.jsonl"
        env = {**self.env, cli_runner.CLI_ENV: json.dumps([sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py")]),
               "KASSIBER_HYBRID_RUST_BIN": str(Path(RUST_BIN).resolve()), "KASSIBER_HYBRID_LOG": str(log)}
        def candidate(args, root=None):
            with mock.patch.dict(os.environ, env), self.reference():
                return cli_runner.run(["--machine", "--data-root", str(root or self.root), *args],
                                      env=env, capture_output=True, timeout=TIMEOUT, secret_input=any(isinstance(v, cli_runner.InputFd) for v in args))
        def same(args, root=None):
            expected = self.call(False, args, root=root)
            actual = candidate(args, root)
            self.assertEqual((actual.returncode, actual.stdout, actual.stderr),
                             (expected.returncode, expected.stdout, expected.stderr))
            return json.loads(actual.stdout)
        same(["networks", "inventory"])
        preview = same(["networks", "plan", "--environment", "main"])["data"]
        self.assertTrue(preview["can_apply"])
        same(["networks", "bind", "--environment", "main", "--plan-id", "stale"])
        bound = self.ok(candidate(["networks", "bind", "--environment", "main", "--plan-id", preview["plan_id"]]))
        self.assertEqual(bound["environment"], "main")
        self.assertEqual(same(["networks", "inventory"])["data"]["binding"], bound)
        same(["networks", "plan", "--environment", "main"])
        split = ["networks", "split-plan", "--environment", "main", "--wallet", "wallet-a"]
        preview = same(split, partition_root)["data"]
        self.assertTrue(preview["can_apply"])
        destination = self.home / "partition.kassiber"
        export = ["networks", "split", "--environment", "main", "--wallet", "wallet-a", "--file", str(destination)]
        same([*export, "--plan-id", "stale"], partition_root)
        result = candidate([*export, "--plan-id", preview["plan_id"], "--backup-passphrase-fd",
                            cli_runner.InputFd(b"partition-test-password")], partition_root)
        self.assertEqual(self.ok(result)["file_path"], str(destination))
        restored = self.home / "restored"
        import_backup(destination, restored, backup_passphrase="partition-test-password", move_into_place=True)
        connection = open_db(restored, require_existing_schema=True)
        try:
            self.assertEqual([row[0] for row in connection.execute("SELECT id FROM transactions")], ["main"])
            self.assertEqual([row[0] for row in connection.execute("SELECT id FROM wallets")], ["wallet-a"])
            self.assertEqual(connection.execute("SELECT count(*) FROM backends").fetchone()[0], 0)
            self.assertEqual(connection.execute("SELECT environment FROM book_network_bindings").fetchone()[0], "main")
        finally:
            connection.close()
        records = [json.loads(line) for line in log.read_text().splitlines()]
        self.assertEqual({r["command"] for r in records}, {"networks.inventory", "networks.plan", "networks.bind", "networks.split-plan", "networks.split"})
        self.assertTrue(all(r["served_by"] == "rust" for r in records))
        # Preserve a witness for orchestrator runs that supply their own log.
        if os.environ.get("KASSIBER_HYBRID_LOG"):
            with Path(os.environ["KASSIBER_HYBRID_LOG"]).open("a") as witness:
                for record in records:
                    witness.write(json.dumps(record) + "\n")

    def test_setup_services_match_python_at_process_boundaries(self):
        self.ok(self.call(False, ["init"]))
        self.ok(self.call(False, ["workspaces", "create", "Shell workspace"]))
        self.ok(self.call(False, ["profiles", "create", "Shell profile"]))

        # Frozen vectors cover generated IDs/times. At the real process boundary,
        # compare deterministic reads, edits, deletes and errors byte for byte.
        self.ok(self.call(False, ["views", "create", "--surface", "swap", "--name", "Stored", "--filter", '{"é": [true, 1, null]}']))
        views = self.ok(self.call(False, ["views", "list"]))
        database = self.root / "kassiber.sqlite3"
        operations = [
            ["workspaces", "list"], ["profiles", "list"], ["profiles", "get"],
            ["accounts", "list"], ["context", "show"], ["context", "current"],
            ["context", "set", "--workspace", "Shell workspace", "--profile", "Shell profile"],
            ["context", "set", "--profile", "absent"], ["context", "set"],
            ["profiles", "set", "--gains-algorithm", "HIFO"],
            ["profiles", "set", "--label", "Renamed"],
            ["profiles", "set", "--tax-country", "at", "--gains-algorithm", "FIFO"],
            ["profiles", "set", "--tax-long-term-days", "-1"],
            ["profiles", "set", "--cost-basis-pool-scope", "wallet"],
            ["profiles", "set"], ["views", "list"],
            ["views", "list", "--surface", "swap"],
            ["views", "delete", "--view-id", views[0]["id"]],
            ["views", "delete", "--view-id", "absent"],
            ["views", "create", "--surface", "swap", "--name", "bad", "--filter", "["],
            ["workspaces", "create", "Shell workspace"],
            ["profiles", "create", "Shell profile"],
            ["accounts", "create", "--code", "treasury", "--label", "Duplicate", "--type", "asset"],
        ]
        for operation in operations:
            with self.subTest(operation=operation):
                initial = database.read_bytes()
                python = self.call(False, operation)
                database.write_bytes(initial)
                rust = self.call(True, operation)
                self.assertEqual((rust.returncode, rust.stdout, rust.stderr),
                                 (python.returncode, python.stdout, python.stderr))
                # The standalone Rust vectors compare logical table state;
                # restore this scenario's starting book for the next operation.
                database.write_bytes(initial)

        profile = self.ok(self.call(False, ["profiles", "get"]))
        scope = {"workspace_id": profile["workspace_id"], "profile_id": profile["id"]}
        requests = [
            ("ui.networks.binding", {}),
            ("ui.networks.inventory", {}),
            ("ui.networks.plan", {"environment": "main"}),
            ("ui.networks.bind", {"environment": "main", "plan_id": "stale"}),
            ("ui.networks.partition_plan", {"environment": "main", "wallet_ids": []}),
            ("ui.networks.partition_export", {"environment": "main", "wallet_ids": [], "plan_id": "stale", "file_path": str(self.home / "refused.kassiber")}),
            ("ui.saved_views.list", {}),
            ("ui.profiles.update", {"profile_id": profile["id"], "tax_country": "at", "gains_algorithm": "FIFO"}),
            ("ui.profiles.rename", {"profile_id": profile["id"], "label": "Desktop renamed"}),
            ("ui.profiles.switch", {"profile_id": profile["id"]}),
            ("ui.profiles.update", {"profile_id": profile["id"], "tax_country": " "}),
            ("ui.profiles.create", {"workspace_id": profile["workspace_id"], "label": "Desktop renamed"}),
            ("ui.saved_views.create", {"surface": "swap", "name": "Stored"}),
            ("ui.saved_views.create", {"surface": "swap", "name": "bad", "filter": [1]}),
            ("ui.saved_views.delete", {"view_id": views[0]["id"], "expected_scope": scope}),
            ("ui.saved_views.list", {"expected_scope": scope}),
            ("ui.saved_views.delete", {"view_id": views[0]["id"]}),
            ("ui.saved_views.list", {"expected_scope": {**scope, "profile_id": "stale"}}),
            ("ui.profiles.switch", {"profile_id": "absent"}),
            ("ui.profiles.update", [1]),
            ("daemon.shutdown", {}),
        ]
        lines = [json.dumps({"request_id": index, "kind": kind, "args": args}) + "\n"
                 for index, (kind, args) in enumerate(requests)]
        initial = database.read_bytes()
        with self.reference():
            expected = _daemon(["--data-root", str(self.root), "daemon"], self.env, lines, reference=True)
            database.write_bytes(initial)
            actual = _daemon(["--data-root", str(self.root), "daemon"], self.env, lines)
        self.assertEqual(expected[0], actual[0])
        self.assertEqual(expected[1][1:], actual[1][1:])
        database.write_bytes(initial)

    def test_backend_configuration_through_hybrid_and_daemon(self):
        self.ok(self.call(False, ["init"]))
        database = self.root / "kassiber.sqlite3"
        log = self.home / "backends-served.jsonl"
        env = self.env | {
            cli_runner.CLI_ENV: json.dumps([sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py")]),
            "KASSIBER_HYBRID_RUST_BIN": str(Path(RUST_BIN).resolve()),
            "KASSIBER_HYBRID_LOG": str(log),
            "KASSIBER_RUST_TEST_SETUP_TIMESTAMP": "2026-01-01T00:00:00Z",
        }
        original = cli_runner.prefix
        script = (
            "import sys; from pathlib import Path; "
            "from kassiber.operator import project; "
            "root=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:root; "
            "import kassiber.backends; kassiber.backends.now_iso=lambda:'2026-01-01T00:00:00Z'; "
            "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
        )
        def prefix(*, reference=False, python_options=()):
            if reference:
                return (sys.executable, "-c", script, str(self.owners))
            return original(reference=reference, python_options=python_options)
        def same(args, *, input=None):
            initial = database.read_bytes()
            argv = ["--machine", "--data-root", str(self.root), "backends", *args]
            with mock.patch.object(cli_runner, "prefix", side_effect=prefix), mock.patch.dict(os.environ, env):
                expected = cli_runner.run(argv, reference=True, env=env, input=input, capture_output=True, timeout=TIMEOUT)
                database.write_bytes(initial)
                actual = cli_runner.run(argv, env=env, input=input, capture_output=True, timeout=TIMEOUT)
            # The Rust-only `bitpanda` kind is the one extra catalog entry.
            self.assertEqual((actual.returncode, without_rust_only_kinds(actual.stdout), actual.stderr),
                             (expected.returncode, expected.stdout, expected.stderr), args)
            return actual
        same(["list"])
        same(["kinds"])
        self.ok(same(["create", "private", "--kind", "bitcoinrpc", "--url", "http://alice:synthetic-url@localhost:18443/?key=synthetic-query",
                      "--token-fd", cli_runner.InputFd(b"synthetic-token\n"), "--username-fd", cli_runner.InputFd(b"synthetic-user"),
                      "--password-fd", cli_runner.InputFd(b"synthetic-password"), "--certificate", "/synthetic/ca.pem"]))
        same(["get", " PRIVATE "])
        same(["list"])
        same(["reveal-token", "private"])
        same(["update", "private", "--url", "http://alice:synthetic-password@[invalid]/?token=synthetic-token",
              "--tor-proxy", "alice:synthetic-password@example.test:synthetic-token"])
        same(["get", "private"])
        same(["update", "private", "--token-stdin"], input=b"synthetic-replacement\r\n")
        same(["update", "private", "--clear", "token", "--clear", "password", "--display-name", "Edited"])
        same(["update", "private", "--token", "synthetic-argv-token"])
        same(["set-default", "private"])
        same(["clear-default"])
        same(["delete", "private"])
        same(["get", "private"])
        records = [json.loads(line) for line in log.read_text().splitlines()]
        self.assertEqual({r["command"] for r in records}, {"backends." + command for command in
                         ("list", "kinds", "get", "create", "update", "delete", "set-default", "clear-default", "reveal-token")})
        self.assertTrue(all(r["served_by"] == "rust" for r in records))
        if os.environ.get("KASSIBER_HYBRID_LOG"):
            with Path(os.environ["KASSIBER_HYBRID_LOG"]).open("a") as witness:
                for record in records:
                    witness.write(json.dumps(record) + "\n")
        requests = [
            ("ui.backends.options", {}), ("ui.backends.public_defaults", {}), ("ui.backends.settings.list", {}),
            ("ui.backends.create", {"name":"desktop", "kind":"electrum", "url":"ssl://example.test:50002", "token":"synthetic-token", "config":{"certificate":"/synthetic/ca.pem"}}),
            ("ui.backends.update", {"name":"desktop", "config":{"display_name":"Desktop edited"}, "clear":["token"]}),
            ("ui.backends.update", {"name":"desktop", "url":"https://alice:synthetic-password@/?token=synthetic-token",
                                    "tor_proxy":"socks5h://alice:synthetic-password@[invalid]:9050"}),
            ("ui.backends.set_default", {"name":"desktop"}), ("status", {}), ("ui.backends.settings.list", {}),
            ("ui.backends.delete", {"name":"desktop"}), ("ui.backends.options", {}),
            ("ui.backends.update", {"name":"fulcrum", "timeout":"bad"}),
            ("ui.backends.create", {"name":"remote-cookie", "kind":"bitcoinrpc", "url":"http://example.test", "config":{"cookiefile":"/synthetic/.cookie"}}),
            *[(kind,{}) for kind in ("ui.backends.bitcoinrpc.test", "ui.backends.btcpay.test", "ui.backends.electrum.test", "ui.backends.http.test", "ui.backends.lightning.test", "ui.backends.detect_core")],
        ]
        # Probe acceptance uses owned loopback fixtures in the chain suite.
        local_requests = requests[:-6]
        lines = [json.dumps({"request_id": i, "kind": kind, "args": args}) + "\n"
                 for i, (kind, args) in enumerate(local_requests)]
        initial = database.read_bytes()
        def exchange(*, reference=False):
            import queue
            import threading
            options = dict(cwd=ROOT, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            proc = (cli_runner.popen(["--data-root", str(self.root), "daemon"], reference=True, **options) if reference
                    else subprocess.Popen(_rust(["--data-root", str(self.root), "daemon"]), **options))
            output = queue.Queue()
            def read():
                for line in proc.stdout:
                    output.put(line)
            reader = threading.Thread(target=read, daemon=True)
            reader.start()
            result = [output.get(timeout=TIMEOUT)]
            try:
                for line in lines:
                    request = json.loads(line)
                    proc.stdin.write(line.encode())
                    proc.stdin.flush()
                    while True:
                        response = output.get(timeout=TIMEOUT)
                        result.append(response)
                        if json.loads(response).get("request_id") == request["request_id"]:
                            break
                proc.stdin.close()
                proc.wait(timeout=TIMEOUT)
                reader.join(timeout=TIMEOUT)
                self.assertEqual(proc.stderr.read(), b"")
                return proc.returncode, result
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=TIMEOUT)
        with mock.patch.object(cli_runner, "prefix", side_effect=prefix):
            expected = exchange(reference=True)
            database.write_bytes(initial)
            actual = exchange()
        self.assertEqual((expected[0], actual[0]), (0, 0))
        self.assertEqual(actual[1][1:], expected[1][1:])
        served = set(json.loads(actual[1][0])["data"]["supported_kinds"])
        # The backend kinds are native now; any that is not must still refuse.
        unported = [(kind, args) for kind, args in requests[-6:] if kind not in served]
        if unported:
            refused = _daemon(["--data-root", str(self.root), "daemon"], env,
                              [json.dumps({"request_id":i,"kind":kind,"args":args})+"\n" for i,(kind,args) in enumerate(unported)])
            for line in refused[1][1:]:
                self.assertEqual(json.loads(line)["error"]["code"], "not_implemented")

    def test_init_backend_bootstrap_and_ephemeral_environment_match_python(self):
        import sqlite3
        env_file = self.home / "bootstrap.env"
        env_file.write_text(
            "KASSIBER_BACKEND_PRIVATE_KIND=bitcoinrpc\n"
            "KASSIBER_BACKEND_PRIVATE_URL='http://example.invalid:18443'\n"
            "KASSIBER_BACKEND_PRIVATE_CHAIN=btc\n"
            "KASSIBER_BACKEND_PRIVATE_NETWORK=regtest\n"
            "KASSIBER_BACKEND_PRIVATE_TIMEOUT=15\n"
            "KASSIBER_BACKEND_PRIVATE_RPCUSER=synthetic\n"
            "KASSIBER_BACKEND_PRIVATE_INSECURE=yes\n"
            "KASSIBER_DEFAULT_BACKEND=private\n", encoding="utf-8",
        )
        self.env.update({
            "KASSIBER_BACKEND_EPHEMERAL_KIND": "electrum",
            "KASSIBER_BACKEND_EPHEMERAL_URL": "tcp://example.invalid:50001",
            "KASSIBER_DEFAULT_BACKEND": "ephemeral",
        })
        stored = []
        for creator in (True, False):
            with self.subTest(rust_creator=creator):
                root = self.home / str(creator) / "data"
                self.ok(self.call(creator, ["--env-file", str(env_file), "init"], root=root))
                conn = sqlite3.connect(root / "kassiber.sqlite3")
                stored.append(conn.execute(
                    "SELECT name,kind,chain,network,url,auth_header,token,batch_size,timeout,tor_proxy,config_json,notes FROM backends ORDER BY name"
                ).fetchall())
                self.assertNotIn("ephemeral", [row[0] for row in stored[-1]])
                conn.close()
                self.assertEqual(self.same_status(root)["default_backend"], "ephemeral")
        self.assertEqual(stored[0], stored[1])
        del self.env["KASSIBER_DEFAULT_BACKEND"]
        for creator in (True, False):
            root = self.home / str(creator) / "data"
            self.assertEqual(self.same_status(root)["default_backend"], "private")

    def encrypted_book(self):
        self.ok(self.call(False, ["init"]))
        self.ok(self.call(False, ["secrets", "init"], secret=self.PASSPHRASE, new_secret=True))

    def test_sqlcipher_fd_unlock_and_no_journal_change(self):
        from kassiber.secrets import sqlcipher
        self.encrypted_book()
        def mode():
            conn = sqlcipher.open_encrypted(self.root / "kassiber.sqlite3", self.PASSPHRASE, enforce_operator_identity=False)
            try:
                return conn.execute("PRAGMA journal_mode").fetchone()[0]
            finally:
                conn.close()
        before = mode()
        status = self.same_status(secret=self.PASSPHRASE)
        self.assertTrue(status["database_encrypted"])
        self.assertEqual(mode(), before)
        missing = self.call(True, ["status"])
        self.assertEqual(json.loads(missing.stdout)["error"]["code"], "interaction_required")
        wrong = self.call(True, ["status"], secret="synthetic wrong key")
        self.assertEqual(json.loads(wrong.stdout)["error"]["code"], "unlock_failed")
        self.assertNotIn(self.PASSPHRASE.encode(), missing.stdout + wrong.stdout + wrong.stderr)

    @contextmanager
    def daemon(self, rust, *, root=None):
        args = ["--data-root", str(root or self.root), "daemon"]
        with self.reference():
            process = (subprocess.Popen(_rust(args), cwd=ROOT, env=self.env,
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE) if rust
                       else cli_runner.popen(args, reference=True, cwd=ROOT, env=self.env,
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE))
        try:
            self.assertEqual(self.read(process)["kind"], "daemon.ready")
            yield process
        finally:
            if process.poll() is None:
                process.stdin.close()
                try:
                    process.wait(timeout=TIMEOUT)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=TIMEOUT)
            process.stdout.close()
            process.stderr.close()

    def read(self, process):
        import queue
        import threading
        records = queue.Queue()
        threading.Thread(target=lambda: records.put(process.stdout.readline()), daemon=True).start()
        raw = records.get(timeout=TIMEOUT)
        self.assertTrue(raw, f"daemon exited {process.poll()}")
        return json.loads(raw)

    def request(self, process, kind, args=None):
        process.stdin.write((json.dumps({"request_id": kind, "kind": kind, "args": args or {}}) + "\n").encode())
        process.stdin.flush()
        return self.read(process)

    def test_daemon_ownership_exclusion_both_directions_and_release(self):
        self.ok(self.call(False, ["init"]))
        for first in (True, False):
            with self.subTest(rust_first=first):
                with self.daemon(first) as owner:
                    self.assertEqual(self.request(owner, "status")["kind"], "status")
                    with self.daemon(not first) as contender:
                        response = self.request(contender, "status")
                        self.assertEqual(response["error"]["code"], "project_in_use")
                        self.assertEqual(self.request(owner, "daemon.lock")["kind"], "daemon.lock")
                        self.assertEqual(self.request(contender, "status")["kind"], "status")
                        self.assertEqual(self.request(contender, "daemon.shutdown")["kind"], "daemon.shutdown")
                    self.assertEqual(self.request(owner, "status")["kind"], "status")
                    self.assertEqual(self.request(owner, "daemon.shutdown")["kind"], "daemon.shutdown")

    def test_fresh_daemon_retains_lock_after_schema_creation(self):
        with self.daemon(True) as daemon:
            for _ in range(2):
                self.assertEqual(self.request(daemon, "status")["kind"], "status")
            with self.daemon(False) as contender:
                self.assertEqual(self.request(contender, "status")["error"]["code"], "project_in_use")
            self.assertEqual(self.request(daemon, "daemon.shutdown")["kind"], "daemon.shutdown")

    def test_sqlcipher_daemon_unlock_status_lock_and_shutdown(self):
        self.encrypted_book()
        with self.daemon(True) as daemon:
            self.assertEqual(self.request(daemon, "status")["kind"], "auth_required")
            self.assertEqual(self.request(daemon, "daemon.unlock")["kind"], "auth_required")
            self.assertEqual(self.request(daemon, "daemon.unlock", {"auth_response": {"passphrase_secret": "wrong"}})["error"]["code"], "local_auth_denied")
            response = self.request(daemon, "daemon.unlock", {"auth_response": {"passphrase_secret": self.PASSPHRASE}, "require_existing_project": True})
            self.assertEqual(response["kind"], "daemon.unlock", response)
            self.assertTrue(response["data"]["status"]["database_encrypted"])
            self.assertEqual(self.request(daemon, "status")["kind"], "status")
            self.assertEqual(self.request(daemon, "daemon.lock")["kind"], "daemon.lock")
            self.assertEqual(self.request(daemon, "status")["kind"], "auth_required")
            self.assertEqual(self.request(daemon, "daemon.shutdown")["kind"], "daemon.shutdown")

    def test_secret_store_control_channel_with_real_open_book(self):
        self.ok(self.call(True, ["init"]))
        with self.daemon(True) as daemon:
            self.assertEqual(self.request(daemon, "status")["kind"], "status")
            request = self.request(daemon, "test.secret_store", {
                "provider_name": "synthetic", "store_id": "synthetic",
                "service": "synthetic", "account": "synthetic", "timeout_ms": 1000,
            })
            self.assertEqual(request["kind"], "supervisor.ai_secret_store.request")
            daemon.stdin.write((json.dumps({
                "kind": "supervisor.ai_secret_store.response", "request_id": request["request_id"],
                "data": {"synthetic": True},
            }) + "\n").encode())
            daemon.stdin.flush()
            self.assertEqual(self.read(daemon)["data"], {"synthetic": True})
            self.assertEqual(self.request(daemon, "status")["kind"], "status")
            self.assertEqual(self.request(daemon, "daemon.shutdown")["kind"], "daemon.shutdown")

    def test_default_init_and_hidden_home_then_legacy_project_migration(self):
        # Start with a flat hidden-home book. Explicit selection bypasses the
        # migration; the hidden switch then exercises the rename-only cutover.
        legacy = self.home / ".kassiber" / "data"
        self.ok(self.call(False, ["init"], root=legacy))
        old_inode = (legacy / "kassiber.sqlite3").stat().st_ino
        moved = subprocess.run(_rust(["--migrate-default-state-root"]), env=self.env, capture_output=True, timeout=TIMEOUT)
        self.assertEqual((moved.returncode, moved.stdout, moved.stderr), (0, b"", b""))
        native = self.home / "xdg-data" / "kassiber"
        self.assertEqual((native / "data/kassiber.sqlite3").stat().st_ino, old_inode)
        status = self.same_status(default=True)
        self.assertEqual(status["project_id"], "default")
        self.assertEqual(status["data_root"], str(native / "projects/default/data"))
        self.ok(self.call(True, ["init"], default=True))
        self.same_status(default=True)

    def test_new_default_project_and_unported_commands_stay_closed(self):
        self.ok(self.call(True, ["init"], default=True))
        self.same_status(default=True)
        unrelated = self.home / "unported" / "data"
        # Any read this build has not ported must fail closed before it
        # resolves or creates the requested book. A test-harness build ports
        # every leaf; product builds keep chat and the MCP server gated, which
        # crates/kassiber/tests/binary.rs walks.
        from tests.test_rust_hybrid_candidate import unported_read_witnesses

        for command in unported_read_witnesses(RUST_BIN, self.home)[:1]:
            response = self.call(True, command, root=unrelated)
            self.assertEqual(json.loads(response.stdout)["error"]["code"], "not_implemented")
            self.assertFalse(unrelated.exists())

    def import_run_book(self):
        """Seed through the Python reference; both runtimes see identical ids."""
        self.ok(self.call(False, ["init"]))
        self.ok(self.call(False, ["workspaces", "create", "Import workspace"]))
        self.ok(self.call(False, ["profiles", "create", "Import profile", "--workspace", "Import workspace"]))
        self.ok(self.call(False, ["context", "set", "--workspace", "Import workspace", "--profile", "Import profile"]))
        self.ok(self.call(False, ["wallets", "create", "--label", "Import source", "--kind", "custom"]))
        source = self.home / "source.json"
        source.write_text(json.dumps([
            {"external_id": "first", "occurred_at": "2024-01-01T00:00:00Z", "amount": "0.1", "asset": "BTC"},
            {"external_id": "second", "occurred_at": "2024-01-02T00:00:00Z", "amount": "-0.01", "asset": "BTC", "fee": "0.000001"},
        ]), encoding="utf-8")
        return self.ok(self.call(False, ["wallets", "import-json", "--wallet", "Import source", "--file", str(source)]))["import_batch_id"]

    def test_wallet_derive_wire_bytes_match_reference(self):
        self.import_run_book()
        vectors = json.loads((ROOT / "crates/wallets/tests/vectors/python_descriptors.json").read_text())
        selected = {"pkh", "wpkh-pair", "sh-wpkh", "tr", "tr-script-tree", "wsh-multi",
                    "wsh-sortedmulti", "sh-wsh-sortedmulti", "sh-multi", "wpkh-fixed",
                    "static-key", "tpub-network-signet", "tpub-network-regtest",
                    "multi-script-p2tr+p2wpkh+p2wpkh", "liquidv1-eltr", "liquidtestnet-elwpkh",
                    "elementsregtest-elwsh", "liquid-tr-script-tree"}
        cases = [case for case in vectors["plans"] if case["name"] in selected]
        self.assertEqual(len(cases), len(selected))
        for case in cases:
            config = dict(case["config"])
            if config.get("chain") == "liquid":
                config["backend"] = "liquid"
            self.ok(self.call(False, ["wallets", "create", "--label", case["name"],
                                     "--kind", "descriptor", "--config", json.dumps(config)]))
            for suffix in (["--count", "2"], ["--start", "3", "--count", "2"],
                           ["--branch", "receive", "--count", "1"], ["--count", "-1"],
                           ["--start", "-1"], ["--branch", "99"]):
                with self.subTest(case=case["name"], suffix=suffix):
                    args = ["wallets", "derive", "--wallet", case["name"], *suffix]
                    python, rust = self.call(False, args), self.call(True, args)
                    self.assertEqual((rust.returncode, rust.stdout, rust.stderr),
                                     (python.returncode, python.stdout, python.stderr))
        for suffix in (["--start", "4294967296"], ["--count", "0"], ["--count", "4294967296"]):
            args = ["wallets", "derive", "--wallet", "wpkh-fixed", *suffix]
            python, rust = self.call(False, args), self.call(True, args)
            self.assertEqual((rust.returncode, rust.stdout, rust.stderr),
                             (python.returncode, python.stdout, python.stderr))
        args = ["wallets", "derive", "--wallet", "wpkh-pair", "--start", "2147483648", "--count", "1"]
        python, rust = self.call(False, args), self.call(True, args)
        self.assertEqual((rust.returncode, rust.stdout, rust.stderr),
                         (python.returncode, python.stdout, python.stderr))

    def test_descriptor_preview_daemon_wire_bytes_match_reference(self):
        self.ok(self.call(False, ["init"]))
        vectors = json.loads((ROOT / "crates/wallets/tests/vectors/python_descriptors.json").read_text())
        cases = [case["args"] for case in vectors["previews"]]
        cases.extend([{}, {"descriptor": 1}, {"wallet_material": " "},
                      {"script_types": {}}, {"script_types": [1]}, {"count": []}])
        requests = [{"request_id": str(index), "kind": "ui.wallets.preview_descriptor", "args": args}
                    for index, args in enumerate(cases)]
        requests.append({"request_id": "quit", "kind": "daemon.shutdown"})
        lines = [json.dumps(request) + "\n" for request in requests]
        args = ["--data-root", str(self.root), "daemon"]
        with self.reference():
            python = _daemon(args, self.env, lines, reference=True)
        rust = _daemon(args, self.env, lines)
        self.assertEqual(rust[0], python[0])
        self.assertEqual(rust[1][1:], python[1][1:])

    def test_wallet_lifecycle_cli_wire_bytes_and_state_match_reference(self):
        import shutil
        import sqlite3
        self.import_run_book()
        other = self.home / "candidate" / "data"
        shutil.copytree(self.root, other)
        cases = [
            ["wallets", "list"],
            ["wallets", "get", "--wallet", " import SOURCE "],
            ["wallets", "get", "--wallet", "missing"],
            ["wallets", "update", "--wallet", "Import source"],
            ["wallets", "update", "--wallet", "Import source", "--clear", "missing"],
            ["wallets", "update", "--wallet", "Import source", "--config", '{"backend":"local","deprecated":"yes"}'],
            ["wallets", "get", "--wallet", "Import source"],
            ["wallets", "update", "--wallet", "Import source", "--label", "Archive"],
            ["wallets", "delete", "--wallet", "Archive"],
            ["wallets", "delete", "--wallet", "Archive", "--cascade"],
            ["wallets", "list"],
        ]
        for args in cases:
            with self.subTest(args=args):
                python = self.call(False, args)
                rust = self.call(True, args, root=other)
                self.assertEqual((rust.returncode, rust.stdout, rust.stderr),
                                 (python.returncode, python.stdout, python.stderr))
        # No generated identities or clocks occur in this mutation sequence.
        def dump(root):
            with sqlite3.connect(root / "kassiber.sqlite3") as conn:
                return list(conn.iterdump())
        self.assertEqual(dump(self.root), dump(other))

    def test_successful_wallet_create_matches_reference_clock_entropy_and_storage(self):
        import shutil
        import sqlite3
        self.import_run_book()
        other = self.home / "candidate" / "data"
        shutil.copytree(self.root, other)
        fixed_id = "11111111-2222-4333-8444-555555555555"
        fixed_time = "2025-01-02T03:04:05Z"
        script = (
            "import sys; from pathlib import Path; from types import SimpleNamespace; "
            "from kassiber.operator import project; "
            "root=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:root; "
            "from kassiber.core import wallets; "
            f"wallets.uuid=SimpleNamespace(uuid4=lambda:{fixed_id!r}); wallets.now_iso=lambda:{fixed_time!r}; "
            "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
        )
        args = ["wallets", "create", "--label", "SP local", "--kind", "silent-payment", "--sp-descriptor", "sp(spscan1q" + "q"*40 + ")", "--backend", "local", "--sp-scan-start-height", "850000", "--birthday", "2025-01-02"]
        with mock.patch.object(cli_runner, "prefix", return_value=(sys.executable, "-c", script, str(self.owners))):
            python = cli_runner.run(["--machine", "--data-root", str(self.root), *args], reference=True, cwd=ROOT, env=self.env, capture_output=True, timeout=TIMEOUT)
        self.env.update({"KASSIBER_RUST_TEST_WALLET_ID":fixed_id,"KASSIBER_RUST_TEST_WALLET_TIME":fixed_time})
        rust = self.call(True, args, root=other)
        self.assertEqual((rust.returncode,rust.stdout,rust.stderr),(python.returncode,python.stdout,python.stderr))
        self.ok(rust)
        def dump(root):
            with sqlite3.connect(root / "kassiber.sqlite3") as conn:
                return list(conn.iterdump())
        self.assertEqual(dump(other),dump(self.root))

    def test_wallet_birthday_and_malformed_config_wire_bytes_match_reference(self):
        import shutil
        self.import_run_book()
        other = self.home / "candidate" / "data"
        shutil.copytree(self.root, other)
        cases = [
            ["wallets", "update", "--wallet", "Import source", "--birthday", "2025-01-02T01:02:03 +01:00"],
            ["wallets", "get", "--wallet", "Import source"],
            ["wallets", "update", "--wallet", "Import source", "--birthday", "bad"],
            ["wallets", "update", "--wallet", "Import source", "--config", '{"birthday":""}'],
        ]
        vectors = json.loads((ROOT / "crates/wallets/tests/vectors/python_descriptors.json").read_text())
        for case in vectors["configs"]:
            if case["name"].startswith("malformed-descriptor-") and "error" in case["expected"]:
                cases.append(["wallets", "create", "--label", "Refused", "--kind", case["kind"], "--config", json.dumps(case["config"])])
            if case["name"].startswith("sp-") and "error" in case["expected"]:
                cases.append(["wallets", "create", "--label", "Refused", "--kind", case["kind"], "--config", json.dumps(case["config"])])
        for args in cases:
            with self.subTest(args=args):
                python = self.call(False, args)
                rust = self.call(True, args, root=other)
                self.assertEqual((rust.returncode, rust.stdout, rust.stderr),
                                 (python.returncode, python.stdout, python.stderr))

    def test_wallet_list_snapshot_daemon_wire_bytes_match_reference(self):
        import sqlite3
        self.import_run_book()
        vectors = json.loads((ROOT / "crates/wallets/tests/vectors/python_descriptors.json").read_text())
        config = next(v["config"] for v in vectors["configs"] if v["name"] == "sp-default")
        config.update({"birthday": "2025-01-02T00:00:00Z", "samourai": {"watch_only": True, "mix_count": "2", "descriptor": "hidden"}, "btcpay_provenance": [{"backend": "LOCAL", "store_id": "store", "payment_method_id": "btc-ln"}]})
        with sqlite3.connect(self.root / "kassiber.sqlite3") as conn:
            conn.execute("UPDATE wallets SET kind='silent-payment',config_json=?", (json.dumps(config, sort_keys=True),))
        lines = [json.dumps({"request_id": str(i), "kind": kind, "args": args}) + "\n" for i, (kind, args) in enumerate([
            ("ui.wallets.list", {}), ("ui.wallets.list", []), ("daemon.shutdown", {}),
        ])]
        with self.reference():
            python = _daemon(["--data-root", str(self.root), "daemon"], self.env, lines, reference=True)
        rust = _daemon(["--data-root", str(self.root), "daemon"], self.env, lines)
        self.assertEqual(rust[0], python[0])
        self.assertEqual(rust[1][1:], python[1][1:])

    def test_wallet_lifecycle_daemon_confirmation_wire_bytes_match_reference(self):
        import shutil
        self.import_run_book()
        other = self.home / "candidate" / "data"
        shutil.copytree(self.root, other)
        requests = [
            ("ui.wallets.create", {"label": "Invalid", "kind": "descriptor", "gap_limit": []}),
            ("ui.wallets.create", {"label": "Invalid", "kind": "address"}),
            ("ui.wallets.update", {"wallet": "Import source", "label": "Archive"}),
            ("ui.wallets.update", {"wallet": "Import source", "clear": "bad"}),
            ("ui.wallets.update", {"wallet": "Import source", "script_types": []}),
            ("ui.wallets.update", {"wallet": "Import source", "label": "Archive", "auth_response": {"plaintext_change_ack": "CHANGE LOCAL DATA"}}),
            ("ui.wallets.delete", {"wallet": "Archive"}),
            ("ui.wallets.delete", {"wallet": "Archive", "confirm": "DELETE", "cascade": "yes"}),
            ("ui.wallets.delete", {"wallet": "Archive", "confirm": "DELETE", "confirm_wallet": "stale"}),
            ("ui.wallets.delete", {"wallet": "Archive", "confirm": "DELETE", "confirm_wallet": "Archive"}),
            ("ui.wallets.delete", {"wallet": "Archive", "confirm": "DELETE", "confirm_wallet": "Archive", "auth_response": {"plaintext_delete_ack": "DELETE LOCAL DATA"}}),
            ("ui.wallets.delete", {"wallet": "Archive", "confirm": "DELETE", "confirm_wallet": "Archive", "cascade": True, "auth_response": {"plaintext_delete_ack": "DELETE LOCAL DATA"}}),
            ("daemon.shutdown", {}),
        ]
        lines = [json.dumps({"request_id": str(i), "kind": kind, "args": args}) + "\n"
                 for i, (kind, args) in enumerate(requests)]
        with self.reference():
            python = _daemon(["--data-root", str(self.root), "daemon"], self.env, lines, reference=True)
        rust = _daemon(["--data-root", str(other), "daemon"], self.env, lines)
        self.assertEqual(rust[0], python[0])
        self.assertEqual(rust[1][1:], python[1][1:])

    def test_wallet_encrypted_daemon_requires_fresh_auth_for_each_mutation(self):
        import shutil
        self.import_run_book()
        self.ok(self.call(False, ["secrets", "init"], secret=self.PASSPHRASE, new_secret=True))
        other = self.home / "candidate" / "data"
        shutil.copytree(self.root, other)
        correct = {"auth_response": {"passphrase_secret": self.PASSPHRASE}}
        wrong = {"auth_response": {"passphrase_secret": "wrong synthetic passphrase"}}
        requests = [
            ("daemon.unlock", correct),
            ("ui.wallets.update", {"wallet": "Import source", "label": "Archive"}),
            ("ui.wallets.update", {"wallet": "Import source", "label": "Archive", **wrong}),
            ("ui.wallets.update", {"wallet": "Import source", "label": "Archive", **correct}),
            ("ui.wallets.update", {"wallet": "Archive", "label": "Archive 2"}),
            ("ui.wallets.delete", {"wallet": "Archive", "confirm": "DELETE", "confirm_wallet": "Archive", "cascade": True}),
            ("ui.wallets.delete", {"wallet": "Archive", "confirm": "DELETE", "confirm_wallet": "Archive", "cascade": True, **wrong}),
            ("ui.wallets.delete", {"wallet": "Archive", "confirm": "DELETE", "confirm_wallet": "Archive", "cascade": True, **correct}),
            ("daemon.shutdown", {}),
        ]
        lines = [json.dumps({"request_id": str(i), "kind": kind, "args": args}) + "\n"
                 for i, (kind, args) in enumerate(requests)]
        with self.reference():
            python = _daemon(["--data-root", str(self.root), "daemon"], self.env, lines, reference=True)
        # Restore the baseline at the same path so unlock status compares as
        # wire bytes, including its project/data-root fields.
        shutil.rmtree(self.root)
        shutil.copytree(other, self.root)
        rust = _daemon(["--data-root", str(self.root), "daemon"], self.env, lines)
        self.assertEqual(rust[0], python[0])
        self.assertEqual(rust[1][1:], python[1][1:])

    def test_import_run_cli_wire_bytes_match_reference(self):
        batch = self.import_run_book()
        for args in [
            ["imports", "list"], ["imports", "list", "--wallet", " import SOURCE "],
            ["imports", "list", "--wallet", ""], ["imports", "list", "--limit", "0"],
            ["imports", "list", "--limit", "999999999999999999999999999999999999999999"],
            ["imports", "list", "--wallet", "missing"],
            ["imports", "rollback", "--batch", batch],
            ["imports", "rollback", "--batch", batch, "--confirm", "--dry-run"],
            ["imports", "rollback", "--batch", "missing"],
        ]:
            with self.subTest(args=args):
                reference, candidate = (self.call(rust, args) for rust in (False, True))
                self.assertEqual((candidate.returncode, candidate.stdout, candidate.stderr),
                                 (reference.returncode, reference.stdout, reference.stderr))

    def _file_import_parity(self, command, scenarios, extra=(), fiat_currency=None, wallet_kind=None, seed=None, wallet_ref="Import source", fresh_ids=False, raw_key=False, distinct_id_steps=False):
        import sqlite3
        if self.scenario_slice is not None:
            part, count = self.scenario_slice
            scenarios = scenarios[part::count]
        from kassiber.secrets import sqlcipher
        from tests.test_rust_store_migrations import normalized_dump
        if "HOME" in os.environ:
            self.env["HOME"] = os.environ["HOME"]
        else:
            self.env.pop("HOME", None)
        self.import_run_book()
        if fiat_currency:
            self.ok(self.call(False, ["profiles", "set", "--fiat-currency", fiat_currency]))
        database = self.root / "kassiber.sqlite3"
        if wallet_kind or seed:
            with closing(sqlite3.connect(database)) as conn:
                if wallet_kind:
                    conn.execute("UPDATE wallets SET kind=? WHERE label='Import source'", (wallet_kind,))
                if seed:
                    seed(conn)
                conn.commit()
        file = self.home / "input.csv"
        fixed_time = "2025-01-02T03:04:05Z"
        identities = [f"00000000-0000-4000-8000-{i:012}" for i in range(1, 21)]
        self.env.update(KASSIBER_RUST_TEST_IMPORT_IDS=",".join(identities), KASSIBER_RUST_TEST_IMPORT_TIME=fixed_time)
        script = (
            "import sys; from pathlib import Path; from types import SimpleNamespace; "
            "from kassiber.operator import project; "
            "root=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:root; "
            "from kassiber.core import imports,import_batches,metadata,wallets,output_inventory; from kassiber.cli import handlers; "
            "ids=iter(__import__('os').environ['KASSIBER_RUST_TEST_IMPORT_IDS'].split(',')); wallets.uuid=imports.uuid=import_batches.uuid=metadata.uuid=SimpleNamespace(uuid4=lambda:next(ids)); "
            f"output_inventory.now_iso=wallets.now_iso=imports.now_iso=import_batches.now_iso=handlers.now_iso=lambda:{fixed_time!r}; "
            "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
        )

        for encrypted in (False, True):
            secret = ("x'" + "42" * 32 + "'" if raw_key else self.PASSPHRASE) if encrypted else None
            if encrypted:
                self.ok(self.call(False, ["secrets", "init"], secret=secret, new_secret=True, timeout=600))
            baseline = database.read_bytes()
            for scenario in scenarios:
                steps = scenario if isinstance(scenario, list) else [scenario]
                results, states = [], []
                for rust in (False, True):
                    database.write_bytes(baseline)
                    wires, dumps = [], []
                    batches = []
                    for step_index, step in enumerate(steps):
                        if distinct_id_steps:
                            self.env["KASSIBER_RUST_TEST_IMPORT_IDS"] = ",".join(f"00000000-0000-4000-8000-{step_index * 100 + i:012}" for i in range(1, 21))
                        elif fresh_ids:
                            self.env["KASSIBER_RUST_TEST_IMPORT_IDS"] = ",".join(f"00000000-0000-4000-8000-{step_index * 20 + i:012}" for i in range(1, 21))
                        content, edits = step if isinstance(step, tuple) else (step, [])
                        step_extra = extra
                        if isinstance(step, dict):
                            content, edits = step["content"], step.get("edits", [])
                            file = self.home / ("input" + step.get("extension", ".csv"))
                            step_extra = step.get("extra", extra)
                        if edits:
                            with closing(sqlcipher.open_encrypted(database, secret, enforce_operator_identity=False) if encrypted else sqlite3.connect(database)) as conn:
                                for sql in edits:
                                    conn.execute(sql)
                                conn.commit()
                        if content is None:
                            file.unlink(missing_ok=True)
                        else:
                            file.write_bytes(content.encode() if isinstance(content, str) else content)
                        args = ["wallets", command, *(["--wallet", wallet_ref] if wallet_ref else []), "--file", str(file), *step_extra]
                        if rust:
                            result = self.call(True, args, secret=secret)
                        else:
                            with tempfile.TemporaryFile() as fd, mock.patch.object(cli_runner, "prefix", return_value=(sys.executable, "-c", script, str(self.owners))):
                                auth = []
                                if secret is not None:
                                    fd.write(secret.encode()); fd.seek(0)
                                    auth = ["--db-passphrase-fd", str(fd.fileno())]
                                result = cli_runner.run(["--machine", *auth, "--data-root", str(self.root), *args], reference=True, cwd=ROOT, env=self.env, capture_output=True, timeout=TIMEOUT, pass_fds=(fd.fileno(),) if secret else ())
                        wires.append((result.returncode, result.stdout, result.stderr))
                        with closing(sqlcipher.open_encrypted(database, secret, enforce_operator_identity=False) if encrypted else sqlite3.connect(database)) as conn:
                            dumps.append(normalized_dump(conn))
                        if result.returncode == 0:
                            batch = json.loads(result.stdout)["data"].get("import_batch_id")
                            if batch:
                                batches.append(batch)
                    for batch in batches:
                        self.ok(self.call(rust, ["imports", "rollback", "--batch", batch, "--confirm"], secret=secret))
                    with closing(sqlcipher.open_encrypted(database, secret, enforce_operator_identity=False) if encrypted else sqlite3.connect(database)) as conn:
                        dumps.append(normalized_dump(conn))
                    results.append(wires)
                    states.append(dumps)
                with (self.subtest or self.subTest)(encrypted=encrypted, content=str(steps[0])[:100]):
                    self.assertEqual(results[1], results[0])
                    self.assertEqual(states[1], states[0])
            database.write_bytes(baseline)

    def test_csv_import_wire_bytes_full_state_dedupe_enrichment_and_rollback(self):
        import csv
        import io
        evidence = io.StringIO()
        writer = csv.writer(evidence)
        writer.writerow(["date", "txid", "amount", "privacy_boundary", "raw_json", "payment_hash", "payment_hash_source", "swap_refund_funding_txid", "swap_refund_funding_vout"])
        for boundary, source, vout in [("payjoin", "chain_script", ""), ("sweep", "chain_script_unique_outpoint", "0"), ("", "chain_script", "1")]:
            writer.writerow(["2025-01-01", "evidence", "1", boundary, json.dumps({"source": "wallet", "privacy_boundary": boundary}), "ab" * 32, source, "cd" * 32, vout])
        scenarios = [
            "date,txid,direction,asset,amount,fee,fiat_rate,description\r\n2025-01-02T00:00 Z,new,in,BTC,0.000000000015,0.000000000005,60000,\"receipt, \"\"one\"\"\r\nline\"\r\n2025-01-02,new,in,BTC,0.000000000015,0.000000000005,60000,receipt\r\n",
            "date,txid,direction,asset,amount,fee,description\n2025-01-02,enrich,in,BTC,1,0,\n2025-01-02,enrich,in,BTC,1,0,added\n",
            "date,txid,amount,fiat_rate,pricing_source_kind,kind,description,counterparty\n2025-01-01,fill,1,10,generic_import,deposit,quote,broker\n2025-01-01,fill,1,11,exchange_execution,buy,fill,exchange\n",
            evidence.getvalue(),
            "date,amount\n2025-01-02,1\ninvalid,2\n",
            "\ufeffdate,amount\n2025-01-02,1\n",
            "date,amount\n2025-01-02,1\n",
            "date,amount\n",
        ]
        self._file_import_parity("import-csv", scenarios)

    def test_json_import_wire_bytes_full_state_and_rollback(self):
        scenarios = [
            json.dumps([{"date":"2025-01-02T00:00:00+01:00", "txid":"json", "amount":1.5e-11, "fee":-5e-12, "fiat_rate":60000.0, "amount_includes_fee":False, "description":"receipt, \"one\"\r\nline"}]),
            json.dumps({"transactions":[{"date":"2025-01-02", "txid":"repeat", "amount":1}, {"date":"2025-01-03", "txid":"repeat", "amount":1, "description":"added"}]}),
            json.dumps([{"date":"2025-01-02", "txid":"noid", "id":1, "amount":0, "amount_includes_fee":False}]),
            json.dumps([{"date":"2025-01-02", "raw_json":{"_kassiber_provenance":{"import_source":"lnd"}, "islikelycoinjoin":True}, "amount":1}]),
            '[{"date":"2025-01-02","amount":1,"fiat_value":NaN,"raw_json":{"n":NaN,"privacy_boundary":"payjoin"}}]',
            '[{"date":"2025-01-02","amount":1},{"date":"invalid","amount":2}]',
            '[{"date":"2025-01-02","amount":1,"kind":[]}]',
            '[{"date":"2025-01-02","amount":1,"kind":[],"description":{},"counterparty":[1]}]',
            '[{"date":"2025-01-02","amount":true}]',
            '[{"date":"2025-01-02","amount":1,"confirmed_at":0}]',
            '[{"date":"2025-01-02","amount":1,"fiat_value":1,"fiat_currency":"USD"}]',
            json.dumps([{"date":"2025-01-01","amount":1}, {"date":"2025-01-02","amount":1}, {"date":"2025-01-02","amount":1}]),
            json.dumps([{"date":"2025-01-03","txid":"routing","amount":1,"kind":"routing_income","fiat_rate":10}, {"date":"2025-01-03","txid":"routing","amount":2,"fee":1e-11,"kind":"routing_income"}]),
            json.dumps([{"date":"2025-01-03","txid":"unpriced","amount":1,"kind":"routing_income","fiat_value":10}, {"date":"2025-01-03","txid":"unpriced","amount":2,"kind":"routing_income"}]),
            json.dumps([{"date":"2025-01-03","txid":"fee","amount":1}, {"date":"2025-01-03","txid":"fee","amount":1,"fee":0.01,"fiat_value":10,"pricing_source_kind":"exchange_execution"}]),
            json.dumps([{"date":"2025-01-03","txid":"ignore","amount":1}, {"date":"2025-01-03","txid":"ignore","amount":1,"fee":0.01,"_match_existing_ignore_fee":True}]),
            json.dumps([{"date":"2025-01-04","amount":1,"_tax_platform_row_identity":"row-identity"}, {"date":"2025-01-04","amount":1,"_tax_platform_row_identity":"row-identity","txid":"ab"*32}]),
            json.dumps([{"date":"2025-01-04","txid":"authored","amount":1,"description":"authored"}, {"date":"2025-01-04","txid":"authored","amount":1,"description":[1]}, {"date":"2025-01-04","txid":"authored","amount":1,"kind":[]}]),
            json.dumps([{"date":"2025-01-04","txid":"invalid-update","amount":1}, {"date":"2025-01-04","txid":"invalid-update","amount":1,"kind":[1]}]),
            '[{"date":"2025-01-02","txid":"opaque-repeat","amount":1},{"date":"2025-01-02","txid":"opaque-repeat","amount":1,"raw_json":"\\ud800"}]',
            '[]', '{}', '{"transactions":null}', '[null]', '\ufeff[]', '[\r\n invalid\r]',
        ]
        self._file_import_parity("import-json", scenarios)

    def test_river_import_wire_bytes_full_state_and_rollback(self):
        from tests.test_cli_smoke import _RIVER_CSV
        self._file_import_parity("import-river", [
            _RIVER_CSV,
            "Date,Sent Amount,Sent Currency,Received Amount,Received Currency,Reference Code,Tag\n2025-01-02,100,USD,0.002,BTC,id,Buy\n2025-01-02,100,USD,0.002,BTC,id,Buy\n",
            "Date,Sent Amount,Sent Currency,Received Amount,Received Currency\n2025-01-02,1,EUR,0.001,BTC\n",
            "bad,header\nx,y\n",
        ])

    def test_bull_import_wire_bytes_full_state_and_rollback(self):
        from tests.test_cli_smoke import _BULLBITCOIN_ORDERS_CSV
        self._file_import_parity("import-bull", [
            _BULLBITCOIN_ORDERS_CSV,
            "ORDER_STATUS,PAYIN_AMOUNT,PAYIN_CURRENCY,PAYOUT_AMOUNT,PAYOUT_CURRENCY,COMPLETED_AT (UTC),TRANSACTION_ID\ncompleted,100,USD,0.002,BTC,2025-01-02,repeat\ncompleted,100,USD,0.002,BTC,2025-01-02,repeat\n",
            "ORDER_STATUS,PAYIN_AMOUNT,PAYIN_CURRENCY,PAYOUT_AMOUNT,PAYOUT_CURRENCY,COMPLETED_AT (UTC),TRANSACTION_ID\ncompleted,100,EUR,0.002,BTC,2025-01-02,foreign\n",
            "bad,header\nx,y\n",
            "",
        ], extra=("--mode", "full"))

    def test_coinfinity_import_wire_bytes_full_state_and_rollback(self):
        from tests.test_cli_smoke import _COINFINITY_ORDERS_CSV, _COINFINITY_LN_ORDERS_CSV
        self._file_import_parity("import-coinfinity", [
            _COINFINITY_ORDERS_CSV, _COINFINITY_LN_ORDERS_CSV,
            "Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\norder,sell,2025-01-02,100,0.002,BTC,,0,1,\norder,sell,2025-01-02,100,0.002,BTC,,0,1,\n",
            "bad,header\nx,y\n", "",
        ], extra=("--mode", "full"), fiat_currency="EUR")

    def test_twentyone_import_wire_bytes_full_state_and_rollback(self):
        from tests.test_cli_smoke import _TWENTYONEBITCOIN_TRANSACTIONS_CSV
        self._file_import_parity("import-21bitcoin", [
            _TWENTYONEBITCOIN_TRANSACTIONS_CSV,
            "id,transaction_date,buy_asset,buy_amount,sell_asset,sell_amount,fee_asset,fee_amount,transaction_type\none,01.01.222,BTC,0.002,EUR,100,EUR,1,trade\none,01.01.222,BTC,0.002,EUR,100,EUR,1,trade\n",
            "bad,header\nx,y\n", "",
        ], fiat_currency="EUR")

    def test_pocket_import_wire_bytes_full_state_and_rollback(self):
        from tests.test_cli_smoke import _POCKETBITCOIN_CSV
        self._file_import_parity("import-pocket", [_POCKETBITCOIN_CSV,
            "bad,header\nx,y\n", "",
        ], extra=("--mode", "full"), fiat_currency="EUR")

    def test_strike_import_wire_bytes_full_state_and_rollback(self):
        from tests.test_cli_smoke import _STRIKE_CSV
        self._file_import_parity("import-strike", [_STRIKE_CSV,
            "bad,header\nx,y\n", "",
        ], fiat_currency="EUR")

    def test_ledgerlive_import_wire_bytes_full_state_and_rollback(self):
        from tests.test_cli_smoke import _LEDGERLIVE_CSV
        self._file_import_parity("import-ledger-live", [
            _LEDGERLIVE_CSV,
            "\ufeff" + _LEDGERLIVE_CSV,
            "Operation Date,Currency Ticker,Operation Type,Operation Amount,Operation Fees,Operation Hash,Account xpub\n"
            "2025-01-02T01:02:03+01:00,L-BTC,SENT,-0.002,0.000000000015,,xpub-secret\n",
            "Operation Date,Currency Ticker,Operation Type,Operation Amount,Operation Hash\n"
            "2025-01-02,BTC,IN,1,repeat\ninvalid,BTC,OUT,1,later\n",
            "bad,header\nx,y\n", "",
        ])

    def test_cointracking_import_wire_bytes_full_state_and_rollback(self):
        from tests.test_tax_platform_migration import COINTRACKING_CSV
        header = "Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Date,Tx-ID\n"
        self._file_import_parity("import-cointracking", [
            COINTRACKING_CSV, "\ufeff" + COINTRACKING_CSV,
            header + "Deposit,0.002,BTC,,,,,2025-01-02,\n" * 2,
            header + "Deposit,0.002,BTC,,,,,2025-01-02,first\nDeposit,1,BTC,,,,,invalid,later\n",
            "bad,header\nx,y\n", "",
        ], fiat_currency="EUR", wallet_kind="cointracking")

    def test_cointracking_coordinator_wire_bytes_full_state_and_rollback(self):
        txid = "ab" * 32
        def seed(conn):
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) SELECT 'observed',workspace_id,profile_id,'Observed','descriptor','2025-01-01' FROM wallets WHERE label='Import source'")
            for identity, external, stamp, amount in (("economic", "observed", "2025-01-02T00:00:00Z", 200000000), ("identity", txid, "2025-01-01T00:00:00Z", 1)):
                conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) SELECT ?,workspace_id,profile_id,id,?,?,?,'inbound','BTC',?,0,'2025-01-01' FROM wallets WHERE id='observed'", (identity, external, identity, stamp, amount))
        header = "Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Date,Tx-ID\n"
        self._file_import_parity("import-cointracking", [
            header + "Deposit,0.002,BTC,,,,,2025-01-02,econ\n"
            + f"Deposit,0.003,BTC,,,,,2025-01-03,{txid}\n"
            + "Deposit,0.004,BTC,,,,,2025-01-04,\n" * 2,
        ], fiat_currency="EUR", wallet_kind="cointracking", seed=seed)

    def _btcpay_reference_files(self, format):
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_btcpay_imports.json").read_text())
        self._file_import_parity("import-btcpay", [
            bytes(case["bytes"]) if "bytes" in case else case["text"] for case in cases if case["format"] == "btcpay_" + format
        ], extra=("--input-format", format))

    def btcpay_csv_reference_files(self):
        self._btcpay_reference_files("csv")

    def ledger_reference_files(self):
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_ledger_imports.json").read_text())
        self._file_import_parity("import-ledger", [
            {"content":bytes(case["bytes"]) if "bytes" in case else case["text"], "extension":case["extension"], "extra":["--column-map",json.dumps(case["column_map"])] if case["column_map"] is not None else []}
            for case in cases
        ], fiat_currency="EUR", raw_key=True)

    def ledger_preview_reference_files(self):
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_ledger_imports.json").read_text())
        self._file_import_parity("import-ledger", [
            {"content":bytes(case["bytes"]) if "bytes" in case else case["text"], "extension":case["extension"], "extra":["--dry-run","--limit",str(case["limit"]), *(["--column-map",json.dumps(case["column_map"])] if case["column_map"] is not None else [])]}
            for case in cases
        ], raw_key=True)

    def test_ledger_coordinator_wire_bytes_full_state_and_rollback(self):
        from crates.wallets.scripts.generate_ledger_vectors import inputs
        body=next(c["text"] for c in inputs() if c["name"]=="ledger:base")
        self._file_import_parity("import-ledger",[[body,body,body.replace("190","191"),body+"Buy,invalid,0.002,BTC,190,EUR,2,EUR,later,later-id\n",body]],fiat_currency="EUR",fresh_ids=True)

    def test_ledger_chartsheet_selection_import_and_preview_wire_bytes_and_full_state(self):
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_ledger_imports.json").read_text())
        selected = [case for case in cases if case["name"].startswith("ledger:xlsx-chartsheet-")]
        self.assertEqual(len(selected), 2)
        self._file_import_parity("import-ledger", [
            {"content": bytes(case["bytes"]), "extension": case["extension"], "extra": flags}
            for case in selected for flags in ([], ["--dry-run"])
        ], fiat_currency="EUR", raw_key=True)

    def test_ledger_column_map_order_wire_bytes_full_state_and_rollback(self):
        cases=json.loads((ROOT / "crates/wallets/tests/vectors/python_ledger_imports.json").read_text())
        self._file_import_parity("import-ledger",[
            {"content":case["text"],"extra":["--column-map",case["column_map_json"]]}
            for case in cases if case["name"].startswith("ledger:mapping-type-order-") or (case["column_map"] is not None and case.get("error", {}).get("details"))
        ], raw_key=True)

    def test_ledger_spreadsheet_integrity_import_and_preview_wire_bytes_and_full_state(self):
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_ledger_imports.json").read_text())
        selected = [case for case in cases if case["extension"] in {".xlsx", ".xlsm"}]
        self.assertEqual(len(selected), 35)
        self._file_import_parity("import-ledger", [
            {"content": bytes(case["bytes"]), "extension": case["extension"], "extra": flags}
            for case in selected for flags in ([], ["--dry-run"])
        ], fiat_currency="EUR", raw_key=True)

    def test_ledger_preview_bypasses_book_scope_and_parses_inline_and_file_maps(self):
        self.env["HOME"]=os.environ.get("HOME","")
        source=self.home / "ledger.csv"
        source.write_text("When,Units\n2025-01-02,0.002\n")
        mapping=self.home / "map.json"
        mapping.write_text('{"date":"When","amount":"Units"}')
        for value in [mapping.read_text(),"@"+str(mapping),"{", "[]", "@"+str(self.home / "absent.json")]:
            args=["wallets","import-ledger","--wallet","missing","--workspace","missing","--profile","missing","--file",str(source),"--dry-run","--column-map",value]
            python,rust=(self.call(native,args) for native in [False,True])
            self.assertEqual((rust.returncode,rust.stdout,rust.stderr),(python.returncode,python.stdout,python.stderr))
        self.assertFalse((self.root / "kassiber.sqlite3").exists())

    def btcpay_json_reference_files(self):
        self._btcpay_reference_files("json")

    def test_btcpay_surrogate_currency_wire_bytes_full_state_and_rollback(self):
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_btcpay_imports.json").read_text())
        selected = [case for case in cases if case["name"].startswith("btcpay:json:currency-surrogate-")]
        self.assertEqual(len(selected), 6)
        self._file_import_parity("import-btcpay", [case["text"] for case in selected], extra=("--input-format", "json"))

    def test_btcpay_coordinator_wire_bytes_full_state_and_rollback(self):
        self.maxDiff = None
        from crates.wallets.scripts.generate_btcpay_vectors import BASE
        body = json.dumps([BASE])
        changed = json.dumps([{**BASE, "Comment":"new comment", "Labels":{"new tag":{}, "old":{"text":"Shop"}}}])
        self._file_import_parity("import-btcpay", [[
            body, body,
            (changed, ["UPDATE transactions SET note='authored'"]),
            (body, ["UPDATE transactions SET note=NULL", "DELETE FROM transaction_tags"]),
            json.dumps([BASE, {**BASE, "Timestamp":"invalid"}]),
            (json.dumps([BASE, {**BASE, "TransactionId":["invalid"]}]), ["UPDATE transactions SET note=NULL", "DELETE FROM transaction_tags"]),
            body,
        ]], extra=("--input-format", "json"), fresh_ids=True)

    def binance_reference_files(self):
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_binance_imports.json").read_text())
        self._file_import_parity("import-binance-supplemental", [
            bytes(case["bytes"]) if "bytes" in case else case["text"] for case in cases
        ], fiat_currency="EUR")

    def test_binance_nan_payload_timestamp_wire_bytes_full_state_and_rollback(self):
        self._file_import_parity("import-binance-supplemental", [
            f"id,amount,asset,divTime\nreward,0.00001,BTC,{value}\n"
            for value in ("NaN123", "-NaN42", "NaN１２３", "+NaN0001")
        ], fiat_currency="EUR")

    def test_binance_default_wallet_wire_bytes_full_state_and_rollback(self):
        from crates.wallets.scripts.generate_binance_vectors import inputs
        self._file_import_parity("import-binance-supplemental", [[
            inputs()['autoinvest'], inputs()['autoinvest'], inputs()['partial-failure'],
        ]], fiat_currency="EUR", wallet_ref=None)

    def test_binance_coordinator_wire_bytes_full_state_and_rollback(self):
        from crates.wallets.scripts.generate_binance_vectors import inputs
        body = inputs()['autoinvest']
        reference = 'binance-supplemental:autoinvest:economic:21289c95049c68b07fe8e59a91647df8732abcd79ff602f39291ebe6b0f23f41:1'
        def seed(conn):
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) SELECT 'chain',workspace_id,profile_id,'Observed','descriptor','{}','2025-01-01' FROM wallets LIMIT 1")
            conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) SELECT 'observed',workspace_id,profile_id,id,?,'observed','2025-01-01T00:00:00Z','inbound','BTC',200000000,1,'2025-01-01' FROM wallets WHERE id='chain'", (reference,))
        self._file_import_parity("import-binance-supplemental", [[
            body, body, (body.replace('100.00 EUR', '200.00 EUR'), []),
            (body, ["UPDATE transactions SET external_id_kind='txid' WHERE wallet_id='chain'"]),
            inputs()['partial-failure'], (body, ["DELETE FROM transactions WHERE wallet_id='chain'"]), body,
        ]], fiat_currency="EUR", seed=seed, fresh_ids=True)

    def test_binance_relevant_coordinator_wire_bytes_full_state_and_rollback(self):
        from crates.wallets.scripts.generate_binance_vectors import inputs
        body = inputs()['autoinvest']
        reference = 'binance-supplemental:autoinvest:economic:21289c95049c68b07fe8e59a91647df8732abcd79ff602f39291ebe6b0f23f41:1'
        def seed(conn):
            conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) SELECT 'observed',workspace_id,profile_id,id,?,'observed','2025-01-01T00:00:00Z','inbound','BTC',200000000,1,'2025-01-01' FROM wallets LIMIT 1", (reference,))
        self._file_import_parity("import-binance-supplemental", [[
            body, body, body.replace('100.00 EUR', '200.00 EUR'), inputs()['partial-failure'],
            (body, ["UPDATE profiles SET fiat_currency='USD'"]),
        ]], extra=("--mode", "relevant"), fiat_currency="EUR", seed=seed)

    def _evidence_relevant_wire_order(self, command, vector_name):
        cases = json.loads((ROOT / f"crates/wallets/tests/vectors/python_{vector_name}_imports.json").read_text())
        case = next(case for case in cases if case.get("records") and case["records"][0]["fields"].get("fiat_currency") == "EUR")
        fields = case["records"][0]["fields"]
        def seed(conn):
            conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) SELECT 'observed',workspace_id,profile_id,id,?,'observed',?,?,?,?,?,'2025-01-01' FROM wallets LIMIT 1", (fields['external_id'], fields['occurred_at'], fields['direction'], fields['asset'], fields['amount'], fields['fee']))
        self._file_import_parity(command, [[case['text'], case['text']]], extra=("--mode", "relevant"), fiat_currency="EUR", seed=seed)

    def test_bull_relevant_wire_order_and_full_state(self):
        self._evidence_relevant_wire_order("import-bull", "bull")

    def test_coinfinity_relevant_wire_order_and_full_state(self):
        self._evidence_relevant_wire_order("import-coinfinity", "coinfinity")

    def test_pocket_relevant_wire_order_and_full_state(self):
        self._evidence_relevant_wire_order("import-pocket", "pocket")

    def blockpit_reference_files(self):
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_blockpit_imports.json").read_text())
        self._file_import_parity("import-blockpit", [
            bytes(case["bytes"]) if "bytes" in case else case["text"] for case in cases
        ], fiat_currency="EUR", wallet_kind="blockpit")

    def test_blockpit_coordinator_wire_bytes_full_state_and_rollback(self):
        txid = "ab" * 32
        def seed(conn):
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) SELECT 'observed',workspace_id,profile_id,'Observed','descriptor','2025-01-01' FROM wallets WHERE label='Import source'")
            for identity, external, stamp, amount in (("economic", "observed", "2025-01-02T00:00:00Z", 200000000), ("identity", txid, "2025-01-01T00:00:00Z", 1)):
                conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) SELECT ?,workspace_id,profile_id,id,?,?,?,'inbound','BTC',?,0,'2025-01-01' FROM wallets WHERE id='observed'", (identity, external, identity, stamp, amount))
        header = "Date (UTC),Integration Name,Label,Outgoing Asset,Outgoing Amount,Incoming Asset,Incoming Amount,Fee Asset,Fee Amount,Comments,Trx. ID\n"
        body = (header + "2025-01-02,Source,Deposit,,,BTC,0.002,,,one,econ\n"
                + f"2025-01-03,Source,Deposit,,,BTC,0.003,,,two,{txid}\n"
                + "2025-01-04,Source,Deposit,,,BTC,0.004,,,three,\n"
                + "2025-01-04,Source,Deposit,,,BTC,0.004,,,four,\n")
        promoted = body.replace("three,\n", f"three,{txid}\n")
        self._file_import_parity("import-blockpit", [[
            body, body,
            (body, ["DELETE FROM transactions WHERE wallet_id='observed'",
                    "UPDATE transactions SET description=NULL WHERE external_id IS NOT NULL"]),
            promoted, promoted.replace("2025-01-02,", "invalid,", 1),
        ]], fiat_currency="EUR", wallet_kind="blockpit", seed=seed)

    def test_phoenix_import_wire_bytes_full_state_and_rollback(self):
        from tests.test_cli_smoke import _PHOENIX_CSV
        scenarios = [
            _PHOENIX_CSV,
            "date,id,type,amount_msat,description\n2025-01-01,zero,legacy_pay_to_open,0, note \n",
            "date,id,type,amount_msat\n2025-01-01,id,swap_in,1\n2025-01-01,id,swap_in,1\n",
            "date,id,type,amount_msat\n2025-01-01,id,swap_in,no\n",
            "bad,header\nx,y\n",
            "date,id,type,amount_msat\n",
        ]
        self._file_import_parity("import-phoenix", scenarios)

    def test_import_run_rollback_matches_complete_state_in_both_codecs(self):
        import sqlite3
        from kassiber.secrets import sqlcipher
        from tests.test_rust_store_migrations import normalized_dump
        batch = self.import_run_book()
        database = self.root / "kassiber.sqlite3"
        with closing(sqlite3.connect(database)) as conn:
            conn.execute("PRAGMA journal_mode=DELETE")
        for encrypted in (False, True):
            with self.subTest(encrypted=encrypted):
                secret = self.PASSPHRASE if encrypted else None
                if encrypted:
                    self.ok(self.call(False, ["secrets", "init"], secret=secret, new_secret=True))
                baseline = database.read_bytes()
                results, states = [], []
                for rust in (False, True):
                    database.write_bytes(baseline)
                    result = self.call(rust, ["imports", "rollback", "--batch", batch, "--confirm"], secret=secret)
                    self.ok(result)
                    results.append((result.returncode, result.stdout, result.stderr))
                    conn = (sqlcipher.open_encrypted(database, secret, enforce_operator_identity=False)
                            if encrypted else sqlite3.connect(database))
                    try:
                        states.append(normalized_dump(conn))
                        self.assertEqual(conn.execute("PRAGMA foreign_key_check").fetchall(), [])
                        self.assertEqual(conn.execute("PRAGMA integrity_check").fetchone()[0], "ok")
                    finally:
                        conn.close()
                self.assertEqual(results[0], results[1])
                self.assertEqual(states[0], states[1])
                database.write_bytes(baseline)

    def test_import_run_daemon_wire_bytes_confirmation_and_scope(self):
        import queue
        import threading
        batch = self.import_run_book()
        database = self.root / "kassiber.sqlite3"
        import sqlite3
        with closing(sqlite3.connect(database)) as conn:
            conn.execute("PRAGMA journal_mode=DELETE")
        baseline = database.read_bytes()
        requests = [
            ("ui.imports.list", {}), ("ui.imports.list", {"wallet": " Import SOURCE "}),
            *[("ui.imports.list", {"limit": limit}) for limit in [0, "0", 2.9, " ２_０ "]],
            ("ui.imports.list", {"wallet": "missing", "limit": "x"}),
            ("ui.imports.list", {"wallet": 1}),
            ("ui.imports.rollback", {"batch": batch, "dry_run": True}),
            ("ui.imports.rollback", {"batch": batch}),
            ("ui.imports.rollback", {"batch": batch, "confirm": True}),
            ("ui.imports.rollback", {"batch": batch, "expected_scope": {"profile_id": "other"}, "confirm": "DELETE"}),
            ("ui.imports.rollback", {"batch": batch, "confirm": "DELETE"}),
        ]
        outputs = []
        for rust in (False, True):
            database.write_bytes(baseline)
            records = []
            with self.daemon(rust) as daemon:
                # The reference writes caught-error tracebacks to stderr. Drain
                # them so a sequence of invalid requests cannot fill its pipe.
                errors = []
                stderr_reader = threading.Thread(target=lambda: errors.append(daemon.stderr.read()), daemon=True)
                stderr_reader.start()
                for index, (kind, args) in enumerate(requests):
                    daemon.stdin.write((json.dumps({"request_id": index, "kind": kind, "args": args}) + "\n").encode())
                    daemon.stdin.flush()
                    received = queue.Queue()
                    threading.Thread(target=lambda: received.put(daemon.stdout.readline()), daemon=True).start()
                    raw = received.get(timeout=TIMEOUT)
                    self.assertTrue(raw, (rust, index, daemon.poll(), records[-2:], errors))
                    records.append(raw)
                daemon.wait(timeout=TIMEOUT)
                stderr_reader.join(timeout=TIMEOUT)
                self.assertEqual(daemon.returncode, 0)
                self.assertEqual(daemon.stdout.read(), b"")
            # A confirmed rollback deliberately closes the reference session.
            # Restart, then prove a second rollback is refused and list is empty.
            with self.daemon(rust) as daemon:
                for kind, args in [("ui.imports.rollback", {"batch": batch, "confirm": "DELETE"}), ("ui.imports.list", {})]:
                    daemon.stdin.write((json.dumps({"request_id": "after", "kind": kind, "args": args}) + "\n").encode())
                    daemon.stdin.flush()
                    received = queue.Queue()
                    threading.Thread(target=lambda: received.put(daemon.stdout.readline()), daemon=True).start()
                    records.append(received.get(timeout=TIMEOUT))
                self.assertEqual(self.request(daemon, "daemon.shutdown")["kind"], "daemon.shutdown")
            outputs.append(records)
        for index, (reference, candidate) in enumerate(zip(*outputs)):
            self.assertEqual(candidate, reference, f"record {index}")

    def test_legacy_project_migration_refuses_held_owner_and_pending_journal(self):
        legacy = self.home / "xdg-data" / "kassiber" / "data"
        self.ok(self.call(False, ["init"], root=legacy))
        with self.daemon(False, root=legacy) as owner:
            self.assertEqual(self.request(owner, "status")["kind"], "status")
            response = self.call(True, ["status"], default=True)
            self.assertEqual(json.loads(response.stdout)["error"]["code"], "state_root_migration_in_use")
            self.assertTrue((legacy / "kassiber.sqlite3").exists())
        sidecar = legacy / "kassiber.sqlite3-wal"
        sidecar.write_bytes(b"synthetic pending WAL")
        response = self.call(True, ["status"], default=True)
        self.assertEqual(json.loads(response.stdout)["error"]["code"], "state_root_migration_in_use")
        self.assertTrue((legacy / "kassiber.sqlite3").exists())
        sidecar.unlink()
        self.same_status(default=True)



    # Python resolves data and app-data roots, so links above them (macOS
    # /var -> /private/var, a linked home) are the user's layout. Each case
    # runs Python, then Rust, from the same saved state at the same paths.
    # SQLCipher key derivation and fsyncs get a longer limit on shared disks.
    SLOW = 600

    def slow(self, rust, args, **options):
        return self.call(rust, args, timeout=self.SLOW, **options)

    def aliased(self, base):
        real = base / "private-var"
        if real.exists():
            shutil.rmtree(real)
        real.mkdir()
        alias = base / "var"
        if not alias.is_symlink():
            alias.symlink_to("private-var")
        return alias, real

    def saved(self, real, build):
        """Build `real` once, then restore that exact tree for every run."""
        saved = Path(tempfile.mkdtemp(dir=self._dir.name)) / "tree"
        def reset():
            if saved.exists():
                shutil.rmtree(real)
                shutil.copytree(saved, real, symlinks=True)
            else:
                build()
                shutil.copytree(real, saved, symlinks=True)
        return reset

    def storage_state(self, root, secret):
        """Every path and link, JSON text and masked database dumps.

        Modes are not compared: Rust creates missing directories and locks
        owner-only where Python follows the umask (rust-store.md).
        """
        import re
        import sqlite3
        from kassiber.secrets import sqlcipher
        from tests.test_rust_store_migrations import normalized_dump
        # Clocks also name migration reports and backups; mask them everywhere.
        clock = re.compile(r"\d{4}-?\d{2}-?\d{2}T\d{2}:?\d{2}:?\d{2}(?:\.\d+)?Z?")
        def mask(dump):
            objects, tables = dump
            masked = {}
            for table, (columns, rows) in tables.items():
                # The database identity and creation clocks are fresh per book.
                masked[table] = (columns, [tuple(
                    (kind, "<clock>" if isinstance(value, str) and clock.match(value)
                     else "<identity>" if table == "settings" and row[0][1] == "database_instance_id" and index == 1
                     else value)
                    for index, (kind, value) in enumerate(row)) for row in rows])
            return objects, masked
        state = []
        for path in root.rglob("*"):
            entry = [clock.sub("<clock>", path.relative_to(root).as_posix())]
            if path.is_symlink():
                entry.append(("link", os.readlink(path)))
            elif path.is_dir():
                entry.append("directory")
            elif path.suffix == ".json":
                entry.append(clock.sub("<clock>", path.read_text(encoding="utf-8")))
            elif path.name == "kassiber.sqlite3":
                conn = (sqlcipher.open_encrypted(path, secret, enforce_operator_identity=False)
                        if secret else sqlite3.connect(path))
                try:
                    entry.append(mask(normalized_dump(conn)))
                finally:
                    conn.close()
            state.append(entry)
        return sorted(state, key=lambda entry: entry[0])

    def both_from(self, reset, steps, root, secret=None):
        records = []
        for rust in (False, True):
            reset()
            results = [step(rust) for step in steps]
            records.append(([(r.returncode, r.stdout, r.stderr) for r in results],
                            self.storage_state(root, secret)))
        self.maxDiff = 4000
        self.assertEqual(records[1][0], records[0][0])
        self.assertEqual(records[1][1], records[0][1])
        return records[0][0]

    def test_data_root_under_symlinked_parent_matches_python(self):
        for secret in (None, self.PASSPHRASE):
            with self.subTest(encrypted=secret is not None):
                alias, real = self.aliased(self.home)
                root = alias / "folders" / "book" / "data"
                # A keyed `init` creates the SQLCipher book directly; the
                # plaintext migration is pinned by the store's unit tests.
                steps = [lambda rust: self.slow(rust, ["init"], root=root, secret=secret)]
                steps += [lambda rust: self.slow(rust, ["status"], root=root, secret=secret)] * 2
                results = self.both_from(lambda: self.aliased(self.home), steps, real, secret)
                self.assertEqual({code for code, _, _ in results}, {0})
                self.assertIn(str(root).encode(), results[-1][1])

    def test_symlinked_storage_files_refuse_like_python(self):
        for secret in (None, self.PASSPHRASE):
            with self.subTest(encrypted=secret is not None):
                alias, real = self.aliased(self.home)
                root = alias / "book" / "data"
                def book():
                    self.aliased(self.home)
                    self.ok(self.slow(False, ["init"], root=root))
                    if secret:
                        self.ok(self.slow(False, ["secrets", "init"], root=root, secret=secret, new_secret=True))
                reset = self.saved(real, book)
                # A link swapped in for the restore journal is refused by both.
                def journal():
                    reset()
                    (real / "book" / ".restore-pending.json").symlink_to("elsewhere.json")
                status = lambda rust: self.slow(rust, ["status"], root=root, secret=secret)
                [(code, stdout, _)] = self.both_from(journal, [status], real, secret)
                error = json.loads(stdout)["error"]
                self.assertEqual((code, error["code"], error["details"]), (1, "unsafe_restore_target", None))
                # Kept stricter than Python: Python creates the book through a
                # link at the database path and follows a linked settings file.
                self.aliased(self.home)
                (real / "book" / "data").mkdir(parents=True)
                (real / "book" / "data" / "kassiber.sqlite3").symlink_to("elsewhere.sqlite3")
                create = ["secrets", "init"] if secret else ["init"]
                refused = self.slow(True, create, root=root, secret=secret, new_secret=bool(secret))
                self.assertEqual(json.loads(refused.stdout)["error"]["code"], "unsafe_configuration")
                self.assertFalse((real / "book" / "data" / "elsewhere.sqlite3").exists())
                reset()
                settings = real / "book" / "config" / "settings.json"
                settings.rename(settings.with_name("real.json"))
                settings.symlink_to("real.json")
                self.ok(self.slow(False, ["status"], root=root, secret=secret))
                refused = self.slow(True, ["status"], root=root, secret=secret)
                self.assertEqual(json.loads(refused.stdout)["error"]["message"],
                                 "configuration is unreadable or unsafe")

    def migrate(self, rust):
        with self.reference():
            if rust:
                return subprocess.run(_rust(["--migrate-default-state-root"]), cwd=ROOT, env=self.env,
                                      capture_output=True, timeout=self.SLOW)
            return cli_runner.run(["--migrate-default-state-root"], reference=True, cwd=ROOT,
                                  env=self.env, capture_output=True, timeout=self.SLOW)

    def test_symlinked_legacy_state_root_stays_like_python(self):
        for secret in (None, self.PASSPHRASE):
            target = self.home / "target"
            shutil.rmtree(self.home)
            self.home.mkdir()
            self.ok(self.slow(False, ["init"], root=target / "data"))
            if secret:
                self.ok(self.slow(False, ["secrets", "init"], root=target / "data",
                                  secret=secret, new_secret=True))
            for inner in (False, True):
                with self.subTest(encrypted=secret is not None, inner=inner):
                    def legacy():
                        if inner:
                            (self.home / ".kassiber").mkdir()
                            (self.home / ".kassiber" / "data").symlink_to(target / "data")
                        else:
                            (self.home / ".kassiber").symlink_to("target")
                    reset = self.saved(self.home, legacy)
                    [result] = self.both_from(reset, [self.migrate], self.home, secret)
                    self.assertEqual(result, (0, b"", b""))
                    self.assertFalse((self.home / "xdg-data" / "kassiber").exists())
                    shutil.rmtree(self.home / ".kassiber") if inner else (self.home / ".kassiber").unlink()

    def test_home_and_app_data_root_under_symlinked_parents_match_python(self):
        base = Path(self._dir.name)
        alias, real = self.aliased(base)
        home = alias / "home"
        self.env = self.env | {key: value for key, value in _environment(home).items()
                               if key in {"HOME", "USERPROFILE", "LOCALAPPDATA"} or key.startswith("XDG_")}
        for secret in (None, self.PASSPHRASE):
            with self.subTest(encrypted=secret is not None):
                # The hidden-home cutover renames across the linked parent.
                def legacy():
                    self.aliased(base)
                    home.mkdir()
                    self.ok(self.slow(False, ["init"], root=home / ".kassiber" / "data"))
                    if secret:
                        self.ok(self.slow(False, ["secrets", "init"], root=home / ".kassiber" / "data",
                                          secret=secret, new_secret=True))
                status = lambda rust: self.slow(rust, ["status"], default=True, secret=secret)
                results = self.both_from(self.saved(real, legacy), [self.migrate, status, status], real, secret)
                self.assertEqual([code for code, _, _ in results], [0, 0, 0])
                self.assertTrue((real / "home/xdg-data/kassiber/projects/default/data/kassiber.sqlite3").is_file())
                # A fresh default project in the app-data root (XDG_DATA_HOME).
                def fresh():
                    self.aliased(base)
                    home.mkdir()
                steps = [lambda rust: self.slow(rust, ["init"], default=True, secret=secret), status]
                results = self.both_from(fresh, steps, real, secret)
                self.assertEqual({code for code, _, _ in results}, {0})
                self.assertIn(str(home / "xdg-data/kassiber/projects/default").encode(), results[-1][1])

    def test_historical_pair_free_fixture_matches_python_full_dump(self):
        from tests.test_rust_store_migrations import (
            HISTORICAL_FIXTURES, LEGACY_PAIR_FEE_FIXTURES, LEGACY_PAIR_FIXTURES, IDENTITY,
            TIMESTAMP, decode_historical, normalized_dump, reference_open,
        )
        import sqlite3
        self.env["KASSIBER_RUST_TEST_MIGRATION_IDENTITY"] = IDENTITY
        self.env["KASSIBER_RUST_TEST_MIGRATION_TIMESTAMP"] = TIMESTAMP
        # Rust leaves 8f's legacy fee pair to Python's review; that refusal is
        # test_ordered_historical_legacy_pair_fee_is_left_for_python.
        for fixture in (path for path in HISTORICAL_FIXTURES if path not in LEGACY_PAIR_FEE_FIXTURES):
            with self.subTest(fixture=fixture.name):
                python_root = self.home / fixture.stem / "reference" / "data"
                rust_root = self.home / fixture.stem / "candidate" / "data"
                for root in (python_root, rust_root):
                    decode_historical(
                        root, fixture, False,
                        without_legacy_pair=fixture in LEGACY_PAIR_FIXTURES,
                    )
                conn = reference_open(python_root)
                expected = normalized_dump(conn)
                conn.close()
                self.ok(self.call(True, ["status"], root=rust_root))
                conn = sqlite3.connect(rust_root / "kassiber.sqlite3")
                self.assertEqual(normalized_dump(conn), expected)
                self.assertEqual(conn.execute("PRAGMA integrity_check").fetchone()[0], "ok")
                self.assertEqual(conn.execute("PRAGMA foreign_key_check").fetchall(), [])
                conn.close()
                self.same_status(rust_root)
                conn = reference_open(rust_root)
                self.assertEqual(normalized_dump(conn), expected)
                conn.close()

    def test_historical_authored_consolidation_refuses_with_python_hint(self):
        from tests.test_rust_store_migrations import LEGACY_PAIR_FIXTURES, decode_historical
        for fixture in LEGACY_PAIR_FIXTURES:
            with self.subTest(fixture=fixture.name):
                root = self.home / fixture.stem / "data"
                decode_historical(root, fixture, False)
                response = self.call(True, ["status"], root=root)
                self.assertEqual(response.returncode, 1)
                error = json.loads(response.stdout)["error"]
                self.assertEqual(error["code"], "unsupported_schema_layout")
                self.assertIn("Python", error["hint"])
                self.assertEqual(error["details"]["reason"], "legacy_authored_custody")

    def wasabi_reference_files(self):
        cases = json.loads((ROOT / "crates/wallets/tests/vectors/python_wasabi_imports.json").read_text())
        self._file_import_parity("import-wasabi", [bytes(case["bytes"]) if "bytes" in case else case["text"] for case in cases])


    def test_wasabi_coordinator_wire_bytes_full_state_and_rollback(self):
        from crates.wallets.scripts.generate_wasabi_vectors import HISTORY, COIN, inputs
        def body(history, **sections):
            return json.dumps({"gethistory": history, **sections})
        self._file_import_parity("import-wasabi", [[
            inputs()["full"], inputs()["full"],
            (body([{**HISTORY, "islikelycoinjoin": True}], listcoins=[COIN]), ["UPDATE transactions SET note='authored',review_status='completed',fee=10"]),
            body([HISTORY], listcoins=[]),
            body([HISTORY]),
            body([{**HISTORY, "tx": "cd"*32}]),
        ], [
            body([{**HISTORY, "tx": None}]), body([HISTORY]), body([HISTORY]),
        ], [
            body([HISTORY]), body([HISTORY], listcoins=[{**COIN, "txid": "bad"}]),
        ], [
            *[case['text'] for case in json.loads((ROOT / 'crates/wallets/tests/vectors/python_wasabi_codepoints.json').read_text())],
            inputs()["metadata-large-float"],
            *[inputs()[name] for name in ("coin-vout-refusal-order", "coin-amount-refusal-order", "account-score-refusal-order", "account-gap-refusal-order")],
        ], None], distinct_id_steps=True)



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
        self.check("crates/setup/scripts/export_vectors.py")

    def test_file_import_vectors_match_python(self):
        self.check("crates/wallets/scripts/generate_import_vectors.py")
        self.check("crates/wallets/scripts/generate_exchange_vectors.py")
        self.check("crates/wallets/scripts/generate_bull_vectors.py")
        self.check("crates/wallets/scripts/generate_coinfinity_vectors.py")
        self.check("crates/wallets/scripts/generate_twentyone_vectors.py")
        self.check("crates/wallets/scripts/generate_strike_vectors.py")
        self.check("crates/wallets/scripts/generate_pocket_vectors.py")
        self.check("crates/wallets/scripts/generate_ledgerlive_vectors.py")
        self.check("crates/wallets/scripts/generate_cointracking_vectors.py")
        self.check("crates/wallets/scripts/generate_blockpit_vectors.py")
        self.check("crates/wallets/scripts/generate_binance_vectors.py")

    def test_btcpay_vectors_match_python_reference(self):
        self.check("crates/wallets/scripts/generate_btcpay_vectors.py")
        result = subprocess.run([sys.executable, "-m", "tests.rust_port.repro_btcpay_reference"], cwd=ROOT, capture_output=True, text=True, timeout=TIMEOUT)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_ledger_vectors_match_python_reference(self):
        self.check("crates/wallets/scripts/generate_ledger_vectors.py")
        result = subprocess.run([sys.executable, "-m", "tests.rust_port.repro_ledger_reference"], cwd=ROOT, capture_output=True, text=True, timeout=TIMEOUT)
        self.assertEqual(result.returncode, 0, result.stderr)


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

    def test_wasabi_import_vectors_match_reference(self):
        self.check("crates/wallets/scripts/generate_wasabi_vectors.py")


# The reference-file corpora replay hundreds of independent files, each from
# the same baseline book. They run in slices so xdist spreads them; each slice
# builds that book exactly as the unittest method did and keeps per-file
# subtests.
REFERENCE_SLICES = 4
reference_slices = pytest.mark.parametrize("part", range(REFERENCE_SLICES), ids=lambda part: f"slice{part:02}")


@pytest.fixture
def real_store(subtests):
    if not RUST_BIN:
        pytest.skip(RUST_SKIP)
    test = RustRealStoreParityTest()
    test.setUp()
    test.subtest = subtests.test
    try:
        yield test
    finally:
        test.tearDown()
        test.doCleanups()


@reference_slices
def test_btcpay_csv_reference_files_wire_bytes_full_state_and_rollback(real_store, part):
    real_store.scenario_slice = (part, REFERENCE_SLICES)
    real_store.btcpay_csv_reference_files()


@reference_slices
def test_ledger_reference_files_wire_bytes_full_state_and_rollback(real_store, part):
    real_store.scenario_slice = (part, REFERENCE_SLICES)
    real_store.ledger_reference_files()


@reference_slices
def test_ledger_preview_reference_files_wire_bytes_and_no_mutations(real_store, part):
    real_store.scenario_slice = (part, REFERENCE_SLICES)
    real_store.ledger_preview_reference_files()


@reference_slices
def test_btcpay_json_reference_files_wire_bytes_full_state_and_rollback(real_store, part):
    real_store.scenario_slice = (part, REFERENCE_SLICES)
    real_store.btcpay_json_reference_files()


@reference_slices
def test_binance_reference_files_wire_bytes_full_state_and_rollback(real_store, part):
    real_store.scenario_slice = (part, REFERENCE_SLICES)
    real_store.binance_reference_files()


@reference_slices
def test_blockpit_reference_files_wire_bytes_full_state_and_rollback(real_store, part):
    real_store.scenario_slice = (part, REFERENCE_SLICES)
    real_store.blockpit_reference_files()


@reference_slices
def test_wasabi_reference_files_wire_bytes_full_state_and_rollback(real_store, part):
    real_store.scenario_slice = (part, REFERENCE_SLICES)
    real_store.wasabi_reference_files()


if __name__ == "__main__":
    unittest.main()
