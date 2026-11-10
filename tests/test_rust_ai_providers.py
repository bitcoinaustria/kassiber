"""R6-6a parity: AI provider kinds and leaves, discovery and connection tests.

Each scenario runs the Python reference (`tests/rust_port/ai_providers_reference.py`,
with the clocks pinned) and the Rust binary (`KASSIBER_RUST_BIN`, a
test-harness build reading the same pinned clocks) on separate copies of one
disposable book. Daemon sessions must answer byte for byte; the provider
tables must end identical; requests to the scripted loopback provider must
carry identical bodies. CLI scenarios compare stdout, stderr and exit codes,
and the hybrid candidate's log must show Rust served every `ai` leaf.

Both sides find the same fake provider broker through
`KASSIBER_AI_BROKER_NODE` and `KASSIBER_AI_PROVIDER_BROKER` (R6-6b); it logs
every request line it reads, and the logs must match byte for byte. The
Rust side's armed observer admits that one script
(`KASSIBER_RUST_TEST_AI_BROKER_FIXTURE`).

Children get their own XDG roots and preference files; HOME stays the
account's. Only the loopback scripted provider is contacted.
"""

from __future__ import annotations

import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from contextlib import closing
from pathlib import Path

from kassiber.ai.providers import ai_provider_secret_service_id
from kassiber.secrets.migration import migrate_plaintext_to_encrypted
from kassiber.db import open_db
from tests.test_ai_core_http_transport import (
    Reply,
    ScriptedProvider,
    chat_stream,
    chunk,
    error_reply,
    message_item,
    responses_stream,
)
from tests.test_rust_shell_parity import _environment

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
TIME = "2026-11-05T12:00:00Z"
CHECKED_AT = "2026-11-05T12:00:00.123456Z"
# The last checks (#675) are kept in the book and compared too.
TABLES = ("ai_providers", "ai_provider_secret_refs", "ai_provider_checks", "settings")
# Documented divergence (docs/reference/rust-shell.md, CLI proxy providers):
# Rust also takes the Rust-only kind `cli_proxy`, so its refusals of an
# unknown kind list it. Python's lines are compared with it added.
KIND_LIST = (b"one of: local, remote, tee", b"one of: local, remote, tee, cli_proxy")


# Answers status and model requests from fixed data, logging each request
# line and where its per-turn root lives. A result line holding a raw U+2028
# comes first: Python's splitlines() breaks it, so both sides skip it.
FAKE_BROKER = r"""
import json, os, sys
LOG = sys.argv[0] + ".log"
line = sys.stdin.readline()
root = os.environ["KASSIBER_AI_BROKER_TMPDIR"]
with open(LOG, "a", encoding="utf-8") as log:
    log.write(line.rstrip("\n") + "\n")
    log.write(json.dumps({"root_parent": os.path.dirname(root), "root_mode": oct(os.stat(root).st_mode & 0o777), "argv": sys.argv[1:]}) + "\n")
request = json.loads(line)
provider = request.get("provider")
names = request.get("providers") or ([provider] if provider else ["claude", "codex", "opencode", "copilot"])
if request["command"] == "status":
    if provider == "claude":
        print(json.dumps({"type": "error", "code": "authentication_required", "hint": "Run claude login."}), flush=True)
        raise SystemExit(0)
    print(json.dumps({"type": "result", "data": [{"provider": "codex", "note": "a\u2028b"}]}, ensure_ascii=False), flush=True)
    print("not json", flush=True)
    rows = [{"provider": name, "state": "ready", "version": "1.0", "latency": 0.000125, "big": 10**20, "label": "Caf\u00e9 \u2603"} for name in names]
    rows.append({"provider": "intruder", "state": "ready"})
    print(json.dumps({"type": "result", "data": rows}), flush=True)
elif request["command"] == "models":
    if provider == "claude":
        print(json.dumps({"type": "error", "message": "claude models failed"}), flush=True)
    elif provider == "copilot":
        print(json.dumps({"type": "result", "data": [{"id": "copilot", "check_kind": "binary_presence", "label": "GitHub Copilot"}]}), flush=True)
    else:
        print(json.dumps({"type": "result", "data": [{"id": "gpt-5.4", "label": "GPT-5.4", "reasoning_efforts": ["low", "high"], "fast_mode": True}, {"id": "gpt-5.4-mini", "context": 1.5e5}]}), flush=True)
"""


