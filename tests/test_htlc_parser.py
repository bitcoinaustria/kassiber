"""HTLC parser unit tests.

Builds synthetic Boltz v1 submarine and reverse-submarine redeem scripts
plus matching claim witnesses, then verifies the parser:

* Extracts ``hashlock160`` from a fund-only script.
* Recovers ``payment_hash`` from a claim witness that reveals the preimage.
* Verifies an HTLC script against a known Lightning ``payment_hash``.
* Rejects malformed scripts, wrong-length preimages, and Taproot-shaped
  witnesses cleanly (returns ``None``, no exceptions).

The tests assemble script bytes directly so they pin the exact byte
layout the parser is expected to handle, independent of any external
Boltz fixture.
"""

import hashlib
import unittest

from embit import hashes as _embit_hashes

from kassiber.core.htlc_parser import (
    HtlcExtraction,
    extract_from_claim_witness,
    extract_from_refund_witness,
    parse_htlc_redeem_script,
    refund_funding_outpoint_from_tx_mapping,
    script_matches_payment_hash,
)


_PUBKEY_CLAIM = bytes.fromhex("02" + "11" * 32)
_PUBKEY_REFUND = bytes.fromhex("03" + "22" * 32)
_CLTV_PUSH = bytes([0x03, 0x00, 0x00, 0x10])  # 3-byte minimal push, value 0x100000
_PREIMAGE = bytes([0xAB] * 32)


def _build_submarine_redeem_script(hashlock160: bytes, *, reverse_variant: bool = False) -> bytes:
    parts = []
    if reverse_variant:
        parts.append(bytes([0x82, 0x01, 0x20, 0x88]))  # OP_SIZE 0x20 OP_EQUALVERIFY
    parts.append(bytes([0xA9, 0x14]) + hashlock160 + bytes([0x87]))  # HASH160 <h> EQUAL
    parts.append(bytes([0x63, 0x21]) + _PUBKEY_CLAIM)  # IF <33 pubkey>
    parts.append(bytes([0x67]) + _CLTV_PUSH + bytes([0xB1, 0x75]))  # ELSE <cltv> CLTV DROP
    parts.append(bytes([0x21]) + _PUBKEY_REFUND + bytes([0x68, 0xAC]))  # <pubkey> ENDIF CHECKSIG
    return b"".join(parts)


