"""The last explicit provider check, kept across restarts.

Settings shows what a provider's last check found after a relaunch. Only a
check the user started writes it; a display read returns it and never probes.
A result belongs to the endpoint and key it was found with.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.ai import create_db_ai_provider, set_default_ai_provider
from kassiber.ai.broker_client import BrokerAIClient
from kassiber.ai.client import OpenAICompatibleClient
from kassiber.ai.providers import list_with_default
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_ai_core_provider_settings import _ctx, _request


READY = {"provider": "codex", "display_name": "Codex", "state": "ready", "version": "1.2.3", "message": "", "models": [{"id": "gpt-5"}]}


class SavedProviderCheckTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-checks-")
        self.data_root = str(Path(self._tmp.name) / "data")
        self.conn = open_db(self.data_root)
        self.ctx = _ctx(self.conn)

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def _relaunch(self):
        # A new daemon: an empty discovery cache over the same book.
        self.conn.close()
        self.conn = open_db(self.data_root)
        self.ctx = _ctx(self.conn)

    def _runtime(self, name, **args):
        return _request(self.ctx, "ai.provider_runtime.status", {"provider": name, **args})["data"]

    def _check_codex(self):
        _request(self.ctx, "ai.providers.acknowledge", {"name": "codex"})
        with patch.object(BrokerAIClient, "runtime_status", return_value=[READY]):
            return self._runtime("codex", refresh=True)

    def test_a_runtime_check_is_shown_after_a_relaunch_without_probing(self):
        checked = self._check_codex()
        self._relaunch()
        with patch.object(BrokerAIClient, "runtime_status") as probe:
            cached = self._runtime("codex", cached_only=True)
        probe.assert_not_called()
        self.assertEqual(cached["providers"], [READY])
        self.assertEqual(cached["checked_at"], checked["checked_at"])
        # Another provider of the same book was never checked.
        with patch.object(BrokerAIClient, "runtime_status") as probe:
            self.assertEqual(self._runtime("claude", cached_only=True)["providers"], [])
        probe.assert_not_called()

    def test_a_failed_check_after_a_relaunch_keeps_the_saved_result_as_stale(self):
        self._check_codex()
        self._relaunch()
        with patch.object(
            BrokerAIClient, "runtime_status", side_effect=AppError("broker down", code="ai_unavailable")
        ):
            refreshed = self._runtime("codex", refresh=True)
        self.assertEqual(refreshed["providers"], [READY])
        self.assertIs(refreshed["stale"], True)
        self.assertEqual(refreshed["error"]["code"], "ai_unavailable")
        self._relaunch()
        self.assertIs(self._runtime("codex", cached_only=True)["stale"], True)

    def test_a_new_address_or_a_real_edit_forgets_it(self):
        self._check_codex()
        _request(self.ctx, "ai.providers.update", {"name": "codex", "enabled": False})
        self._relaunch()
        # The switch is a preference: the result stays.
        self.assertEqual(self._runtime("codex", cached_only=True)["providers"], [READY])
        _request(self.ctx, "ai.providers.update", {"name": "codex", "notes": "edited"})
        self._relaunch()
        self.assertEqual(self._runtime("codex", cached_only=True)["providers"], [])

    def test_an_http_listing_is_kept_per_endpoint_and_read_without_probing(self):
        create_db_ai_provider(self.conn, "lab", "http://127.0.0.1:9000/v1")
        models = [{"id": "qwen"}]
        with patch.object(OpenAICompatibleClient, "list_models", return_value=models):
            listed = _request(self.ctx, "ai.list_models", {"provider": "lab", "refresh": True})["data"]
        self._relaunch()
        with patch.object(OpenAICompatibleClient, "list_models") as probe:
            cached = _request(self.ctx, "ai.list_models", {"provider": "lab", "cached_only": True})["data"]
        probe.assert_not_called()
        self.assertEqual(cached["models"], models)
        self.assertEqual(cached["checked_at"], listed["checked_at"])
        with self.assertRaises(AppError):
            _request(self.ctx, "ai.list_models", {"provider": "lab", "cached_only": True, "refresh": True})
        # Moved to another endpoint, the old listing is not reported for it.
        _request(self.ctx, "ai.providers.update", {"name": "lab", "base_url": "http://127.0.0.1:9001/v1"})
        self._relaunch()
        cached = _request(self.ctx, "ai.list_models", {"provider": "lab", "cached_only": True})["data"]
        self.assertEqual(cached["models"], [])
        self.assertIsNone(cached["checked_at"])

    def test_a_new_key_forgets_it(self):
        create_db_ai_provider(self.conn, "lab", "http://127.0.0.1:9000/v1")
        with patch.object(OpenAICompatibleClient, "list_models", return_value=[{"id": "qwen"}]):
            _request(self.ctx, "ai.list_models", {"provider": "lab", "refresh": True})
        _request(self.ctx, "ai.providers.set_api_key", {"name": "lab", "api_key": "sk-new"})
        self._relaunch()
        cached = _request(self.ctx, "ai.list_models", {"provider": "lab", "cached_only": True})["data"]
        self.assertEqual(cached["models"], [])

    def test_a_display_read_of_an_unacknowledged_provider_contacts_nothing(self):
        create_db_ai_provider(self.conn, "cloud", "https://llm.example.test/v1", kind="remote")
        with patch.object(OpenAICompatibleClient, "list_models") as probe:
            cached = _request(self.ctx, "ai.list_models", {"provider": "cloud", "cached_only": True})["data"]
        probe.assert_not_called()
        self.assertEqual(cached["models"], [])


class ProviderSetUpTest(unittest.TestCase):
    """Which built-in providers Settings folds away as not set up."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-ai-provider-setup-")
        self.conn = open_db(str(Path(self._tmp.name) / "data"))
        self.ctx = _ctx(self.conn)

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def _set_up(self):
        return {row["name"]: row["set_up"] for row in list_with_default(self.conn)["providers"]}

    def test_untouched_built_ins_are_not_set_up_until_used(self):
        rows = self._set_up()
        # The default is in use; the others wait.
        self.assertIs(rows["ollama"], True)
        for name in ("omlx", "codex", "claude", "opencode", "copilot"):
            self.assertIs(rows[name], False, name)
        _request(self.ctx, "ai.providers.acknowledge", {"name": "claude"})
        self.assertIs(self._set_up()["claude"], True)
        with patch.object(OpenAICompatibleClient, "list_models", return_value=[]):
            _request(self.ctx, "ai.list_models", {"provider": "omlx", "refresh": True})
        self.assertIs(self._set_up()["omlx"], True)
        set_default_ai_provider(self.conn, "opencode")
        self.assertIs(self._set_up()["opencode"], True)
        self.assertIs(self._set_up()["copilot"], False)

    def test_a_provider_the_user_added_is_set_up(self):
        create_db_ai_provider(self.conn, "lab", "http://127.0.0.1:9000/v1")
        self.assertIs(self._set_up()["lab"], True)

    def test_the_list_reports_when_it_was_last_checked(self):
        rows = {row["name"]: row for row in list_with_default(self.conn)["providers"]}
        self.assertIsNone(rows["omlx"]["last_checked_at"])
        with patch.object(OpenAICompatibleClient, "list_models", return_value=[]):
            listed = _request(self.ctx, "ai.list_models", {"provider": "omlx", "refresh": True})["data"]
        rows = {row["name"]: row for row in list_with_default(self.conn)["providers"]}
        self.assertEqual(rows["omlx"]["last_checked_at"], listed["checked_at"])


if __name__ == "__main__":
    unittest.main()
