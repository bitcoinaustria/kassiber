"""Unit tests for the AI core (SSE parser, error mapping, provider CRUD).

Stdlib-only, no pytest dep. Mirrors `tests/test_cli_smoke.py` style. Smoke
tests for the CLI/daemon surface live in test_cli_smoke.py; these tests
cover the underlying primitives directly so failures point at one layer.
"""

from __future__ import annotations

import io
import json
import socket
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch

from kassiber.ai import (
    create_db_ai_provider,
    delete_db_ai_provider,
    get_db_ai_provider,
    list_db_ai_providers,
    redact_ai_provider_for_output,
    require_ai_provider_acknowledged,
    resolve_ai_provider,
    set_default_ai_provider,
    clear_default_ai_provider,
    update_db_ai_provider,
    seed_default_ai_provider_if_empty,
)
from kassiber.ai.client import (
    DEFAULT_TIMEOUT_SECONDS,
    OpenAICompatClient,
    ToolCallAccumulator,
    parse_sse_chunks,
    _http_error_app_error,
    _network_error_app_error,
)
from kassiber.ai.prompt import (
    DEFAULT_KASSIBER_SYSTEM_PROMPT,
    build_chat_messages,
    build_openai_tools,
)
from kassiber.ai.tools import (
    SKILL_REFERENCE_NAMES,
    get_tool,
    read_skill_reference,
    redact_tool_arguments,
    summarize_tool_call,
)
from kassiber.ai.providers import list_with_default
from kassiber.db import open_db
from kassiber.errors import AppError


class SseParserTest(unittest.TestCase):
    """The SSE parser is the load-bearing piece for streaming. Pin the
    edge cases the wire format throws at us in practice."""

    def _parse(self, text):
        return list(parse_sse_chunks(text.splitlines(keepends=True)))

    def test_single_event(self):
        chunks = self._parse('data: {"choices":[{"delta":{"content":"hi"}}]}\n\n')
        self.assertEqual(len(chunks), 1)
        self.assertEqual(chunks[0]["choices"][0]["delta"]["content"], "hi")

    def test_multiple_events(self):
        text = (
            'data: {"choices":[{"delta":{"content":"a"}}]}\n\n'
            'data: {"choices":[{"delta":{"content":"b"}}]}\n\n'
        )
        chunks = self._parse(text)
        self.assertEqual([c["choices"][0]["delta"]["content"] for c in chunks], ["a", "b"])

    def test_done_terminates_stream(self):
        text = (
            'data: {"choices":[{"delta":{"content":"a"}}]}\n\n'
            "data: [DONE]\n\n"
            'data: {"choices":[{"delta":{"content":"b"}}]}\n\n'
        )
        chunks = self._parse(text)
        # Anything after [DONE] is intentionally dropped.
        self.assertEqual(len(chunks), 1)
        self.assertEqual(chunks[0]["choices"][0]["delta"]["content"], "a")

    def test_comments_and_unknown_fields_ignored(self):
        text = (
            ": keepalive\n"
            "event: message\n"
            "id: 1\n"
            'data: {"choices":[{"delta":{"content":"a"}}]}\n'
            "\n"
        )
        chunks = self._parse(text)
        self.assertEqual(len(chunks), 1)

    def test_multi_line_data_concatenated(self):
        # Per the SSE spec, consecutive `data:` lines join with `\n` before
        # the event boundary. JSON tolerates internal whitespace, so a
        # well-formed pretty-printed object across lines parses cleanly.
        text = (
            'data: {"choices":[\n'
            'data:   {"delta":{"content":"hi"}}\n'
            "data: ]}\n"
            "\n"
        )
        chunks = self._parse(text)
        self.assertEqual(len(chunks), 1)
        self.assertEqual(chunks[0]["choices"][0]["delta"]["content"], "hi")

    def test_malformed_payload_skipped(self):
        text = (
            "data: not-json\n\n"
            'data: {"choices":[{"delta":{"content":"ok"}}]}\n\n'
        )
        chunks = self._parse(text)
        self.assertEqual(len(chunks), 1)
        self.assertEqual(chunks[0]["choices"][0]["delta"]["content"], "ok")

    def test_trailing_chunk_without_blank_line(self):
        # Some servers don't emit a final blank line before closing the
        # connection. The parser should still flush the buffered data.
        text = 'data: {"choices":[{"delta":{"content":"end"}}]}'
        chunks = self._parse(text)
        self.assertEqual(len(chunks), 1)
        self.assertEqual(chunks[0]["choices"][0]["delta"]["content"], "end")


