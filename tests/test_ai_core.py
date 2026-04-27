"""Unit tests for the AI core (SSE parser, error mapping, provider CRUD).

Stdlib-only, no pytest dep. Mirrors `tests/test_cli_smoke.py` style. Smoke
tests for the CLI/daemon surface live in test_cli_smoke.py; these tests
cover the underlying primitives directly so failures point at one layer.
"""

from __future__ import annotations

import io
import tempfile
import unittest
import urllib.error
from pathlib import Path

from kassiber.ai import (
    create_db_ai_provider,
    delete_db_ai_provider,
    get_db_ai_provider,
    list_db_ai_providers,
    redact_ai_provider_for_output,
    resolve_ai_provider,
    set_default_ai_provider,
    clear_default_ai_provider,
    update_db_ai_provider,
    seed_default_ai_provider_if_empty,
)
from kassiber.ai.client import (
    DEFAULT_TIMEOUT_SECONDS,
    OpenAICompatClient,
    parse_sse_chunks,
    _http_error_app_error,
    _network_error_app_error,
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
