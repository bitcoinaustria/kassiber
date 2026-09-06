from __future__ import annotations

import json
import io
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from kassiber.ai.broker_client import BrokerAIClient
from kassiber.ai.contracts import ResponsesRequestContext, cli_provider_for_locator
from kassiber.daemon import ActiveAiChats
from kassiber.errors import AppError


FAKE_BROKER = """
import json, sys, time
request = json.loads(sys.stdin.readline())
if request["command"] == "status":
    print(json.dumps({"type":"result","data":[{"provider":"codex","state":"ready","models":[]}]}), flush=True)
elif request["command"] == "models":
    print(json.dumps({"type":"result","data":[{"id":"model-a"}]}), flush=True)
elif request.get("model") == "wait":
    time.sleep(30)
elif request.get("model") == "sensitive":
    assert request["options"] == {"sensitive_context": True, "reasoning_effort": "high"}
    assert request["tools"] == []
    print(json.dumps({"type":"delta","content":"selected only"}), flush=True)
    print(json.dumps({"type":"done","finish_reason":"stop","provider_session_id":"must-not-retain"}), flush=True)
elif request.get("model") == "tool-call":
    print(json.dumps({"type":"tool_call","call_id":"native-call-1","name":"ui_reports_tax_summary","arguments":{}}), flush=True)
    reply = json.loads(sys.stdin.readline())
    assert reply["command"] == "tool_results"
    assert reply["results"][0]["call_id"] == "native-call-1"
    print(json.dumps({"type":"delta","content":"report ready"}), flush=True)
    print(json.dumps({"type":"done","finish_reason":"stop","provider_session_id":"native-session-1"}), flush=True)
else:
    print(json.dumps({"type":"delta","content":"hello "}), flush=True)
    print(json.dumps({"type":"delta","content":"world"}), flush=True)
    print(json.dumps({"type":"done","finish_reason":"stop","provider_session_id":"native-session-1"}), flush=True)
"""


