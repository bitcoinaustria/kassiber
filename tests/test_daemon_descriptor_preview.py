"""Unit tests for the descriptor preview daemon endpoint.

Exercises ``_preview_descriptor_payload`` directly so the connection setup
form can show derived addresses before the user commits to creating a
wallet. The daemon smoke suite already covers the JSONL transport — these
focus on input validation and the wallet_material → addresses round trip.
"""

from __future__ import annotations

import unittest

from embit import bip32

from kassiber.daemon import _preview_descriptor_payload
from kassiber.errors import AppError


# User-approved public mainnet zpub fixture with real mainnet history. Preview
# tests use it only as watch-only material and do not contact the network.
PUBLIC_MAINNET_ZPUB_FIXTURE = (
    "zpub6rFR7y4Q2AijBEqTUquhVz398htDFrtymD9xYYfG1m4wAcvPhXNfE3EfH1r1AD"
    "qtfSdVCToUG868RvUUkgDKf31mGDtKsAYz2oz2AGutZYs"
)


def _xpub_from_seed() -> str:
    seed = bytes.fromhex("000102030405060708090a0b0c0d0e0f")
    account = bip32.HDKey.from_seed(seed).derive("m/84h/0h/0h").to_public()
    return account.to_base58()


class PreviewDescriptorTests(unittest.TestCase):
    def test_zpub_material_returns_receive_and_change_addresses(self):
        result = _preview_descriptor_payload(
            {"wallet_material": PUBLIC_MAINNET_ZPUB_FIXTURE, "count": 3}
        )

        self.assertEqual(result["chain"], "bitcoin")
        self.assertEqual(result["network"], "main")
        self.assertTrue(result["has_change_branch"])
        receive = [addr for addr in result["addresses"] if addr["branch"] == "receive"]
        change = [addr for addr in result["addresses"] if addr["branch"] == "change"]
        self.assertEqual(len(receive), 3)
        self.assertEqual(len(change), 1)
        for entry in receive + change:
            self.assertTrue(entry["address"].startswith("bc1"))

    def test_explicit_descriptor_with_change_branch_is_honored(self):
        xpub = _xpub_from_seed()

        result = _preview_descriptor_payload(
            {
                "descriptor": f"wpkh({xpub}/0/*)",
                "change_descriptor": f"wpkh({xpub}/1/*)",
                "count": 2,
            }
        )

        self.assertEqual(len(result["addresses"]), 3)  # 2 receive + 1 change
        self.assertTrue(result["has_change_branch"])

    def test_count_is_clamped_to_twenty(self):
        result = _preview_descriptor_payload(
            {"wallet_material": PUBLIC_MAINNET_ZPUB_FIXTURE, "count": 999}
        )

        receive = [addr for addr in result["addresses"] if addr["branch"] == "receive"]
        self.assertEqual(len(receive), 20)

    def test_missing_descriptor_is_rejected(self):
        with self.assertRaises(AppError) as ctx:
            _preview_descriptor_payload({})
        self.assertEqual(ctx.exception.code, "validation")

    def test_unparseable_descriptor_returns_validation_error(self):
        with self.assertRaises(AppError) as ctx:
            _preview_descriptor_payload({"descriptor": "wpkh(not-a-key/0/*)"})
        self.assertEqual(ctx.exception.code, "validation")
        self.assertIn("descriptor", str(ctx.exception).lower())


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
