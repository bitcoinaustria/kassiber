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

from .descriptor_fixtures import PUBLIC_MAINNET_ZPUB_FIXTURE


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

    def test_receive_only_descriptor_synthesizes_change_branch(self):
        # A lone receive-chain descriptor (no explicit change descriptor) must
        # still derive the sibling change chain. Otherwise change/internal UTXOs
        # are never scanned and silently vanish from balances and the UTXO list.
        xpub = _xpub_from_seed()

        result = _preview_descriptor_payload(
            {"descriptor": f"wpkh({xpub}/0/*)", "count": 3}
        )

        self.assertTrue(result["has_change_branch"])
        receive = [addr for addr in result["addresses"] if addr["branch"] == "receive"]
        change = [addr for addr in result["addresses"] if addr["branch"] == "change"]
        self.assertEqual(len(receive), 3)
        self.assertEqual(len(change), 1)
        # The change address derives from chain index 1 and is distinct from
        # every receive address.
        self.assertEqual(change[0]["derivation_path"], "m/1/0")
        self.assertNotIn(change[0]["address"], {addr["address"] for addr in receive})

    def test_bsms_wallet_material_previews_receive_and_change(self):
        xpub = _xpub_from_seed()
        material = "\n".join(
            [
                "BSMS 1.0",
                f"wpkh({xpub}/**)",
                "/0/*,/1/*",
                "bc1qplaceholderfirstaddress",
            ]
        )

        result = _preview_descriptor_payload({"wallet_material": material, "count": 2})

        self.assertTrue(result["has_change_branch"])
        receive = [addr for addr in result["addresses"] if addr["branch"] == "receive"]
        change = [addr for addr in result["addresses"] if addr["branch"] == "change"]
        self.assertEqual(len(receive), 2)
        self.assertEqual(len(change), 1)
        self.assertEqual(change[0]["derivation_path"], "m/1/0")

    def test_fixed_single_address_descriptor_has_no_change_branch(self):
        # A non-ranged descriptor is a single fixed address, not a wallet chain;
        # it must not gain a synthetic change branch.
        xpub = _xpub_from_seed()

        result = _preview_descriptor_payload(
            {"descriptor": f"wpkh({xpub}/0/5)", "count": 3}
        )

        self.assertFalse(result["has_change_branch"])
        self.assertTrue(all(addr["branch"] == "receive" for addr in result["addresses"]))

    def test_count_is_clamped_to_twenty(self):
        result = _preview_descriptor_payload(
            {"wallet_material": PUBLIC_MAINNET_ZPUB_FIXTURE, "count": 999}
        )

        receive = [addr for addr in result["addresses"] if addr["branch"] == "receive"]
        self.assertEqual(len(receive), 20)

    def test_bare_xpub_with_script_type_derives_addresses(self):
        # A bare xpub is ambiguous on its own; once the setup form supplies a
        # script type the preview must derive the matching address kind. This
        # also confirms taproot (tr), which the SLIP132 path never produced.
        xpub = _xpub_from_seed()
        prefixes = {
            "p2wpkh": "bc1q",
            "p2sh-p2wpkh": "3",
            "p2pkh": "1",
            "p2tr": "bc1p",
        }
        for script_type, prefix in prefixes.items():
            with self.subTest(script_type=script_type):
                result = _preview_descriptor_payload(
                    {"wallet_material": xpub, "script_type": script_type, "count": 2}
                )
                self.assertTrue(result["has_change_branch"])
                for entry in result["addresses"]:
                    self.assertTrue(
                        entry["address"].startswith(prefix),
                        f"{script_type}: {entry['address']} !~ {prefix}",
                    )

    def test_bare_xpub_with_script_types_previews_each_type(self):
        # Auto-detect / multi-script: the preview derives a labeled receive set
        # per enabled type plus one change sample each.
        xpub = _xpub_from_seed()

        result = _preview_descriptor_payload(
            {"wallet_material": xpub, "script_types": ["p2wpkh", "p2tr"], "count": 2}
        )

        self.assertTrue(result["has_change_branch"])
        self.assertEqual(
            {addr["branch"] for addr in result["addresses"]},
            {"p2wpkh receive", "p2wpkh change", "p2tr receive", "p2tr change"},
        )
        p2wpkh_receive = [a for a in result["addresses"] if a["branch"] == "p2wpkh receive"]
        p2tr_receive = [a for a in result["addresses"] if a["branch"] == "p2tr receive"]
        self.assertEqual(len(p2wpkh_receive), 2)
        self.assertEqual(len(p2tr_receive), 2)
        self.assertTrue(all(a["address"].startswith("bc1q") for a in p2wpkh_receive))
        self.assertTrue(all(a["address"].startswith("bc1p") for a in p2tr_receive))
        change = [a for a in result["addresses"] if a["branch"].endswith("change")]
        self.assertEqual(len(change), 2)

    def test_bare_xpub_without_script_type_is_rejected(self):
        with self.assertRaises(AppError) as ctx:
            _preview_descriptor_payload({"wallet_material": _xpub_from_seed()})
        self.assertEqual(ctx.exception.code, "validation")
        self.assertIn("ambiguous", str(ctx.exception).lower())

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
