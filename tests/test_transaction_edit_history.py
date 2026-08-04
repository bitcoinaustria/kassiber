import json
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.cli.handlers import (
    _metadata_hooks,
    resolve_quarantine_exclude,
    resolve_quarantine_price_override,
    resolve_scope,
    resolve_transaction,
)
from kassiber.core import audit_package
from kassiber.core import metadata as core_metadata
from kassiber.core.ui_snapshot import build_report_blockers_snapshot
from kassiber.db import open_db
from kassiber.daemon import (
    _quarantine_resolution_payload,
    _transaction_metadata_update_payload,
)
from kassiber.errors import AppError
from kassiber.time_utils import now_iso


ROOT = Path(__file__).resolve().parent.parent


class TransactionEditHistoryTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-edit-history-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.data_root = self.root / "data"
        self._seed()

    def _run_cli(self, *args, machine=False):
        cmd = [sys.executable, "-m", "kassiber", "--data-root", str(self.data_root)]
        if machine:
            cmd.append("--machine")
        cmd.extend(args)
        return subprocess.run(
            cmd,
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def _run_json(self, *args):
        result = self._run_cli(*args, machine=True)
        self.assertEqual(result.returncode, 0, msg=result.stderr or result.stdout)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["schema_version"], 1)
        return payload

    def _seed(self):
        csv_path = self.root / "transactions.csv"
        csv_path.write_text(
            "\n".join(
                [
                    "date,txid,direction,asset,amount,fee,fiat_rate,description",
                    "2026-01-01T10:00:00Z,seed-inbound-1,inbound,BTC,0.10000000,0,50000,Seed acquisition",
                    "2026-01-02T10:00:00Z,seed-inbound-2,inbound,BTC,0.20000000,0,60000,Second acquisition",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        for args in (
            ("init",),
            ("workspaces", "create", "Demo"),
            ("profiles", "create", "Main", "--fiat-currency", "EUR", "--tax-country", "at"),
            (
                "wallets",
                "create",
                "--label",
                "Cold",
                "--kind",
                "address",
                "--address",
                "bc1qtestaddress0000000000000000000000000000000",
            ),
            ("wallets", "import-csv", "--wallet", "Cold", "--file", str(csv_path)),
            ("metadata", "tags", "create", "--code", "reviewed", "--label", "Reviewed"),
            ("metadata", "tags", "create", "--code", "income", "--label", "Income"),
        ):
            result = self._run_cli(*args, machine=True)
            self.assertEqual(result.returncode, 0, msg=result.stderr or result.stdout)

    def test_cli_history_captures_real_edits_and_suppresses_noops(self):
        first = self._run_json(
            "metadata",
            "records",
            "note",
            "set",
            "--transaction",
            "seed-inbound-1",
            "--note",
            "Invoice 42 reviewed",
            "--reason",
            "matched invoice",
        )
        self.assertTrue(first["data"]["updated"])
        self.assertTrue(first["data"]["history_event_id"])

        noop = self._run_json(
            "metadata",
            "records",
            "note",
            "set",
            "--transaction",
            "seed-inbound-1",
            "--note",
            "Invoice 42 reviewed",
        )
        self.assertFalse(noop["data"]["updated"])
        self.assertIsNone(noop["data"]["history_event_id"])

        history = self._run_json(
            "metadata",
            "records",
            "history",
            "list",
            "--transaction",
            "seed-inbound-1",
        )
        self.assertEqual(history["kind"], "metadata.records.history.list")
        self.assertEqual(len(history["data"]["events"]), 1)
        event = history["data"]["events"][0]
        self.assertEqual(event["source"], "cli")
        self.assertEqual(event["reason"], "matched invoice")
        self.assertEqual(event["summary"], "Note updated")
        self.assertEqual(event["fields"][0]["field"], "note")
        self.assertEqual(event["fields"][0]["before_value"], None)
        self.assertEqual(event["fields"][0]["after_value"], "Invoice 42 reviewed")

    def test_grouped_fields_sources_filters_pagination_and_stale_counts(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        hooks = _metadata_hooks()
        first = core_metadata.update_transaction_metadata(
            conn,
            None,
            None,
            "seed-inbound-1",
            hooks,
            note="Desktop review",
            note_set=True,
            tags=["Reviewed", "Income"],
            excluded=True,
            pricing_update={
                "fiat_currency": "EUR",
                "fiat_rate": "65000",
                "source_kind": "manual_override",
                "quality": "exact",
                "external_ref": "invoice=42 secret=not-exported",
            },
            review_status="review",
            review_status_set=True,
            taxable=False,
            taxable_set=True,
            at_regime="outside",
            at_regime_set=True,
            at_category="none",
            at_category_set=True,
            source="gui",
            reason="desktop detail save",
        )
        self.assertTrue(first["updated"])
        second = core_metadata.update_transaction_metadata(
            conn,
            None,
            None,
            "seed-inbound-2",
            hooks,
            note="Assistant reviewed",
            note_set=True,
            source="ai_tool",
            reason="assistant tool call",
        )
        self.assertTrue(second["updated"])

        activity = core_metadata.list_activity_history(conn, None, None, hooks, limit=1)
        self.assertEqual(len(activity["events"]), 1)
        self.assertTrue(activity["has_more"])
        next_page = core_metadata.list_activity_history(
            conn,
            None,
            None,
            hooks,
            cursor=activity["next_cursor"],
            limit=1,
        )
        self.assertEqual(len(next_page["events"]), 1)

        pricing = core_metadata.list_activity_history(
            conn,
            None,
            None,
            hooks,
            pricing_only=True,
        )
        self.assertEqual(len(pricing["events"]), 1)
        self.assertEqual(pricing["events"][0]["summary"], "Pricing provenance updated")
        self.assertIn("pricing", pricing["events"][0]["families"])
        external_ref = next(
            field
            for field in pricing["events"][0]["fields"]
            if field["field"] == "pricing_external_ref"
        )
        self.assertIn("[redacted]", external_ref["after_value"])
        self.assertTrue(external_ref["redacted"])

        ai = core_metadata.list_activity_history(conn, None, None, hooks, ai_only=True)
        self.assertEqual(len(ai["events"]), 1)
        self.assertEqual(ai["events"][0]["source"], "ai_tool")

        stale = core_metadata.stale_transaction_edit_summary(conn, None, None, hooks)
        self.assertEqual(stale["edit_count"], 2)
        self.assertEqual(stale["source_counts"]["ai_tool"], 1)
        self.assertEqual(stale["source_counts"]["gui"], 1)
        self.assertIn("pricing", stale["family_counts"])

        blockers = build_report_blockers_snapshot(conn)
        stale_blocker = next(item for item in blockers["blockers"] if item["id"] == "journals_stale")
        self.assertEqual(stale_blocker["edit_history"]["edit_count"], 2)

    def test_activity_history_can_skip_stale_summary_for_fast_timeline_pages(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        hooks = _metadata_hooks()
        edited = core_metadata.update_transaction_metadata(
            conn,
            None,
            None,
            "seed-inbound-1",
            hooks,
            note="Desktop review",
            note_set=True,
            source="gui",
            reason="desktop detail save",
        )
        self.assertTrue(edited["updated"])

        with patch("kassiber.core.transaction_history.stale_summary") as stale_summary:
            activity = core_metadata.list_activity_history(
                conn,
                None,
                None,
                hooks,
                limit=1,
                include_stale=False,
            )

        stale_summary.assert_not_called()
        self.assertEqual(len(activity["events"]), 1)
        self.assertNotIn("stale", activity)

    def test_revert_creates_forward_edit_and_tag_diff_snapshot(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        hooks = _metadata_hooks()
        core_metadata.update_transaction_metadata(
            conn,
            None,
            None,
            "seed-inbound-1",
            hooks,
            note="Initial note",
            note_set=True,
            tags=["Reviewed", "Income"],
            source="gui",
        )
        edited = core_metadata.update_transaction_metadata(
            conn,
            None,
            None,
            "seed-inbound-1",
            hooks,
            note="Changed note",
            note_set=True,
            tags=["Reviewed"],
            source="gui",
        )
        event_id = edited["history_event_id"]
        history = core_metadata.list_transaction_history(conn, None, None, "seed-inbound-1", hooks)
        tag_event = next(event for event in history["events"] if event["id"] == event_id)
        tag_field = next(field for field in tag_event["fields"] if field["field"] == "tags")
        self.assertEqual(tag_field["diff"]["removed"], ["Income"])

        reverted = core_metadata.revert_transaction_edit(
            conn,
            None,
            None,
            "seed-inbound-1",
            hooks,
            event_id=event_id,
            field="note",
            source="gui",
            reason="undo note only",
        )
        self.assertTrue(reverted["updated"])
        self.assertEqual(reverted["transaction"]["note"], "Initial note")
        self.assertEqual(reverted["reverted_fields"], ["note"])

        after = core_metadata.list_transaction_history(conn, None, None, "seed-inbound-1", hooks)
        self.assertEqual(len(after["events"]), 3)
        revert_event = next(event for event in after["events"] if event["id"] == reverted["history_event_id"])
        self.assertEqual(revert_event["reason"], "undo note only")
        self.assertEqual(revert_event["fields"][0]["after_value"], "Initial note")

    def test_quarantine_resolution_uses_audited_metadata_path(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        _, profile = resolve_scope(conn, None, None)
        first_tx = resolve_transaction(conn, profile["id"], "seed-inbound-1")
        second_tx = resolve_transaction(conn, profile["id"], "seed-inbound-2")
        conn.executemany(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    first_tx["id"],
                    profile["workspace_id"],
                    profile["id"],
                    "missing_price",
                    "{}",
                    now_iso(),
                ),
                (
                    second_tx["id"],
                    profile["workspace_id"],
                    profile["id"],
                    "missing_price",
                    "{}",
                    now_iso(),
                ),
            ],
        )
        conn.commit()

        excluded = resolve_quarantine_exclude(conn, None, None, "seed-inbound-1")
        self.assertTrue(excluded["history_event_id"])
        override = resolve_quarantine_price_override(
            conn,
            None,
            None,
            "seed-inbound-2",
            fiat_rate="70000",
        )
        self.assertTrue(override["history_event_id"])

        remaining = conn.execute("SELECT COUNT(*) AS count FROM journal_quarantines").fetchone()
        self.assertEqual(remaining["count"], 0)
        first_history = core_metadata.list_transaction_history(
            conn,
            None,
            None,
            "seed-inbound-1",
            _metadata_hooks(),
        )
        first_event = first_history["events"][0]
        self.assertEqual(first_event["source"], "cli")
        self.assertEqual(first_event["summary"], "Excluded from reports")
        self.assertEqual(first_event["fields"][0]["field"], "excluded")
        second_history = core_metadata.list_transaction_history(
            conn,
            None,
            None,
            "seed-inbound-2",
            _metadata_hooks(),
        )
        self.assertEqual(second_history["events"][0]["summary"], "Pricing provenance updated")

    def test_ai_quarantine_resolution_is_narrow_audited_and_verified(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        _, profile = resolve_scope(conn, None, None)
        tx = resolve_transaction(conn, profile["id"], "seed-inbound-1")
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                tx["id"],
                profile["workspace_id"],
                profile["id"],
                "missing_price",
                "{}",
                now_iso(),
            ),
        )
        conn.commit()

        payload = _quarantine_resolution_payload(
            conn,
            {
                "transaction": "seed-inbound-1",
                "action": "price_override",
                "fiat_rate": "71000",
                "reason": "Reviewed exchange statement",
                "reprocess": False,
            },
            default_source="ai_tool",
        )

        self.assertTrue(payload["cleared"])
        self.assertFalse(payload["reprocessed"])
        self.assertIsNone(payload["remaining_quarantine"])
        history = core_metadata.list_transaction_history(
            conn,
            None,
            None,
            "seed-inbound-1",
            _metadata_hooks(),
        )
        self.assertEqual(history["events"][0]["source"], "ai_tool")
        self.assertEqual(history["events"][0]["reason"], "Reviewed exchange statement")

        with self.assertRaisesRegex(AppError, "requires fiat_rate or fiat_value"):
            _quarantine_resolution_payload(
                conn,
                {
                    "transaction": "seed-inbound-1",
                    "action": "price_override",
                    "reason": "No evidence supplied",
                },
                default_source="ai_tool",
            )

    def test_audit_package_includes_edit_history_only_when_requested(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        hooks = _metadata_hooks()
        core_metadata.update_transaction_metadata(
            conn,
            None,
            None,
            "seed-inbound-1",
            hooks,
            note="Auditor note",
            note_set=True,
            source="gui",
        )
        audit_hooks = audit_package.AuditPackageHooks(
            resolve_scope=resolve_scope,
            resolve_transaction=resolve_transaction,
            now_iso=now_iso,
        )
        excluded_dir = self.root / "audit-no-history"
        excluded = audit_package.export_audit_package(
            conn,
            str(self.data_root),
            None,
            None,
            excluded_dir,
            audit_hooks,
            transaction_refs=["seed-inbound-1"],
        )
        excluded_manifest = json.loads(Path(excluded["manifest"]).read_text(encoding="utf-8"))
        self.assertFalse(excluded_manifest["summary"]["edit_history_included"])
        self.assertEqual(excluded_manifest["summary"]["edit_history_event_count"], 1)
        self.assertNotIn("edit_history", excluded_manifest["transactions"][0])
        self.assertIn(
            "edit_history_excluded",
            {warning["code"] for warning in excluded_manifest["package"]["warnings"]},
        )

        included_dir = self.root / "audit-with-history"
        included = audit_package.export_audit_package(
            conn,
            str(self.data_root),
            None,
            None,
            included_dir,
            audit_hooks,
            transaction_refs=["seed-inbound-1"],
            include_edit_history=True,
        )
        included_manifest = json.loads(Path(included["manifest"]).read_text(encoding="utf-8"))
        self.assertTrue(included_manifest["summary"]["edit_history_included"])
        self.assertEqual(len(included_manifest["transactions"][0]["edit_history"]), 1)

    def test_kind_classification_is_auditable_and_direction_guarded(self):
        # Tags are cosmetic; `kind` is what the tax engine reads. Without this
        # path a Lightning/third-party receipt could never be declared as
        # income — it stayed an unclassified inbound and rp2 booked it BUY.
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        hooks = _metadata_hooks()

        record = core_metadata.update_transaction_metadata(
            conn, None, None, "seed-inbound-1", hooks,
            kind="income", kind_set=True,
            source="gui", reason="freelance invoice paid over LN",
        )
        self.assertTrue(record["updated"])
        stored = resolve_transaction(conn, resolve_scope(conn, None, None)[1]["id"], "seed-inbound-1")
        self.assertEqual(stored["kind_override"], "income")

        history = self._run_json(
            "metadata", "records", "history", "list", "--transaction", "seed-inbound-1",
        )["data"]["events"]
        kind_fields = [
            field
            for event in history
            for field in event["fields"]
            if field["field"] == "kind"
        ]
        self.assertEqual(len(kind_fields), 1)
        self.assertIsNone(kind_fields[0]["before_value"])
        self.assertEqual(kind_fields[0]["after_value"], "income")

        # Clearing returns it to unclassified rather than to a guessed kind.
        core_metadata.update_transaction_metadata(
            conn, None, None, "seed-inbound-1", hooks, kind=None, kind_set=True,
        )
        stored = resolve_transaction(conn, resolve_scope(conn, None, None)[1]["id"], "seed-inbound-1")
        self.assertIsNone(stored["kind_override"])

        # A disposal kind on an inbound row is a classification error, not a
        # silent write: income kinds describe what arrived.
        with self.assertRaises(AppError) as ctx:
            core_metadata.update_transaction_metadata(
                conn, None, None, "seed-inbound-1", hooks, kind="sell", kind_set=True,
            )
        self.assertEqual(ctx.exception.code, "validation")

        with self.assertRaises(AppError):
            core_metadata.update_transaction_metadata(
                conn, None, None, "seed-inbound-1", hooks,
                kind="not-a-kind", kind_set=True,
            )

    def test_kind_override_is_revertable_and_leaves_provenance_intact(self):
        # The classification is an override: `kind` keeps the importer's
        # provenance (trust checks such as the Lightning payment-hash pairing
        # in transfers.py key off it), so clearing restores the source value
        # instead of blanking the row.
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        hooks = _metadata_hooks()
        profile_id = resolve_scope(conn, None, None)[1]["id"]
        conn.execute(
            "UPDATE transactions SET kind = 'lnd_invoice' WHERE external_id = 'seed-inbound-1'"
        )
        conn.commit()

        # One save carrying an unrelated field plus the classification, as the
        # desktop detail sheet does.
        core_metadata.update_transaction_metadata(
            conn, None, None, "seed-inbound-1", hooks,
            note="combined edit", note_set=True,
            kind="income", kind_set=True, source="gui",
        )
        row = resolve_transaction(conn, profile_id, "seed-inbound-1")
        self.assertEqual(row["kind"], "lnd_invoice")
        self.assertEqual(row["kind_override"], "income")

        event = core_metadata.list_transaction_history(
            conn, None, None, "seed-inbound-1", hooks,
        )["events"][0]
        kind_field = next(f for f in event["fields"] if f["field"] == "kind")
        self.assertEqual(kind_field["label"], "Recorded as")

        # Reverting the whole event must not choke on the tax field and take
        # the note down with it.
        core_metadata.revert_transaction_edit(
            conn, None, None, "seed-inbound-1", hooks, event_id=event["id"],
        )
        row = resolve_transaction(conn, profile_id, "seed-inbound-1")
        self.assertIsNone(row["kind_override"])
        self.assertIsNone(row["note"])
        self.assertEqual(row["kind"], "lnd_invoice")

    def test_cli_and_daemon_can_set_the_classification(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        profile_id = resolve_scope(conn, None, None)[1]["id"]

        self._run_json(
            "metadata", "records", "kind", "set",
            "--transaction", "seed-inbound-2", "--kind", "mining",
        )
        self.assertEqual(
            resolve_transaction(conn, profile_id, "seed-inbound-2")["kind_override"],
            "mining",
        )

        payload = _transaction_metadata_update_payload(
            conn,
            {"transaction": "seed-inbound-2", "kind": "staking"},
            default_source="gui",
        )
        self.assertTrue(payload["updated"])
        self.assertEqual(
            resolve_transaction(conn, profile_id, "seed-inbound-2")["kind_override"],
            "staking",
        )

        # The daemon path is not a validation bypass.
        with self.assertRaises(AppError):
            _transaction_metadata_update_payload(
                conn,
                {"transaction": "seed-inbound-2", "kind": "sell"},
                default_source="gui",
            )

        self._run_json(
            "metadata", "records", "kind", "clear", "--transaction", "seed-inbound-2",
        )
        self.assertIsNone(
            resolve_transaction(conn, profile_id, "seed-inbound-2")["kind_override"],
        )

    def test_desktop_kind_vocabulary_matches_the_python_one(self):
        # `TRANSACTION_KIND_OPTIONS` is a hand-written mirror of the Python
        # vocabulary; without this it drifts silently and the UI offers kinds
        # the daemon rejects (or hides ones it accepts).
        source = (ROOT / "ui-tauri/src/lib/transactionTypeLabel.ts").read_text(
            encoding="utf-8",
        )
        block = source.split("TRANSACTION_KIND_OPTIONS = [", 1)[1].split("] as const", 1)[0]
        ts_kinds = {
            (match.group(1), match.group(2) == "true")
            for match in re.finditer(
                r'\{\s*kind:\s*"([a-z_]+)",\s*inbound:\s*(true|false)', block,
            )
        }
        py_kinds = {
            (kind, direction == "inbound")
            for kind, direction in core_metadata.SUPPORTED_TRANSACTION_KINDS.items()
        }
        self.assertEqual(ts_kinds, py_kinds)


if __name__ == "__main__":
    unittest.main()
