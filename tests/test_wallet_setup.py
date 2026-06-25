"""Unit tests for ``kassiber.wallet_setup.normalize_wallet_material``.

The SLIP132 conversion path runs a custom base58check codec and a
version-byte swap. End-to-end daemon tests only exercise descriptor JSON,
so this module covers the SLIP132 paths directly using ``embit`` as an
independent ground truth for encoding the alternate prefixes.
"""

from __future__ import annotations

import json
import unittest

from embit import bip32

from kassiber.errors import AppError
from kassiber.wallet_setup import normalize_wallet_material


_SLIP132_MAINNET_VERSIONS = {
    "ypub": bytes.fromhex("049d7cb2"),
    "zpub": bytes.fromhex("04b24746"),
}

_SLIP132_TESTNET_VERSIONS = {
    "upub": bytes.fromhex("044a5262"),
    "vpub": bytes.fromhex("045f1cf6"),
}


def _account_xpub() -> tuple[bip32.HDKey, str]:
    seed = bytes.fromhex("000102030405060708090a0b0c0d0e0f")
    account = bip32.HDKey.from_seed(seed).derive("m/84h/0h/0h").to_public()
    return account, account.to_base58()


class NormalizeWalletMaterialSlip132Tests(unittest.TestCase):
    def test_zpub_converts_to_native_segwit_descriptors(self):
        account, xpub = _account_xpub()
        zpub = account.to_base58(version=_SLIP132_MAINNET_VERSIONS["zpub"])

        result = normalize_wallet_material(zpub)

        self.assertEqual(result["descriptor"], f"wpkh({xpub}/0/*)")
        self.assertEqual(result["change_descriptor"], f"wpkh({xpub}/1/*)")

    def test_ypub_converts_to_p2sh_wrapped_segwit_descriptors(self):
        account, xpub = _account_xpub()
        ypub = account.to_base58(version=_SLIP132_MAINNET_VERSIONS["ypub"])

        result = normalize_wallet_material(ypub)

        self.assertEqual(result["descriptor"], f"sh(wpkh({xpub}/0/*))")
        self.assertEqual(result["change_descriptor"], f"sh(wpkh({xpub}/1/*))")

    def test_vpub_converts_to_testnet_native_segwit_descriptors(self):
        account, _ = _account_xpub()
        tpub = account.to_base58(version=bytes.fromhex("043587cf"))
        vpub = account.to_base58(version=_SLIP132_TESTNET_VERSIONS["vpub"])

        result = normalize_wallet_material(vpub)

        self.assertEqual(result["descriptor"], f"wpkh({tpub}/0/*)")
        self.assertEqual(result["change_descriptor"], f"wpkh({tpub}/1/*)")

    def test_upub_converts_to_testnet_p2sh_wrapped_segwit_descriptors(self):
        account, _ = _account_xpub()
        tpub = account.to_base58(version=bytes.fromhex("043587cf"))
        upub = account.to_base58(version=_SLIP132_TESTNET_VERSIONS["upub"])

        result = normalize_wallet_material(upub)

        self.assertEqual(result["descriptor"], f"sh(wpkh({tpub}/0/*))")
        self.assertEqual(result["change_descriptor"], f"sh(wpkh({tpub}/1/*))")

    def test_bare_xpub_is_rejected_as_ambiguous(self):
        _, xpub = _account_xpub()

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(xpub)

        self.assertIn("ambiguous", str(ctx.exception).lower())

    def test_bare_tpub_is_rejected_as_ambiguous(self):
        account, _ = _account_xpub()
        tpub = account.to_base58(version=bytes.fromhex("043587cf"))

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(tpub)

        self.assertIn("ambiguous", str(ctx.exception).lower())

    def test_corrupted_zpub_checksum_is_rejected(self):
        account, _ = _account_xpub()
        zpub = account.to_base58(version=_SLIP132_MAINNET_VERSIONS["zpub"])
        # Mutate the last character to break the checksum without changing length.
        flipped = "A" if zpub[-1] != "A" else "B"
        broken = zpub[:-1] + flipped

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(broken)

        self.assertIn("checksum", str(ctx.exception).lower())


