"""R6-7 parity: whole `ai.chat` streams, consent, cancellation and history.

Each scenario runs one daemon session on the Python reference
(`tests/rust_port/ai_chat_reference.py`, clocks pinned) and one on the Rust
test-harness binary (`KASSIBER_RUST_BIN`, reading the same pinned clocks), on
separate copies of one disposable book, against the same scripted loopback
provider and fake provider broker. Requests are sent one at a time and the
driver waits for each request's terminal record, answering consent prompts
and sending cancels as the scenario says.

Compared byte for byte, after mapping each side's uuid4 values (chat session
and message ids) to labels in order of appearance:

- every response line, grouped by request id in arrival order (a control
  response and the chat it controls may interleave either way, on both
  sides);
- the chat history tables, the provider tables and the settings;
- every request the scripted provider received (method, path, body and
  authorization) and every line the fake broker read.

Children get their own XDG roots and preference files; HOME stays the
account's. Only the loopback scripted provider is contacted.
"""

from __future__ import annotations

import json
import os
import re
import select
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

from kassiber.db import open_db
from kassiber.secrets.migration import migrate_plaintext_to_encrypted
from tests.test_ai_core_http_transport import (
    Reply,
    ScriptedProvider,
    chat_stream,
    chunk,
    message_item,
    responses_stream,
)
from tests.test_rust_shell_parity import _environment

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
TIME = "2026-11-05T12:00:00Z"
CHECKED_AT = "2026-11-05T12:00:00.123456Z"
UUID = re.compile(rb"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")
TABLES = ("ai_chat_sessions", "ai_chat_messages", "ai_providers", "settings")
WAIT = 60.0

# A CLI agent run through the broker: one tool round, then an answer, and a
# provider session cursor. It logs every request line it reads.
FAKE_BROKER = r"""
import json, sys
LOG = sys.argv[0] + ".log"

def read():
    line = sys.stdin.readline()
    with open(LOG, "a", encoding="utf-8") as log:
        log.write(line)
    return json.loads(line)

def say(event):
    print(json.dumps(event), flush=True)

request = read()
if request.get("command") != "chat":
    say({"type": "result", "data": []})
    raise SystemExit(0)
if request.get("tools"):
    say({"type": "status", "phase": "fast_mode_unavailable"})
    say({"type": "delta", "content": "Checking ", "reasoning": "plan"})
    say({"type": "tool_call", "call_id": "broker-call-1", "name": "status", "arguments": {}})
    reply = read()
    say({"type": "delta", "content": "all good"})
    say({"type": "done", "finish_reason": "stop", "provider_session_id": "native-cursor-1"})
else:
    say({"type": "delta", "content": "plain answer"})
    say({"type": "done", "provider_session_id": "native-cursor-2"})
"""


def _function_call(call_id: str, name: str, arguments: str = "{}") -> dict:
    return {
        "type": "function_call",
        "id": f"fc_{call_id}",
        "call_id": call_id,
        "name": name,
        "arguments": arguments,
    }


def _answer(text: str) -> Reply:
    return responses_stream([message_item(text)], text=text)


