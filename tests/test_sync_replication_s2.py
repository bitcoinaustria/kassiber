from __future__ import annotations

import hashlib
from io import BytesIO
import json
import tarfile
import tempfile
import time
import unittest
import unittest.mock as mock
import uuid
from pathlib import Path

from kassiber.cli.handlers import process_journals
from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.sync_replication.bundle import _membership_catalog, build_bundle, parse_bundle
from kassiber.core.sync_replication.capture import (
    authored_state_digest,
    capture_full_snapshot,
    capture_local_changes,
)
from kassiber.core.sync_replication.conflicts import list_conflicts, resolve_conflict
from kassiber.core.sync_replication.identity import disable_sync, enable_sync
from kassiber.core.sync_replication.membership import (
    create_invitation,
    create_join_request,
    join_invitation,
    revoke_device,
    revoke_member,
)
from kassiber.core.sync_replication.merge import (
    _event_role_rejection,
    _merge_membership_catalog,
    import_bundle,
)
from kassiber.core.transaction_history import append_event
from kassiber.core.ui_snapshot import build_report_blockers_snapshot
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import sqlcipher_available
from kassiber.time_utils import now_iso


class BundleParserValidationTests(unittest.TestCase):
    @staticmethod
    def _plaintext_bundle(manifest) -> bytes:
        output = BytesIO()
        with tarfile.open(fileobj=output, mode="w") as archive:
            for name, payload in (
                ("manifest.json", json.dumps(manifest).encode("utf-8")),
                ("events.jsonl", b""),
            ):
                info = tarfile.TarInfo(name)
                info.size = len(payload)
                archive.addfile(info, BytesIO(payload))
        return output.getvalue()

    def _parse_manifest(self, manifest):
        plaintext = self._plaintext_bundle(manifest)

        def decrypt(_source, destination, **_kwargs):
            destination.write(plaintext)

        with mock.patch(
            "kassiber.core.sync_replication.bundle.decrypt_age_stream",
            side_effect=decrypt,
        ):
            return parse_bundle(b"age-ciphertext", age_identity="AGE-SECRET-KEY-test")

    def test_manifest_requires_object_and_typed_inventory_fields(self):
        for malformed in ([], None, True, "manifest"):
            with self.subTest(manifest=malformed), self.assertRaises(AppError) as raised:
                self._parse_manifest(malformed)
            self.assertEqual(raised.exception.code, "sync_bundle_invalid")

        baseline = {
            "schema_version": 1,
            "events_sha256": hashlib.sha256(b"").hexdigest(),
            "event_count": 0,
            "blob_hmacs": [],
        }
        for field, malformed in (
            ("schema_version", True),
            ("event_count", True),
            ("event_count", "0"),
            ("blob_hmacs", {}),
            ("blob_hmacs", [1]),
        ):
            with self.subTest(field=field, malformed=malformed):
                manifest = baseline | {field: malformed}
                with self.assertRaises(AppError) as raised:
                    self._parse_manifest(manifest)
                self.assertEqual(raised.exception.code, "sync_bundle_invalid")

        parsed = self._parse_manifest(baseline)
        self.assertEqual(parsed.events, ())


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
        disable_sync(self.owner, profile_id=self.profile["id"])
        resolve_conflict(
            self.owner,
            profile_id=self.profile["id"],
            conflict_id=owner_conflict["id"],
            source_event_id=chosen,
        )
        enable_sync(
            self.owner,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            member_name="",
            device_label="",
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
        with self.assertRaisesRegex(AppError, "sender is revoked"):
            import_bundle(
                self.owner,
                profile_id=self.profile["id"],
                ciphertext=malicious.ciphertext,
            )
        self.assertEqual(self.owner.execute("SELECT label FROM profiles").fetchone()[0], "Books")

    def test_revocation_fence_uses_causal_sequence_not_backdated_hlc(self):
        _, joined, _ = self._initial_sync("editor")
        revoke_member(
            self.owner,
            profile_id=self.profile["id"],
            member_id=joined["member_id"],
        )
        member = self.owner.execute(
            "SELECT * FROM sync_members WHERE id = ?",
            (joined["member_id"],),
        ).fetchone()
        replica = self.owner.execute(
            "SELECT * FROM sync_replicas WHERE member_id = ?",
            (joined["member_id"],),
        ).fetchone()
        fence = json.loads(member["revoked_context_json"])
        self.assertEqual(
            _event_role_rejection(
                self.owner,
                member,
                replica,
                {
                    "replica_id": replica["id"],
                    "replica_seq": int(fence.get(replica["id"], 0)) + 1,
                    "hlc": f"0000000000000000:0000000000:{replica['id']}",
                    "event_type": "row.upsert",
                },
            ),
            "revoked_member_event",
        )

    def test_revoked_device_bundle_is_rejected_without_revoking_member(self):
        _, joined, _ = self._initial_sync("editor")
        revoke_device(
            self.owner,
            profile_id=self.profile["id"],
            device_id=joined["device_id"],
        )
        member = self.owner.execute(
            "SELECT revoked_at FROM sync_members WHERE id = ?",
            (joined["member_id"],),
        ).fetchone()
        self.assertIsNone(member["revoked_at"])
        self.peer.execute(
            "UPDATE profiles SET label = 'Revoked device edit' WHERE id = ?",
            (self.profile["id"],),
        )
        malicious = build_bundle(self.peer, profile_id=self.profile["id"])
        with self.assertRaisesRegex(AppError, "sender is revoked"):
            import_bundle(
                self.owner,
                profile_id=self.profile["id"],
                ciphertext=malicious.ciphertext,
            )
        self.assertEqual(self.owner.execute("SELECT label FROM profiles").fetchone()[0], "Books")

    def test_revoking_one_device_does_not_revoke_members_other_device(self):
        _, joined, _ = self._initial_sync("editor")
        revoked_replica = self.owner.execute(
            "SELECT * FROM sync_replicas WHERE device_id = ?",
            (joined["device_id"],),
        ).fetchone()
        revoke_device(
            self.owner,
            profile_id=self.profile["id"],
            device_id=joined["device_id"],
        )
        member = self.owner.execute(
            "SELECT * FROM sync_members WHERE id = ?",
            (joined["member_id"],),
        ).fetchone()
        rejected = _event_role_rejection(
            self.owner,
            member,
            revoked_replica,
            {
                "replica_id": revoked_replica["id"],
                "replica_seq": int(revoked_replica["last_seq"]) + 1,
                "event_type": "row.upsert",
            },
        )
        self.assertEqual(rejected, "revoked_device_event")

        second_device_id = str(uuid.uuid4())
        second_replica_id = str(uuid.uuid4())
        original_device = self.owner.execute(
            "SELECT * FROM sync_devices WHERE id = ?",
            (joined["device_id"],),
        ).fetchone()
        self.owner.execute(
            """
            INSERT INTO sync_devices(
                id, workspace_id, profile_id, member_id, recipient_public_key,
                label, paired_hlc, paired_at, record_signer_member_id,
                record_signature
            ) VALUES(?, ?, ?, ?, ?, 'Second device', ?, ?, ?, ?)
            """,
            (
                second_device_id,
                original_device["workspace_id"],
                original_device["profile_id"],
                original_device["member_id"],
                f"age1second{uuid.uuid4().hex}",
                original_device["paired_hlc"],
                original_device["paired_at"],
                original_device["record_signer_member_id"],
                original_device["record_signature"],
            ),
        )
        self.owner.execute(
            """
            INSERT INTO sync_replicas(
                id, workspace_id, profile_id, member_id, device_id, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                second_replica_id,
                original_device["workspace_id"],
                original_device["profile_id"],
                original_device["member_id"],
                second_device_id,
                now_iso(),
            ),
        )
        second_replica = self.owner.execute(
            "SELECT * FROM sync_replicas WHERE id = ?",
            (second_replica_id,),
        ).fetchone()
        self.assertIsNone(
            _event_role_rejection(
                self.owner,
                member,
                second_replica,
                {
                    "replica_id": second_replica_id,
                    "replica_seq": 1,
                    "event_type": "row.upsert",
                },
            )
        )

    def test_cross_book_device_and_replica_catalog_injection_is_rejected(self):
        self._initial_sync("editor")
        catalog = _membership_catalog(self.peer, self.profile["id"])
        catalog["devices"][0]["profile_id"] = "attacker-profile"
        catalog["devices"][0]["workspace_id"] = "attacker-workspace"
        book = self.owner.execute(
            "SELECT * FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()
        with self.assertRaisesRegex(AppError, "device row targets another book"):
            _merge_membership_catalog(self.owner, book=book, catalog=catalog)

        catalog = _membership_catalog(self.peer, self.profile["id"])
        catalog["replicas"][0]["profile_id"] = "attacker-profile"
        with self.assertRaisesRegex(AppError, "replica row targets another book"):
            _merge_membership_catalog(self.owner, book=book, catalog=catalog)

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

    def test_full_snapshot_preserves_nullable_edit_history_values(self):
        self._join_peer("editor")
        _, tx_id, _, _ = self._insert_wallet_transaction_attachment()
        tx = self.owner.execute("SELECT * FROM transactions WHERE id = ?", (tx_id,)).fetchone()
        history_id = append_event(
            self.owner,
            workspace=self.workspace,
            profile=self.profile,
            tx=tx,
            source="gui",
            reason="nullable history",
            changed_at=now_iso(),
            changed_fields=["notes"],
            before_state={"notes": None},
            after_state={"notes": None},
        )
        self.owner.execute(
            """
            UPDATE transaction_edit_fields
            SET before_value = NULL, after_value = NULL
            WHERE event_id = ?
            """,
            (history_id,),
        )
        events = capture_full_snapshot(self.owner, profile_id=self.profile["id"])
        snapshot = next(
            event
            for event in events
            if event.event_type == "transaction.edit" and event.entity_key == history_id
        )
        field = snapshot.payload["fields"][0]
        self.assertIsNone(field["before_value"])
        self.assertIsNone(field["after_value"])
        self.assertEqual(field["diff"], {})

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
