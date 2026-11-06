"""Stale journal figures reach the model as an explicit state, never as zeros."""

import tempfile
import unittest

from kassiber import daemon
from kassiber.cli import handlers
from kassiber.db import set_setting
from kassiber.mcp.tools import run_tool
from tests.test_custody_quantity_handler import _book


class AiStaleFiguresTest(unittest.TestCase):
    def _open(self, root, *, processed=True):
        conn = _book(root, "FIFO")
        self.addCleanup(conn.close)
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", "profile")
        conn.commit()
        if processed:
            handlers.process_journals(conn, "Books", "Book")
            # Inputs change after processing: the stored journals are stale.
            conn.execute("UPDATE profiles SET journal_input_version = journal_input_version + 1")
            conn.commit()
        return conn

    def _call(self, conn, root, name, arguments=None):
        # MCP reads never refresh journals, so the stale state reaches the tool.
        return run_tool(
            conn,
            data_root=root,
            runtime_config={},
            name=name,
            arguments=arguments or {},
            workspace="ws",
            profile="profile",
            project_name=None,
        )["data"]

    def test_stale_transfer_audit_is_unavailable_not_zero(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            data = self._call(conn, root, "journals_transfers_list")

        self.assertEqual(data["status"], "unavailable")
        self.assertEqual(data["reason"], "journals_stale")
        self.assertEqual(data["remediation"]["tool"], "ui.journals.process")
        self.assertIsNone(data["summary"]["manual_pairs"])
        self.assertIsNone(data["summary"]["journal_transfer_entries"])
        self.assertIsNone(data["pairs"])

    def test_stale_capital_gains_and_events_withhold_their_figures(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            gains = self._call(conn, root, "reports_capital_gains")
            events = self._call(conn, root, "journals_events_list")
            snapshot = self._call(conn, root, "journals_snapshot")

        for data in (gains, events, snapshot):
            self.assertEqual(data["status"], "unavailable")
            self.assertEqual(data["reason"], "journals_stale")
        self.assertIsNone(gains["totals"])
        self.assertIsNone(gains["rows"])
        self.assertIsNone(events["events"])
        self.assertIsNone(events["summary"]["count"])
        self.assertIsNone(snapshot["entryTypes"])

    def test_stale_quarantine_keeps_its_rows_but_says_they_are_stale(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root)
            data = self._call(conn, root, "journals_quarantine")
            groups = self._call(conn, root, "journals_quarantine_groups")

        self.assertEqual(data["projection"]["status"], "stale")
        self.assertIsNotNone(data["projection"]["as_of"])
        self.assertIsInstance(data["summary"]["count"], int)
        self.assertEqual(groups["projection"]["status"], "stale")

    def test_never_processed_quarantine_is_unavailable_not_empty(self):
        with tempfile.TemporaryDirectory() as root:
            conn = self._open(root, processed=False)
            data = self._call(conn, root, "journals_quarantine")
            groups = self._call(conn, root, "journals_quarantine_groups")

        self.assertEqual(data["status"], "unavailable")
        self.assertEqual(data["reason"], "journals_never_processed")
        self.assertIsNone(data["summary"]["count"])
        self.assertIsNone(data["items"])
        self.assertEqual(groups["reason"], "journals_never_processed")
        self.assertNotIn("by_root", groups)

    def test_current_figures_pass_through_unchanged(self):
        payload = {"summary": {"projection_status": None, "manual_pairs": 0}, "pairs": []}
        self.assertIs(
            daemon._project_stale_journal_figures_for_ai("ui.journals.transfers.list", payload),
            payload,
        )
        overview = {"fiat": {"eurCostBasis": 5.0, "completeness": {"state": "complete"}}}
        self.assertIs(
            daemon._project_stale_journal_figures_for_ai("ui.overview.snapshot", overview),
            overview,
        )
        stale = daemon._project_stale_journal_figures_for_ai(
            "ui.overview.snapshot",
            {"fiat": {"eurCostBasis": 5.0, "eurUnrealized": 1.0, "eurRealizedYTD": 2.0,
                      "completeness": {"state": "stale"}}},
        )
        self.assertIsNone(stale["fiat"]["eurCostBasis"])
        self.assertEqual(stale["fiat"]["availability"]["status"], "unavailable")


if __name__ == "__main__":
    unittest.main()
