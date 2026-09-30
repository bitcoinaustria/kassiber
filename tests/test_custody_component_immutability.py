"""Local SQLite invariants for authored custody-component revisions."""

from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
import uuid
from pathlib import Path

from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.custody_components import (
    activate_component,
    create_component,
    update_component,
)
from kassiber.db import open_db
from kassiber.secrets.sqlcipher import require_sqlcipher, sqlcipher_available


NOW = "2026-01-01T00:00:00Z"


@unittest.skipUnless(sqlcipher_available(), "SQLCipher driver unavailable")
class CustodyComponentImmutabilityTests(unittest.TestCase):
    def setUp(self):
        self.temp_owner = tempfile.TemporaryDirectory()
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

    def tearDown(self):
        self.owner.close()
        self.temp_owner.cleanup()

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
            component_type="manual_bridge",
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

    def test_open_db_backfills_preexisting_local_activation_snapshot(self):
        component = self._create_component(active=True)
        self.owner.execute("DROP TRIGGER trg_custody_component_revision_immutable")
        self.owner.execute(
            "DROP TRIGGER trg_custody_component_evidence_revision_delete_immutable"
        )
        self.owner.execute(
            "DELETE FROM custody_component_evidence_commitments WHERE component_id = ?",
            (component["id"],),
        )
        self.owner.execute(
            "UPDATE custody_components SET expected_evidence_count = NULL WHERE id = ?",
            (component["id"],),
        )
        self.owner.commit()
        self.owner.close()
        self.owner = open_db(Path(self.temp_owner.name), passphrase="owner-passphrase")

        header = self.owner.execute(
            "SELECT expected_evidence_count FROM custody_components WHERE id = ?",
            (component["id"],),
        ).fetchone()
        self.assertEqual(2, header["expected_evidence_count"])
        self.assertEqual(
            2,
            self.owner.execute(
                "SELECT COUNT(*) FROM custody_component_evidence_commitments "
                "WHERE component_id = ?",
                (component["id"],),
            ).fetchone()[0],
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


if __name__ == "__main__":
    unittest.main()
