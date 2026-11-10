"""CLI proxy providers (`kind = "cli_proxy"`) exist only on the Rust backend.

The Rust store writes them into the shared `ai_providers` table. This
backend can show such a row, but every path that would contact it (chat,
model listing, the connection test) refuses it with
`ai_provider_unsupported` before any client is built, and it cannot create
one. See docs/reference/ai-cli-proxy.md.
"""

from __future__ import annotations

import queue
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber import daemon as daemon_runtime
from kassiber.ai import (
    create_db_ai_provider,
    get_db_ai_provider,
    require_ai_provider_contactable,
)
from kassiber.db import open_db
from kassiber.errors import AppError


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
        ctx,
        {"kind": kind, "request_id": "req-1", "args": args or {}},
        out=None,
    )
    return envelope


class CliProxyRustOnlyTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-cli-proxy-")
        self.conn = open_db(str(Path(self._tmp.name) / "data"))
        self.ctx = _ctx(self.conn)
        # As the Rust store leaves it: an acknowledged, enabled CLI proxy.
        create_db_ai_provider(
            self.conn,
            "cli-proxy",
            "http://127.0.0.1:8317/v1",
            kind="remote",
            acknowledged=True,
        )
        self.conn.execute("UPDATE ai_providers SET kind = 'cli_proxy' WHERE name = 'cli-proxy'")
        self.conn.commit()

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def assert_unsupported(self, error: AppError) -> None:
        self.assertEqual(error.code, "ai_provider_unsupported")
        self.assertEqual(error.details, {"provider": "cli-proxy", "kind": "cli_proxy"})
        self.assertIn("only the Rust backend", str(error))

    def test_display_reads_still_show_the_row(self):
        listed = _request(self.ctx, "ai.providers.list")["data"]["providers"]
        row = next(provider for provider in listed if provider["name"] == "cli-proxy")
        self.assertEqual(row["kind"], "cli_proxy")
        cached = _request(
            self.ctx, "ai.list_models", {"provider": "cli-proxy", "cached_only": True}
        )["data"]
        self.assertEqual(cached["models"], [])

    def test_every_contact_is_refused_before_a_client_exists(self):
        with self.assertRaises(AppError) as gate:
            require_ai_provider_contactable(get_db_ai_provider(self.conn, "cli-proxy"))
        self.assert_unsupported(gate.exception)
        requests = [
            ("ai.list_models", {"provider": "cli-proxy", "refresh": True}),
            (
                "ai.test_connection",
                {
                    "provider": "cli-proxy",
                    "base_url": "http://127.0.0.1:8317/v1",
                    "acknowledged": True,
                    "model": "claude-sonnet-5-5",
                },
            ),
            (
                "ai.chat",
                {
                    "provider": "cli-proxy",
                    "model": "claude-sonnet-5-5",
                    "messages": [{"role": "user", "content": "private prompt"}],
                    "persist": False,
                },
            ),
        ]
        with patch("kassiber.daemon.ai_client_for_locator") as client_factory, patch(
            "kassiber.daemon._run_ai_chat_stream"
        ) as worker:
            for kind, args in requests:
                with self.subTest(kind=kind), self.assertRaises(AppError) as refused:
                    _request(self.ctx, kind, args)
                self.assert_unsupported(refused.exception)
        client_factory.assert_not_called()
        worker.assert_not_called()

    def test_this_backend_cannot_create_one(self):
        with self.assertRaises(AppError) as refused:
            create_db_ai_provider(
                self.conn,
                "another",
                "http://127.0.0.1:8317/v1",
                kind="cli_proxy",
                acknowledged=True,
            )
        self.assertEqual(refused.exception.code, "validation")
        with self.assertRaises(AppError) as tested:
            _request(
                self.ctx,
                "ai.test_connection",
                {"base_url": "http://127.0.0.1:8317/v1", "kind": "cli_proxy"},
            )
        self.assertEqual(tested.exception.code, "validation")


if __name__ == "__main__":
    unittest.main()
