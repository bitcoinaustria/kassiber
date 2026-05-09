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

    def _insert_case_with_recipient(self, recipient_id: str, *, snapshot: dict[str, str]) -> str:
        """Insert a stub case row referencing the recipient.

        Includes the snapshot columns so list_cases-style consumers
        return preserved historical attribution.
        """
        case_id = str(uuid.uuid4())
        self.conn.execute(
            "INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING",
            ("acct-x", self.workspace_id, self.profile_id, "x", "x", "personal", "2026-04-01T00:00:00Z"),
        )
        self.conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, created_at)"
            " VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO NOTHING",
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
                f"ext-{tx_id[:8]}",
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
                created_at, updated_at, recipient_id,
                recipient_label_snapshot, recipient_kind_snapshot, recipient_reveal_mode_snapshot)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                recipient_id,
                snapshot["label"],
                snapshot["kind"],
                snapshot["default_reveal_mode"],
            ),
        )
        self.conn.commit()
        return case_id

    def test_delete_is_soft_and_preserves_recipient_id_on_cases(self):
        recipient = source_funds_recipients.create_recipient(
            self.conn,
            self.workspace_id,
            self.profile_id,
            label="Bank Z",
            kind="bank",
        )
        case_id = self._insert_case_with_recipient(
            recipient["id"],
            snapshot={"label": recipient["label"], "kind": recipient["kind"], "default_reveal_mode": recipient["default_reveal_mode"]},
        )
        result = source_funds_recipients.delete_recipient(self.conn, self.profile_id, recipient["id"])
        self.assertFalse(result["active"])
        row = self.conn.execute(
            """
            SELECT recipient_id, recipient_label_snapshot, recipient_kind_snapshot,
                recipient_reveal_mode_snapshot
            FROM source_funds_cases WHERE id = ?
            """,
            (case_id,),
        ).fetchone()
        # Stable identifier survives so audit consumers can still answer
        # "was this case sent to recipient X?"
        self.assertEqual(row["recipient_id"], recipient["id"])
        # Snapshot fields are also preserved.
        self.assertEqual(row["recipient_label_snapshot"], "Bank Z")
        self.assertEqual(row["recipient_kind_snapshot"], "bank")
        self.assertEqual(row["recipient_reveal_mode_snapshot"], "standard")
        # The recipient row itself still exists (soft delete) but is inactive.
        self.assertFalse(
            self.conn.execute(
                "SELECT active FROM source_funds_recipients WHERE id = ?",
                (recipient["id"],),
            ).fetchone()["active"]
        )

    def test_default_list_hides_inactive_recipients(self):
        source_funds_recipients.create_recipient(
            self.conn, self.workspace_id, self.profile_id, label="Active", kind="bank",
        )
        retired = source_funds_recipients.create_recipient(
            self.conn, self.workspace_id, self.profile_id, label="Retired", kind="bank",
        )
        source_funds_recipients.delete_recipient(self.conn, self.profile_id, retired["id"])
        listing = source_funds_recipients.list_recipients(self.conn, self.profile_id)
        labels = {row["label"] for row in listing}
        self.assertEqual(labels, {"Active"})
        with_inactive = source_funds_recipients.list_recipients(
            self.conn, self.profile_id, include_inactive=True,
        )
        self.assertEqual({row["label"] for row in with_inactive}, {"Active", "Retired"})

    def test_can_recreate_recipient_after_soft_delete_with_same_label(self):
        original = source_funds_recipients.create_recipient(
            self.conn, self.workspace_id, self.profile_id,
            label="Bank Austria", kind="bank",
        )
        source_funds_recipients.delete_recipient(self.conn, self.profile_id, original["id"])
        replacement = source_funds_recipients.create_recipient(
            self.conn, self.workspace_id, self.profile_id,
            label="Bank Austria", kind="bank",
        )
        self.assertNotEqual(replacement["id"], original["id"])
        active_listing = source_funds_recipients.list_recipients(
            self.conn, self.profile_id,
        )
        self.assertEqual({row["id"] for row in active_listing}, {replacement["id"]})

    def test_resolve_recipient_by_label_skips_inactive(self):
        retired = source_funds_recipients.create_recipient(
            self.conn, self.workspace_id, self.profile_id,
            label="Retired Recipient", kind="bank",
        )
        source_funds_recipients.delete_recipient(self.conn, self.profile_id, retired["id"])
        with self.assertRaises(AppError) as cm:
            source_funds_recipients.resolve_recipient(
                self.conn, self.profile_id, "Retired Recipient",
            )
        self.assertEqual(cm.exception.code, "not_found")
        # by-id still resolves so retrospective lookups (e.g., from a case
        # row's recipient_id) keep working.
        by_id = source_funds_recipients.resolve_recipient(
            self.conn, self.profile_id, retired["id"],
        )
        self.assertEqual(by_id["id"], retired["id"])
        self.assertFalse(by_id["active"])

    def test_restore_recipient_reactivates_soft_deleted_row(self):
        recipient = source_funds_recipients.create_recipient(
            self.conn, self.workspace_id, self.profile_id,
            label="Bank Restore", kind="bank",
        )
        source_funds_recipients.delete_recipient(self.conn, self.profile_id, recipient["id"])
        restored = source_funds_recipients.restore_recipient(
            self.conn, self.profile_id, recipient["id"],
        )
        self.assertTrue(restored["active"])
        listing = source_funds_recipients.list_recipients(self.conn, self.profile_id)
        self.assertEqual({row["id"] for row in listing}, {recipient["id"]})

    def test_restore_recipient_blocks_when_label_already_active(self):
        retired = source_funds_recipients.create_recipient(
            self.conn, self.workspace_id, self.profile_id,
            label="Same Label", kind="bank",
        )
        source_funds_recipients.delete_recipient(self.conn, self.profile_id, retired["id"])
        source_funds_recipients.create_recipient(
            self.conn, self.workspace_id, self.profile_id,
            label="Same Label", kind="bank",
        )
        with self.assertRaises(AppError) as cm:
            source_funds_recipients.restore_recipient(
                self.conn, self.profile_id, retired["id"],
            )
        self.assertEqual(cm.exception.code, "validation")

    def test_rename_does_not_rewrite_historical_case_attribution(self):
        recipient = source_funds_recipients.create_recipient(
            self.conn,
            self.workspace_id,
            self.profile_id,
            label="Original Label",
            kind="bank",
            default_reveal_mode="minimal",
        )
        case_id = self._insert_case_with_recipient(
            recipient["id"],
            snapshot={
                "label": recipient["label"],
                "kind": recipient["kind"],
                "default_reveal_mode": recipient["default_reveal_mode"],
            },
        )
        source_funds_recipients.update_recipient(
            self.conn,
            self.profile_id,
            recipient["id"],
            label="Renamed Label",
            kind="exchange",
            default_reveal_mode="full",
        )
        row = self.conn.execute(
            """
            SELECT recipient_label_snapshot, recipient_kind_snapshot,
                recipient_reveal_mode_snapshot
            FROM source_funds_cases WHERE id = ?
            """,
            (case_id,),
        ).fetchone()
        # Historical case row keeps the recipient state at the time of save.
        self.assertEqual(row["recipient_label_snapshot"], "Original Label")
        self.assertEqual(row["recipient_kind_snapshot"], "bank")
        self.assertEqual(row["recipient_reveal_mode_snapshot"], "minimal")

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


class RecipientEnvelopeKindTest(unittest.TestCase):
    """Pin the machine-output envelope kind for nested recipient subcommands.

    Without source_funds_recipients_command in _KIND_SUBCOMMAND_ATTRS,
    every recipient subcommand collapsed to the same kind, breaking
    deterministic command identity for machine consumers.
    """

    def test_each_subcommand_gets_a_distinct_kind(self):
        import argparse

        from kassiber.envelope import derive_kind

        kinds = {
            sub: derive_kind(
                argparse.Namespace(
                    command="source-funds",
                    source_funds_command="recipients",
                    source_funds_recipients_command=sub,
                )
            )
            for sub in ("list", "create", "update", "delete")
        }
        self.assertEqual(kinds["list"], "source-funds.recipients.list")
        self.assertEqual(kinds["create"], "source-funds.recipients.create")
        self.assertEqual(kinds["update"], "source-funds.recipients.update")
        self.assertEqual(kinds["delete"], "source-funds.recipients.delete")
        # Distinct kinds is the contract.
        self.assertEqual(len(set(kinds.values())), 4)


if __name__ == "__main__":
    unittest.main()
