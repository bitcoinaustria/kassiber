import json
import sqlite3
import unittest

from kassiber.core.source_funds_assembly import build_owned_outpoint_index


class SourceFundsOwnedOutpointIndexTests(unittest.TestCase):
    def test_missing_legacy_network_uses_chain_default(self):
        txid = "10" * 32
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE wallet_utxos (
                profile_id TEXT, wallet_id TEXT, chain TEXT, asset TEXT,
                txid TEXT, vout INTEGER, amount INTEGER, branch_label TEXT,
                spent_by TEXT
            )
            """
        )
        conn.execute(
            """
            INSERT INTO wallet_utxos VALUES(
                'profile', 'wallet', 'bitcoin', 'BTC', ?, 0, 100,
                'receive', NULL
            )
            """,
            (txid,),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()
        self.assertEqual(list(index), [("bitcoin", "main", txid, 0)])

    def test_identical_outpoints_on_different_networks_stay_separate(self):
        txid = "20" * 32
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE wallet_utxos (
                profile_id TEXT, wallet_id TEXT, chain TEXT, network TEXT,
                asset TEXT, txid TEXT, vout INTEGER, amount INTEGER,
                branch_label TEXT, spent_by TEXT
            )
            """
        )
        conn.executemany(
            """
            INSERT INTO wallet_utxos VALUES(
                'profile', ?, 'bitcoin', ?, 'BTC', ?, 0, 100,
                'receive', NULL
            )
            """,
            (("main-wallet", "main", txid), ("regtest-wallet", "regtest", txid)),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()
        self.assertEqual(
            set(index),
            {("bitcoin", "main", txid, 0), ("bitcoin", "regtest", txid, 0)},
        )

    def test_liquid_requires_consensus_asset_identity(self):
        txid = "21" * 32
        asset_id = "ab" * 32
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE wallet_utxos (
                profile_id TEXT, wallet_id TEXT, chain TEXT, network TEXT,
                asset TEXT, txid TEXT, vout INTEGER, amount INTEGER,
                branch_label TEXT, spent_by TEXT, raw_json TEXT
            )
            """
        )
        conn.executemany(
            """
            INSERT INTO wallet_utxos VALUES(
                'profile', ?, 'liquid', 'liquidv1', 'LBTC', ?, ?, 100,
                'receive', NULL, ?
            )
            """,
            (
                ("known", txid, 0, json.dumps({"asset_id": asset_id})),
                ("unknown", txid, 1, "{}"),
            ),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()
        self.assertEqual(set(index), {("liquid", "liquidv1", txid, 0)})
        self.assertEqual(index[("liquid", "liquidv1", txid, 0)]["asset_identity"], asset_id)
