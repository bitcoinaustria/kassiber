"""Tests for the on-chain transaction-leg reducers and verify-backend guard.

These exercise the pure parsing/validation seams of the ``wallets identify
--verify-on-chain`` path without any network: the leg builders are fed plain
dicts / fake decoded objects, and the backend guard is asserted to raise before
any fetch happens.
"""

import types
import unittest

from kassiber.core import sync_backends
from kassiber.errors import AppError


class EsploraLegsTests(unittest.TestCase):
    def test_esplora_tx_yields_input_and_output_scripts(self):
        tx = {
            "vin": [{"txid": "AA", "vout": 0, "prevout": {"scriptpubkey": "0014aa"}}],
            "vout": [{"scriptpubkey": "0014bb"}, {"scriptpubkey": "0014cc"}],
        }
        legs = sync_backends._legs_from_esplora_tx(tx, "bitcoin")
        self.assertEqual(legs["source"], "chain")
        self.assertEqual(legs["chain"], "bitcoin")
        self.assertEqual(legs["inputs"][0], {"outpoint": "aa:0", "script": "0014aa"})
        self.assertEqual([o["script"] for o in legs["outputs"]], ["0014bb", "0014cc"])


class BitcoinDecodeLegsTests(unittest.TestCase):
    def test_decode_dict_yields_output_scripts_and_outpoints(self):
        parsed = {
            "vin": [{"txid": "AA", "vout": 1, "sequence": 0}],
            "vout": [{"n": 0, "script_hex": "0014bb"}],
        }
        legs = sync_backends._legs_from_bitcoin_tx(parsed)
        self.assertEqual(legs["chain"], "bitcoin")
        self.assertEqual(legs["inputs"][0]["outpoint"], "aa:1")
        self.assertIsNone(legs["inputs"][0]["script"])
        self.assertEqual(legs["outputs"][0]["script"], "0014bb")


class _FakeOutput:
    def __init__(self, hex_str):
        self.script_pubkey = types.SimpleNamespace(data=bytes.fromhex(hex_str))


class _FakeInput:
    def __init__(self, txid, vout):
        self.txid = txid
        self.vout = vout


class LiquidDecodeLegsTests(unittest.TestCase):
    def test_liquid_tx_outputs_visible_and_fee_is_empty(self):
        decoded = types.SimpleNamespace(
            vin=[_FakeInput("aa" * 32, 0)],
            vout=[_FakeOutput("0014bb"), _FakeOutput("")],  # second is the fee output
        )
        legs = sync_backends._legs_from_liquid_tx(decoded)
        self.assertEqual(legs["chain"], "liquid")
        self.assertEqual(legs["inputs"][0]["outpoint"], "aa" * 32 + ":0")
        self.assertEqual(legs["outputs"][0]["script"], "0014bb")
        self.assertIsNone(legs["outputs"][1]["script"])  # fee output -> None


class FetchTransactionLegsGuardTests(unittest.TestCase):
    def test_non_esplora_electrum_backend_rejected(self):
        with self.assertRaises(AppError) as ctx:
            sync_backends.fetch_transaction_legs(
                {"name": "shop", "kind": "btcpay", "url": "https://x"}, "ab" * 32
            )
        self.assertEqual(ctx.exception.code, "validation")

    def test_electrum_without_chain_rejected_before_fetch(self):
        # No chain on the backend and no hint -> refuse rather than guess the
        # decoder. Raises before any socket is opened.
        with self.assertRaises(AppError) as ctx:
            sync_backends.fetch_transaction_legs(
                {"name": "fulcrum", "kind": "electrum", "url": "ssl://x:50002"},
                "ab" * 32,
            )
        self.assertEqual(ctx.exception.code, "validation")

    def test_verify_session_esplora_yields_callable_without_connecting(self):
        backend = {"name": "mempool", "kind": "esplora", "url": "https://x"}
        with sync_backends.verify_session(backend) as fetcher:
            self.assertTrue(callable(fetcher))


if __name__ == "__main__":
    unittest.main()
