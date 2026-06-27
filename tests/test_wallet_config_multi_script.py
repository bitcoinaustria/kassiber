"""Unit tests for multi-script xpub wallet config building.

``parse_wallet_config`` (the CLI ``wallet create`` builder) turns a bare xpub +
``--script-type`` flags into the stored ``xpub`` / ``script_types`` shape, while
a full descriptor with no script types stays a plain descriptor wallet.
``_validated_wallet_config`` then accepts an xpub-derived wallet as satisfying
the descriptor kind without an explicit ``descriptor``.
"""

from __future__ import annotations

import json
import tempfile
import types
import unittest
from pathlib import Path

from embit import bip32

from kassiber.core import accounts as core_accounts
from kassiber.core import freshness as core_freshness
from kassiber.core.sync import classify_wallet_sync
from kassiber.core.ui_snapshot import _wallet_backend_summary
from kassiber.core.wallets import (
    _validated_wallet_config,
    create_wallet,
    has_descriptor_sync_material,
    normalize_addresses,
    parse_wallet_config,
    update_wallet,
)
from kassiber.db import open_db
from kassiber.errors import AppError


def _xpub() -> str:
    seed = bytes.fromhex("000102030405060708090a0b0c0d0e0f")
    return bip32.HDKey.from_seed(seed).derive("m/84h/0h/0h").to_public().to_base58()


def _args(**overrides) -> types.SimpleNamespace:
    base = {
        "config": None,
        "config_file": None,
        "backend": None,
        "descriptor": None,
        "descriptor_file": None,
        "change_descriptor": None,
        "change_descriptor_file": None,
        "script_type": None,
        "address": None,
        "chain": None,
        "network": None,
        "gap_limit": None,
        "policy_asset": None,
        "source_file": None,
        "source_format": None,
    }
    base.update(overrides)
    return types.SimpleNamespace(**base)


class ParseWalletConfigMultiScriptTests(unittest.TestCase):
    def test_xpub_with_script_types_stores_xpub_and_set(self):
        xpub = _xpub()

        config = parse_wallet_config(
            _args(descriptor=xpub, script_type=["p2wpkh", "p2tr"], chain="bitcoin")
        )

        self.assertEqual(config.get("xpub"), xpub)
        self.assertEqual(config.get("script_types"), ["p2tr", "p2wpkh"])
        self.assertNotIn("descriptor", config)
        self.assertEqual(config.get("chain"), "bitcoin")

    def test_full_descriptor_without_script_type_is_unchanged(self):
        xpub = _xpub()

        config = parse_wallet_config(
            _args(descriptor=f"wpkh({xpub}/0/*)", chain="bitcoin")
        )

        self.assertEqual(config.get("descriptor"), f"wpkh({xpub}/0/*)")
        self.assertNotIn("xpub", config)
        self.assertNotIn("script_types", config)

    def test_script_type_without_material_is_rejected(self):
        with self.assertRaises(AppError) as ctx:
            parse_wallet_config(_args(script_type=["p2wpkh"]))
        self.assertEqual(ctx.exception.code, "validation")


class ValidatedWalletConfigTests(unittest.TestCase):
    def test_xpub_config_satisfies_descriptor_kind(self):
        config = _validated_wallet_config(
            "descriptor",
            {
                "xpub": _xpub(),
                "script_types": ["p2wpkh"],
                "chain": "bitcoin",
                "network": "main",
            },
        )

        self.assertEqual(config["chain"], "bitcoin")
        self.assertEqual(config["script_types"], ["p2wpkh"])

    def test_descriptor_kind_without_material_is_rejected(self):
        with self.assertRaises(AppError):
            _validated_wallet_config("descriptor", {})


class XpubWalletIsSyncableTests(unittest.TestCase):
    """Regression: a multi-script xpub wallet (no `descriptor`) must classify as a
    syncable, descriptor-backed wallet across the sync / snapshot gates — else its
    transactions never fetch and the gap limit is hidden."""

    def _config(self) -> dict:
        return {
            "xpub": _xpub(),
            "script_types": ["p2wpkh", "p2tr"],
            "chain": "bitcoin",
            "network": "main",
            "gap_limit": 40,
        }

    def test_has_descriptor_sync_material(self):
        self.assertTrue(has_descriptor_sync_material(self._config()))
        self.assertFalse(has_descriptor_sync_material({"xpub": _xpub()}))  # no types
        self.assertFalse(has_descriptor_sync_material({"addresses": ["bc1q"]}))

    def test_classify_wallet_sync_is_backend(self):
        wallet = {"kind": "xpub", "config_json": json.dumps(self._config())}
        self.assertEqual(classify_wallet_sync(wallet, normalize_addresses), "backend")

    def test_backend_summary_is_descriptor_sync_mode(self):
        summary = _wallet_backend_summary("xpub", self._config(), "mempool")
        self.assertEqual(summary["sync_mode"], "backend_descriptor")


class WalletConfigFreshnessTests(unittest.TestCase):
    def _db(self):
        tmp = tempfile.TemporaryDirectory(prefix="kassiber-wallet-config-")
        self.addCleanup(tmp.cleanup)
        conn = open_db(Path(tmp.name) / "data")
        self.addCleanup(conn.close)
        return conn

    def test_config_change_resets_onchain_freshness_checkpoint(self):
        conn = self._db()
        workspace = core_accounts.create_workspace(conn, "Main")
        profile = core_accounts.create_profile(
            conn,
            workspace["id"],
            "Book",
            "EUR",
            "FIFO",
            "generic",
            365,
        )
        wallet = create_wallet(
            conn,
            workspace["id"],
            profile["id"],
            "Vault",
            "xpub",
            config={
                "xpub": _xpub(),
                "script_types": ["p2wpkh", "p2tr"],
                "chain": "bitcoin",
                "network": "main",
                "gap_limit": 40,
            },
        )
        source_key = core_freshness.source_key(
            core_freshness.SOURCE_ONCHAIN,
            wallet["id"],
        )
        core_freshness.upsert_source_state(
            conn,
            profile_id=profile["id"],
            source_key=source_key,
            source_type=core_freshness.SOURCE_ONCHAIN,
            source_label="Vault on-chain history",
            status=core_freshness.STATUS_FRESH,
            checkpoint={
                "highest_used": {"4": 12, "6": 2},
                "esplora_scripthashes": {"abc": {"tx_count": 1}},
            },
        )

        update_wallet(
            conn,
            workspace["id"],
            profile["id"],
            wallet["id"],
            {"config": {"script_types": ["p2wpkh"]}},
        )

        state = core_freshness.get_source_state(conn, profile["id"], source_key)
        self.assertEqual(state["checkpoint"], {})
        self.assertEqual(state["stale_reason"], "wallet_config_changed")
        self.assertEqual(state["status"], core_freshness.STATUS_PARTIALLY_STALE)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
