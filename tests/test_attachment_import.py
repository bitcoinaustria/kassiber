"""The assistant imports a chat attachment only as an approved, bound plan.

Pinned here: planning changes nothing; applying lands exactly the planned rows
and refuses a changed file, a changed book or a changed effect; chain wallets
are not targets; a restricted provider gets counts, not values; no local path
reaches a tool result; and the run rolls back like a desktop import.
"""

from __future__ import annotations

import json
import os
import queue
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from kassiber import daemon
from kassiber.ai.tools import CORE_TOOL_NAMES, get_tool
from kassiber.db import open_db


REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ADDRESS = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
OPAQUE_CSV = (
    "Ausfuehrung,Richtung,Stueck,Spesen,Waehrung,Gegenwert\n"
    "2024-03-01T10:00:00Z,Kauf,0.5,0.0001,EUR,21000\n"
    "2024-04-02T10:00:00Z,Verkauf,0.25,0.0001,EUR,12000\n"
)
COLUMN_MAP = {
    "date": "Ausfuehrung",
    "type": "Richtung",
    "amount": "Stueck",
    "fee": "Spesen",
    "fiat_currency": "Waehrung",
    "fiat_value": "Gegenwert",
}


class AttachmentImportTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.data_root = os.path.join(self._tmp.name, "data")
        self.csv_path = os.path.join(self._tmp.name, "Umsatz_Mustermann.csv")
        with open(self.csv_path, "w", encoding="utf-8") as handle:
            handle.write(OPAQUE_CSV)
        for args in (
            ("init",),
            ("workspaces", "create", "W"),
            ("profiles", "create", "P", "--workspace", "W",
             "--fiat-currency", "EUR", "--tax-country", "at"),
            ("wallets", "create", "--workspace", "W", "--profile", "P",
             "--label", "Exchange XYZ", "--kind", "custom"),
            ("wallets", "create", "--workspace", "W", "--profile", "P",
             "--label", "Cold", "--kind", "address", "--address", ADDRESS),
        ):
            subprocess.run(
                [sys.executable, "-m", "kassiber", "--data-root", self.data_root,
                 "--machine", *args],
                capture_output=True, text=True, cwd=REPO_ROOT, check=True,
            )
        self.conn = open_db(self.data_root)
        self.addCleanup(self.conn.close)
        patcher = patch(
            "kassiber.daemon._run_on_daemon_main_thread",
            side_effect=lambda runtime, callback: callback(self.conn),
        )
        patcher.start()
        self.addCleanup(patcher.stop)

    def runtime(self, *, full_access=True, attached=True):
        state = {"provider_kind": "openai", "provider_full_access": full_access}
        if attached:
            state["attachment_source_file"] = self.csv_path
        return daemon.AiToolRuntime(self.data_root, {}, queue.Queue(), state)

    def read(self, name, arguments, runtime=None):
        return daemon._execute_read_only_ai_tool(
            daemon.ParsedAiToolCall("call", name, arguments), runtime or self.runtime()
        )

    def write(self, name, arguments, runtime=None):
        return daemon._execute_mutating_ai_tool(
            daemon.ParsedAiToolCall("call", name, arguments), runtime or self.runtime()
        )

    def plan(self, runtime=None, **arguments):
        arguments = {"wallet": "Exchange XYZ", "column_map": COLUMN_MAP, **arguments}
        result = self.read("ui.wallets.import_attachment.plan", arguments, runtime)
        self.assertTrue(result["ok"], result)
        return result["envelope"]["data"]

    def apply(self, plan, runtime=None, **arguments):
        arguments = {"wallet": "Exchange XYZ", "column_map": COLUMN_MAP, **arguments,
                     **plan["apply_with"]}
        return self.write("ui.wallets.import_attachment.apply", arguments, runtime)

    def tx_count(self):
        return self.conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0]

    def assert_no_local_path(self, value):
        text = json.dumps(value)
        self.assertNotIn(self._tmp.name, text)
        self.assertNotIn("kassiber-import-", text)

    def test_tools_are_in_the_default_profile_and_bound_by_consent(self):
        for name in ("ui.wallets.import_attachment.plan", "ui.wallets.import_attachment.apply",
                     "ui.imports.list", "ui.imports.rollback.plan", "ui.imports.rollback",
                     "ui.wallets.request_input"):
            self.assertIn(name, CORE_TOOL_NAMES)
        self.assertEqual(get_tool("ui.wallets.import_attachment.apply").kind_class, "mutating")
        self.assertEqual(get_tool("ui.imports.rollback").kind_class, "mutating")
        self.assertIn("ui.wallets.import_attachment.apply", daemon.AI_TOOL_ONCE_ONLY_CONSENT)
        self.assertIn("ui.imports.rollback", daemon.AI_TOOL_ONCE_ONLY_CONSENT)
        for name in ("ui.wallets.import_attachment.plan", "ui.wallets.import_attachment.apply"):
            properties = set(get_tool(name).parameters["properties"])
            self.assertFalse({"path", "file", "source_file", "token"} & properties)

    def test_plan_reports_the_effect_and_changes_nothing(self):
        plan = self.plan()
        self.assertTrue(plan["ready"], plan["blockers"])
        self.assertEqual(plan["rows"]["to_import"], 2)
        self.assertEqual(plan["date_range"]["first"], "2024-03-01T10:00:00Z")
        totals = {(row["direction"], row["asset"]): row for row in plan["totals"]}
        self.assertEqual(totals[("inbound", "BTC")]["amount_msat"], 50_000_000_000)
        self.assertEqual(totals[("outbound", "BTC")]["amount_msat"], 25_000_000_000)
        self.assertEqual(plan["target"]["completeness_class"], "manual")
        self.assertEqual(self.tx_count(), 0)
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM import_batches").fetchone()[0], 0)
        self.assert_no_local_path(plan)

    def test_apply_imports_exactly_the_plan_and_rolls_back(self):
        plan = self.plan()
        result = self.apply(plan)
        self.assertTrue(result["ok"], result)
        applied = result["envelope"]["data"]
        self.assertTrue(applied["applied"])
        self.assertEqual(applied["rows"]["to_import"], 2)
        self.assertEqual(self.tx_count(), 2)
        self.assert_no_local_path(applied)

        runs = self.read("ui.imports.list", {})["envelope"]["data"]["batches"]
        self.assertEqual([run["id"] for run in runs], [applied["import_batch_id"]])
        self.assertEqual(runs[0]["source_filename"], "Umsatz_Mustermann.csv")

        # Re-planning the same file finds nothing new.
        again = self.plan()
        self.assertFalse(again["ready"])
        self.assertIn("nothing_to_import", {b["code"] for b in again["blockers"]})

        rollback = self.read("ui.imports.rollback.plan", {"batch": applied["import_batch_id"]})
        rollback = rollback["envelope"]["data"]
        self.assertTrue(rollback["ready"])
        self.assertEqual(rollback["transactions_to_delete"], 2)
        done = self.write("ui.imports.rollback", {
            "batch": applied["import_batch_id"],
            "plan_digest": rollback["plan_digest"],
            "expected_input_version": rollback["input_version"],
        })
        self.assertTrue(done["ok"], done)
        self.assertEqual(done["envelope"]["data"]["transactions_deleted"], 2)
        self.assertEqual(self.tx_count(), 0)

    def test_apply_refuses_a_changed_attachment(self):
        plan = self.plan()
        with open(self.csv_path, "a", encoding="utf-8") as handle:
            handle.write("2024-05-03T10:00:00Z,Kauf,0.75,0.0002,EUR,30000\n")
        result = self.apply(plan)
        self.assertFalse(result["ok"])
        self.assertEqual(result["reason"], "attachment_changed")
        self.assertEqual(self.tx_count(), 0)

    def test_apply_refuses_a_stale_input_version_or_digest(self):
        plan = self.plan()
        self.conn.execute(
            "UPDATE profiles SET journal_input_version = journal_input_version + 1"
        )
        self.conn.commit()
        stale = self.apply(plan)
        self.assertFalse(stale["ok"])
        self.assertEqual(stale["reason"], "import_plan_stale")

        fresh = self.plan()
        tampered = self.apply({**fresh, "apply_with": {**fresh["apply_with"], "plan_digest": "0" * 64}})
        self.assertFalse(tampered["ok"])
        self.assertEqual(tampered["reason"], "import_plan_stale")
        # A different target than the one planned is a different effect.
        retarget = self.apply(fresh, wallet=None, new_wallet={"label": "Other"})
        self.assertFalse(retarget["ok"])
        self.assertEqual(self.tx_count(), 0)

    def test_chain_wallets_are_not_targets(self):
        plan = self.plan(wallet="Cold")
        self.assertFalse(plan["ready"])
        self.assertIn("target_not_file_wallet", {b["code"] for b in plan["blockers"]})
        result = self.apply(plan, wallet="Cold")
        self.assertFalse(result["ok"])
        self.assertEqual(result["reason"], "import_plan_blocked")
        self.assertEqual(self.tx_count(), 0)

    def test_a_new_wallet_is_created_only_on_apply(self):
        arguments = {"wallet": None, "new_wallet": {"label": "Broker"}}
        arguments = {key: value for key, value in arguments.items() if value is not None}
        plan_result = self.read(
            "ui.wallets.import_attachment.plan", {**arguments, "column_map": COLUMN_MAP}
        )
        plan = plan_result["envelope"]["data"]
        self.assertTrue(plan["ready"], plan["blockers"])
        self.assertIsNone(
            self.conn.execute("SELECT 1 FROM wallets WHERE label = 'Broker'").fetchone()
        )
        result = self.write("ui.wallets.import_attachment.apply", {
            **arguments, "column_map": COLUMN_MAP, **plan["apply_with"],
        })
        self.assertTrue(result["ok"], result)
        row = self.conn.execute("SELECT kind FROM wallets WHERE label = 'Broker'").fetchone()
        self.assertEqual(row["kind"], "custom")
        self.assertTrue(result["envelope"]["data"]["wallet"]["created"])

    def test_restricted_provider_gets_counts_not_values(self):
        runtime = self.runtime(full_access=False)
        plan = self.plan(runtime)
        self.assertTrue(plan["cell_values_withheld"])
        self.assertIsNone(plan["date_range"])
        self.assertNotIn("filename", plan["attachment"])
        self.assertNotIn("amount_msat", json.dumps(plan["totals"]))
        self.assertEqual(plan["rows"]["to_import"], 2)
        result = self.apply(plan, runtime)
        self.assertTrue(result["ok"], result)
        runs = self.read("ui.imports.list", {}, runtime)["envelope"]["data"]["batches"]
        self.assertNotIn("source_filename", runs[0])

    def test_asset_hints_and_missing_attachments_are_refused(self):
        hinted = self.read("ui.wallets.import_attachment.plan", {
            "wallet": "Exchange XYZ", "column_map": {**COLUMN_MAP, "amount_header_asset": "SATS"},
        })
        self.assertFalse(hinted["ok"])
        missing = self.read(
            "ui.wallets.import_attachment.plan",
            {"wallet": "Exchange XYZ"},
            self.runtime(attached=False),
        )
        self.assertFalse(missing["ok"])
        self.assertEqual(missing["reason"], "ai_attachment_missing")

    def test_rollback_that_would_delete_reviewed_work_is_blocked(self):
        applied = self.apply(self.plan())["envelope"]["data"]
        tx_id = self.conn.execute("SELECT id FROM transactions LIMIT 1").fetchone()[0]
        profile = self.conn.execute("SELECT id, workspace_id FROM profiles").fetchone()
        self.conn.execute(
            "INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at) "
            "VALUES('t1', ?, ?, 'reviewed', 'Reviewed', '2024-01-01T00:00:00Z')",
            (profile["workspace_id"], profile["id"]),
        )
        self.conn.execute("INSERT INTO transaction_tags VALUES(?, 't1')", (tx_id,))
        self.conn.commit()
        plan = self.read("ui.imports.rollback.plan", {"batch": applied["import_batch_id"]})
        plan = plan["envelope"]["data"]
        self.assertFalse(plan["ready"])
        blocker = plan["blockers"][0]
        self.assertEqual(blocker["code"], "rollback_removes_reviewed_work")
        self.assertTrue(blocker["remediation"]["desktop_only"])
        result = self.write("ui.imports.rollback", {
            "batch": applied["import_batch_id"],
            "plan_digest": plan["plan_digest"],
            "expected_input_version": plan["input_version"],
        })
        self.assertFalse(result["ok"])
        self.assertEqual(result["reason"], "rollback_blocked")
        self.assertEqual(self.tx_count(), 2)

    def test_consent_card_shows_the_daemons_plan(self):
        plan = self.plan()
        arguments = {"wallet": "Exchange XYZ", "column_map": COLUMN_MAP, **plan["apply_with"]}
        preview, summary = daemon._ai_plan_consent_preview(
            self.runtime(), get_tool("ui.wallets.import_attachment.apply"), arguments
        )
        self.assertEqual(preview["status"], "ready")
        self.assertEqual(preview["plan"]["rows"]["to_import"], 2)
        self.assertEqual(summary, "Import 2 row(s) from the attached file into wallet Exchange XYZ")
        stale, _ = daemon._ai_plan_consent_preview(
            self.runtime(), get_tool("ui.wallets.import_attachment.apply"),
            {**arguments, "plan_digest": "0" * 64},
        )
        self.assertEqual(stale["status"], "stale")

    def test_descriptor_wallets_are_a_typed_handoff(self):
        result = self.read("ui.wallets.request_input", {
            "source": "descriptor", "explanation": "Connect the cold wallet to see its spends.",
        })
        self.assertTrue(result["ok"], result)
        packet = result["envelope"]["data"]
        self.assertEqual(packet["action"], "connect_wallet")
        self.assertEqual(packet["source"], "descriptor")
        self.assertTrue(packet["desktop_only"])
        self.assertRegex(packet["request_id"], "^[0-9a-f]{64}$")
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM wallets").fetchone()[0], 2
        )
        refused = self.read("ui.wallets.request_input", {"source": "custom"})
        self.assertFalse(refused["ok"])


class WalletInputSourceDriftTests(unittest.TestCase):
    def test_handoff_sources_match_the_desktop_card_and_catalog(self):
        import re

        from kassiber.ai.tools import WALLET_INPUT_SOURCES

        ui = os.path.join(REPO_ROOT, "ui-tauri", "src")
        with open(os.path.join(ui, "components", "ai", "walletInputRequest.ts"), encoding="utf-8") as handle:
            card = handle.read()
        block = re.search(r"WALLET_INPUT_SOURCES = \[(.*?)\] as const", card, re.DOTALL)
        self.assertIsNotNone(block)
        self.assertEqual(tuple(re.findall(r'"([^"]+)"', block.group(1))), WALLET_INPUT_SOURCES)
        with open(os.path.join(ui, "lib", "connectionCatalog.tsx"), encoding="utf-8") as handle:
            catalog_ids = set(re.findall(r'^    id: "([^"]+)"', handle.read(), re.MULTILINE))
        self.assertLessEqual(set(WALLET_INPUT_SOURCES), catalog_ids)


if __name__ == "__main__":
    unittest.main()
