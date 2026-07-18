"""Unit tests for ``kassiber.wallet_setup.normalize_wallet_material``.

The SLIP132 conversion path runs a custom base58check codec and a
version-byte swap. End-to-end daemon tests only exercise descriptor JSON,
so this module covers the SLIP132 paths directly using ``embit`` as an
independent ground truth for encoding the alternate prefixes.
"""

from __future__ import annotations

import json
import unittest
from unittest import mock

from embit import bip32

from kassiber.errors import AppError
from kassiber.wallet_descriptors import derive_descriptor_target, load_descriptor_plan
from kassiber.wallet_setup import (
    BSMS_DESCRIPTOR_SOURCE,
    normalize_script_types,
    normalize_wallet_material,
)
from kassiber.wallet_security import assert_descriptor_text_is_watch_only


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


def _private_account() -> bip32.HDKey:
    seed = bytes.fromhex("000102030405060708090a0b0c0d0e0f")
    return bip32.HDKey.from_seed(seed).derive("m/84h/0h/0h")


class NormalizeWalletMaterialSlip132Tests(unittest.TestCase):
    def test_zpub_converts_to_native_segwit_descriptors(self):
        account, xpub = _account_xpub()
        zpub = account.to_base58(version=_SLIP132_MAINNET_VERSIONS["zpub"])

        result = normalize_wallet_material(zpub)

        self.assertEqual(result["descriptor"], f"wpkh({xpub}/0/*)")
        self.assertEqual(result["change_descriptor"], f"wpkh({xpub}/1/*)")
        self.assertEqual(result["network"], "main")

    def test_ypub_converts_to_p2sh_wrapped_segwit_descriptors(self):
        account, xpub = _account_xpub()
        ypub = account.to_base58(version=_SLIP132_MAINNET_VERSIONS["ypub"])

        result = normalize_wallet_material(ypub)

        self.assertEqual(result["descriptor"], f"sh(wpkh({xpub}/0/*))")
        self.assertEqual(result["change_descriptor"], f"sh(wpkh({xpub}/1/*))")
        self.assertEqual(result["network"], "main")

    def test_vpub_converts_to_testnet_native_segwit_descriptors(self):
        account, _ = _account_xpub()
        tpub = account.to_base58(version=bytes.fromhex("043587cf"))
        vpub = account.to_base58(version=_SLIP132_TESTNET_VERSIONS["vpub"])

        result = normalize_wallet_material(vpub)

        self.assertEqual(result["descriptor"], f"wpkh({tpub}/0/*)")
        self.assertEqual(result["change_descriptor"], f"wpkh({tpub}/1/*)")
        self.assertEqual(result["network"], "test")

        plan = load_descriptor_plan({"chain": "bitcoin", **result})
        self.assertEqual(plan.network, "test")
        self.assertTrue(derive_descriptor_target(plan, 0, 0).address.startswith("tb1"))

    def test_upub_converts_to_testnet_p2sh_wrapped_segwit_descriptors(self):
        account, _ = _account_xpub()
        tpub = account.to_base58(version=bytes.fromhex("043587cf"))
        upub = account.to_base58(version=_SLIP132_TESTNET_VERSIONS["upub"])

        result = normalize_wallet_material(upub)

        self.assertEqual(result["descriptor"], f"sh(wpkh({tpub}/0/*))")
        self.assertEqual(result["change_descriptor"], f"sh(wpkh({tpub}/1/*))")
        self.assertEqual(result["network"], "test")

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
        self.assertEqual(result["network"], "test")

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


