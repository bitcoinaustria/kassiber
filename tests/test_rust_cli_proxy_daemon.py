"""CLI proxy providers end to end on the Rust daemon and CLI (docs/reference/ai-cli-proxy.md).

One daemon session against a loopback fake CLIProxyAPI: the provider is
created, keyed, listed and read back without a request; a chat before any
model list is refused without one; Test & discover saves the list; chats then
reach each model in its family's format (Claude over Chat Completions, Codex
over the Responses API), a tool call goes through consent, and a model the
list lacks is refused before anything is sent. The CLI creates one with
`ai providers create --kind cli_proxy --acknowledge` (a Rust-only grammar,
docs/reference/rust-shell.md#cli-proxy-providers), the key from a file
descriptor, and contacts the proxy only for `ai models` and `ai providers
test`. Rust only: the Python backend
refuses the kind (`tests/test_ai_cli_proxy_rust_only.py`).

Children get their own XDG roots and preference files; HOME stays the
account's. Only the loopback fake is contacted.
"""

from __future__ import annotations

import json
import os
import select
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

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
WAIT = 60.0
KEY = "sk-proxy-0123456789abcdef"
MODELS = {
    "object": "list",
    "data": [
        {"id": "claude-sonnet-5-5", "object": "model", "created": 1, "owned_by": "anthropic"},
        {"id": "gpt-6.1-sol", "object": "model", "created": 1, "owned_by": "openai"},
        {"id": "gpt-image-2", "object": "model", "created": 1, "owned_by": "openai"},
    ],
}


def _chat(model: str, **extra) -> dict:
    return {
        "kind": "ai.chat",
        "args": {
            "provider": "proxy",
            "model": model,
            "messages": [{"role": "user", "content": "hello"}],
            "persist": False,
            **extra,
        },
    }


