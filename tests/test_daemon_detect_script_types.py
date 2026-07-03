"""Unit tests for the script-type auto-detect daemon endpoint.

Exercises ``_detect_script_types_payload`` directly: the add-wallet form probes
a bare xpub to learn which script types it has on-chain history for, then
creates a wallet watching those. Detection must stay rate-limit-safe (exactly
one probe per script type) and degrade gracefully — a missing/unreachable
backend falls back to Native SegWit so the user can still create and pick
manually, while a malformed key is a hard validation error.
"""

from __future__ import annotations

import types
import unittest
from unittest.mock import patch

from embit import bip32

from kassiber.daemon import _detect_script_types_payload
from kassiber.errors import AppError
from kassiber.wallet_descriptors import (
    SCRIPT_TYPE_BRANCH_BASE,
    derive_descriptor_target,
    load_descriptor_plan,
)

_ESPLORA_BACKEND = {
    "name": "esplora",
    "kind": "esplora",
    "url": "https://esplora.example",
}


def _xpub_from_seed() -> str:
    seed = bytes.fromhex("000102030405060708090a0b0c0d0e0f")
    return bip32.HDKey.from_seed(seed).derive("m/84h/0h/0h").to_public().to_base58()


def _ctx(backends: dict | None = None, default: str = "esplora"):
    return types.SimpleNamespace(
        runtime_config={
            "default_backend": default,
            "backends": backends if backends is not None else {"esplora": _ESPLORA_BACKEND},
            "env_file": "(test)",
        }
    )


def _receive_script_pubkey(xpub: str, script_type: str) -> str:
    plan = load_descriptor_plan(
        {
            "xpub": xpub,
            "script_types": [script_type],
            "chain": "bitcoin",
            "network": "main",
            "gap_limit": 1,
        }
    )
    base = SCRIPT_TYPE_BRANCH_BASE[script_type]
    return derive_descriptor_target(plan, base, 0).script_pubkey


class DetectScriptTypesTests(unittest.TestCase):
    def test_active_types_detected_with_exactly_four_probes(self):
        xpub = _xpub_from_seed()
        active = {
            _receive_script_pubkey(xpub, "p2wpkh"),
            _receive_script_pubkey(xpub, "p2tr"),
        }
        calls: list[str] = []

        def fake_has_history(url, script_pubkey, timeout=30, proxy_url=None):
            del proxy_url
            calls.append(script_pubkey)
            return script_pubkey in active

        with patch(
            "kassiber.core.sync_backends.esplora_scripthash_has_history",
            side_effect=fake_has_history,
        ):
            result = _detect_script_types_payload(
                _ctx(), {"wallet_material": xpub, "chain": "bitcoin", "network": "main"}
            )

        # Rate-limit pin: one probe per candidate type, never a gap-window scan.
        self.assertEqual(len(calls), 4)
        self.assertTrue(result["probed"])
        self.assertFalse(result["fallback_used"])
        self.assertEqual(set(result["active"]), {"p2wpkh", "p2tr"})
        self.assertEqual(
            {entry["script_type"] for entry in result["detected"]},
            set(SCRIPT_TYPE_BRANCH_BASE),
        )

    def test_no_history_falls_back_to_native_segwit(self):
        with patch(
            "kassiber.core.sync_backends.esplora_scripthash_has_history",
            return_value=False,
        ):
            result = _detect_script_types_payload(
                _ctx(), {"wallet_material": _xpub_from_seed()}
            )

        self.assertTrue(result["probed"])
        self.assertTrue(result["fallback_used"])
        self.assertEqual(result["active"], ["p2wpkh"])

    def test_unresolvable_backend_falls_back_without_probing(self):
        result = _detect_script_types_payload(
            _ctx(backends={}, default="ghost"),
            {"wallet_material": _xpub_from_seed()},
        )

        self.assertFalse(result["probed"])
        self.assertTrue(result["fallback_used"])
        self.assertEqual(result["active"], ["p2wpkh"])

    def test_non_xpub_material_is_rejected(self):
        xpub = _xpub_from_seed()
        with self.assertRaises(AppError) as ctx:
            _detect_script_types_payload(
                _ctx(), {"wallet_material": f"wpkh({xpub}/0/*)"}
            )
        self.assertEqual(ctx.exception.code, "validation")

    def test_malformed_xpub_is_rejected(self):
        xpub = _xpub_from_seed()
        broken = xpub[:-1] + ("A" if xpub[-1] != "A" else "B")
        with self.assertRaises(AppError) as ctx:
            _detect_script_types_payload(_ctx(), {"wallet_material": broken})
        self.assertEqual(ctx.exception.code, "validation")
        self.assertIn("checksum", str(ctx.exception).lower())


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
