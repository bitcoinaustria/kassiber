"""Reversible AI tools, their undo hints, and undoing a dismissal.

A reversible mutating tool's result names the call that takes it back; the
classification is pinned so a new tool is irreversible until reviewed.
"""

from __future__ import annotations

import queue
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

from kassiber import daemon
from kassiber.ai.tools import REVERSIBLE_TOOLS, TOOL_CATALOG, get_tool
from kassiber.ai.undo import UNDO_HINTS, undo_hint
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_swap_matching_cli import (
    _LIQUID_CSV,
    _PHOENIX_CSV,
    _bootstrap_profile,
    _run,
)


class ReversibilityTest(unittest.TestCase):
    EXPECTED_REVERSIBLE = {
        "ui.journals.process",
        "ui.transfers.pair",
        "ui.transfers.unpair",
        "ui.transfers.dismiss",
        "ui.transfers.dismissals.delete",
        "ui.transfers.rules.create",
        "ui.transfers.rules.delete",
        "ui.transfers.rules.set_enabled",
        "ui.transfers.payouts.create",
        "ui.transfers.components.apply",
        "ui.custody.review.apply",
        "ui.saved_views.create",
        "ui.loans.mark",
        "ui.transactions.metadata.update",
        "ui.transactions.history.revert",
        "ui.journals.quarantine.resolve",
        "ui.wallets.import_attachment.apply",
        "ui.chain_analysis.cases.save",
        "ui.chain_analysis.watches.create",
    }

    def test_every_consented_tool_has_a_deliberate_classification(self):
        reversible = {tool.name for tool in TOOL_CATALOG if tool.reversible}
        # A new mutating tool is irreversible (Auto asks) until reviewed here.
        self.assertEqual(reversible, self.EXPECTED_REVERSIBLE)
        self.assertEqual(set(REVERSIBLE_TOOLS), self.EXPECTED_REVERSIBLE)
        for tool in TOOL_CATALOG:
            if tool.reversible:
                self.assertEqual(tool.kind_class, "mutating", tool.name)
                self.assertFalse(tool.egresses, tool.name)

    def test_tools_that_can_contact_the_network_say_so(self):
        for name in ("ui.wallets.sync", "ui.rates.latest", "ui.rates.rebuild", "ui.maintenance.run"):
            self.assertTrue(get_tool(name).egresses, name)

    def test_core_profile_keeps_each_reversible_tools_inverse(self):
        from kassiber.ai.tools import CORE_TOOL_NAMES

        inverse = {
            "ui.transfers.pair": "ui.transfers.unpair",
            "ui.transfers.unpair": "ui.transfers.pair",
            "ui.transfers.dismiss": "ui.transfers.dismissals.delete",
            "ui.transfers.dismissals.delete": "ui.transfers.dismiss",
            "ui.journals.quarantine.resolve": "ui.transactions.history.revert",
            "ui.transactions.history.revert": "ui.transactions.history.revert",
            "ui.transfers.components.apply": "ui.transfers.components.plan",
            "ui.custody.review.apply": "ui.custody.review.plan",
            "ui.wallets.import_attachment.apply": "ui.imports.rollback.plan",
        }
        core_reversible = {
            name for name in CORE_TOOL_NAMES if get_tool(name) and get_tool(name).reversible
        }
        self.assertEqual(core_reversible - {"ui.journals.process"}, set(inverse))
        for name, undo in inverse.items():
            self.assertIn(undo, CORE_TOOL_NAMES, name)

    def test_every_hint_builder_belongs_to_a_reversible_tool(self):
        self.assertLessEqual(set(UNDO_HINTS), self.EXPECTED_REVERSIBLE)
        # Only the journal rebuild has nothing to undo.
        self.assertEqual(self.EXPECTED_REVERSIBLE - set(UNDO_HINTS), {"ui.journals.process"})