class ToolCatalogPromptTest(unittest.TestCase):
    def test_tool_catalog_stability(self):
        expected_tool_names = {
            "status",
            "ui_overview_snapshot",
            "ui_transactions_list",
            "ui_wallets_list",
            "ui_backends_list",
            "ui_profiles_snapshot",
            "ui_reports_capital_gains",
            "ui_journals_snapshot",
            "ui_journals_quarantine",
            "ui_journals_transfers_list",
            "ui_journals_process",
            "ui_rates_summary",
            "ui_workspace_health",
            "ui_next_actions",
            "read_skill_reference",
            "ui_wallets_sync",
        }
        tool_names = {
            tool["function"]["name"]
            for tool in build_openai_tools()
            if tool.get("type") == "function"
        }
        self.assertEqual(tool_names, expected_tool_names)
        for tool_name in tool_names:
            self.assertRegex(tool_name, r"^[A-Za-z0-9_-]{1,64}$")
        self.assertEqual(get_tool("ui_overview_snapshot").name, "ui.overview.snapshot")
        self.assertEqual(get_tool("ui_workspace_health").name, "ui.workspace.health")
        self.assertEqual(get_tool("ui_next_actions").kind_class, "read_only")
        self.assertEqual(get_tool("ui_wallets_list").kind_class, "read_only")
        self.assertEqual(get_tool("ui_backends_list").kind_class, "read_only")
        self.assertEqual(get_tool("ui_journals_quarantine").kind_class, "read_only")
        self.assertEqual(get_tool("ui_rates_summary").kind_class, "read_only")
        self.assertEqual(get_tool("ui_wallets_sync").name, "ui.wallets.sync")
        self.assertEqual(get_tool("ui.wallets.sync").kind_class, "mutating")
        self.assertEqual(get_tool("ui_journals_process").name, "ui.journals.process")
        self.assertEqual(get_tool("ui.journals.process").kind_class, "mutating")
        self.assertIn("ui_wallets_sync", tool_names)
        self.assertIn("ui_journals_process", tool_names)

    def test_mutating_tool_preview_redacts_secret_like_arguments(self):
        tool = get_tool("ui.wallets.sync")
        self.assertEqual(
            redact_tool_arguments(
                {
                    "wallet": "cold",
                    "descriptor": "wpkh(xpub...)",
                    "mnemonic": "abandon abandon abandon",
                    "recovery_phrase": "abandon abandon abandon",
                    "seed": "00" * 32,
                    "wif": "Kxabc",
                    "xprv": "xprv9s21...",
                    "nested": {"api_token": "secret"},
                }
            ),
            {
                "wallet": "cold",
                "descriptor": "<redacted>",
                "mnemonic": "<redacted>",
                "recovery_phrase": "<redacted>",
                "seed": "<redacted>",
                "wif": "<redacted>",
                "xprv": "<redacted>",
                "nested": {"api_token": "<redacted>"},
            },
        )
        self.assertEqual(summarize_tool_call(tool, {"wallet": "cold"}), "Sync wallet cold")
        journal_tool = get_tool("ui.journals.process")
        self.assertEqual(summarize_tool_call(journal_tool, {}), "Process journals")

    def test_read_skill_reference_allowlist(self):
        self.assertIn("index", SKILL_REFERENCE_NAMES)
        self.assertIn("wallets-backends", SKILL_REFERENCE_NAMES)
        index = read_skill_reference("index")
        self.assertEqual(index["name"], "index")
        self.assertIn("Kassiber In-App Skill Index", index["content"])
        self.assertIn("wallets-backends", index["content"])
        self.assertNotIn("kassiber backends create my-esplora", index["content"])
        reference = read_skill_reference("wallets-backends")
        self.assertEqual(reference["name"], "wallets-backends")
        self.assertIn("Wallets and Backends", reference["content"])
        with self.assertRaises(AppError) as ctx:
            read_skill_reference("../AGENTS")
        self.assertEqual(ctx.exception.code, "tool_not_allowed")

    def test_system_prompt_omits_reference_bodies_until_requested(self):
        messages = build_chat_messages(
            [{"role": "user", "content": "What is pending?"}],
            system_prompt_kind="kassiber",
        )
        self.assertEqual(messages[0]["role"], "system")
        self.assertIn("read_skill_reference", messages[0]["content"])
        self.assertIn('name "index"', messages[0]["content"])
        self.assertIn("When tools show stale journals", messages[0]["content"])
        self.assertNotIn("kassiber backends create my-esplora", messages[0]["content"])
        self.assertLess(len(DEFAULT_KASSIBER_SYSTEM_PROMPT), 2000)


