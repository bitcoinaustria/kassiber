from __future__ import annotations

import hashlib
import json
import tarfile
import tempfile
import time
import unittest
import uuid
from pathlib import Path

from kassiber.cli.handlers import process_journals
from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.sync_replication.bundle import build_bundle, parse_bundle
from kassiber.core.sync_replication.capture import authored_state_digest, capture_local_changes
from kassiber.core.sync_replication.conflicts import list_conflicts, resolve_conflict
from kassiber.core.sync_replication.identity import enable_sync
from kassiber.core.sync_replication.membership import (
    create_invitation,
    create_join_request,
    join_invitation,
    revoke_member,
)
from kassiber.core.sync_replication.merge import import_bundle
from kassiber.core.transaction_history import append_event
from kassiber.core.ui_snapshot import build_report_blockers_snapshot
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import sqlcipher_available
from kassiber.time_utils import now_iso


@unittest.skipUnless(sqlcipher_available(), "SQLCipher driver unavailable")
class SyncBundleReplayTests(unittest.TestCase):
    def setUp(self):
        self.temp_a = tempfile.TemporaryDirectory()
        self.temp_b = tempfile.TemporaryDirectory()
        self.root_a = Path(self.temp_a.name)
        self.root_b = Path(self.temp_b.name)
        self.attachments_a = self.root_a / "attachments-files"
        self.attachments_b = self.root_b / "attachments-files"
        self.attachments_a.mkdir()
        self.attachments_b.mkdir()
        self.owner = open_db(self.root_a, passphrase="owner-passphrase")
        self.workspace = create_workspace(self.owner, "Org")
        self.profile = create_profile(
            self.owner,
            self.workspace["id"],
            "Books",
            "EUR",
            "FIFO",
            "generic",
            365,
        )
        self.owner_status = enable_sync(
            self.owner,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            member_name="Owner",
            device_label="Owner Mac",
        )
        self.peer = open_db(self.root_b, passphrase="peer-passphrase")

    def tearDown(self):
        self.owner.close()
        self.peer.close()
        self.temp_a.cleanup()
        self.temp_b.cleanup()

    def _join_peer(self, role="editor"):
        request = create_join_request(
            self.peer,
            member_name="Peer",
            device_label="Peer Mac",
        )
        invitation = create_invitation(
            self.owner,
            profile_id=self.profile["id"],
            join_request=request,
            role=role,
        )
        joined = join_invitation(
            self.peer,
            request_id=request["request_id"],
            ciphertext=invitation,
        )
        return request, joined

    def _initial_sync(self, role="editor"):
        request, joined = self._join_peer(role)
        bundle = build_bundle(
            self.owner,
            profile_id=self.profile["id"],
            attachments_root=self.attachments_a,
        )
        result = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
            attachments_root=self.attachments_b,
        )
        self.assertGreater(result.applied_events, 0)
        return request, joined, bundle

    def _insert_wallet_transaction_attachment(self):
        account = self.owner.execute(
            "SELECT * FROM accounts WHERE profile_id = ? LIMIT 1",
            (self.profile["id"],),
        ).fetchone()
        wallet_id = str(uuid.uuid4())
        tx_id = str(uuid.uuid4())
        attachment_id = str(uuid.uuid4())
        timestamp = now_iso()
        self.owner.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            ) VALUES(?, ?, ?, ?, 'Watch', 'xpub', ?, ?)
            """,
            (
                wallet_id,
                self.workspace["id"],
                self.profile["id"],
                account["id"],
                json.dumps(
                    {
                        "chain": "bitcoin",
                        "network": "mainnet",
                        "xpub": "xpub-public-material",
                        "token": "backend-secret-must-not-sync",
                    }
                ),
                timestamp,
            ),
        )
        self.owner.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, direction, asset, amount, fee,
                raw_json, created_at
            ) VALUES(?, ?, ?, ?, 'tx-1', ?, '2026-01-01T00:00:00Z',
                     'inbound', 'BTC', 100000, 0, ?, ?)
            """,
            (
                tx_id,
                self.workspace["id"],
                self.profile["id"],
                wallet_id,
                "raw-fingerprint-must-not-sync",
                json.dumps({"secret": "raw-json-must-not-sync"}),
                timestamp,
            ),
        )
        content = b"auditor evidence\x00bitcoin"
        sha256 = hashlib.sha256(content).hexdigest()
        stored_relpath = "aa/evidence.pdf"
        path = self.attachments_a / stored_relpath
        path.parent.mkdir(parents=True)
        path.write_bytes(content)
        self.owner.execute(
            """
            INSERT INTO attachments(
                id, workspace_id, profile_id, transaction_id, attachment_type,
                label, original_filename, stored_relpath, media_type, size_bytes,
                sha256, created_at
            ) VALUES(?, ?, ?, ?, 'file', 'Evidence', 'evidence.pdf', ?,
                     'application/pdf', ?, ?, ?)
            """,
            (
                attachment_id,
                self.workspace["id"],
                self.profile["id"],
                tx_id,
                stored_relpath,
                len(content),
                sha256,
                timestamp,
            ),
        )
        return wallet_id, tx_id, attachment_id, content

    def test_sealed_allowlisted_bundle_round_trips_attachment_without_plaintext_leaks(self):
        self._join_peer("editor")
        _, tx_id, attachment_id, content = self._insert_wallet_transaction_attachment()
        bundle = build_bundle(
            self.owner,
            profile_id=self.profile["id"],
            attachments_root=self.attachments_a,
        )
        self.assertNotIn(b"raw-fingerprint-must-not-sync", bundle.ciphertext)
        self.assertNotIn(b"backend-secret-must-not-sync", bundle.ciphertext)
        self.assertNotIn(content, bundle.ciphertext)
        with self.assertRaises(tarfile.ReadError):
            tarfile.open(fileobj=__import__("io").BytesIO(bundle.ciphertext), mode="r:*")

        identity = self.peer.execute(
            "SELECT age_identity FROM sync_device_private_keys"
        ).fetchone()[0]
        parsed = parse_bundle(bundle.ciphertext, age_identity=identity)
        serialized = json.dumps(parsed.events, sort_keys=True)
        self.assertNotIn("raw-fingerprint-must-not-sync", serialized)
        self.assertNotIn("backend-secret-must-not-sync", serialized)
        self.assertNotIn("raw-json-must-not-sync", serialized)
        self.assertIn("fingerprint_hmac", serialized)
        self.assertIn("content_hmac", serialized)

        result = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
            attachments_root=self.attachments_b,
        )
        self.assertGreater(result.row_mutations, 0)
        remote_tx = self.peer.execute("SELECT * FROM transactions WHERE id = ?", (tx_id,)).fetchone()
        self.assertTrue(remote_tx["fingerprint"].startswith("sync:"))
        remote_attachment = self.peer.execute(
            "SELECT * FROM attachments WHERE id = ?",
            (attachment_id,),
        ).fetchone()
        remote_path = self.attachments_b / remote_attachment["stored_relpath"]
        self.assertEqual(remote_path.read_bytes(), content)
        self.assertEqual(hashlib.sha256(content).hexdigest(), remote_attachment["sha256"])

    def test_duplicate_reordered_bundles_are_idempotent_and_converge(self):
        self._initial_sync()
        self.owner.execute(
            "UPDATE profiles SET label = 'One' WHERE id = ?",
            (self.profile["id"],),
        )
        first = build_bundle(self.owner, profile_id=self.profile["id"])
        self.owner.execute(
            "UPDATE profiles SET label = 'Two' WHERE id = ?",
            (self.profile["id"],),
        )
        second = build_bundle(self.owner, profile_id=self.profile["id"])

        queued = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=second.ciphertext,
        )
        self.assertEqual(queued.pending_events, 1)
        applied = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=first.ciphertext,
        )
        self.assertEqual(applied.applied_events, 2)
        duplicate = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=second.ciphertext,
        )
        self.assertTrue(duplicate.already_ingested)
        self.assertEqual(duplicate.duplicate_events, 1)
        self.assertEqual(
            self.peer.execute("SELECT label FROM profiles WHERE id = ?", (self.profile["id"],)).fetchone()[0],
            "Two",
        )
        self.assertEqual(
            authored_state_digest(self.owner, profile_id=self.profile["id"]),
            authored_state_digest(self.peer, profile_id=self.profile["id"]),
        )

    def test_high_stakes_conflict_blocks_journals_and_resolution_converges(self):
        self._initial_sync()
        self.owner.execute(
            "UPDATE profiles SET fiat_currency = 'USD' WHERE id = ?",
            (self.profile["id"],),
        )
        self.peer.execute(
            "UPDATE profiles SET fiat_currency = 'CHF' WHERE id = ?",
            (self.profile["id"],),
        )
        owner_bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        peer_bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        import_bundle(self.owner, profile_id=self.profile["id"], ciphertext=peer_bundle.ciphertext)
        import_bundle(self.peer, profile_id=self.profile["id"], ciphertext=owner_bundle.ciphertext)

        owner_conflict = list_conflicts(self.owner, profile_id=self.profile["id"])[0]
        peer_conflict = list_conflicts(self.peer, profile_id=self.profile["id"])[0]
        self.assertEqual(owner_conflict["id"], peer_conflict["id"])
        self.assertEqual(
            {owner_conflict["first_value"], owner_conflict["second_value"]},
            {"USD", "CHF"},
        )
        with self.assertRaisesRegex(AppError, "blocked by unresolved sync conflicts"):
            process_journals(self.owner, self.workspace["id"], self.profile["id"])
        blockers = build_report_blockers_snapshot(self.owner)["blockers"]
        self.assertIn("sync_conflicts", {blocker["id"] for blocker in blockers})

        chosen = (
            owner_conflict["first_event_id"]
            if owner_conflict["first_value"] == "USD"
            else owner_conflict["second_event_id"]
        )
        resolve_conflict(
            self.owner,
            profile_id=self.profile["id"],
            conflict_id=owner_conflict["id"],
            source_event_id=chosen,
        )
        resolution = build_bundle(self.owner, profile_id=self.profile["id"])
        import_bundle(self.peer, profile_id=self.profile["id"], ciphertext=resolution.ciphertext)
        self.assertEqual(
            self.owner.execute("SELECT fiat_currency FROM profiles").fetchone()[0],
            self.peer.execute("SELECT fiat_currency FROM profiles").fetchone()[0],
        )
        self.assertEqual(
            self.peer.execute("SELECT status FROM sync_conflicts").fetchone()[0],
            "resolved",
        )

    def test_stale_peer_never_resurrects_deleted_row(self):
        self._initial_sync()
        account_id = self.owner.execute(
            "SELECT id FROM accounts WHERE profile_id = ? LIMIT 1",
            (self.profile["id"],),
        ).fetchone()[0]
        self.owner.execute("DELETE FROM accounts WHERE id = ?", (account_id,))
        self.peer.execute("UPDATE accounts SET label = 'Stale edit' WHERE id = ?", (account_id,))
        owner_bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        peer_bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        import_bundle(self.owner, profile_id=self.profile["id"], ciphertext=peer_bundle.ciphertext)
        import_bundle(self.peer, profile_id=self.profile["id"], ciphertext=owner_bundle.ciphertext)
        for conn in (self.owner, self.peer):
            self.assertEqual(
                conn.execute("SELECT COUNT(*) FROM accounts WHERE id = ?", (account_id,)).fetchone()[0],
                0,
            )

    def test_auditor_authored_event_is_rejected_with_visible_notice(self):
        _, joined, _ = self._initial_sync("auditor")
        self.peer.execute(
            "UPDATE sync_members SET role = 'editor' WHERE id = ?",
            (joined["member_id"],),
        )
        self.peer.execute(
            "UPDATE profiles SET label = 'Auditor edit' WHERE id = ?",
            (self.profile["id"],),
        )
        capture_local_changes(self.peer, profile_id=self.profile["id"])
        self.peer.execute(
            "UPDATE sync_members SET role = 'auditor' WHERE id = ?",
            (joined["member_id"],),
        )
        malicious = build_bundle(self.peer, profile_id=self.profile["id"])
        result = import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=malicious.ciphertext,
        )
        self.assertEqual(result.rejected_events, 1)
        self.assertEqual(self.owner.execute("SELECT label FROM profiles").fetchone()[0], "Books")
        notice = self.owner.execute("SELECT code FROM sync_notices").fetchone()[0]
        self.assertEqual(notice, "auditor_authored_event")

    def test_revoked_member_event_is_rejected(self):
        _, joined, _ = self._initial_sync("editor")
        revoke_member(
            self.owner,
            profile_id=self.profile["id"],
            member_id=joined["member_id"],
        )
        time.sleep(0.002)
        self.peer.execute(
            "UPDATE profiles SET label = 'Revoked edit' WHERE id = ?",
            (self.profile["id"],),
        )
        malicious = build_bundle(self.peer, profile_id=self.profile["id"])
        result = import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=malicious.ciphertext,
        )
        self.assertEqual(result.rejected_events, 1)
        self.assertEqual(self.owner.execute("SELECT label FROM profiles").fetchone()[0], "Books")
        self.assertEqual(
            self.owner.execute("SELECT code FROM sync_notices").fetchone()[0],
            "revoked_member_event",
        )

    def test_transaction_history_replays_after_transaction_anchor(self):
        self._join_peer("editor")
        _, tx_id, _, _ = self._insert_wallet_transaction_attachment()
        self.owner.execute("UPDATE transactions SET excluded = 1 WHERE id = ?", (tx_id,))
        tx = self.owner.execute("SELECT * FROM transactions WHERE id = ?", (tx_id,)).fetchone()
        history_id = append_event(
            self.owner,
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
        bundle = build_bundle(
            self.owner,
            profile_id=self.profile["id"],
            attachments_root=self.attachments_a,
        )
        import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
            attachments_root=self.attachments_b,
        )
        remote_history = self.peer.execute(
            "SELECT * FROM transaction_edit_events WHERE id = ?",
            (history_id,),
        ).fetchone()
        self.assertIsNotNone(remote_history)
        self.assertTrue(remote_history["sync_signature"])
        self.assertEqual(
            self.peer.execute(
                "SELECT after_value FROM transaction_edit_fields WHERE event_id = ?",
                (history_id,),
            ).fetchone()[0],
            "true",
        )

    def test_reviewed_btcpay_link_syncs_snapshot_without_fetched_provenance_fk(self):
        self._initial_sync("editor")
        _, tx_id, _, _ = self._insert_wallet_transaction_attachment()
        record_id = str(uuid.uuid4())
        link_id = str(uuid.uuid4())
        timestamp = now_iso()
        self.owner.execute(
            """
            INSERT INTO btcpay_provenance_records(
                id, workspace_id, profile_id, store_id, record_type,
                stable_key, raw_json, created_at, updated_at
            ) VALUES(?, ?, ?, 'store', 'payment', 'stable-payment', '{}', ?, ?)
            """,
            (record_id, self.workspace["id"], self.profile["id"], timestamp, timestamp),
        )
        snapshot = {
            "origin_label": "Invoice 42",
            "occurred_at": "2026-01-01T12:00:00Z",
            "fiat_currency": "EUR",
            "fiat_value_exact": "500.00",
        }
        self.owner.execute(
            """
            INSERT INTO commercial_links(
                id, workspace_id, profile_id, btcpay_record_id, transaction_id,
                link_type, state, confidence, method, reconciliation_state,
                reviewed_record_snapshot_json, reviewed_at, created_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, 'payment', 'reviewed', 'exact', 'manual',
                     'reconciled', ?, ?, ?, ?)
            """,
            (
                link_id,
                self.workspace["id"],
                self.profile["id"],
                record_id,
                tx_id,
                json.dumps(snapshot),
                timestamp,
                timestamp,
                timestamp,
            ),
        )
        bundle = build_bundle(
            self.owner,
            profile_id=self.profile["id"],
            attachments_root=self.attachments_a,
        )
        identity = self.peer.execute("SELECT age_identity FROM sync_device_private_keys").fetchone()[0]
        parsed = parse_bundle(bundle.ciphertext, age_identity=identity)
        self.assertNotIn(record_id, json.dumps(parsed.events))
        import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
            attachments_root=self.attachments_b,
        )
        self.assertEqual(
            self.peer.execute("SELECT COUNT(*) FROM btcpay_provenance_records").fetchone()[0],
            0,
        )
        remote = self.peer.execute(
            "SELECT * FROM commercial_links WHERE id = ?", (link_id,)
        ).fetchone()
        self.assertIsNone(remote["btcpay_record_id"])
        self.assertIsNotNone(remote["document_id"])
        document = self.peer.execute(
            "SELECT * FROM external_documents WHERE id = ?", (remote["document_id"],)
        ).fetchone()
        self.assertEqual(document["document_type"], "commercial_record_snapshot")
        self.assertEqual(document["fiat_value_exact"], "500.00")

    def test_ciphertext_tampering_is_detected_before_replay(self):
        self._join_peer("editor")
        bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        tampered = bytearray(bundle.ciphertext)
        tampered[len(tampered) // 2] ^= 0x01
        with self.assertRaises(AppError):
            import_bundle(
                self.peer,
                profile_id=self.profile["id"],
                ciphertext=bytes(tampered),
            )


if __name__ == "__main__":
    unittest.main()
