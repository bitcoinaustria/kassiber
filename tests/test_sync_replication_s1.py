from __future__ import annotations

import json
import tempfile
import unittest
import uuid
from pathlib import Path

from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.sync_replication.capture import (
    capture_local_changes,
    preferred_wire_id,
)
from kassiber.core.sync_replication.clock import HybridLogicalClock, observe_clock, tick_clock
from kassiber.core.sync_replication.events import author_event, verify_event
from kassiber.core.sync_replication.identity import enable_sync
from kassiber.core.sync_replication.merge import (
    _apply_row_delete,
    _causal_dependencies_satisfied,
    _prepare_actual_row,
)
from kassiber.core.sync_replication.schema_allowlist import (
    NEVER_SYNC_TABLES,
    SYNC_TABLE_MAP,
    public_wallet_config,
    serialize_row,
    validate_wire_row,
)
from kassiber.core.transaction_history import append_event
from kassiber.daemon_sync_replication import dispatch_sync_ui
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import sqlcipher_available
from kassiber.time_utils import now_iso


class HybridLogicalClockTests(unittest.TestCase):
    def test_clock_is_monotonic_under_wall_clock_regression(self):
        first = tick_clock(None, "replica-a", now_ms=1000)
        second = tick_clock(first, "replica-a", now_ms=900)
        self.assertEqual(second, HybridLogicalClock(1000, 1, "replica-a"))
        self.assertGreater(second.encode(), first.encode())

    def test_observe_advances_past_remote(self):
        local = HybridLogicalClock(1000, 2, "replica-a")
        remote = HybridLogicalClock(1000, 7, "replica-b")
        observed = observe_clock(local, remote, "replica-a", now_ms=800)
        self.assertEqual(observed, HybridLogicalClock(1000, 8, "replica-a"))

    def test_observe_rejects_unbounded_remote_clock_drift(self):
        with self.assertRaisesRegex(ValueError, "future-drift"):
            observe_clock(
                None,
                HybridLogicalClock(253402300799000, 0, "replica-b"),
                "replica-a",
                now_ms=1000,
            )


class SyncSchemaBoundaryTests(unittest.TestCase):
    def test_allowlist_excludes_secrets_and_derived_state(self):
        self.assertFalse(set(SYNC_TABLE_MAP) & NEVER_SYNC_TABLES)
        for forbidden in (
            "backends",
            "ai_providers",
            "journal_entries",
            "wallet_utxos",
            "chain_observer_instances",
            "chain_observer_coverage",
            "chain_observer_values",
            "rates_cache",
            "sync_member_private_keys",
            "sync_device_private_keys",
        ):
            self.assertNotIn(forbidden, SYNC_TABLE_MAP)

    def test_wallet_config_is_positive_allowlist_and_rejects_private_material(self):
        safe = public_wallet_config(
            {
                "chain": "bitcoin",
                "network": "mainnet",
                "xpub": "xpub-public-watch-material",
                "descriptor": "wpkh(xprv-private-material/0/*)",
                "token": "backend-secret",
                "source_file": "/private/wallet.json",
                "blinding_key": "private-blinding-key",
            }
        )
        self.assertEqual(safe["chain"], "bitcoin")
        self.assertEqual(safe["xpub"], "xpub-public-watch-material")
        self.assertNotIn("descriptor", safe)
        self.assertNotIn("token", safe)
        self.assertNotIn("source_file", safe)
        self.assertNotIn("blinding_key", safe)

        for private_descriptor in (f"wpkh({'L' + '1' * 51})", f"wpkh({'a' * 64})"):
            self.assertNotIn(
                "descriptor",
                public_wallet_config({"descriptor": private_descriptor}),
            )

    def test_wire_upsert_requires_every_allowlisted_column(self):
        with self.assertRaisesRegex(AppError, "sync schema allowlist"):
            validate_wire_row("profiles", {"id": "profile-only"})


