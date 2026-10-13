"""OpenAI-compatible HTTP transport against a scripted loopback provider.

Every request goes to a fake server on 127.0.0.1, so the real request
machinery (egress ledger, offline policy, redirects, headers, SSE parsing)
runs end to end without leaving the machine.
"""

from __future__ import annotations

import json
import queue
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

from kassiber.ai.client import OpenAICompatibleClient
from kassiber.ai.contracts import ResponsesRequestContext
from kassiber.errors import AppError


def sse(events: list[Any], *, done: bool = True) -> bytes:
    lines = [f"data: {json.dumps(event)}\n\n" for event in events]
    if done:
        lines.append("data: [DONE]\n\n")
    return "".join(lines).encode("utf-8")


class Reply:
    """One scripted answer: a status with a JSON body, an SSE body, or raw bytes."""

    def __init__(
        self,
        status: int = 200,
        *,
        json_body: Any = None,
        sse_events: list[Any] | None = None,
        raw: bytes | None = None,
        done: bool = True,
    ) -> None:
        self.status = status
        if sse_events is not None:
            self.body = sse(sse_events, done=done)
            self.content_type = "text/event-stream"
        elif raw is not None:
            self.body = raw
            self.content_type = "text/plain"
        else:
            self.body = json.dumps(json_body if json_body is not None else {}).encode("utf-8")
            self.content_type = "application/json"


def error_reply(status: int, message: str, **error: Any) -> Reply:
    return Reply(status, json_body={"error": {"message": message, **error}})


class _ProviderHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - http.server API
        self._answer()

    def do_POST(self) -> None:  # noqa: N802 - http.server API
        self._answer()

    def _answer(self) -> None:
        server: ScriptedProvider = self.server  # type: ignore[assignment]
        length = int(self.headers.get("content-length") or "0")
        raw = self.rfile.read(length) if length else b""
        body = json.loads(raw.decode("utf-8")) if raw else None
        with server.lock:
            server.requests.append(
                {
                    "method": self.command,
                    "path": self.path,
                    "body": body,
                    "authorization": self.headers.get("authorization"),
                }
            )
            queue = server.routes.get((self.command, self.path))
            reply = queue.pop(0) if queue else None
        if reply is None:
            reply = Reply(404, json_body={"detail": "Not Found"})
        self.send_response(reply.status)
        self.send_header("content-type", reply.content_type)
        self.send_header("content-length", str(len(reply.body)))
        self.end_headers()
        self.wfile.write(reply.body)

    def log_message(self, *args: Any) -> None:
        return


class ScriptedProvider(ThreadingHTTPServer):
    """Answers each (method, path) from its own queue; 404 when nothing is queued."""

    daemon_threads = True

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), _ProviderHandler)
        self.lock = threading.Lock()
        self.routes: dict[tuple[str, str], list[Reply]] = {}
        self.requests: list[dict[str, Any]] = []
        self._thread = threading.Thread(
            target=self.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True
        )
        self._thread.start()

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.server_port}/v1"

    def queue(self, method: str, path: str, *replies: Reply) -> None:
        with self.lock:
            self.routes.setdefault((method, f"/v1/{path}"), []).extend(replies)

    def bodies(self, path: str) -> list[Any]:
        return [
            request["body"]
            for request in self.requests
            if request["path"] == f"/v1/{path}"
        ]

    def stop(self) -> None:
        self.shutdown()
        self.server_close()


def responses_stream(
    output: list[dict[str, Any]],
    *,
    text: str | None = None,
    status: str = "completed",
    incomplete_reason: str | None = None,
) -> Reply:
    events: list[Any] = []
    if text:
        events.append({"type": "response.output_text.delta", "output_index": 0, "delta": text})
    response: dict[str, Any] = {"id": "resp_1", "status": status, "output": output}
    if incomplete_reason:
        response["incomplete_details"] = {"reason": incomplete_reason}
    terminal = "response.incomplete" if status == "incomplete" else "response.completed"
    events.append({"type": terminal, "response": response})
    return Reply(sse_events=events)


