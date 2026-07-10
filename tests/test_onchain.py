import json
import unittest

from kassiber.core.onchain import (
    parse_identification_legs,
    parse_valued_tx,
    parse_vin_outpoints,
)


class StoredOnchainParserTests(unittest.TestCase):
    def test_esplora_and_electrum_scripts_share_one_parser(self):
        raw = json.dumps(
            {
                "txid": "aa",
                "vin": [
                    {
                        "txid": "BB",
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
        self.assertEqual(identify["inputs"][0]["outpoint"], "bb:1")
        self.assertEqual(
            [output["script"] for output in identify["outputs"]],
            ["0014bb", "0014cc"],
        )
        self.assertEqual(
            [output["value_sats"] for output in valued["outputs"]],
            [1234, 5678],
        )

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


if __name__ == "__main__":
    unittest.main()
