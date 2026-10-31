"""Fast serving mode (Codex Fast service tier, Claude Opus fast mode).

The option is broker-only: it must cross to the provider broker only when it
is exactly ``True``, be rejected when it is not a boolean, never reach an
OpenAI-compatible HTTP body, and survive model-metadata normalization.
"""

from __future__ import annotations

import argparse
import io
import json
import queue
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from kassiber import daemon
from kassiber.ai.broker_client import BrokerAIClient
from kassiber.ai.client import _responses_options
from kassiber.ai.contracts import ResponsesRequestContext
from kassiber.ai.model_metadata import safe_model_capabilities
from kassiber.cli.chat import ChatSessionResult, ChatTurnResult, _chat_options
from kassiber.core import accounts as core_accounts
from kassiber.core import chat_history as core_chat_history
from kassiber.db import open_db
from kassiber.errors import AppError


FAKE_BROKER = """
import json, sys
request = json.loads(sys.stdin.readline())
with open(sys.argv[1] if len(sys.argv) > 1 else %(capture)r, "w") as handle:
    json.dump(request.get("options"), handle)
print(json.dumps({"type":"delta","content":"ok"}), flush=True)
print(json.dumps({"type":"done","finish_reason":"stop"}), flush=True)
"""


class BrokerFastModeTest(unittest.TestCase):
    def _options_sent(self, options: dict) -> object:
        with tempfile.TemporaryDirectory(prefix="kassiber-fast-test-") as tmp:
            capture = Path(tmp) / "options.json"
            script = Path(tmp) / "fake_broker.py"
            script.write_text(FAKE_BROKER % {"capture": str(capture)}, encoding="utf-8")
            with patch.dict("os.environ", {
                "KASSIBER_AI_BROKER_NODE": sys.executable,
                "KASSIBER_AI_PROVIDER_BROKER": str(script),
            }):
                client = BrokerAIClient(locator="codex-cli://default")
                chunks = list(client.stream_chat(
                    messages=[{"role": "user", "content": "Reply with just: ok"}],
                    model="gpt-5.4",
                    options=options,
                ))
            self.assertEqual(chunks[-1].finish_reason, "stop")
            return json.loads(capture.read_text(encoding="utf-8"))

    def test_forwards_only_an_explicit_true(self):
        self.assertEqual(
            self._options_sent({"fast_mode": True, "reasoning_effort": "high"}),
            {"reasoning_effort": "high", "fast_mode": True},
        )
        self.assertEqual(self._options_sent({"fast_mode": False}), {})

    def test_rejects_non_boolean_before_spawn(self):
        client = BrokerAIClient(locator="claude-cli://default")
        for value in ("true", 1, None):
            with self.subTest(value=value), patch("subprocess.Popen") as spawn:
                with self.assertRaises(AppError) as raised:
                    list(client.stream_chat(model="opus", options={"fast_mode": value}))
                self.assertEqual(raised.exception.code, "ai_request_invalid")
                spawn.assert_not_called()


class HttpFastModeTest(unittest.TestCase):
    def test_never_reaches_an_openai_compatible_body(self):
        body = _responses_options({"fast_mode": True, "reasoning_effort": "low"})
        self.assertNotIn("fast_mode", body)
        self.assertEqual(body["reasoning"]["effort"], "low")


class ModelMetadataFastModeTest(unittest.TestCase):
    def test_keeps_support_and_bounded_description(self):
        metadata = safe_model_capabilities({
            "supports_fast_mode": True,
            "fast_mode_description": "  2x speed, increased usage  ",
        })
        self.assertEqual(metadata["supports_fast_mode"], True)
        self.assertEqual(metadata["fast_mode_description"], "2x speed, increased usage")
        long = safe_model_capabilities({
            "supports_fast_mode": True,
            "fast_mode_description": "x" * 500,
        })
        self.assertLessEqual(len(long["fast_mode_description"]), 96)

    def test_drops_malformed_values(self):
        self.assertEqual(safe_model_capabilities({"supports_fast_mode": "yes"}), {})
        self.assertEqual(
            safe_model_capabilities({
                "supports_fast_mode": False,
                "fast_mode_description": "ignored without support",
            }),
            {"supports_fast_mode": False},
        )


class DaemonFastModeTest(unittest.TestCase):
    def _args(self, options: dict) -> dict:
        return daemon._ai_chat_args({
            "model": "gpt-5.4",
            "messages": [{"role": "user", "content": "Reply with just: ok"}],
            "options": options,
        })

    def test_accepts_boolean_fast_mode(self):
        self.assertEqual(self._args({"fast_mode": True})["options"], {"fast_mode": True})
        self.assertEqual(self._args({"fast_mode": False})["options"], {"fast_mode": False})

    def test_rejects_non_boolean_fast_mode(self):
        for value in ("true", 1, None, {}):
            with self.subTest(value=value):
                with self.assertRaises(AppError) as raised:
                    self._args({"fast_mode": value})
                self.assertEqual(raised.exception.code, "validation")


