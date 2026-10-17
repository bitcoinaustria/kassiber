"""The transaction list exports even when the book cannot report yet.

A list of observed transactions is not a tax report. Book-wide review states
(an incomplete custody component, unresolved custody quantity, stale
journals) hold back reports and other tools' import files, but the list still
exports, with the journal's columns left empty and the reason named.
"""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from kassiber.errors import AppError
from tests.test_report_transaction_formats import _run

_SCOPE = ("--workspace", "Main", "--profile", "Draft")
_CSV = """date,txid,direction,asset,amount,fee,fiat_value,fiat_rate,description,kind
2025-01-10T10:00:00Z,buy-1,inbound,BTC,1.00000000,0,40000,40000,Buy,buy
2025-06-01T10:00:00Z,pay-1,outbound,BTC,0.10000000,0.00010000,,60000,Pay,withdrawal
"""


class TransactionListDraftTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-list-draft-")
        tmp = Path(cls._tmp.name)
        cls.tmp = tmp
        cls.root = tmp / "data"

        def run(*args):
            payload, code = _run(cls.root, *args)
            if code != 0:
                raise AssertionError(f"{args} -> {json.dumps(payload)[:400]}")
            return payload["data"]

        run("init")
        run("workspaces", "create", "Main")
        run("profiles", "create", "--workspace", "Main", "--fiat-currency", "EUR", "Draft")
        source = tmp / "hot.csv"
        source.write_text(_CSV, encoding="utf-8")
        run("wallets", "create", *_SCOPE, "--label", "Hot", "--kind", "custom")
        run("wallets", "import-csv", *_SCOPE, "--wallet", "Hot", "--file", str(source))
        # Journals are never processed: the book cannot report.

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def test_the_list_exports_as_a_draft_while_reports_are_held_back(self):
        target = self.tmp / "list.csv"
        payload, code = _run(self.root, "transactions", "export", *_SCOPE, "--export-format", "csv", "--file", str(target))
        self.assertEqual(code, 0, json.dumps(payload)[:400])
        self.assertEqual(payload["data"]["rows"], 2)
        self.assertTrue(payload["data"]["warnings"][0].startswith("Draft list:"))
        # Unprocessed journals: transfer links cannot be trusted either.
        self.assertIn("transfer links", payload["data"]["warnings"][0])
        # The file travels without the warning, so it names itself a draft.
        self.assertIn("Transactions (draft:", target.read_text(encoding="utf-8"))
        workbook = self.tmp / "list.xlsx"
        payload, code = _run(self.root, "transactions", "export", *_SCOPE, "--file", str(workbook))
        self.assertEqual(code, 0, json.dumps(payload)[:400])
        # Another tool would import a guessed tax event: still refused.
        payload, code = _run(
            self.root, "transactions", "export", *_SCOPE, "--export-format", "cointracking-csv",
            "--file", str(self.tmp / "ct.csv"),
        )
        self.assertNotEqual(code, 0)

    def test_a_custody_component_block_keeps_the_list_and_drops_journal_columns(self):
        from kassiber.cli.handlers import _report_hooks
        from kassiber.core import reports
        from kassiber.db import open_db

        blocked = AppError("Reports are blocked by an incomplete custody component.", code="custody_component_incomplete")
        conn = open_db(self.root)
        try:
            with mock.patch.object(reports, "_require_report_context", side_effect=blocked), mock.patch.object(
                reports.core_custody_journal, "projection_freshness", return_value={"is_current": True}
            ):
                context = reports._transactions_export_context(conn, None, None, _report_hooks())
                self.assertIn("incomplete custody component", context["warnings"][0])
                rows = context["spec"]["rows"]
                self.assertTrue(all(row["cost_basis"] is None and row["gain_loss"] is None for row in rows))
                # Journals are current: reviewed transfer links stay, so own-wallet
                # moves are still recognisable in the list.
                self.assertNotIn("transfer links", context["warnings"][0])
                # Anything else is still an error.
                other = AppError("Book is not set up for accounting", code="book_accounting_unavailable")
                with mock.patch.object(reports, "_require_report_context", side_effect=other):
                    with self.assertRaises(AppError):
                        reports._transactions_export_context(conn, None, None, _report_hooks())
        finally:
            conn.close()

    def test_stale_journals_do_not_hide_an_earlier_gate(self):
        """Journals are stale here, yet a gate checked before freshness still stops the list."""
        from kassiber.cli.handlers import _report_hooks
        from kassiber.core import book_network, reports
        from kassiber.db import open_db

        blocked = AppError("This book contains incompatible network histories", code="book_network_review_required")
        conn = open_db(self.root)
        try:
            with mock.patch.object(book_network, "require_book_accounting", side_effect=blocked):
                with self.assertRaises(AppError) as raised:
                    reports._transactions_export_context(conn, None, None, _report_hooks())
            self.assertEqual(raised.exception.code, "book_network_review_required")
        finally:
            conn.close()

if __name__ == "__main__":
    unittest.main()