class ToolCallAccumulatorTest(unittest.TestCase):
    def test_accumulates_partial_arguments(self):
        accumulator = ToolCallAccumulator()
        first = accumulator.add_delta(
            [
                {
                    "index": 0,
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "read_skill_reference",
                        "arguments": '{"name":"wallets',
                    },
                }
            ]
        )
        self.assertEqual(first[0]["function"]["arguments"], '{"name":"wallets')
        second = accumulator.add_delta(
            [
                {
                    "index": 0,
                    "function": {
                        "arguments": '-backends"}',
                    },
                }
            ]
        )
        self.assertEqual(second[0]["id"], "call_1")
        self.assertEqual(second[0]["function"]["name"], "read_skill_reference")
        self.assertEqual(
            second[0]["function"]["arguments"],
            '{"name":"wallets-backends"}',
        )


class HttpErrorMappingTest(unittest.TestCase):
    """`_http_error_app_error` decides whether errors are retryable, what
    `code` they get, and what hint the user sees. Pin those mappings."""

    def _err(self, status, body=b""):
        return urllib.error.HTTPError(
            url="http://test/v1/chat/completions",
            code=status,
            msg="",
            hdrs=None,  # type: ignore[arg-type]
            fp=io.BytesIO(body),
        )

    def test_401_is_auth_failed(self):
        app_err = _http_error_app_error(self._err(401))
        self.assertEqual(app_err.code, "ai_auth_failed")
        self.assertFalse(app_err.retryable)

    def test_403_is_auth_failed(self):
        app_err = _http_error_app_error(self._err(403))
        self.assertEqual(app_err.code, "ai_auth_failed")

    def test_429_is_rate_limited_and_retryable(self):
        app_err = _http_error_app_error(self._err(429))
        self.assertEqual(app_err.code, "ai_rate_limited")
        self.assertTrue(app_err.retryable)

    def test_500_is_unavailable_and_retryable(self):
        app_err = _http_error_app_error(self._err(503))
        self.assertEqual(app_err.code, "ai_unavailable")
        self.assertTrue(app_err.retryable)

    def test_400_is_request_invalid(self):
        app_err = _http_error_app_error(self._err(400, b'{"error":"bad"}'))
        self.assertEqual(app_err.code, "ai_request_invalid")
        self.assertFalse(app_err.retryable)
        self.assertIn("body", app_err.details)

    def test_404_is_request_invalid(self):
        app_err = _http_error_app_error(self._err(404))
        self.assertEqual(app_err.code, "ai_request_invalid")

    def test_network_error_is_unavailable(self):
        app_err = _network_error_app_error(ConnectionRefusedError("nope"))
        self.assertEqual(app_err.code, "ai_unavailable")
        self.assertTrue(app_err.retryable)


class ClientDefaultsTest(unittest.TestCase):
    """Constructor invariants the rest of the system relies on."""

    def test_defaults(self):
        client = OpenAICompatClient(base_url="http://localhost:11434/v1")
        self.assertEqual(client.timeout, DEFAULT_TIMEOUT_SECONDS)
        self.assertIsNone(client.api_key)
        headers = client._headers(json_body=True)
        self.assertEqual(headers["Content-Type"], "application/json")
        self.assertNotIn("Authorization", headers)

    def test_bearer_when_key_present(self):
        client = OpenAICompatClient(base_url="http://x/v1", api_key="sk-test")
        headers = client._headers(json_body=False, accept_sse=True)
        self.assertEqual(headers["Authorization"], "Bearer sk-test")
        self.assertEqual(headers["Accept"], "text/event-stream")


