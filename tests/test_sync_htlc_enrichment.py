"""Sync-side opportunistic HTLC enrichment for BTC + Liquid.

Pins three landings:

1. Esplora BTC sync surfaces ``payment_hash`` whenever a vin's witness
   exposes a Boltz v1 HTLC claim (preimage + redeem script).
2. Electrum BTC sync surfaces the same field — its raw-transaction
   decoder now preserves witness items as hex so the parser can run.
3. The Liquid descriptor sync helper extracts ``payment_hash`` from
   embit-shaped ``TxInWitness`` objects.

The redeem script and preimage are constructed synthetically from
``tests.test_htlc_parser``'s template helper so each test pins exact
byte shapes the parser is expected to handle.
"""

import hashlib
import unittest
from types import SimpleNamespace

from embit import hashes as _embit_hashes

from kassiber.core.sync_backends import (
    _esplora_witness_items,
    _extract_payment_hash_from_witnesses,
    _extract_refund_funding_txid,
    _liquid_witness_items,
    _payment_hash_fields,
    _swap_refund_fields,
    decode_raw_transaction,
    liquid_input_txid,
    record_from_bitcoin_esplora_tx,
    record_from_electrum_tx,
)


_PREIMAGE = bytes([0xAB] * 32)
_HASHLOCK160 = _embit_hashes.hash160(_PREIMAGE)
_PAYMENT_HASH = hashlib.sha256(_PREIMAGE).hexdigest()
_PUBKEY_CLAIM = bytes.fromhex("02" + "11" * 32)
_PUBKEY_REFUND = bytes.fromhex("03" + "22" * 32)
_CLTV_PUSH = bytes([0x03, 0x00, 0x00, 0x10])


def _redeem_script():
    return b"".join(
        [
            bytes([0xA9, 0x14]) + _HASHLOCK160 + bytes([0x87]),
            bytes([0x63, 0x21]) + _PUBKEY_CLAIM,
            bytes([0x67]) + _CLTV_PUSH + bytes([0xB1, 0x75]),
            bytes([0x21]) + _PUBKEY_REFUND + bytes([0x68, 0xAC]),
        ]
    )


def _claim_witness_hex():
    return [
        "3045" + "00" * 70,
        _PREIMAGE.hex(),
        "01",
        _redeem_script().hex(),
    ]


def _refund_witness_hex():
    # CLTV timeout spend: empty selector where a claim would push the preimage.
    return [
        "3045" + "00" * 70,
        "",
        _redeem_script().hex(),
    ]


class EsploraEnrichmentTests(unittest.TestCase):
    def test_witness_items_decoded_to_bytes(self):
        items = _esplora_witness_items({"witness": _claim_witness_hex()})
        self.assertEqual(items[1], _PREIMAGE)
        self.assertEqual(items[-1], _redeem_script())

    def test_missing_or_bad_witness_yields_empty(self):
        self.assertEqual(_esplora_witness_items({}), [])
        self.assertEqual(_esplora_witness_items({"witness": ["zz"]}), [])

    def test_extract_payment_hash_from_iterable(self):
        result = _extract_payment_hash_from_witnesses([_esplora_witness_items({"witness": _claim_witness_hex()})])
        self.assertEqual(result, _PAYMENT_HASH)

    def test_payment_hash_fields_short_circuits_on_none(self):
        self.assertEqual(_payment_hash_fields(None), {})
        self.assertEqual(
            _payment_hash_fields(_PAYMENT_HASH),
            {"payment_hash": _PAYMENT_HASH, "payment_hash_source": "chain_script"},
        )

    def test_bitcoin_esplora_record_carries_payment_hash(self):
        spk = "0020" + "ab" * 32  # synthetic tracked P2WSH scriptpubkey
        tx = {
            "txid": "tx-claim-1",
            "vin": [
                {
                    "prevout": {"scriptpubkey": spk, "value": 100_000},
                    "witness": _claim_witness_hex(),
                }
            ],
            "vout": [{"scriptpubkey": "0014" + "cd" * 20, "value": 99_500}],
            "fee": 500,
            "status": {"block_time": 1_741_000_000},
        }
        record = record_from_bitcoin_esplora_tx(tx, tracked_scripts={spk}, backend_name="esplora")
        self.assertIsNotNone(record)
        self.assertEqual(record["payment_hash"], _PAYMENT_HASH)
        self.assertEqual(record["payment_hash_source"], "chain_script")

    def test_bitcoin_esplora_record_without_htlc_witness_has_no_field(self):
        spk = "0020" + "ab" * 32
        tx = {
            "txid": "tx-no-htlc",
            "vin": [{"prevout": {"scriptpubkey": spk, "value": 50_000}, "witness": []}],
            "vout": [{"scriptpubkey": "0014" + "cd" * 20, "value": 49_500}],
            "fee": 500,
            "status": {"block_time": 1_741_000_000},
        }
        record = record_from_bitcoin_esplora_tx(tx, tracked_scripts={spk}, backend_name="esplora")
        self.assertNotIn("payment_hash", record)