class CliFastFlagTest(unittest.TestCase):
    def test_fast_flag_adds_the_option(self):
        args = argparse.Namespace(
            temperature=None, max_tokens=None, reasoning_effort="auto", fast=True,
        )
        self.assertEqual(_chat_options(args), {"fast_mode": True})
        args.fast = False
        self.assertIsNone(_chat_options(args))

    def test_parser_exposes_fast(self):
        from kassiber.cli.main import build_parser

        parsed = build_parser().parse_args(["chat", "--fast", "hello"])
        self.assertIs(parsed.fast, True)
        self.assertIs(build_parser().parse_args(["chat", "hello"]).fast, False)


STATUS_BROKER = """
import json, sys
request = json.loads(sys.stdin.readline())
print(json.dumps({"type":"status","phase":"connecting","message":"Starting Claude"}), flush=True)
print(json.dumps({"type":"status","phase":"fast_mode_unavailable","message":"ignored text"}), flush=True)
print(json.dumps({"type":"status","phase":"provider_says_anything","message":"<b>x</b>"}), flush=True)
print(json.dumps({"type":"delta","content":"ok"}), flush=True)
print(json.dumps({"type":"done","finish_reason":"stop"}), flush=True)
"""


class FastModeNoticeTest(unittest.TestCase):
    def _chunks(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-fast-status-") as tmp:
            script = Path(tmp) / "fake_broker.py"
            script.write_text(STATUS_BROKER, encoding="utf-8")
            with patch.dict("os.environ", {
                "KASSIBER_AI_BROKER_NODE": sys.executable,
                "KASSIBER_AI_PROVIDER_BROKER": str(script),
            }):
                client = BrokerAIClient(locator="claude-cli://default")
                return list(client.stream_chat(
                    messages=[{"role": "user", "content": "Reply with just: ok"}],
                    model="opus",
                    options={"fast_mode": True},
                ))

    def test_broker_client_forwards_only_known_notices(self):
        chunks = self._chunks()
        self.assertEqual(
            [chunk.status_phase for chunk in chunks if chunk.status_phase],
            ["fast_mode_unavailable"],
        )
        notice = next(chunk for chunk in chunks if chunk.status_phase)
        self.assertEqual(notice.delta, {})
        self.assertEqual(
            "".join(chunk.delta.get("content", "") for chunk in chunks), "ok",
        )

    def test_daemon_stream_emits_the_notice_as_chat_status(self):
        chunks = self._chunks()

        class Client:
            def stream_chat(self, **_kwargs):
                yield from chunks

        written: list[dict] = []

        class Out:
            def write(self, payload):
                written.append(payload)

        daemon._stream_ai_chat_tool_turn(
            "req-1",
            Client(),
            {"model": "opus", "options": {"fast_mode": True}},
            ResponsesRequestContext(instructions=None, input_items=[]),
            [],
            Out(),
            threading.Event(),
        )
        statuses = [
            payload["data"] for payload in written if payload.get("kind") == "ai.chat.status"
        ]
        self.assertIn(
            {
                "phase": "fast_mode_unavailable",
                "label": "Fast mode unavailable; answering at standard speed",
            },
            statuses,
        )
        # The notice is not an empty delta, and the answer still streams.
        deltas = [
            payload["data"]["delta"] for payload in written if payload.get("kind") == "ai.chat.delta"
        ]
        self.assertEqual(deltas, [{"content": "ok"}])


class FastModeNoticeResultTest(unittest.TestCase):
    """The notice outlives the stream: terminal result, CLI JSON, history."""

    class Out:
        def __init__(self):
            self.written: list[dict] = []

        def write(self, payload):
            self.written.append(payload)

    def _runtime(self):
        return daemon.AiToolRuntime(
            data_root="/not-used",
            runtime_config={},
            main_thread_tasks=queue.Queue(),
            maintenance_state={"egress_after_id": 0},
        )

    def _terminal(self, runtime) -> dict:
        out = self.Out()
        daemon._write_ai_chat_terminal(
            out,
            "req-1",
            {"name": "claude", "kind": "cli"},
            {
                "model": "opus",
                "persist": False,
                "messages": [{"role": "user", "content": "Reply with just: ok"}],
            },
            "stop",
            runtime,
            assistant_content="ok",
        )
        return next(payload for payload in out.written if payload.get("kind") == "ai.chat")

    def test_terminal_result_and_machine_output_keep_the_notice(self):
        runtime = self._runtime()
        chunks = FastModeNoticeTest()._chunks()

        class Client:
            def stream_chat(self, **_kwargs):
                yield from chunks

        turn = daemon._stream_ai_chat_tool_turn(
            "req-1",
            Client(),
            {"model": "opus", "options": {"fast_mode": True}},
            ResponsesRequestContext(instructions=None, input_items=[]),
            [],
            self.Out(),
            threading.Event(),
        )
        self.assertEqual(turn.notices, ("fast_mode_unavailable",))
        # The tool loop records each round's notices; the plain stream path
        # records through the same helper, and a repeat is kept once.
        daemon._ai_chat_notices(runtime).extend(turn.notices)
        for chunk in chunks:
            daemon._forward_ai_chat_notice(
                self.Out(), "req-1", chunk, daemon._ai_chat_notices(runtime)
            )
        # Only recognised phases reach the result, whatever else is recorded.
        runtime.maintenance_state["notices"].append("provider_says_anything")

        terminal = self._terminal(runtime)
        self.assertEqual(
            terminal["data"]["provenance"]["notices"], ["fast_mode_unavailable"]
        )
        payload = ChatSessionResult(
            turns=[ChatTurnResult(content="ok", terminal=terminal)]
        ).to_payload()
        self.assertEqual(payload["provenance"]["notices"], ["fast_mode_unavailable"])

    def test_tool_loop_carries_a_round_notice_to_the_result(self):
        def fake_turn(*_args):
            return daemon.AiToolTurnResult(
                tool_calls=[],
                content="ok",
                reasoning="",
                finish_reason="stop",
                response_output=[],
                notices=("fast_mode_unavailable",),
            )

        stream = io.StringIO()
        active_ai_chats = daemon.ActiveAiChats()
        _key, active_chat = active_ai_chats.register("req-1")
        with patch.object(daemon, "_stream_ai_chat_tool_turn", fake_turn), patch.object(
            daemon, "_run_auto_read_tools", lambda **_kwargs: None
        ):
            daemon._run_ai_chat_tool_loop(
                "req-1",
                SimpleNamespace(last_provider_session_id=None),
                {"name": "claude", "kind": "cli", "base_url": "claude-cli://default"},
                {
                    "messages": [{"role": "user", "content": "Reply with just: ok"}],
                    "model": "opus",
                    "options": {"fast_mode": True},
                    "session_id": None,
                    "persist": False,
                    "system_prompt_kind": "kassiber",
                    "system_prompt": None,
                    "tools_enabled": True,
                    "tool_profile": "scoped",
                    "tool_loop_max_iterations": 2,
                    "screen_context": None,
                    "attachments": [],
                },
                daemon._OutputChannel(stream),
                active_chat,
                self._runtime(),
                active_ai_chats,
            )
        records = [json.loads(line) for line in stream.getvalue().splitlines()]
        terminal = next(record for record in records if record["kind"] == "ai.chat")
        self.assertEqual(
            terminal["data"]["provenance"]["notices"], ["fast_mode_unavailable"]
        )

    def test_a_turn_without_notices_reports_none(self):
        terminal = self._terminal(self._runtime())
        self.assertEqual(terminal["data"]["provenance"]["notices"], [])

    def test_stored_assistant_message_keeps_the_notice(self):
        provenance = self._terminal_with_notice()
        with tempfile.TemporaryDirectory(prefix="kassiber-fast-history-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                workspace = core_accounts.create_workspace(conn, "Demo", commit=False)
                profile = core_accounts.create_profile(
                    conn, workspace["id"], "Main", "EUR", None, "generic", 365,
                    commit=False,
                )
                session = core_chat_history.create_session(
                    conn, workspace["id"], profile["id"], title="fast",
                    provider="claude", model="opus", commit=False,
                )
                core_chat_history.append_exchange(
                    conn, profile["id"], session["id"],
                    user_content="Reply with just: ok", assistant_content="ok",
                    provenance=provenance, finish_reason="stop", commit=True,
                )
                stored = core_chat_history.get_session(conn, profile["id"], session["id"])
            finally:
                conn.close()
        assistant = stored["messages"][-1]
        self.assertEqual(assistant["role"], "assistant")
        self.assertEqual(assistant["provenance"]["notices"], ["fast_mode_unavailable"])

    def _terminal_with_notice(self) -> dict:
        runtime = self._runtime()
        daemon._ai_chat_notices(runtime).append("fast_mode_unavailable")
        return self._terminal(runtime)["data"]["provenance"]


class SeededNoticeTest(unittest.TestCase):
    """A branched chat's copied answers keep their notices when stored."""

    MESSAGES = [
        {"role": "user", "content": "First question"},
        {"role": "assistant", "content": "First answer"},
        {"role": "user", "content": "Second question"},
        {"role": "assistant", "content": "Second answer"},
        {"role": "user", "content": "Branched prompt"},
    ]

    def _args(self, seed_notices) -> dict:
        return daemon._ai_chat_args({
            "model": "opus",
            "messages": [dict(message) for message in self.MESSAGES],
            "seed_history": True,
            "seed_notices": seed_notices,
        })

    def test_seed_notices_are_validated_and_kept_out_of_the_prompt(self):
        validated = self._args([
            {"index": 1, "notices": ["fast_mode_unavailable", "provider_says_anything"]},
            {"index": 3, "notices": ["provider_says_anything"]},
        ])
        # Unknown phases are dropped; an answer left with none is not listed.
        self.assertEqual(validated["seed_notices"], {1: ["fast_mode_unavailable"]})
        # The model's messages carry no notices.
        self.assertEqual(validated["messages"], self.MESSAGES)
        self.assertEqual(self._args(None)["seed_notices"], {})

        for bad in (
            "fast_mode_unavailable",
            [{"index": 0, "notices": ["fast_mode_unavailable"]}],  # a user message
            [{"index": 4, "notices": ["fast_mode_unavailable"]}],  # the live prompt
            [{"index": 9, "notices": ["fast_mode_unavailable"]}],
            [{"index": True, "notices": ["fast_mode_unavailable"]}],
            [{"index": 1, "notices": "fast_mode_unavailable"}],
            [{"index": 1, "notices": [1]}],
        ):
            with self.subTest(seed_notices=bad), self.assertRaises(AppError):
                self._args(bad)

    def test_branched_session_stores_the_seeded_notice(self):
        validated = {
            **self._args([{"index": 1, "notices": ["fast_mode_unavailable"]}]),
            "persist": True,
            "session_id": None,
        }
        with tempfile.TemporaryDirectory(prefix="kassiber-fast-seed-") as tmp:
            data_root = Path(tmp) / "data"
            conn = open_db(str(data_root))
            try:
                workspace = core_accounts.create_workspace(conn, "Demo", commit=False)
                profile = core_accounts.create_profile(
                    conn, workspace["id"], "Main", "EUR", None, "generic", 365,
                    commit=True,
                )
                core_chat_history.set_history_mode(conn, "on")
                runtime = daemon.AiToolRuntime(
                    data_root=str(data_root),
                    runtime_config={},
                    main_thread_tasks=queue.Queue(),
                    maintenance_state={
                        "scope_workspace_id": workspace["id"],
                        "scope_profile_id": profile["id"],
                    },
                )
                results: list = []
                thread = threading.Thread(
                    target=lambda: results.append(
                        daemon._persist_ai_chat_exchange(
                            runtime,
                            {"name": "claude"},
                            validated,
                            finish_reason="stop",
                            assistant_content="Branched answer",
                            provenance={"notices": []},
                        )
                    )
                )
                thread.start()
                task = runtime.main_thread_tasks.get(timeout=5)
                task.response.put((True, task.callback(conn)))
                thread.join(timeout=5)
                stored = core_chat_history.get_session(conn, profile["id"], results[0])
            finally:
                conn.close()
        rows = [
            (message["role"], message["content"], (message["provenance"] or {}).get("notices"))
            for message in stored["messages"]
        ]
        self.assertEqual(
            rows,
            [
                ("user", "First question", None),
                ("assistant", "First answer", ["fast_mode_unavailable"]),
                ("user", "Second question", None),
                ("assistant", "Second answer", None),
                ("user", "Branched prompt", None),
                ("assistant", "Branched answer", []),
            ],
        )


class FastModeReferenceTest(unittest.TestCase):
    """The checked-in skill and the in-product AI both learn `--fast`."""

    def test_command_references_document_fast(self):
        from kassiber.ai.tools import read_skill_reference

        root = Path(__file__).resolve().parent.parent
        references = {
            "skills/kassiber/SKILL.md": (root / "skills/kassiber/SKILL.md").read_text(
                encoding="utf-8"
            ),
            "command-templates": read_skill_reference("command-templates")["content"],
        }
        for name, text in references.items():
            with self.subTest(reference=name):
                self.assertIn("`--fast`", text)
                for fact in ("higher rate", "standard speed", "fast_mode_unavailable"):
                    self.assertIn(fact, text)


if __name__ == "__main__":
    unittest.main()