class ListModelsStrictModeTest(unittest.TestCase):
    """`list_models(strict=True)` must surface 4xx so `ai.test_connection`
    can tell a misconfigured base URL apart from a provider that simply
    doesn't expose `/v1/models`."""

    def _http_error(self, status):
        return urllib.error.HTTPError(
            url="http://x/v1/models",
            code=status,
            msg="",
            hdrs=None,  # type: ignore[arg-type]
            fp=io.BytesIO(b""),
        )

    class _FakeResponse:
        def __init__(self, payload: bytes):
            self._payload = payload

        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def read(self):
            return self._payload

    def test_default_mode_swallows_4xx_to_empty_list(self):
        # Picker UX: providers that skip /v1/models still let the user fall
        # back to a configured default_model.
        with patch("urllib.request.urlopen", side_effect=self._http_error(404)):
            client = OpenAICompatClient(base_url="http://x/v1")
            self.assertEqual(client.list_models(), [])

    def test_strict_mode_propagates_4xx(self):
        with patch("urllib.request.urlopen", side_effect=self._http_error(404)):
            client = OpenAICompatClient(base_url="http://x/v1")
            with self.assertRaises(AppError) as ctx:
                client.list_models(strict=True)
            self.assertEqual(ctx.exception.code, "ai_request_invalid")

    def test_strict_mode_rejects_invalid_json_200(self):
        with patch(
            "urllib.request.urlopen",
            return_value=self._FakeResponse(b"<html>not json</html>"),
        ):
            client = OpenAICompatClient(base_url="http://x/v1")
            self.assertEqual(client.list_models(), [])
            with self.assertRaises(AppError) as ctx:
                client.list_models(strict=True)
            self.assertEqual(ctx.exception.code, "ai_request_invalid")

    def test_strict_mode_rejects_unexpected_200_shape(self):
        with patch(
            "urllib.request.urlopen",
            return_value=self._FakeResponse(b'{"ok":true}'),
        ):
            client = OpenAICompatClient(base_url="http://x/v1")
            self.assertEqual(client.list_models(), [])
            with self.assertRaises(AppError) as ctx:
                client.list_models(strict=True)
            self.assertEqual(ctx.exception.code, "ai_request_invalid")

    def test_strict_mode_does_not_change_auth_failure(self):
        # 401 was never swallowed; strict mode shouldn't change that path.
        with patch("urllib.request.urlopen", side_effect=self._http_error(401)):
            client = OpenAICompatClient(base_url="http://x/v1")
            with self.assertRaises(AppError) as ctx:
                client.list_models()
            self.assertEqual(ctx.exception.code, "ai_auth_failed")
            with self.assertRaises(AppError) as ctx:
                client.list_models(strict=True)
            self.assertEqual(ctx.exception.code, "ai_auth_failed")


class ChatBodyContractTest(unittest.TestCase):
    """Caller-supplied options must not override the OpenAI wire contract."""

    class _ReadResponse:
        def __init__(self, payload: bytes):
            self._payload = payload

        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def read(self):
            return self._payload

    class _StreamResponse:
        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def __iter__(self):
            yield b"data: [DONE]\n"
            yield b"\n"

    def test_chat_forces_reserved_fields_after_options(self):
        captured: dict = {}

        def fake_urlopen(request, timeout=None):
            captured.update(json.loads(request.data.decode("utf-8")))
            return self._ReadResponse(
                b'{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}'
            )

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            client = OpenAICompatClient(base_url="http://x/v1")
            client.chat(
                messages=[{"role": "user", "content": "real"}],
                model="real-model",
                options={
                    "stream": True,
                    "model": "wrong-model",
                    "messages": [],
                    "temperature": 0.2,
                },
            )
        self.assertEqual(captured["stream"], False)
        self.assertEqual(captured["model"], "real-model")
        self.assertEqual(captured["messages"], [{"role": "user", "content": "real"}])
        self.assertEqual(captured["temperature"], 0.2)

    def test_stream_chat_forces_stream_true_after_options(self):
        captured: dict = {}

        def fake_urlopen(request, timeout=None):
            captured.update(json.loads(request.data.decode("utf-8")))
            return self._StreamResponse()

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            client = OpenAICompatClient(base_url="http://x/v1")
            list(
                client.stream_chat(
                    messages=[{"role": "user", "content": "real"}],
                    model="real-model",
                    options={"stream": False, "model": "wrong-model", "messages": []},
                )
            )
        self.assertEqual(captured["stream"], True)
        self.assertEqual(captured["model"], "real-model")
        self.assertEqual(captured["messages"], [{"role": "user", "content": "real"}])

    def test_chat_sends_explicit_tools_after_options(self):
        captured: dict = {}
        tools = [{"type": "function", "function": {"name": "status", "parameters": {}}}]

        def fake_urlopen(request, timeout=None):
            captured.update(json.loads(request.data.decode("utf-8")))
            return self._ReadResponse(
                b'{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}'
            )

        with patch("urllib.request.urlopen", side_effect=fake_urlopen):
            client = OpenAICompatClient(base_url="http://x/v1")
            client.chat(
                messages=[{"role": "user", "content": "real"}],
                model="real-model",
                tools=tools,
                tool_choice="auto",
                options={"tools": [], "tool_choice": "none"},
            )
        self.assertEqual(captured["tools"], tools)
        self.assertEqual(captured["tool_choice"], "auto")


