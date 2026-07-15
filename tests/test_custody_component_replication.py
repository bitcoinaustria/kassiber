"""Replication invariants for authored custody-component revisions."""

from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
import uuid
from pathlib import Path
from types import SimpleNamespace

from kassiber.cli.handlers import build_ledger_state
from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.custody_components import (
    activate_component,
    create_component,
    get_component,
    list_components,
    reconcile_active_memberships,
    supersede_component,
    update_component,
)
from kassiber.core import custody_filed_reports
from kassiber.core.sync_replication.bundle import build_bundle
from kassiber.core.sync_replication.capture import preferred_wire_id
from kassiber.core.sync_replication.conflicts import resolve_conflict
from kassiber.core.sync_replication.events import author_event
from kassiber.core.sync_replication.identity import enable_sync
from kassiber.core.sync_replication.membership import (
    create_invitation,
    create_join_request,
    join_invitation,
)
from kassiber.core.sync_replication.merge import import_bundle
from kassiber.core.sync_replication.schema_allowlist import (
    SYNC_TABLE_MAP,
    row_key,
    serialize_row,
)
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import require_sqlcipher, sqlcipher_available


NOW = "2026-01-01T00:00:00Z"


@unittest.skipUnless(sqlcipher_available(), "SQLCipher driver unavailable")
class CustodyComponentReplicationTests(unittest.TestCase):
    def setUp(self):
        self.temp_owner = tempfile.TemporaryDirectory()
        self.temp_peer = tempfile.TemporaryDirectory()
        self.temp_third = tempfile.TemporaryDirectory()
        self.owner = open_db(Path(self.temp_owner.name), passphrase="owner-passphrase")
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
        enable_sync(
            self.owner,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            member_name="Owner",
            device_label="Owner device",
        )
        self.peer = open_db(Path(self.temp_peer.name), passphrase="peer-passphrase")
        self.third = open_db(Path(self.temp_third.name), passphrase="third-passphrase")

    def tearDown(self):
        self.owner.close()
        self.peer.close()
        self.third.close()
        self.temp_owner.cleanup()
        self.temp_peer.cleanup()
        self.temp_third.cleanup()

    def _join_peer(self) -> None:
        request = create_join_request(
            self.peer,
            member_name="Peer",
            device_label="Peer device",
        )
        invitation = create_invitation(
            self.owner,
            profile_id=self.profile["id"],
            join_request=request,
            role="editor",
        )
        join_invitation(
            self.peer,
            request_id=request["request_id"],
            ciphertext=invitation,
        )

    def _join_third(self) -> None:
        request = create_join_request(
            self.third,
            member_name="Third",
            device_label="Third device",
        )
        invitation = create_invitation(
            self.owner,
            profile_id=self.profile["id"],
            join_request=request,
            role="editor",
        )
        join_invitation(
            self.third,
            request_id=request["request_id"],
            ciphertext=invitation,
        )

    def _sync_owner_to_peer(self):
        bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)
        return import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )

    def _insert_wallet_and_transactions(self) -> tuple[str, str, str]:
        account_id = self.owner.execute(
            "SELECT id FROM accounts WHERE profile_id = ? ORDER BY id LIMIT 1",
            (self.profile["id"],),
        ).fetchone()[0]
        wallet_id = str(uuid.uuid4())
        out_id = str(uuid.uuid4())
        in_id = str(uuid.uuid4())
        self.owner.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            ) VALUES(?, ?, ?, ?, 'Watch', 'xpub',
                     '{"chain":"bitcoin","network":"regtest"}', ?)
            """,
            (
                wallet_id,
                self.workspace["id"],
                self.profile["id"],
                account_id,
                NOW,
            ),
        )
        for tx_id, direction in ((out_id, "outbound"), (in_id, "inbound")):
            txid = hashlib.sha256(tx_id.encode("ascii")).hexdigest()
            self.owner.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id,
                    fingerprint, occurred_at, direction, asset, amount, fee,
                    raw_json, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, 'BTC', 100000, 0, ?, ?)
                """,
                (
                    tx_id,
                    self.workspace["id"],
                    self.profile["id"],
                    wallet_id,
                    txid,
                    f"fingerprint-{tx_id}",
                    NOW,
                    direction,
                    json.dumps({"txid": txid}),
                    NOW,
                ),
            )
        return wallet_id, out_id, in_id

    def _create_component(
        self, *, active: bool, authored_source: str = "user"
    ) -> dict:
        wallet_id, out_id, in_id = self._insert_wallet_and_transactions()
        component = create_component(
            self.owner,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            component_type="native_transfer",
            evidence_kind="ownership_graph",
            evidence_grade="exact",
            legs=[
                {
                    "role": "source",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "regtest",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 100000,
                    "transaction_id": out_id,
                    "wallet_id": wallet_id,
                },
                {
                    "role": "destination",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "regtest",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 100000,
                    "transaction_id": in_id,
                    "wallet_id": wallet_id,
                },
            ],
            allocations=[
                {
                    "source_ordinal": 0,
                    "sink_ordinal": 1,
                    "source_amount_msat": 100000,
                    "sink_amount_msat": 100000,
                }
            ],
            created_at=NOW,
            authored_source=authored_source,
        )
        return activate_component(self.owner, component["id"], activated_at=NOW) if active else component

    def test_mutually_linked_headers_replay_without_row_order_dependency(self):
        self._join_peer()
        first = self._create_component(active=False)
        second = update_component(
            self.owner,
            first["id"],
            notes="replacement",
            created_at="2026-01-02T00:00:00Z",
        )

        result = self._sync_owner_to_peer()

        self.assertGreater(result.row_mutations, 0)
        remote_first = get_component(self.peer, first["id"])
        remote_second = get_component(self.peer, second["id"])
        self.assertEqual(second["id"], remote_first["superseded_by_component_id"])
        self.assertEqual(first["id"], remote_second["supersedes_component_id"])
        self.assertEqual("superseded", remote_first["state"])
        self.assertEqual("draft", remote_second["state"])

    def test_ai_component_attribution_replicates_with_revision(self):
        self._join_peer()
        component = self._create_component(
            active=False,
            authored_source="ai_tool",
        )

        result = self._sync_owner_to_peer()

        self.assertGreater(result.row_mutations, 0)
        remote = get_component(self.peer, component["id"])
        self.assertEqual(component["authored_source"], "ai_tool")
        self.assertEqual(remote["authored_source"], "ai_tool")

    def test_complete_replicated_component_gets_commitments_not_raw_evidence(self):
        self._join_peer()
        component = self._create_component(active=True)

        result = self._sync_owner_to_peer()

        self.assertGreater(result.row_mutations, 0)
        remote = get_component(self.peer, component["id"])
        self.assertEqual(remote["effective_state"], "active")
        replicated_transactions = self.peer.execute(
            "SELECT external_id_kind, raw_json FROM transactions "
            "WHERE id IN (?, ?) ORDER BY id",
            tuple(leg["transaction_id"] for leg in component["legs"]),
        ).fetchall()
        self.assertEqual(2, len(replicated_transactions))
        self.assertTrue(
            all(row["external_id_kind"] == "txid" for row in replicated_transactions)
        )
        self.assertTrue(
            all(row["raw_json"] == "{}" for row in replicated_transactions)
        )
        peer_snapshots = self.peer.execute(
            """
            SELECT quantity_hash, detail_hash, payload_json
            FROM custody_authored_evidence_snapshots
            WHERE profile_id = ? AND subject_kind = 'custody_component'
              AND subject_id = ?
            ORDER BY quantity_hash, detail_hash
            """,
            (self.profile["id"], component["id"]),
        ).fetchall()
        self.assertEqual(len(peer_snapshots), 0)
        activate_component(self.peer, component["id"], activated_at=NOW)
        self.assertEqual(
            0,
            self.peer.execute(
                "SELECT COUNT(*) FROM custody_authored_evidence_snapshots "
                "WHERE subject_id = ?",
                (component["id"],),
            ).fetchone()[0],
        )

    def test_filed_snapshot_and_sealed_custody_impact_converge(self):
        self._join_peer()
        component = self._create_component(active=True)
        snapshot = custody_filed_reports.create_filed_report_snapshot(
            self.owner,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            report_kind="capital-gains",
            report_state="filed",
            period_start_year=2026,
            period_end_year=2026,
            content_sha256="ab" * 32,
            classification_summary={
                "external_presumed": {"count": 1, "amount_msat": 100000}
            },
            gain_summary={
                "fiat_currency": "EUR",
                "gain_loss_exact": "1.00",
                "status": "final",
            },
            created_at=NOW,
        )
        self.owner.execute(
            """
            INSERT INTO custody_gap_reviews(
                id, workspace_id, profile_id, gap_id, revision,
                candidate_fingerprint, action, component_id, authored_source,
                reason, snapshot_json, created_at
            ) VALUES('filed-review', ?, ?, 'gap', 1, ?, 'resolved', ?, 'user',
                     'reviewed bridge', '{}', ?)
            """,
            (
                self.workspace["id"],
                self.profile["id"],
                "cd" * 32,
                component["id"],
                NOW,
            ),
        )
        review_transaction_id = component["legs"][0]["transaction_id"]
        self.owner.execute(
            """
            INSERT INTO custody_gap_review_transactions(
                id, review_id, workspace_id, profile_id, ordinal,
                role, transaction_id, created_at
            ) VALUES('filed-review-source', 'filed-review', ?, ?, 0,
                     'source', ?, ?)
            """,
            (
                self.workspace["id"],
                self.profile["id"],
                review_transaction_id,
                NOW,
            ),
        )
        custody_filed_reports.append_custody_impacts(
            self.owner,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            component_id=component["id"],
            review_id="filed-review",
            gap_id="gap",
            candidate=SimpleNamespace(
                started_at=NOW,
                ended_at=NOW,
                retained_msat=100000,
                residual_msat=0,
                source_fee_msat=0,
            ),
            created_at=NOW,
        )
        owner_resolutions = custody_filed_reports.resolve_pending_custody_impacts(
            self.owner,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            rebuilt_at=NOW,
            created_at=NOW,
        )
        self.assertEqual(len(owner_resolutions), 1)
        self.owner.commit()

        result = self._sync_owner_to_peer()

        self.assertGreater(result.row_mutations, 0)
        authored_tables = [
            row["entity_table"]
            for row in self.owner.execute(
                "SELECT entity_table FROM sync_events ORDER BY replica_seq"
            ).fetchall()
        ]
        self.assertLess(
            authored_tables.index("transactions"),
            authored_tables.index("custody_gap_reviews"),
        )
        self.assertLess(
            authored_tables.index("custody_gap_reviews"),
            authored_tables.index("custody_gap_review_transactions"),
        )
        owner_review_transactions = [
            dict(row)
            for row in self.owner.execute(
                """
                SELECT review_id, workspace_id, profile_id, ordinal,
                       role, transaction_id, created_at
                FROM custody_gap_review_transactions WHERE review_id = 'filed-review'
                """
            ).fetchall()
        ]
        peer_review_transactions = [
            dict(row)
            for row in self.peer.execute(
                """
                SELECT review_id, workspace_id, profile_id, ordinal,
                       role, transaction_id, created_at
                FROM custody_gap_review_transactions WHERE review_id = 'filed-review'
                """
            ).fetchall()
        ]
        self.assertEqual(peer_review_transactions, owner_review_transactions)
        self.assertEqual(
            custody_filed_reports.list_filed_report_snapshots(
                self.peer, self.profile["id"]
            ),
            [snapshot],
        )
        owner_impacts = custody_filed_reports.list_custody_impacts(
            self.owner, self.profile["id"]
        )
        peer_impacts = custody_filed_reports.list_custody_impacts(
            self.peer, self.profile["id"]
        )
        self.assertEqual(peer_impacts, owner_impacts)
        self.assertEqual(
            peer_impacts[0]["after_gain_summary"],
            {"status": "pending_journal_rebuild"},
        )
        self.assertEqual(peer_impacts[0]["resolution"], owner_resolutions[0])
        owner_commitments = [
            tuple(row)
            for row in self.owner.execute(
                """
                SELECT ordinal, quantity_hash, detail_hash
                FROM custody_component_evidence_commitments
                WHERE component_id = ? ORDER BY ordinal
                """,
                (component["id"],),
            ).fetchall()
        ]
        peer_commitments = [
            tuple(row)
            for row in self.peer.execute(
                """
                SELECT ordinal, quantity_hash, detail_hash
                FROM custody_component_evidence_commitments
                WHERE component_id = ? ORDER BY ordinal
                """,
                (component["id"],),
            ).fetchall()
        ]
        self.assertEqual(owner_commitments, peer_commitments)
        self.assertEqual(2, len(peer_commitments))
        peer_profile = self.peer.execute(
            "SELECT * FROM profiles WHERE id = ?",
            (self.profile["id"],),
        ).fetchone()
        ledger = build_ledger_state(self.peer, peer_profile)
        quantity_state = ledger["custody_quantity"]
        self.assertFalse(quantity_state.report_blocked)
        self.assertTrue(
            any(
                decision.component_id == component["id"]
                and decision.state == "internal_reviewed"
                for decision in quantity_state.projection.decisions
            )
        )
        self.assertNotIn(
            "custody_authored_evidence_snapshots",
            SYNC_TABLE_MAP,
        )
        self.assertIn("custody_component_evidence_commitments", SYNC_TABLE_MAP)
        self.assertIn("custody_gap_review_transactions", SYNC_TABLE_MAP)
        self.assertEqual(
            0,
            self.owner.execute(
                "SELECT COUNT(*) FROM sync_events "
                "WHERE entity_table = 'custody_authored_evidence_snapshots'"
            ).fetchone()[0],
        )

    def test_changed_synced_evidence_drifts_on_author_and_receiver(self):
        self._join_peer()
        component = self._create_component(active=True)
        self._sync_owner_to_peer()
        transaction_id = component["legs"][0]["transaction_id"]

        self.owner.execute(
            "UPDATE transactions SET amount = amount + 1 WHERE id = ?",
            (transaction_id,),
        )
        local = get_component(self.owner, component["id"])
        self.assertEqual("draft", local["effective_state"])
        self.assertEqual("evidence_mismatch", local["evidence_status"]["status"])

        bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)
        result = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )
        self.assertGreater(result.row_mutations, 0)
        remote = get_component(self.peer, component["id"])
        self.assertEqual("draft", remote["effective_state"])
        self.assertEqual("evidence_mismatch", remote["evidence_status"]["status"])

    def test_synced_draft_can_receive_atomic_activation_commitments(self):
        self._join_peer()
        draft = self._create_component(active=False)
        self._sync_owner_to_peer()
        self.assertEqual("draft", get_component(self.peer, draft["id"])["state"])

        activated = activate_component(self.owner, draft["id"], activated_at=NOW)
        result = self._sync_owner_to_peer()

        self.assertEqual(0, result.rejected_events)
        remote = get_component(self.peer, draft["id"])
        self.assertEqual("active", remote["state"])
        self.assertEqual("active", remote["effective_state"])
        self.assertEqual(
            activated["expected_evidence_count"],
            remote["expected_evidence_count"],
        )
        self.assertEqual("matched", remote["evidence_status"]["status"])

    def test_missing_commitment_keeps_authored_active_component_ineffective(self):
        component = self._create_component(active=True)
        self.owner.execute(
            "INSERT INTO custody_component_purge_authorizations(profile_id) VALUES(?)",
            (self.profile["id"],),
        )
        self.owner.execute(
            "DELETE FROM custody_component_evidence_commitments "
            "WHERE component_id = ? AND ordinal = 1",
            (component["id"],),
        )
        self.owner.execute(
            "DELETE FROM custody_component_purge_authorizations WHERE profile_id = ?",
            (self.profile["id"],),
        )

        blocked = get_component(self.owner, component["id"])
        self.assertEqual("active", blocked["state"])
        self.assertEqual("draft", blocked["effective_state"])
        self.assertEqual("commitments_incomplete", blocked["evidence_status"]["status"])

    def test_commitment_collision_creates_sync_conflict_and_fails_closed(self):
        self._join_peer()
        component = self._create_component(active=True)
        self._sync_owner_to_peer()
        row = self.peer.execute(
            "SELECT * FROM custody_component_evidence_commitments "
            "WHERE component_id = ? ORDER BY ordinal LIMIT 1",
            (component["id"],),
        ).fetchone()
        spec = SYNC_TABLE_MAP["custody_component_evidence_commitments"]
        book = self.peer.execute(
            "SELECT hmac_key_b64 FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()
        wire_row = serialize_row(spec, row, hmac_key_b64=book["hmac_key_b64"])
        wire_row["detail_hash"] = "f" * 64
        event = author_event(
            self.peer,
            profile_id=self.profile["id"],
            event_type="row.upsert",
            entity_table=spec.table,
            entity_key=row_key(spec, row),
            payload={"row": wire_row},
        )
        self.assertIsNotNone(event)
        bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)

        result = import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )

        self.assertGreater(result.conflicts_created, 0)
        self.assertEqual(
            1,
            self.owner.execute(
                "SELECT COUNT(*) FROM sync_conflicts "
                "WHERE entity_table = 'custody_component_evidence_commitments' "
                "AND status = 'open'"
            ).fetchone()[0],
        )
        conflicted = get_component(self.owner, component["id"])
        self.assertEqual("draft", conflicted["effective_state"])
        self.assertEqual("commitments_conflicted", conflicted["evidence_status"]["status"])
        self.assertEqual(
            0,
            self.owner.execute(
                "SELECT COUNT(*) FROM custody_component_transaction_memberships "
                "WHERE component_id = ?",
                (component["id"],),
            ).fetchone()[0],
        )

    def test_open_db_backfills_only_preexisting_local_activation_snapshots(self):
        self._join_peer()
        component = self._create_component(active=True)
        self._sync_owner_to_peer()

        for conn in (self.owner, self.peer):
            conn.execute("DROP TRIGGER trg_custody_component_revision_immutable")
            conn.execute(
                "DROP TRIGGER trg_custody_component_evidence_revision_delete_immutable"
            )
            conn.execute(
                "DELETE FROM custody_component_evidence_commitments WHERE component_id = ?",
                (component["id"],),
            )
            conn.execute(
                "UPDATE custody_components SET expected_evidence_count = NULL WHERE id = ?",
                (component["id"],),
            )
            conn.commit()
        self.owner.close()
        self.peer.close()
        self.owner = open_db(
            Path(self.temp_owner.name), passphrase="owner-passphrase"
        )
        self.peer = open_db(Path(self.temp_peer.name), passphrase="peer-passphrase")

        owner_header = self.owner.execute(
            "SELECT expected_evidence_count FROM custody_components WHERE id = ?",
            (component["id"],),
        ).fetchone()
        peer_header = self.peer.execute(
            "SELECT expected_evidence_count FROM custody_components WHERE id = ?",
            (component["id"],),
        ).fetchone()
        self.assertEqual(2, owner_header["expected_evidence_count"])
        self.assertEqual(
            2,
            self.owner.execute(
                "SELECT COUNT(*) FROM custody_component_evidence_commitments "
                "WHERE component_id = ?",
                (component["id"],),
            ).fetchone()[0],
        )
        self.assertIsNone(peer_header["expected_evidence_count"])
        self.assertEqual(
            0,
            self.peer.execute(
                "SELECT COUNT(*) FROM custody_component_evidence_commitments "
                "WHERE component_id = ?",
                (component["id"],),
            ).fetchone()[0],
        )
        self.assertEqual("draft", get_component(self.peer, component["id"])["effective_state"])

    def test_concurrent_active_revisions_remain_visible_but_lose_memberships(self):
        original = self._create_component(active=True)
        self._join_peer()
        self._sync_owner_to_peer()
        self.assertEqual(
            2,
            self.peer.execute(
                "SELECT COUNT(*) FROM custody_component_transaction_memberships"
            ).fetchone()[0],
        )

        owner_revision = update_component(
            self.owner,
            original["id"],
            notes="owner revision",
            created_at="2026-01-02T00:00:00Z",
        )
        peer_revision = update_component(
            self.peer,
            original["id"],
            notes="peer revision",
            created_at="2026-01-02T00:00:01Z",
        )
        activate_component(
            self.owner,
            owner_revision["id"],
            activated_at="2026-01-03T00:00:00Z",
        )
        activate_component(
            self.peer,
            peer_revision["id"],
            activated_at="2026-01-03T00:00:01Z",
        )

        owner_bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        peer_bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        self.assertIsNotNone(owner_bundle)
        self.assertIsNotNone(peer_bundle)
        owner_result = import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=peer_bundle.ciphertext,
        )
        peer_result = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=owner_bundle.ciphertext,
        )

        self.assertGreater(owner_result.conflicts_created, 0)
        self.assertGreater(peer_result.conflicts_created, 0)
        for conn in (self.owner, self.peer):
            active = list_components(
                conn,
                profile_id=self.profile["id"],
                state="active",
            )
            self.assertEqual(2, len(active))
            self.assertEqual({2}, {item["revision"] for item in active})
            self.assertEqual({"draft"}, {item["effective_state"] for item in active})
            for item in active:
                self.assertIn(
                    "active_lineage_conflict",
                    {issue["code"] for issue in item["validation"]["issues"]},
                )
            self.assertEqual(
                0,
                conn.execute(
                    "SELECT COUNT(*) FROM custody_component_transaction_memberships"
                ).fetchone()[0],
            )

        supersede_component(
            self.owner,
            peer_revision["id"],
            reason="resolved concurrent revision",
        )
        surviving = list_components(
            self.owner,
            profile_id=self.profile["id"],
            state="active",
        )
        self.assertEqual([owner_revision["id"]], [item["id"] for item in surviving])
        self.assertEqual("active", surviving[0]["effective_state"])
        self.assertEqual(
            2,
            self.owner.execute(
                "SELECT COUNT(*) FROM custody_component_transaction_memberships"
            ).fetchone()[0],
        )

    def test_open_lifecycle_conflict_keeps_lww_active_revision_ineffective(self):
        component = self._create_component(active=True)
        event_ids = [
            row["id"]
            for row in self.owner.execute(
                "SELECT id FROM sync_events WHERE profile_id = ? ORDER BY replica_seq LIMIT 2",
                (self.profile["id"],),
            ).fetchall()
        ]
        conflict_id = str(uuid.uuid4())
        self.owner.execute(
            """
            INSERT INTO sync_conflicts(
                id, workspace_id, profile_id, entity_table, entity_key, field,
                local_event_id, remote_event_id, local_value_json,
                remote_value_json, status, created_at
            ) VALUES(?, ?, ?, 'custody_components', ?, 'state',
                     ?, ?, '"superseded"', '"active"', 'open', ?)
            """,
            (
                conflict_id,
                self.workspace["id"],
                self.profile["id"],
                json.dumps([component["id"]], separators=(",", ":")),
                event_ids[0],
                event_ids[1],
                NOW,
            ),
        )
        reconcile_active_memberships(self.owner, profile_id=self.profile["id"])

        conflicted = get_component(self.owner, component["id"])

        self.assertEqual(conflicted["state"], "active")
        self.assertEqual(conflicted["effective_state"], "draft")
        self.assertIn(
            "component_lifecycle_conflict",
            {issue["code"] for issue in conflicted["validation"]["issues"]},
        )
        self.assertEqual(
            0,
            self.owner.execute(
                "SELECT COUNT(*) FROM custody_component_transaction_memberships "
                "WHERE component_id = ?",
                (component["id"],),
            ).fetchone()[0],
        )

        self.owner.execute(
            "UPDATE sync_conflicts SET status = 'resolved' WHERE id = ?",
            (conflict_id,),
        )
        reconcile_active_memberships(self.owner, profile_id=self.profile["id"])
        resolved = get_component(self.owner, component["id"])
        self.assertEqual(resolved["effective_state"], "active")

    def test_cross_replica_dependency_waits_for_missing_signed_prefix(self):
        self._join_peer()
        self._join_third()
        wallet_id, out_id, in_id = self._insert_wallet_and_transactions()

        # Build while both recipients are known, but deliver this dependency
        # bundle only to the peer that will author the component first.
        dependency_bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        self.assertIsNotNone(dependency_bundle)
        import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=dependency_bundle.ciphertext,
        )
        component = create_component(
            self.peer,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            component_type="native_transfer",
            evidence_kind="ownership_graph",
            evidence_grade="exact",
            legs=[
                {
                    "role": "source",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "regtest",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 100000,
                    "transaction_id": out_id,
                    "wallet_id": wallet_id,
                },
                {
                    "role": "destination",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "regtest",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 100000,
                    "transaction_id": in_id,
                    "wallet_id": wallet_id,
                },
            ],
            allocations=[
                {
                    "source_ordinal": 0,
                    "sink_ordinal": 1,
                    "source_amount_msat": 100000,
                    "sink_amount_msat": 100000,
                }
            ],
            created_at=NOW,
        )
        component = activate_component(
            self.peer,
            component["id"],
            activated_at="2026-01-01T00:00:01Z",
        )
        component_bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        self.assertIsNotNone(component_bundle)

        deferred = import_bundle(
            self.third,
            profile_id=self.profile["id"],
            ciphertext=component_bundle.ciphertext,
        )
        self.assertGreater(deferred.pending_events, 0)
        self.assertIsNone(
            self.third.execute(
                "SELECT 1 FROM custody_components WHERE id = ?",
                (component["id"],),
            ).fetchone()
        )
        self.assertEqual(
            0,
            self.third.execute(
                "SELECT COUNT(*) FROM custody_authored_evidence_snapshots"
            ).fetchone()[0],
        )

        replayed = import_bundle(
            self.third,
            profile_id=self.profile["id"],
            ciphertext=dependency_bundle.ciphertext,
        )
        self.assertGreater(replayed.applied_events, 0)
        replayed_component = get_component(self.third, component["id"])
        self.assertEqual(component["id"], replayed_component["id"])
        self.assertEqual("active", replayed_component["effective_state"])
        self.assertEqual(
            0,
            self.third.execute(
                "SELECT COUNT(*) FROM custody_authored_evidence_snapshots "
                "WHERE subject_id = ?",
                (component["id"],),
            ).fetchone()[0],
        )
        self.assertEqual(
            2,
            self.third.execute(
                "SELECT COUNT(*) FROM custody_component_evidence_commitments "
                "WHERE component_id = ?",
                (component["id"],),
            ).fetchone()[0],
        )
        self.assertEqual(
            0,
            self.third.execute(
                "SELECT COUNT(*) FROM sync_pending_events WHERE profile_id = ?",
                (self.profile["id"],),
            ).fetchone()[0],
        )

    def test_fingerprint_dedup_preserves_wire_anchor_without_alias_tombstone(self):
        self._join_peer()
        wallet_id, out_id, in_id = self._insert_wallet_and_transactions()
        peer_out_id = str(uuid.uuid4())
        self.peer.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            ) VALUES(?, ?, ?, NULL, 'Peer watch', 'xpub', '{}', ?)
            """,
            (wallet_id, self.workspace["id"], self.profile["id"], NOW),
        )
        self.peer.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, direction, asset, amount, fee,
                raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, 'outbound', 'BTC', 100000, 0, '{}', ?)
            """,
            (
                peer_out_id,
                self.workspace["id"],
                self.profile["id"],
                wallet_id,
                f"peer-{peer_out_id}",
                f"fingerprint-{out_id}",
                NOW,
                NOW,
            ),
        )
        owner_bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        self.assertIsNotNone(owner_bundle)
        import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=owner_bundle.ciphertext,
        )
        mapped = self.peer.execute(
            "SELECT local_id FROM sync_id_map "
            "WHERE profile_id = ? AND entity_table = 'transactions' AND wire_id = ?",
            (self.profile["id"], out_id),
        ).fetchone()
        self.assertEqual(peer_out_id, mapped["local_id"])

        component = create_component(
            self.peer,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            component_type="native_transfer",
            evidence_kind="ownership_graph",
            evidence_grade="exact",
            legs=[
                {
                    "role": "source",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "regtest",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 100000,
                    "transaction_id": peer_out_id,
                    "wallet_id": wallet_id,
                },
                {
                    "role": "destination",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "regtest",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 100000,
                    "transaction_id": in_id,
                    "wallet_id": wallet_id,
                },
            ],
            allocations=[
                {
                    "source_ordinal": 0,
                    "sink_ordinal": 1,
                    "source_amount_msat": 100000,
                    "sink_amount_msat": 100000,
                }
            ],
            created_at=NOW,
        )
        peer_bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        self.assertIsNotNone(peer_bundle)
        local_replica_id = self.peer.execute(
            "SELECT local_replica_id FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()[0]
        transaction_events = self.peer.execute(
            """
            SELECT event_type, entity_key FROM sync_events
            WHERE profile_id = ? AND replica_id = ?
              AND entity_table = 'transactions'
            ORDER BY replica_seq
            """,
            (self.profile["id"], local_replica_id),
        ).fetchall()
        self.assertNotIn(
            ("row.delete", json.dumps([out_id], separators=(",", ":"))),
            {(row["event_type"], row["entity_key"]) for row in transaction_events},
        )
        leg_payloads = [
            json.loads(row["payload_json"])["row"]
            for row in self.peer.execute(
                """
                SELECT payload_json FROM sync_events
                WHERE profile_id = ? AND replica_id = ?
                  AND entity_table = 'custody_component_legs'
                  AND event_type = 'row.upsert'
                """,
                (self.profile["id"], local_replica_id),
            ).fetchall()
        ]
        source_payload = next(
            row for row in leg_payloads if row["role"] == "source"
        )
        self.assertEqual(out_id, source_payload["transaction_id"])
        self.assertEqual(out_id, source_payload["anchor_transaction_id"])
        self.assertNotIn(peer_out_id, source_payload.values())

        result = import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=peer_bundle.ciphertext,
        )
        self.assertEqual(0, result.rejected_events)
        self.assertIsNotNone(
            self.owner.execute("SELECT 1 FROM transactions WHERE id = ?", (out_id,)).fetchone()
        )
        remote = get_component(self.owner, component["id"])
        source = next(leg for leg in remote["legs"] if leg["role"] == "source")
        self.assertEqual(out_id, source["transaction_id"])
        self.assertEqual(out_id, source["anchor_transaction_id"])

        expected_alias = preferred_wire_id(
            self.owner,
            profile_id=self.profile["id"],
            table="transactions",
            local_id=out_id,
        )
        convergence_bundle = build_bundle(
            self.owner, profile_id=self.profile["id"]
        )
        self.assertIsNotNone(convergence_bundle)
        import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=convergence_bundle.ciphertext,
        )
        self.assertEqual(
            expected_alias,
            preferred_wire_id(
                self.peer,
                profile_id=self.profile["id"],
                table="transactions",
                local_id=peer_out_id,
            ),
        )
        peer_convergence_bundle = build_bundle(
            self.peer, profile_id=self.profile["id"]
        )
        self.assertIsNotNone(peer_convergence_bundle)
        import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=peer_convergence_bundle.ciphertext,
        )
        self.assertEqual(
            expected_alias,
            preferred_wire_id(
                self.owner,
                profile_id=self.profile["id"],
                table="transactions",
                local_id=out_id,
            ),
        )

        # A bundle authored by the pre-fix capture path could contain local B
        # followed by a false tombstone for imported A. Replay must retire only
        # alias A while signed alias B still maps to the same materialized row.
        transaction_spec = SYNC_TABLE_MAP["transactions"]
        peer_transaction = self.peer.execute(
            "SELECT * FROM transactions WHERE id = ?", (peer_out_id,)
        ).fetchone()
        peer_book = self.peer.execute(
            "SELECT hmac_key_b64 FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()
        legacy_payload = serialize_row(
            transaction_spec,
            peer_transaction,
            hmac_key_b64=peer_book["hmac_key_b64"],
        )
        author_event(
            self.peer,
            profile_id=self.profile["id"],
            event_type="row.upsert",
            entity_table="transactions",
            entity_key=json.dumps([peer_out_id], separators=(",", ":")),
            payload={"row": legacy_payload},
        )
        author_event(
            self.peer,
            profile_id=self.profile["id"],
            event_type="row.delete",
            entity_table="transactions",
            entity_key=json.dumps([out_id], separators=(",", ":")),
            payload={
                "key": json.dumps([out_id], separators=(",", ":")),
                "reason": "legacy-alias-row-missing",
            },
        )
        legacy_bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        self.assertIsNotNone(legacy_bundle)
        import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=legacy_bundle.ciphertext,
        )
        self.assertIsNotNone(
            self.owner.execute("SELECT 1 FROM transactions WHERE id = ?", (out_id,)).fetchone()
        )
        self.assertEqual(
            out_id,
            get_component(self.owner, component["id"])["legs"][0]["anchor_transaction_id"],
        )

    def test_economic_rows_are_sqlite_immutable_but_revision_api_still_works(self):
        component = self._create_component(active=True)
        leg_id = component["legs"][0]["id"]
        allocation_id = component["allocations"][0]["id"]
        commitment_id = self.owner.execute(
            "SELECT id FROM custody_component_evidence_commitments "
            "WHERE component_id = ? ORDER BY ordinal LIMIT 1",
            (component["id"],),
        ).fetchone()[0]

        for sql, params in (
            ("UPDATE custody_components SET notes = 'rewrite' WHERE id = ?", (component["id"],)),
            ("UPDATE custody_component_legs SET amount_msat = amount_msat + 1 WHERE id = ?", (leg_id,)),
            (
                "UPDATE custody_component_allocations "
                "SET source_amount_msat = source_amount_msat + 1 WHERE id = ?",
                (allocation_id,),
            ),
            (
                "UPDATE custody_component_evidence_commitments "
                "SET detail_hash = lower(hex(randomblob(32))) WHERE id = ?",
                (commitment_id,),
            ),
        ):
            with self.assertRaises(require_sqlcipher().IntegrityError):
                self.owner.execute(sql, params)

        for sql, params in (
            ("DELETE FROM custody_component_allocations WHERE id = ?", (allocation_id,)),
            (
                "DELETE FROM custody_component_evidence_commitments WHERE id = ?",
                (commitment_id,),
            ),
            ("DELETE FROM custody_component_legs WHERE id = ?", (leg_id,)),
            ("DELETE FROM custody_components WHERE id = ?", (component["id"],)),
            (
                """
                INSERT INTO custody_component_legs(
                    id, component_id, workspace_id, profile_id, ordinal, role,
                    rail, chain, network, asset, exposure, conservation_unit,
                    amount_msat, valuation_unit, valuation_amount, occurred_at,
                    transaction_id, anchor_transaction_id, wallet_id,
                    location_ref, notes, created_at
                )
                SELECT ?, component_id, workspace_id, profile_id, 99, role,
                       rail, chain, network, asset, exposure, conservation_unit,
                       amount_msat, valuation_unit, valuation_amount, occurred_at,
                       transaction_id, anchor_transaction_id, wallet_id,
                       location_ref, notes, created_at
                FROM custody_component_legs WHERE id = ?
                """,
                (str(uuid.uuid4()), leg_id),
            ),
            (
                """
                INSERT INTO custody_component_allocations(
                    id, component_id, workspace_id, profile_id, ordinal,
                    source_leg_id, sink_leg_id, source_amount_msat,
                    sink_amount_msat, created_at
                )
                SELECT ?, component_id, workspace_id, profile_id, 99,
                       source_leg_id, sink_leg_id, source_amount_msat,
                       sink_amount_msat, created_at
                FROM custody_component_allocations WHERE id = ?
                """,
                (str(uuid.uuid4()), allocation_id),
            ),
        ):
            with self.assertRaises(require_sqlcipher().IntegrityError):
                self.owner.execute(sql, params)

        revision = update_component(
            self.owner,
            component["id"],
            notes="new immutable revision",
            created_at="2026-01-02T00:00:00Z",
        )
        self.assertNotEqual(component["id"], revision["id"])
        self.assertEqual(2, revision["revision"])
        self.assertEqual("new immutable revision", revision["notes"])

    def test_profile_cascade_can_remove_complete_custody_scope(self):
        self._create_component(active=False)
        self.owner.execute("DELETE FROM profiles WHERE id = ?", (self.profile["id"],))
        for table in (
            "custody_component_evidence_commitments",
            "custody_component_allocations",
            "custody_component_legs",
            "custody_components",
        ):
            self.assertEqual(
                0,
                self.owner.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0],
            )

    def test_replay_rejects_in_place_economic_rewrite_and_advances_stream(self):
        self._join_peer()
        component = self._create_component(active=False)
        self._sync_owner_to_peer()
        leg = self.peer.execute(
            "SELECT * FROM custody_component_legs WHERE component_id = ? ORDER BY ordinal",
            (component["id"],),
        ).fetchone()
        spec = SYNC_TABLE_MAP["custody_component_legs"]
        book = self.peer.execute(
            "SELECT hmac_key_b64 FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()
        wire_row = serialize_row(spec, leg, hmac_key_b64=book["hmac_key_b64"])
        wire_row["amount_msat"] = int(wire_row["amount_msat"]) + 1
        authored = author_event(
            self.peer,
            profile_id=self.profile["id"],
            event_type="row.upsert",
            entity_table=spec.table,
            entity_key=row_key(spec, leg),
            payload={"row": wire_row},
        )
        self.assertIsNotNone(authored)
        supersede_component(
            self.peer,
            component["id"],
            reason="leg rewrite rejected; lifecycle remains legal",
        )
        bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)

        result = import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )
        self.assertEqual(1, result.rejected_events)
        self.assertGreater(result.applied_events, 0)
        self.assertEqual(
            100000,
            self.owner.execute(
                "SELECT amount_msat FROM custody_component_legs WHERE id = ?",
                (leg["id"],),
            ).fetchone()[0],
        )
        self.assertEqual("superseded", get_component(self.owner, component["id"])["state"])
        rejection = self.owner.execute(
            "SELECT reason FROM sync_rejected_events WHERE event_hash = ?",
            (authored.event_hash,),
        ).fetchone()
        self.assertEqual("custody_revision_immutable", rejection["reason"])

    def test_replay_rejects_signed_revision_and_child_deletes(self):
        self._join_peer()
        component = self._create_component(active=False)
        self._sync_owner_to_peer()
        targets = (
            ("custody_components", component["id"]),
            ("custody_component_legs", component["legs"][0]["id"]),
            ("custody_component_allocations", component["allocations"][0]["id"]),
        )
        authored = []
        for table, row_id in targets:
            event = author_event(
                self.peer,
                profile_id=self.profile["id"],
                event_type="row.delete",
                entity_table=table,
                entity_key=json.dumps([row_id], separators=(",", ":")),
                payload={"key": json.dumps([row_id], separators=(",", ":")), "reason": "legacy-delete"},
            )
            self.assertIsNotNone(event)
            authored.append(event)
        supersede_component(
            self.peer,
            component["id"],
            reason="delete attempts rejected",
        )
        bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)

        result = import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )
        self.assertEqual(3, result.rejected_events)
        self.assertEqual("superseded", get_component(self.owner, component["id"])["state"])
        for table, row_id in targets:
            self.assertIsNotNone(
                self.owner.execute(f"SELECT 1 FROM {table} WHERE id = ?", (row_id,)).fetchone()
            )
        reasons = {
            row["reason"]
            for row in self.owner.execute(
                "SELECT reason FROM sync_rejected_events WHERE event_hash IN (?, ?, ?)",
                tuple(event.event_hash for event in authored),
            ).fetchall()
        }
        self.assertEqual({"custody_revision_immutable"}, reasons)

    def test_replay_rejects_signed_child_append_after_commitment_is_full(self):
        self._join_peer()
        component = self._create_component(active=True)
        self._sync_owner_to_peer()
        book = self.peer.execute(
            "SELECT hmac_key_b64 FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()
        authored = []
        for table, row in (
            (
                "custody_component_legs",
                self.peer.execute(
                    "SELECT * FROM custody_component_legs "
                    "WHERE component_id = ? ORDER BY ordinal LIMIT 1",
                    (component["id"],),
                ).fetchone(),
            ),
            (
                "custody_component_allocations",
                self.peer.execute(
                    "SELECT * FROM custody_component_allocations "
                    "WHERE component_id = ? ORDER BY ordinal LIMIT 1",
                    (component["id"],),
                ).fetchone(),
            ),
        ):
            spec = SYNC_TABLE_MAP[table]
            wire_row = serialize_row(spec, row, hmac_key_b64=book["hmac_key_b64"])
            wire_row["id"] = str(uuid.uuid4())
            wire_row["ordinal"] = 99
            event = author_event(
                self.peer,
                profile_id=self.profile["id"],
                event_type="row.upsert",
                entity_table=table,
                entity_key=json.dumps([wire_row["id"]], separators=(",", ":")),
                payload={"row": wire_row},
            )
            self.assertIsNotNone(event)
            authored.append(event)
        supersede_component(self.peer, component["id"], reason="append attempts rejected")
        bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)

        result = import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )
        self.assertEqual(2, result.rejected_events)
        remote = get_component(self.owner, component["id"])
        self.assertEqual(remote["expected_leg_count"], len(remote["legs"]))
        self.assertEqual(
            remote["expected_allocation_count"], len(remote["allocations"])
        )
        self.assertEqual("superseded", remote["state"])

    def test_retracted_transaction_syncs_live_null_without_erasing_anchor(self):
        self._join_peer()
        component = self._create_component(active=False)
        self._sync_owner_to_peer()
        source = component["legs"][0]

        self.owner.execute("DELETE FROM transactions WHERE id = ?", (source["transaction_id"],))
        bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)
        result = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )

        self.assertEqual(0, result.rejected_events)
        remote = self.peer.execute(
            "SELECT transaction_id, anchor_transaction_id "
            "FROM custody_component_legs WHERE id = ?",
            (source["id"],),
        ).fetchone()
        self.assertIsNone(remote["transaction_id"])
        self.assertEqual(source["transaction_id"], remote["anchor_transaction_id"])
        validation_codes = {
            issue["code"]
            for issue in get_component(self.peer, component["id"])["validation"]["issues"]
        }
        self.assertIn("anchor_transaction_retracted", validation_codes)

    def test_retracted_wallet_syncs_nullable_leg_without_erasing_anchor(self):
        self._join_peer()
        component = self._create_component(active=False)
        self._sync_owner_to_peer()
        source = component["legs"][0]

        self.owner.execute("DELETE FROM wallets WHERE id = ?", (source["wallet_id"],))
        bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)
        result = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )

        self.assertEqual(0, result.rejected_events)
        remote = self.peer.execute(
            "SELECT transaction_id, anchor_transaction_id, wallet_id "
            "FROM custody_component_legs WHERE id = ?",
            (source["id"],),
        ).fetchone()
        self.assertIsNone(remote["transaction_id"])
        self.assertIsNone(remote["wallet_id"])
        self.assertEqual(source["transaction_id"], remote["anchor_transaction_id"])
        self.assertIn(
            "anchor_transaction_retracted",
            {
                issue["code"]
                for issue in get_component(self.peer, component["id"])["validation"]["issues"]
            },
        )

    def test_live_remote_leg_does_not_wedge_peer_with_retracted_anchor(self):
        self._join_peer()
        wallet_id, out_id, in_id = self._insert_wallet_and_transactions()
        self._sync_owner_to_peer()
        self.peer.execute("DELETE FROM transactions WHERE id = ?", (out_id,))

        component = create_component(
            self.owner,
            workspace_id=self.workspace["id"],
            profile_id=self.profile["id"],
            component_type="native_transfer",
            evidence_kind="ownership_graph",
            evidence_grade="exact",
            legs=[
                {
                    "role": "source",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "regtest",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 100000,
                    "transaction_id": out_id,
                    "wallet_id": wallet_id,
                },
                {
                    "role": "destination",
                    "rail": "bitcoin",
                    "chain": "bitcoin",
                    "network": "regtest",
                    "asset": "BTC",
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": 100000,
                    "transaction_id": in_id,
                    "wallet_id": wallet_id,
                },
            ],
            allocations=[
                {
                    "source_ordinal": 0,
                    "sink_ordinal": 1,
                    "source_amount_msat": 100000,
                    "sink_amount_msat": 100000,
                }
            ],
            created_at=NOW,
        )
        bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)

        result = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )

        self.assertEqual(0, result.rejected_events)
        remote = self.peer.execute(
            "SELECT transaction_id, anchor_transaction_id "
            "FROM custody_component_legs WHERE component_id = ? AND ordinal = 0",
            (component["id"],),
        ).fetchone()
        self.assertIsNone(remote["transaction_id"])
        self.assertEqual(out_id, remote["anchor_transaction_id"])
        validation_codes = {
            issue["code"]
            for issue in get_component(self.peer, component["id"])["validation"]["issues"]
        }
        self.assertIn("anchor_transaction_retracted", validation_codes)

    def test_legacy_leg_event_without_anchor_preserves_materialized_anchor(self):
        self._join_peer()
        component = self._create_component(active=False)
        self._sync_owner_to_peer()
        leg = self.peer.execute(
            "SELECT * FROM custody_component_legs WHERE component_id = ? ORDER BY ordinal",
            (component["id"],),
        ).fetchone()
        spec = SYNC_TABLE_MAP["custody_component_legs"]
        book = self.peer.execute(
            "SELECT hmac_key_b64 FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()
        legacy_wire_row = serialize_row(
            spec,
            leg,
            hmac_key_b64=book["hmac_key_b64"],
        )
        legacy_wire_row.pop("anchor_transaction_id")
        authored = author_event(
            self.peer,
            profile_id=self.profile["id"],
            event_type="row.upsert",
            entity_table=spec.table,
            entity_key=row_key(spec, leg),
            payload={"row": legacy_wire_row},
        )
        self.assertIsNotNone(authored)
        bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)

        result = import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )
        self.assertEqual(0, result.rejected_events)
        preserved = self.owner.execute(
            "SELECT transaction_id, anchor_transaction_id "
            "FROM custody_component_legs WHERE id = ?",
            (leg["id"],),
        ).fetchone()
        self.assertEqual(leg["transaction_id"], preserved["transaction_id"])
        self.assertEqual(leg["transaction_id"], preserved["anchor_transaction_id"])

    def test_legacy_leg_insert_preserves_anchor_after_live_transaction_retraction(self):
        self._join_peer()
        component = self._create_component(active=False)
        self._sync_owner_to_peer()
        source = self.owner.execute(
            "SELECT * FROM custody_component_legs "
            "WHERE component_id = ? ORDER BY ordinal LIMIT 1",
            (component["id"],),
        ).fetchone()
        self.peer.execute(
            "INSERT INTO custody_component_purge_authorizations(profile_id) VALUES(?)",
            (self.profile["id"],),
        )
        self.peer.execute(
            "DELETE FROM custody_component_legs WHERE id = ?",
            (source["id"],),
        )
        self.peer.execute(
            "DELETE FROM custody_component_purge_authorizations WHERE profile_id = ?",
            (self.profile["id"],),
        )
        self.peer.execute(
            "DELETE FROM transactions WHERE id = ?",
            (source["transaction_id"],),
        )

        spec = SYNC_TABLE_MAP["custody_component_legs"]
        book = self.owner.execute(
            "SELECT hmac_key_b64 FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()
        legacy_wire_row = serialize_row(
            spec,
            source,
            hmac_key_b64=book["hmac_key_b64"],
        )
        legacy_wire_row.pop("anchor_transaction_id")
        authored = author_event(
            self.owner,
            profile_id=self.profile["id"],
            event_type="row.upsert",
            entity_table=spec.table,
            entity_key=row_key(spec, source),
            payload={"row": legacy_wire_row},
        )
        self.assertIsNotNone(authored)
        bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)

        result = import_bundle(
            self.peer,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )

        self.assertEqual(0, result.rejected_events)
        preserved = self.peer.execute(
            "SELECT transaction_id, anchor_transaction_id "
            "FROM custody_component_legs WHERE id = ?",
            (legacy_wire_row["id"],),
        ).fetchone()
        self.assertIsNone(preserved["transaction_id"])
        self.assertEqual(
            source["transaction_id"],
            preserved["anchor_transaction_id"],
        )

    def test_replicated_custody_revision_rejects_invalid_created_at(self):
        self._join_peer()
        component = self._create_component(active=False)
        self._sync_owner_to_peer()
        row = self.peer.execute(
            "SELECT * FROM custody_components WHERE id = ?", (component["id"],)
        ).fetchone()
        spec = SYNC_TABLE_MAP["custody_components"]
        book = self.peer.execute(
            "SELECT hmac_key_b64 FROM sync_books WHERE profile_id = ?",
            (self.profile["id"],),
        ).fetchone()
        wire_row = serialize_row(spec, row, hmac_key_b64=book["hmac_key_b64"])
        wire_row["id"] = str(uuid.uuid4())
        wire_row["lineage_id"] = wire_row["id"]
        wire_row["created_at"] = "not-a-timestamp"
        event = author_event(
            self.peer,
            profile_id=self.profile["id"],
            event_type="row.upsert",
            entity_table="custody_components",
            entity_key=json.dumps([wire_row["id"]], separators=(",", ":")),
            payload={"row": wire_row},
        )
        self.assertIsNotNone(event)
        bundle = build_bundle(self.peer, profile_id=self.profile["id"])
        result = import_bundle(
            self.owner,
            profile_id=self.profile["id"],
            ciphertext=bundle.ciphertext,
        )

        self.assertEqual(1, result.rejected_events)
        rejected = self.owner.execute(
            "SELECT * FROM sync_rejected_events "
            "WHERE reason = 'custody_revision_timestamp_invalid' AND event_hash = ?",
            (event.event_hash,),
        ).fetchone()
        self.assertIsNotNone(rejected)
        replica = self.owner.execute(
            "SELECT * FROM sync_replicas WHERE id = ?", (rejected["replica_id"],)
        ).fetchone()
        self.assertGreaterEqual(replica["last_seq"], rejected["replica_seq"])
        self.assertEqual(
            0,
            self.owner.execute(
                "SELECT COUNT(*) FROM sync_pending_events WHERE replica_id = ?",
                (rejected["replica_id"],),
            ).fetchone()[0],
        )

    def test_legacy_immutable_conflict_requires_a_new_revision(self):
        component = self._create_component(active=False)
        bundle = build_bundle(self.owner, profile_id=self.profile["id"])
        self.assertIsNotNone(bundle)
        event_ids = [
            row["id"]
            for row in self.owner.execute(
                "SELECT id FROM sync_events WHERE profile_id = ? ORDER BY replica_seq LIMIT 2",
                (self.profile["id"],),
            ).fetchall()
        ]
        self.assertEqual(2, len(event_ids))
        conflict_id = str(uuid.uuid4())
        self.owner.execute(
            """
            INSERT INTO sync_conflicts(
                id, workspace_id, profile_id, entity_table, entity_key, field,
                local_event_id, remote_event_id, local_value_json,
                remote_value_json, status, created_at
            ) VALUES(?, ?, ?, 'custody_components', ?, 'component_type',
                     ?, ?, ?, ?, 'open', ?)
            """,
            (
                conflict_id,
                self.workspace["id"],
                self.profile["id"],
                json.dumps([component["id"]], separators=(",", ":")),
                event_ids[0],
                event_ids[1],
                json.dumps("native_transfer"),
                json.dumps("conversion"),
                NOW,
            ),
        )
        with self.assertRaises(AppError) as caught:
            resolve_conflict(
                self.owner,
                profile_id=self.profile["id"],
                conflict_id=conflict_id,
                custom_value="conversion",
                use_custom_value=True,
            )
        self.assertEqual("sync_conflict_requires_revision", caught.exception.code)

        resolved = resolve_conflict(
            self.owner,
            profile_id=self.profile["id"],
            conflict_id=conflict_id,
            custom_value=component["component_type"],
            use_custom_value=True,
        )
        self.assertEqual("resolved", resolved["status"])
        self.assertEqual(
            component["component_type"],
            get_component(self.owner, component["id"])["component_type"],
        )

    def test_existence_conflict_holds_active_component_until_acknowledged(self):
        component = self._create_component(active=True)
        event_ids = [
            row["id"]
            for row in self.owner.execute(
                "SELECT id FROM sync_events WHERE profile_id = ? ORDER BY replica_seq LIMIT 2",
                (self.profile["id"],),
            ).fetchall()
        ]
        conflict_id = str(uuid.uuid4())
        self.owner.execute(
            """
            INSERT INTO sync_conflicts(
                id, workspace_id, profile_id, entity_table, entity_key, field,
                local_event_id, remote_event_id, local_value_json,
                remote_value_json, status, created_at
            ) VALUES(?, ?, ?, 'custody_components', ?, '__exists__',
                     ?, ?, 'false', 'true', 'open', ?)
            """,
            (
                conflict_id,
                self.workspace["id"],
                self.profile["id"],
                json.dumps([component["id"]], separators=(",", ":")),
                event_ids[0],
                event_ids[1],
                NOW,
            ),
        )
        reconcile_active_memberships(self.owner, profile_id=self.profile["id"])
        self.assertEqual("draft", get_component(self.owner, component["id"])["effective_state"])

        resolved = resolve_conflict(
            self.owner,
            profile_id=self.profile["id"],
            conflict_id=conflict_id,
            custom_value=True,
            use_custom_value=True,
        )

        self.assertEqual("resolved", resolved["status"])
        self.assertEqual("active", get_component(self.owner, component["id"])["effective_state"])


if __name__ == "__main__":
    unittest.main()
