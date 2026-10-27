"""Which OpenAI wire format an HTTP provider speaks, and how it is detected.

`api_format` is the user's setting (auto, responses, chat_completions) and
`detected_api_format` what an endpoint was seen to answer. An explicit format
wins; auto uses the detection, else the Responses API, and falls back to Chat
Completions once when the server has no /responses.
"""

from __future__ import annotations

import contextvars
import io
import json
import queue
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

from kassiber import daemon as daemon_runtime
from kassiber.ai import (
    create_db_ai_provider,
    get_ai_provider_api_key_for_use,
    get_db_ai_provider,
    update_db_ai_provider,
)
from kassiber.ai.providers import (
    list_with_default,
    record_detected_api_format,
    redact_ai_provider_for_output,
)
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_ai_core_http_transport import (
    ProviderTestCase,
    Reply,
    chat_stream,
    chunk,
    error_reply,
    message_item,
    responses_stream,
)


ROOT = Path(__file__).resolve().parent.parent


def _ctx(conn):
    return daemon_runtime.DaemonContext(
        conn=conn,
        data_root="",
        runtime_config={},
        active_ai_chats=daemon_runtime.ActiveAiChats(),
        main_thread_tasks=queue.Queue(),
        auth_backoff=daemon_runtime.AuthAttemptBackoff(),
        input_lines=queue.Queue(),
        deferred_input_lines=[],
        out=None,
        freshness_stop_event=threading.Event(),
    )


def _request(ctx, kind, args=None):
    envelope, _ = daemon_runtime.handle_request(
        ctx, {"kind": kind, "request_id": "req-1", "args": args or {}}, out=None
    )
    return envelope


class ProviderFormatStorageTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-format-")
        self.conn = open_db(str(Path(self._tmp.name) / "data"))

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def _redacted(self, name):
        return redact_ai_provider_for_output(get_db_ai_provider(self.conn, name))

    def test_providers_start_on_auto_with_nothing_detected(self):
        create_db_ai_provider(self.conn, "lan", "http://127.0.0.1:1234/v1")
        for row in list_with_default(self.conn)["providers"]:
            self.assertEqual(row["api_format"], "auto", row["name"])
            self.assertIsNone(row["detected_api_format"], row["name"])
        rows = {row["name"]: row for row in list_with_default(self.conn)["providers"]}
        # HTTP providers speak Responses until something else is chosen or found.
        self.assertEqual(rows["lan"]["effective_api_format"], "responses")
        self.assertEqual(rows["ollama"]["effective_api_format"], "responses")
        # A CLI agent has its own protocol.
        self.assertIsNone(rows["codex"]["effective_api_format"])

    def test_an_explicit_format_wins_over_the_detected_one(self):
        create_db_ai_provider(
            self.conn,
            "router",
            "http://127.0.0.1:1234/v1",
            api_format="chat-completions",
        )
        row = self._redacted("router")
        self.assertEqual(row["api_format"], "chat_completions")
        self.assertEqual(row["effective_api_format"], "chat_completions")

        update_db_ai_provider(
            self.conn, "router", {"api_format": "auto", "detected_api_format": "chat_completions"}
        )
        row = self._redacted("router")
        self.assertEqual(row["api_format"], "auto")
        self.assertEqual(row["effective_api_format"], "chat_completions")

        update_db_ai_provider(self.conn, "router", {"api_format": "responses"})
        row = self._redacted("router")
        self.assertEqual(row["detected_api_format"], "chat_completions")
        self.assertEqual(row["effective_api_format"], "responses")

    def test_cli_agents_take_no_format(self):
        with self.assertRaises(AppError) as raised:
            create_db_ai_provider(
                self.conn,
                "claude-work",
                "claude-cli://default",
                kind="remote",
                api_format="responses",
            )
        self.assertEqual(raised.exception.code, "validation")
        for updates in ({"api_format": "chat_completions"}, {"detected_api_format": "responses"}):
            with self.subTest(updates=updates), self.assertRaises(AppError) as raised:
                update_db_ai_provider(self.conn, "codex", updates)
            self.assertEqual(raised.exception.code, "validation")
        # auto is fine for them.
        update_db_ai_provider(self.conn, "codex", {"api_format": "auto"})

    def test_a_provider_moved_to_a_cli_agent_drops_its_pinned_format(self):
        create_db_ai_provider(
            self.conn,
            "router",
            "https://openrouter.ai/api/v1",
            kind="remote",
            api_format="responses",
            detected_api_format="responses",
        )
        # Moved without a format of its own: the pinned HTTP format goes.
        update_db_ai_provider(self.conn, "router", {"base_url": "codex-cli://default"})
        row = self._redacted("router")
        self.assertEqual(row["api_format"], "auto")
        self.assertIsNone(row["detected_api_format"])
        self.assertIsNone(row["effective_api_format"])
        # Back to HTTP, it starts from auto.
        update_db_ai_provider(self.conn, "router", {"base_url": "https://openrouter.ai/api/v1"})
        self.assertEqual(self._redacted("router")["api_format"], "auto")

        # An explicit HTTP format for a CLI agent is still refused, and so is
        # a pinned format kept while the CLI locator stays.
        update_db_ai_provider(self.conn, "router", {"api_format": "chat_completions"})
        with self.assertRaises(AppError) as raised:
            update_db_ai_provider(
                self.conn,
                "router",
                {"base_url": "claude-cli://default", "api_format": "chat_completions"},
            )
        self.assertEqual(raised.exception.code, "validation")
        self.assertEqual(self._redacted("router")["base_url"], "https://openrouter.ai/api/v1")

    def test_unknown_values_are_rejected(self):
        for kwargs in ({"api_format": "completions"}, {"detected_api_format": "auto"}):
            with self.subTest(kwargs=kwargs), self.assertRaises(AppError) as raised:
                create_db_ai_provider(self.conn, "x", "http://127.0.0.1:9/v1", **kwargs)
            self.assertEqual(raised.exception.code, "validation")

    def test_a_new_endpoint_forgets_the_old_detection(self):
        create_db_ai_provider(
            self.conn, "lan", "http://127.0.0.1:1234/v1", detected_api_format="chat_completions"
        )
        update_db_ai_provider(self.conn, "lan", {"notes": "same endpoint"})
        self.assertEqual(get_db_ai_provider(self.conn, "lan")["detected_api_format"], "chat_completions")
        update_db_ai_provider(self.conn, "lan", {"base_url": "http://127.0.0.1:5678/v1"})
        self.assertIsNone(get_db_ai_provider(self.conn, "lan")["detected_api_format"])
        # A connection test of the edited form can save its finding with the URL.
        update_db_ai_provider(
            self.conn,
            "lan",
            {"base_url": "http://127.0.0.1:9999/v1", "detected_api_format": "responses"},
        )
        self.assertEqual(get_db_ai_provider(self.conn, "lan")["detected_api_format"], "responses")
        update_db_ai_provider(self.conn, "lan", {"clear": ["detected_api_format"]})
        self.assertIsNone(get_db_ai_provider(self.conn, "lan")["detected_api_format"])

    def test_detection_is_recorded_only_for_the_endpoint_it_was_made_for(self):
        create_db_ai_provider(self.conn, "lan", "http://127.0.0.1:1234/v1")
        before = get_db_ai_provider(self.conn, "lan")["updated_at"]
        self.assertFalse(
            record_detected_api_format(self.conn, "lan", "http://127.0.0.1:9/v1", "chat_completions")
        )
        self.assertIsNone(get_db_ai_provider(self.conn, "lan")["detected_api_format"])
        self.assertTrue(
            record_detected_api_format(self.conn, "lan", "http://127.0.0.1:1234/v1", "chat_completions")
        )
        row = get_db_ai_provider(self.conn, "lan")
        self.assertEqual(row["detected_api_format"], "chat_completions")
        # A finding about the endpoint, not a settings edit.
        self.assertEqual(row["updated_at"], before)
        self.assertFalse(record_detected_api_format(self.conn, "codex", "codex-cli://default", "responses"))

    def test_older_book_gains_the_format_columns(self):
        data_root = str(Path(self._tmp.name) / "legacy")
        conn = open_db(data_root)
        list_with_default(conn)
        conn.execute("ALTER TABLE ai_providers DROP COLUMN api_format")
        conn.execute("ALTER TABLE ai_providers DROP COLUMN detected_api_format")
        conn.commit()
        conn.close()
        conn = open_db(data_root)
        try:
            columns = {row["name"] for row in conn.execute("PRAGMA table_info(ai_providers)")}
            self.assertLessEqual({"api_format", "detected_api_format"}, columns)
            for row in list_with_default(conn)["providers"]:
                self.assertEqual(row["api_format"], "auto")
                self.assertIsNone(row["detected_api_format"])
        finally:
            conn.close()


class ProviderFormatDaemonTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-format-daemon-")
        self.conn = open_db(str(Path(self._tmp.name) / "data"))
        self.ctx = _ctx(self.conn)

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def test_create_update_and_list_carry_the_format(self):
        created = _request(
            self.ctx,
            "ai.providers.create",
            {
                "name": "openrouter",
                "base_url": "https://openrouter.ai/api/v1",
                "kind": "remote",
                "acknowledged": True,
                "api_format": "auto",
                "detected_api_format": "chat_completions",
            },
        )["data"]
        self.assertEqual(created["api_format"], "auto")
        self.assertEqual(created["detected_api_format"], "chat_completions")
        self.assertEqual(created["effective_api_format"], "chat_completions")

        updated = _request(
            self.ctx,
            "ai.providers.update",
            {"name": "openrouter", "api_format": "responses"},
        )["data"]
        self.assertEqual(updated["effective_api_format"], "responses")

        listed = {
            row["name"]: row for row in _request(self.ctx, "ai.providers.list")["data"]["providers"]
        }
        self.assertEqual(listed["openrouter"]["api_format"], "responses")
        self.assertNotIn("api_key", listed["openrouter"])

        for args in (
            {"name": "openrouter", "api_format": 3},
            {"name": "openrouter", "detected_api_format": ["responses"]},
        ):
            with self.subTest(args=args), self.assertRaises(AppError) as raised:
                _request(self.ctx, "ai.providers.update", args)
            self.assertEqual(raised.exception.code, "validation")

    def test_chat_hands_the_format_to_the_client(self):
        update_db_ai_provider(
            self.conn, "omlx", {"api_format": "auto", "detected_api_format": "chat_completions"}
        )
        provider = get_db_ai_provider(self.conn, "omlx")
        captured: dict[str, Any] = {}

        def factory(**kwargs):
            captured.update(kwargs)
            raise AppError("stop here", code="ai_unavailable")

        snapshot = {
            "name": provider["name"],
            "base_url": provider["base_url"],
            "kind": provider["kind"],
            "api_format": provider["api_format"],
            "detected_api_format": provider["detected_api_format"],
        }
        with patch("kassiber.daemon.ai_client_for_locator", side_effect=factory):
            stream = io.StringIO()
            active = daemon_runtime.ActiveAiChats()
            key, chat = active.register("req-1")
            # The stream is a thread target that sets the log request id; run
            # it in its own context so the id does not leak into later tests.
            contextvars.copy_context().run(
                daemon_runtime._run_ai_chat_stream,
                "req-1",
                snapshot,
                daemon_runtime._ai_chat_args(
                    {"model": "m", "messages": [{"role": "user", "content": "hi"}]}
                ),
                daemon_runtime._OutputChannel(stream),
                chat,
                active,
                key,
                daemon_runtime.AiToolRuntime(
                    data_root="",
                    runtime_config={},
                    main_thread_tasks=queue.Queue(),
                    maintenance_state={"egress_after_id": 0},
                ),
            )
        self.assertEqual(captured["api_format"], "auto")
        self.assertEqual(captured["detected_api_format"], "chat_completions")


