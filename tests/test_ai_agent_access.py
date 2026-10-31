"""Per-provider agent access: full by default, restricted by the user's choice."""

from __future__ import annotations

import queue
import os
import tempfile
import unittest
from pathlib import Path

from kassiber.ai.providers import (
    create_db_ai_provider,
    get_db_ai_provider,
    update_db_ai_provider,
)
from kassiber.daemon import (
    AiToolRuntime,
    _analyze_attached_file_for_ai,
    _chat_files_note,
    _provider_has_full_agent_access,
)
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_file_analysis import OPAQUE_CSV, OPAQUE_PLAN, _write


class AgentAccessStorageTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.conn = open_db(str(Path(self._tmp.name) / "data"))
        self.addCleanup(self.conn.close)

    def test_new_providers_get_full_access_and_the_user_can_restrict_one(self):
        created = create_db_ai_provider(
            self.conn, "cloud", "https://api.example.com/v1", acknowledged=True
        )
        self.assertEqual(created["agent_access"], "full")
        updated = update_db_ai_provider(
            self.conn, "cloud", {"agent_access": "restricted"}
        )
        self.assertEqual(updated["agent_access"], "restricted")
        # An unrelated update keeps the choice.
        kept = update_db_ai_provider(self.conn, "cloud", {"notes": "work"})
        self.assertEqual(kept["agent_access"], "restricted")

    def test_unknown_values_are_refused_and_unknown_stored_values_restrict(self):
        with self.assertRaises(AppError) as raised:
            create_db_ai_provider(
                self.conn, "cloud", "https://api.example.com/v1", agent_access="all"
            )
        self.assertEqual(raised.exception.code, "validation")
        create_db_ai_provider(self.conn, "cloud", "https://api.example.com/v1")
        self.conn.execute("UPDATE ai_providers SET agent_access = 'future' WHERE name = 'cloud'")
        self.assertEqual(get_db_ai_provider(self.conn, "cloud")["agent_access"], "restricted")


class AgentAccessDecisionTests(unittest.TestCase):
    def test_on_device_is_always_full_and_off_device_follows_the_setting(self):
        on_device = {"kind": "local", "base_url": "http://127.0.0.1:11434/v1"}
        remote = {"kind": "remote", "base_url": "https://api.example.com/v1"}
        self.assertTrue(_provider_has_full_agent_access({**on_device, "agent_access": "restricted"}))
        self.assertTrue(_provider_has_full_agent_access(remote))
        self.assertTrue(_provider_has_full_agent_access({**remote, "agent_access": "full"}))
        self.assertFalse(_provider_has_full_agent_access({**remote, "agent_access": "restricted"}))

    def test_full_access_remote_reads_attachment_cells_but_never_the_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = _write(tmp, "opaque.csv", OPAQUE_CSV)
            state = {**{"chat_files": [{"id": "file-1", "source_file": path, "filename": os.path.basename(path), "kind": "delimited", "new": True}]}, "provider_on_device": False}
            full = _analyze_attached_file_for_ai(
                AiToolRuntime(tmp, {}, queue.Queue(), {**state, "provider_full_access": True}),
                {"column_map": OPAQUE_PLAN},
            )
            restricted = _analyze_attached_file_for_ai(
                AiToolRuntime(tmp, {}, queue.Queue(), {**state, "provider_full_access": False}),
                {"column_map": OPAQUE_PLAN},
            )
        self.assertEqual(full["preview"][0]["fiat_value"], "21000")
        self.assertNotIn(path, repr(full))
        self.assertTrue(restricted["cell_values_withheld"])

    def test_attachment_note_names_the_file_only_with_full_access(self):
        state = {
            "chat_files": [{"id": "file-1", "filename": "Umsatzliste_AT61.csv", "kind": "delimited"}],
            "provider_on_device": False,
        }
        self.assertIn(
            "Umsatzliste_AT61.csv",
            _chat_files_note({**state, "provider_full_access": True}, {}),
        )
        self.assertNotIn(
            "AT61", _chat_files_note({**state, "provider_full_access": False}, {})
        )


if __name__ == "__main__":
    unittest.main()
