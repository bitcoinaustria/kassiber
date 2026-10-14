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


def chunk(delta: dict[str, Any] | None = None, *, finish: str | None = None, **extra: Any) -> dict[str, Any]:
    """One Chat Completions stream chunk with a single choice."""

    return {
        "id": "chatcmpl-1",
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": delta or {}, "finish_reason": finish}],
        **extra,
    }


def chat_stream(*chunks: dict[str, Any], done: bool = True, comments: bool = False) -> Reply:
    body = sse(list(chunks), done=done)
    if comments:
        # OpenRouter keeps idle streams open with SSE comments.
        body = b": OPENROUTER PROCESSING\n\n" + body.replace(
            b"\n\ndata:", b"\n\n: OPENROUTER PROCESSING\n\ndata:", 1
        )
    reply = Reply(raw=body)
    reply.content_type = "text/event-stream"
    return reply


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


class ResponsesIncompleteTest(ProviderTestCase):
    """`response.incomplete` ends a turn the way a length stop does."""

    def _terminal(self, chunks):
        terminal = [chunk for chunk in chunks if chunk.response_output is not None]
        self.assertEqual(len(terminal), 1)
        return terminal[0]

    def test_cut_off_answer_finishes_with_length_or_content_filter(self):
        for reason, finish in (
            ("max_output_tokens", "length"),
            ("content_filter", "content_filter"),
        ):
            with self.subTest(reason=reason):
                self.provider.queue(
                    "POST",
                    "responses",
                    responses_stream(
                        [message_item("Partial answ")],
                        text="Partial answ",
                        status="incomplete",
                        incomplete_reason=reason,
                    ),
                )
                chunks = self.stream(self.client())
                self.assertEqual(
                    "".join(chunk.delta.get("content", "") for chunk in chunks),
                    "Partial answ",
                )
                terminal = self._terminal(chunks)
                self.assertEqual(terminal.finish_reason, finish)
                self.assertEqual(terminal.response_output[0]["type"], "message")

    def test_cut_off_function_call_is_neither_run_nor_replayed(self):
        finished = {
            "type": "function_call",
            "status": "completed",
            "call_id": "call_done",
            "name": "status",
            "arguments": "{}",
        }
        cut_off = {
            "type": "function_call",
            "status": "incomplete",
            "call_id": "call_cut",
            "name": "ui_journals_process",
            "arguments": '{"wallet": "va',
        }
        self.provider.queue(
            "POST",
            "responses",
            Reply(
                sse_events=[
                    {
                        "type": "response.output_item.added",
                        "output_index": 1,
                        "item": {**cut_off, "arguments": ""},
                    },
                    {
                        "type": "response.incomplete",
                        "response": {
                            "status": "incomplete",
                            "incomplete_details": {"reason": "max_output_tokens"},
                            "output": [finished, cut_off],
                        },
                    },
                ]
            ),
        )
        terminal = self._terminal(self.stream(self.client()))
        self.assertEqual(terminal.finish_reason, "length")
        self.assertEqual(
            [call["id"] for call in terminal.delta["tool_calls"]], ["call_done"]
        )
        self.assertEqual(
            [item.get("call_id") for item in terminal.response_output], ["call_done"]
        )

    def test_stream_cut_off_mid_call_withdraws_the_call(self):
        self.provider.queue(
            "POST",
            "responses",
            Reply(
                sse_events=[
                    {
                        "type": "response.output_item.added",
                        "output_index": 0,
                        "item": {"type": "function_call", "call_id": "c1", "name": "status", "arguments": ""},
                    },
                    {"type": "response.function_call_arguments.delta", "output_index": 0, "delta": '{"a'},
                ],
                done=False,
            ),
        )
        chunks = self.stream(self.client())
        self.assertEqual(chunks[-2].delta["tool_calls"][0]["id"], "c1")
        self.assertEqual(chunks[-1].delta, {"tool_calls": []})

    def test_a_call_without_a_status_in_an_incomplete_answer_is_unfinished(self):
        # Some endpoints leave out each item's status. Under an incomplete
        # response, such a call may be the one that was cut off.
        cut_off = {
            "type": "function_call",
            "call_id": "call_cut",
            "name": "ui_journals_process",
            "arguments": '{"wallet": "va',
        }
        self.provider.queue(
            "POST",
            "responses",
            Reply(
                sse_events=[
                    {"type": "response.output_item.done", "output_index": 0, "item": cut_off},
                    {
                        "type": "response.incomplete",
                        "response": {
                            "status": "incomplete",
                            "incomplete_details": {"reason": "max_output_tokens"},
                            "output": [cut_off],
                        },
                    },
                ]
            ),
        )
        terminal = self._terminal(self.stream(self.client()))
        self.assertEqual(terminal.finish_reason, "length")
        self.assertEqual(terminal.delta["tool_calls"], [])
        self.assertEqual(terminal.response_output, [])

    def test_a_body_without_stream_events_fails(self):
        self.provider.queue("POST", "responses", Reply(raw=b'{"error": "Unexpected endpoint"}'))
        with self.assertRaises(AppError) as raised:
            self.stream(self.client(api_format="responses"))
        self.assertEqual(raised.exception.details["reason"], "no_stream_events")

    def test_blocking_chat_reports_length(self):
        self.provider.queue(
            "POST",
            "responses",
            Reply(
                json_body={
                    "status": "incomplete",
                    "incomplete_details": {"reason": "max_output_tokens"},
                    "output": [
                        message_item("Partial"),
                        {
                            "type": "function_call",
                            "status": "in_progress",
                            "call_id": "call_cut",
                            "name": "status",
                            "arguments": "{",
                        },
                    ],
                }
            ),
        )
        result = self.client().chat(
            messages=[{"role": "user", "content": "hello"}], model="test-model"
        )
        self.assertEqual(result["finish_reason"], "length")
        self.assertEqual(result["content"], "Partial")
        self.assertNotIn("tool_calls", result)