class NormalizeScriptTypesTests(unittest.TestCase):
    def test_validates_dedupes_and_sorts(self):
        self.assertEqual(
            normalize_script_types(["p2tr", "p2wpkh", "p2wpkh", "P2TR"]),
            ["p2tr", "p2wpkh"],
        )

    def test_none_and_empty_yield_empty_list(self):
        self.assertEqual(normalize_script_types(None), [])
        self.assertEqual(normalize_script_types([]), [])
        self.assertEqual(normalize_script_types([""]), [])

    def test_single_string_is_accepted(self):
        self.assertEqual(normalize_script_types("p2wpkh"), ["p2wpkh"])

    def test_unknown_type_is_rejected(self):
        with self.assertRaises(AppError) as ctx:
            normalize_script_types(["p2wpkh", "p2wsh"])
        self.assertIn("script type", str(ctx.exception).lower())


class NormalizeWalletMaterialMultiScriptTests(unittest.TestCase):
    def test_bare_xpub_with_script_types_returns_xpub_and_sorted_set(self):
        _, xpub = _account_xpub()

        result = normalize_wallet_material(xpub, script_types=["p2tr", "p2wpkh"])

        self.assertEqual(
            result,
            {
                "xpub": xpub,
                "script_types": ["p2tr", "p2wpkh"],
                "network": "main",
            },
        )
        self.assertNotIn("descriptor", result)

    def test_bare_xpub_with_single_script_type_in_list(self):
        _, xpub = _account_xpub()

        result = normalize_wallet_material(xpub, script_types=["p2wpkh"])

        self.assertEqual(
            result,
            {"xpub": xpub, "script_types": ["p2wpkh"], "network": "main"},
        )

    def test_empty_script_types_falls_back_to_ambiguous(self):
        _, xpub = _account_xpub()

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(xpub, script_types=[])

        self.assertIn("ambiguous", str(ctx.exception).lower())

    def test_unknown_script_type_in_list_is_rejected(self):
        _, xpub = _account_xpub()

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(xpub, script_types=["p2wpkh", "nope"])

        self.assertIn("script type", str(ctx.exception).lower())

    def test_corrupted_bare_xpub_with_script_types_is_rejected(self):
        _, xpub = _account_xpub()
        flipped = "A" if xpub[-1] != "A" else "B"
        broken = xpub[:-1] + flipped

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(broken, script_types=["p2wpkh"])

        self.assertIn("checksum", str(ctx.exception).lower())

    def test_slip132_key_ignores_script_types(self):
        account, xpub = _account_xpub()
        ypub = account.to_base58(version=_SLIP132_MAINNET_VERSIONS["ypub"])

        # A ypub already encodes its script type; multi-script hints are ignored.
        result = normalize_wallet_material(ypub, script_types=["p2wpkh", "p2tr"])

        self.assertEqual(result["descriptor"], f"sh(wpkh({xpub}/0/*))")
        self.assertNotIn("xpub", result)