class ElectrumDecoderPreservesWitnessesTests(unittest.TestCase):
    def _build_raw_segwit_tx(self):
        # Minimal segwit tx: 1 vin (with our witness), 1 vout, locktime 0.
        version = (1).to_bytes(4, "little")
        flag = bytes([0x00, 0x01])  # segwit marker + flag
        input_count = bytes([0x01])
        prev_txid = bytes(32)
        prev_vout = (0).to_bytes(4, "little")
        script_sig = bytes([0x00])  # empty
        sequence = (0xFFFFFFFF).to_bytes(4, "little")
        output_count = bytes([0x01])
        value = (10_000).to_bytes(8, "little")
        out_script = bytes([0x16]) + bytes([0x00, 0x14]) + bytes(20)  # P2WPKH placeholder
        # Witness for the single input: 4 items.
        witness_items = [
            bytes.fromhex("3045" + "00" * 70),
            _PREIMAGE,
            bytes([0x01]),
            _redeem_script(),
        ]
        witness_blob = bytes([len(witness_items)])
        for item in witness_items:
            witness_blob += bytes([len(item)]) + item
        locktime = (0).to_bytes(4, "little")
        return (
            version
            + flag
            + input_count
            + prev_txid
            + prev_vout
            + script_sig
            + sequence
            + output_count
            + value
            + out_script
            + witness_blob
            + locktime
        ).hex()

    def test_decode_preserves_witness_as_hex_strings(self):
        raw_hex = self._build_raw_segwit_tx()
        tx = decode_raw_transaction(raw_hex)
        self.assertEqual(len(tx["vin"]), 1)
        witness = tx["vin"][0]["witness"]
        self.assertEqual(witness[1], _PREIMAGE.hex())
        self.assertEqual(witness[-1], _redeem_script().hex())

    def test_electrum_record_surfaces_payment_hash(self):
        raw_hex = self._build_raw_segwit_tx()
        tx = decode_raw_transaction(raw_hex)
        tracked_script_hex = tx["vout"][0]["script_hex"]
        prev_tx = {"vout": [{"value_sats": 10_000, "script_hex": tracked_script_hex}]}
        # Wire the synthetic vin so its prevout points at a tracked output, so
        # the record-builder considers this an outbound spend the witness rides on.
        tx["vin"][0]["txid"] = "prev-funding"
        tx["vin"][0]["vout"] = 0
        record = record_from_electrum_tx(
            "tx-claim-electrum",
            tx,
            height=850_000,
            tracked_scripts={tracked_script_hex},
            backend_name="electrum",
            tx_lookup=lambda _txid: prev_tx,
        )
        self.assertIsNotNone(record)
        self.assertEqual(record["payment_hash"], _PAYMENT_HASH)
        self.assertEqual(record["payment_hash_source"], "chain_script")


class LiquidWitnessExtractionTests(unittest.TestCase):
    def test_liquid_witness_items_pulled_from_embit_shape(self):
        # Mimic the embit attribute path: vin.witness.script_witness.items
        items = [_PREIMAGE, _redeem_script()]
        vin = SimpleNamespace(
            witness=SimpleNamespace(script_witness=SimpleNamespace(items=items))
        )
        self.assertEqual(_liquid_witness_items(vin), items)

    def test_liquid_missing_witness_yields_empty(self):
        self.assertEqual(_liquid_witness_items(SimpleNamespace()), [])
        self.assertEqual(
            _liquid_witness_items(SimpleNamespace(witness=None)), []
        )
        self.assertEqual(
            _liquid_witness_items(SimpleNamespace(witness=SimpleNamespace(script_witness=None))),
            [],
        )

    def test_extract_payment_hash_from_liquid_witnesses(self):
        items = [
            bytes.fromhex("3045" + "00" * 70),
            _PREIMAGE,
            bytes([0x01]),
            _redeem_script(),
        ]
        vin = SimpleNamespace(
            witness=SimpleNamespace(script_witness=SimpleNamespace(items=items))
        )
        result = _extract_payment_hash_from_witnesses([_liquid_witness_items(vin)])
        self.assertEqual(result, _PAYMENT_HASH)


