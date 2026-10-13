"""Which OpenAI wire format an HTTP provider speaks, and how it is detected.

`api_format` is the user's setting (auto, responses, chat_completions) and
`detected_api_format` what an endpoint was seen to answer. An explicit format
wins; auto uses the detection, else the Responses API, and falls back to Chat
Completions once when the server has no /responses.
"""

from __future__ import annotations

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
from kassiber.ai import create_db_ai_provider, get_db_ai_provider, update_db_ai_provider
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
            self.assertTrue({"api_format", "detected_api_format"} <= columns)
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
            daemon_runtime._run_ai_chat_stream(
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


class ProviderFormatCliTest(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
