import json
import tempfile
import unittest
from pathlib import Path

from kassiber.core import attachments as core_attachments
from kassiber.core import audit_package
from kassiber.core import source_funds
from kassiber.db import open_db, resolve_attachments_root
from kassiber.errors import AppError


NOW = "2026-05-01T12:00:00Z"


def _format_table(*args, **kwargs):
    return []


class AuditPackageCoreTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-audit-package-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.data_root = self.root / "data"
        self.conn = open_db(self.data_root)
        self.workspace_id = "ws"
        self.profile_id = "pf"
        self.account_id = "acct"
        self.wallet_id = "wallet"
        self.tx_id = "tx-target"
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES (?, ?, ?)",
            (self.workspace_id, "Workspace", NOW),
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (self.profile_id, self.workspace_id, "Default", "EUR", "generic", 365, "FIFO", NOW),
        )
        self.conn.execute(
            """
            INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            """,
            (self.account_id, self.workspace_id, self.profile_id, "main", "Main", "personal", NOW),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (self.wallet_id, self.workspace_id, self.profile_id, self.account_id, "Wallet", "custom", "{}", NOW),
        )
        self._insert_transaction()
        self.conn.commit()
        self.audit_hooks = audit_package.AuditPackageHooks(
            resolve_scope=self._resolve_scope,
            resolve_transaction=self._resolve_transaction,
            now_iso=lambda: NOW,
        )
        self.attachment_hooks = core_attachments.AttachmentHooks(
            resolve_scope=self._resolve_scope,
            resolve_transaction=self._resolve_transaction,
            now_iso=lambda: NOW,
        )
        self.source_hooks = source_funds.SourceFundsHooks(
            resolve_scope=self._resolve_scope,
            resolve_transaction=self._resolve_transaction,
            format_table=_format_table,
        )

    def _insert_transaction(self):
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, raw_json, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                self.tx_id,
                self.workspace_id,
                self.profile_id,
                self.wallet_id,
                "target-ext",
                "fp-target",
                "2026-04-01T09:00:00Z",
                "inbound",
                "BTC",
                100_000_000,
                0,
                "EUR",
                50_000.0,
                50.0,
                "deposit",
                "Board decision funded treasury receive",
                "{}",
                NOW,
            ),
        )

    def _insert_source_transaction(self):
        source_tx_id = "tx-source"
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, raw_json, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                source_tx_id,
                self.workspace_id,
                self.profile_id,
                self.wallet_id,
                "recurring-approval-jan",
                "fp-source",
                "2026-01-31T09:00:00Z",
                "inbound",
                "BTC",
                100_000_000,
                0,
                "EUR",
                50_000.0,
                50.0,
                "treasury",
                "Recurring payment approved by board decision",
                "{}",
                NOW,
            ),
        )
        self.conn.commit()
        return source_tx_id

    def _resolve_scope(self, conn, workspace_ref, profile_ref):
        workspace = conn.execute("SELECT * FROM workspaces WHERE id = ?", (self.workspace_id,)).fetchone()
        profile = conn.execute("SELECT * FROM profiles WHERE id = ?", (self.profile_id,)).fetchone()
        return workspace, profile

    def _resolve_transaction(self, conn, profile_id, ref, direction=None):
        row = conn.execute(
            "SELECT * FROM transactions WHERE profile_id = ? AND (id = ? OR external_id = ?)",
            (profile_id, ref, ref),
        ).fetchone()
        if row is None:
            raise AppError(f"Transaction '{ref}' not found", code="not_found")
        return row

    def _mark_journals_current(self):
        self.conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = ?, last_processed_tx_count = 1,
                last_processed_input_version = journal_input_version
            WHERE id = ?
            """,
            (NOW, self.profile_id),
        )
        self.conn.commit()

    def _add_file_attachment(self, name="receipt.pdf", content=b"receipt\n", tx_id=None):
        path = self.root / name
        path.write_bytes(content)
        return core_attachments.add_attachment(
            self.conn,
            str(self.data_root),
            None,
            None,
            tx_id or self.tx_id,
            self.attachment_hooks,
            file_path=str(path),
            label=name,
        )

    def _add_url_attachment(self, url="https://docs.example.test/board-decision", tx_id=None):
        return core_attachments.add_attachment(
            self.conn,
            str(self.data_root),
            None,
            None,
            tx_id or self.tx_id,
            self.attachment_hooks,
            url=url,
            label="Board decision link",
        )

    def _add_source_link(self, *, attachment_ids, state="reviewed"):
        source = source_funds.create_source(
            self.conn,
            None,
            None,
            self.source_hooks,
            source_type="fiat_purchase",
            label="Exchange purchase",
            asset="BTC",
            amount="0.00100000",
            acquired_at="2026-03-01T09:00:00Z",
            attachment_ids=attachment_ids[:1],
        )
        return source_funds.create_link(
            self.conn,
            None,
            None,
            self.source_hooks,
            to_transaction_ref=self.tx_id,
            from_source_ref=source["id"],
            link_type="manual_source",
            state=state,
            confidence="exact",
            method="manual",
            allocation_amount="0.00100000",
            allocation_policy="explicit",
            explanation="Reviewed source evidence for auditor handoff.",
            attachment_ids=attachment_ids[1:],
        )

    def _add_reviewed_source_link(self, *, attachment_ids):
        return self._add_source_link(attachment_ids=attachment_ids, state="reviewed")

    def test_evidence_summary_flags_persisted_missing_state(self):
        summary = audit_package.build_evidence_summary(
            self.conn,
            str(self.data_root),
            None,
            None,
            self.audit_hooks,
            transaction_refs=[self.tx_id],
        )

        tx_summary = summary["transactions"][0]
        warning_codes = {
            warning["code"]
            for warning in tx_summary["readiness"]["warnings"]
        }
        self.assertEqual(tx_summary["readiness"]["status"], "blocked")
        self.assertIn("receipt_missing", warning_codes)
        self.assertIn("decision_evidence_missing", warning_codes)
        self.assertIn("source_link_missing", warning_codes)
        self.assertIn("pricing_evidence_missing", warning_codes)
        self.assertIn("journal_stale", warning_codes)
        self.assertIn("sensitive_material_excluded", warning_codes)

    def test_empty_transaction_scope_stays_empty(self):
        summary = audit_package.build_evidence_summary(
            self.conn,
            str(self.data_root),
            None,
            None,
            self.audit_hooks,
            transaction_refs=[],
        )

        self.assertEqual(summary["scope"], {"type": "transactions", "transaction_count": 0})
        self.assertEqual(summary["summary"]["transaction_count"], 0)
        self.assertEqual(summary["transactions"], [])

    def test_audit_package_manifest_includes_files_urls_and_exclusions(self):
        self._mark_journals_current()
        self.conn.execute(
            """
            UPDATE transactions
            SET pricing_source_kind = 'exchange_execution',
                pricing_quality = 'exact',
                pricing_external_ref = 'statement-row-1'
            WHERE id = ?
            """,
            (self.tx_id,),
        )
        self.conn.commit()
        file_attachment = self._add_file_attachment()
        url_attachment = self._add_url_attachment()
        self._add_reviewed_source_link(
            attachment_ids=[url_attachment["id"], file_attachment["id"]]
        )
        output_dir = self.root / "exports" / "audit"

        result = audit_package.export_audit_package(
            self.conn,
            str(self.data_root),
            None,
            None,
            output_dir,
            self.audit_hooks,
            transaction_refs=[self.tx_id],
        )

        manifest = json.loads(Path(result["manifest"]).read_text(encoding="utf-8"))
        self.assertEqual(manifest["summary"]["transaction_count"], 1)
        self.assertEqual(manifest["package"]["options"]["include_copied_attachments"], True)
        self.assertEqual(manifest["package"]["options"]["include_url_references"], True)
        self.assertIn("wallet descriptors", manifest["excluded_sensitive_material"])
        self.assertIn("backend URLs", manifest["excluded_sensitive_material"])
        evidence_files = manifest["package"]["evidence_files"]
        self.assertEqual(len(evidence_files), 1)
        copied = output_dir / evidence_files[0]["path"]
        self.assertTrue(copied.exists())
        self.assertEqual(evidence_files[0]["sha256"], file_attachment["sha256"])
        references = manifest["package"]["url_references"]
        self.assertEqual(references[0]["url"], "https://docs.example.test/board-decision")
        tx_manifest = manifest["transactions"][0]
        direct_ids = {
            attachment["attachment_type"]: attachment["id"]
            for attachment in tx_manifest["direct_attachments"]
        }
        self.assertEqual(direct_ids["file"], file_attachment["id"])
        self.assertEqual(direct_ids["url"], url_attachment["id"])
        self.assertEqual(tx_manifest["source_funds_links"][0]["state"], "reviewed")

    def test_audit_package_sanitizes_attachment_id_in_copied_evidence_path(self):
        self._mark_journals_current()
        file_attachment = self._add_file_attachment(name="receipt.txt", content=b"controlled\n")
        malicious_id = "../../escaped/owned"
        self.conn.execute(
            "UPDATE attachments SET id = ? WHERE id = ?",
            (malicious_id, file_attachment["id"]),
        )
        self.conn.commit()
        output_dir = self.root / "exports" / "audit"

        result = audit_package.export_audit_package(
            self.conn,
            str(self.data_root),
            None,
            None,
            output_dir,
            self.audit_hooks,
            transaction_refs=[self.tx_id],
        )

        manifest = json.loads(Path(result["manifest"]).read_text(encoding="utf-8"))
        evidence_files = manifest["package"]["evidence_files"]
        self.assertEqual(len(evidence_files), 1)
        self.assertEqual(evidence_files[0]["attachment_id"], malicious_id)
        self.assertEqual(evidence_files[0]["path"], "evidence/escaped_owned-receipt.txt")
        self.assertTrue((output_dir / evidence_files[0]["path"]).exists())
        self.assertFalse((self.root / "escaped" / "owned-receipt.txt").exists())

    def test_copy_evidence_duplicates_file_and_url_rows_with_provenance(self):
        source_tx_id = self._insert_source_transaction()
        source_file = self._add_file_attachment(
            name="board-approval.pdf",
            content=b"board approval\n",
            tx_id=source_tx_id,
        )
        source_url = self._add_url_attachment(
            "https://docs.example.test/board/approval",
            tx_id=source_tx_id,
        )
        result = core_attachments.copy_attachments(
            self.conn,
            str(self.data_root),
            None,
            None,
            self.tx_id,
            [source_file["id"], source_url["id"]],
            self.attachment_hooks,
            source_tx_ref=source_tx_id,
        )

        self.assertEqual(result["copied"], 2)
        copied_by_kind = {
            attachment["attachment_type"]: attachment
            for attachment in result["attachments"]
        }
        copied_file = copied_by_kind["file"]
        copied_url = copied_by_kind["url"]
        self.assertNotEqual(copied_file["id"], source_file["id"])
        self.assertNotEqual(copied_file["stored_relpath"], source_file["stored_relpath"])
        self.assertEqual(copied_file["sha256"], source_file["sha256"])
        self.assertEqual(copied_file["size_bytes"], source_file["size_bytes"])
        self.assertEqual(copied_file["copied_from_attachment_id"], source_file["id"])
        self.assertEqual(copied_file["copied_from_transaction_id"], source_tx_id)
        self.assertEqual(copied_url["url"], source_url["url"])
        self.assertEqual(copied_url["copied_from_attachment_id"], source_url["id"])

        attachments_root = resolve_attachments_root(self.data_root)
        copied_file_path = attachments_root / copied_file["stored_relpath"]
        source_file_path = attachments_root / source_file["stored_relpath"]
        self.assertTrue(copied_file_path.exists())
        self.assertTrue(source_file_path.exists())

        core_attachments.remove_attachment(
            self.conn,
            str(self.data_root),
            None,
            None,
            source_file["id"],
            self.attachment_hooks,
        )
        self.assertTrue(copied_file_path.exists())
        self.assertFalse(source_file_path.exists())
        stored_copy = self.conn.execute(
            "SELECT * FROM attachments WHERE id = ?",
            (copied_file["id"],),
        ).fetchone()
        self.assertEqual(stored_copy["copied_from_attachment_id"], source_file["id"])
        self.assertEqual(stored_copy["copied_from_transaction_id"], source_tx_id)

    def test_audit_package_manifest_includes_copied_evidence_provenance(self):
        self._mark_journals_current()
        source_tx_id = self._insert_source_transaction()
        source_file = self._add_file_attachment(
            name="board-approval.pdf",
            content=b"board approval\n",
            tx_id=source_tx_id,
        )
        source_url = self._add_url_attachment(
            "https://docs.example.test/board/approval",
            tx_id=source_tx_id,
        )
        copied = core_attachments.copy_attachments(
            self.conn,
            str(self.data_root),
            None,
            None,
            self.tx_id,
            [source_file["id"], source_url["id"]],
            self.attachment_hooks,
            source_tx_ref=source_tx_id,
        )
        copied_file = next(
            attachment
            for attachment in copied["attachments"]
            if attachment["attachment_type"] == "file"
        )
        copied_url = next(
            attachment
            for attachment in copied["attachments"]
            if attachment["attachment_type"] == "url"
        )
        output_dir = self.root / "exports" / "audit-copied"

        result = audit_package.export_audit_package(
            self.conn,
            str(self.data_root),
            None,
            None,
            output_dir,
            self.audit_hooks,
            transaction_refs=[self.tx_id],
        )

        manifest = json.loads(Path(result["manifest"]).read_text(encoding="utf-8"))
        evidence_file = manifest["package"]["evidence_files"][0]
        self.assertEqual(evidence_file["attachment_id"], copied_file["id"])
        self.assertEqual(evidence_file["copied_from_attachment_id"], source_file["id"])
        self.assertEqual(evidence_file["copied_from_transaction_id"], source_tx_id)
        reference = manifest["package"]["url_references"][0]
        self.assertEqual(reference["attachment_id"], copied_url["id"])
        self.assertEqual(reference["copied_from_attachment_id"], source_url["id"])
        tx_manifest = manifest["transactions"][0]
        copied_attachment_ids = {
            attachment["copied_from_attachment_id"]
            for attachment in tx_manifest["direct_attachments"]
        }
        self.assertIn(source_file["id"], copied_attachment_ids)
        self.assertIn(source_url["id"], copied_attachment_ids)

    def test_audit_package_excludes_suggested_source_link_evidence(self):
        self._mark_journals_current()
        self.conn.execute(
            "UPDATE transactions SET pricing_source_kind = 'manual_override' WHERE id = ?",
            (self.tx_id,),
        )
        self.conn.commit()
        source_tx_id = self._insert_source_transaction()
        source_file = self._add_file_attachment(
            name="unreviewed-source.pdf",
            content=b"unreviewed source\n",
            tx_id=source_tx_id,
        )
        source_url = self._add_url_attachment(
            "https://docs.example.test/unreviewed-source",
            tx_id=source_tx_id,
        )
        self._add_source_link(
            attachment_ids=[source_file["id"], source_url["id"]],
            state="suggested",
        )
        output_dir = self.root / "exports" / "audit-suggested"

        result = audit_package.export_audit_package(
            self.conn,
            str(self.data_root),
            None,
            None,
            output_dir,
            self.audit_hooks,
            transaction_refs=[self.tx_id],
        )

        manifest = json.loads(Path(result["manifest"]).read_text(encoding="utf-8"))
        self.assertEqual(manifest["package"]["evidence_files"], [])
        self.assertEqual(manifest["package"]["url_references"], [])
        link_manifest = manifest["transactions"][0]["source_funds_links"][0]
        self.assertEqual(link_manifest["state"], "suggested")
        self.assertTrue(link_manifest["review_details_redacted"])
        self.assertEqual(link_manifest["attachments"], [])
        self.assertIsNone(link_manifest["from_source"])
        self.assertIsNone(link_manifest["from_transaction"])
        self.assertIsNone(link_manifest["allocation_amount"])
        self.assertEqual(link_manifest["explanation"], "")
        manifest_text = json.dumps(manifest, sort_keys=True)
        self.assertNotIn(source_file["id"], manifest_text)
        self.assertNotIn(source_url["id"], manifest_text)
        self.assertNotIn("unreviewed-source.pdf", manifest_text)
        self.assertNotIn("docs.example.test/unreviewed-source", manifest_text)
        warning_codes = {
            warning["code"]
            for warning in manifest["transactions"][0]["readiness"]["warnings"]
        }
        self.assertIn("source_link_unreviewed", warning_codes)

    def test_audit_package_redacts_secret_bearing_url_reference(self):
        self._mark_journals_current()
        self.conn.execute(
            """
            UPDATE transactions
            SET pricing_source_kind = 'manual_override',
                pricing_quality = 'exact'
            WHERE id = ?
            """,
            (self.tx_id,),
        )
        self.conn.commit()
        secret_url = self._add_url_attachment(
            "https://docs.example.test/receipt?access_token=secret#refresh_token=fragment-secret"
        )
        self._add_reviewed_source_link(attachment_ids=[secret_url["id"]])
        output_dir = self.root / "exports" / "audit-secret-url"

        result = audit_package.export_audit_package(
            self.conn,
            str(self.data_root),
            None,
            None,
            output_dir,
            self.audit_hooks,
            transaction_refs=[self.tx_id],
        )

        manifest = json.loads(Path(result["manifest"]).read_text(encoding="utf-8"))
        reference = manifest["package"]["url_references"][0]
        self.assertEqual(reference["url"], "")
        self.assertIn("access_token=REDACTED", reference["redacted_url"])
        self.assertIn("refresh_token=REDACTED", reference["redacted_url"])
        self.assertNotIn("fragment-secret", reference["redacted_url"])
        warning_codes = {warning["code"] for warning in manifest["package"]["warnings"]}
        self.assertIn("secret_bearing_url_redacted", warning_codes)

    def test_audit_package_review_state_exclusion_does_not_create_false_source_blocker(self):
        self._mark_journals_current()
        self.conn.execute(
            "UPDATE transactions SET pricing_source_kind = 'manual_override' WHERE id = ?",
            (self.tx_id,),
        )
        self.conn.commit()
        direct = self._add_file_attachment("board-decision.pdf", b"decision")
        self._add_reviewed_source_link(attachment_ids=[direct["id"]])

        summary = audit_package.build_evidence_summary(
            self.conn,
            str(self.data_root),
            None,
            None,
            self.audit_hooks,
            transaction_refs=[self.tx_id],
            include_review_state=False,
        )

        warning_codes = {
            warning["code"]
            for warning in summary["transactions"][0]["readiness"]["warnings"]
        }
        self.assertIn("review_state_excluded", warning_codes)
        self.assertNotIn("source_link_missing", warning_codes)
        self.assertNotIn("source_link_unreviewed", warning_codes)

    def test_audit_package_journal_state_exclusion_does_not_create_false_journal_blocker(self):
        self.conn.execute(
            "UPDATE transactions SET pricing_source_kind = 'manual_override' WHERE id = ?",
            (self.tx_id,),
        )
        self.conn.commit()
        direct = self._add_file_attachment("board-decision.pdf", b"decision")
        self._add_reviewed_source_link(attachment_ids=[direct["id"]])

        summary = audit_package.build_evidence_summary(
            self.conn,
            str(self.data_root),
            None,
            None,
            self.audit_hooks,
            transaction_refs=[self.tx_id],
            include_journal_state=False,
        )

        tx_summary = summary["transactions"][0]
        warning_codes = {
            warning["code"]
            for warning in tx_summary["readiness"]["warnings"]
        }
        self.assertEqual(summary["journal_freshness"]["status"], "not_processed")
        self.assertEqual(tx_summary["readiness"]["status"], "ready")
        self.assertIn("journal_state_excluded", warning_codes)
        self.assertNotIn("journal_stale", warning_codes)
        self.assertNotIn("journal_quarantined", warning_codes)
        self.assertNotIn("source_link_missing", warning_codes)

    def test_audit_package_export_omits_journal_payload_when_journal_state_excluded(self):
        self.conn.execute(
            "UPDATE transactions SET pricing_source_kind = 'manual_override' WHERE id = ?",
            (self.tx_id,),
        )
        self.conn.commit()
        direct = self._add_file_attachment("board-decision.pdf", b"decision")
        self._add_reviewed_source_link(attachment_ids=[direct["id"]])
        output_dir = self.root / "exports" / "audit-no-journal"

        result = audit_package.export_audit_package(
            self.conn,
            str(self.data_root),
            None,
            None,
            output_dir,
            self.audit_hooks,
            transaction_refs=[self.tx_id],
            include_journal_state=False,
        )

        manifest = json.loads(Path(result["manifest"]).read_text(encoding="utf-8"))
        tx_manifest = manifest["transactions"][0]
        warning_codes = {
            warning["code"]
            for warning in tx_manifest["readiness"]["warnings"]
        }
        self.assertEqual(manifest["journal_freshness"]["status"], "not_processed")
        self.assertNotIn("journal", tx_manifest)
        self.assertEqual(tx_manifest["readiness"]["status"], "ready")
        self.assertIn("journal_state_excluded", warning_codes)
        self.assertNotIn("journal_stale", warning_codes)

    def test_audit_package_can_exclude_copied_files_and_url_references(self):
        self._mark_journals_current()
        self.conn.execute(
            "UPDATE transactions SET pricing_source_kind = 'manual_override' WHERE id = ?",
            (self.tx_id,),
        )
        self.conn.commit()
        file_attachment = self._add_file_attachment("receipt.txt", b"receipt")
        url_attachment = self._add_url_attachment()
        self._add_reviewed_source_link(
            attachment_ids=[url_attachment["id"], file_attachment["id"]]
        )
        output_dir = self.root / "exports" / "audit-excluded"

        result = audit_package.export_audit_package(
            self.conn,
            str(self.data_root),
            None,
            None,
            output_dir,
            self.audit_hooks,
            transaction_refs=[self.tx_id],
            include_copied_attachments=False,
            include_url_references=False,
        )

        manifest = json.loads(Path(result["manifest"]).read_text(encoding="utf-8"))
        self.assertEqual(manifest["package"]["evidence_files"], [])
        self.assertFalse((output_dir / "evidence").exists())
        self.assertEqual(manifest["package"]["url_references"], [])
        manifest_text = json.dumps(manifest, sort_keys=True)
        self.assertNotIn("docs.example.test", manifest_text)
        self.assertNotIn("Board decision link", manifest_text)
        self.assertNotIn(url_attachment["id"], manifest_text)
        warning_codes = {warning["code"] for warning in manifest["package"]["warnings"]}
        self.assertIn("copied_attachments_excluded", warning_codes)
        self.assertIn("url_references_excluded", warning_codes)
        attachments_root = resolve_attachments_root(self.data_root)
        self.assertTrue((attachments_root / file_attachment["stored_relpath"]).exists())


if __name__ == "__main__":
    unittest.main()