class BrokerClientTest(unittest.TestCase):
    def test_sensitive_windows_provider_fails_before_resolution_or_spawn(self):
        client = BrokerAIClient(locator='claude-cli://default')
        with patch('kassiber.ai.broker_client.os.name', 'nt'), \
             patch('kassiber.ai.broker_client._node_executable') as resolve, \
             patch('kassiber.ai.broker_client.subprocess.Popen') as spawn:
            with self.assertRaises(AppError) as error:
                list(client.stream_chat(messages=[{'role':'user','content':'private fixture'}], model='test', options={'sensitive_context':True}))
            self.assertEqual(error.exception.code, 'ai_sensitive_provider_unavailable')
            resolve.assert_not_called()
            spawn.assert_not_called()

    def test_cancel_is_latched_before_resolution_spawn_registration_and_send(self):
        for stage in ('before', 'resolution', 'spawn', 'serialization'):
            with self.subTest(stage=stage):
                client = BrokerAIClient(locator='claude-cli://default')
                sent = []
                stdin = SimpleNamespace(write=sent.append, flush=lambda: None, close=lambda: None)
                process = SimpleNamespace(stdin=stdin, stdout=io.StringIO('{"type":"done","finish_reason":"stop"}\n'),
                    poll=lambda: 0, wait=lambda **kw: 0, pid=0)
                def resolve():
                    if stage == 'resolution':
                        client.cancel()
                    return sys.executable
                def spawn(*args, **kwargs):
                    if stage == 'spawn':
                        client.cancel()
                    return process
                encode = json.dumps
                def serialize(value):
                    if stage == 'serialization':
                        client.cancel()
                    return encode(value)
                if stage == 'before':
                    client.cancel()
                with patch('kassiber.ai.broker_client._node_executable', resolve), \
                     patch('kassiber.ai.broker_client._broker_script', return_value=Path(__file__)), \
                     patch('kassiber.ai.broker_client.subprocess.Popen', side_effect=spawn) as popen, \
                     patch('kassiber.ai.broker_client.json.dumps', serialize):
                    with self.assertRaises(AppError) as raised:
                        list(client.stream_chat(messages=[{'role':'user','content':'selected sensitive fixture'}],
                            model='test', options={'sensitive_context':True}))
                    self.assertEqual(raised.exception.code, 'ai_cancelled')
                    self.assertEqual(sent, [])
                    self.assertIsNone(client._process)
                    if stage in ('before', 'resolution'):
                        popen.assert_not_called()

    def test_cancellation_does_not_wait_for_a_blocked_prompt_writer(self):
        client = BrokerAIClient(locator='codex-cli://default')
        writing, signalled = threading.Event(), threading.Event()
        outcome = []
        def write(_):
            writing.set()
            self.assertTrue(signalled.wait(2), 'cancel was blocked by the pipe writer')
            raise BrokenPipeError()
        process = SimpleNamespace(stdin=SimpleNamespace(write=write, flush=lambda:None, close=lambda:None),
            stdout=io.StringIO(''), poll=lambda: 0 if signalled.is_set() else None, wait=lambda **kw:0, pid=0)
        def consume():
            try:
                list(client.stream_chat(messages=[{'role':'user','content':'synthetic'}], model='test', options={'sensitive_context':True}))
            except AppError as error:
                outcome.append(error.code)
        with patch('kassiber.ai.broker_client._node_executable', return_value=sys.executable), \
             patch('kassiber.ai.broker_client._broker_script', return_value=Path(__file__)), \
             patch('kassiber.ai.broker_client.subprocess.Popen', return_value=process), \
             patch.object(client, '_signal_group', side_effect=lambda *_: signalled.set()):
            reader = threading.Thread(target=consume)
            reader.start()
            self.assertTrue(writing.wait(2))
            started = time.monotonic()
            client.cancel()
            self.assertLess(time.monotonic() - started, .5)
            reader.join(2)
            self.assertFalse(reader.is_alive())
        self.assertEqual(outcome, ['ai_cancelled'])
        self.assertIsNone(client._process)

    def test_sensitive_context_crosses_broker_and_cannot_retain_session(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-broker-test-") as tmp:
            script = self._fake_broker(Path(tmp))
            with patch.dict("os.environ", {
                "KASSIBER_AI_BROKER_NODE": sys.executable,
                "KASSIBER_AI_PROVIDER_BROKER": str(script),
            }):
                client = BrokerAIClient(locator="claude-cli://default")
                chunks = list(client.stream_chat(
                    messages=[{"role": "user", "content": "selected fixture"}],
                    model="sensitive", options={"sensitive_context": True, "reasoning_effort": "high"},
                ))
                self.assertEqual(chunks[-1].finish_reason, "stop")
                self.assertIsNone(client.last_provider_session_id)

    def test_sensitive_mode_rejects_resume_tools_and_nonboolean_before_spawn(self):
        client = BrokerAIClient(locator="codex-cli://default")
        for options, tools in [
            ({"sensitive_context": "true"}, []),
            ({"sensitive_context": True, "provider_session_id": "old"}, []),
            ({"sensitive_context": True}, [{"name": "tool"}]),
        ]:
            with self.subTest(options=options), patch("subprocess.Popen") as spawn:
                with self.assertRaises(AppError):
                    list(client.stream_chat(model="default", options=options, tools=tools))
                spawn.assert_not_called()

    def test_sensitive_mode_rejects_hostile_broker_tool_event(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-broker-test-") as tmp:
            script = self._fake_broker(Path(tmp))
            with patch.dict("os.environ", {
                "KASSIBER_AI_BROKER_NODE": sys.executable,
                "KASSIBER_AI_PROVIDER_BROKER": str(script),
            }):
                client = BrokerAIClient(locator="codex-cli://default")
                with self.assertRaises(AppError) as raised:
                    list(client.stream_chat(model="tool-call", options={"sensitive_context": True}))
                self.assertEqual(raised.exception.code, "ai_request_invalid")
                self.assertFalse(client._pending_call_ids)

    def _fake_broker(self, root: Path) -> Path:
        script = root / "fake_broker.py"
        script.write_text(FAKE_BROKER, encoding="utf-8")
        return script

    def test_runtime_status_and_model_discovery_translate_jsonl(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-broker-test-") as tmp:
            script = self._fake_broker(Path(tmp))
            with patch.dict(
                "os.environ",
                {
                    "KASSIBER_AI_BROKER_NODE": sys.executable,
                    "KASSIBER_AI_PROVIDER_BROKER": str(script),
                },
            ):
                self.assertEqual(
                    BrokerAIClient.runtime_status()[0]["state"], "ready"
                )
                client = BrokerAIClient(locator="codex-cli://default")
                self.assertEqual(client.list_models(), [{"id": "model-a"}])

    def test_native_provider_locators_share_one_registry(self):
        self.assertEqual(cli_provider_for_locator(" CODEX-CLI://DEFAULT "), "codex")
        self.assertEqual(cli_provider_for_locator("claude-cli://default"), "claude")
        self.assertEqual(cli_provider_for_locator("opencode-cli://default"), "opencode")
        self.assertIsNone(cli_provider_for_locator("https://example.test/v1"))

    def test_streaming_and_provider_session_continuation_metadata(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-broker-test-") as tmp:
            script = self._fake_broker(Path(tmp))
            with patch.dict(
                "os.environ",
                {
                    "KASSIBER_AI_BROKER_NODE": sys.executable,
                    "KASSIBER_AI_PROVIDER_BROKER": str(script),
                },
            ):
                client = BrokerAIClient(locator="opencode-cli://default")
                chunks = list(
                    client.stream_chat(
                        messages=[{"role": "user", "content": "hi"}],
                        model="provider/model",
                        options={"provider_session_id": "native-session-0"},
                    )
                )
                self.assertEqual(
                    "".join(chunk.delta.get("content", "") for chunk in chunks),
                    "hello world",
                )
                self.assertEqual(client.last_provider_session_id, "native-session-1")
                self.assertEqual(chunks[-1].finish_reason, "stop")

    def test_kassiber_typed_tools_cross_the_chat_only_broker(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-broker-test-") as tmp:
            script = self._fake_broker(Path(tmp))
            with patch.dict(
                "os.environ",
                {
                    "KASSIBER_AI_BROKER_NODE": sys.executable,
                    "KASSIBER_AI_PROVIDER_BROKER": str(script),
                },
            ):
                client = BrokerAIClient(locator="codex-cli://default")
                chunks = list(
                    client.stream_chat(
                        model="tool-call",
                        tools=[
                            {
                                "type": "function",
                                "name": "ui_reports_tax_summary",
                                "description": "Read the tax summary",
                                "parameters": {"type": "object", "properties": {}},
                            }
                        ],
                        tool_choice="auto",
                        context=ResponsesRequestContext(
                            instructions="You are Kassiber's assistant.",
                            input_items=[
                                {
                                    "type": "message",
                                    "role": "user",
                                    "content": "Generate my report",
                                }
                            ],
                        ),
                    )
                )

                call = chunks[-1].delta["tool_calls"][0]
                continued = list(
                    client.stream_chat(
                        model="tool-call",
                        tools=[
                            {
                                "type": "function",
                                "name": "ui_reports_tax_summary",
                                "description": "Read the tax summary",
                                "parameters": {"type": "object", "properties": {}},
                            }
                        ],
                        context=ResponsesRequestContext(
                            instructions="You are Kassiber's assistant.",
                            input_items=[
                                {
                                    "type": "function_call_output",
                                    "call_id": call["id"],
                                    "output": '{"ok":true}',
                                }
                            ],
                        ),
                    )
                )

        self.assertEqual(call["function"]["name"], "ui_reports_tax_summary")
        self.assertEqual(chunks[-1].finish_reason, "tool_calls")
        self.assertEqual(
            "".join(chunk.delta.get("content", "") for chunk in continued),
            "report ready",
        )
        self.assertEqual(continued[-1].finish_reason, "stop")

    def test_cancel_terminates_active_broker_process_group(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-broker-test-") as tmp:
            script = self._fake_broker(Path(tmp))
            with patch.dict(
                "os.environ",
                {
                    "KASSIBER_AI_BROKER_NODE": sys.executable,
                    "KASSIBER_AI_PROVIDER_BROKER": str(script),
                },
            ):
                client = BrokerAIClient(locator="codex-cli://default")
                finished = threading.Event()

                def consume() -> None:
                    try:
                        list(
                            client.stream_chat(
                                messages=[{"role": "user", "content": "wait"}],
                                model="wait",
                            )
                        )
                    except AppError:
                        pass
                    finally:
                        finished.set()

                thread = threading.Thread(target=consume, daemon=True)
                thread.start()
                deadline = time.monotonic() + 2
                while client._process is None and time.monotonic() < deadline:
                    time.sleep(0.01)
                client.cancel()
                self.assertTrue(finished.wait(3), "cancel did not stop the broker")

    def test_missing_node_reports_unavailable_without_credentials(self):
        with patch("kassiber.ai.broker_client._node_executable", return_value=None):
            with self.assertRaises(AppError) as raised:
                BrokerAIClient.runtime_status()
        self.assertEqual(raised.exception.code, "ai_unavailable")
        self.assertNotIn("token", str(raised.exception).lower())

    def test_provider_resume_cursor_is_bound_to_the_visible_chat_branch(self):
        registry = ActiveAiChats()
        registry.remember_provider_session(
            chat_session_id="chat-1",
            provider_name="codex",
            provider_session_id="native-1",
            history_fingerprint="branch-a",
        )
        self.assertEqual(
            registry.provider_session(
                chat_session_id="chat-1",
                provider_name="codex",
                history_fingerprint="branch-a",
            ),
            "native-1",
        )
        self.assertIsNone(
            registry.provider_session(
                chat_session_id="chat-1",
                provider_name="codex",
                history_fingerprint="edited-branch",
            )
        )


if __name__ == "__main__":
    unittest.main()