class NormalizeWalletMaterialBareXpubScriptTypeTests(unittest.TestCase):
    def test_each_script_type_wraps_bare_xpub(self):
        _, xpub = _account_xpub()
        cases = {
            "p2wpkh": (f"wpkh({xpub}/0/*)", f"wpkh({xpub}/1/*)"),
            "p2sh-p2wpkh": (f"sh(wpkh({xpub}/0/*))", f"sh(wpkh({xpub}/1/*))"),
            "p2pkh": (f"pkh({xpub}/0/*)", f"pkh({xpub}/1/*)"),
            "p2tr": (f"tr({xpub}/0/*)", f"tr({xpub}/1/*)"),
        }
        for script_type, (receive, change) in cases.items():
            with self.subTest(script_type=script_type):
                result = normalize_wallet_material(xpub, script_type=script_type)
                self.assertEqual(result["descriptor"], receive)
                self.assertEqual(result["change_descriptor"], change)

    def test_script_type_wraps_bare_tpub(self):
        account, _ = _account_xpub()
        tpub = account.to_base58(version=bytes.fromhex("043587cf"))

        result = normalize_wallet_material(tpub, script_type="p2wpkh")

        self.assertEqual(result["descriptor"], f"wpkh({tpub}/0/*)")
        self.assertEqual(result["change_descriptor"], f"wpkh({tpub}/1/*)")

    def test_unknown_script_type_is_rejected(self):
        _, xpub = _account_xpub()

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(xpub, script_type="p2wsh")

        self.assertIn("script type", str(ctx.exception).lower())

    def test_blank_script_type_still_rejects_bare_xpub_as_ambiguous(self):
        _, xpub = _account_xpub()

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(xpub, script_type="")

        self.assertIn("ambiguous", str(ctx.exception).lower())

    def test_slip132_key_ignores_script_type(self):
        account, xpub = _account_xpub()
        ypub = account.to_base58(version=_SLIP132_MAINNET_VERSIONS["ypub"])

        # A ypub already encodes its script type; a stray script_type hint
        # must not override the SLIP132 resolution.
        result = normalize_wallet_material(ypub, script_type="p2wpkh")

        self.assertEqual(result["descriptor"], f"sh(wpkh({xpub}/0/*))")
        self.assertEqual(result["change_descriptor"], f"sh(wpkh({xpub}/1/*))")

    def test_corrupted_bare_xpub_with_script_type_is_rejected(self):
        _, xpub = _account_xpub()
        flipped = "A" if xpub[-1] != "A" else "B"
        broken = xpub[:-1] + flipped

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(broken, script_type="p2wpkh")

        self.assertIn("checksum", str(ctx.exception).lower())


class NormalizeWalletMaterialOtherShapesTests(unittest.TestCase):
    def test_descriptor_json_export_keeps_receive_and_change_branches(self):
        payload = json.dumps(
            {
                "descriptors": [
                    {"desc": "wpkh([abcd0123/84h/0h/0h]xpub6.../0/*)", "active": True, "internal": False},
                    {"desc": "wpkh([abcd0123/84h/0h/0h]xpub6.../1/*)", "active": True, "internal": True},
                ]
            }
        )

        result = normalize_wallet_material(payload)

        self.assertEqual(result["descriptor"], "wpkh([abcd0123/84h/0h/0h]xpub6.../0/*)")
        self.assertEqual(result["change_descriptor"], "wpkh([abcd0123/84h/0h/0h]xpub6.../1/*)")

    def test_blank_input_is_rejected(self):
        with self.assertRaises(AppError):
            normalize_wallet_material("   ")

    def test_unknown_format_is_rejected(self):
        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material("not-a-descriptor-or-key")

        self.assertIn("unsupported", str(ctx.exception).lower())


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
