"""Per-provider settings: enable switch, default reasoning effort, and the
explicit, scoped runtime check behind Settings → AI providers.

The enable switch and default effort are business state, so the CLI, the
daemon and the desktop all read them from the same `ai_providers` row; these
tests pin that row and the daemon/CLI surfaces that write it.
"""

from __future__ import annotations

import argparse
import json
import queue
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber import daemon as daemon_runtime
from kassiber.ai import (
    create_db_ai_provider,
    contactable_cli_runtimes,
    get_db_ai_provider,
    set_default_ai_provider,
    update_db_ai_provider,
)
from kassiber.ai.broker_client import BrokerAIClient
from kassiber.ai.providers import list_with_default
from kassiber.cli import chat as cli_chat
from kassiber.db import open_db
from kassiber.errors import AppError


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


def _request(ctx, kind, args=None, request_id="req-1"):
    envelope, _ = daemon_runtime.handle_request(
        ctx,
        {"kind": kind, "request_id": request_id, "args": args or {}},
        out=None,
    )
    return envelope


class ProviderSettingsStorageTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-settings-")
        self.conn = open_db(str(Path(self._tmp.name) / "data"))

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def test_seeded_providers_start_enabled_without_default_effort(self):
        rows = list_with_default(self.conn)["providers"]
        self.assertTrue(rows)
        for row in rows:
            self.assertIs(row["enabled"], True, row["name"])
            self.assertIsNone(row["default_reasoning_effort"], row["name"])

    def test_enable_switch_and_default_effort_persist(self):
        update_db_ai_provider(
            self.conn,
            "claude",
            {"enabled": False, "default_reasoning_effort": "HIGH"},
        )
        row = get_db_ai_provider(self.conn, "claude")
        self.assertFalse(row["enabled"])
        self.assertEqual(row["default_reasoning_effort"], "high")
        listed = {p["name"]: p for p in list_with_default(self.conn)["providers"]}
        self.assertFalse(listed["claude"]["enabled"])
        self.assertEqual(listed["claude"]["default_reasoning_effort"], "high")

        # Unrelated edits keep both values.
        update_db_ai_provider(self.conn, "claude", {"notes": "edited"})
        row = get_db_ai_provider(self.conn, "claude")
        self.assertFalse(row["enabled"])
        self.assertEqual(row["default_reasoning_effort"], "high")

        update_db_ai_provider(
            self.conn,
            "claude",
            {"enabled": True, "clear": ["default_reasoning_effort"]},
        )
        row = get_db_ai_provider(self.conn, "claude")
        self.assertTrue(row["enabled"])
        self.assertIsNone(row["default_reasoning_effort"])

    def test_unknown_effort_is_rejected(self):
        with self.assertRaises(AppError) as ctx:
            update_db_ai_provider(self.conn, "codex", {"default_reasoning_effort": "turbo"})
        self.assertEqual(ctx.exception.code, "validation")
        with self.assertRaises(AppError):
            create_db_ai_provider(
                self.conn,
                "bad-effort",
                "http://127.0.0.1:9999/v1",
                default_reasoning_effort="fast",
            )
        self.assertIsNone(get_db_ai_provider(self.conn, "codex")["default_reasoning_effort"])

    def test_default_provider_cannot_be_disabled_and_disabled_cannot_be_default(self):
        set_default_ai_provider(self.conn, "ollama")
        with self.assertRaises(AppError) as ctx:
            update_db_ai_provider(self.conn, "ollama", {"enabled": False})
        self.assertEqual(ctx.exception.code, "conflict")
        self.assertTrue(get_db_ai_provider(self.conn, "ollama")["enabled"])

        update_db_ai_provider(self.conn, "codex", {"enabled": False})
        with self.assertRaises(AppError) as ctx:
            set_default_ai_provider(self.conn, "codex")
        self.assertEqual(ctx.exception.code, "conflict")

    def test_older_book_gains_columns_with_providers_enabled(self):
        data_root = str(Path(self._tmp.name) / "legacy")
        conn = open_db(data_root)
        list_with_default(conn)
        conn.execute("ALTER TABLE ai_providers DROP COLUMN enabled")
        conn.execute("ALTER TABLE ai_providers DROP COLUMN default_reasoning_effort")
        conn.commit()
        conn.close()
        conn = open_db(data_root)
        try:
            columns = {row["name"] for row in conn.execute("PRAGMA table_info(ai_providers)")}
            self.assertIn("enabled", columns)
            self.assertIn("default_reasoning_effort", columns)
            for row in list_with_default(conn)["providers"]:
                self.assertTrue(row["enabled"])
        finally:
            conn.close()


class ProviderSettingsDaemonTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-daemon-")
        self.conn = open_db(str(Path(self._tmp.name) / "data"))
        self.ctx = _ctx(self.conn)

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def test_update_kind_writes_enabled_and_effort(self):
        envelope = _request(
            self.ctx,
            "ai.providers.update",
            {"name": "copilot", "enabled": False, "default_reasoning_effort": "xhigh"},
        )
        self.assertEqual(envelope["kind"], "ai.providers.update")
        self.assertFalse(envelope["data"]["enabled"])
        self.assertEqual(envelope["data"]["default_reasoning_effort"], "xhigh")
        with self.assertRaises(AppError):
            _request(self.ctx, "ai.providers.update", {"name": "copilot", "enabled": "no"})

    def test_chat_refuses_disabled_provider_before_contacting_it(self):
        update_db_ai_provider(self.conn, "omlx", {"enabled": False})
        with patch("kassiber.daemon.ai_client_for_locator") as client_factory, patch(
            "kassiber.daemon._run_ai_chat_stream"
        ) as worker:
            with self.assertRaises(AppError) as ctx:
                _request(
                    self.ctx,
                    "ai.chat",
                    {
                        "provider": "omlx",
                        "model": "mlx-local",
                        "messages": [{"role": "user", "content": "private prompt"}],
                        "persist": False,
                    },
                )
        self.assertEqual(ctx.exception.code, "ai_provider_disabled")
        client_factory.assert_not_called()
        worker.assert_not_called()

    def test_cached_runtime_read_never_probes(self):
        with patch.object(BrokerAIClient, "runtime_status") as probe:
            envelope = _request(
                self.ctx,
                "ai.provider_runtime.status",
                {"provider": "claude", "cached_only": True},
            )
        probe.assert_not_called()
        self.assertEqual(envelope["data"]["providers"], [])
        self.assertIsNone(envelope["data"]["checked_at"])
        self.assertEqual(envelope["data"]["provider"], "claude")

    def test_provider_check_requires_acknowledgement(self):
        with patch.object(BrokerAIClient, "runtime_status") as probe:
            with self.assertRaises(AppError) as ctx:
                _request(
                    self.ctx,
                    "ai.provider_runtime.status",
                    {"provider": "claude", "refresh": True},
                )
        self.assertEqual(ctx.exception.code, "ai_remote_ack_required")
        probe.assert_not_called()

    def test_provider_check_probes_only_that_runtime_and_is_cached(self):
        _request(self.ctx, "ai.providers.acknowledge", {"name": "copilot"})
        status = {
            "provider": "copilot",
            "display_name": "GitHub Copilot",
            "state": "authentication_required",
            "message": "Run `copilot login` outside Kassiber.",
            "models": [],
        }
        with patch.object(BrokerAIClient, "runtime_status", return_value=[status]) as probe:
            checked = _request(
                self.ctx,
                "ai.provider_runtime.status",
                {"provider": "copilot", "refresh": True},
            )
            cached = _request(
                self.ctx,
                "ai.provider_runtime.status",
                {"provider": "copilot", "cached_only": True},
            )
            other = _request(
                self.ctx,
                "ai.provider_runtime.status",
                {"provider": "codex", "cached_only": True},
            )
        probe.assert_called_once_with("copilot")
        self.assertEqual(checked["data"]["providers"], [status])
        self.assertIsInstance(checked["data"]["checked_at"], str)
        self.assertEqual(cached["data"]["providers"], [status])
        self.assertEqual(cached["data"]["checked_at"], checked["data"]["checked_at"])
        self.assertEqual(other["data"]["providers"], [])

    def test_switch_and_effort_edits_keep_check_results(self):
        _request(self.ctx, "ai.providers.acknowledge", {"name": "codex"})
        status = {"provider": "codex", "state": "ready", "message": "Ready.", "models": []}
        with patch.object(BrokerAIClient, "runtime_status", return_value=[status]):
            _request(self.ctx, "ai.provider_runtime.status", {"provider": "codex", "refresh": True})

        def cached():
            return _request(
                self.ctx,
                "ai.provider_runtime.status",
                {"provider": "codex", "cached_only": True},
            )["data"]["providers"]

        _request(self.ctx, "ai.providers.update", {"name": "codex", "enabled": False})
        _request(
            self.ctx,
            "ai.providers.update",
            {"name": "codex", "clear": ["default_reasoning_effort"]},
        )
        self.assertEqual(cached(), [status])
        # A real configuration change still drops what discovery found.
        _request(self.ctx, "ai.providers.update", {"name": "codex", "notes": "edited"})
        self.assertEqual(cached(), [])

    def test_disabled_provider_check_never_starts_its_cli(self):
        _request(self.ctx, "ai.providers.acknowledge", {"name": "codex"})
        _request(self.ctx, "ai.providers.update", {"name": "codex", "enabled": False})
        with patch.object(BrokerAIClient, "runtime_status") as probe:
            with self.assertRaises(AppError) as ctx:
                _request(
                    self.ctx,
                    "ai.provider_runtime.status",
                    {"provider": "codex", "refresh": True},
                )
            # A plain read without refresh would probe on a cold cache too.
            with self.assertRaises(AppError):
                _request(self.ctx, "ai.provider_runtime.status", {"provider": "codex"})
            # The display read stays available and still never probes.
            cached = _request(
                self.ctx,
                "ai.provider_runtime.status",
                {"provider": "codex", "cached_only": True},
            )
        self.assertEqual(ctx.exception.code, "ai_provider_disabled")
        probe.assert_not_called()
        self.assertEqual(cached["data"]["providers"], [])

    def _acknowledge_cli_providers(self):
        for name in ("codex", "claude", "opencode", "copilot"):
            _request(self.ctx, "ai.providers.acknowledge", {"name": name})

    def test_all_runtimes_read_never_starts_an_unacknowledged_cli(self):
        # A fresh book: every CLI row is seeded and enabled, none acknowledged.
        # A plain read on a cold cache must not start one of them; Copilot's
        # probe alone would open an authenticated ACP session.
        with patch.object(BrokerAIClient, "_single_result") as single:
            for args in ({}, {"refresh": True}):
                envelope = _request(self.ctx, "ai.provider_runtime.status", args)
                self.assertEqual(envelope["data"]["providers"], [])
                # Nothing was checked, so no check time is claimed.
                self.assertIsNone(envelope["data"]["checked_at"])
        single.assert_not_called()

        _request(self.ctx, "ai.providers.acknowledge", {"name": "copilot"})
        with patch.object(
            BrokerAIClient,
            "_single_result",
            return_value=[{"provider": "copilot", "state": "ready"}],
        ) as single:
            envelope = _request(self.ctx, "ai.provider_runtime.status", {})
        # Only the acknowledged runtime is named, through the providers scope.
        single.assert_called_once_with(
            {"command": "status", "providers": ["copilot"]}, timeout=35.0
        )
        self.assertEqual(
            [row["provider"] for row in envelope["data"]["providers"]], ["copilot"]
        )

    def test_all_runtimes_probe_skips_switched_off_runtimes(self):
        self._acknowledge_cli_providers()
        with patch.object(BrokerAIClient, "runtime_status", return_value=[]) as probe:
            _request(self.ctx, "ai.provider_runtime.status", {"refresh": True})
        probe.assert_called_once_with(providers=["claude", "codex", "opencode", "copilot"])

        _request(self.ctx, "ai.providers.update", {"name": "codex", "enabled": False})
        with patch.object(
            BrokerAIClient,
            "runtime_status",
            side_effect=lambda runtime=None, providers=None: [
                {"provider": name, "state": "ready"} for name in providers or ()
            ],
        ) as probe:
            envelope = _request(self.ctx, "ai.provider_runtime.status", {"refresh": True})
        # One request for the others; codex is never asked for.
        probe.assert_called_once_with(providers=["claude", "opencode", "copilot"])
        self.assertEqual(
            sorted(row["provider"] for row in envelope["data"]["providers"]),
            ["claude", "copilot", "opencode"],
        )

    def test_all_runtimes_probe_stays_one_bounded_request_with_a_runtime_off(self):
        self._acknowledge_cli_providers()
        update_db_ai_provider(self.conn, "codex", {"enabled": False})
        update_db_ai_provider(self.conn, "opencode", {"enabled": False})
        requests: list[tuple[dict, float]] = []

        def single_result(request, *, timeout):
            requests.append((request, timeout))
            return [{"provider": name, "state": "ready"} for name in request.get("providers", ())]

        with patch.object(BrokerAIClient, "_single_result", side_effect=single_result):
            envelope = _request(self.ctx, "ai.provider_runtime.status", {"refresh": True})
        # One broker, one 35 s deadline, however many runtimes remain; not one
        # 35 s request per runtime in turn.
        self.assertEqual(
            requests,
            [({"command": "status", "providers": ["claude", "copilot"]}, 35.0)],
        )
        self.assertEqual(
            sorted(row["provider"] for row in envelope["data"]["providers"]),
            ["claude", "copilot"],
        )

    def test_runtime_is_contactable_through_any_enabled_acknowledged_row(self):
        self.assertEqual(contactable_cli_runtimes(self.conn), ())
        create_db_ai_provider(
            self.conn, "codex-work", "codex-cli://default", kind="remote", enabled=False
        )
        _request(self.ctx, "ai.providers.acknowledge", {"name": "codex"})
        self.assertEqual(contactable_cli_runtimes(self.conn), ("codex",))
        update_db_ai_provider(self.conn, "codex", {"enabled": False})
        self.assertEqual(contactable_cli_runtimes(self.conn), ())
        # Enabled but never acknowledged is still not consent.
        update_db_ai_provider(self.conn, "codex-work", {"enabled": True})
        self.assertEqual(contactable_cli_runtimes(self.conn), ())
        _request(self.ctx, "ai.providers.acknowledge", {"name": "codex-work"})
        self.assertEqual(contactable_cli_runtimes(self.conn), ("codex",))

    def test_disabled_http_provider_is_not_listed_or_tested(self):
        update_db_ai_provider(self.conn, "omlx", {"enabled": False})
        with patch("kassiber.daemon.ai_client_for_locator") as client_factory:
            with self.assertRaises(AppError) as listed:
                _request(self.ctx, "ai.list_models", {"provider": "omlx", "refresh": True})
            with self.assertRaises(AppError) as tested:
                _request(
                    self.ctx,
                    "ai.test_connection",
                    {"provider": "omlx", "base_url": "http://127.0.0.1:8000/v1"},
                )
        self.assertEqual(listed.exception.code, "ai_provider_disabled")
        self.assertEqual(tested.exception.code, "ai_provider_disabled")
        client_factory.assert_not_called()

    def test_model_results_follow_the_checked_endpoint(self):
        calls: list[str] = []

        def client_for(**kwargs):
            class Client:
                def list_models(self):
                    calls.append(kwargs["base_url"])
                    return [{"id": f"model@{kwargs['base_url']}"}]

            return Client()

        with patch("kassiber.daemon.ai_client_for_locator", side_effect=client_for):
            first = _request(self.ctx, "ai.list_models", {"provider": "omlx"})
            # Edited outside this daemon (another `kassiber` process), so no
            # update handler ran to drop the cache.
            update_db_ai_provider(self.conn, "omlx", {"base_url": "http://127.0.0.1:8001/v1"})
            second = _request(self.ctx, "ai.list_models", {"provider": "omlx"})
        self.assertEqual(calls, ["http://127.0.0.1:8000/v1", "http://127.0.0.1:8001/v1"])
        self.assertEqual(first["data"]["models"], [{"id": "model@http://127.0.0.1:8000/v1"}])
        self.assertEqual(second["data"]["models"], [{"id": "model@http://127.0.0.1:8001/v1"}])

    def test_model_check_bound_to_an_endpoint_refuses_an_edited_row(self):
        with patch(
            "kassiber.daemon.ai_client_for_locator",
            return_value=type("C", (), {"list_models": lambda self: [{"id": "m"}]})(),
        ) as client_factory:
            # The endpoint the caller saw still matches, trailing slash or not.
            listed = _request(
                self.ctx,
                "ai.list_models",
                {"provider": "omlx", "base_url": "http://127.0.0.1:8000/v1/", "refresh": True},
            )
            self.assertEqual(listed["data"]["models"], [{"id": "m"}])
            # Edited meanwhile, here from another process.
            update_db_ai_provider(self.conn, "omlx", {"base_url": "http://127.0.0.1:8001/v1"})
            with self.assertRaises(AppError) as ctx:
                _request(
                    self.ctx,
                    "ai.list_models",
                    {"provider": "omlx", "base_url": "http://127.0.0.1:8000/v1", "refresh": True},
                )
        self.assertEqual(ctx.exception.code, "ai_provider_changed")
        # Only the first, matching check built a client; the new endpoint was
        # never contacted on behalf of a check of the old one.
        self.assertEqual(client_factory.call_count, 1)
        with self.assertRaises(AppError) as bad:
            _request(self.ctx, "ai.list_models", {"provider": "omlx", "base_url": 8000})
        self.assertEqual(bad.exception.code, "validation")

    def test_runtime_check_bound_to_a_locator_refuses_an_edited_row(self):
        _request(self.ctx, "ai.providers.acknowledge", {"name": "codex"})
        update_db_ai_provider(self.conn, "codex", {"base_url": "claude-cli://default"})
        with patch.object(BrokerAIClient, "runtime_status") as probe:
            for args in (
                {"provider": "codex", "base_url": "codex-cli://default", "refresh": True},
                {"provider": "codex", "base_url": "codex-cli://default", "cached_only": True},
            ):
                with self.assertRaises(AppError) as ctx:
                    _request(self.ctx, "ai.provider_runtime.status", args)
                self.assertEqual(ctx.exception.code, "ai_provider_changed")
            with self.assertRaises(AppError) as unscoped:
                _request(
                    self.ctx,
                    "ai.provider_runtime.status",
                    {"base_url": "codex-cli://default"},
                )
        probe.assert_not_called()
        self.assertEqual(unscoped.exception.code, "validation")

        status = {"provider": "claude", "state": "ready", "models": []}
        with patch.object(BrokerAIClient, "runtime_status", return_value=[status]) as probe:
            checked = _request(
                self.ctx,
                "ai.provider_runtime.status",
                {"provider": "codex", "base_url": "claude-cli://default", "refresh": True},
            )
        probe.assert_called_once_with("claude")
        self.assertEqual(checked["data"]["providers"], [status])

    def test_endpoint_edit_through_the_daemon_drops_model_results(self):
        with patch(
            "kassiber.daemon.ai_client_for_locator",
            return_value=type("C", (), {"list_models": lambda self: [{"id": "m"}]})(),
        ) as client_factory:
            _request(self.ctx, "ai.list_models", {"provider": "omlx"})
            _request(
                self.ctx,
                "ai.providers.update",
                {"name": "omlx", "base_url": "http://127.0.0.1:8002/v1"},
            )
            _request(
                self.ctx,
                "ai.providers.update",
                {"name": "omlx", "base_url": "http://127.0.0.1:8000/v1"},
            )
            _request(self.ctx, "ai.list_models", {"provider": "omlx"})
        # Back on the first endpoint, the earlier result was not kept.
        self.assertEqual(client_factory.call_count, 2)

    def test_runtime_snapshot_follows_the_rows_locator(self):
        _request(self.ctx, "ai.providers.acknowledge", {"name": "codex"})
        status = {"provider": "codex", "state": "ready", "message": "Ready.", "models": []}
        with patch.object(BrokerAIClient, "runtime_status", return_value=[status]):
            _request(self.ctx, "ai.provider_runtime.status", {"provider": "codex", "refresh": True})
        # Re-pointed from another process, so no daemon handler dropped the cache.
        update_db_ai_provider(self.conn, "codex", {"base_url": "claude-cli://default"})
        with patch.object(BrokerAIClient, "runtime_status") as probe:
            cached = _request(
                self.ctx,
                "ai.provider_runtime.status",
                {"provider": "codex", "cached_only": True},
            )
        probe.assert_not_called()
        # Snapshots are keyed by the runtime the locator names, not the row.
        self.assertEqual(cached["data"]["providers"], [])
        self.assertIsNone(cached["data"]["checked_at"])

    def test_provider_check_rejects_http_providers(self):
        with self.assertRaises(AppError) as ctx:
            _request(
                self.ctx,
                "ai.provider_runtime.status",
                {"provider": "ollama", "refresh": True},
            )
        self.assertEqual(ctx.exception.code, "validation")


