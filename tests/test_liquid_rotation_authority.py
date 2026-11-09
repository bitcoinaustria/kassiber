"""An owned Liquid rotation keeps its observer authority across database opens.

Liquid sync commits provenance to the exact graph it stores, with the asset
identity inside ``component``. The Liquid identity backfill runs on every open
and used to add top-level ``asset``/``asset_id`` to those rows, which silently
revoked their authority: the same-txid move between two owned wallets then
booked as a disposal plus a fresh acquisition.
"""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from kassiber.cli import handlers
from kassiber.core.chain_observer.provenance import (
    canonical_graph_hash,
    persist_chain_observation_provenance,
    row_has_current_authoritative_observation,
)
from kassiber.db import open_db
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.msat import btc_to_msat

POLICY_ASSET = "b2e15d0d7a0c94e4e2ce0fe6e8691b9e451377f6e46e8045a86f7c4b5d4f0f23"
FUNDING_TXID = "8d37c223ff548ea044c423c49299d7606a44a434346a993d709d5eeced4e8434"
ROTATION_TXID = "1d845bdeb4e60cce7ab48b881bcf684d1a618e3b940a5dcfa2eee53742041bca"
CREATED_AT = "2026-01-01T00:00:00Z"


def _wallet_config() -> str:
    return json.dumps(
        {
            "chain": "liquid",
            "network": "elementsregtest",
            "policy_asset": POLICY_ASSET,
        },
        sort_keys=True,
    )


def _sync_raw(txid: str, net_sats: int, fee_sats: int) -> str:
    """The payload shape current Liquid sync stores and commits to."""

    return json.dumps(
        {
            "txid": txid,
            "chain": "liquid",
            "network": "elementsregtest",
            "ownership_graph_version": 1,
            "vin": [],
            "vout": [],
            "component": {
                "asset_id": POLICY_ASSET,
                "asset": "LBTC",
                "net_sats": net_sats,
                "fee_sats": fee_sats,
            },
        },
        sort_keys=True,
    )


class LiquidRotationAuthorityTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="kassiber-liquid-rotation-")
        self.addCleanup(temp.cleanup)
        self.data_root = Path(temp.name) / "data"
        conn = open_db(self.data_root)
        try:
            self._seed(conn)
            conn.commit()
        finally:
            conn.close()

    def _seed(self, conn):
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Books', ?)",
            (CREATED_AT,),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES('profile', 'ws', 'Book', 'EUR', 'generic', 365, 'FIFO', ?)
            """,
            (CREATED_AT,),
        )
        for wallet_id, label in (("old", "Liquid Treasury"), ("new", "Liquid Treasury Rotation")):
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, label, kind, config_json, created_at
                ) VALUES(?, 'ws', 'profile', ?, 'descriptor', ?, ?)
                """,
                (wallet_id, label, _wallet_config(), CREATED_AT),
            )
        self._observe(conn, "funding", "old", FUNDING_TXID, "2024-01-10T10:00:00Z",
                      "inbound", "0.1", "0", 40000, net_sats=10_000_000, fee_sats=0)
        self._observe(conn, "rotation-out", "old", ROTATION_TXID, "2024-05-24T09:51:00Z",
                      "outbound", "0.0575", "0.0002491", 60000,
                      net_sats=-5_774_910, fee_sats=24_910)
        self._observe(conn, "rotation-in", "new", ROTATION_TXID, "2024-05-24T09:51:00Z",
                      "inbound", "0.0575", "0", 60000, net_sats=5_750_000, fee_sats=0)

    def _observe(self, conn, transaction_id, wallet_id, txid, occurred_at, direction,
                 amount, fee, rate, *, net_sats, fee_sats):
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, kind, raw_json, created_at
            ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, ?, 'LBTC', ?, ?, 'EUR', ?, ?, ?, ?, ?)
            """,
            (
                transaction_id,
                wallet_id,
                txid,
                make_transaction_fingerprint(
                    wallet_id, txid, occurred_at, direction, "LBTC", amount, fee
                ),
                occurred_at,
                occurred_at,
                direction,
                btc_to_msat(amount),
                btc_to_msat(fee),
                rate,
                float(amount) * rate,
                "deposit" if direction == "inbound" else "withdrawal",
                _sync_raw(txid, net_sats, fee_sats),
                CREATED_AT,
            ),
        )
        record = {"external_id": txid, "asset": "LBTC", "direction": direction}
        persist_chain_observation_provenance(
            conn,
            conn.execute("SELECT * FROM profiles WHERE id = 'profile'").fetchone(),
            conn.execute("SELECT * FROM wallets WHERE id = ?", (wallet_id,)).fetchone(),
            application_revision=f"test:{transaction_id}",
            chain="liquid",
            network="elementsregtest",
            entries=({**record, "observer_ids": ["test:electrum"], "observer_kinds": ["electrum"]},),
            resolved_records=({**record, "transaction_id": transaction_id},),
        )

    def _authority(self, conn):
        rows = conn.execute(
            """
            SELECT t.*, p.authority_version AS observation_authority_version,
                   p.graph_hash AS observation_graph_hash,
                   p.quantity_hash AS observation_quantity_hash
            FROM transactions t
            JOIN chain_observation_provenance p ON p.transaction_id = t.id
            ORDER BY t.id
            """
        ).fetchall()
        return {row["id"]: row_has_current_authoritative_observation(row) for row in rows}

    def _journal_entries(self, conn):
        handlers.process_journals(conn, "ws", "profile")
        return {
            (row["transaction_id"], row["entry_type"]): row["gain_loss"]
            for row in conn.execute(
                "SELECT transaction_id, entry_type, gain_loss FROM journal_entries"
            )
        }

    def _assert_rotation_is_a_move(self, conn):
        entries = self._journal_entries(conn)
        self.assertIn(("rotation-out", "transfer_out"), entries)
        self.assertIn(("rotation-in", "transfer_in"), entries)
        self.assertNotIn(("rotation-out", "disposal"), entries)
        self.assertNotIn(("rotation-in", "acquisition"), entries)
        decisions = conn.execute(
            """
            SELECT source_transaction_id, target_transaction_id, state, reason
            FROM journal_custody_decisions
            """
        ).fetchall()
        self.assertEqual(
            [tuple(row) for row in decisions],
            [("rotation-out", "rotation-in", "internal_verified", "row_matched")],
        )

    def test_reopened_book_keeps_owned_liquid_rotation_a_move(self):
        conn = open_db(self.data_root)
        try:
            self.assertEqual(
                self._authority(conn),
                {"funding": True, "rotation-in": True, "rotation-out": True},
            )
            raw = json.loads(
                conn.execute(
                    "SELECT raw_json FROM transactions WHERE id = 'rotation-out'"
                ).fetchone()["raw_json"]
            )
            self.assertNotIn("asset_id", raw)
            self._assert_rotation_is_a_move(conn)
        finally:
            conn.close()

    def test_open_restores_graphs_an_earlier_backfill_rewrote(self):
        conn = open_db(self.data_root)
        try:
            for row in conn.execute("SELECT id, raw_json FROM transactions").fetchall():
                # What the backfill used to write on every open.
                raw = {**json.loads(row["raw_json"]), "asset": "LBTC", "asset_id": POLICY_ASSET}
                conn.execute(
                    "UPDATE transactions SET raw_json = ? WHERE id = ?",
                    (json.dumps(raw, sort_keys=True), row["id"]),
                )
            conn.execute(
                "UPDATE profiles SET last_processed_at = '2026-01-02T00:00:00Z' WHERE id = 'profile'"
            )
            conn.commit()
            self.assertFalse(any(self._authority(conn).values()))
            input_version = conn.execute(
                "SELECT journal_input_version FROM profiles WHERE id = 'profile'"
            ).fetchone()[0]
        finally:
            conn.close()

        conn = open_db(self.data_root)
        try:
            self.assertEqual(
                self._authority(conn),
                {"funding": True, "rotation-in": True, "rotation-out": True},
            )
            profile = conn.execute(
                "SELECT journal_input_version, last_processed_at FROM profiles WHERE id = 'profile'"
            ).fetchone()
            self.assertEqual(profile["journal_input_version"], input_version + 1)
            self.assertIsNone(profile["last_processed_at"])
            self._assert_rotation_is_a_move(conn)
        finally:
            conn.close()

    def test_unprovable_rewrite_stays_unauthoritative(self):
        conn = open_db(self.data_root)
        try:
            row = conn.execute(
                "SELECT raw_json FROM transactions WHERE id = 'rotation-out'"
            ).fetchone()
            raw = {**json.loads(row["raw_json"]), "asset": "LBTC", "vin": [{"forged": True}]}
            conn.execute(
                "UPDATE transactions SET raw_json = ? WHERE id = 'rotation-out'",
                (json.dumps(raw, sort_keys=True),),
            )
            conn.commit()
        finally:
            conn.close()

        conn = open_db(self.data_root)
        try:
            raw_json = conn.execute(
                "SELECT raw_json FROM transactions WHERE id = 'rotation-out'"
            ).fetchone()["raw_json"]
            self.assertIn('"forged": true', raw_json)
            self.assertNotEqual(
                canonical_graph_hash(raw_json),
                conn.execute(
                    "SELECT graph_hash FROM chain_observation_provenance WHERE transaction_id = 'rotation-out'"
                ).fetchone()[0],
            )
            self.assertFalse(self._authority(conn)["rotation-out"])
        finally:
            conn.close()


if __name__ == "__main__":
    unittest.main()
