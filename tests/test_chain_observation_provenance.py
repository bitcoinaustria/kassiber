from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

from kassiber.core.chain_observer.provenance import (
    canonical_graph_hash,
    canonical_observed_quantity_hash,
    persist_chain_observation_provenance,
    provenance_entries_for_facts,
    row_has_current_authoritative_observation,
)
from kassiber.core.custody_evidence import assess_authoritative_chain_observation
from kassiber.core.imports import ImportCoordinatorHooks, insert_wallet_records
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.msat import btc_to_msat
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

    def _observer_record(
        self,
        *,
        txid: str = "ab" * 32,
        amount: str = "0.000001",
    ) -> dict[str, object]:
        return {
            "txid": txid,
            "occurred_at": self._row()["occurred_at"],
            "direction": "outbound",
            "asset": "LBTC",
            "amount": amount,
            "fee": "0",
            "raw_json": {"observer": "lwk"},
        }

    def _set_base_projection(
        self,
        *,
        txid: str = "ab" * 32,
        amount: str = "0.000001",
        excluded: bool = False,
    ) -> None:
        self.conn.execute(
            """
            UPDATE transactions
            SET external_id = ?, fingerprint = ?, amount = ?, excluded = ?
            WHERE id = 'tx'
            """,
            (
                txid,
                make_transaction_fingerprint(
                    "wallet",
                    txid,
                    self._row()["occurred_at"],
                    "outbound",
                    "LBTC",
                    amount,
                    "0",
                ),
                btc_to_msat(amount),
                1 if excluded else 0,
            ),
        )

    def _insert_sibling(
        self,
        *,
        amount: str,
        excluded: bool,
        transaction_id: str = "sibling",
        txid: str = "ab" * 32,
    ) -> None:
        row = self._row()
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, direction, asset, amount, fee,
                excluded, note, raw_json, created_at
            ) VALUES(
                ?, 'ws', 'profile', 'wallet', ?, ?, ?, 'outbound', 'LBTC',
                ?, 0, ?, 'preserve sibling evidence', '{}', ?
            )
            """,
            (
                transaction_id,
                txid,
                make_transaction_fingerprint(
                    "wallet",
                    txid,
                    row["occurred_at"],
                    "outbound",
                    "LBTC",
                    amount,
                    "0",
                ),
                row["occurred_at"],
                btc_to_msat(amount),
                1 if excluded else 0,
                row["created_at"],
            ),
        )

    def _insert_authoritative(self, record: dict[str, object]):
        return insert_wallet_records(
            self.conn,
            self.profile,
            self.wallet,
            [record],
            "lwk",
            ImportCoordinatorHooks(
                ensure_tag_row=Mock(),
                invalidate_journals=Mock(),
            ),
            commit=False,
            report_updates=True,
            authoritative_chain_observer=True,
        )

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
            resolved_records=(
                {
                    "transaction_id": "tx",
                    "external_id": "ab" * 32,
                    "asset": "LBTC",
                    "direction": "outbound",
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
            resolved_records=(
                {
                    "transaction_id": "tx",
                    "external_id": "ab" * 32,
                    "asset": asset_id,
                    "direction": "outbound",
                },
            ),
        )

        self.assertEqual(persisted, 1)
        self.assertTrue(row_has_current_authoritative_observation(self._row()))

    def test_authoritative_import_normalizes_case_and_preserves_row_id(self):
        uppercase_txid = ("ab" * 32).upper()
        self._set_base_projection(txid=uppercase_txid)

        outcome = self._insert_authoritative(self._observer_record())

        row = self._row()
        self.assertEqual(row["external_id"], "ab" * 32)
        self.assertEqual(
            row["fingerprint"],
            make_transaction_fingerprint(
                "wallet",
                "ab" * 32,
                row["occurred_at"],
                "outbound",
                "LBTC",
                "0.000001",
                "0",
            ),
        )
        self.assertEqual(outcome["imported"], 0)
        self.assertEqual(
            outcome["_observer_resolved_records"][0]["transaction_id"],
            "tx",
        )
        self.assertEqual(
            self.conn.execute(
                "SELECT COUNT(*) FROM transactions WHERE wallet_id = 'wallet'"
            ).fetchone()[0],
            1,
        )

    def test_explicit_exclusion_selects_active_keeper_for_provenance(self):
        self._set_base_projection()
        self._insert_sibling(amount="0.000002", excluded=True)

        outcome = self._insert_authoritative(
            self._observer_record(amount="0.000003")
        )
        resolved = outcome["_observer_resolved_records"]
        persisted = persist_chain_observation_provenance(
            self.conn,
            self.profile,
            self.wallet,
            application_revision="resolved-keeper",
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
            resolved_records=resolved,
        )

        rows = self.conn.execute(
            """
            SELECT id, amount, excluded, note
            FROM transactions
            WHERE wallet_id = 'wallet'
            ORDER BY id
            """
        ).fetchall()
        self.assertEqual(persisted, 1)
        self.assertEqual(resolved[0]["transaction_id"], "tx")
        self.assertEqual(rows[0]["id"], "sibling")
        self.assertEqual(rows[0]["amount"], btc_to_msat("0.000002"))
        self.assertEqual(rows[0]["excluded"], 1)
        self.assertEqual(rows[0]["note"], "preserve sibling evidence")
        self.assertEqual(rows[1]["id"], "tx")
        self.assertEqual(rows[1]["amount"], btc_to_msat("0.000003"))
        self.assertEqual(
            self.conn.execute(
                "SELECT transaction_id FROM chain_observation_provenance"
            ).fetchone()[0],
            "tx",
        )

    def test_multiple_active_or_all_excluded_rows_remain_fail_closed(self):
        for base_excluded, sibling_excluded, expected_kind in (
            (False, False, "multiple_active_transaction_rows"),
            (True, True, "multiple_excluded_transaction_rows"),
        ):
            with self.subTest(expected_kind=expected_kind):
                self._set_base_projection(excluded=base_excluded)
                self.conn.execute("DELETE FROM transactions WHERE id = 'sibling'")
                self._insert_sibling(
                    amount="0.000002",
                    excluded=sibling_excluded,
                )
                before = self.conn.execute(
                    """
                    SELECT id, fingerprint, amount, excluded
                    FROM transactions ORDER BY id
                    """
                ).fetchall()

                with self.assertRaises(AppError) as raised:
                    self._insert_authoritative(
                        self._observer_record(amount="0.000003")
                    )

                self.assertEqual(
                    raised.exception.details["conflict_kind"],
                    expected_kind,
                )
                after = self.conn.execute(
                    """
                    SELECT id, fingerprint, amount, excluded
                    FROM transactions ORDER BY id
                    """
                ).fetchall()
                self.assertEqual([tuple(row) for row in after], [tuple(row) for row in before])

    def test_mixed_case_legacy_duplicate_remains_ambiguous(self):
        self._set_base_projection()
        self._insert_sibling(
            amount="0.000002",
            excluded=False,
            txid="aB" * 32,
        )

        with self.assertRaises(AppError) as raised:
            self._insert_authoritative(self._observer_record(amount="0.000003"))

        self.assertEqual(
            raised.exception.details["conflict_kind"],
            "multiple_active_transaction_rows",
        )

    def test_authoritative_lookup_uses_case_insensitive_index(self):
        plan = self.conn.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT id
            FROM transactions
            WHERE wallet_id = ? AND external_id IS NOT NULL
              AND LOWER(external_id) = ?
              AND direction = ? AND asset = ?
            ORDER BY created_at DESC, id DESC
            """,
            ("wallet", "ab" * 32, "outbound", "LBTC"),
        ).fetchall()

        self.assertTrue(
            any(
                "idx_transactions_wallet_external_ci_match" in row["detail"]
                for row in plan
            ),
            [row["detail"] for row in plan],
        )

    def test_excluded_exact_fingerprint_never_switches_active_keeper(self):
        self._set_base_projection()
        self._insert_sibling(amount="0.000002", excluded=True)

        with self.assertRaises(AppError) as raised:
            self._insert_authoritative(
                self._observer_record(amount="0.000002")
            )

        self.assertEqual(
            raised.exception.details["conflict_kind"],
            "excluded_exact_transaction_row",
        )
        self.assertEqual(self._row()["amount"], btc_to_msat("0.000001"))

    def test_provenance_rejects_a_resolved_id_outside_the_observer_scope(self):
        with self.assertRaises(AppError) as raised:
            persist_chain_observation_provenance(
                self.conn,
                self.profile,
                self.wallet,
                application_revision="wrong-row",
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
                resolved_records=(
                    {
                        "transaction_id": "missing",
                        "external_id": "ab" * 32,
                        "asset": "LBTC",
                        "direction": "outbound",
                    },
                ),
            )

        self.assertEqual(
            raised.exception.details["conflict_kind"],
            "provenance_resolution_mismatch",
        )
        self.assertEqual(
            raised.exception.details["resolution_reason"],
            "resolved_row_missing",
        )


if __name__ == "__main__":
    unittest.main()
