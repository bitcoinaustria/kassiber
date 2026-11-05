"""R6-6b parity: the provider broker client's chat legs, chunk for chunk.

Each scenario runs Python's `BrokerAIClient` (`tests/rust_port/ai_broker_reference.py`)
and the Rust `BrokerClient` (`crates/ai/examples/broker_stream.rs`, named by
``KASSIBER_RUST_AI_BROKER_EXAMPLE``) against the same fake broker, found
through ``KASSIBER_AI_BROKER_NODE`` and ``KASSIBER_AI_PROVIDER_BROKER``. Every
streamed chunk and each leg's outcome must match byte for byte, and so must
the request lines the fake broker read. The daemon kinds and CLI leaves that
start the broker are compared in `tests/test_rust_ai_providers.py`; `ai.chat`
itself is R6-7.

Children get their own XDG data root; HOME stays the account's. Nothing is
contacted: the fake broker is a local script.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXAMPLE = os.environ.get("KASSIBER_RUST_AI_BROKER_EXAMPLE")

FAKE_BROKER = r"""
import json, sys, time
LOG = sys.argv[0] + ".log"

def read():
    line = sys.stdin.readline()
    with open(LOG, "a", encoding="utf-8") as log:
        log.write(line)
    return json.loads(line)

def say(event, **kwargs):
    print(json.dumps(event, **kwargs), flush=True)

request = read()
model = request.get("model")
if model == "tools":
    say({"type": "status", "phase": "reasoning_effort_unavailable"})
    say({"type": "delta", "content": "Looking é ", "reasoning": "plan"})
    say({"type": "delta", "content": "", "reasoning": ""})
    say({"type": "tool_call", "call_id": "call-😀", "name": "ui_reports_tax_summary", "arguments": {"z": 1, "a": [1.5, "☃"], "m": {"y": None}}})
    reply = read()
    say({"type": "delta", "content": "done"})
    say({"type": "done", "finish_reason": "length", "provider_session_id": "session-2"})
elif model == "quirks":
    print("not json", flush=True)
    print("[1]", flush=True)
    sys.stdout.write(json.dumps({"type": "delta", "content": "cr"}) + "\r")
    sys.stdout.write(json.dumps({"type": "delta", "content": "lf"}) + "\r\n")
    say({"type": "delta", "content": "a b"}, ensure_ascii=False)
    say({"type": "status", "phase": "connecting"})
    say({"type": "done", "finish_reason": 7})
elif model == "fail":
    say({"type": "delta", "content": "partial"})
    say({"type": "error", "code": "authentication_required"})
elif model == "bad-call":
    say({"type": "tool_call", "call_id": "", "name": "x", "arguments": {}})
elif model == "slow":
    for index in range(5):
        say({"type": "delta", "content": str(index)})
        time.sleep(0.2)
    say({"type": "done"})
else:
    say({"type": "delta", "content": "hello"})
    say({"type": "done", "provider_session_id": "session-1"})
"""

TOOLS = [
    {"type": "function", "function": {"name": "ui_reports_tax_summary", "description": "Read", "parameters": {"type": "object"}}},
    {"type": "function", "name": "unknown_tool", "parameters": {}},
    {"name": "bare_tool", "description": ""},
    "not a tool",
]


@unittest.skipUnless(EXAMPLE and os.name == "posix", "set KASSIBER_RUST_AI_BROKER_EXAMPLE to the broker_stream example")
class RustAiBrokerParityTest(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-r6-ai-broker-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.broker = self.base / "fake_broker.py"
        self.broker.write_text(FAKE_BROKER, encoding="utf-8")
        self.env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("KASSIBER_", "SATBOOKS_"))
        }
        self.env.update(
            {
                "XDG_DATA_HOME": str(self.base / "xdg-data"),
                "PYTHONIOENCODING": "utf-8",
                "KASSIBER_AI_BROKER_NODE": sys.executable,
                "KASSIBER_AI_PROVIDER_BROKER": str(self.broker),
            }
        )

    def run_both(self, scenario):
        outcomes = []
        log = Path(str(self.broker) + ".log")
        for command in (
            [sys.executable, "-m", "tests.rust_port.ai_broker_reference"],
            [str(Path(EXAMPLE).resolve())],
        ):
            log.unlink(missing_ok=True)
            result = subprocess.run(
                command,
                input=json.dumps(scenario).encode(),
                capture_output=True,
                env=self.env,
                cwd=ROOT,
                timeout=120,
            )
            self.assertEqual((result.returncode, result.stderr), (0, b""), result.stdout[-2000:])
            outcomes.append((result.stdout, log.read_bytes() if log.exists() else b""))
        (python_out, python_log), (rust_out, rust_log) = outcomes
        self.assertEqual(rust_out.decode().splitlines(), python_out.decode().splitlines())
        self.assertEqual(rust_out, python_out)
        self.assertEqual(rust_log, python_log)
        return [json.loads(line) for line in rust_out.splitlines()], rust_log

    def test_a_tool_round_keeps_one_broker_and_matches_every_chunk(self):
        lines, log = self.run_both(
            {
                "locator": " CODEX-CLI://default ",
                "legs": [
                    {
                        "model": "tools",
                        "options": {"reasoning_effort": "high", "temperature": 1, "provider_session_id": "s0", "fast_mode": True},
                        "tools": TOOLS,
                        "context": {
                            "instructions": "Be brief.",
                            "input_items": [
                                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "one"}, {"type": "image", "text": "no"}, {"type": "text", "text": "two"}]},
                                {"type": "message", "role": None, "content": "fallback role"},
                                {"type": "message", "role": "assistant", "content": []},
                                {"type": "reasoning", "summary": []},
                            ],
                        },
                    },
                    {
                        "model": "tools",
                        "context": {
                            "instructions": None,
                            "input_items": [
                                {"type": "function_call_output", "call_id": "call-😀", "output": None},
                                {"type": "function_call_output", "call_id": "other", "output": "x"},
                            ],
                        },
                    },
                ],
            }
        )
        self.assertEqual(lines[-1], {"launches": 1})
        self.assertEqual(len(log.splitlines()), 2)

    def test_quirks_errors_and_a_new_broker_per_finished_leg(self):
        lines, _ = self.run_both(
            {
                "locator": "claude-cli://default",
                "legs": [
                    {"model": "quirks", "messages": [{"role": "user", "content": "hi"}, {"content": 5}, "skip"], "options": {"stateless_session": True}},
                    {"model": "fail", "messages": [{"role": "user", "content": "hi"}]},
                    {"model": "fail", "messages": [], "options": {"sensitive_context": True}},
                    {"model": "bad-call", "options": None},
                    {"model": "plain", "options": {"stateless_session": "yes"}},
                    {"model": "plain", "options": {"sensitive_context": True, "provider_session_id": "x"}},
                    {"model": "plain", "options": {"fast_mode": None}},
                    {"model": "plain"},
                ],
            }
        )
        self.assertEqual(lines[-1], {"launches": 5})

    def test_cancel_mid_stream_and_offline_mode(self):
        lines, _ = self.run_both(
            {"locator": "opencode-cli://default", "legs": [{"model": "slow", "cancel_after": 2}, {"model": "plain"}]}
        )
        self.assertEqual(lines[-1], {"launches": 1})
        lines, log = self.run_both({"locator": "copilot-cli://default", "offline": True, "legs": [{"model": "plain"}]})
        self.assertEqual(lines[0]["end"]["error"]["code"], "network_egress_disabled")
        self.assertEqual((lines[-1], log), ({"launches": 0}, b""))


if __name__ == "__main__":
    unittest.main()