class UndoHintTest(unittest.TestCase):
    def test_representative_hints(self):
        self.assertEqual(
            undo_hint("ui.transfers.pair", {}, {"id": "pair-1"}),
            {"tool": "ui_transfers_unpair", "arguments": {"pair_id": "pair-1"}},
        )
        self.assertEqual(
            undo_hint(
                "ui.transfers.unpair",
                {"pair_id": "pair-1"},
                {
                    "deleted": "pair-1",
                    "pair": {
                        "out_transaction_id": "out",
                        "in_transaction_id": "in",
                        "kind": "chain-swap",
                        "policy": "carrying-value",
                        "notes": None,
                        "out_amount": 150_000_000,
                    },
                },
            ),
            {
                "tool": "ui_transfers_pair",
                "arguments": {
                    "tx_out": "out",
                    "tx_in": "in",
                    "kind": "chain-swap",
                    "policy": "carrying-value",
                    "out_amount": "0.0015",
                },
            },
        )
        self.assertEqual(
            undo_hint("ui.transfers.dismiss", {}, {"id": "d-1"}),
            {"tool": "ui_transfers_dismissals_delete", "arguments": {"dismissal_id": "d-1"}},
        )
        in_ten_days = (datetime.now(timezone.utc) + timedelta(days=9, hours=12)).isoformat()
        hint = undo_hint(
            "ui.transfers.dismissals.delete",
            {},
            {"dismissal": {"out_transaction_id": "o", "in_transaction_id": "i",
                           "reason": "not a swap", "expires_at": in_ten_days}},
        )
        self.assertEqual(hint["tool"], "ui_transfers_dismiss")
        self.assertEqual(hint["arguments"]["expires_in_days"], 10)
        permanent = undo_hint(
            "ui.transfers.dismissals.delete",
            {},
            {"dismissal": {"out_transaction_id": "o", "in_transaction_id": "i",
                           "reason": None, "expires_at": None}},
        )
        self.assertEqual(permanent["arguments"]["expires_in_days"], 0)
        self.assertEqual(
            undo_hint(
                "ui.journals.quarantine.resolve",
                {},
                {"transaction_id": "tx", "resolution": {"history_event_id": "ev"}},
            ),
            {"tool": "ui_transactions_history_revert",
             "arguments": {"transaction": "tx", "event": "ev"}},
        )
        # An edit that changed nothing has nothing to revert.
        self.assertIsNone(
            undo_hint("ui.transactions.metadata.update", {}, {"updated": False})
        )
        self.assertEqual(
            undo_hint(
                "ui.transfers.components.apply",
                {"action": "create"},
                {"components": [{"id": "c-1", "state": "active"}]},
            ),
            {"tool": "ui_transfers_components_plan",
             "arguments": {"action": "supersede", "component_id": "c-1"}},
        )
        self.assertEqual(
            undo_hint(
                "ui.transfers.components.apply",
                {"action": "revise", "component_id": "old"},
                {"component": {"id": "new", "state": "active"}},
            )["arguments"],
            {"action": "undo", "component_id": "old"},
        )
        # Several created components have no single inverse call.
        self.assertIsNone(
            undo_hint(
                "ui.transfers.components.apply",
                {"action": "create"},
                {"components": [{"id": "a", "state": "active"}, {"id": "b", "state": "active"}]},
            )
        )
        self.assertEqual(
            undo_hint("ui.wallets.import_attachment.apply", {}, {"import_batch_id": "batch-1"}),
            {"tool": "ui_imports_rollback_plan", "arguments": {"batch": "batch-1"}},
        )


class DismissalUndoThroughAiToolsTest(unittest.TestCase):
    def test_dismiss_returns_an_undo_that_round_trips(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-undo-") as tmp:
            phoenix = Path(tmp) / "phoenix.csv"
            phoenix.write_text(_PHOENIX_CSV, encoding="utf-8")
            liquid = Path(tmp) / "liquid.csv"
            liquid.write_text(_LIQUID_CSV, encoding="utf-8")
            data_root = Path(tmp) / "data"
            _bootstrap_profile(data_root, phoenix, liquid)
            suggested, _ = _run(
                data_root, "transfers", "suggest", "--workspace", "Main", "--profile", "Swap"
            )
            candidate = suggested["data"]["candidates"][0]
            conn = open_db(data_root)
            try:
                runtime = daemon.AiToolRuntime(str(data_root), {}, queue.Queue(), {})

                def execute(name, arguments):
                    with patch.object(
                        daemon,
                        "_run_on_daemon_main_thread",
                        side_effect=lambda _runtime, callback: callback(conn),
                    ):
                        return daemon._execute_mutating_ai_tool(
                            daemon.ParsedAiToolCall("c", name, arguments), runtime
                        )

                dismissed = execute(
                    "ui.transfers.dismiss",
                    {"tx_out": candidate["out_id"], "tx_in": candidate["in_id"],
                     "reason": "not one movement", "expires_in_days": 30},
                )
                self.assertTrue(dismissed["ok"], dismissed)
                undo = dismissed["envelope"]["data"]["undo"]
                self.assertEqual(undo["tool"], "ui_transfers_dismissals_delete")

                deleted = execute(undo["tool"], undo["arguments"])
                self.assertTrue(deleted["ok"], deleted)
                self.assertEqual(
                    conn.execute("SELECT COUNT(*) FROM transaction_pair_dismissals").fetchone()[0],
                    0,
                )
                redo = deleted["envelope"]["data"]["undo"]
                self.assertEqual(redo["tool"], "ui_transfers_dismiss")
                self.assertEqual(redo["arguments"]["tx_out"], candidate["out_id"])
                self.assertEqual(redo["arguments"]["expires_in_days"], 30)
            finally:
                conn.close()


class ReviewBatchUndismissTest(unittest.TestCase):
    def test_undismiss_pair_is_a_review_operation(self):
        from kassiber.core import review_workflow

        operations = review_workflow._operations(
            [{"type": "undismiss_pair", "out_transaction_id": "o",
              "in_transaction_id": "i", "reason": "user wants it suggested again"}]
        )
        self.assertEqual(operations[0]["type"], "undismiss_pair")
        with self.assertRaises(AppError):
            review_workflow._operations(
                [{"type": "undismiss_pair", "out_transaction_id": "o", "in_transaction_id": "i"}]
            )
        # A restricted provider cannot batch it.
        with self.assertRaises(AppError) as raised:
            daemon._validate_ai_review_operations(operations, full_access=False)
        self.assertEqual(raised.exception.code, "agent_access_restricted")


if __name__ == "__main__":
    unittest.main()
