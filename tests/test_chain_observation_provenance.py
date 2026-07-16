from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from kassiber.core.chain_observer.provenance import (
    canonical_graph_hash,
    canonical_observed_quantity_hash,
    persist_chain_observation_provenance,
    provenance_entries_for_facts,
    row_has_current_authoritative_observation,
)
from kassiber.core.custody_evidence import assess_authoritative_chain_observation
from kassiber.db import open_db
from kassiber.time_utils import now_iso


class ChainObservationProvenanceTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kassiber-observation-proof-")
        self.addCleanup(self.temp.cleanup)
        self.conn = open_db(Path(self.temp.name) / "data")
        self.addCleanup(self.conn.close)
        timestamp = now_iso()
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'WS', ?)",
            (timestamp,),
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES('profile', 'ws', 'Profile', 'EUR', 'generic', 365, 'FIFO', ?)
            """,
            (timestamp,),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES('wallet', 'ws', 'profile', 'Liquid', 'descriptor', '{}', ?)
            """,
            (timestamp,),
        )
        self.raw = json.dumps(
            {
                "txid": "ab" * 32,
                # A generic import can imitate this text; it grants no authority.
                "observer": "lwk",
                "component": {"fee_attribution": "implicit_wallet_delta"},
                "vin": [],
                "vout": [],
            },
            sort_keys=True,
        )
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                external_id_kind, fingerprint, occurred_at, direction, asset,
                amount, fee, amount_includes_fee, raw_json, created_at
            ) VALUES(
                'tx', 'ws', 'profile', 'wallet', ?, NULL, 'fingerprint', ?,
                'outbound', 'LBTC', 100000, 0, 1, ?, ?
            )
            """,
            ("ab" * 32, timestamp, self.raw, timestamp),
        )
        self.profile = self.conn.execute(
            "SELECT * FROM profiles WHERE id = 'profile'"
        ).fetchone()
        self.wallet = self.conn.execute(
            "SELECT * FROM wallets WHERE id = 'wallet'"
        ).fetchone()

    def _row(self):
        return self.conn.execute(
            """
            SELECT
                tx.*,
                proof.authority_version AS observation_authority_version,
                proof.graph_hash AS observation_graph_hash,
                proof.quantity_hash AS observation_quantity_hash,
                proof.fee_attribution AS observation_fee_attribution
            FROM transactions tx
            LEFT JOIN chain_observation_provenance proof
              ON proof.transaction_id = tx.id
            WHERE tx.id = 'tx'
            """
        ).fetchone()

    def test_raw_observer_marker_never_grants_authority(self):
        self.assertFalse(row_has_current_authoritative_observation(self._row()))

    def test_persisted_authority_is_bound_to_graph_and_quantity(self):
        persisted = persist_chain_observation_provenance(
            self.conn,
            self.profile,
            self.wallet,
            application_revision="apply-random-id",
            chain="liquid",
            network="main",
            entries=(
                {
                    "external_id": "ab" * 32,
                    "asset": "LBTC",
                    "direction": "outbound",
                    "observer_ids": ["descriptor:structural"],
                    "observer_kinds": ["lwk"],
                },
            ),
        )
        self.assertEqual(persisted, 1)
        row = self._row()
        self.assertTrue(row_has_current_authoritative_observation(row))
        self.assertEqual(row["external_id_kind"], "txid")
        self.assertTrue(assess_authoritative_chain_observation(row).authoritative)
        self.assertEqual(row["observation_fee_attribution"], "implicit_wallet_delta")
        self.assertEqual(row["observation_graph_hash"], canonical_graph_hash(self.raw))
        self.assertEqual(
            row["observation_quantity_hash"],
            canonical_observed_quantity_hash(row),
        )

        self.conn.execute(
            "UPDATE transactions SET raw_json = ? WHERE id = 'tx'",
            (json.dumps({"observer": "lwk", "vin": [{"fake": True}]}),),
        )
        self.assertFalse(row_has_current_authoritative_observation(self._row()))

        self.conn.execute(
            "UPDATE transactions SET raw_json = ?, amount = amount + 1 WHERE id = 'tx'",
            (self.raw,),
        )
        self.assertFalse(row_has_current_authoritative_observation(self._row()))

    def test_issued_asset_identity_uses_canonical_lowercase_hex(self):
        asset_id = "b2" * 32
        self.conn.execute(
            "UPDATE transactions SET asset = ? WHERE id = 'tx'",
            (asset_id,),
        )

        entries = provenance_entries_for_facts(
            (
                (
                    SimpleNamespace(id="descriptor:structural", observer_kind="lwk"),
                    (
                        {
                            "external_id": "ab" * 32,
                            "asset": asset_id.upper(),
                            "direction": "outbound",
                        },
                    ),
                ),
            ),
            (
                {
                    "external_id": "ab" * 32,
                    "asset": asset_id,
                    "direction": "outbound",
                },
            ),
        )
        self.assertEqual(entries[0]["asset"], asset_id)

        persisted = persist_chain_observation_provenance(
            self.conn,
            self.profile,
            self.wallet,
            application_revision="issued-asset-apply",
            chain="liquid",
            network="regtest",
            entries=entries,
        )

        self.assertEqual(persisted, 1)
        self.assertTrue(row_has_current_authoritative_observation(self._row()))


if __name__ == "__main__":
    unittest.main()