class NormalizeWalletMaterialWatchOnlyTests(unittest.TestCase):
    def _assert_secret_free_rejection(self, material: str, *, script_type=None):
        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(material, script_type=script_type)
        error = ctx.exception
        self.assertEqual(error.code, "wallet_spending_private_material")
        rendered = json.dumps(
            {
                "message": str(error),
                "hint": error.hint,
                "details": error.details,
            },
            sort_keys=True,
        )
        self.assertNotIn(material, rendered)
        self.assertNotIn("xprv", rendered.lower())
        self.assertNotIn("wif", rendered.lower())

    def test_rejects_all_supported_extended_private_key_versions(self):
        account = _private_account()
        versions = {
            "xprv": "0488ade4",
            "tprv": "04358394",
            "yprv": "049d7878",
            "zprv": "04b2430c",
            "uprv": "044a4e28",
            "vprv": "045f18bc",
        }
        for expected_prefix, version in versions.items():
            material = account.to_base58(version=bytes.fromhex(version))
            with self.subTest(prefix=expected_prefix):
                self.assertTrue(material.startswith(expected_prefix))
                self._assert_secret_free_rejection(
                    material,
                    script_type="p2wpkh",
                )

    def test_rejects_bare_wif(self):
        self._assert_secret_free_rejection(_private_account().key.wif())

    def test_rejects_private_receive_or_change_in_json_export(self):
        account = _private_account()
        xpub = account.to_public().to_base58()
        xprv = account.to_base58()
        for payload in (
            {"descriptor": f"wpkh({xprv}/0/*)"},
            {
                "descriptor": f"wpkh({xpub}/0/*)",
                "change_descriptor": f"wpkh({xprv}/1/*)",
            },
        ):
            with self.subTest(fields=tuple(payload)):
                material = json.dumps(payload)
                with self.assertRaises(AppError) as ctx:
                    normalize_wallet_material(material)
                self.assertEqual(
                    ctx.exception.code,
                    "wallet_spending_private_material",
                )
                self.assertNotIn(xprv, str(ctx.exception))

    def test_descriptor_preflight_propagates_typed_security_errors(self):
        rejection = AppError(
            "private material",
            code="wallet_spending_private_material",
            retryable=False,
        )
        with mock.patch(
            "embit.descriptor.Descriptor.from_string",
            side_effect=rejection,
        ):
            with self.assertRaises(AppError) as ctx:
                assert_descriptor_text_is_watch_only("wpkh(placeholder)")
        self.assertIs(ctx.exception, rejection)


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

    def test_bsms_template_expands_receive_and_change_descriptors(self):
        template = (
            "wsh(sortedmulti(2,"
            "[11111111/48h/0h/0h/2h]xpubA/**,"
            "[22222222/48h/0h/0h/2h]xpubB/**))#checksum"
        )
        material = "\n".join(
            [
                "BSMS 1.0",
                template,
                "/0/*,/1/*",
                "bc1qexamplefirstaddress",
            ]
        )

        result = normalize_wallet_material(material)

        self.assertEqual(
            result["descriptor"],
            template.replace("/**", "/0/*"),
        )
        self.assertEqual(
            result["change_descriptor"],
            template.replace("/**", "/1/*"),
        )
        self.assertEqual(result["descriptor_source"], BSMS_DESCRIPTOR_SOURCE)
        self.assertFalse(result["synthesize_change"])

    def test_bsms_descriptor_without_path_restrictions_is_accepted(self):
        descriptor = (
            "wsh(sortedmulti(1,"
            "[11111111/48h/0h/0h/2h]021111111111111111111111111111111111111111111111111111111111111111,"
            "[22222222/48h/0h/0h/2h]032222222222222222222222222222222222222222222222222222222222222222))"
        )
        material = "\n".join(
            [
                "BSMS 1.0",
                descriptor,
                "No path restrictions",
                "bc1qexamplefirstaddress",
            ]
        )

        result = normalize_wallet_material(material)

        self.assertEqual(result["descriptor"], descriptor)
        self.assertEqual(result["descriptor_source"], BSMS_DESCRIPTOR_SOURCE)
        self.assertFalse(result["synthesize_change"])

    def test_bsms_signer_key_record_is_rejected(self):
        material = "\n".join(
            [
                "BSMS 1.0",
                "00",
                "[11111111/48h/0h/0h/2h]xpubA",
                "Signer 1 key",
                "signature",
            ]
        )

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(material)
        self.assertIn("key records", str(ctx.exception).lower())

    def test_bsms_with_extra_restrictions_is_rejected(self):
        material = "\n".join(
            [
                "BSMS 1.0",
                "wsh(sortedmulti(2,[11111111/48h/0h/0h/2h]xpubA/**,[22222222/48h/0h/0h/2h]xpubB/**))",
                "/0/*,/1/*,/2/*",
                "bc1qexamplefirstaddress",
            ]
        )

        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material(material)
        self.assertIn("receive/change", str(ctx.exception).lower())

    def test_blank_input_is_rejected(self):
        with self.assertRaises(AppError):
            normalize_wallet_material("   ")

    def test_unknown_format_is_rejected(self):
        with self.assertRaises(AppError) as ctx:
            normalize_wallet_material("not-a-descriptor-or-key")

        self.assertIn("unsupported", str(ctx.exception).lower())


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