@unittest.skipUnless(RUST_BIN and os.name == "posix", "set KASSIBER_RUST_BIN to a test-harness build")
class RustCliProxyDaemonTest(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-cli-proxy-daemon-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / "data"
        self.env = _environment(self.base / "preferences")
        self.env["HOME"] = os.environ["HOME"]
        self.env.update(
            {
                "KASSIBER_STATE_ROOT": str(self.base / "state"),
                # AI keeps its own online gate; the observer admits loopback.
                "KASSIBER_NO_EGRESS": "1",
                "KASSIBER_TEST_NO_EGRESS": "1",
            }
        )
        self.proxy = ScriptedProvider()
        self.addCleanup(self.proxy.stop)
        for args in (["init"], ["workspaces", "create", "Demo"], ["profiles", "create", "Main"]):
            result = subprocess.run(
                [str(Path(RUST_BIN).resolve()), "--machine", "--data-root", str(self.root), *args],
                capture_output=True,
                env=self.env,
                cwd=ROOT,
                timeout=120,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def paths(self) -> list[tuple[str, str]]:
        with self.proxy.lock:
            return [(request["method"], request["path"]) for request in self.proxy.requests]

    def session(self, requests: list[dict]) -> list[list[dict]]:
        """One daemon session; each request's records, consents allowed."""
        process = subprocess.Popen(
            [str(Path(RUST_BIN).resolve()), "--data-root", str(self.root), "daemon"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=self.env,
            cwd=ROOT,
        )
        buffer = b""

        def read_record(deadline):
            nonlocal buffer
            while b"\n" not in buffer:
                remaining = deadline - time.monotonic()
                self.assertGreater(remaining, 0, "daemon timed out")
                ready, _, _ = select.select([process.stdout], [], [], remaining)
                if ready:
                    data = os.read(process.stdout.fileno(), 65536)
                    if not data:
                        self.fail(f"daemon exited: {process.stderr.read()[-2000:]!r}")
                    buffer += data
            line, buffer = buffer.split(b"\n", 1)
            return json.loads(line)

        def send(request):
            process.stdin.write(json.dumps(request).encode() + b"\n")
            process.stdin.flush()

        results: list[list[dict]] = []
        try:
            self.assertEqual(read_record(time.monotonic() + WAIT)["kind"], "daemon.ready")
            for index, request in enumerate([*requests, {"kind": "daemon.shutdown"}]):
                rid = f"r{index}"
                send({"request_id": rid, **request})
                records: list[dict] = []
                waiting = {rid}
                deadline = time.monotonic() + WAIT
                while waiting:
                    record = read_record(deadline)
                    if record.get("request_id") == rid:
                        records.append(record)
                        kind = record.get("kind")
                        if request["kind"] != "ai.chat" or kind in {"ai.chat", "error"}:
                            waiting.discard(rid)
                        elif kind == "ai.chat.tool_consent_required":
                            control = f"{rid}c{len(waiting)}"
                            send({
                                "request_id": control,
                                "kind": "ai.tool_call.consent",
                                "args": {
                                    "target_request_id": rid,
                                    "call_id": record["data"]["call_id"],
                                    "decision": "allow_once",
                                },
                            })
                            waiting.add(control)
                    else:
                        waiting.discard(record.get("request_id"))
                results.append(records)
            process.stdin.close()
            self.assertEqual(process.wait(timeout=WAIT), 0)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)
            process.stdout.close()
            process.stderr.close()
        return results

    def test_a_cli_proxy_is_set_up_and_chats_in_each_models_format(self):
        base = self.proxy.base_url
        self.proxy.queue("GET", "models", Reply(json_body=MODELS))
        self.proxy.queue(
            "POST",
            "chat/completions",
            chat_stream(chunk({"role": "assistant", "content": "from claude"}), chunk({}, finish="stop")),
            # The tool turn: one call, then the answer after its result.
            chat_stream(
                chunk({"role": "assistant", "tool_calls": [{
                    "index": 0, "id": "call_proxy", "type": "function",
                    "function": {"name": "ui_journals_process", "arguments": "{}"},
                }]}),
                chunk({}, finish="tool_calls"),
            ),
            chat_stream(chunk({"content": "processed"}), chunk({}, finish="stop")),
        )
        self.proxy.queue(
            "POST", "responses", responses_stream([message_item("from codex")], text="from codex")
        )
        records = self.session(
            [
                {"kind": "ai.providers.create", "args": {
                    "name": "proxy", "base_url": base, "kind": "cli_proxy", "acknowledged": True,
                }},
                {"kind": "ai.providers.set_api_key", "args": {"name": "proxy", "api_key": KEY}},
                {"kind": "ai.providers.list"},
                {"kind": "ai.list_models", "args": {"provider": "proxy", "cached_only": True}},
                _chat("claude-sonnet-5-5"),
                {"kind": "ai.test_connection", "args": {"provider": "proxy", "base_url": base}},
                _chat("claude-sonnet-5-5", options={"reasoning_effort": "xhigh"}),
                _chat("gpt-6.1-sol"),
                _chat("claude-sonnet-5-5", tools_enabled=True, tool_profile="full"),
                _chat("claude-unlisted"),
                _chat("gpt-image-2"),
            ]
        )
        created, keyed, listed, cached, unchecked, tested, claude, codex, tools, unlisted, image = records[:11]
        self.assertEqual(created[-1]["data"]["kind"], "cli_proxy")
        self.assertIsNone(created[-1]["data"]["effective_api_format"])
        self.assertTrue(keyed[-1]["data"]["has_api_key"])
        self.assertEqual(cached[-1]["data"]["models"], [])
        self.assertEqual(unchecked[-1]["kind"], "error")
        self.assertEqual(unchecked[-1]["error"]["code"], "ai_cli_proxy_models_unchecked")
        self.assertNotIn(KEY, json.dumps(listed))

        self.assertTrue(tested[-1]["data"]["models_saved"])
        formats = {row["id"]: row["api_format"] for row in tested[-1]["data"]["models"]}
        self.assertEqual(
            formats,
            {"claude-sonnet-5-5": "chat_completions", "gpt-6.1-sol": "responses", "gpt-image-2": None},
        )

        def answer(chat):
            return "".join(
                record["data"]["delta"].get("content") or ""
                for record in chat
                if record["kind"] == "ai.chat.delta"
            )

        self.assertEqual(claude[-1]["kind"], "ai.chat", claude[-1])
        self.assertEqual(answer(claude), "from claude")
        self.assertEqual(codex[-1]["kind"], "ai.chat", codex[-1])
        self.assertEqual(answer(codex), "from codex")
        kinds = [record["kind"] for record in tools]
        self.assertIn("ai.chat.tool_consent_required", kinds)
        result = next(record for record in tools if record["kind"] == "ai.chat.tool_result")
        self.assertTrue(result["data"]["ok"], result)
        self.assertEqual(tools[-1]["kind"], "ai.chat", tools[-1])
        self.assertEqual(unlisted[-1]["error"]["code"], "ai_cli_proxy_model_unknown")
        self.assertEqual(image[-1]["error"]["code"], "ai_cli_proxy_format_unknown")
        self.assertEqual(image[-1]["error"]["details"]["reason"], "not_a_chat_model")

        # Nothing before Test & discover; then the listing, two Claude legs
        # plus the tool turn's two over Chat Completions, Codex over the
        # Responses API, and nothing for the refused models.
        self.assertEqual(
            self.paths(),
            [
                ("GET", "/v1/models"),
                ("POST", "/v1/chat/completions"),
                ("POST", "/v1/responses"),
                ("POST", "/v1/chat/completions"),
                ("POST", "/v1/chat/completions"),
            ],
        )
        with self.proxy.lock:
            requests = list(self.proxy.requests)
        self.assertTrue(all(request["authorization"] == f"Bearer {KEY}" for request in requests))
        effort_turn = requests[1]["body"]
        self.assertEqual(effort_turn["reasoning_effort"], "xhigh")
        self.assertIs(effort_turn["store"], False)
        self.assertIs(requests[2]["body"]["store"], False)


    def cli(self, *args: str, key: str | None = None) -> tuple[int, dict]:
        """One `--machine` CLI run; the key, if any, through `--api-key-fd`."""
        command = [str(Path(RUST_BIN).resolve()), "--machine", "--data-root", str(self.root), *args]
        fds: tuple[int, ...] = ()
        if key is not None:
            read, write = os.pipe()
            os.write(write, key.encode())
            os.close(write)
            command += ["--api-key-fd", str(read)]
            fds = (read,)
        try:
            result = subprocess.run(
                command, capture_output=True, env=self.env, cwd=ROOT, timeout=120, pass_fds=fds
            )
        finally:
            for fd in fds:
                os.close(fd)
        self.assertNotIn(KEY.encode(), result.stdout + result.stderr)
        return result.returncode, json.loads(result.stdout)

    def test_the_cli_creates_a_cli_proxy_and_contacts_it_only_to_list_or_test(self):
        base = self.proxy.base_url
        create = ["ai", "providers", "create", "proxy", "--kind", "cli_proxy", "--base-url", base]
        # Like the desktop wizard, it is saved only with the acknowledgement.
        code, refused = self.cli(*create, key=KEY)
        self.assertNotEqual(code, 0)
        self.assertEqual(refused["error"]["code"], "validation")
        self.assertEqual(refused["error"]["message"], "A CLI proxy provider needs --acknowledge")
        # It takes no format, no store choice and no CLI locator.
        for extra, message in [
            (["--api-format", "responses"], "CLI proxy providers do not take an API format"),
            (["--omit-store-field"], "CLI proxy providers do not take omit_store_field"),
        ]:
            code, envelope = self.cli(*create, "--acknowledge", *extra)
            self.assertEqual(envelope["error"]["message"], message)
        code, envelope = self.cli(
            "ai", "providers", "create", "proxy", "--kind", "cli_proxy",
            "--base-url", "claude-cli://default", "--acknowledge",
        )
        self.assertEqual(envelope["error"]["message"], "A CLI proxy provider needs the proxy's HTTP address")

        code, created = self.cli(*create, "--acknowledge", "--default-model", "claude-sonnet-5-5", key=KEY)
        self.assertEqual(code, 0, created)
        self.assertEqual(created["data"]["kind"], "cli_proxy")
        self.assertTrue(created["data"]["has_api_key"])
        self.assertIsNotNone(created["data"]["acknowledged_at"])
        self.assertTrue(created["data"]["supports_reasoning_effort"])
        self.cli("ai", "providers", "list")
        self.cli("ai", "providers", "get", "proxy")
        self.assertEqual(self.paths(), [])

        self.proxy.queue("GET", "models", Reply(json_body=MODELS), Reply(json_body=MODELS))
        self.proxy.queue(
            "POST",
            "chat/completions",
            chat_stream(chunk({"role": "assistant", "content": "OK"}), chunk({}, finish="stop")),
        )
        code, listed = self.cli("ai", "models", "--provider", "proxy")
        self.assertEqual(code, 0, listed)
        formats = {row["id"]: row["api_format"] for row in listed["data"]["models"]}
        self.assertEqual(formats["claude-sonnet-5-5"], "chat_completions")
        self.assertEqual(formats["gpt-6.1-sol"], "responses")
        code, tested = self.cli("ai", "providers", "test", "proxy")
        self.assertEqual(code, 0, tested)
        self.assertEqual(tested["data"]["probe"]["api_format"], "chat_completions")
        self.assertTrue(tested["data"]["models_saved"])
        self.assertEqual(
            self.paths(),
            [("GET", "/v1/models"), ("GET", "/v1/models"), ("POST", "/v1/chat/completions")],
        )
        with self.proxy.lock:
            self.assertTrue(all(r["authorization"] == f"Bearer {KEY}" for r in self.proxy.requests))
        # The grammar names the kind.
        result = subprocess.run(
            [str(Path(RUST_BIN).resolve()), "ai", "providers", "create", "--help"],
            capture_output=True, env=self.env, cwd=ROOT, timeout=60,
        )
        self.assertIn(b"{local,remote,tee,cli_proxy}", result.stdout)


if __name__ == "__main__":
    unittest.main()
