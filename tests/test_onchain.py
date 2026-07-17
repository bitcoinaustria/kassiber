import json
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from kassiber.core.onchain import (
    merge_ownership_txs,
    parse_identification_legs,
    parse_ownership_tx,
    parse_valued_tx,
    parse_vin_outpoints,
)


class StoredOnchainParserTests(unittest.TestCase):
    def test_esplora_and_electrum_scripts_share_one_parser(self):
        raw = json.dumps(
            {
                "txid": "aa" * 32,
                "vin": [
                    {
                        "txid": "BB" * 32,
                        "vout": 1,
                        "prevout": {"scriptpubkey": "0014aa"},
                    }
                ],
                "vout": [
                    {"n": 0, "scriptpubkey": "0014bb", "value": 1234},
                    {"n": 1, "script_hex": "0014cc", "value_sats": 5678},
                ],
            }
        )
        identify = parse_identification_legs(raw)
        valued = parse_valued_tx(raw)
        self.assertEqual(identify["inputs"][0]["outpoint"], f"{'bb' * 32}:1")
        self.assertEqual(
            [output["script"] for output in identify["outputs"]],
            ["0014bb", "0014cc"],
        )
        self.assertEqual(
            [output["value_sats"] for output in valued["outputs"]],
            [1234, 5678],
        )

    def test_bitcoin_core_script_and_decimal_value_share_the_parser(self):
        raw = {
            "txid": "aa" * 32,
            "vin": [
                {
                    "txid": "bb" * 32,
                    "vout": 0,
                    "prevout": {
                        "value": 0.00002000,
                        "scriptPubKey": {"hex": "0014AA"},
                    },
                }
            ],
            "vout": [
                {
                    "n": 0,
                    "value": 0.00001900,
                    "scriptPubKey": {"hex": "0014BB"},
                }
            ],
        }
        parsed = parse_ownership_tx(raw)
        self.assertEqual(parsed["inputs"][0]["script"], "0014AA")
        self.assertEqual(parsed["inputs"][0]["value_sats"], 2000)
        self.assertEqual(parsed["outputs"][0]["script"], "0014BB")
        self.assertEqual(parsed["outputs"][0]["value_sats"], 1900)

    def test_decimal_value_is_btc_and_liquid_fee_output_is_preserved(self):
        valued = parse_valued_tx(
            {
                "vin": [],
                "vout": [
                    {"scriptpubkey": "0014aa", "value": "0.00001000"},
                    {"scriptpubkey": "", "value": 250},
                ],
            }
        )
        self.assertEqual(
            [(row["script"], row["value_sats"]) for row in valued["outputs"]],
            [("0014aa", 1000), ("", 250)],
        )

    def test_lineage_accepts_nested_core_shape_and_skips_coinbase(self):
        self.assertEqual(
            parse_vin_outpoints(
                {
                    "tx": {
                        "vin": [
                            {"txid": "0" * 64, "vout": 0},
                            {"txid": "AB" * 32, "vout": 2},
                            {"txid": "bad", "vout": -1},
                        ]
                    }
                }
            ),
            [("ab" * 32, 2)],
        )

    def test_valued_parser_rejects_script_only_legacy_graph(self):
        raw = {"vin": [], "vout": [{"script_hex": "0014aa"}]}
        self.assertIsNotNone(parse_identification_legs(raw))
        self.assertIsNone(parse_valued_tx(raw))

    def test_liquid_prevout_value_and_asset_identity_survive_nested_shape(self):
        asset_id = "11" * 32
        raw = {
            "chain": "liquid",
            "network": "liquidv1",
            "txid": "aa" * 32,
            "tx": {
                "txid": "aa" * 32,
                "vin": [{"txid": "bb" * 32, "vout": 3}],
                "vout": [{"n": 0, "scriptpubkey": "0014bb"}],
            },
            "vin": [
                {
                    "txid": "bb" * 32,
                    "vout": 3,
                    "prevout": {
                        "scriptpubkey": "0014aa",
                        "value_sats": 50_000,
                        "asset_id": asset_id,
                        "asset": "LBTC",
                    },
                }
            ],
            "vout": [
                {
                    "n": 0,
                    "scriptpubkey": "0014bb",
                    "value_sats": 49_900,
                    "asset_id": asset_id,
                    "asset": "LBTC",
                }
            ],
            "component": {"asset_id": asset_id, "asset": "LBTC"},
        }
        parsed = parse_ownership_tx(raw)
        self.assertEqual(parsed["chain"], "liquid")
        self.assertEqual(parsed["network"], "liquidv1")
        self.assertEqual(parsed["inputs"][0]["value_sats"], 50_000)
        self.assertEqual(parsed["inputs"][0]["asset_id"], asset_id)
        self.assertEqual(parsed["outputs"][0]["value_sats"], 49_900)
        self.assertEqual(parsed["outputs"][0]["asset_id"], asset_id)

    def test_injected_liquid_value_resolver_keeps_secret_context_outside_parser(self):
        asset_id = "22" * 32
        raw = {
            "chain": "liquid",
            "network": "elementsregtest",
            "txid": "cc" * 32,
            "vin": [{"txid": "dd" * 32, "vout": 1}],
            "vout": [{"n": 0, "scriptpubkey": "0014cc"}],
            "component": {"asset_id": asset_id, "asset": "LBTC"},
        }

        def resolve(_raw, _parsed):
            return {
                "txid": "cc" * 32,
                "chain": "liquid",
                "network": "elementsregtest",
                "inputs": [],
                "outputs": [
                    {
                        "n": 0,
                        "script": "0014cc",
                        "value_sats": 1234,
                        "asset_id": asset_id,
                        "asset": "LBTC",
                    }
                ],
            }

        parsed = parse_valued_tx(raw, value_resolver=resolve)
        self.assertEqual(parsed["outputs"][0]["value_sats"], 1234)
        self.assertEqual(parsed["outputs"][0]["asset_id"], asset_id)

    def test_liquid_raw_hex_is_decoded_locally_for_public_structure(self):
        asset_id = "33" * 32
        decoded = SimpleNamespace(
            vin=[SimpleNamespace(txid=bytes.fromhex("44" * 32), vout=2)],
            vout=[
                SimpleNamespace(
                    script_pubkey=SimpleNamespace(data=bytes.fromhex("0014abcd")),
                    is_blinded=False,
                    value=250,
                    asset=bytes(reversed(bytes.fromhex(asset_id))),
                ),
                SimpleNamespace(
                    script_pubkey=SimpleNamespace(data=b""),
                    is_blinded=False,
                    value=5,
                    asset=bytes(reversed(bytes.fromhex(asset_id))),
                ),
            ],
        )
        with patch(
            "kassiber.wallet_descriptors.decode_liquid_transaction",
            return_value=decoded,
        ):
            parsed = parse_ownership_tx(
                {
                    "chain": "liquid",
                    "network": "elementsregtest",
                    "txid": "55" * 32,
                    "raw_hex": "00",
                    "component": {"asset_id": asset_id, "asset": "LBTC"},
                }
            )
        self.assertEqual(parsed["inputs"][0]["outpoint"], f"{'44' * 32}:2")
        self.assertEqual(parsed["outputs"][0]["script"], "0014abcd")
        self.assertEqual(parsed["outputs"][0]["value_sats"], 250)
        self.assertEqual(parsed["outputs"][0]["asset_id"], asset_id)
        self.assertEqual(parsed["outputs"][1]["role"], "fee")

    def test_merge_flags_conflicting_liquid_asset_evidence(self):
        base = {
            "txid": "ee" * 32,
            "chain": "liquid",
            "network": "liquidv1",
            "inputs": [],
            "outputs": [
                {
                    "n": 0,
                    "script": "0014ee",
                    "value_sats": 100,
                    "asset_id": "11" * 32,
                }
            ],
        }
        conflicting = {
            **base,
            "outputs": [
                {
                    "n": 0,
                    "script": "0014ee",
                    "value_sats": 100,
                    "asset_id": "22" * 32,
                }
            ],
        }
        merged = merge_ownership_txs([base, conflicting])
        self.assertIn("output:0.asset_id", merged["evidence_conflicts"])

    def test_merge_owned_observation_refines_external_role_without_conflict(self):
        base = {
            "txid": "ef" * 32,
            "chain": "liquid",
            "network": "liquidv1",
            "inputs": [],
            "outputs": [
                {
                    "n": 0,
                    "script": "0014ef",
                    "value_sats": 100,
                    "asset_id": "11" * 32,
                    "role": "external",
                }
            ],
        }
        owned = {
            **base,
            "outputs": [{**base["outputs"][0], "role": "owned"}],
        }

        merged = merge_ownership_txs([base, owned])

        self.assertEqual(merged["outputs"][0]["role"], "owned")
        self.assertEqual(merged["evidence_conflicts"], [])

    def test_merge_deduplicates_position_only_and_outpoint_input_observations(self):
        outpoint = f"{'aa' * 32}:1"
        position_only = {
            "txid": "bb" * 32,
            "chain": "bitcoin",
            "network": "main",
            "inputs": [{"script": "0014aa", "value_sats": 100}],
            "outputs": [],
        }
        exact = {
            **position_only,
            "inputs": [
                {
                    "outpoint": outpoint,
                    "script": "0014aa",
                    "value_sats": 100,
                }
            ],
        }

        for observations in ((position_only, exact), (exact, position_only)):
            with self.subTest(order=bool(observations[0]["inputs"][0].get("outpoint"))):
                merged = merge_ownership_txs(observations)
                self.assertEqual(len(merged["inputs"]), 1)
                self.assertEqual(merged["inputs"][0]["outpoint"], outpoint)


if __name__ == "__main__":
    unittest.main()
