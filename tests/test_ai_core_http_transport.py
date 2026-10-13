"""OpenAI-compatible HTTP transport against a scripted loopback provider.

Every request goes to a fake server on 127.0.0.1, so the real request
machinery (egress ledger, offline policy, redirects, headers, SSE parsing)
runs end to end without leaving the machine.
"""

from __future__ import annotations

import json
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


if __name__ == "__main__":
    unittest.main()
