"""A transaction is checked against its stored graph, locally and without addresses."""

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.core import transaction_verify
from kassiber.core.sync_backends import address_to_scriptpubkey
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError

NOW = "2026-01-01T00:00:00Z"
ADDR_A = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
ADDR_B = "bc1q0xcqpzrky6eff2g52qdye53xkk9jxkvrh6yhyw"
ADDR_C = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
SCRIPT_A = address_to_scriptpubkey(ADDR_A).hex()
SCRIPT_B = address_to_scriptpubkey(ADDR_B).hex()
SCRIPT_C = address_to_scriptpubkey(ADDR_C).hex()
PREV_TXID = "a" * 64
TXID = "b" * 64


def _raw(fee_sats=1_000_000, *, drop_change=False):
    vout = [
        {"n": 0, "scriptpubkey": SCRIPT_B, "scriptpubkey_address": ADDR_B, "value": 50_000_000},
        {"n": 1, "scriptpubkey": SCRIPT_C, "scriptpubkey_address": ADDR_C, "value": 9_000_000},
    ]
    if drop_change:
        vout = vout[:1]
    return {
        "txid": TXID,
        "fee": fee_sats,
        "vin": [
            {
                "txid": PREV_TXID,
                "vout": 0,
                "prevout": {"scriptpubkey": SCRIPT_A, "scriptpubkey_address": ADDR_A, "value": 60_000_000},
            }
        ],
        "vout": vout,
    }


class TransactionVerifyTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-verify-")
        self.addCleanup(self.tmp.cleanup)
        self.conn = open_db(Path(self.tmp.name) / "data")
        self.addCleanup(self.conn.close)
        conn = self.conn
        conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Main', ?)", (NOW,))
        conn.execute(
            """
            INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country,
                                 tax_long_term_days, gains_algorithm, created_at)
            VALUES('pf', 'ws', 'Book', 'EUR', 'generic', 365, 'FIFO', ?)
            """,
            (NOW,),
        )
        for wallet_id, label, kind in (
            ("cold", "Cold", "custom"),
            ("hot", "Hot", "custom"),
            ("exchange", "Exchange", "csv"),
        ):
            conn.execute(
                """
                INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
                VALUES(?, 'ws', 'pf', ?, ?, '{}', ?)
                """,
                (wallet_id, label, kind, NOW),
            )
        # Ownership evidence: Cold's spent input and change, Hot's receipt.
        self._utxo("cold", ADDR_A, PREV_TXID, 0, 60_000_000, "receive")
        self._utxo("cold", ADDR_C, TXID, 1, 9_000_000, "change")
        self._utxo("hot", ADDR_B, TXID, 0, 50_000_000, "receive")
        set_setting(conn, "context_workspace", "ws")
        set_setting(conn, "context_profile", "pf")
        conn.commit()

    def _utxo(self, wallet_id, address, txid, vout, sats, branch):
        self.conn.execute(
            """
            INSERT INTO wallet_utxos(
                id, workspace_id, profile_id, wallet_id, chain, network, asset,
                amount, txid, vout, outpoint, confirmation_status, address,
                branch_label, branch_index, address_index, first_seen_at, last_seen_at
            ) VALUES(?, 'ws', 'pf', ?, 'bitcoin', 'main', 'BTC', ?, ?, ?, ?, 'confirmed', ?, ?, ?, 0, ?, ?)
            """,
            (
                f"utxo-{txid}-{vout}", wallet_id, sats * 1000, txid, vout,
                f"{txid}:{vout}", address, branch, 1 if branch == "change" else 0, NOW, NOW,
            ),
        )

    def _tx(self, tx_id, wallet_id, direction, amount_msat, raw, *, fee_msat=0, external_id=TXID):
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency, kind, raw_json, created_at
            ) VALUES(?, 'ws', 'pf', ?, ?, ?, ?, ?, 'BTC', ?, ?, 'EUR', ?, ?, ?)
            """,
            (
                tx_id, wallet_id, external_id, f"fp-{tx_id}", NOW, direction, amount_msat,
                fee_msat, "withdrawal" if direction == "outbound" else "deposit",
                json.dumps(raw), NOW,
            ),
        )
        self.conn.commit()

    def _verify(self, ref="send"):
        return transaction_verify.verify_transaction(self.conn, "pf", ref)

    def _both_legs(self, *, hot_amount=50_000_000_000, raw=None):
        raw = raw or _raw()
        self._tx("send", "cold", "outbound", 50_000_000_000, raw, fee_msat=1_000_000_000)
        self._tx("receive", "hot", "inbound", hot_amount, raw)

    def test_complete_transaction_verifies_with_keychains_and_no_addresses(self):
        self._both_legs()

        result = self._verify()

        self.assertEqual(result["status"], "ok")
        self.assertEqual(result["reasons"], [])
        self.assertFalse(result["network_contacted"])
        conservation = result["conservation"]
        self.assertEqual(conservation["status"], "ok")
        self.assertEqual(conservation["inputs_msat"], 60_000_000_000)
        self.assertEqual(conservation["outputs_msat"], 59_000_000_000)
        self.assertEqual(conservation["fee_expected_msat"], 1_000_000_000)
        self.assertEqual(conservation["fee_recorded_msat"], 1_000_000_000)
        legs = {leg["wallet"]: leg for leg in result["legs"]}
        self.assertEqual(legs["Cold"]["expected_delta_msat"], -51_000_000_000)
        self.assertEqual(legs["Cold"]["rows"][0]["recorded_delta_msat"], -51_000_000_000)
        self.assertEqual(legs["Hot"]["expected_delta_msat"], 50_000_000_000)
        self.assertEqual({leg["status"] for leg in legs.values()}, {"ok"})
        outputs = {node["index"]: node for node in result["outputs"]}
        self.assertEqual((outputs[0]["wallet"], outputs[0]["keychain"]), ("Hot", "receive"))
        self.assertEqual((outputs[1]["wallet"], outputs[1]["keychain"]), ("Cold", "change"))
        self.assertEqual(outputs[1]["amount_msat"], 9_000_000_000)
        serialized = json.dumps(result)
        for secret in (ADDR_A, ADDR_B, ADDR_C, SCRIPT_A, SCRIPT_B, SCRIPT_C):
            self.assertNotIn(secret, serialized)

    def test_a_missing_imported_leg_is_a_mismatch(self):
        self._tx("send", "cold", "outbound", 50_000_000_000, _raw(), fee_msat=1_000_000_000)

        result = self._verify()

        self.assertEqual(result["status"], "mismatch")
        self.assertIn("leg:missing_imported_leg", result["reasons"])
        hot = next(leg for leg in result["legs"] if leg["wallet"] == "Hot")
        self.assertEqual(hot["rows"], [])
        self.assertEqual(hot["expected_delta_msat"], 50_000_000_000)

    def test_a_leg_booked_with_the_wrong_amount_is_a_mismatch(self):
        self._both_legs(hot_amount=40_000_000_000)

        result = self._verify("receive")

        self.assertEqual(result["status"], "mismatch")
        hot = next(leg for leg in result["legs"] if leg["wallet"] == "Hot")
        self.assertEqual(hot["reason"], "amount_differs")
        self.assertEqual(hot["rows"][0]["recorded_delta_msat"], 40_000_000_000)

    def test_a_graph_missing_an_output_fails_conservation(self):
        # The importer stored a graph without the change output: inputs minus
        # outputs no longer equal the fee it recorded.
        self._both_legs(raw=_raw(drop_change=True))

        result = self._verify()

        self.assertEqual(result["status"], "mismatch")
        self.assertEqual(result["conservation"]["reason"], "fee_differs")
        self.assertEqual(result["conservation"]["fee_expected_msat"], 10_000_000_000)

    def test_an_exchange_record_of_the_txid_is_reported_not_failed(self):
        self._both_legs()
        self._tx("exchange-row", "exchange", "outbound", 50_000_000_000, {})

        result = self._verify()

        self.assertEqual(result["status"], "ok")
        self.assertEqual(
            [(row["transaction_id"], row["status"]) for row in result["unexplained_rows"]],
            [("exchange-row", "external_record")],
        )

    def test_a_row_without_a_stored_graph_is_unverifiable_not_ok(self):
        self._tx("csv", "cold", "outbound", 1_000, {}, external_id="csv-1")

        result = self._verify("csv")

        self.assertEqual(result["status"], "unverifiable")
        self.assertEqual(result["reasons"], ["graphless_import"])
        self.assertIsNone(result["conservation"])
        self.assertIsNone(result["legs"])

    def test_unknown_transaction_is_an_error(self):
        with self.assertRaises(AppError) as raised:
            self._verify("nope")
        self.assertEqual(raised.exception.code, "not_found")

    def test_the_ai_tool_returns_the_same_verdict_without_addresses(self):
        from kassiber.mcp.tools import run_tool

        self._both_legs()
        data = run_tool(
            self.conn,
            data_root=str(Path(self.tmp.name) / "data"),
            runtime_config={},
            name="transactions_verify",
            arguments={"transaction": TXID},
            workspace="ws",
            profile="pf",
            project_name=None,
        )["data"]

        self.assertEqual(data["status"], "ok")
        self.assertIn("keychain", data["definitions"])
        serialized = json.dumps(data)
        for secret in (ADDR_A, ADDR_B, ADDR_C, SCRIPT_A, SCRIPT_B, SCRIPT_C):
            self.assertNotIn(secret, serialized)

    def test_verification_never_contacts_the_network(self):
        self._both_legs()
        with patch(
            "kassiber.core.transaction_graph._fetch_reference_graph_from_backend",
            side_effect=AssertionError("network"),
        ):
            self.assertEqual(self._verify()["status"], "ok")


if __name__ == "__main__":
    unittest.main()
