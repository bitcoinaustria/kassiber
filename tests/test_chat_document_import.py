"""A photo or PDF attached to a chat becomes rows only as an approved, bound plan.

Pinned here: the document is read only by a local vision model; the draft's
ready rows are planned without changing the book and imported only as the
unchanged plan describes, with the document kept as evidence; rows the
reader was unsure of are never imported from chat; a restricted provider
gets counts, not values; a changed document or a used draft is refused; and
the tools are offered only for a chat that holds an image or PDF.
"""

from __future__ import annotations

import os
import queue
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from kassiber import daemon
from kassiber.ai.tools import CHAT_DOCUMENT_TOOL_NAMES, get_tool, tool_capabilities
from kassiber.core import document_import
from kassiber.db import open_db
from tests.test_document_import import FakeVisionClient

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PNG = b"\x89PNG\r\n\x1a\n" + (13).to_bytes(4, "big") + b"IHDR" + (800).to_bytes(4, "big") + (600).to_bytes(4, "big")


class ChatDocumentImportTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.data_root = os.path.join(self._tmp.name, "data")
        for args in (
            ("init",),
            ("workspaces", "create", "W"),
            ("profiles", "create", "P", "--workspace", "W",
             "--fiat-currency", "EUR", "--tax-country", "at"),
            ("wallets", "create", "--workspace", "W", "--profile", "P",
             "--label", "Receipts", "--kind", "custom"),
        ):
            subprocess.run(
                [sys.executable, "-m", "kassiber", "--data-root", self.data_root, "--machine", *args],
                capture_output=True, text=True, cwd=REPO_ROOT, check=True,
            )
        self.conn = open_db(self.data_root)
        self.addCleanup(self.conn.close)
        for target, side_effect in (
            ("kassiber.daemon._run_on_daemon_main_thread", lambda runtime, callback: callback(self.conn)),
        ):
            patcher = patch(target, side_effect=side_effect)
            patcher.start()
            self.addCleanup(patcher.stop)
        self.client = FakeVisionClient()
        patcher = patch.object(document_import, "_client_for_provider", return_value=self.client)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.receipt = os.path.join(self._tmp.name, "Beleg_Mustermann.png")
        with open(self.receipt, "wb") as handle:
            handle.write(PNG)
        self.sessions = daemon.DocumentImportSessions()
        workspace, profile = daemon.resolve_scope(self.conn, None, None)
        self.token = self.sessions.stage(
            source_file=self.receipt,
            workspace_id=str(workspace["id"]),
            profile_id=str(profile["id"]),
            data_root=self.data_root,
        )

    def runtime(self, *, full_access=True):
        return daemon.AiToolRuntime(self.data_root, {}, queue.Queue(), {
            "provider_full_access": full_access,
            "document_import_sessions": self.sessions,
            "chat_files": [{
                "id": "file-1", "source_file": self.receipt, "filename": "Beleg_Mustermann.png",
                "kind": "image", "new": True, "token": self.token,
            }],
        })

    def read(self, name, arguments, runtime):
        return daemon._execute_read_only_ai_tool(daemon.ParsedAiToolCall("call", name, arguments), runtime)

    def write(self, name, arguments, runtime):
        return daemon._execute_mutating_ai_tool(daemon.ParsedAiToolCall("call", name, arguments), runtime)

    def tx_count(self):
        return self.conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0]

    def read_receipt(self, runtime):
        result = self.read("ui.wallets.read_document", {"file": "file-1"}, runtime)
        self.assertTrue(result["ok"], result)
        return result["envelope"]["data"]

    def test_tools_are_offered_only_for_a_chat_document(self):
        for name in CHAT_DOCUMENT_TOOL_NAMES:
            # No capability pack selects them; the turn adds them for a file.
            self.assertEqual(tool_capabilities(get_tool(name)), frozenset({"chat_documents"}))
        self.assertTrue(get_tool("ui.wallets.read_document").egresses)
        self.assertEqual(get_tool("ui.wallets.import_document.apply").kind_class, "mutating")
        self.assertIn("ui.wallets.import_document.apply", daemon.AI_TOOL_ONCE_ONLY_CONSENT)

    def test_reads_plans_and_imports_only_the_ready_rows(self):
        runtime = self.runtime()
        draft = self.read_receipt(runtime)
        self.assertEqual(draft["summary"]["ready"], 1)
        self.assertEqual(draft["summary"]["quarantined"], 1)
        self.assertEqual(draft["rows"][0]["record"]["fiat_value"], "500")
        self.assertNotIn("evidence_text", draft["rows"][0])
        self.assertNotIn(self._tmp.name, repr(draft))
        self.assertEqual(self.tx_count(), 0)

        plan = self.read("ui.wallets.import_document.plan",
                         {"draft": draft["draft"], "wallet": "Receipts"}, runtime)
        self.assertTrue(plan["ok"], plan)
        plan = plan["envelope"]["data"]
        self.assertTrue(plan["ready"], plan["blockers"])
        self.assertEqual(plan["rows"]["to_import"], 1)
        self.assertEqual(plan["rows"]["held_for_review"], 1)
        self.assertIn("rows_held_for_review", plan["warnings"])
        self.assertEqual(self.tx_count(), 0)

        applied = self.write("ui.wallets.import_document.apply",
                             {"draft": draft["draft"], "wallet": "Receipts", **plan["apply_with"]}, runtime)
        self.assertTrue(applied["ok"], applied)
        data = applied["envelope"]["data"]
        self.assertEqual(data["rows"]["imported"], 1)
        self.assertEqual(data["evidence_attached"], 1)
        self.assertTrue(data["import_batch_id"])
        # Recorded as an import run; with the document kept as evidence, the
        # rollback is the desktop's, not a tool's.
        self.assertEqual(data["rollback"]["batch"], data["import_batch_id"])
        self.assertTrue(data["rollback"]["desktop_only"])
        self.assertFalse(get_tool("ui.wallets.import_document.apply").reversible)
        self.assertEqual(self.tx_count(), 1)
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM attachments").fetchone()[0], 1)
        # The draft is used up; a second apply is refused.
        again = self.write("ui.wallets.import_document.apply",
                           {"draft": draft["draft"], "wallet": "Receipts", **plan["apply_with"]}, runtime)
        self.assertFalse(again["ok"])
        self.assertEqual(self.tx_count(), 1)

    def test_a_new_wallet_is_created_only_on_apply(self):
        runtime = self.runtime()
        draft = self.read_receipt(runtime)
        target = {"new_wallet": {"label": "Bar"}}
        plan = self.read("ui.wallets.import_document.plan", {"draft": draft["draft"], **target}, runtime)
        plan = plan["envelope"]["data"]
        self.assertTrue(plan["ready"], plan["blockers"])
        self.assertIsNone(self.conn.execute("SELECT 1 FROM wallets WHERE label = 'Bar'").fetchone())
        applied = self.write("ui.wallets.import_document.apply",
                             {"draft": draft["draft"], **target, **plan["apply_with"]}, runtime)
        self.assertTrue(applied["ok"], applied)
        self.assertIsNotNone(self.conn.execute("SELECT 1 FROM wallets WHERE label = 'Bar'").fetchone())

    def test_a_changed_document_is_refused(self):
        runtime = self.runtime()
        draft = self.read_receipt(runtime)
        plan = self.read("ui.wallets.import_document.plan",
                         {"draft": draft["draft"], "wallet": "Receipts"}, runtime)["envelope"]["data"]
        with open(self.receipt, "ab") as handle:
            handle.write(b"edited")
        applied = self.write("ui.wallets.import_document.apply",
                             {"draft": draft["draft"], "wallet": "Receipts", **plan["apply_with"]}, runtime)
        self.assertFalse(applied["ok"])
        self.assertEqual(self.tx_count(), 0)

    def test_a_reread_with_other_values_needs_a_new_plan(self):
        runtime = self.runtime()
        first = self.read_receipt(runtime)
        plan = self.read("ui.wallets.import_document.plan",
                         {"draft": first["draft"], "wallet": "Receipts"}, runtime)["envelope"]["data"]
        # Same BTC amounts, another fiat value: the effect looks the same.
        self.client._content = self.client._content.replace('"500.00"', '"999.00"')
        second = self.read_receipt(runtime)
        self.assertEqual(second["rows"][0]["record"]["fiat_value"], "999")
        applied = self.write("ui.wallets.import_document.apply",
                             {"draft": second["draft"], "wallet": "Receipts", **plan["apply_with"]}, runtime)
        self.assertFalse(applied["ok"], applied)
        self.assertEqual(applied["reason"], "import_plan_stale")
        self.assertEqual(self.tx_count(), 0)

    def test_a_restricted_provider_gets_counts_not_values(self):
        runtime = self.runtime(full_access=False)
        draft = self.read_receipt(runtime)
        self.assertTrue(draft["row_values_withheld"])
        self.assertNotIn("rows", draft)
        self.assertNotIn("500", repr(draft))
        self.assertNotIn("Mustermann", repr(draft))
        plan = self.read("ui.wallets.import_document.plan",
                         {"draft": draft["draft"], "wallet": "Receipts"}, runtime)["envelope"]["data"]
        self.assertNotIn("row_ids", plan)
        self.assertEqual(plan["rows"]["to_import"], 1)

    def test_without_a_local_model_it_says_how_to_add_one(self):
        with patch.object(daemon, "list_db_ai_providers", return_value=[]):
            result = self.read("ui.wallets.read_document", {"file": "file-1"}, self.runtime())
        self.assertFalse(result["ok"])
        self.assertEqual(result["reason"], "document_reader_unavailable")
        self.assertEqual(self.client.chat_requests, [])

    def test_a_spreadsheet_is_not_read_as_a_document(self):
        runtime = self.runtime()
        runtime.maintenance_state["chat_files"][0]["kind"] = "delimited"
        result = self.read("ui.wallets.read_document", {"file": "file-1"}, runtime)
        self.assertFalse(result["ok"])
        self.assertEqual(result["reason"], "ai_attachment_unsupported")


if __name__ == "__main__":
    unittest.main()