class AutoFallbackTest(ProviderTestCase):
    """Only an auto provider with nothing detected falls back, and only on a missing path."""

    def _answer(self, text="ok"):
        return chat_stream(chunk({"content": text}), chunk({}, finish="stop"))

    def _content(self, chunks):
        return "".join(c.delta.get("content", "") for c in chunks)

    def test_missing_responses_path_falls_back_once_and_reports_detection(self):
        for missing in (Reply(404, json_body={"detail": "Not Found"}), Reply(405, raw=b"Method Not Allowed")):
            with self.subTest(status=missing.status):
                self.provider.requests.clear()
                self.provider.queue("POST", "responses", missing)
                self.provider.queue("POST", "chat/completions", self._answer(), self._answer("again"))
                client = self.client()
                self.assertEqual(self._content(self.stream(client)), "ok")
                self.assertEqual(client.newly_detected_api_format, "chat_completions")
                # The rest of the turn goes straight to Chat Completions.
                self.assertEqual(self._content(self.stream(client)), "again")
                self.assertEqual(
                    [request["path"] for request in self.provider.requests],
                    ["/v1/responses", "/v1/chat/completions", "/v1/chat/completions"],
                )

    def test_a_501_from_responses_falls_back_like_a_missing_path(self):
        for missing in (Reply(501, raw=b""), error_reply(501, "Responses API unavailable on this server")):
            with self.subTest(body=missing.body[:30]):
                self.provider.requests.clear()
                self.provider.routes.clear()
                self.provider.queue("POST", "responses", missing)
                self.provider.queue("POST", "chat/completions", self._answer())
                client = self.client()
                self.assertEqual(self._content(self.stream(client)), "ok")
                self.assertEqual(client.newly_detected_api_format, "chat_completions")
        # Only before anything answered, and only for an auto provider.
        self.provider.requests.clear()
        self.provider.routes.clear()
        self.provider.queue("POST", "responses", Reply(501, raw=b""))
        with self.assertRaises(AppError):
            self.stream(self.client(api_format="responses"))
        self.assertEqual(
            [request["path"] for request in self.provider.requests], ["/v1/responses"]
        )
        # A 501 about the model or the tools is not a missing path.
        self.provider.requests.clear()
        self.provider.routes.clear()
        self.provider.queue(
            "POST", "responses", error_reply(501, "Tool calling is not implemented for this model")
        )
        with self.assertRaises(AppError):
            self.stream(self.client())
        self.assertEqual(
            [request["path"] for request in self.provider.requests], ["/v1/responses"]
        )

    def test_a_400_naming_the_endpoint_unknown_falls_back(self):
        self.provider.queue(
            "POST",
            "responses",
            error_reply(400, "Unexpected endpoint or method. (POST /v1/responses)"),
        )
        self.provider.queue("POST", "chat/completions", self._answer())
        client = self.client()
        self.assertEqual(self._content(self.stream(client)), "ok")
        self.assertEqual(client.newly_detected_api_format, "chat_completions")

    def test_a_400_without_an_endpoint_phrase_does_not_fall_back(self):
        for body in ("File not found.", "Unexpected value for 'input'.", "Unknown endpoint for model gpt-x."):
            with self.subTest(body=body):
                self.provider.requests.clear()
                self.provider.queue("POST", "responses", error_reply(400, body))
                with self.assertRaises(AppError):
                    self.stream(self.client())
                self.assertEqual([r["path"] for r in self.provider.requests], ["/v1/responses"])

    def test_blocking_chat_falls_back_too(self):
        self.provider.queue(
            "POST",
            "chat/completions",
            Reply(json_body={"choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}]}),
        )
        client = self.client()
        result = client.chat(messages=[{"role": "user", "content": "x"}], model="m")
        self.assertEqual(result["content"], "ok")
        self.assertEqual(client.newly_detected_api_format, "chat_completions")

    def test_no_fallback_for_an_explicit_or_detected_format(self):
        for kwargs in ({"api_format": "responses"}, {"detected_api_format": "responses"}):
            with self.subTest(kwargs=kwargs):
                self.provider.requests.clear()
                self.provider.queue("POST", "responses", Reply(404, json_body={"detail": "Not Found"}))
                client = self.client(**kwargs)
                with self.assertRaises(AppError) as raised:
                    self.stream(client)
                self.assertEqual(raised.exception.code, "ai_request_invalid")
                self.assertEqual([r["path"] for r in self.provider.requests], ["/v1/responses"])
                self.assertIsNone(client.newly_detected_api_format)

    def test_no_fallback_for_other_failures(self):
        failures = [
            error_reply(400, "Invalid 'input'."),
            error_reply(401, "Incorrect API key provided."),
            error_reply(429, "Too many requests."),
            error_reply(500, "Internal error."),
            # About the model or the tools, not a path the server lacks.
            error_reply(404, "The model `gpt-x` does not exist.", code="model_not_found"),
            error_reply(404, "No endpoints found that support tool use."),
        ]
        for failure in failures:
            with self.subTest(status=failure.status, body=failure.body[:60]):
                self.provider.requests.clear()
                self.provider.queue("POST", "responses", failure)
                with self.assertRaises(AppError):
                    self.stream(self.client())
                self.assertEqual([r["path"] for r in self.provider.requests], ["/v1/responses"])

    def test_no_fallback_once_responses_answered(self):
        self.provider.queue(
            "POST",
            "responses",
            responses_stream([message_item("ok")], text="ok"),
            Reply(404, json_body={"detail": "Not Found"}),
        )
        client = self.client()
        self.stream(client)
        with self.assertRaises(AppError):
            self.stream(client)
        self.assertNotIn("/v1/chat/completions", [r["path"] for r in self.provider.requests])

    def test_both_paths_missing_names_the_base_url(self):
        with self.assertRaises(AppError) as raised:
            self.stream(self.client())
        self.assertIn("neither a Responses nor a Chat Completions", str(raised.exception))
        self.assertIn("/v1", raised.exception.hint)


class ConnectionTestDaemonTest(ProviderTestCase):
    """`ai.test_connection` lists models and sends one fixed request."""

    def setUp(self):
        super().setUp()
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-connection-test-")
        self.addCleanup(self._tmp.cleanup)
        self.conn = open_db(str(Path(self._tmp.name) / "data"))
        self.addCleanup(self.conn.close)
        self.ctx = _ctx(self.conn)

    def _models(self, *ids):
        self.provider.queue(
            "GET", "models", Reply(json_body={"object": "list", "data": [{"id": i} for i in ids]})
        )

    def _ok(self, path="chat/completions"):
        if path == "responses":
            return responses_stream([message_item("OK")], text="OK")
        return chat_stream(chunk({"content": "OK"}), chunk({}, finish="stop"))

    def _test(self, **args):
        args.setdefault("base_url", self.provider.base_url)
        return _request(self.ctx, "ai.test_connection", args)["data"]

    def _posts(self):
        return [r["path"] for r in self.provider.requests if r["method"] == "POST"]

    def test_a_dropped_responses_stream_fails_the_test_instead_of_switching_format(self):
        # The endpoint speaks Responses but the connection drops before
        # response.completed: a transient failure, so the fixed prompt is not
        # resent to Chat Completions and no format is recorded.
        create_db_ai_provider(self.conn, "lan", self.provider.base_url, kind="local")
        self._models("m")
        self.provider.queue(
            "POST",
            "responses",
            Reply(
                sse_events=[
                    {"type": "response.created", "response": {"id": "r1"}},
                    {"type": "response.output_text.delta", "delta": "O"},
                ],
                done=False,
            ),
        )
        self.provider.queue("POST", "chat/completions", self._ok())
        with self.assertRaises(AppError) as raised:
            self._test(provider="lan", model="m")
        self.assertEqual(raised.exception.details["reason"], "stream_incomplete")
        self.assertEqual(self._posts(), ["/v1/responses"])
        self.assertIsNone(get_db_ai_provider(self.conn, "lan")["detected_api_format"])

    def test_a_stream_in_another_shape_still_moves_to_the_next_format(self):
        # /responses answered with Chat Completions chunks and no Responses
        # event at all: that is an unserved format, so auto tries the next one.
        self._models("m")
        self.provider.queue(
            "POST",
            "responses",
            Reply(sse_events=[{"choices": [{"delta": {"content": "OK"}}]}], done=False),
        )
        self.provider.queue("POST", "chat/completions", self._ok())
        data = self._test(model="m")
        self.assertEqual(data["detected_api_format"], "chat_completions")

    def test_auto_finds_chat_completions_and_saves_it_on_the_saved_provider(self):
        create_db_ai_provider(
            self.conn, "lan", self.provider.base_url, kind="local", default_model="qwen"
        )
        self._models("qwen", "nomic-embed")
        self.provider.queue("POST", "responses", Reply(404, json_body={"detail": "Not Found"}))
        self.provider.queue("POST", "chat/completions", self._ok())
        data = self._test(provider="lan")

        self.assertEqual(data["model_count"], 2)
        self.assertEqual(data["api_format"], "auto")
        self.assertEqual(data["detected_api_format"], "chat_completions")
        self.assertIs(data["tools_accepted"], True)
        self.assertIs(data["detection_saved"], True)
        self.assertEqual(
            data["probe"],
            {"status": "ok", "reason": None, "model": "qwen", "finish_reason": "stop"},
        )
        self.assertEqual(get_db_ai_provider(self.conn, "lan")["detected_api_format"], "chat_completions")

        (responses_body,) = self.provider.bodies("responses")
        self.assertEqual(responses_body["input"], [{"type": "message", "role": "user", "content": "Reply with OK"}])
        self.assertEqual(responses_body["max_output_tokens"], 16)
        self.assertIs(responses_body["store"], False)
        self.assertEqual([tool["name"] for tool in responses_body["tools"]], ["connection_check"])
        (chat_body,) = self.provider.bodies("chat/completions")
        self.assertEqual(chat_body["messages"], [{"role": "user", "content": "Reply with OK"}])
        self.assertEqual(chat_body["max_tokens"], 16)
        self.assertEqual(chat_body["tools"][0]["function"]["name"], "connection_check")
        self.assertEqual(chat_body["tool_choice"], "auto")
        self.assertIs(chat_body["store"], False)

    def test_an_unsaved_form_gets_the_detection_back_unsaved(self):
        self._models("only-model")
        self.provider.queue("POST", "responses", self._ok("responses"))
        data = self._test(kind="local")
        # One listed model is enough to test with.
        self.assertEqual(data["probe"]["model"], "only-model")
        self.assertEqual(data["detected_api_format"], "responses")
        self.assertIs(data["detection_saved"], False)

    def test_a_different_url_than_the_saved_one_is_not_saved(self):
        create_db_ai_provider(self.conn, "lan", "http://127.0.0.1:1/v1", kind="local")
        self._models("m")
        self.provider.queue("POST", "responses", self._ok("responses"))
        data = self._test(provider="lan", model="m")
        self.assertEqual(data["detected_api_format"], "responses")
        self.assertIs(data["detection_saved"], False)
        self.assertIsNone(get_db_ai_provider(self.conn, "lan")["detected_api_format"])

    def test_a_saved_key_is_reused_for_another_path_on_its_origin(self):
        # Saved at another path of the same origin; the form tests this path.
        create_db_ai_provider(
            self.conn,
            "gw",
            self.provider.base_url.replace("/v1", "/old/v1"),
            kind="local",
            api_key="sk-same-origin",
        )
        self._models("m")
        self.provider.queue("POST", "responses", self._ok("responses"))
        data = self._test(provider="gw", model="m")
        self.assertEqual(data["detected_api_format"], "responses")
        for request in self.provider.requests:
            self.assertEqual(request["authorization"], "Bearer sk-same-origin")
        # The detection belongs to the exact URL tested, not the saved one.
        self.assertIs(data["detection_saved"], False)

    def test_a_row_moved_meanwhile_lends_no_key_and_takes_no_detection(self):
        # test_connection's base_url is the endpoint to test, not the row's,
        # so it has no `ai_provider_changed` binding: what it takes from a
        # saved row that has since moved can still reach only that row's URL.
        create_db_ai_provider(
            self.conn, "lan", self.provider.base_url, kind="local", api_key="sk-old"
        )
        update_db_ai_provider(
            self.conn,
            "lan",
            {"base_url": "http://127.0.0.1:1/v1", "api_key": "sk-moved"},
        )
        # The form still holds the old URL: the key now belongs to the new
        # one, so it is refused before anything is contacted.
        with self.assertRaises(AppError) as refused:
            self._test(provider="lan", model="m")
        self.assertEqual(refused.exception.code, "validation")
        self.assertEqual(self.provider.requests, [])

        # Without a stored key the test runs, keyless, and its detection is
        # returned for the form but not written onto the moved row.
        update_db_ai_provider(self.conn, "lan", {"clear": ["api_key"]})
        self._models("m")
        self.provider.queue("POST", "responses", self._ok("responses"))
        data = self._test(provider="lan", model="m")
        self.assertEqual(data["detected_api_format"], "responses")
        self.assertIs(data["detection_saved"], False)
        self.assertIsNone(get_db_ai_provider(self.conn, "lan")["detected_api_format"])
        self.assertTrue(self.provider.requests)
        for request in self.provider.requests:
            self.assertIsNone(request["authorization"])

    def _refused_before_any_request(self, **args):
        with self.assertRaises(AppError) as refused:
            self._test(model="m", **args)
        self.assertEqual(refused.exception.code, "ai_remote_ack_required")
        self.assertEqual(self.provider.requests, [])

    def _answers(self):
        self._models("m")
        self.provider.queue("POST", "responses", self._ok("responses"))

    def test_an_unacknowledged_saved_remote_provider_is_not_tested(self):
        # Enabled and saved, but never acknowledged for off-device use, as
        # chat and `kassiber ai providers test` refuse it too.
        create_db_ai_provider(self.conn, "gateway", self.provider.base_url, kind="remote")
        self.assertIsNone(get_db_ai_provider(self.conn, "gateway")["acknowledged_at"])
        self._refused_before_any_request(provider="gateway")
        self._refused_before_any_request(provider="gateway", kind="remote")
        self._refused_before_any_request(provider="gateway", kind="tee")

        # Its own acknowledgement lets the test run.
        update_db_ai_provider(self.conn, "gateway", {"acknowledged": True})
        self._answers()
        self.assertEqual(self._test(provider="gateway", model="m")["detected_api_format"], "responses")

    def test_unsaved_off_device_values_need_the_explicit_acknowledgement(self):
        # The provider form's values, its checkbox not ticked.
        self._refused_before_any_request(kind="remote")
        self._refused_before_any_request(kind="tee", acknowledged=False)
        # A local claim for an address off this device is not on-device.
        self._refused_before_any_request(base_url="https://llm.example.test/v1", kind="local")
        # Without a kind, an address off this device counts as remote.
        self._refused_before_any_request(base_url="https://llm.example.test/v1")
        with self.assertRaises(AppError) as invalid:
            self._test(kind="remote", acknowledged="yes")
        self.assertEqual(invalid.exception.code, "validation")
        self.assertEqual(self.provider.requests, [])

        self._answers()
        data = self._test(kind="remote", model="m", acknowledged=True)
        self.assertEqual(data["detected_api_format"], "responses")

    def test_a_local_rows_acknowledgement_does_not_cover_an_off_device_test(self):
        # A local row is acknowledged implicitly; that never covered prompts
        # leaving the device, as when the form moves it to remote.
        create_db_ai_provider(self.conn, "box", self.provider.base_url, kind="local")
        self.assertIsNotNone(get_db_ai_provider(self.conn, "box")["acknowledged_at"])
        self._refused_before_any_request(provider="box", kind="remote")

    def test_an_acknowledged_remote_row_covers_its_form_test(self):
        create_db_ai_provider(
            self.conn, "gateway", "http://127.0.0.1:1/v1", kind="remote", acknowledged=True
        )
        # Retargeted in the form; the acknowledgement belongs to the provider,
        # as it stays when a remote provider is saved with another URL.
        self._answers()
        data = self._test(provider="gateway", kind="remote", model="m")
        self.assertEqual(data["detected_api_format"], "responses")

    def test_local_loopback_targets_need_no_acknowledgement(self):
        self._answers()
        self.assertEqual(self._test(kind="local", model="m")["probe"]["status"], "ok")
        self._answers()
        # Inferred local, as `create` would.
        self.assertEqual(self._test(model="m")["probe"]["status"], "ok")

    def test_refused_tools_are_retried_without_them(self):
        refusals = [
            error_reply(400, 'registry.ollama.ai/library/gemma:2b does not support tools'),
            # OpenRouter's routing 404 is about tools, not a missing path.
            error_reply(404, "No endpoints found that support tool use."),
        ]
        for refusal in refusals:
            with self.subTest(status=refusal.status):
                self.provider.requests.clear()
                self._models("m")
                self.provider.queue("POST", "responses", refusal, self._ok("responses"))
                data = self._test(model="m")
                self.assertEqual(data["detected_api_format"], "responses")
                self.assertIs(data["tools_accepted"], False)
                with_tool, without_tool = self.provider.bodies("responses")
                self.assertIn("tools", with_tool)
                self.assertNotIn("tools", without_tool)
                self.assertNotIn("/v1/chat/completions", self._posts())

    def _probe_bodies(self):
        return self.provider.bodies("chat/completions")

    def _only_the_fixed_prompt(self, bodies):
        for body in bodies:
            self.assertEqual(body["messages"], [{"role": "user", "content": "Reply with OK"}])

    def test_a_store_refusal_is_reported_and_tested_without_it(self):
        unknown_store = error_reply(400, "store is not supported with value false", param="store")
        create_db_ai_provider(
            self.conn, "gw", self.provider.base_url, kind="local", api_format="chat_completions"
        )
        self._models("m")
        self.provider.queue("POST", "chat/completions", unknown_store, self._ok())
        data = self._test(provider="gw", model="m")
        self.assertIs(data["store_accepted"], False)
        self.assertEqual(data["detected_api_format"], "chat_completions")
        self.assertIs(data["tools_accepted"], True)
        with_store, without_store = self._probe_bodies()
        self.assertIs(with_store["store"], False)
        self.assertNotIn("store", without_store)
        # Only the fixed test prompt went without the flag.
        self._only_the_fixed_prompt([with_store, without_store])
        # The test changes no setting: chats still send store: false.
        self.assertFalse(get_db_ai_provider(self.conn, "gw")["omit_store_field"])

    def test_store_and_tool_refusals_are_each_retried_once_in_the_test(self):
        unknown_store = error_reply(400, "Unrecognized request argument supplied: store")
        no_tools = error_reply(400, "This model does not support tools.")
        sequences = {
            "store, then tools": [unknown_store, no_tools, self._ok()],
            "tools, then store": [no_tools, unknown_store, self._ok()],
        }
        for name, replies in sequences.items():
            with self.subTest(sequence=name):
                self.provider.requests.clear()
                self.provider.routes.clear()
                self._models("m")
                self.provider.queue("POST", "chat/completions", *replies)
                data = self._test(model="m", api_format="chat_completions")
                self.assertIs(data["store_accepted"], False)
                self.assertIs(data["tools_accepted"], False)
                bodies = self._probe_bodies()
                self.assertEqual(len(bodies), 3)
                self._only_the_fixed_prompt(bodies)
        # A second refusal of the same kind is not retried again.
        self.provider.requests.clear()
        self.provider.routes.clear()
        self._models("m")
        self.provider.queue("POST", "chat/completions", unknown_store, unknown_store, self._ok())
        with self.assertRaises(AppError):
            self._test(model="m", api_format="chat_completions")
        self.assertEqual(len(self._probe_bodies()), 2)

    def test_store_accepted_is_reported_when_the_flag_goes_through(self):
        self._models("m")
        self.provider.queue("POST", "chat/completions", self._ok())
        data = self._test(model="m", api_format="chat_completions")
        self.assertIs(data["store_accepted"], True)
        # A provider that already leaves it out is not asked.
        self.provider.requests.clear()
        self._models("m")
        self.provider.queue("POST", "chat/completions", self._ok())
        data = self._test(model="m", api_format="chat_completions", omit_store_field=True)
        self.assertIsNone(data["store_accepted"])
        (body,) = self._probe_bodies()
        self.assertNotIn("store", body)
        with self.assertRaises(AppError) as invalid:
            self._test(model="m", omit_store_field="yes")
        self.assertEqual(invalid.exception.code, "validation")

    def test_a_responses_store_refusal_is_not_retried_in_the_test(self):
        self._models("m")
        self.provider.queue(
            "POST",
            "responses",
            error_reply(400, "Unknown parameter: 'store'.", param="store"),
            self._ok("responses"),
        )
        with self.assertRaises(AppError) as raised:
            self._test(model="m", api_format="responses")
        self.assertEqual(raised.exception.code, "ai_store_refused")
        self.assertEqual(len(self.provider.bodies("responses")), 1)

    def test_a_partial_answer_is_not_kept_as_a_complete_one(self):
        # The connection ends after some text, without [DONE] or a finish
        # reason: the chat fails instead of ending as if answered.
        create_db_ai_provider(
            self.conn, "gw", self.provider.base_url, kind="local", api_format="chat_completions"
        )
        self.provider.queue(
            "POST", "chat/completions", chat_stream(chunk({"content": "Half an ans"}), done=False)
        )
        stream = io.StringIO()
        before = {thread.ident for thread in threading.enumerate()}
        daemon_runtime.handle_request(
            self.ctx,
            {
                "kind": "ai.chat",
                "request_id": "chat-1",
                "args": {"provider": "gw", "model": "m", "messages": [{"role": "user", "content": "hi"}]},
            },
            out=daemon_runtime._OutputChannel(stream),
        )
        for thread in threading.enumerate():
            if thread.ident not in before and thread.name == "kassiber-ai-chat":
                thread.join(timeout=10)
        records = [json.loads(line) for line in stream.getvalue().splitlines() if line.strip()]
        self.assertEqual([r for r in records if r.get("kind") == "ai.chat"], [])
        errors = [r for r in records if r.get("kind") == "error"]
        self.assertEqual(len(errors), 1, records)
        self.assertEqual(errors[0]["error"]["code"], "ai_unavailable")
        self.assertEqual(errors[0]["error"]["details"]["reason"], "stream_incomplete")

    def test_a_test_of_another_address_does_not_inherit_the_exemption(self):
        # The exemption was chosen for the saved address; saving another
        # address resets it, so testing that other address must send
        # store: false like the saved provider would.
        create_db_ai_provider(
            self.conn,
            "gw",
            "http://127.0.0.1:1/v1",
            kind="local",
            api_format="chat_completions",
            omit_store_field=True,
        )
        self._models("m")
        self.provider.queue("POST", "chat/completions", self._ok())
        data = self._test(provider="gw", model="m", api_format="chat_completions")
        self.assertEqual(data["probe"]["status"], "ok")
        (body,) = self.provider.bodies("chat/completions")
        self.assertIs(body.get("store"), False)
        self.assertIs(data["store_accepted"], True)

    def test_a_provider_that_omits_store_chats_without_it(self):
        create_db_ai_provider(
            self.conn,
            "gw",
            self.provider.base_url,
            kind="local",
            api_format="chat_completions",
            omit_store_field=True,
        )
        self.provider.queue("POST", "chat/completions", self._ok())
        stream = io.StringIO()
        before = {thread.ident for thread in threading.enumerate()}
        daemon_runtime.handle_request(
            self.ctx,
            {
                "kind": "ai.chat",
                "request_id": "chat-1",
                "args": {"provider": "gw", "model": "m", "messages": [{"role": "user", "content": "hi"}]},
            },
            out=daemon_runtime._OutputChannel(stream),
        )
        for thread in threading.enumerate():
            if thread.ident not in before and thread.name == "kassiber-ai-chat":
                thread.join(timeout=10)
        records = [json.loads(line) for line in stream.getvalue().splitlines() if line.strip()]
        terminal = [record for record in records if record.get("kind") == "ai.chat"]
        self.assertEqual(len(terminal), 1, records)
        (body,) = self.provider.bodies("chat/completions")
        self.assertNotIn("store", body)
        self.assertEqual(body["messages"], [{"role": "user", "content": "hi"}])
    def test_an_echoed_tool_definition_is_not_a_tool_refusal(self):
        self._models("m")
        self.provider.queue(
            "POST",
            "chat/completions",
            Reply(
                422,
                json_body={
                    "detail": [
                        {
                            "loc": ["body", "temperature"],
                            "msg": "Input should be less than or equal to 2",
                            "input": {"tools": [{"type": "function"}], "tool_choice": "auto"},
                        }
                    ]
                },
            ),
            self._ok(),
        )
        with self.assertRaises(AppError):
            self._test(model="m", api_format="chat_completions")
        self.assertEqual(len(self.provider.bodies("chat/completions")), 1)

    def test_a_tool_refusal_inside_the_stream_is_retried_without_tools(self):
        # OpenRouter-style: the request was accepted (HTTP 200) and the
        # routing or tool error arrives as an error event in the stream,
        # with the status as its code, or with no number at all.
        refusals = {
            "responses, numeric code": (
                "responses",
                chat_stream(
                    {"type": "error", "code": 404, "message": "No endpoints found that support tool use."}
                ),
            ),
            "responses, string code": (
                "responses",
                chat_stream(
                    {
                        "type": "error",
                        "code": "invalid_request_error",
                        "message": "This model does not support tools.",
                    }
                ),
            ),
            "chat completions, numeric code": (
                "chat_completions",
                chat_stream(
                    chunk(
                        {},
                        finish="error",
                        error={"code": 400, "message": "No endpoints found that support tool use."},
                    )
                ),
            ),
        }
        for name, (wire, refusal) in refusals.items():
            with self.subTest(case=name):
                self.provider.requests.clear()
                self.provider.routes.clear()
                self._models("m")
                path = "responses" if wire == "responses" else "chat/completions"
                self.provider.queue("POST", path, refusal, self._ok(path))
                data = self._test(model="m", api_format=wire)
                self.assertEqual(data["detected_api_format"], wire)
                self.assertIs(data["tools_accepted"], False)
                with_tool, without_tool = self.provider.bodies(path)
                self.assertIn("tools", with_tool)
                self.assertNotIn("tools", without_tool)

    def test_an_error_inside_the_stream_is_not_a_missing_format(self):
        # The path answered with a stream, so its status-like code says
        # nothing about the format: auto does not move on to Chat Completions.
        self._models("m")
        self.provider.queue(
            "POST",
            "responses",
            chat_stream({"type": "error", "code": 404, "message": "Not Found"}),
        )
        self.provider.queue("POST", "chat/completions", self._ok())
        with self.assertRaises(AppError) as raised:
            self._test(model="m")
        self.assertEqual(raised.exception.details["status"], 404)
        self.assertIs(raised.exception.details["in_stream"], True)
        self.assertNotIn("/v1/chat/completions", self._posts())

    def test_an_endpoint_that_never_replies_does_not_pass_the_test(self):
        # Chat Completions answering with only [DONE]: nothing is detected,
        # and nothing about tools or store is reported as accepted.
        create_db_ai_provider(self.conn, "gw", self.provider.base_url, kind="local")
        self._models("m")
        self.provider.queue("POST", "chat/completions", chat_stream(), chat_stream(), chat_stream())
        with self.assertRaises(AppError) as raised:
            self._test(provider="gw", model="m", api_format="chat_completions")
        self.assertEqual(raised.exception.details["reason"], "no_answer")
        self.assertIsNone(get_db_ai_provider(self.conn, "gw")["detected_api_format"])
        self.assertEqual(len(self.provider.bodies("chat/completions")), 1)

    def test_an_empty_completed_response_is_no_answer_in_that_format(self):
        # /responses "completes" with nothing in it; auto moves on to Chat
        # Completions, which does answer.
        self._models("m")
        self.provider.queue("POST", "responses", responses_stream([]))
        self.provider.queue("POST", "chat/completions", self._ok())
        data = self._test(model="m")
        self.assertEqual(data["detected_api_format"], "chat_completions")
        self.assertEqual(self._posts(), ["/v1/responses", "/v1/chat/completions"])

    def test_a_501_from_responses_tries_chat_completions_in_the_test(self):
        self._models("m")
        self.provider.queue(
            "POST", "responses", error_reply(501, "Responses API unavailable on this server")
        )
        self.provider.queue("POST", "chat/completions", self._ok())
        self.assertEqual(self._test(model="m")["detected_api_format"], "chat_completions")

    def test_a_body_saying_the_path_is_unknown_tries_chat_completions(self):
        self._models("m")
        self.provider.queue(
            "POST",
            "responses",
            error_reply(400, "Unexpected endpoint or method. (POST /v1/responses)"),
        )
        self.provider.queue("POST", "chat/completions", self._ok())
        self.assertEqual(self._test(model="m")["detected_api_format"], "chat_completions")

    def test_an_explicit_format_is_tested_alone(self):
        self._models("m")
        self.provider.queue("POST", "chat/completions", Reply(404, json_body={"detail": "Not Found"}))
        with self.assertRaises(AppError):
            self._test(model="m", api_format="chat_completions")
        self.assertEqual(self._posts(), ["/v1/chat/completions"])

    def test_without_a_model_to_ask_only_models_are_listed(self):
        self._models("a", "b")
        data = self._test()
        self.assertEqual(
            data["probe"], {"status": "skipped", "reason": "no_model", "model": None, "finish_reason": None}
        )
        self.assertIsNone(data["detected_api_format"])
        self.assertIsNone(data["tools_accepted"])
        self.assertEqual(self._posts(), [])

    def test_other_failures_fail_the_test(self):
        self._models("m")
        self.provider.queue("POST", "responses", error_reply(401, "Incorrect API key provided."))
        with self.assertRaises(AppError) as raised:
            self._test(model="m")
        self.assertEqual(raised.exception.code, "ai_auth_failed")
        self.assertEqual(self._posts(), ["/v1/responses"])

    def test_a_length_stop_still_counts_as_an_answer(self):
        # A reasoning model can spend all 16 tokens thinking.
        self._models("m")
        self.provider.queue(
            "POST",
            "responses",
            responses_stream([], status="incomplete", incomplete_reason="max_output_tokens"),
        )
        data = self._test(model="m")
        self.assertEqual(data["detected_api_format"], "responses")
        self.assertEqual(data["probe"]["finish_reason"], "length")

    def test_offline_mode_refuses_a_remote_test_before_any_request(self):
        from kassiber import egress_policy

        preference = Path(self._tmp.name) / "offline.json"
        with patch.dict("os.environ", {egress_policy.OFFLINE_PREFERENCE_ENV: str(preference)}):
            egress_policy.set_offline_mode(True)
            with self.assertRaises(AppError) as raised:
                self._test(kind="remote", model="m", acknowledged=True)
        self.assertEqual(raised.exception.code, "network_egress_disabled")
        self.assertEqual(self.provider.requests, [])

    def test_a_provider_retargeted_to_a_cli_agent_tests_without_its_http_settings(self):
        from kassiber.ai.broker_client import BrokerAIClient

        # A keyless HTTP provider with HTTP-only settings, edited in the form
        # to a CLI locator that has not been saved yet.
        create_db_ai_provider(
            self.conn,
            "gw",
            "https://llm.example.test/v1",
            kind="remote",
            acknowledged=True,
            api_format="chat_completions",
            detected_api_format="chat_completions",
            omit_store_field=True,
        )
        with patch.object(BrokerAIClient, "list_models", return_value=[{"id": "default"}]):
            data = self._test(
                base_url="codex-cli://default", provider="gw", kind="remote", acknowledged=True
            )
        self.assertEqual(data["probe"]["reason"], "cli_provider")
        self.assertIsNone(data["api_format"])
        self.assertIsNone(data["store_accepted"])
        self.assertIs(data["detection_saved"], False)
        # An explicit HTTP-only choice for a CLI target is still a mistake.
        with self.assertRaises(AppError) as raised:
            self._test(
                base_url="codex-cli://default",
                provider="gw",
                kind="remote",
                acknowledged=True,
                omit_store_field=True,
            )
        self.assertEqual(raised.exception.code, "validation")

    def test_cli_agents_are_not_probed(self):
        from kassiber.ai.broker_client import BrokerAIClient

        with patch.object(BrokerAIClient, "list_models", return_value=[{"id": "default"}]):
            data = self._test(base_url="claude-cli://default", kind="remote", acknowledged=True)
        self.assertEqual(data["probe"]["reason"], "cli_provider")
        self.assertIsNone(data["api_format"])
        with self.assertRaises(AppError):
            self._test(
                base_url="claude-cli://default",
                kind="remote",
                api_format="responses",
                acknowledged=True,
            )


def _run_json(data_root, *args):
    result = subprocess.run(
        [sys.executable, "-m", "kassiber", "--data-root", str(data_root), "--machine", *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
        timeout=60,
        stdin=subprocess.DEVNULL,
    )
    return result.returncode, json.loads(result.stdout)


class ProviderTestCliTest(ProviderTestCase):
    def test_providers_test_detects_and_stores_the_format(self):
        self.provider.queue("GET", "models", Reply(json_body={"data": [{"id": "m"}]}))
        self.provider.queue("POST", "responses", Reply(404, json_body={"detail": "Not Found"}))
        self.provider.queue(
            "POST", "chat/completions", chat_stream(chunk({"content": "OK"}), chunk({}, finish="stop"))
        )
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-test-cli-") as tmp:
            data_root = Path(tmp) / "data"
            code, payload = _run_json(
                data_root,
                "ai", "providers", "create", "lan",
                "--base-url", self.provider.base_url,
                "--kind", "local",
                "--default-model", "m",
            )
            self.assertEqual(code, 0, payload)
            code, payload = _run_json(data_root, "ai", "providers", "test", "lan")
            self.assertEqual(code, 0, payload)
            self.assertEqual(payload["kind"], "ai.providers.test")
            self.assertEqual(payload["data"]["detected_api_format"], "chat_completions")
            self.assertTrue(payload["data"]["tools_accepted"])
            self.assertTrue(payload["data"]["detection_saved"])
            code, payload = _run_json(data_root, "ai", "providers", "get", "lan")
            self.assertEqual(payload["data"]["effective_api_format"], "chat_completions")

    def test_without_a_model_the_test_fails_instead_of_passing_on_models(self):
        self.provider.queue("GET", "models", Reply(json_body={"data": [{"id": "a"}, {"id": "b"}]}))
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-test-model-") as tmp:
            data_root = Path(tmp) / "data"
            code, payload = _run_json(
                data_root,
                "ai", "providers", "create", "lan",
                "--base-url", self.provider.base_url,
            )
            self.assertEqual(code, 0, payload)
            code, payload = _run_json(data_root, "ai", "providers", "test", "lan")
        self.assertNotEqual(code, 0)
        self.assertEqual(payload["error"]["code"], "ai_model_required")
        self.assertEqual(payload["error"]["details"]["model_count"], 2)
        self.assertEqual([r["method"] for r in self.provider.requests], ["GET"])

    def test_an_unacknowledged_remote_provider_is_not_contacted(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-test-ack-") as tmp:
            data_root = Path(tmp) / "data"
            code, payload = _run_json(
                data_root,
                "ai", "providers", "create", "remote-one",
                "--base-url", self.provider.base_url,
                "--kind", "remote",
                "--default-model", "m",
            )
            self.assertEqual(code, 0, payload)
            code, payload = _run_json(data_root, "ai", "providers", "test", "remote-one")
            self.assertNotEqual(code, 0)
            self.assertEqual(payload["error"]["code"], "ai_remote_ack_required")
        self.assertEqual(self.provider.requests, [])


class OmitStoreFieldSettingTest(unittest.TestCase):
    """The explicit choice to send Chat Completions without `store: false`."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-store-setting-")
        self.conn = open_db(str(Path(self._tmp.name) / "data"))
        self.ctx = _ctx(self.conn)

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def test_off_by_default_and_round_trips_through_the_daemon(self):
        listed = _request(self.ctx, "ai.providers.list")["data"]["providers"]
        self.assertTrue(listed)
        for row in listed:
            self.assertIs(row["omit_store_field"], False)
        created = _request(
            self.ctx,
            "ai.providers.create",
            {"name": "gw", "base_url": "https://llm.example.test/v1", "kind": "remote", "acknowledged": True},
        )["data"]
        self.assertIs(created["omit_store_field"], False)
        updated = _request(
            self.ctx, "ai.providers.update", {"name": "gw", "omit_store_field": True}
        )["data"]
        self.assertIs(updated["omit_store_field"], True)
        self.assertNotIn("api_key", updated)
        self.assertIs(get_db_ai_provider(self.conn, "gw")["omit_store_field"], True)
        created = _request(
            self.ctx,
            "ai.providers.create",
            {"name": "gw2", "base_url": "https://llm.example.test/v1", "kind": "remote", "omit_store_field": True},
        )["data"]
        self.assertIs(created["omit_store_field"], True)
        for args in (
            {"name": "gw", "omit_store_field": "true"},
            {"name": "gw", "omit_store_field": 1},
        ):
            with self.subTest(args=args), self.assertRaises(AppError) as raised:
                _request(self.ctx, "ai.providers.update", args)
            self.assertEqual(raised.exception.code, "validation")

    def test_a_new_address_resets_the_exemption_unless_restated(self):
        create_db_ai_provider(
            self.conn, "gw", "https://llm.example.test/v1", kind="remote", omit_store_field=True
        )
        # Chosen for one endpoint: another one goes back to the default.
        updated = _request(
            self.ctx, "ai.providers.update", {"name": "gw", "base_url": "https://llm.example.test/v2"}
        )["data"]
        self.assertIs(updated["omit_store_field"], False)
        updated = _request(
            self.ctx,
            "ai.providers.update",
            {"name": "gw", "base_url": "https://other.example.test/v1", "omit_store_field": True},
        )["data"]
        self.assertIs(updated["omit_store_field"], True)
        # Other edits leave it alone.
        updated = _request(self.ctx, "ai.providers.update", {"name": "gw", "notes": "same"})["data"]
        self.assertIs(updated["omit_store_field"], True)

    def test_cli_agents_take_no_store_choice(self):
        with self.assertRaises(AppError) as raised:
            _request(self.ctx, "ai.providers.update", {"name": "codex", "omit_store_field": True})
        self.assertEqual(raised.exception.code, "validation")
        create_db_ai_provider(self.conn, "gw", "https://llm.example.test/v1", kind="remote", omit_store_field=True)
        # Moved to a CLI agent, it goes back to the default.
        update_db_ai_provider(self.conn, "gw", {"base_url": "codex-cli://default"})
        self.assertIs(get_db_ai_provider(self.conn, "gw")["omit_store_field"], False)

    def test_an_existing_book_gets_the_default(self):
        self.conn.execute("ALTER TABLE ai_providers DROP COLUMN omit_store_field")
        self.conn.commit()
        self.conn.close()
        self.conn = open_db(str(Path(self._tmp.name) / "data"))
        for row in _request(_ctx(self.conn), "ai.providers.list")["data"]["providers"]:
            self.assertIs(row["omit_store_field"], False)


class StoredKeyOriginTest(unittest.TestCase):
    """A stored key belongs to its origin: scheme, host and port."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-key-origin-")
        self.conn = open_db(str(Path(self._tmp.name) / "data"))
        self.ctx = _ctx(self.conn)

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def _key(self, name):
        return get_ai_provider_api_key_for_use(get_db_ai_provider(self.conn, name))

    def test_a_path_change_on_the_same_origin_keeps_the_key(self):
        create_db_ai_provider(
            self.conn, "gw", "https://llm.example.test/v1", kind="remote", api_key="sk-kept"
        )
        for url in ("https://llm.example.test/openai/v1", "https://LLM.example.test:443/v2"):
            update_db_ai_provider(self.conn, "gw", {"base_url": url})
            self.assertEqual(self._key("gw"), "sk-kept", url)
        # Through the daemon, as the desktop saves it.
        updated = _request(
            self.ctx, "ai.providers.update", {"name": "gw", "base_url": "https://llm.example.test/v3"}
        )["data"]
        self.assertIs(updated["has_api_key"], True)
        self.assertEqual(self._key("gw"), "sk-kept")

    def test_another_origin_clears_the_key(self):
        for url in (
            "https://other.example.test/v1",
            "http://llm.example.test/v1",
            "https://llm.example.test:8443/v1",
        ):
            with self.subTest(url=url):
                self.conn.execute("DELETE FROM ai_providers WHERE name = 'gw'")
                self.conn.commit()
                create_db_ai_provider(
                    self.conn, "gw", "https://llm.example.test/v1", kind="remote", api_key="sk-old"
                )
                updated = _request(
                    self.ctx, "ai.providers.update", {"name": "gw", "base_url": url}
                )["data"]
                self.assertIs(updated["has_api_key"], False)
                self.assertIsNone(self._key("gw"))


class ProviderFormatCliTest(unittest.TestCase):
    def test_store_field_flags(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-store-cli-") as tmp:
            data_root = Path(tmp) / "data"
            code, payload = _run_json(
                data_root, "ai", "providers", "create", "gw",
                "--base-url", "https://llm.example.test/v1", "--kind", "remote",
            )
            self.assertEqual(code, 0, payload)
            self.assertIs(payload["data"]["omit_store_field"], False)
            code, payload = _run_json(
                data_root, "ai", "providers", "update", "gw", "--omit-store-field"
            )
            self.assertEqual(code, 0, payload)
            self.assertIs(payload["data"]["omit_store_field"], True)
            code, payload = _run_json(
                data_root, "ai", "providers", "update", "gw", "--send-store-field"
            )
            self.assertEqual(code, 0, payload)
            self.assertIs(payload["data"]["omit_store_field"], False)
            code, payload = _run_json(
                data_root, "ai", "providers", "create", "gw2",
                "--base-url", "https://llm.example.test/v1", "--kind", "remote", "--omit-store-field",
            )
            self.assertEqual(code, 0, payload)
            self.assertIs(payload["data"]["omit_store_field"], True)


    def test_api_format_flag_and_clearing_a_detection(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-format-cli-") as tmp:
            data_root = Path(tmp) / "data"
            code, payload = _run_json(
                data_root,
                "ai",
                "providers",
                "create",
                "lan",
                "--base-url",
                "http://127.0.0.1:1234/v1",
                "--kind",
                "local",
                "--api-format",
                "chat-completions",
            )
            self.assertEqual(code, 0, payload)
            self.assertEqual(payload["data"]["api_format"], "chat_completions")
            self.assertEqual(payload["data"]["effective_api_format"], "chat_completions")

            code, payload = _run_json(
                data_root, "ai", "providers", "update", "lan", "--api-format", "auto"
            )
            self.assertEqual(code, 0, payload)
            self.assertEqual(payload["data"]["effective_api_format"], "responses")

            code, payload = _run_json(
                data_root, "ai", "providers", "update", "codex", "--api-format", "responses"
            )
            self.assertNotEqual(code, 0)
            self.assertEqual(payload["error"]["code"], "validation")

            code, payload = _run_json(
                data_root, "ai", "providers", "update", "lan", "--clear", "detected_api_format"
            )
            self.assertEqual(code, 0, payload)
            self.assertIsNone(payload["data"]["detected_api_format"])


class ProviderCommandReferenceTest(unittest.TestCase):
    """The checked-in skill and the in-product AI both learn the provider flags."""

    def test_command_references_document_formats_kind_inference_and_test(self):
        from kassiber.ai.tools import read_skill_reference

        references = {
            "skills/kassiber/SKILL.md": (ROOT / "skills/kassiber/SKILL.md").read_text(
                encoding="utf-8"
            ),
            "command-templates": read_skill_reference("command-templates")["content"],
        }
        for name, text in references.items():
            with self.subTest(reference=name):
                for fact in (
                    "--api-format",
                    "chat-completions",
                    "ai providers test",
                    "--acknowledge",
                    "reasoning_effort_unavailable",
                    "--omit-store-field",
                    "store_accepted",
                ):
                    self.assertIn(fact, text)


if __name__ == "__main__":
    unittest.main()
