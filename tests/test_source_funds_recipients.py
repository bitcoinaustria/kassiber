import sqlite3
import subprocess
import sys
import tempfile
import unittest
import uuid
from pathlib import Path

from kassiber.errors import AppError
from kassiber.core import source_funds_recipients


ROOT = Path(__file__).resolve().parent.parent


class RecipientCrudTests(unittest.TestCase):
    """Direct-DB unit tests for the recipients CRUD module.

    The schema is loaded via kassiber.db.open_db() so the tests stay in
    sync with the production schema (including the post-CREATE
    ensure_column adjustments).
    """

    def setUp(self):
        from kassiber import db as kassiber_db

        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.db_path = Path(self.tmp.name) / "kassiber.sqlite3"
        self.conn = kassiber_db.open_db(self.db_path)
        self.workspace_id = "ws-1"
        self.profile_id = "prof-1"
        now = "2026-04-01T00:00:00Z"
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES (?, ?, ?)",
            (self.workspace_id, "ws", now),
        )
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?)",
            (self.profile_id, self.workspace_id, "Default", "EUR", "generic", now),
        )
        self.conn.commit()

    def test_create_returns_normalized_row(self):
        recipient = source_funds_recipients.create_recipient(
            self.conn,
            self.workspace_id,
            self.profile_id,
            label="Finanzamt Wien",
            kind="tax_authority",
            default_reveal_mode="standard",
            notes="annual filings",
        )
        self.assertEqual(recipient["label"], "Finanzamt Wien")
        self.assertEqual(recipient["kind"], "tax_authority")
        self.assertEqual(recipient["default_reveal_mode"], "standard")
        self.assertEqual(recipient["notes"], "annual filings")
        self.assertIn("id", recipient)
        self.assertIn("created_at", recipient)

    def test_unique_label_per_profile(self):
        source_funds_recipients.create_recipient(
            self.conn,
            self.workspace_id,
            self.profile_id,
            label="Finanzamt Wien",
            kind="tax_authority",
        )
        with self.assertRaises(AppError) as cm:
            source_funds_recipients.create_recipient(
                self.conn,
                self.workspace_id,
                self.profile_id,
                label="Finanzamt Wien",
                kind="tax_authority",
            )
        self.assertEqual(cm.exception.code, "validation")

    def test_invalid_kind_rejected(self):
        with self.assertRaises(AppError) as cm:
            source_funds_recipients.create_recipient(
                self.conn,
                self.workspace_id,
                self.profile_id,
                label="X",
                kind="alien_overlord",
            )
        self.assertEqual(cm.exception.code, "validation")

    def test_invalid_reveal_mode_rejected(self):
        with self.assertRaises(AppError):
            source_funds_recipients.create_recipient(
                self.conn,
                self.workspace_id,
                self.profile_id,
                label="X",
                kind="exchange",
                default_reveal_mode="paranoid",
            )

    def test_list_recipients_orders_by_label(self):
        for label in ["Beta Bank", "Alpha Exchange", "Gamma Lawyer"]:
            source_funds_recipients.create_recipient(
                self.conn,
                self.workspace_id,
                self.profile_id,
                label=label,
                kind="other",
            )
        rows = source_funds_recipients.list_recipients(self.conn, self.profile_id)
        self.assertEqual([row["label"] for row in rows], ["Alpha Exchange", "Beta Bank", "Gamma Lawyer"])

    def test_resolve_recipient_by_id_and_label(self):
        created = source_funds_recipients.create_recipient(
            self.conn,
            self.workspace_id,
            self.profile_id,
            label="My Bank",
            kind="bank",
        )
        by_id = source_funds_recipients.resolve_recipient(self.conn, self.profile_id, created["id"])
        by_label = source_funds_recipients.resolve_recipient(self.conn, self.profile_id, "My Bank")
        self.assertEqual(by_id["id"], created["id"])
        self.assertEqual(by_label["id"], created["id"])

    def test_resolve_unknown_raises_not_found(self):
        with self.assertRaises(AppError) as cm:
            source_funds_recipients.resolve_recipient(self.conn, self.profile_id, "ghost")
        self.assertEqual(cm.exception.code, "not_found")

    def test_update_changes_fields(self):
        created = source_funds_recipients.create_recipient(
            self.conn,
            self.workspace_id,
            self.profile_id,
            label="Bank A",
            kind="bank",
        )
        updated = source_funds_recipients.update_recipient(
            self.conn,
            self.profile_id,
            created["id"],
            label="Bank A (renamed)",
            default_reveal_mode="minimal",
            notes="treats us nicely",
        )
        self.assertEqual(updated["label"], "Bank A (renamed)")
        self.assertEqual(updated["default_reveal_mode"], "minimal")
        self.assertEqual(updated["notes"], "treats us nicely")
        self.assertEqual(updated["kind"], "bank")  # unchanged

    def test_delete_clears_recipient_id_on_existing_cases(self):
        recipient = source_funds_recipients.create_recipient(
            self.conn,
            self.workspace_id,
            self.profile_id,
            label="Bank Z",
            kind="bank",
        )
        # Insert a fake case row with this recipient_id, then assert it's
        # nulled on delete.
        case_id = str(uuid.uuid4())
        # Need a transactions row for the FK; insert a stub.
        self.conn.execute(
            "INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?, ?)",
            ("acct-x", self.workspace_id, self.profile_id, "x", "x", "personal", "2026-04-01T00:00:00Z"),
        )
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?, ?)",
            ("wal-x", self.workspace_id, self.profile_id, "acct-x", "Wallet X", "personal", "2026-04-01T00:00:00Z"),
        )
        tx_id = str(uuid.uuid4())
        self.conn.execute(
            """
            INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, raw_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                self.workspace_id,
                self.profile_id,
                "wal-x",
                "ext-x",
                f"fp-{tx_id}",
                "2026-04-01T09:00:00Z",
                "inbound",
                "BTC",
                100_000,
                "{}",
                "2026-04-01T09:00:00Z",
            ),
        )
        self.conn.execute(
            """
            INSERT INTO source_funds_cases(id, workspace_id, profile_id, target_transaction_id,
                target_amount, asset, label, reveal_mode, status, snapshot_hash, snapshot_json,
                created_at, updated_at, recipient_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                case_id,
                self.workspace_id,
                self.profile_id,
                tx_id,
                100_000,
                "BTC",
                None,
                "standard",
                "exportable",
                "deadbeef",
                "{}",
                "2026-04-01T09:00:00Z",
                "2026-04-01T09:00:00Z",
                recipient["id"],
            ),
        )
        self.conn.commit()
        source_funds_recipients.delete_recipient(self.conn, self.profile_id, recipient["id"])
        row = self.conn.execute(
            "SELECT recipient_id FROM source_funds_cases WHERE id = ?",
            (case_id,),
        ).fetchone()
        self.assertIsNone(row["recipient_id"])

    def test_effective_reveal_mode_explicit_wins(self):
        created = source_funds_recipients.create_recipient(
            self.conn,
            self.workspace_id,
            self.profile_id,
            label="Bank E",
            kind="bank",
            default_reveal_mode="minimal",
        )
        mode, recipient = source_funds_recipients.effective_reveal_mode(
            self.conn,
            self.profile_id,
            explicit_reveal_mode="full",
            recipient_ref=created["id"],
        )
        self.assertEqual(mode, "full")
        self.assertIsNotNone(recipient)
        self.assertEqual(recipient["id"], created["id"])

    def test_effective_reveal_mode_recipient_default_applies(self):
        created = source_funds_recipients.create_recipient(
            self.conn,
            self.workspace_id,
            self.profile_id,
            label="Auditor",
            kind="lawyer",
            default_reveal_mode="full",
        )
        mode, recipient = source_funds_recipients.effective_reveal_mode(
            self.conn,
            self.profile_id,
            explicit_reveal_mode=None,
            recipient_ref=created["id"],
        )
        self.assertEqual(mode, "full")
        self.assertEqual(recipient["id"], created["id"])

    def test_effective_reveal_mode_no_recipient_falls_back_to_standard(self):
        mode, recipient = source_funds_recipients.effective_reveal_mode(
            self.conn,
            self.profile_id,
            explicit_reveal_mode=None,
            recipient_ref=None,
        )
        self.assertEqual(mode, "standard")
        self.assertIsNone(recipient)


class RecipientCliSmokeTest(unittest.TestCase):
    def test_help_works(self):
        result = subprocess.run(
            [sys.executable, "-m", "kassiber", "source-funds", "recipients", "create", "--help"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--label", result.stdout)
        self.assertIn("--kind", result.stdout)
        self.assertIn("--default-reveal-mode", result.stdout)


if __name__ == "__main__":
    unittest.main()