class ChatCompletionsStreamTest(ProviderTestCase):
    """Chat Completions emits the same events as the Responses path."""

    def client(self, **kwargs: Any) -> OpenAICompatibleClient:
        kwargs.setdefault("api_format", "chat_completions")
        return super().client(**kwargs)

    def _text(self, chunks, key="content"):
        return "".join(chunk.delta.get(key, "") for chunk in chunks)

    def _terminal(self, chunks):
        terminal = [c for c in chunks if c.response_output is not None]
        self.assertEqual(len(terminal), 1)
        self.assertIs(chunks[-1], terminal[0])
        return terminal[0]

    def test_streams_text_with_keep_alive_comments_and_a_usage_chunk(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            chat_stream(
                chunk({"role": "assistant", "content": ""}),
                chunk({"content": "Hel"}),
                chunk({"content": "lo"}),
                chunk({}, finish="stop"),
                # OpenAI's usage chunk has no choices; OpenRouter repeats the
                # finish reason on it.
                {"id": "chatcmpl-1", "choices": [], "usage": {"total_tokens": 9}},
                comments=True,
            ),
        )
        context = ResponsesRequestContext(
            instructions="Be brief.",
            input_items=[{"type": "message", "role": "user", "content": "hi"}],
        )
        chunks = self.stream(self.client(), context=context)
        self.assertEqual(self._text(chunks), "Hello")
        terminal = self._terminal(chunks)
        self.assertEqual(terminal.finish_reason, "stop")
        self.assertEqual(terminal.delta, {})
        self.assertEqual(
            terminal.response_output,
            [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "Hello"}],
                }
            ],
        )
        (body,) = self.provider.bodies("chat/completions")
        self.assertEqual(
            body,
            {
                "model": "test-model",
                "messages": [
                    {"role": "system", "content": "Be brief."},
                    {"role": "user", "content": "hi"},
                ],
                "stream": True,
                # OpenAI stores Chat Completions by default for new accounts.
                "store": False,
            },
        )
        # Nothing went to the Responses endpoint.
        self.assertEqual(self.provider.bodies("responses"), [])

    def test_reasoning_fields_stay_reasoning(self):
        for field in ("reasoning_content", "reasoning"):
            with self.subTest(field=field):
                self.provider.queue(
                    "POST",
                    "chat/completions",
                    chat_stream(
                        chunk({field: "Let me think. "}),
                        chunk({field: "Done."}),
                        chunk({"content": "Answer"}),
                        chunk({}, finish="stop"),
                    ),
                )
                chunks = self.stream(self.client())
                self.assertEqual(self._text(chunks, "reasoning"), "Let me think. Done.")
                self.assertEqual(self._text(chunks), "Answer")
                reasoning_item = self._terminal(chunks).response_output[0]
                self.assertEqual(reasoning_item["type"], "reasoning")
                self.assertEqual(
                    reasoning_item["chat_completions_replay"], {"field": field}
                )

    def test_tool_calls_split_by_index_with_parallel_calls(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            chat_stream(
                chunk({"role": "assistant", "content": None}),
                chunk(
                    {
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call_a",
                                "type": "function",
                                "function": {"name": "status", "arguments": ""},
                            }
                        ]
                    }
                ),
                chunk({"tool_calls": [{"index": 0, "function": {"arguments": '{"ver'}}]}),
                chunk(
                    {
                        "tool_calls": [
                            {
                                "index": 1,
                                "id": "call_b",
                                "type": "function",
                                "function": {"name": "ui_wallets_list", "arguments": "{}"},
                            }
                        ]
                    }
                ),
                chunk({"tool_calls": [{"index": 0, "function": {"arguments": 'bose": true}'}}]}),
                chunk({}, finish="tool_calls"),
            ),
        )
        chunks = self.stream(
            self.client(),
            tools=[{"type": "function", "name": "status", "parameters": {"type": "object"}, "strict": False}],
            tool_choice="auto",
        )
        terminal = self._terminal(chunks)
        self.assertEqual(terminal.finish_reason, "tool_calls")
        self.assertEqual(
            terminal.delta["tool_calls"],
            [
                {
                    "id": "call_a",
                    "type": "function",
                    "function": {"name": "status", "arguments": '{"verbose": true}'},
                },
                {
                    "id": "call_b",
                    "type": "function",
                    "function": {"name": "ui_wallets_list", "arguments": "{}"},
                },
            ],
        )
        self.assertEqual(
            [(item["type"], item.get("call_id")) for item in terminal.response_output],
            [("function_call", "call_a"), ("function_call", "call_b")],
        )
        (body,) = self.provider.bodies("chat/completions")
        # Nested function tools, no Responses-only `strict`.
        self.assertEqual(
            body["tools"],
            [
                {
                    "type": "function",
                    "function": {"name": "status", "parameters": {"type": "object"}},
                }
            ],
        )
        self.assertEqual(body["tool_choice"], "auto")

    def test_calls_without_an_index_are_kept_apart_by_id(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            chat_stream(
                chunk({"tool_calls": [{"id": "x1", "function": {"name": "a", "arguments": "{}"}}]}),
                chunk({"tool_calls": [{"id": "x2", "function": {"name": "b", "arguments": "{"}}]}),
                chunk({"tool_calls": [{"function": {"arguments": "}"}}]}),
                # A server reusing index 0 for a second call with a new id.
                chunk({"tool_calls": [{"index": 0, "id": "x3", "function": {"name": "c", "arguments": "{}"}}]}),
                chunk({}, finish="stop"),
            ),
        )
        terminal = self._terminal(self.stream(self.client()))
        # tool calls with finish_reason "stop" still finish as tool_calls.
        self.assertEqual(terminal.finish_reason, "tool_calls")
        self.assertEqual(
            [(c["id"], c["function"]["name"], c["function"]["arguments"]) for c in terminal.delta["tool_calls"]],
            [("x1", "a", "{}"), ("x2", "b", "{}"), ("x3", "c", "{}")],
        )

    def test_two_indices_with_one_id_fail_closed_instead_of_merging(self):
        # Merged, these halves would form one runnable call: {"limit":5}.
        self.provider.queue(
            "POST",
            "chat/completions",
            chat_stream(
                chunk({"tool_calls": [{"index": 0, "id": "dup", "function": {"name": "ui_transactions_list", "arguments": '{"limit":'}}]}),
                chunk({"tool_calls": [{"index": 1, "id": "dup", "function": {"arguments": "5}"}}]}),
                chunk({}, finish="tool_calls"),
            ),
        )
        chunks: list[Any] = []
        with self.assertRaises(AppError) as raised:
            for item in self.client().stream_chat(model="m", context=user_context()):
                chunks.append(item)
        self.assertEqual(raised.exception.code, "ai_request_invalid")
        self.assertEqual(raised.exception.details["reason"], "duplicate_tool_call_id")
        self.assertTrue(all(c.response_output is None for c in chunks))
        self.assertFalse(
            any('{"limit":5}' == call["function"]["arguments"] for c in chunks for call in c.delta.get("tool_calls", []))
        )

    def test_finish_reasons(self):
        cases = [
            ("length", "length"),
            ("content_filter", "content_filter"),
            ("function_call", "tool_calls"),
            (None, "stop"),
        ]
        for finish, expected in cases:
            with self.subTest(finish=finish):
                self.provider.queue(
                    "POST",
                    "chat/completions",
                    chat_stream(chunk({"content": "partial"}), chunk({}, finish=finish)),
                )
                self.assertEqual(self._terminal(self.stream(self.client())).finish_reason, expected)

    def test_cut_off_tool_calls_are_dropped(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            chat_stream(
                chunk({"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "status", "arguments": '{"a'}}]}),
                chunk({}, finish="length"),
            ),
        )
        terminal = self._terminal(self.stream(self.client()))
        self.assertEqual(terminal.finish_reason, "length")
        # The streamed call is withdrawn so the daemon runs nothing.
        self.assertEqual(terminal.delta, {"tool_calls": []})
        self.assertEqual(terminal.response_output, [])

    def test_stream_cut_off_without_finish_has_no_terminal_chunk(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            chat_stream(chunk({"content": "half"}), done=False),
        )
        chunks = self.stream(self.client())
        self.assertEqual(self._text(chunks), "half")
        self.assertTrue(all(c.response_output is None for c in chunks))

    def test_stream_cut_off_mid_call_withdraws_the_call(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            chat_stream(
                chunk({"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "status", "arguments": '{"a'}}]}),
                done=False,
            ),
        )
        chunks = self.stream(self.client())
        self.assertEqual(chunks[-1].delta, {"tool_calls": []})
        self.assertIsNone(chunks[-1].response_output)

    def test_error_payloads_are_mapped(self):
        cases = [
            # OpenRouter after the response was committed.
            (
                chunk({"content": ""}, finish="error", error={"code": "server_error", "message": "Provider disconnected"}),
                "ai_unavailable",
            ),
            ({"error": {"code": 429, "message": "Rate limit exceeded"}}, "ai_rate_limited"),
            # vLLM-style error object.
            ({"object": "error", "message": "bad", "type": "BadRequestError", "code": 400}, "ai_request_invalid"),
        ]
        for event, code in cases:
            with self.subTest(code=code):
                self.provider.queue("POST", "chat/completions", chat_stream(event))
                with self.assertRaises(AppError) as raised:
                    self.stream(self.client())
                self.assertEqual(raised.exception.code, code)

    def test_an_answer_with_no_stream_events_fails(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            Reply(raw=b'{"error":"Unexpected endpoint or method."}'),
        )
        with self.assertRaises(AppError) as raised:
            self.stream(self.client())
        self.assertEqual(raised.exception.details["reason"], "no_stream_events")


class ChatCompletionsRequestTest(ProviderTestCase):
    """What a Chat Completions request carries, and its retries."""

    def client(self, **kwargs: Any) -> OpenAICompatibleClient:
        kwargs.setdefault("api_format", "chat_completions")
        return super().client(**kwargs)

    def _answer(self, text: str = "ok") -> Reply:
        return chat_stream(chunk({"content": text}), chunk({}, finish="stop"))

    def test_two_round_tool_loop_replays_calls_outputs_and_reasoning(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            chat_stream(
                chunk({"reasoning_content": "Need the status."}),
                chunk({"content": "Checking."}),
                chunk({"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "status", "arguments": "{}"}}]}),
                chunk({"tool_calls": [{"index": 1, "id": "call_2", "function": {"name": "health", "arguments": "{}"}}]}),
                chunk({}, finish="tool_calls"),
            ),
            self._answer("All good."),
        )
        client = self.client()
        context = ResponsesRequestContext(
            instructions="Kassiber prompt",
            input_items=[
                {"type": "message", "role": "user", "content": "earlier"},
                {"type": "message", "role": "assistant", "content": "earlier answer"},
                {"type": "message", "role": "user", "content": "status?"},
            ],
        )
        first = self.stream(client, context=context, tools=[{"type": "function", "name": "status"}])
        terminal = [c for c in first if c.response_output is not None][0]
        # What the daemon's tool loop does between rounds.
        context.input_items.extend(terminal.response_output)
        for call in terminal.delta["tool_calls"]:
            context.input_items.append(
                {"type": "function_call_output", "call_id": call["id"], "output": '{"ok":true}'}
            )
        second = self.stream(client, context=context, tools=[{"type": "function", "name": "status"}])
        self.assertEqual("".join(c.delta.get("content", "") for c in second), "All good.")
        _round_one, round_two = self.provider.bodies("chat/completions")
        self.assertEqual(
            round_two["messages"],
            [
                {"role": "system", "content": "Kassiber prompt"},
                {"role": "user", "content": "earlier"},
                {"role": "assistant", "content": "earlier answer"},
                {"role": "user", "content": "status?"},
                {
                    "role": "assistant",
                    "content": "Checking.",
                    # Replayed under the field it arrived in (DeepSeek needs it).
                    "reasoning_content": "Need the status.",
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {"name": "status", "arguments": "{}"}},
                        {"id": "call_2", "type": "function", "function": {"name": "health", "arguments": "{}"}},
                    ],
                },
                {"role": "tool", "tool_call_id": "call_1", "content": '{"ok":true}'},
                {"role": "tool", "tool_call_id": "call_2", "content": '{"ok":true}'},
            ],
        )

    def test_openrouter_reasoning_details_are_merged_and_replayed(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            chat_stream(
                chunk(
                    {
                        "reasoning": "Plan",
                        "reasoning_details": [
                            {"type": "reasoning.text", "text": "Plan", "format": "anthropic-claude-v1", "index": 0}
                        ],
                    }
                ),
                chunk(
                    {
                        "reasoning": " it",
                        "reasoning_details": [
                            {"type": "reasoning.text", "text": " it", "signature": "sig-1", "index": 0}
                        ],
                    }
                ),
                chunk({"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "status", "arguments": "{}"}}]}),
                chunk({}, finish="tool_calls"),
            ),
            self._answer(),
        )
        client = self.client()
        context = user_context()
        first = self.stream(client, context=context, tools=[{"type": "function", "name": "status"}])
        terminal = [c for c in first if c.response_output is not None][0]
        context.input_items.extend(terminal.response_output)
        context.input_items.append({"type": "function_call_output", "call_id": "call_1", "output": "{}"})
        self.stream(client, context=context, tools=[{"type": "function", "name": "status"}])
        assistant = self.provider.bodies("chat/completions")[1]["messages"][1]
        self.assertEqual(
            assistant["reasoning_details"],
            [
                {
                    "type": "reasoning.text",
                    "text": "Plan it",
                    "format": "anthropic-claude-v1",
                    "index": 0,
                    "signature": "sig-1",
                }
            ],
        )
        # The structured details replace the plain text copy.
        self.assertNotIn("reasoning", assistant)

    def test_effort_is_clamped_and_named_per_endpoint(self):
        self.provider.queue("POST", "chat/completions", self._answer())
        self.stream(self.client(), options={"reasoning_effort": "ultra", "max_tokens": 64})
        (body,) = self.provider.bodies("chat/completions")
        self.assertEqual(body["reasoning_effort"], "high")
        self.assertEqual(body["max_tokens"], 64)
        self.assertNotIn("reasoning", body)
        self.assertIs(body["store"], False)

        openrouter = OpenAICompatibleClient(
            base_url="https://openrouter.ai/api/v1", api_format="chat_completions"
        )
        body = openrouter._chat_completions_body(
            context=user_context(),
            model="anthropic/claude",
            stream=True,
            options={"reasoning_effort": "xhigh"},
            tools=None,
            tool_choice=None,
        )
        # OpenRouter's documented Chat Completions shape.
        self.assertEqual(body["reasoning"], {"effort": "high"})
        self.assertNotIn("reasoning_effort", body)
        for auto in ("auto", None):
            body = openrouter._chat_completions_body(
                context=user_context(),
                model="m",
                stream=True,
                options={"reasoning_effort": auto},
                tools=None,
                tool_choice=None,
            )
            self.assertNotIn("reasoning", body)

    def test_options_pass_through_only_when_known(self):
        self.provider.queue("POST", "chat/completions", self._answer())
        self.stream(
            self.client(),
            options={
                "temperature": 0,
                "response_format": {"type": "json_object"},
                "fast_mode": True,
                "provider_session_id": "cursor",
                "store": True,
                "include": ["x"],
            },
        )
        (body,) = self.provider.bodies("chat/completions")
        self.assertEqual(body["temperature"], 0)
        self.assertEqual(body["response_format"], {"type": "json_object"})
        # A caller cannot turn provider-side storage on.
        self.assertIs(body["store"], False)
        for absent in ("fast_mode", "provider_session_id", "include", "tools", "tool_choice"):
            self.assertNotIn(absent, body)

    def test_store_false_is_dropped_only_for_a_request_refusing_the_field(self):
        refusals = [
            Reply(
                422,
                json_body={
                    "detail": [
                        {"type": "extra_forbidden", "loc": ["body", "store"], "msg": "Extra inputs are not permitted"}
                    ]
                },
            ),
            error_reply(400, "Unrecognized request argument supplied: store"),
            error_reply(400, "property 'store' is unsupported"),
        ]
        for refusal in refusals:
            with self.subTest(status=refusal.status):
                self.provider.requests.clear()
                self.provider.queue(
                    "POST", "chat/completions", refusal, self._answer(), self._answer("again")
                )
                client = self.client()
                self.stream(client)
                self.stream(client)
                refused, retried, later = self.provider.bodies("chat/completions")
                self.assertIs(refused["store"], False)
                self.assertNotIn("store", retried)
                # The next request asks again; nothing is dropped for good.
                self.assertIs(later["store"], False)

    def test_store_is_kept_when_the_refusal_is_about_something_else(self):
        for refusal in (
            error_reply(400, "Invalid 'messages': expected an array."),
            error_reply(400, "The datastore is unavailable, unknown error."),
        ):
            with self.subTest(body=refusal.body[:40]):
                self.provider.requests.clear()
                self.provider.queue("POST", "chat/completions", refusal)
                with self.assertRaises(AppError):
                    self.stream(self.client())
                self.assertEqual(len(self.provider.bodies("chat/completions")), 1)

    def test_store_named_by_the_error_param_is_dropped_for_that_request(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            error_reply(
                400,
                "Unknown parameter: 'store'.",
                param="store",
                code="unknown_parameter",
            ),
            self._answer(),
        )
        self.stream(self.client())
        refused, retried = self.provider.bodies("chat/completions")
        self.assertIs(refused["store"], False)
        self.assertNotIn("store", retried)

    def test_store_echoed_in_the_input_of_another_fields_error_is_kept(self):
        # A validation error about the effort that echoes the request: the
        # echoed `store` is not what the endpoint refused.
        echoed = {"store": False, "reasoning_effort": "high"}
        refusals = {
            "with loc": Reply(
                422,
                json_body={
                    "detail": [
                        {
                            "type": "value_error",
                            "loc": ["body", "reasoning_effort"],
                            "msg": "Value error, unsupported reasoning_effort",
                            "input": echoed,
                        }
                    ]
                },
            ),
            "message only": Reply(
                422,
                json_body={"detail": [{"msg": "unsupported reasoning_effort", "input": echoed}]},
            ),
        }
        for name, refusal in refusals.items():
            with self.subTest(error=name):
                self.provider.requests.clear()
                self.provider.queue("POST", "chat/completions", refusal, self._answer())
                self.stream(self.client(), options={"reasoning_effort": "high"})
                refused, retried = self.provider.bodies("chat/completions")
                self.assertEqual(refused["reasoning_effort"], "high")
                self.assertIs(refused["store"], False)
                # The effort goes; store: false stays.
                self.assertNotIn("reasoning_effort", retried)
                self.assertIs(retried["store"], False)

    def test_store_is_not_dropped_after_another_part_was(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            Reply(
                422,
                json_body={
                    "detail": [
                        {
                            "loc": ["body", "reasoning_effort"],
                            "msg": "unsupported reasoning_effort",
                            "input": {"store": False},
                        }
                    ]
                },
            ),
            error_reply(400, "Unrecognized request argument supplied: store"),
            self._answer(),
        )
        with self.assertRaises(AppError):
            self.stream(self.client(), options={"reasoning_effort": "high"})
        bodies = self.provider.bodies("chat/completions")
        self.assertEqual(len(bodies), 2)
        for body in bodies:
            self.assertIs(body["store"], False)

    def test_nothing_more_is_dropped_once_store_was(self):
        # The endpoint refuses store, then the effort: the turn fails after
        # those two requests instead of sending a third without either.
        self.provider.queue(
            "POST",
            "chat/completions",
            error_reply(400, "Unknown parameter: 'store'.", param="store"),
            error_reply(
                400,
                "Unsupported parameter: 'reasoning_effort' is not supported with this model.",
                param="reasoning_effort",
            ),
            self._answer(),
        )
        with self.assertRaises(AppError):
            self.stream(self.client(), options={"reasoning_effort": "high", "max_tokens": 32})
        first, second = self.provider.bodies("chat/completions")
        self.assertIs(first["store"], False)
        self.assertNotIn("store", second)
        self.assertEqual(second["reasoning_effort"], "high")
        # Nor is max_tokens renamed after the store drop.
        error_after_store = error_reply(
            400,
            "Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead.",
            param="max_tokens",
        )
        self.provider.requests.clear()
        self.provider.routes.clear()
        self.provider.queue(
            "POST",
            "chat/completions",
            error_reply(400, "Unrecognized request argument supplied: store"),
            error_after_store,
            self._answer(),
        )
        with self.assertRaises(AppError):
            self.stream(self.client(), options={"max_tokens": 32})
        self.assertEqual(len(self.provider.bodies("chat/completions")), 2)
        for body in self.provider.bodies("chat/completions"):
            self.assertNotIn("max_completion_tokens", body)

    def test_store_is_kept_when_the_error_names_other_fields_too(self):
        refusals = {
            "two validation errors": Reply(
                422,
                json_body={
                    "detail": [
                        {"type": "extra_forbidden", "loc": ["body", "store"], "msg": "Extra inputs are not permitted"},
                        {
                            "type": "extra_forbidden",
                            "loc": ["body", "reasoning_effort"],
                            "msg": "Extra inputs are not permitted",
                        },
                    ]
                },
            ),
            "one message": error_reply(
                400, "Unrecognized request arguments supplied: reasoning_effort, store"
            ),
        }
        for name, refusal in refusals.items():
            with self.subTest(error=name):
                self.provider.requests.clear()
                self.provider.queue("POST", "chat/completions", refusal, self._answer())
                self.stream(self.client(), options={"reasoning_effort": "high"})
                refused, retried = self.provider.bodies("chat/completions")
                self.assertIs(refused["store"], False)
                # Only the effort, which the client may drop, goes.
                self.assertNotIn("reasoning_effort", retried)
                self.assertIs(retried["store"], False)

    def test_store_is_kept_when_an_openai_error_names_another_param(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            error_reply(
                400,
                "Unsupported value: 'temperature' does not support 0 with this model "
                "when store is false.",
                param="temperature",
                code="unsupported_value",
            ),
        )
        with self.assertRaises(AppError):
            self.stream(self.client(), options={"temperature": 0})
        (body,) = self.provider.bodies("chat/completions")
        self.assertIs(body["store"], False)

    def test_an_error_inside_the_stream_drops_nothing(self):
        # Only a refused request may lose a part. An error inside an accepted
        # stream, even one that names store and carries a 400, ends the turn
        # with store: false and the effort still in the one request sent.
        self.provider.queue(
            "POST",
            "chat/completions",
            chat_stream(
                chunk(
                    {},
                    finish="error",
                    error={
                        "code": 400,
                        "message": "Unrecognized request argument supplied: store",
                        "param": "store",
                    },
                )
            ),
            self._answer(),
        )
        with self.assertRaises(AppError) as raised:
            self.stream(self.client(), options={"reasoning_effort": "high"})
        self.assertEqual(raised.exception.details["status"], 400)
        (body,) = self.provider.bodies("chat/completions")
        self.assertIs(body["store"], False)
        self.assertEqual(body["reasoning_effort"], "high")

    def test_incognito_flag_reaches_no_http_body(self):
        # Incognito asks CLI agents not to keep their own session. An HTTP
        # request already sends store: false and keeps no provider-side
        # conversation, so the broker-only flag stays out of both formats,
        # including the auto fallback from /responses to /chat/completions.
        self.provider.queue("POST", "responses", Reply(404, json_body={"detail": "Not Found"}))
        self.provider.queue("POST", "chat/completions", self._answer())
        auto = OpenAICompatibleClient(base_url=self.provider.base_url)
        self.stream(auto, options={"stateless_session": True, "reasoning_effort": "high"})
        self.provider.queue("POST", "chat/completions", self._answer())
        self.stream(self.client(), options={"stateless_session": True})
        bodies = self.provider.bodies("responses") + self.provider.bodies("chat/completions")
        self.assertEqual(len(bodies), 3)
        for body in bodies:
            self.assertNotIn("stateless_session", json.dumps(body))
            self.assertIs(body["store"], False)

    def test_responses_never_drops_store_false(self):
        self.provider.queue(
            "POST", "responses", error_reply(400, "Unknown parameter: 'store'.", param="store")
        )
        with self.assertRaises(AppError):
            self.stream(OpenAICompatibleClient(base_url=self.provider.base_url, api_format="responses"))
        (body,) = self.provider.bodies("responses")
        self.assertIs(body["store"], False)

    def test_refused_max_tokens_is_sent_as_max_completion_tokens(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            error_reply(
                400,
                "Unsupported parameter: 'max_tokens' is not supported with this model. "
                "Use 'max_completion_tokens' instead.",
                param="max_tokens",
            ),
            self._answer(),
            self._answer(),
        )
        client = self.client()
        self.stream(client, options={"max_tokens": 16})
        self.stream(client, options={"max_tokens": 16})
        refused, retried, later = self.provider.bodies("chat/completions")
        self.assertEqual(refused["max_tokens"], 16)
        self.assertEqual(retried["max_completion_tokens"], 16)
        self.assertNotIn("max_tokens", retried)
        self.assertEqual(later["max_completion_tokens"], 16)

    def test_refused_effort_is_retried_without_it_and_noticed(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            error_reply(400, "Unrecognized request argument supplied: reasoning_effort"),
            self._answer(),
        )
        chunks = self.stream(self.client(), options={"reasoning_effort": "medium"})
        self.assertEqual(
            [c.status_phase for c in chunks if c.status_phase],
            ["reasoning_effort_unavailable"],
        )
        refused, retried = self.provider.bodies("chat/completions")
        self.assertEqual(refused["reasoning_effort"], "medium")
        self.assertNotIn("reasoning_effort", retried)

    def test_blocking_chat_converts_images_and_reads_the_message(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            Reply(
                json_body={
                    "choices": [
                        {
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": '{"rows": []}',
                                "reasoning_content": "Looked at the image.",
                            },
                            "finish_reason": "stop",
                        }
                    ],
                    "usage": {"total_tokens": 3},
                }
            ),
        )
        result = self.client().chat(
            messages=[
                {"role": "system", "content": "Extract JSON."},
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "Read this."},
                        {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA", "detail": "high"}},
                    ],
                },
            ],
            model="vision",
            options={"temperature": 0, "response_format": {"type": "json_object"}},
        )
        self.assertEqual(result["content"], '{"rows": []}')
        self.assertEqual(result["reasoning"], "Looked at the image.")
        self.assertEqual(result["finish_reason"], "stop")
        self.assertEqual(result["usage"], {"total_tokens": 3})
        (body,) = self.provider.bodies("chat/completions")
        self.assertIs(body["stream"], False)
        self.assertEqual(
            body["messages"],
            [
                {"role": "system", "content": "Extract JSON."},
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "Read this."},
                        {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA", "detail": "high"}},
                    ],
                },
            ],
        )
        self.assertEqual(body["response_format"], {"type": "json_object"})

    def test_blocking_chat_tool_calls(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            Reply(
                json_body={
                    "choices": [
                        {
                            "message": {
                                "role": "assistant",
                                "content": None,
                                "tool_calls": [
                                    {"id": "call_9", "type": "function", "function": {"name": "status", "arguments": "{}"}}
                                ],
                            },
                            "finish_reason": "tool_calls",
                        }
                    ]
                }
            ),
        )
        result = self.client().chat(messages=[{"role": "user", "content": "x"}], model="m")
        self.assertEqual(result["finish_reason"], "tool_calls")
        self.assertEqual(result["tool_calls"][0]["id"], "call_9")
        self.assertEqual(result["content"], "")

    def test_egress_ledger_records_each_request(self):
        from kassiber.egress_ledger import get_egress_ledger

        ledger = get_egress_ledger()
        cursor = ledger.snapshot(limit=0)["last_id"]
        self.provider.queue(
            "POST",
            "chat/completions",
            error_reply(400, "Unsupported parameter: 'reasoning_effort'."),
            self._answer(),
        )
        self.stream(self.client(), options={"reasoning_effort": "low"})
        records = [
            record
            for record in ledger.snapshot(after_id=cursor)["records"]
            if record["port"] == self.provider.server_port
        ]
        # The refused request and its retry were both sent, so both are recorded.
        self.assertEqual([record["method"] for record in records], ["POST", "POST"])
        self.assertTrue(all(record["subsystem"] == "ai" for record in records))
        self.assertTrue(all(record["bytes_out"] > 0 for record in records))


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


class DaemonCutOffToolCallTest(ProviderTestCase):
    """No tool call runs from an answer cut off by length or a content filter."""

    def _turn(self, client):
        from kassiber import daemon

        return daemon._stream_ai_chat_tool_turn(
            "req-1",
            client,
            {"model": "test-model", "options": {}},
            user_context(),
            [],
            _Out(),
            threading.Event(),
        )

    def test_no_call_runs_from_a_cut_off_answer_in_either_format(self):
        from kassiber.ai.contracts import ChatDelta

        call = {"id": "call_1", "type": "function", "function": {"name": "status", "arguments": "{}"}}

        class _Client:
            def __init__(self, finish):
                self.finish = finish

            def stream_chat(self, **_kwargs):
                yield ChatDelta(delta={"tool_calls": [call]}, finish_reason=None, raw={})
                yield ChatDelta(
                    delta={"tool_calls": [call]},
                    finish_reason=self.finish,
                    raw={},
                    response_output=[
                        {"type": "function_call", "call_id": "call_1", "name": "status", "arguments": "{}"}
                    ],
                )

        for finish in ("length", "content_filter"):
            with self.subTest(finish=finish):
                turn = self._turn(_Client(finish))
                self.assertEqual(turn.finish_reason, finish)
                self.assertEqual(turn.tool_calls, [])
                self.assertEqual(turn.response_output, [])
        # A call the answer finished still runs.
        turn = self._turn(_Client("tool_calls"))
        self.assertEqual(turn.tool_calls, [call])

        # A Responses answer cut off after a completed call: the client keeps
        # that call, and the daemon still runs none of it.
        self.provider.queue(
            "POST",
            "responses",
            responses_stream(
                [
                    {
                        "type": "function_call",
                        "status": "completed",
                        "call_id": "call_done",
                        "name": "status",
                        "arguments": "{}",
                    }
                ],
                status="incomplete",
                incomplete_reason="max_output_tokens",
            ),
        )
        turn = self._turn(self.client())
        self.assertEqual(turn.finish_reason, "length")
        self.assertEqual(turn.tool_calls, [])


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