def _ok_chat():
    return chat_stream(chunk({"content": "OK"}), chunk({}, finish="stop"))


@unittest.skipUnless(RUST_BIN and os.name == "posix", "set KASSIBER_RUST_BIN to a test-harness build")
class RustAiProvidersParityTest(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-r6-ai-providers-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / "data"
        with closing(open_db(str(self.root))):
            pass
        self.env = _environment(self.base / "preferences")
        self.env["HOME"] = os.environ["HOME"]
        self.env.update(
            {
                "KASSIBER_STATE_ROOT": str(self.base / "state"),
                "KASSIBER_RUST_TEST_AI_TIME": TIME,
                "KASSIBER_RUST_TEST_AI_CHECKED_AT": CHECKED_AT,
                # AI keeps Python's separate online gate: the product switch
                # leaves it alone, and the observer admits only loopback.
                "KASSIBER_NO_EGRESS": "1",
                "KASSIBER_TEST_NO_EGRESS": "1",
            }
        )
        self.provider = ScriptedProvider()
        self.addCleanup(self.provider.stop)
        self.log = self.base / "served.jsonl"
        self.broker = self.base / "broker" / "fake_broker.py"
        self.broker.parent.mkdir()
        self.broker.write_text(FAKE_BROKER, encoding="utf-8")
        self.env.update(
            {
                "KASSIBER_AI_BROKER_NODE": sys.executable,
                "KASSIBER_AI_PROVIDER_BROKER": str(self.broker),
                "KASSIBER_RUST_TEST_AI_BROKER_FIXTURE": str(self.broker),
            }
        )

    # --- running both sides ---------------------------------------------

    def command(self, runtime):
        return {
            "python": [sys.executable, "-m", "tests.rust_port.ai_providers_reference"],
            "rust": [str(Path(RUST_BIN).resolve())],
            "hybrid": [sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py")],
        }[runtime]

    def run_side(self, runtime, root, args, *, input=b""):
        env = dict(self.env)
        if runtime == "hybrid":
            env["KASSIBER_HYBRID_RUST_BIN"] = str(Path(RUST_BIN).resolve())
            env["KASSIBER_HYBRID_LOG"] = str(self.log)
            env["KASSIBER_TEST_CLI"] = ""
        return subprocess.run(
            [*self.command(runtime), "--machine", "--data-root", str(root), *args],
            input=input,
            capture_output=True,
            env=env,
            cwd=ROOT,
            timeout=300,
        )

    def tables(self, root):
        # A secret ref's service is bound to its data root, so each copy has
        # its own: compare it as "this root's service".
        own = ai_provider_secret_service_id(str(root.resolve()))
        if getattr(self, "passphrase", None):
            conn = open_db(str(root), passphrase=self.passphrase, require_existing_schema=True)
        else:
            conn = sqlite3.connect(root / "kassiber.sqlite3")
        try:
            tables = {
                table: sorted((tuple(row) for row in conn.execute(f'SELECT * FROM "{table}"')), key=repr)
                for table in TABLES
            }
        finally:
            conn.close()
        tables["ai_provider_secret_refs"] = [
            (row[0], row[1], "<own root>" if row[2] == own else row[2], *row[3:])
            for row in tables["ai_provider_secret_refs"]
        ]
        return tables

    def copy(self, name):
        target = self.base / name / "data"
        shutil.copytree(self.root, target)
        return target

    def broker_requests(self):
        log = Path(str(self.broker) + ".log")
        return log.read_bytes() if log.exists() else b""

    def script(self, setup):
        """Reset the scripted provider and the broker log, and queue a
        scenario's replies."""
        Path(str(self.broker) + ".log").unlink(missing_ok=True)
        with self.provider.lock:
            self.provider.routes.clear()
            self.provider.requests.clear()
        if setup:
            setup(self.provider)

    def requests_seen(self):
        return [
            (request["method"], request["path"], request["body"], request["authorization"])
            for request in self.provider.requests
        ]

    def daemon(self, requests, *, setup=None):
        """One session on each side; returns the Rust response lines."""
        wire = "".join(
            json.dumps({"request_id": str(index), **request}) + "\n"
            for index, request in enumerate(
                [*requests, {"kind": "daemon.shutdown"}]
            )
        ).encode()
        outcomes = []
        for runtime in ("python", "rust"):
            root = self.copy(f"{runtime}-{len(list(self.base.glob(runtime + '-*')))}")
            self.script(setup)
            result = self.run_side(runtime, root, ["daemon"], input=wire)
            self.assertEqual((result.returncode, result.stderr), (0, b""), result.stdout[-2000:])
            # Status payloads name each side's own copy of the book.
            lines = [
                line.replace(str(root.parent).encode(), b"<copy>")
                for line in result.stdout.splitlines()[1:]
            ]
            if runtime == "python":
                lines = [line.replace(*KIND_LIST) for line in lines]
            outcomes.append((lines, self.tables(root), self.requests_seen(), self.broker_requests()))
        (python_lines, python_tables, python_hits, python_broker), (rust_lines, rust_tables, rust_hits, rust_broker) = outcomes
        self.assertEqual(len(rust_lines), len(python_lines))
        mismatches = [
            (request, got, wanted)
            for request, got, wanted in zip([*requests, {"kind": "daemon.shutdown"}], rust_lines, python_lines)
            if got != wanted
        ]
        self.assertEqual(mismatches, [])
        self.assertEqual(rust_tables, python_tables)
        self.assertEqual(rust_hits, python_hits)
        self.assertEqual(rust_broker, python_broker)
        self.broker_log = rust_broker
        return [json.loads(line) for line in rust_lines]

    def cli(self, commands, *, setup=None):
        """Each command on Python, Rust and the hybrid candidate, on copies."""
        outcomes = []
        for runtime in ("python", "rust", "hybrid"):
            root = self.copy(f"cli-{runtime}-{len(list(self.base.glob('cli-' + runtime + '-*')))}")
            self.script(setup)
            results = []
            for args, stdin in commands:
                result = self.run_side(runtime, root, args, input=stdin)
                results.append((result.returncode, result.stdout, result.stderr))
            outcomes.append((results, self.tables(root), self.requests_seen(), self.broker_requests()))
        python, rust, hybrid = outcomes
        mismatches = [
            (command[0], got, wanted)
            for command, got, wanted in zip(commands, rust[0], python[0])
            if got != wanted
        ]
        self.assertEqual(mismatches, [])
        self.assertEqual(rust[1:], python[1:])
        self.assertEqual(hybrid, rust)
        served = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(served), len(commands))
        self.assertTrue(all(row["served_by"] == "rust" for row in served), served)
        self.broker_log = rust[3]
        return rust[0]

    # --- scenarios --------------------------------------------------------

    def test_daemon_provider_crud_contacts_nothing(self):
        responses = self.daemon(
            [
                {"kind": "ai.providers.list"},
                {"kind": "ai.providers.list", "args": [1]},
                {"kind": "ai.providers.create", "args": {"name": "Router", "base_url": "https://openrouter.ai/api/v1/"}},
                {"kind": "ai.providers.create", "args": {"name": "router", "base_url": "https://x.test/v1"}},
                {"kind": "ai.providers.create", "args": {"name": "k", "base_url": "https://x.test/v1", "api_key": "sk-1"}},
                {"kind": "ai.providers.create", "args": {"name": "k", "base_url": "https://x.test/v1", "kind": "local"}},
                {"kind": "ai.providers.create", "args": {"name": "k", "base_url": "https://x.test/v1", "kind": 5}},
                {"kind": "ai.providers.create", "args": {"name": "lan", "base_url": "http://127.0.0.1:9/v1", "display_name": "LAN", "enabled": False, "api_format": "chat-completions", "detected_api_format": "responses", "omit_store_field": True, "default_model": "m", "notes": " n "}},
                {"kind": "ai.providers.create", "args": {"name": "bad", "base_url": "codex-cli://default", "omit_store_field": True}},
                # Agent access (#678): full unless the user restricts it.
                {"kind": "ai.providers.create", "args": {"name": "narrow", "base_url": "https://n.test/v1", "agent_access": "restricted"}},
                {"kind": "ai.providers.create", "args": {"name": "x", "base_url": "https://x.test/v1", "agent_access": 5}},
                {"kind": "ai.providers.create", "args": {"name": "x", "base_url": "https://x.test/v1", "agent_access": "all"}},
                {"kind": "ai.providers.update", "args": {"name": "narrow", "agent_access": "nope"}},
                {"kind": "ai.providers.update", "args": {"name": "narrow", "agent_access": "full"}},
                {"kind": "ai.providers.update", "args": {"name": "router", "default_model": "openai/gpt", "default_reasoning_effort": "HIGH", "api_format": "responses"}},
                {"kind": "ai.providers.update", "args": {"name": "router"}},
                {"kind": "ai.providers.update", "args": {"name": "router", "clear": ["kind", "notes"]}},
                {"kind": "ai.providers.update", "args": {"name": "router", "clear": "notes"}},
                {"kind": "ai.providers.update", "args": {"name": "router", "enabled": "no"}},
                {"kind": "ai.providers.update", "args": {"name": "router", "base_url": "https://other.test/v1", "acknowledged": True}},
                {"kind": "ai.providers.update", "args": {"name": "lan", "base_url": "https://llm.test/v1"}},
                {"kind": "ai.providers.get", "args": {"name": "LAN"}},
                {"kind": "ai.providers.get", "args": {"name": "missing"}},
                {"kind": "ai.providers.get", "args": {}},
                {"kind": "ai.providers.set_api_key", "args": {"name": "router", "api_key": "sk-inline-secret"}},
                {"kind": "ai.providers.set_api_key", "args": {"name": "router", "api_key": "sk-x", "store_id": "macos_keychain"}},
                {"kind": "ai.providers.set_api_key", "args": {"name": "router", "api_key": "sk-x", "store_id": "vault"}},
                {"kind": "ai.providers.move_api_key", "args": {"name": "router", "store_id": "sqlcipher_inline"}},
                {"kind": "ai.providers.move_api_key", "args": {"name": "router", "store_id": " "}},
                {"kind": "ai.providers.move_api_key", "args": {"name": "router", "store_id": "windows_dpapi"}},
                {"kind": "ai.providers.set_api_key", "args": {"name": "router", "api_key": None}},
                {"kind": "ai.providers.acknowledge", "args": {"name": "omlx"}},
                {"kind": "ai.providers.set_default", "args": {"name": "lan"}},
                {"kind": "ai.providers.update", "args": {"name": "lan", "enabled": True, "clear": ["default_model"]}},
                {"kind": "ai.providers.set_default", "args": {"name": "lan"}},
                {"kind": "ai.providers.delete", "args": {"name": "lan"}},
                {"kind": "ai.providers.update", "args": {"name": "lan", "enabled": False}},
                {"kind": "ai.providers.clear_default"},
                {"kind": "ai.providers.delete", "args": {"name": "lan"}},
                {"kind": "ai.providers.delete", "args": {"name": "lan"}},
                {"kind": "ai.provider_runtime.status", "args": {"cached_only": True}},
                {"kind": "ai.provider_runtime.status"},
                {"kind": "ai.provider_runtime.status", "args": {"provider": "codex", "cached_only": True}},
                {"kind": "ai.provider_runtime.status", "args": {"provider": "ollama", "cached_only": True}},
                {"kind": "ai.provider_runtime.status", "args": {"refresh": True, "cached_only": True}},
                {"kind": "ai.provider_runtime.status", "args": {"base_url": "codex-cli://default"}},
                {"kind": "ai.provider_runtime.status", "args": {"provider": "codex", "base_url": "claude-cli://default"}},
                {"kind": "ai.provider_runtime.status", "args": {"provider": "codex"}},
                {"kind": "ai.providers.list"},
            ]
        )
        self.assertEqual(self.provider.requests, [])
        # Listing, editing and display reads start no broker either.
        self.assertEqual(self.broker_log, b"")
        listed = responses[-2]["data"]
        self.assertNotIn("sk-inline-secret", json.dumps(responses))
        self.assertIsNone(listed["default"])

    def test_daemon_model_listing_uses_the_cache_and_its_gates(self):
        local = self.provider.base_url

        def setup(provider):
            provider.queue("GET", "models", Reply(json_body={"object": "list", "data": [
                {"id": "a", "owned_by": "x", "supported_parameters": ["tools", "reasoning"]},
                {"id": "b", "architecture": {"input_modalities": ["Text", "image"]}},
            ]}))
            provider.queue("GET", "models", Reply(500, json_body={"error": "down"}))

        self.daemon(
            [
                {"kind": "ai.providers.create", "args": {"name": "lan", "base_url": local}},
                {"kind": "ai.providers.create", "args": {"name": "far", "base_url": "https://far.invalid/v1"}},
                # A display read before any check: nothing, and no contact.
                {"kind": "ai.list_models", "args": {"provider": "lan", "cached_only": True}},
                {"kind": "ai.list_models", "args": {"provider": "lan"}},
                {"kind": "ai.list_models", "args": {"provider": "lan"}},
                {"kind": "ai.list_models", "args": {"provider": "lan", "cached_only": True}},
                {"kind": "ai.list_models", "args": {"provider": "lan", "cached_only": True, "refresh": True}},
                {"kind": "ai.providers.list"},
                {"kind": "ai.list_models", "args": {"provider": "lan", "refresh": True}},
                # The failed refresh saved the stale result; a preference
                # edit keeps it, a new address drops it (#675).
                {"kind": "ai.providers.update", "args": {"name": "lan", "agent_access": "restricted"}},
                {"kind": "ai.list_models", "args": {"provider": "lan", "cached_only": True}},
                {"kind": "ai.providers.update", "args": {"name": "far", "notes": "n"}},
                {"kind": "ai.providers.list"},
                {"kind": "ai.list_models", "args": {"provider": "lan", "base_url": "http://127.0.0.1:1/v1"}},
                {"kind": "ai.list_models", "args": {"provider": "lan", "refresh": "yes"}},
                {"kind": "ai.list_models", "args": {"provider": "far"}},
                {"kind": "ai.list_models", "args": {"provider": 3}},
                {"kind": "ai.providers.update", "args": {"name": "lan", "enabled": False}},
                {"kind": "ai.list_models", "args": {"provider": "lan"}},
            ],
            setup=setup,
        )
        self.assertEqual([r["path"] for r in self.provider.requests], ["/v1/models", "/v1/models"])

    def test_encrypted_books_keep_checks_and_agent_access(self):
        # The same provider rows and saved checks on a SQLCipher book.
        passphrase = "correct horse battery staple"
        migrate_plaintext_to_encrypted(self.root / "kassiber.sqlite3", passphrase)
        self.passphrase = passphrase
        local = self.provider.base_url

        def setup(provider):
            provider.queue("GET", "models", Reply(json_body={"data": [{"id": "a"}]}))

        self.daemon(
            [
                {"kind": "daemon.unlock", "args": {"auth_response": {"passphrase_secret": passphrase}}},
                {"kind": "ai.providers.create", "args": {"name": "lan", "base_url": local, "agent_access": "restricted"}},
                {"kind": "ai.list_models", "args": {"provider": "lan", "cached_only": True}},
                {"kind": "ai.list_models", "args": {"provider": "lan"}},
                {"kind": "ai.providers.update", "args": {"name": "lan", "agent_access": "full"}},
                {"kind": "ai.list_models", "args": {"provider": "lan", "cached_only": True}},
                {"kind": "ai.providers.list"},
            ],
            setup=setup,
        )
        self.assertEqual([r["path"] for r in self.provider.requests], ["/v1/models"])

    def test_daemon_connection_tests_detect_and_record_the_format(self):
        local = self.provider.base_url

        def setup(provider):
            for _ in range(3):
                provider.queue("GET", "models", Reply(json_body={"data": [{"id": "only"}]}))
            provider.queue(
                "POST",
                "responses",
                Reply(404, json_body={"detail": "Not Found"}),
                Reply(sse_events=[{"type": "response.created"}, {"type": "response.output_text.delta", "delta": "O"}], done=False),
            )
            provider.queue(
                "POST",
                "chat/completions",
                error_reply(400, "llama3 does not support tools"),
                _ok_chat(),
                _ok_chat(),
            )

        responses = self.daemon(
            [
                {"kind": "ai.providers.create", "args": {"name": "lan", "base_url": local}},
                {"kind": "ai.test_connection", "args": {"base_url": local, "provider": "lan"}},
                {"kind": "ai.providers.get", "args": {"name": "lan"}},
                {"kind": "ai.test_connection", "args": {"base_url": local, "model": "only", "api_format": "responses", "kind": "local"}},
                {"kind": "ai.test_connection", "args": {"base_url": "https://far.invalid/v1"}},
                {"kind": "ai.test_connection", "args": {"base_url": "https://far.invalid/v1", "acknowledged": "yes"}},
                {"kind": "ai.test_connection", "args": {"base_url": " "}},
                {"kind": "ai.test_connection", "args": {"base_url": local, "kind": "cloud"}},
                {"kind": "ai.providers.create", "args": {"name": "k", "base_url": local, "kind": "cloud"}},
                {"kind": "ai.test_connection", "args": {"base_url": "codex-cli://default", "api_format": "responses", "acknowledged": True}},
                {"kind": "ai.test_connection", "args": {"base_url": "codex-cli://default", "omit_store_field": True, "acknowledged": True}},
                {"kind": "ai.test_connection", "args": {"base_url": local, "api_format": "chat-completions"}},
            ],
            setup=setup,
        )
        # The documented divergence, pinned on the Rust side.
        self.assertEqual(
            responses[7]["error"]["message"],
            "ai.test_connection kind must be one of: local, remote, tee, cli_proxy",
        )
        self.assertEqual(responses[8]["error"]["hint"], "Choose one of: local, remote, tee, cli_proxy")
        self.assertEqual(responses[1]["data"]["detected_api_format"], "chat_completions")
        self.assertTrue(responses[1]["data"]["detection_saved"])
        self.assertFalse(responses[1]["data"]["tools_accepted"])

    def test_daemon_offline_mode_keeps_only_a_local_loopback_provider(self):
        offline = Path(self.env["KASSIBER_OFFLINE_PREFERENCE_FILE"])
        offline.parent.mkdir(parents=True, exist_ok=True)
        offline.write_text('{"schema_version":1,"enabled":true}')
        offline.chmod(0o600)
        local = self.provider.base_url

        def setup(provider):
            provider.queue("GET", "models", Reply(json_body={"data": [{"id": "m"}]}))

        self.daemon(
            [
                {"kind": "ai.providers.create", "args": {"name": "lan", "base_url": local}},
                {"kind": "ai.providers.create", "args": {"name": "gateway", "base_url": local.replace("127.0.0.1", "localhost"), "kind": "remote", "acknowledged": True}},
                {"kind": "ai.list_models", "args": {"provider": "gateway"}},
                {"kind": "ai.list_models", "args": {"provider": "lan"}},
            ],
            setup=setup,
        )
        self.assertEqual([r["path"] for r in self.provider.requests], ["/v1/models"])

    def _contactable_cli_rows(self):
        return [
            request
            for name in ("codex", "claude", "copilot")
            for request in (
                {"kind": "ai.providers.update", "args": {"name": name, "enabled": True}},
                {"kind": "ai.providers.acknowledge", "args": {"name": name}},
            )
        ]

    def test_daemon_broker_probes_listings_and_connection_tests(self):
        responses = self.daemon(
            [
                {"kind": "ai.provider_runtime.status"},
                *self._contactable_cli_rows(),
                {"kind": "ai.provider_runtime.status", "args": {"cached_only": True}},
                {"kind": "ai.provider_runtime.status"},
                {"kind": "ai.provider_runtime.status"},
                {"kind": "ai.provider_runtime.status", "args": {"cached_only": True}},
                {"kind": "ai.provider_runtime.status", "args": {"refresh": True}},
                {"kind": "ai.provider_runtime.status", "args": {"provider": "codex"}},
                {"kind": "ai.provider_runtime.status", "args": {"provider": "codex", "base_url": "codex-cli://default", "refresh": True}},
                {"kind": "ai.provider_runtime.status", "args": {"provider": "claude"}},
                {"kind": "ai.provider_runtime.status", "args": {"provider": "claude", "cached_only": True}},
                {"kind": "ai.list_models", "args": {"provider": "codex"}},
                {"kind": "ai.list_models", "args": {"provider": "codex"}},
                {"kind": "ai.list_models", "args": {"provider": "copilot", "refresh": True}},
                {"kind": "ai.list_models", "args": {"provider": "claude"}},
                {"kind": "ai.test_connection", "args": {"base_url": "codex-cli://default", "provider": "codex"}},
                {"kind": "ai.test_connection", "args": {"base_url": "copilot-cli://default", "acknowledged": True}},
                {"kind": "ai.test_connection", "args": {"base_url": " Claude-CLI://default ", "provider": "claude", "model": "opus"}},
                {"kind": "ai.providers.update", "args": {"name": "codex", "enabled": False}},
                {"kind": "ai.list_models", "args": {"provider": "codex"}},
                {"kind": "ai.provider_runtime.status"},
            ]
        )
        self.assertEqual(responses[0]["data"]["providers"], [])
        probed = responses[8]["data"]
        self.assertEqual([row["provider"] for row in probed["providers"]], ["claude", "codex", "copilot"])
        self.assertEqual(responses[21]["data"]["check_kind"], "binary_presence")
        self.assertEqual(self.provider.requests, [])
        requests = [json.loads(line) for line in self.broker_log.splitlines()[::2]]
        self.assertEqual(
            [request["command"] for request in requests],
            ["status", "status", "status", "status", "status", "models", "models", "models", "models", "models", "models", "status"],
        )

    def test_daemon_offline_mode_refuses_the_broker(self):
        offline = Path(self.env["KASSIBER_OFFLINE_PREFERENCE_FILE"])
        offline.parent.mkdir(parents=True, exist_ok=True)
        offline.write_text('{"schema_version":1,"enabled":true}')
        offline.chmod(0o600)
        self.daemon(
            [
                *self._contactable_cli_rows(),
                {"kind": "ai.provider_runtime.status"},
                {"kind": "ai.provider_runtime.status", "args": {"provider": "codex"}},
                {"kind": "ai.provider_runtime.status", "args": {"cached_only": True}},
                {"kind": "ai.list_models", "args": {"provider": "codex"}},
                {"kind": "ai.test_connection", "args": {"base_url": "codex-cli://default", "provider": "codex"}},
            ]
        )
        self.assertEqual(self.broker_log, b"")

    def test_cli_broker_leaves_match_and_are_served_by_rust(self):
        enable = [
            (["ai", "providers", "update", name, "--enable", "--acknowledge"], b"")
            for name in ("codex", "claude", "copilot")
        ]
        results = self.cli(
            [
                (["ai", "models", "--provider", "codex"], b""),
                *enable,
                (["ai", "models", "--provider", "codex"], b""),
                (["ai", "models", "--provider", "claude"], b""),
                (["ai", "providers", "test", "codex"], b""),
                (["ai", "providers", "test", "copilot", "--model", "x"], b""),
                (["ai", "providers", "test", "claude"], b""),
            ]
        )
        self.assertNotEqual(results[0][0], 0)
        self.assertEqual(results[4][0], 0)
        self.assertEqual(len(self.broker_log.splitlines()), 10)

    def test_cli_leaves_match_and_are_served_by_rust(self):
        local = self.provider.base_url

        def setup(provider):
            provider.queue("GET", "models", Reply(json_body={"data": [{"id": "m"}, {"id": "n"}]}))
            provider.queue("GET", "models", Reply(json_body={"data": [{"id": "m"}]}))
            provider.queue("POST", "responses", responses_stream([message_item("OK")], text="OK"))

        results = self.cli(
            [
                (["ai", "providers", "list"], b""),
                (["ai", "providers", "create", "Lan", "--base-url", local, "--api-key-stdin", "--default-effort", "high"], b"sk-cli-secret\n"),
                (["ai", "providers", "create", "far", "--base-url", "https://far.invalid/v1", "--api-key", "sk-argv", "--api-format", "chat-completions", "--disabled", "--agent-access", "restricted"], b""),
                (["ai", "providers", "get", "lan"], b""),
                (["ai", "providers", "update", "far", "--enable", "--clear", "api_key", "--notes", "n", "--omit-store-field"], b""),
                (["ai", "providers", "update", "far"], b""),
                (["ai", "providers", "update", "far", "--agent-access", "full"], b""),
                (["ai", "providers", "set-default", "lan"], b""),
                (["ai", "providers", "delete", "lan"], b""),
                (["ai", "models", "--provider", "far"], b""),
                (["ai", "models"], b""),
                (["ai", "providers", "test", "lan"], b""),
                (["ai", "providers", "test", "lan", "--model", "m"], b""),
                (["ai", "providers", "clear-default"], b""),
                (["ai", "providers", "delete", "far"], b""),
                (["ai", "providers", "test", "codex"], b""),
            ],
            setup=setup,
        )
        self.assertEqual(results[1][0], 0)
        self.assertNotIn(b"sk-cli-secret", b"".join(out + err for _, out, err in results))
        self.assertIn(b"warning: --api-key <value> exposes secrets", results[2][2])


if __name__ == "__main__":
    unittest.main()
