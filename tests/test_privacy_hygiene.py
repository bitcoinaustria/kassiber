import json
import tempfile
import unittest
from pathlib import Path

from kassiber.core.privacy_hygiene import build_privacy_hygiene_snapshot
from kassiber.db import open_db, set_setting


NOW = "2026-07-01T12:00:00Z"
P2WPKH_SCRIPT = "0014" + ("11" * 20)
P2WPKH_SCRIPT_2 = "0014" + ("12" * 20)
P2PKH_SCRIPT = "76a914" + ("22" * 20) + "88ac"
P2TR_SCRIPT = "5120" + ("33" * 32)
OP_RETURN_SCRIPT = "6a" + "04" + "74657374"


class PrivacyHygieneTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-privacy-hygiene-")
        self.conn = open_db(Path(self._tmp.name) / "data")
        self._bootstrap_book()

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def _bootstrap_book(self):
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws", "Main", NOW),
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            )
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("pf", "ws", "Default", "EUR", "generic", 365, "FIFO", NOW),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            )
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("wal", "ws", "pf", None, "Treasury", "descriptor", "{}", NOW),
        )
        set_setting(self.conn, "context_workspace", "ws")
        set_setting(self.conn, "context_profile", "pf")

    def _insert_transaction(
        self,
        *,
        tx_id: str,
        external_id: str,
        raw_json: dict,
        privacy_boundary: str | None = None,
        direction: str = "outbound",
        wallet_id: str = "wal",
    ):
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, confirmed_at, direction, asset,
                amount, fee, privacy_boundary, kind, description, raw_json,
                created_at
            )
            VALUES(
                ?, 'ws', 'pf', ?, ?, ?, ?, ?, ?, 'BTC',
                ?, ?, ?, ?, ?, ?, ?
            )
            """,
            (
                tx_id,
                wallet_id,
                external_id,
                f"fp-{tx_id}",
                NOW,
                NOW,
                direction,
                100_000_000,
                1_000_000,
                privacy_boundary,
                "withdrawal" if direction == "outbound" else "deposit",
                "Synced",
                json.dumps(raw_json, sort_keys=True),
                NOW,
            ),
        )

    def _insert_utxo(
        self,
        *,
        utxo_id: str,
        txid: str,
        vout: int,
        sats: int,
        address: str,
        script: str,
        spent_at: str | None = None,
        anonymity_score: int | None = None,
    ):
        self.conn.execute(
            """
            INSERT INTO wallet_utxos(
                id, workspace_id, profile_id, wallet_id, backend_name,
                backend_kind, chain, network, asset, amount, txid, vout,
                outpoint, confirmation_status, confirmations, block_height,
                block_time, address, script_pubkey, address_label,
                branch_label, branch_index, address_index, anonymity_score,
                spent_by, excluded_from_coinjoin, key_state, anon_history_json,
                first_seen_at, last_seen_at, spent_at, raw_json
            )
            VALUES(
                ?, 'ws', 'pf', 'wal', 'mempool',
                'esplora', 'bitcoin', 'main', 'BTC', ?, ?, ?,
                ?, 'confirmed', 12, 880000,
                ?, ?, ?, '', '', NULL, NULL, ?,
                NULL, NULL, '', '[]', ?, ?, ?, '{}'
            )
            """,
            (
                utxo_id,
                sats * 1000,
                txid,
                vout,
                f"{txid}:{vout}",
                NOW,
                address,
                script,
                anonymity_score,
                NOW,
                NOW,
                spent_at,
            ),
        )

    def test_phase1_transaction_tells_are_scored_without_leaking_addresses(self):
        input_a = "a" * 64
        input_b = "b" * 64
        spend_txid = "c" * 64
        self._insert_utxo(
            utxo_id="in-a",
            txid=input_a,
            vout=0,
            sats=150_000,
            address="bc1qinputreused000000000000000000000000000000",
            script=P2WPKH_SCRIPT,
            spent_at=NOW,
        )
        self._insert_utxo(
            utxo_id="in-b",
            txid=input_b,
            vout=1,
            sats=75_000,
            address="bc1qinputreused000000000000000000000000000000",
            script=P2WPKH_SCRIPT_2,
            spent_at=NOW,
        )
        self._insert_utxo(
            utxo_id="change",
            txid=spend_txid,
            vout=1,
            sats=124_000,
            address="bc1qchange0000000000000000000000000000000000",
            script=P2WPKH_SCRIPT,
        )
        self._insert_transaction(
            tx_id="spend",
            external_id=spend_txid,
            raw_json={
                "txid": spend_txid,
                "version": 2,
                "locktime": 880000,
                "fee": 1000,
                "vsize": 100,
                "vin": [
                    {
                        "txid": input_a,
                        "vout": 0,
                        "sequence": 0xFFFFFFFD,
                        "witness": ["00", "11"],
                        "prevout": {
                            "value": 150_000,
                            "scriptpubkey": P2WPKH_SCRIPT,
                            "scriptpubkey_address": "bc1qinputreused000000000000000000000000000000",
                        },
                    },
                    {
                        "txid": input_b,
                        "vout": 1,
                        "sequence": 0xFFFFFFFD,
                        "witness": ["00", "11"],
                        "prevout": {
                            "value": 75_000,
                            "scriptpubkey": P2WPKH_SCRIPT_2,
                            "scriptpubkey_address": "bc1qinputreused000000000000000000000000000000",
                        },
                    },
                ],
                "vout": [
                    {
                        "n": 0,
                        "value": 100_000,
                        "scriptpubkey": P2PKH_SCRIPT,
                        "scriptpubkey_address": "1RoundLegacyAddressxxxxxxxxxxxxxxxx",
                    },
                    {
                        "n": 1,
                        "value": 124_000,
                        "scriptpubkey": P2WPKH_SCRIPT,
                        "scriptpubkey_address": "bc1qchange0000000000000000000000000000000000",
                    },
                    {
                        "n": 2,
                        "value": 0,
                        "scriptpubkey": OP_RETURN_SCRIPT,
                        "scriptpubkey_type": "op_return",
                    },
                ],
            },
        )

        snapshot = build_privacy_hygiene_snapshot(self.conn, {"limit": 5})

        self.assertLess(snapshot["summary"]["score"], 50)
        tx = snapshot["transactions"][0]
        self.assertEqual(tx["state"], "full")
        self.assertLess(tx["score"], 50)
        codes = {finding["code"] for finding in tx["top_findings"]}
        self.assertIn("common_input_ownership", codes)
        self.assertIn("op_return_metadata", codes)
        self.assertIn("round_output_amount", codes)
        self.assertEqual(snapshot["coverage"]["transaction_full"], 1)
        self.assertEqual(snapshot["wallets"][0]["address"]["reused_address_count"], 1)

        encoded = json.dumps(snapshot, sort_keys=True)
        self.assertNotIn("bc1qinputreused", encoded)
        self.assertNotIn(P2WPKH_SCRIPT, encoded)
        self.assertNotIn(P2PKH_SCRIPT, encoded)

    def test_graphless_import_is_explicitly_not_analysable(self):
        self._insert_transaction(
            tx_id="csv-row",
            external_id="csv-external-id",
            direction="inbound",
            raw_json={"source": "csv", "label": "exchange export"},
        )

        snapshot = build_privacy_hygiene_snapshot(self.conn)

        self.assertIsNone(snapshot["transactions"][0]["score"])
        self.assertEqual(snapshot["transactions"][0]["state"], "not_analysable")
        self.assertEqual(snapshot["coverage"]["transaction_not_analysable"], 1)
        self.assertEqual(
            snapshot["transactions"][0]["top_findings"][0]["code"],
            "transaction_not_analysable",
        )

    def test_coinjoin_evidence_raises_score_and_suppresses_cioh(self):
        txid = "d" * 64
        vin = []
        for index in range(6):
            prev_txid = f"{index + 1:064x}"
            vin.append(
                {
                    "txid": prev_txid,
                    "vout": 0,
                    "sequence": 0xFFFFFFFF,
                    "prevout": {
                        "value": 60_000,
                        "scriptpubkey": P2WPKH_SCRIPT,
                    },
                }
            )
        self._insert_transaction(
            tx_id="coinjoin",
            external_id=txid,
            privacy_boundary="coinjoin",
            raw_json={
                "txid": txid,
                "version": 2,
                "locktime": 0,
                "islikelycoinjoin": True,
                "vin": vin,
                "vout": [
                    {
                        "n": index,
                        "value": 50_000,
                        "scriptpubkey": P2WPKH_SCRIPT,
                    }
                    for index in range(6)
                ],
            },
        )

        snapshot = build_privacy_hygiene_snapshot(self.conn)

        tx = snapshot["transactions"][0]
        self.assertGreaterEqual(tx["score"], 90)
        codes = {finding["code"] for finding in tx["top_findings"]}
        self.assertIn("coinjoin_pattern", codes)
        self.assertNotIn("common_input_ownership", codes)


if __name__ == "__main__":
    unittest.main()