def message_item(text: str) -> dict[str, Any]:
    return {
        "type": "message",
        "role": "assistant",
        "status": "completed",
        "content": [{"type": "output_text", "text": text}],
    }


def user_context(text: str = "hello", *items: dict[str, Any]) -> ResponsesRequestContext:
    return ResponsesRequestContext(
        instructions=None,
        input_items=[{"type": "message", "role": "user", "content": text}, *items],
    )


class ProviderTestCase(unittest.TestCase):
    def setUp(self) -> None:
        self.provider = ScriptedProvider()
        self.addCleanup(self.provider.stop)

    def client(self, **kwargs: Any) -> OpenAICompatibleClient:
        return OpenAICompatibleClient(base_url=self.provider.base_url, **kwargs)

    def stream(self, client: OpenAICompatibleClient, **kwargs: Any) -> list[Any]:
        kwargs.setdefault("model", "test-model")
        kwargs.setdefault("context", user_context())
        return list(client.stream_chat(**kwargs))


class ResponsesReasoningReplayTest(ProviderTestCase):
    """Stateless (`store: false`) reasoning replay between tool rounds."""

    def test_effort_asks_for_encrypted_reasoning(self):
        self.provider.queue(
            "POST",
            "responses",
            responses_stream([message_item("ok")], text="ok"),
            responses_stream([message_item("ok")], text="ok"),
        )
        client = self.client()
        self.stream(client, options={"reasoning_effort": "high"})
        self.stream(client)
        with_effort, without_effort = self.provider.bodies("responses")
        self.assertIs(with_effort["store"], False)
        self.assertEqual(with_effort["reasoning"], {"effort": "high"})
        self.assertEqual(with_effort["include"], ["reasoning.encrypted_content"])
        self.assertNotIn("reasoning", without_effort)
        self.assertNotIn("include", without_effort)

    def test_only_encrypted_reasoning_items_are_replayed(self):
        self.provider.queue("POST", "responses", responses_stream([message_item("ok")]))
        dangling = {"type": "reasoning", "id": "rs_plain", "summary": []}
        empty = {"type": "reasoning", "id": "rs_empty", "summary": [], "encrypted_content": ""}
        encrypted = {
            "type": "reasoning",
            "id": "rs_sealed",
            "summary": [],
            "encrypted_content": "gAAAA-sealed",
        }
        call = {"type": "function_call", "call_id": "call_1", "name": "status", "arguments": "{}"}
        output = {"type": "function_call_output", "call_id": "call_1", "output": "{}"}
        self.stream(
            self.client(),
            context=user_context("hello", dangling, empty, encrypted, call, output),
            options={"reasoning_effort": "medium"},
        )
        (body,) = self.provider.bodies("responses")
        self.assertEqual(
            [item.get("id") or item.get("type") for item in body["input"]],
            ["message", "rs_sealed", "function_call", "function_call_output"],
        )

    def test_refused_include_is_retried_once_and_then_left_out(self):
        self.provider.queue(
            "POST",
            "responses",
            error_reply(400, "Unknown parameter: 'include'.", param="include"),
            responses_stream([message_item("ok")], text="ok"),
            responses_stream([message_item("again")], text="again"),
        )
        client = self.client()
        chunks = self.stream(client, options={"reasoning_effort": "low"})
        self.assertEqual("".join(c.delta.get("content", "") for c in chunks), "ok")
        self.stream(client, options={"reasoning_effort": "low"})
        refused, retried, later = self.provider.bodies("responses")
        self.assertIn("include", refused)
        self.assertNotIn("include", retried)
        # The effort itself was not refused, so it stays.
        self.assertEqual(retried["reasoning"], {"effort": "low"})
        # A later round of the same turn leaves include out from the start.
        self.assertNotIn("include", later)

    def test_include_is_retried_at_most_once(self):
        refusal = error_reply(400, "Unknown parameter: 'include'.", param="include")
        self.provider.queue("POST", "responses", refusal, refusal)
        with self.assertRaises(AppError) as raised:
            self.stream(self.client(), options={"reasoning_effort": "low"})
        self.assertEqual(raised.exception.code, "ai_request_invalid")
        self.assertEqual(len(self.provider.bodies("responses")), 2)

    def test_unrelated_refusal_is_not_retried(self):
        self.provider.queue(
            "POST",
            "responses",
            error_reply(400, "Invalid value for 'temperature'.", param="temperature"),
        )
        with self.assertRaises(AppError):
            self.stream(self.client(), options={"reasoning_effort": "low"})
        self.assertEqual(len(self.provider.bodies("responses")), 1)