@unittest.skipUnless(sqlcipher_available(), "SQLCipher driver unavailable")
class SyncIdentityAndCaptureTests(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory()
        self.root = Path(self.tempdir.name)
        self.conn = open_db(self.root, passphrase="sync-test-passphrase")
        self.workspace = create_workspace(self.conn, "Sync Test")
        self.profile = create_profile(
            self.conn,
            self.workspace["id"],
            "Book",
            "EUR",
            "FIFO",
            "generic",
            365,
        )

    def tearDown(self):
        self.conn.close()
        self.tempdir.cleanup()

    def _enable(self):
        return enable_sync(
            self.conn,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            member_name="Owner",
            device_label="Test Mac",
        )

    def _insert_wallet_and_transaction(self):
        account = self.conn.execute(
            "SELECT * FROM accounts WHERE profile_id = ? ORDER BY created_at, id LIMIT 1",
            (self.profile["id"],),
        ).fetchone()
        wallet_id = str(uuid.uuid4())
        tx_id = str(uuid.uuid4())
        created_at = now_iso()
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                wallet_id,
                self.workspace["id"],
                self.profile["id"],
                account["id"],
                "Watch Wallet",
                "xpub",
                json.dumps(
                    {
                        "chain": "bitcoin",
                        "network": "mainnet",
                        "xpub": "xpub-public-watch-material",
                        "token": "backend-secret-must-not-sync",
                        "source_file": "/private/raw-wallet.json",
                    }
                ),
                created_at,
            ),
        )
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, direction, asset, amount, fee,
                raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, 'inbound', 'BTC', 100000, 0, ?, ?)
            """,
            (
                tx_id,
                self.workspace["id"],
                self.profile["id"],
                wallet_id,
                "tx-1",
                "raw-fingerprint-must-not-sync",
                "2026-01-01T00:00:00Z",
                json.dumps({"secret": "raw-json-must-not-sync"}),
                created_at,
            ),
        )
        attachment_id = str(uuid.uuid4())
        self.conn.execute(
            """
            INSERT INTO attachments(
                id, workspace_id, profile_id, transaction_id, attachment_type,
                label, stored_relpath, sha256, created_at
            ) VALUES(?, ?, ?, ?, 'file', 'Evidence', 'aa/evidence.pdf', ?, ?)
            """,
            (
                attachment_id,
                self.workspace["id"],
                self.profile["id"],
                tx_id,
                "raw-attachment-sha256-must-not-sync",
                created_at,
            ),
        )
        return wallet_id, tx_id, attachment_id

    def test_sync_disabled_book_has_no_identity_or_keys(self):
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM sync_books").fetchone()[0], 0)
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM sync_members").fetchone()[0], 0)
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM sync_member_private_keys").fetchone()[0],
            0,
        )

    def test_desktop_enable_returns_status_with_local_device(self):
        result = dispatch_sync_ui(
            self.conn,
            data_root=self.root,
            kind="ui.sync.enable",
            args={"member_name": "Owner", "device_label": "Test Mac"},
        )

        self.assertTrue(result["configured"])
        self.assertTrue(result["enabled"])
        self.assertEqual(result["members_list"][0]["display_name"], "Owner")
        self.assertEqual(result["devices_list"][0]["label"], "Test Mac")
        self.assertEqual(result["devices_list"][0]["local_device"], 1)

    def test_signed_event_rejects_malleable_hlc_and_sequence_encodings(self):
        self._enable()
        self.conn.execute(
            "UPDATE profiles SET label = 'Canonical event' WHERE id = ?",
            (self.profile["id"],),
        )
        event = capture_local_changes(self.conn, profile_id=self.profile["id"])[0]
        wire = event.to_wire_dict()
        public_key = self.conn.execute(
            "SELECT signing_public_key_b64 FROM sync_members WHERE id = ?",
            (event.author_member_id,),
        ).fetchone()[0]
        self.assertTrue(verify_event(wire, public_key))

        malleable_hlc = dict(wire)
        physical, logical, replica = wire["hlc"].split(":", 2)
        malleable_hlc["hlc"] = f"+{physical}:{logical}:{replica}"
        self.assertFalse(verify_event(malleable_hlc, public_key))

        string_sequence = dict(wire)
        string_sequence["replica_seq"] = f"+00{wire['replica_seq']}"
        self.assertFalse(verify_event(string_sequence, public_key))

    def test_plaintext_database_cannot_hold_sync_keys(self):
        other_root = self.root / "plaintext"
        plaintext = open_db(other_root)
        try:
            workspace = create_workspace(plaintext, "Plain")
            profile = create_profile(
                plaintext,
                workspace["id"],
                "Book",
                "EUR",
                "FIFO",
                "generic",
                365,
            )
            with self.assertRaisesRegex(AppError, "SQLCipher"):
                enable_sync(
                    plaintext,
                    workspace_id=workspace["id"],
                    profile_id=profile["id"],
                    member_name="Owner",
                    device_label="Plain Device",
                )
            self.assertEqual(plaintext.execute("SELECT COUNT(*) FROM sync_books").fetchone()[0], 0)
        finally:
            plaintext.close()

    def test_root_events_are_signed_and_hash_chained(self):
        status = self._enable()
        rows = self.conn.execute(
            "SELECT * FROM sync_events ORDER BY replica_seq"
        ).fetchall()
        self.assertEqual([row["replica_seq"] for row in rows], [1, 2])
        self.assertIsNone(rows[0]["previous_hash"])
        self.assertEqual(rows[1]["previous_hash"], rows[0]["event_hash"])
        member = self.conn.execute(
            "SELECT * FROM sync_members WHERE id = ?",
            (status["local_member_id"],),
        ).fetchone()
        for row in rows:
            event = dict(row)
            event["payload"] = json.loads(event.pop("payload_json"))
            event["context"] = json.loads(event.pop("context_json"))
            event.pop("applied_at")
            self.assertTrue(verify_event(event, member["signing_public_key_b64"]))
        tampered = dict(event)
        tampered["payload"] = {"tampered": True}
        self.assertFalse(verify_event(tampered, member["signing_public_key_b64"]))

    def test_transaction_edit_gets_replica_clock_sequence_and_signature(self):
        status = self._enable()
        _, tx_id, _ = self._insert_wallet_and_transaction()
        tx = self.conn.execute("SELECT * FROM transactions WHERE id = ?", (tx_id,)).fetchone()
        event_id = append_event(
            self.conn,
            workspace=self.workspace,
            profile=self.profile,
            tx=tx,
            source="gui",
            reason="reviewed",
            changed_at=now_iso(),
            changed_fields=["excluded"],
            before_state={"excluded": False},
            after_state={"excluded": True},
        )
        row = self.conn.execute(
            "SELECT * FROM transaction_edit_events WHERE id = ?",
            (event_id,),
        ).fetchone()
        self.assertEqual(row["sync_replica_id"], status["local_replica_id"])
        self.assertGreater(row["sync_replica_seq"], 2)
        self.assertTrue(row["sync_hlc"])
        self.assertTrue(row["sync_signature"])
        self.assertEqual(
            json.loads(row["sync_context_json"])[status["local_replica_id"]],
            row["sync_replica_seq"] - 1,
        )

    def test_capture_uses_hmac_ids_and_omits_secret_and_derived_columns(self):
        self._enable()
        self._insert_wallet_and_transaction()
        capture_local_changes(self.conn, profile_id=self.profile["id"])
        serialized = "\n".join(
            row["payload_json"]
            for row in self.conn.execute(
                "SELECT payload_json FROM sync_events ORDER BY replica_seq"
            ).fetchall()
        )
        self.assertNotIn("raw-fingerprint-must-not-sync", serialized)
        self.assertNotIn("raw-attachment-sha256-must-not-sync", serialized)
        self.assertNotIn("backend-secret-must-not-sync", serialized)
        self.assertNotIn("raw-json-must-not-sync", serialized)
        self.assertNotIn("/private/raw-wallet.json", serialized)
        self.assertIn("fingerprint_hmac", serialized)
        self.assertIn("content_hmac", serialized)
        self.assertIn("xpub-public-watch-material", serialized)

    def test_missing_authored_rows_become_signed_tombstones(self):
        self._enable()
        _, tx_id, attachment_id = self._insert_wallet_and_transaction()
        capture_local_changes(self.conn, profile_id=self.profile["id"])
        self.conn.execute("DELETE FROM transactions WHERE id = ?", (tx_id,))
        emitted = capture_local_changes(self.conn, profile_id=self.profile["id"])
        deleted = {(event.entity_table, event.entity_key) for event in emitted if event.event_type == "row.delete"}
        self.assertIn(("transactions", json.dumps([tx_id], separators=(",", ":"))), deleted)
        self.assertIn(("attachments", json.dumps([attachment_id], separators=(",", ":"))), deleted)
        tombstones = self.conn.execute(
            "SELECT entity_table, entity_key FROM sync_tombstones WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchall()
        self.assertEqual(
            {(row["entity_table"], row["entity_key"]) for row in tombstones},
            deleted,
        )

    def test_composite_tombstone_maps_each_referenced_primary_key(self):
        self._enable()
        _, tx_id, _ = self._insert_wallet_and_transaction()
        tag_id = str(uuid.uuid4())
        created_at = now_iso()
        self.conn.execute(
            "INSERT INTO tags(id, workspace_id, profile_id, code, label, created_at) "
            "VALUES(?, ?, ?, 'reviewed', 'Reviewed', ?)",
            (tag_id, self.workspace["id"], self.profile["id"], created_at),
        )
        self.conn.execute(
            "INSERT INTO transaction_tags(transaction_id, tag_id) VALUES(?, ?)",
            (tx_id, tag_id),
        )
        wire_tx_id = "wire-transaction"
        wire_tag_id = "wire-tag"
        self.conn.executemany(
            "INSERT INTO sync_id_map(profile_id, entity_table, wire_id, local_id, created_at) "
            "VALUES(?, ?, ?, ?, ?)",
            [
                (self.profile["id"], "transactions", wire_tx_id, tx_id, created_at),
                (self.profile["id"], "tags", wire_tag_id, tag_id, created_at),
            ],
        )
        key = json.dumps([wire_tx_id, wire_tag_id], separators=(",", ":"))
        authored = author_event(
            self.conn,
            profile_id=self.profile["id"],
            event_type="row.delete",
            entity_table="transaction_tags",
            entity_key=key,
            payload={"key": key, "reason": "reviewed-delete"},
        )
        self.assertIsNotNone(authored)
        book = self.conn.execute(
            "SELECT * FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()

        _apply_row_delete(self.conn, book=book, event=authored.to_wire_dict())

        self.assertIsNone(
            self.conn.execute(
                "SELECT 1 FROM transaction_tags WHERE transaction_id = ? AND tag_id = ?",
                (tx_id, tag_id),
            ).fetchone()
        )

    def test_wire_alias_preference_is_device_independent(self):
        self._enable()
        _, tx_id, _ = self._insert_wallet_and_transaction()
        self.conn.executemany(
            "INSERT INTO sync_id_map("
            "profile_id, entity_table, wire_id, local_id, created_at"
            ") VALUES(?, 'transactions', ?, ?, ?)",
            [
                (self.profile["id"], "z-wire", tx_id, "2025-01-01T00:00:00Z"),
                (self.profile["id"], "0-wire", tx_id, "2026-01-01T00:00:00Z"),
            ],
        )

        self.assertEqual(
            preferred_wire_id(
                self.conn,
                profile_id=self.profile["id"],
                table="transactions",
                local_id=tx_id,
            ),
            "0-wire",
        )

    def test_unknown_causal_replica_defers_instead_of_invalidating_event(self):
        self._enable()
        self.assertFalse(
            _causal_dependencies_satisfied(
                self.conn,
                profile_id=self.profile["id"],
                event={"context": {"replica-not-received-yet": 1}},
            )
        )

    def test_older_transaction_event_preserves_optional_refund_vout(self):
        self._enable()
        _, tx_id, _ = self._insert_wallet_and_transaction()
        self.conn.execute(
            "UPDATE transactions SET swap_refund_funding_txid = ?, "
            "swap_refund_funding_vout = 7 WHERE id = ?",
            ("ab" * 32, tx_id),
        )
        spec = SYNC_TABLE_MAP["transactions"]
        row = self.conn.execute(
            "SELECT * FROM transactions WHERE id = ?", (tx_id,)
        ).fetchone()
        book = self.conn.execute(
            "SELECT * FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()
        wire_row = serialize_row(
            spec,
            row,
            hmac_key_b64=book["hmac_key_b64"],
        )
        wire_row.pop("swap_refund_funding_vout")

        actual, _ = _prepare_actual_row(
            self.conn,
            book=book,
            spec=spec,
            wire_row=wire_row,
            blobs={},
            attachments_root=None,
            created_files=[],
        )

        self.assertEqual(actual["swap_refund_funding_vout"], 7)


if __name__ == "__main__":
    unittest.main()