class ChatReasoningPassthroughTest(unittest.TestCase):
    """OpenAI o1/o3 and Ollama's OpenAI-compat shim for Qwen3 / Gemma
    reasoning builds emit a structured `reasoning` field on the chat
    message. Surface it on the result so callers (CLI envelope, future
    tool-use plumbing, the assistant UI) can show or ignore it without
    re-parsing the upstream payload."""

    class _FakeResponse:
        def __init__(self, payload: bytes):
            self._payload = payload

        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def read(self):
            return self._payload

    def test_chat_includes_reasoning_when_provider_emits_it(self):
        payload = (
            b'{"choices":[{"message":{"role":"assistant","content":"hi",'
            b'"reasoning":"thinking out loud"},"finish_reason":"stop"}],'
            b'"usage":{"prompt_tokens":1,"completion_tokens":2}}'
        )
        with patch(
            "urllib.request.urlopen",
            return_value=self._FakeResponse(payload),
        ):
            client = OpenAICompatClient(base_url="http://x/v1")
            result = client.chat(
                messages=[{"role": "user", "content": "x"}], model="m"
            )
        self.assertEqual(result["content"], "hi")
        self.assertEqual(result["reasoning"], "thinking out loud")
        self.assertEqual(result["finish_reason"], "stop")

    def test_chat_omits_reasoning_when_provider_does_not_emit_it(self):
        payload = (
            b'{"choices":[{"message":{"role":"assistant","content":"plain"},'
            b'"finish_reason":"stop"}]}'
        )
        with patch(
            "urllib.request.urlopen",
            return_value=self._FakeResponse(payload),
        ):
            client = OpenAICompatClient(base_url="http://x/v1")
            result = client.chat(
                messages=[{"role": "user", "content": "x"}], model="m"
            )
        self.assertEqual(result["content"], "plain")
        self.assertNotIn("reasoning", result)


class StreamChatErrorMappingTest(unittest.TestCase):
    """Read-time socket failures during a stream must map to retryable
    `ai_unavailable`, not bubble up as raw `URLError`/`OSError` and
    surface as non-retryable `internal_error` from the daemon thread."""

    class _ResponseRaisingMidIteration:
        """Fake urlopen response that yields one SSE event then breaks.

        urllib's response object iterates line-by-line, so the mock needs
        to surface the data line and the event-boundary blank line as
        separate iterations before raising — otherwise the parser is
        still waiting for the boundary when the timeout fires.
        """

        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def __iter__(self):
            yield b'data: {"choices":[{"delta":{"content":"hi"}}]}\n'
            yield b"\n"
            raise socket.timeout("read timed out mid-stream")

    def test_socket_timeout_mid_stream_maps_to_ai_unavailable(self):
        with patch(
            "urllib.request.urlopen",
            return_value=self._ResponseRaisingMidIteration(),
        ):
            client = OpenAICompatClient(base_url="http://x/v1")
            it = client.stream_chat(
                messages=[{"role": "user", "content": "hi"}], model="m"
            )
            # The first delta should arrive normally; the second iteration
            # raises the mapped AppError.
            first = next(it)
            self.assertEqual(first.delta.get("content"), "hi")
            with self.assertRaises(AppError) as ctx:
                next(it)
            self.assertEqual(ctx.exception.code, "ai_unavailable")
            self.assertTrue(ctx.exception.retryable)