class ReasoningRefusalTest(ProviderTestCase):
    """An endpoint that refuses the reasoning effort answers without it."""

    def _content(self, chunks):
        return "".join(chunk.delta.get("content", "") for chunk in chunks)

    def _notices(self, chunks):
        return [chunk.status_phase for chunk in chunks if chunk.status_phase]

    def test_refused_effort_is_retried_without_reasoning_and_noticed(self):
        refusals = {
            "openai": error_reply(
                400,
                "Unsupported parameter: 'reasoning.effort' is not supported with this model.",
                param="reasoning.effort",
                code="unsupported_parameter",
            ),
            "ollama": error_reply(400, '"llama3.2" does not support thinking'),
            "strict schema": Reply(
                422,
                json_body={
                    "detail": [
                        {
                            "type": "extra_forbidden",
                            "loc": ["body", "reasoning"],
                            "msg": "Extra inputs are not permitted",
                        }
                    ]
                },
            ),
        }
        for name, refusal in refusals.items():
            with self.subTest(endpoint=name):
                self.provider.requests.clear()
                self.provider.queue(
                    "POST",
                    "responses",
                    refusal,
                    responses_stream([message_item("ok")], text="ok"),
                    responses_stream([message_item("again")], text="again"),
                )
                client = self.client()
                chunks = self.stream(client, options={"reasoning_effort": "max"})
                self.assertEqual(self._notices(chunks), ["reasoning_effort_unavailable"])
                # The notice comes before the answer and carries no delta.
                self.assertIsNotNone(chunks[0].status_phase)
                self.assertEqual(chunks[0].delta, {})
                self.assertEqual(self._content(chunks), "ok")
                refused, retried = self.provider.bodies("responses")
                self.assertEqual(refused["reasoning"], {"effort": "high"})
                self.assertNotIn("reasoning", retried)
                self.assertNotIn("include", retried)
                # The next round of the turn sends no effort and says nothing again.
                later = self.stream(client, options={"reasoning_effort": "max"})
                self.assertEqual(self._notices(later), [])
                self.assertNotIn("reasoning", self.provider.bodies("responses")[2])

    def test_reasoning_is_retried_at_most_once(self):
        refusal = error_reply(400, "Unsupported parameter: 'reasoning.effort'.")
        self.provider.queue("POST", "responses", refusal, refusal)
        with self.assertRaises(AppError):
            self.stream(self.client(), options={"reasoning_effort": "low"})
        self.assertEqual(len(self.provider.bodies("responses")), 2)

    def test_refused_replayed_reasoning_is_dropped_without_a_notice(self):
        self.provider.queue(
            "POST",
            "responses",
            error_reply(400, "Invalid input item: reasoning items are not supported."),
            responses_stream([message_item("ok")], text="ok"),
        )
        sealed = {
            "type": "reasoning",
            "id": "rs_1",
            "summary": [],
            "encrypted_content": "sealed",
        }
        chunks = self.stream(self.client(), context=user_context("hello", sealed))
        # No effort was asked for, so nothing was taken from the user.
        self.assertEqual(self._notices(chunks), [])
        refused, retried = self.provider.bodies("responses")
        self.assertIn("rs_1", [item.get("id") for item in refused["input"]])
        self.assertNotIn("rs_1", [item.get("id") for item in retried["input"]])

    def test_other_failures_naming_reasoning_are_not_retried(self):
        for status in (401, 404, 429, 500):
            with self.subTest(status=status):
                self.provider.requests.clear()
                self.provider.queue(
                    "POST",
                    "responses",
                    error_reply(status, "reasoning effort trouble"),
                )
                with self.assertRaises(AppError):
                    self.stream(self.client(), options={"reasoning_effort": "low"})
                self.assertEqual(len(self.provider.bodies("responses")), 1)

    def test_blocking_chat_records_the_notice(self):
        self.provider.queue(
            "POST",
            "responses",
            error_reply(400, "Unsupported parameter: 'reasoning.effort'."),
            Reply(json_body={"status": "completed", "output": [message_item("ok")]}),
        )
        client = self.client()
        result = client.chat(
            messages=[{"role": "user", "content": "hello"}],
            model="test-model",
            options={"reasoning_effort": "medium"},
        )
        self.assertEqual(result["content"], "ok")
        self.assertEqual(client.notices, ["reasoning_effort_unavailable"])