class ParseRedeemScriptTests(unittest.TestCase):
    def test_submarine_swap_script_parses(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        result = parse_htlc_redeem_script(script)
        self.assertIsNotNone(result)
        self.assertIsInstance(result, HtlcExtraction)
        self.assertEqual(result.template, "boltz_v1_submarine")
        self.assertEqual(result.role, "fund")
        self.assertEqual(result.hashlock160, hashlock160.hex())
        self.assertIsNone(result.payment_hash)
        self.assertIsNone(result.preimage)

    def test_reverse_swap_script_parses_with_size_prefix(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160, reverse_variant=True)
        result = parse_htlc_redeem_script(script)
        self.assertIsNotNone(result)
        self.assertEqual(result.template, "boltz_v1_reverse")
        self.assertEqual(result.hashlock160, hashlock160.hex())

    def test_empty_script_returns_none(self):
        self.assertIsNone(parse_htlc_redeem_script(b""))

    def test_random_bytes_return_none(self):
        self.assertIsNone(parse_htlc_redeem_script(b"\x00" * 80))
        self.assertIsNone(parse_htlc_redeem_script(b"\xff" * 80))

    def test_trailing_bytes_disqualify_script(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160) + b"\x00"
        self.assertIsNone(parse_htlc_redeem_script(script))

    def test_missing_endif_disqualifies_script(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        # Replace OP_ENDIF with OP_DROP to simulate a malformed shape.
        broken = script[:-2] + bytes([0x75, 0xAC])
        self.assertIsNone(parse_htlc_redeem_script(broken))

    def test_wrong_pubkey_length_rejected(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        # Build a script with a 32-byte (Schnorr-sized) pubkey in the claim branch.
        bad_pubkey = bytes.fromhex("11" * 32)
        script = b"".join(
            [
                bytes([0xA9, 0x14]) + hashlock160 + bytes([0x87]),
                bytes([0x63, 0x20]) + bad_pubkey,
                bytes([0x67]) + _CLTV_PUSH + bytes([0xB1, 0x75]),
                bytes([0x21]) + _PUBKEY_REFUND + bytes([0x68, 0xAC]),
            ]
        )
        self.assertIsNone(parse_htlc_redeem_script(script))


class ExtractFromClaimWitnessTests(unittest.TestCase):
    def _build_witness(self, *, preimage: bytes, script: bytes):
        return [
            bytes.fromhex("3045" + "00" * 70),  # synthetic signature placeholder
            preimage,
            bytes([0x01]),  # OP_TRUE selector for IF branch
            script,
        ]

    def test_claim_witness_recovers_payment_hash(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        witness = self._build_witness(preimage=_PREIMAGE, script=script)
        result = extract_from_claim_witness(witness)
        self.assertIsNotNone(result)
        self.assertEqual(result.role, "claim")
        self.assertEqual(result.preimage, _PREIMAGE.hex())
        self.assertEqual(result.payment_hash, hashlib.sha256(_PREIMAGE).hexdigest())
        self.assertEqual(result.hashlock160, hashlock160.hex())

    def test_claim_witness_with_reverse_variant_script(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160, reverse_variant=True)
        witness = self._build_witness(preimage=_PREIMAGE, script=script)
        result = extract_from_claim_witness(witness)
        self.assertIsNotNone(result)
        self.assertEqual(result.template, "boltz_v1_reverse")
        self.assertEqual(result.payment_hash, hashlib.sha256(_PREIMAGE).hexdigest())

    def test_wrong_preimage_length_ignored(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        wrong = _PREIMAGE[:31]  # 31 bytes, not 32
        witness = self._build_witness(preimage=wrong, script=script)
        self.assertIsNone(extract_from_claim_witness(witness))

    def test_unrelated_preimage_ignored(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        other = bytes([0xCD] * 32)
        witness = self._build_witness(preimage=other, script=script)
        self.assertIsNone(extract_from_claim_witness(witness))

    def test_taproot_keypath_witness_returns_none(self):
        # Taproot key-path spends are a single 64-byte Schnorr signature.
        self.assertIsNone(extract_from_claim_witness([bytes(64)]))

    def test_empty_witness_returns_none(self):
        self.assertIsNone(extract_from_claim_witness([]))
        self.assertIsNone(extract_from_claim_witness([bytes(33)]))


class ExtractFromRefundWitnessTests(unittest.TestCase):
    def _build_refund_witness(self, *, script: bytes, selector: bytes = b""):
        # CLTV timeout spend: an empty (falsy) selector where a claim would
        # push the preimage, so the script falls through to the ELSE branch.
        return [
            bytes.fromhex("3045" + "00" * 70),  # synthetic signature placeholder
            selector,
            script,
        ]

    def test_refund_witness_recognized(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        witness = self._build_refund_witness(script=script)
        result = extract_from_refund_witness(witness)
        self.assertIsNotNone(result)
        self.assertIsInstance(result, HtlcExtraction)
        self.assertEqual(result.role, "refund")
        self.assertEqual(result.template, "boltz_v1_submarine")
        self.assertEqual(result.hashlock160, hashlock160.hex())
        self.assertIsNone(result.payment_hash)
        self.assertIsNone(result.preimage)

    def test_refund_witness_reverse_variant(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160, reverse_variant=True)
        result = extract_from_refund_witness(self._build_refund_witness(script=script))
        self.assertIsNotNone(result)
        self.assertEqual(result.role, "refund")
        self.assertEqual(result.template, "boltz_v1_reverse")

    def test_claim_witness_is_not_a_refund(self):
        # A witness that reveals the preimage is a claim; the refund decoder
        # must decline it so the two paths never both fire on one spend.
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        claim_witness = [
            bytes.fromhex("3045" + "00" * 70),
            _PREIMAGE,
            bytes([0x01]),
            script,
        ]
        self.assertIsNone(extract_from_refund_witness(claim_witness))

    def test_unrelated_non_preimage_selector_still_refund(self):
        # A non-empty selector that is not the 32-byte preimage still spends
        # the timeout branch — it just has to be falsy on-chain.
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        witness = self._build_refund_witness(script=script, selector=bytes([0x00]))
        result = extract_from_refund_witness(witness)
        self.assertIsNotNone(result)
        self.assertEqual(result.role, "refund")

    def test_non_htlc_witness_returns_none(self):
        self.assertIsNone(extract_from_refund_witness([bytes(64)]))
        self.assertIsNone(extract_from_refund_witness([b"", b"\x00" * 80]))

    def test_empty_or_short_witness_returns_none(self):
        self.assertIsNone(extract_from_refund_witness([]))
        self.assertIsNone(extract_from_refund_witness([bytes(33)]))

    def test_stored_transaction_recovers_only_one_unique_refund_outpoint(self):
        script = _build_submarine_redeem_script(_embit_hashes.hash160(_PREIMAGE))
        funding_txid = "ab" * 32
        refund_vin = {
            "txid": funding_txid,
            "vout": 7,
            "witness": ["3045", "", script.hex()],
        }
        self.assertEqual(
            refund_funding_outpoint_from_tx_mapping({"vin": [refund_vin]}),
            (funding_txid, 7),
        )
        second = {**refund_vin, "txid": "cd" * 32, "vout": 1}
        self.assertIsNone(
            refund_funding_outpoint_from_tx_mapping({"vin": [refund_vin, second]})
        )


class ScriptMatchesPaymentHashTests(unittest.TestCase):
    def test_matching_payment_hash_returns_true(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        payment_hash = hashlib.sha256(_PREIMAGE).hexdigest()
        self.assertTrue(script_matches_payment_hash(script, payment_hash))

    def test_mismatched_payment_hash_returns_false(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        unrelated = hashlib.sha256(b"unrelated").hexdigest()
        self.assertFalse(script_matches_payment_hash(script, unrelated))

    def test_invalid_hex_returns_false(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        self.assertFalse(script_matches_payment_hash(script, "not-hex"))

    def test_short_payment_hash_returns_false(self):
        hashlock160 = _embit_hashes.hash160(_PREIMAGE)
        script = _build_submarine_redeem_script(hashlock160)
        self.assertFalse(script_matches_payment_hash(script, "aa" * 16))

    def test_non_htlc_script_returns_false(self):
        payment_hash = hashlib.sha256(_PREIMAGE).hexdigest()
        self.assertFalse(script_matches_payment_hash(b"\x00\x14" + b"\x00" * 20, payment_hash))


if __name__ == "__main__":
    unittest.main()