class ProvidersCrudTest(unittest.TestCase):
    """SQLite-backed CRUD round-trip; the API key never leaks into output."""

    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-")
        cls.data_root = Path(cls._tmp.name) / "data"

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _conn(self):
        return open_db(str(self.data_root))

    def test_seed_inserts_local_ollama_when_empty(self):
        conn = self._conn()
        try:
            providers = list_db_ai_providers(conn)
            self.assertEqual(len(providers), 1)
            self.assertEqual(providers[0]["name"], "ollama")
            self.assertEqual(providers[0]["kind"], "local")
            self.assertEqual(providers[0]["base_url"], "http://localhost:11434/v1")
        finally:
            conn.close()

    def test_seed_does_not_recreate_after_delete(self):
        # Use a fresh DB so the previous test's state doesn't bleed in.
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-seed-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                seed_default_ai_provider_if_empty(conn)
                clear_default_ai_provider(conn)
                delete_db_ai_provider(conn, "ollama")
                seed_default_ai_provider_if_empty(conn)
                self.assertEqual(list_db_ai_providers(conn), [])
            finally:
                conn.close()

    def test_create_and_get_with_remote_kind(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-crud-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                created = create_db_ai_provider(
                    conn,
                    "openai",
                    "https://api.openai.com/v1",
                    api_key="sk-secret",
                    default_model="gpt-4o-mini",
                    kind="remote",
                    notes="Cloud OpenAI.",
                )
                self.assertEqual(created["name"], "openai")
                self.assertEqual(created["kind"], "remote")
                self.assertIsNone(created["acknowledged_at"])

                fetched = get_db_ai_provider(conn, "openai")
                self.assertEqual(fetched["api_key"], "sk-secret")

                redacted = redact_ai_provider_for_output(fetched, default_name="ollama")
                self.assertNotIn("api_key", redacted)
                self.assertTrue(redacted["has_api_key"])
                self.assertFalse(redacted["is_default"])

                with self.assertRaises(AppError) as ctx:
                    require_ai_provider_acknowledged(fetched)
                self.assertEqual(ctx.exception.code, "ai_remote_ack_required")

                acknowledged = update_db_ai_provider(
                    conn,
                    "openai",
                    {"acknowledged": True},
                )
                require_ai_provider_acknowledged(acknowledged)
            finally:
                conn.close()

    def test_update_clears_and_sets_fields(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-update-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                create_db_ai_provider(
                    conn,
                    "openrouter",
                    "https://openrouter.ai/api/v1",
                    api_key="sk-old",
                    kind="remote",
                )
                updated = update_db_ai_provider(
                    conn,
                    "openrouter",
                    {"api_key": "sk-new", "default_model": "anthropic/claude-3.5-sonnet"},
                )
                self.assertEqual(updated["api_key"], "sk-new")
                self.assertEqual(updated["default_model"], "anthropic/claude-3.5-sonnet")

                cleared = update_db_ai_provider(
                    conn,
                    "openrouter",
                    {"clear": ["api_key", "default_model"]},
                )
                self.assertIsNone(cleared["api_key"])
                self.assertIsNone(cleared["default_model"])
            finally:
                conn.close()

    def test_default_pointer_roundtrip(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-default-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                create_db_ai_provider(
                    conn,
                    "remote-1",
                    "https://example.test/v1",
                    api_key="sk-x",
                    kind="remote",
                )
                set_default_ai_provider(conn, "remote-1")
                self.assertEqual(resolve_ai_provider(conn)["name"], "remote-1")

                clear_default_ai_provider(conn)
                with self.assertRaises(AppError) as ctx:
                    resolve_ai_provider(conn)
                self.assertEqual(ctx.exception.code, "ai_provider_not_configured")
            finally:
                conn.close()

    def test_list_with_default_payload_shape(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-list-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                payload = list_with_default(conn)
                self.assertIn("providers", payload)
                self.assertIn("default", payload)
                # Seeded ollama is the default.
                self.assertEqual(payload["default"], "ollama")
                self.assertEqual(payload["providers"][0]["name"], "ollama")
                # No raw api_key in any redacted row.
                for row in payload["providers"]:
                    self.assertNotIn("api_key", row)
            finally:
                conn.close()

    def test_acknowledge_local_kind_is_implicit(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-ack-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                created = create_db_ai_provider(
                    conn,
                    "lan-ollama",
                    "http://192.168.1.10:11434/v1",
                    kind="local",
                )
                self.assertIsNotNone(created["acknowledged_at"])
            finally:
                conn.close()

    def test_delete_refuses_active_default(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-delete-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                # ollama is the seeded default
                with self.assertRaises(AppError) as ctx:
                    delete_db_ai_provider(conn, "ollama")
                self.assertEqual(ctx.exception.code, "conflict")
            finally:
                conn.close()

    def test_invalid_kind_rejected(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-kind-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                with self.assertRaises(AppError) as ctx:
                    create_db_ai_provider(
                        conn,
                        "x",
                        "http://x/v1",
                        kind="invalid",
                    )
                self.assertEqual(ctx.exception.code, "validation")
            finally:
                conn.close()

    def test_invalid_base_url_rejected(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-url-") as tmp:
            conn = open_db(str(Path(tmp) / "data"))
            try:
                with self.assertRaises(AppError):
                    create_db_ai_provider(conn, "x", "no-scheme", kind="local")
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