class RefundLinkEnrichmentTests(unittest.TestCase):
    def test_swap_refund_fields_short_circuits_on_none(self):
        self.assertEqual(_swap_refund_fields(None), {})
        self.assertEqual(
            _swap_refund_fields("ab" * 32),
            {"swap_refund_funding_txid": "ab" * 32},
        )

    def test_extract_refund_funding_txid_from_refund_vin(self):
        vins = [
            {"txid": "funding-lockup", "vout": 0, "witness": _refund_witness_hex()},
        ]
        self.assertEqual(
            _extract_refund_funding_txid(vins, _esplora_witness_items),
            "funding-lockup",
        )

    def test_extract_refund_funding_txid_liquid_embit_vin(self):
        # Liquid vins are embit objects: witness via script_witness.items and
        # prevout txid via liquid_input_txid, not the dict-shaped defaults.
        items = [bytes.fromhex("3045" + "00" * 70), b"", _redeem_script()]
        vin = SimpleNamespace(
            txid="aa" * 32,
            witness=SimpleNamespace(script_witness=SimpleNamespace(items=items)),
        )
        self.assertEqual(
            _extract_refund_funding_txid(
                [vin], _liquid_witness_items, prev_txid_fn=liquid_input_txid
            ),
            "aa" * 32,
        )

    def test_extract_refund_funding_txid_ignores_claim_vin(self):
        # A claim reveals the preimage and is handled by the payment_hash path,
        # not the refund-link path.
        vins = [
            {"txid": "funding-lockup", "vout": 0, "witness": _claim_witness_hex()},
        ]
        self.assertIsNone(_extract_refund_funding_txid(vins, _esplora_witness_items))

    def test_esplora_inbound_refund_record_carries_link(self):
        htlc_spk = "0020" + "ab" * 32  # the swap HTLC output, NOT a tracked script
        wallet_spk = "0014" + "cd" * 20  # the refund lands back on the wallet
        tx = {
            "txid": "tx-refund-1",
            "vin": [
                {
                    "txid": "funding-lockup-txid",
                    "vout": 0,
                    "prevout": {"scriptpubkey": htlc_spk, "value": 100_000},
                    "witness": _refund_witness_hex(),
                }
            ],
            "vout": [{"scriptpubkey": wallet_spk, "value": 99_500}],
            "fee": 500,
            "status": {"block_time": 1_741_000_000},
        }
        record = record_from_bitcoin_esplora_tx(
            tx, tracked_scripts={wallet_spk}, backend_name="esplora"
        )
        self.assertIsNotNone(record)
        self.assertEqual(record["direction"], "inbound")
        self.assertEqual(record["swap_refund_funding_txid"], "funding-lockup-txid")
        self.assertNotIn("payment_hash", record)

    def test_esplora_claim_record_has_no_refund_link(self):
        htlc_spk = "0020" + "ab" * 32
        wallet_spk = "0014" + "cd" * 20
        tx = {
            "txid": "tx-claim-1",
            "vin": [
                {
                    "txid": "funding-lockup-txid",
                    "vout": 0,
                    "prevout": {"scriptpubkey": htlc_spk, "value": 100_000},
                    "witness": _claim_witness_hex(),
                }
            ],
            "vout": [{"scriptpubkey": wallet_spk, "value": 99_500}],
            "fee": 500,
            "status": {"block_time": 1_741_000_000},
        }
        record = record_from_bitcoin_esplora_tx(
            tx, tracked_scripts={wallet_spk}, backend_name="esplora"
        )
        self.assertIsNotNone(record)
        self.assertEqual(record["payment_hash"], _PAYMENT_HASH)
        self.assertNotIn("swap_refund_funding_txid", record)


if __name__ == "__main__":
    unittest.main()