@unittest.skipUnless(RUST_BIN and os.name == "posix", "set KASSIBER_RUST_BIN to a test-harness build")
class RustAiChatParityTest(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-r6-ai-chat-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / "data"
        self.env = _environment(self.base / "preferences")
        self.env["HOME"] = os.environ["HOME"]
        self.env.update(
            {
                "KASSIBER_STATE_ROOT": str(self.base / "state"),
                "KASSIBER_RUST_TEST_AI_TIME": TIME,
                "KASSIBER_RUST_TEST_AI_CHECKED_AT": CHECKED_AT,
                # AI keeps its own online gate; the observer admits loopback.
                "KASSIBER_NO_EGRESS": "1",
                "KASSIBER_TEST_NO_EGRESS": "1",
            }
        )
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
        self.provider = ScriptedProvider()
        self.addCleanup(self.provider.stop)
        self.passphrase = None
        for args in (["init"], ["workspaces", "create", "Demo"], ["profiles", "create", "Main"]):
            result = subprocess.run(
                [*self.command("python"), "--machine", "--data-root", str(self.root), *args],
                capture_output=True,
                env=self.env,
                cwd=ROOT,
                timeout=120,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    # --- running both sides ---------------------------------------------

    def command(self, runtime):
        return {
            "python": [sys.executable, "-m", "tests.rust_port.ai_chat_reference"],
            "rust": [str(Path(RUST_BIN).resolve())],
        }[runtime]

    def script(self, setup):
        Path(str(self.broker) + ".log").unlink(missing_ok=True)
        with self.provider.lock:
            self.provider.routes.clear()
            self.provider.requests.clear()
        if setup:
            setup(self.provider)

    def tables(self, root, labels):
        if self.passphrase:
            conn = open_db(str(root), passphrase=self.passphrase, require_existing_schema=True)
        else:
            conn = sqlite3.connect(root / "kassiber.sqlite3")
        try:
            raw = {
                table: [tuple(row) for row in conn.execute(f'SELECT * FROM "{table}"').fetchall()]
                for table in TABLES
            }
        finally:
            conn.close()
        # Message ids never reach an envelope: compare messages without them.
        raw["ai_chat_messages"] = [row[1:] for row in raw["ai_chat_messages"]]
        return {
            table: sorted(
                (
                    tuple(
                        _label(value.encode(), labels, grow=False).decode()
                        if isinstance(value, str)
                        else value
                        for value in row
                    )
                    for row in rows
                ),
                key=repr,
            )
            for table, rows in raw.items()
        }

    def run_session(self, runtime, root, steps, *, before=None):
        process = subprocess.Popen(
            [*self.command(runtime), "--data-root", str(root), "daemon"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=self.env,
            cwd=ROOT,
        )
        lines: list[bytes] = []
        buffer = b""

        def read_line(deadline):
            nonlocal buffer
            while b"\n" not in buffer:
                remaining = deadline - time.monotonic()
                self.assertGreater(remaining, 0, f"{runtime}: timed out; got {lines[-3:]}")
                ready, _, _ = select.select([process.stdout], [], [], remaining)
                if not ready:
                    continue
                data = os.read(process.stdout.fileno(), 65536)
                if not data:
                    self.fail(f"{runtime}: daemon exited; stderr={process.stderr.read()[-2000:]!r}")
                buffer += data
            line, buffer = buffer.split(b"\n", 1)
            return line

        def send(request):
            process.stdin.write(json.dumps(request).encode() + b"\n")
            process.stdin.flush()

        deadline = time.monotonic() + WAIT
        self.assertEqual(json.loads(read_line(deadline))["kind"], "daemon.ready")
        seen: dict = {}
        try:
            for index, step in enumerate([*steps, {"request": {"kind": "daemon.shutdown"}}]):
                request = step["request"](seen) if callable(step["request"]) else step["request"]
                rid = f"r{index}"
                send({"request_id": rid, **request})
                waiting = {rid}
                streaming = request["kind"] == "ai.chat"
                consent = list(step.get("consent", []))
                cancel_on = step.get("cancel_on")
                deadline = time.monotonic() + WAIT
                extra = 0
                while waiting:
                    line = read_line(deadline)
                    lines.append(line)
                    record = json.loads(line)
                    record_id = record.get("request_id")
                    kind = record.get("kind")
                    if record_id == rid and streaming:
                        if kind in {"ai.chat", "error", "auth_required"}:
                            waiting.discard(rid)
                            if kind == "ai.chat":
                                seen["session_id"] = record["data"]["session_id"]
                        elif kind == "ai.chat.tool_consent_required" and consent:
                            decision = consent.pop(0)
                            if isinstance(decision, list):
                                # Requests sent while the chat waits, in order.
                                for request in decision:
                                    extra += 1
                                    control = f"{rid}c{extra}"
                                    send({"request_id": control, **json.loads(json.dumps(request).replace("{rid}", rid))})
                                    waiting.add(control)
                                continue
                            extra += 1
                            control = f"{rid}c{extra}"
                            if decision == "cancel":
                                send({"request_id": control, "kind": "ai.chat.cancel", "args": {"target_request_id": rid}})
                            else:
                                send({"request_id": control, "kind": "ai.tool_call.consent", "args": {"target_request_id": rid, "call_id": record["data"]["call_id"], "decision": decision}})
                            waiting.add(control)
                        elif cancel_on and kind == cancel_on:
                            cancel_on = None
                            extra += 1
                            control = f"{rid}c{extra}"
                            send({"request_id": control, "kind": "ai.chat.cancel", "args": {"target_request_id": rid}})
                            waiting.add(control)
                    elif record_id in waiting:
                        waiting.discard(record_id)
            process.stdin.close()
            code = process.wait(timeout=WAIT)
            stderr = process.stderr.read()
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)
            process.stdout.close()
            process.stderr.close()
        self.assertEqual((code, stderr), (0, b""), f"{runtime} exited badly")
        return lines

    def session(self, steps, *, setup=None, prepare=None):
        """One session per side; returns the Rust records."""
        outcomes = []
        for runtime in ("python", "rust"):
            # One path for both sides: status payloads name it.
            root = self.base / "side" / "data"
            shutil.rmtree(root.parent, ignore_errors=True)
            shutil.copytree(self.root, root)
            if prepare:
                prepare(root)
            self.script(setup)
            lines = self.run_session(runtime, root, steps)
            grouped: dict[str, list[bytes]] = {}
            for line in lines:
                grouped.setdefault(json.loads(line).get("request_id"), []).append(line)
            labels: dict[bytes, bytes] = {}
            ordered = sorted(grouped.items(), key=lambda item: _rid_order(item[0]))
            mapped = {rid: [_label(line, labels) for line in group] for rid, group in ordered}
            requests = [
                (request["method"], request["path"], request["body"], request["authorization"])
                for request in self.provider.requests
            ]
            log = Path(str(self.broker) + ".log")
            broker = log.read_bytes() if log.exists() else b""
            outcomes.append((mapped, self.tables(root, labels), requests, broker))
        (python, python_tables, python_requests, python_broker), (rust, rust_tables, rust_requests, rust_broker) = outcomes
        self.assertEqual(sorted(rust), sorted(python))
        mismatches = [
            (rid, got, python[rid])
            for rid, got in rust.items()
            if got != python[rid]
        ]
        self.assertEqual(mismatches, [])
        self.assertEqual(rust_tables, python_tables)
        self.assertEqual(rust_requests, python_requests)
        self.assertEqual(rust_broker, python_broker)
        self.requests = rust_requests
        self.broker_log = rust_broker
        return {rid: [json.loads(line) for line in group] for rid, group in rust.items()}

    def provider_steps(self, *extra):
        return [
            {"request": {"kind": "ai.providers.create", "args": {"name": "tool-local", "base_url": self.provider.base_url, "kind": "local", "default_model": "test-model"}}},
            {"request": {"kind": "ai.providers.set_default", "args": {"name": "tool-local"}}},
            *extra,
        ]

    # --- scenarios --------------------------------------------------------

    def test_tools_off_chat_history_policy_and_sessions(self):
        def setup(provider):
            provider.queue(
                "POST",
                "responses",
                _answer("first answer"),
                _answer("second answer"),
                _answer("not stored"),
                _answer("auto policy"),
            )

        user = lambda text: {"role": "user", "content": text}  # noqa: E731
        records = self.session(
            self.provider_steps(
                {"request": {"kind": "ui.chat.history.configure"}},
                {"request": {"kind": "ui.chat.history.configure", "args": {"history": "bogus"}}},
                {"request": {"kind": "ui.chat.sessions.list"}},
                # `auto` on a plaintext book stores nothing, even when asked.
                {"request": {"kind": "ai.chat", "args": {"model": "test-model", "persist": "auto", "messages": [user("auto question")]}}},
                {"request": {"kind": "ui.chat.history.configure", "args": {"history": "on"}}},
                {"request": {"kind": "ai.chat", "args": {"model": "test-model", "persist": True, "messages": [user("What   changed\nthis week?")]}}},
                {"request": lambda seen: {"kind": "ai.chat", "args": {"model": "test-model", "session_id": seen["session_id"], "messages": [user("What changed this week?"), {"role": "assistant", "content": "first answer"}, user("And before?")]}}},
                {"request": {"kind": "ai.chat", "args": {"model": "test-model", "persist": False, "messages": [user("incognito")]}}},
                {"request": {"kind": "ui.chat.sessions.list", "args": {"limit": "1"}}},
                {"request": {"kind": "ui.chat.sessions.list", "args": {"limit": "x"}}},
                {"request": lambda seen: {"kind": "ui.chat.sessions.get", "args": {"session_id": seen["session_id"]}}},
                {"request": {"kind": "ui.chat.sessions.get", "args": {}}},
                {"request": {"kind": "ui.chat.sessions.get", "args": {"session_id": "missing"}}},
                {"request": {"kind": "ai.chat", "args": {"model": "m", "session_id": "missing", "messages": [user("x")]}}},
                {"request": lambda seen: {"kind": "ui.chat.sessions.delete", "args": {"session_id": seen["session_id"]}}},
                {"request": {"kind": "ui.chat.sessions.clear"}},
                {"request": {"kind": "ui.chat.history.configure", "args": {"history": "off"}}},
            ),
            setup=setup,
        )
        terminals = [group[-1] for group in records.values() if group[-1]["kind"] == "ai.chat"]
        self.assertEqual([t["data"]["session_id"] is None for t in terminals], [True, False, False, True])
        self.assertEqual(len(self.requests), 4)

    def test_validation_errors_contact_nothing(self):
        user = {"role": "user", "content": "hi"}
        cases = [
            {},
            {"model": " "},
            {"model": "m"},
            {"model": "m", "messages": [5]},
            {"model": "m", "messages": [{"role": "robot", "content": "x"}]},
            {"model": "m", "messages": [{"role": "user", "content": 5}]},
            {"model": "m", "messages": [{"role": "tool", "content": "x", "tool_call_id": ""}]},
            {"model": "m", "messages": [{"role": "assistant", "content": "x", "tool_calls": "no"}]},
            {"model": "m", "messages": [user], "options": [1]},
            {"model": "m", "messages": [user], "options": {"fast_mode": "yes"}},
            {"model": "m", "messages": [user], "options": {"stateless_session": 1}},
            {"model": "m", "messages": [user], "provider": 5},
            {"model": "m", "messages": [user], "tools_enabled": "yes"},
            {"model": "m", "messages": [user], "tool_profile": "huge"},
            {"model": "m", "messages": [user], "timeout_seconds": "soon"},
            {"model": "m", "messages": [user], "timeout_seconds": 0},
            {"model": "m", "messages": [user], "timeout_seconds": "1e9"},
            {"model": "m", "messages": [user], "tool_loop_max_iterations": "8.0"},
            {"model": "m", "messages": [user], "tool_loop_max_iterations": 33},
            {"model": "m", "messages": [user], "system_prompt_kind": "other"},
            {"model": "m", "messages": [user], "system_prompt": 5},
            {"model": "m", "messages": [user], "system_prompt": "x"},
            {"model": "m", "messages": [user], "session_id": ""},
            {"model": "m", "messages": [user], "persist": "always"},
            {"model": "m", "messages": [user], "seed_history": "yes"},
            {"model": "m", "messages": [user], "seed_notices": {}},
            {"model": "m", "messages": [user], "seed_notices": [{"index": 0, "notices": []}]},
            {"model": "m", "messages": [user], "screen_context": []},
            {"model": "m", "messages": [user], "screen_context": {"route": "/nowhere"}},
            {"model": "m", "messages": [user], "screen_context": {"route": "/", "zoom": 1}},
            {"model": "m", "messages": [user], "screen_context": {"entity_type": "planet"}},
            {"model": "m", "messages": [user], "screen_context": {"entity_id": "/etc/passwd"}},
            {"model": "m", "messages": [user], "screen_context": {"filters": {"path": "x"}}},
            {"model": "m", "messages": [user], "screen_context": {"filters": {"api_key": "sk-1"}}},
            {"model": "m", "messages": [user], "screen_context": {"capabilities": ["wizardry"]}},
            {"model": "m", "messages": [user], "screen_context": {"metrics": {"incoming_count": 1}}},
            {"model": "m", "messages": [user], "screen_context": {"metrics": {"other": 1}, "metrics_scope": {}}},
            {"model": "m", "messages": [user], "screen_context": {"metrics_scope": {"workspace_id": "w", "profile_id": "p", "input_version": -1}}},
            {"model": "m", "messages": [user], "attachment": "file.csv"},
            {"model": "m", "messages": [user], "attachment": {"path": "/x"}},
            {"model": "m", "messages": [user], "attachment": {"token": " "}},
            {"model": "m", "messages": [user], "attachment": {"token": "t", "label": "x" * 501}},
            {"model": "m", "messages": [user], "attachment": {"token": "stale"}},
            {"model": "m", "messages": [user], "provider": "nobody"},
            {"model": "m", "messages": [user], "provider": ""},
        ]
        self.session(
            [
                *[{"request": {"kind": "ai.chat", "args": args}} for args in cases],
                {"request": {"kind": "ai.chat", "args": [1]}},
                {"request": {"kind": "ai.chat.cancel", "args": {}}},
                {"request": {"kind": "ai.chat.cancel", "args": {"target_request_id": 5}}},
                {"request": {"kind": "ai.tool_call.consent", "args": {"target_request_id": "x"}}},
                {"request": {"kind": "ai.tool_call.consent", "args": {"target_request_id": "x", "call_id": "c", "decision": "maybe"}}},
                {"request": {"kind": "ai.tool_call.consent", "args": {"target_request_id": "missing-chat", "call_id": "call_1", "decision": "allow_once"}}},
            ]
        )
        self.assertEqual(self.requests, [])

    def test_tool_loop_records_consent_and_denials(self):
        def setup(provider):
            provider.queue(
                "POST",
                "responses",
                responses_stream(
                    [
                        message_item("Let me look."),
                        _function_call("call_status", "status"),
                        _function_call("call_unknown", "no_such_tool"),
                        _function_call("call_bad", "status", "{not json"),
                        _function_call("call_dup", "ui_journals_process"),
                        _function_call("call_dup", "ui_journals_process"),
                        _function_call("call_mut", "ui_journals_process"),
                    ],
                    text="Let me look.",
                ),
                _answer("Kassiber is ready."),
            )

        records = self.session(
            self.provider_steps(
                {
                    "request": {
                        "kind": "ai.chat",
                        "args": {
                            "model": "test-model",
                            "tools_enabled": True,
                            "tool_profile": "core",
                            "messages": [{"role": "user", "content": "hello there"}],
                            "screen_context": {
                                "route": "/transactions",
                                "entity_type": "transaction",
                                "entity_id": "tx-not-in-this-book",
                                "capabilities": ["transactions"],
                                "filters": {"period": "ytd"},
                                "metrics": {"incoming_count": 3},
                                "metrics_scope": {"workspace_id": "elsewhere", "profile_id": "other", "input_version": 0},
                            },
                        },
                    },
                    # The first `call_dup` and `call_mut` ask; the repeated
                    # id is denied without a prompt.
                    "consent": ["deny", "deny"],
                },
            ),
            setup=setup,
        )
        chat = records["r2"]
        kinds = [record["kind"] for record in chat]
        self.assertEqual(kinds[0], "ai.chat.status")
        self.assertEqual(kinds[-1], "ai.chat")
        results = {r["data"]["call_id"]: r["data"] for r in chat if r["kind"] == "ai.chat.tool_result"}
        self.assertTrue(results["call_status"]["ok"])
        self.assertEqual(results["call_unknown"]["reason"], "tool_not_allowed")
        self.assertEqual(results["call_bad"]["reason"], "invalid_arguments")
        self.assertEqual(results["call_mut"]["reason"], "user_denied")
        second = self.requests[1][2]
        outputs = [item for item in second["input"] if item.get("type") == "function_call_output"]
        self.assertEqual(len(outputs), 6)
        # The foreign transaction and the stale metrics never reach the model.
        context = json.dumps(self.requests[0][2])
        self.assertNotIn("tx-not-in-this-book", context)
        self.assertNotIn("incoming_count", context)

    def test_cancel_before_registration_contacts_nothing(self):
        records = self.session(
            self.provider_steps(
                {"request": {"kind": "ai.chat.cancel", "args": {"target_request_id": "r3"}}},
                {"request": {"kind": "ai.chat", "args": {"model": "test-model", "messages": [{"role": "user", "content": "hello"}]}}},
            )
        )
        self.assertEqual([record["kind"] for record in records["r3"]], ["ai.chat"])
        self.assertEqual(records["r3"][0]["data"]["finish_reason"], "cancelled")
        self.assertEqual(self.requests, [])

    def test_cancel_while_waiting_for_consent(self):
        def setup(provider):
            provider.queue(
                "POST",
                "responses",
                responses_stream([_function_call("call_mut", "ui_journals_process")]),
            )

        records = self.session(
            self.provider_steps(
                {
                    "request": {
                        "kind": "ai.chat",
                        "args": {
                            "model": "test-model",
                            "tools_enabled": True,
                            "tool_profile": "full",
                            "persist": True,
                            "messages": [{"role": "user", "content": "hello there"}],
                        },
                    },
                    "consent": ["cancel"],
                },
            ),
            setup=setup,
        )
        self.assertEqual(records["r2"][-1]["data"]["finish_reason"], "cancelled")
        self.assertEqual(len(self.requests), 1)

    def test_cancel_after_the_book_locks_mid_turn(self):
        passphrase = "correct horse battery staple"

        def prepare(root):
            migrate_plaintext_to_encrypted(root / "kassiber.sqlite3", passphrase)
            self.passphrase = passphrase

        def setup(provider):
            provider.queue(
                "POST",
                "responses",
                responses_stream([_function_call("call_sync", "ui_wallets_sync")]),
            )

        records = self.session(
            [
                {"request": {"kind": "status"}},
                {"request": {"kind": "ai.chat", "args": {"model": "m", "messages": [{"role": "user", "content": "x"}]}}},
                {"request": {"kind": "daemon.unlock", "args": {"auth_response": {"passphrase_secret": passphrase}}}},
                *self.provider_steps(
                    {
                        "request": {
                            "kind": "ai.chat",
                            "args": {
                                "model": "test-model",
                                "tools_enabled": True,
                                "tool_profile": "full",
                                "persist": True,
                                # A raw prompt plans no automatic reads.
                                "system_prompt_kind": "raw",
                                "system_prompt": "Use tools.",
                                "messages": [{"role": "user", "content": "Sync my wallets"}],
                            },
                        },
                        "consent": [[
                            {"kind": "daemon.lock"},
                            {"kind": "ai.chat.cancel", "args": {"target_request_id": "{rid}"}},
                        ]],
                    },
                ),
                {"request": {"kind": "ai.chat", "args": {"model": "m", "messages": [{"role": "user", "content": "x"}]}}},
            ],
            setup=setup,
            prepare=prepare,
        )
        self.assertEqual(records["r1"][0]["kind"], "auth_required")
        self.assertEqual(records["r5"][-1]["data"]["finish_reason"], "cancelled")
        self.assertEqual(records["r6"][0]["kind"], "auth_required")

    def test_auto_format_fallback_is_remembered(self):
        def setup(provider):
            provider.queue("POST", "responses", Reply(404, json_body={"detail": "Not Found"}))
            provider.queue(
                "POST",
                "chat/completions",
                chat_stream(chunk({"content": "Hi there"}), chunk({}, finish="stop")),
                chat_stream(chunk({"content": "Again"}), chunk({}, finish="length")),
            )

        self.session(
            self.provider_steps(
                {"request": {"kind": "ai.chat", "args": {"model": "test-model", "messages": [{"role": "user", "content": "hello"}]}}},
                {"request": {"kind": "ai.providers.get", "args": {"name": "tool-local"}}},
                {"request": {"kind": "ai.chat", "args": {"model": "test-model", "tools_enabled": True, "system_prompt_kind": "raw", "system_prompt": "Be terse.", "messages": [{"role": "user", "content": "again"}]}}},
            ),
            setup=setup,
        )
        self.assertEqual([request[1] for request in self.requests], ["/v1/responses", "/v1/chat/completions", "/v1/chat/completions"])

    def test_broker_chat_tool_round_and_resumed_cursor(self):
        self.session(
            [
                {"request": {"kind": "ai.providers.acknowledge", "args": {"name": "codex"}}},
                {"request": {"kind": "ai.providers.update", "args": {"name": "codex", "enabled": True}}},
                {"request": {"kind": "ui.chat.history.configure", "args": {"history": "on"}}},
                {"request": {"kind": "ai.chat", "args": {"provider": "codex", "model": "default", "tools_enabled": True, "persist": True, "options": {"fast_mode": True}, "messages": [{"role": "user", "content": "hello"}]}}},
                {"request": lambda seen: {"kind": "ai.chat", "args": {"provider": "codex", "model": "default", "session_id": seen["session_id"], "messages": [{"role": "user", "content": "hello"}, {"role": "assistant", "content": "Checking all good"}, {"role": "user", "content": "thanks"}]}}},
                {"request": {"kind": "ai.chat", "args": {"provider": "codex", "model": "default", "persist": False, "messages": [{"role": "user", "content": "private"}]}}},
            ]
        )
        sent = [json.loads(line) for line in self.broker_log.splitlines()]
        chats = [line for line in sent if line.get("command") == "chat"]
        self.assertEqual(len(chats), 3)
        # Python records the cursor against the turn's transcript plus the
        # last leg's answer, then looks it up against the next request's
        # whole transcript, which already holds the new prompt: a cursor is
        # never resumed (tests/rust_port/repro_chat_cursor_never_resumes.py).
        # Rust keeps that behaviour.
        self.assertNotIn("provider_session_id", chats[1].get("options") or {})
        self.assertTrue(chats[2]["options"].get("stateless_session"))

    def test_offline_mode_refuses_a_remote_provider_before_contact(self):
        offline = self.base / "preferences" / "offline.json"
        offline.parent.mkdir(parents=True, exist_ok=True)

        def prepare(_root):
            offline.write_text(json.dumps({"enabled": True, "schema_version": 1}) + "\n", encoding="utf-8")
            offline.chmod(0o600)

        self.session(
            [
                {"request": {"kind": "ai.providers.create", "args": {"name": "far", "base_url": "https://far.invalid/v1", "acknowledged": True}}},
                {"request": {"kind": "ai.providers.acknowledge", "args": {"name": "far"}}},
                {"request": {"kind": "ai.chat", "args": {"provider": "far", "model": "m", "messages": [{"role": "user", "content": "hello"}]}}},
                {"request": {"kind": "ai.chat", "args": {"provider": "codex", "model": "default", "messages": [{"role": "user", "content": "hello"}]}}},
            ],
            prepare=prepare,
        )
        self.assertEqual(self.requests, [])
        self.assertEqual(self.broker_log, b"")


def _rid_order(rid):
    if not isinstance(rid, str):
        return (0, -1, "")
    match = re.fullmatch(r"r(\d+)(.*)", rid)
    return (1, int(match.group(1)), match.group(2)) if match else (2, 0, rid)


def _label(line: bytes, labels: dict[bytes, bytes], *, grow: bool = True) -> bytes:
    def replace(match):
        value = match.group(0)
        if value not in labels:
            if not grow:
                return value
            labels[value] = f"<uuid-{len(labels)}>".encode()
        return labels[value]

    return UUID.sub(replace, line)


if __name__ == "__main__":
    unittest.main()