class _Out:
    def __init__(self) -> None:
        self.written: list[dict[str, Any]] = []

    def write(self, payload: dict[str, Any]) -> None:
        self.written.append(payload)

    def kinds(self, kind: str) -> list[dict[str, Any]]:
        return [payload["data"] for payload in self.written if payload.get("kind") == kind]


def _runtime() -> Any:
    from kassiber import daemon

    return daemon.AiToolRuntime(
        data_root="/not-used",
        runtime_config={},
        main_thread_tasks=queue.Queue(),
        maintenance_state={"egress_after_id": 0},
    )


class DaemonReasoningNoticeTest(ProviderTestCase):
    """The refused-effort notice reaches the stream, the result and history."""

    def test_notice_is_streamed_and_kept_in_the_answer_provenance(self):
        from kassiber import daemon

        self.provider.queue(
            "POST",
            "responses",
            error_reply(400, "Unsupported parameter: 'reasoning.effort'."),
            responses_stream([message_item("ok")], text="ok"),
        )
        out = _Out()
        runtime = _runtime()
        turn = daemon._stream_ai_chat_tool_turn(
            "req-1",
            self.client(),
            {"model": "test-model", "options": {"reasoning_effort": "high"}},
            user_context(),
            [],
            out,
            threading.Event(),
        )
        self.assertEqual(turn.notices, ("reasoning_effort_unavailable",))
        self.assertEqual(turn.content, "ok")
        self.assertIn(
            {
                "phase": "reasoning_effort_unavailable",
                "label": "Reasoning effort unavailable; answering without it",
            },
            out.kinds("ai.chat.status"),
        )
        self.assertEqual(
            [delta["delta"] for delta in out.kinds("ai.chat.delta")], [{"content": "ok"}]
        )
        daemon._ai_chat_notices(runtime).extend(turn.notices)
        terminal_out = _Out()
        daemon._write_ai_chat_terminal(
            terminal_out,
            "req-1",
            {"name": "http", "kind": "local"},
            {
                "model": "test-model",
                "persist": False,
                "messages": [{"role": "user", "content": "hello"}],
            },
            "stop",
            runtime,
            assistant_content="ok",
        )
        (terminal,) = terminal_out.kinds("ai.chat")
        self.assertEqual(terminal["provenance"]["notices"], ["reasoning_effort_unavailable"])

    def test_branched_chat_keeps_the_notice_on_a_copied_answer(self):
        from kassiber import daemon

        validated = daemon._ai_chat_args(
            {
                "model": "test-model",
                "messages": [
                    {"role": "user", "content": "First"},
                    {"role": "assistant", "content": "Answer"},
                    {"role": "user", "content": "Branched"},
                ],
                "seed_history": True,
                "seed_notices": [
                    {"index": 1, "notices": ["reasoning_effort_unavailable", "made_up"]}
                ],
            }
        )
        self.assertEqual(validated["seed_notices"], {1: ["reasoning_effort_unavailable"]})


if __name__ == "__main__":
    unittest.main()