FAKE_STATUS_BROKER = """
import json, sys
request = json.loads(sys.stdin.readline())
rows = [
    {"provider": name, "state": "ready", "models": []}
    for name in ("codex", "claude", "opencode", "copilot")
]
if request.get("provider"):
    rows = [row for row in rows if row["provider"] == request["provider"]]
elif "providers" in request:
    rows = [row for row in rows if row["provider"] in request["providers"]]
print(json.dumps({"type": "result", "data": rows}), flush=True)
"""


class BrokerScopedStatusTest(unittest.TestCase):
    def test_an_empty_scope_starts_no_broker(self):
        with patch.object(BrokerAIClient, "_single_result") as single:
            self.assertEqual(BrokerAIClient.runtime_status(providers=[]), [])
        single.assert_not_called()

    def test_scoped_status_asks_the_broker_for_one_provider(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-broker-scope-") as tmp:
            script = Path(tmp) / "broker.py"
            script.write_text(FAKE_STATUS_BROKER, encoding="utf-8")
            with patch.dict(
                "os.environ",
                {
                    "KASSIBER_AI_BROKER_NODE": sys.executable,
                    "KASSIBER_AI_PROVIDER_BROKER": str(script),
                },
            ):
                self.assertEqual(
                    [row["provider"] for row in BrokerAIClient.runtime_status("copilot")],
                    ["copilot"],
                )
                self.assertEqual(len(BrokerAIClient.runtime_status()), 4)
                self.assertEqual(
                    [
                        row["provider"]
                        for row in BrokerAIClient.runtime_status(providers=["claude", "copilot"])
                    ],
                    ["claude", "copilot"],
                )
                with self.assertRaises(AppError):
                    BrokerAIClient.runtime_status(providers=["claude", "gemini"])
                with self.assertRaises(AppError):
                    BrokerAIClient.runtime_status("gemini")


def _run_json(data_root, *args):
    result = subprocess.run(
        [sys.executable, "-m", "kassiber", "--data-root", str(data_root), "--machine", *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        timeout=120,
    )
    return result.returncode, json.loads(result.stdout) if result.stdout.strip() else None


class ProviderSettingsCliTest(unittest.TestCase):
    def test_enable_disable_and_default_effort_flags(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-cli-") as tmp:
            data_root = Path(tmp) / "data"
            code, payload = _run_json(
                data_root, "ai", "providers", "update", "codex", "--disable", "--default-effort", "high"
            )
            self.assertEqual(code, 0, payload)
            self.assertFalse(payload["data"]["enabled"])
            self.assertEqual(payload["data"]["default_reasoning_effort"], "high")

            code, payload = _run_json(data_root, "ai", "providers", "list")
            self.assertEqual(code, 0, payload)
            listed = {row["name"]: row for row in payload["data"]["providers"]}
            self.assertFalse(listed["codex"]["enabled"])
            self.assertTrue(listed["claude"]["enabled"])

            code, payload = _run_json(
                data_root,
                "ai",
                "providers",
                "update",
                "codex",
                "--enable",
                "--clear",
                "default_reasoning_effort",
            )
            self.assertEqual(code, 0, payload)
            self.assertTrue(payload["data"]["enabled"])
            self.assertIsNone(payload["data"]["default_reasoning_effort"])

            code, payload = _run_json(data_root, "ai", "providers", "set-default", "ollama")
            self.assertEqual(code, 0, payload)
            code, payload = _run_json(data_root, "ai", "providers", "update", "ollama", "--disable")
            self.assertNotEqual(code, 0)
            self.assertEqual(payload["error"]["code"], "conflict")


class ProviderKindInferenceTest(unittest.TestCase):
    """A provider created without a kind is local on loopback, else remote."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-kind-")
        self.conn = open_db(str(Path(self._tmp.name) / "data"))

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def test_create_infers_local_or_remote_never_tee(self):
        cases = {
            "http://localhost:1234/v1": "local",
            "http://127.0.0.1:8080/v1": "local",
            "http://[::1]:8000/v1": "local",
            "http://192.168.1.20:11434/v1": "remote",
            "https://openrouter.ai/api/v1": "remote",
            "claude-cli://default": "remote",
        }
        for index, (base_url, kind) in enumerate(cases.items()):
            with self.subTest(base_url=base_url):
                row = create_db_ai_provider(self.conn, f"p{index}", base_url)
                self.assertEqual(row["kind"], kind)
                if kind == "local":
                    self.assertIsNotNone(row["acknowledged_at"])
                else:
                    # Inferred off-device still needs acknowledgement.
                    self.assertIsNone(row["acknowledged_at"])

    def test_update_moves_local_off_device_only_with_a_new_url(self):
        create_db_ai_provider(self.conn, "lan", "http://127.0.0.1:1234/v1")
        row = update_db_ai_provider(self.conn, "lan", {"base_url": "http://localhost:4321/v1"})
        self.assertEqual(row["kind"], "local")
        row = update_db_ai_provider(self.conn, "lan", {"base_url": "https://llm.example.test/v1"})
        self.assertEqual(row["kind"], "remote")
        # The local provider's implicit acknowledgement does not carry over.
        self.assertIsNone(row["acknowledged_at"])
        # Back on loopback it stays off-device: a gateway can forward prompts.
        row = update_db_ai_provider(
            self.conn, "lan", {"base_url": "http://127.0.0.1:1234/v1", "acknowledged": True}
        )
        self.assertEqual(row["kind"], "remote")
        self.assertIsNotNone(row["acknowledged_at"])

    def test_leaving_local_explicitly_needs_acknowledgement_again(self):
        create_db_ai_provider(self.conn, "lan", "http://127.0.0.1:1234/v1")
        row = update_db_ai_provider(self.conn, "lan", {"kind": "tee"})
        self.assertIsNone(row["acknowledged_at"])
        create_db_ai_provider(self.conn, "lan2", "http://127.0.0.1:1235/v1")
        row = update_db_ai_provider(self.conn, "lan2", {"kind": "remote", "acknowledged": True})
        self.assertIsNotNone(row["acknowledged_at"])

    def test_daemon_create_without_kind_infers_it(self):
        envelope = _request(
            _ctx(self.conn),
            "ai.providers.create",
            {"name": "router", "base_url": "https://openrouter.ai/api/v1"},
        )
        self.assertEqual(envelope["data"]["kind"], "remote")
        self.assertIsNone(envelope["data"]["acknowledged_at"])


class ProviderKindInferenceCliTest(unittest.TestCase):
    def test_kind_is_inferred_from_the_url(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-kind-cli-") as tmp:
            data_root = Path(tmp) / "data"
            code, payload = _run_json(
                data_root,
                "ai", "providers", "create", "openrouter",
                "--base-url", "https://openrouter.ai/api/v1",
                "--api-format", "chat-completions",
            )
            self.assertEqual(code, 0, payload)
            self.assertEqual(payload["data"]["kind"], "remote")
            self.assertIsNone(payload["data"]["acknowledged_at"])

            code, payload = _run_json(
                data_root,
                "ai", "providers", "create", "lmstudio",
                "--base-url", "http://localhost:1234/v1",
            )
            self.assertEqual(code, 0, payload)
            self.assertEqual(payload["data"]["kind"], "local")

            code, payload = _run_json(
                data_root,
                "ai", "providers", "create", "claude-work",
                "--base-url", "claude-cli://default",
                "--acknowledge",
            )
            self.assertEqual(code, 0, payload)
            self.assertEqual(payload["data"]["kind"], "remote")
            self.assertIsNotNone(payload["data"]["acknowledged_at"])


class ProviderDisabledCliTest(unittest.TestCase):
    def test_ai_models_refuses_a_disabled_provider_before_contacting_it(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-cli-off-") as tmp:
            data_root = Path(tmp) / "data"
            code, payload = _run_json(data_root, "ai", "providers", "update", "omlx", "--disable")
            self.assertEqual(code, 0, payload)
            code, payload = _run_json(data_root, "ai", "models", "--provider", "omlx")
            self.assertNotEqual(code, 0)
            self.assertEqual(payload["error"]["code"], "ai_provider_disabled", payload)


class _FakeChatClient:
    def __init__(self, rows):
        self.rows = rows
        self.sent: list[dict] = []

    def send(self, payload, *, record=True):
        self.sent.append(payload)


class CliChatEffortSeedTest(unittest.TestCase):
    ROWS = [
        {"name": "claude", "base_url": "claude-cli://default", "default_model": "default",
         "default_reasoning_effort": "high", "is_default": True},
        {"name": "codex", "base_url": "codex-cli://default", "default_model": "default",
         "default_reasoning_effort": None, "is_default": False},
    ]

    def _resolve(self, args):
        client = _FakeChatClient(self.ROWS)
        with patch.object(
            cli_chat,
            "_read_control_response",
            return_value={"kind": "ai.providers.list", "data": {"providers": self.ROWS}},
        ):
            cli_chat._resolve_default_model(client, args)
        return args

    def test_provider_default_effort_seeds_the_session(self):
        args = self._resolve(argparse.Namespace(provider=None, model=None, reasoning_effort=None))
        self.assertEqual(args.model, "default")
        self.assertEqual(args.reasoning_effort, "high")
        self.assertEqual(cli_chat._chat_options(args), {"reasoning_effort": "high"})

    def test_explicit_effort_wins_even_with_explicit_model(self):
        args = self._resolve(
            argparse.Namespace(provider=None, model="opus", reasoning_effort="auto")
        )
        self.assertEqual(args.reasoning_effort, "auto")
        self.assertIsNone(cli_chat._chat_options(args))

    def test_provider_without_default_stays_auto(self):
        args = self._resolve(argparse.Namespace(provider="codex", model=None, reasoning_effort=None))
        self.assertIsNone(args.reasoning_effort)
        self.assertIsNone(cli_chat._chat_options(args))


if __name__ == "__main__":
    unittest.main()
