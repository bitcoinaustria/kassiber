"""Opt-in live Liquid (Elements) regtest wallet-sync tests.

Skipped by default. Set ``KASSIBER_LIVE_SYNC_TESTS=1`` to enable. Use
``KASSIBER_REQUIRE_LIQUID_REGTEST=1`` to turn "Docker or image not available"
into a hard failure instead of a skip.

A single elementsd + electrs-liquid stack is shared across the module
(``setUpModule`` / ``tearDownModule``). Each test gets its own temporary
Kassiber data root.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

from tests.live_sync import (
    DockerUnavailable,
    LIVE_SYNC_PULL,
    LIVE_SYNC_TESTS,
    REQUIRE_LIQUID_REGTEST,
    assert_kassiber_ok,
    run_kassiber_json,
)
from tests.live_sync.liquid import LiquidRegtestStack


_stack: LiquidRegtestStack | None = None
_skip_reason: str | None = None


def setUpModule() -> None:
    global _stack, _skip_reason
    if not LIVE_SYNC_TESTS:
        _skip_reason = "set KASSIBER_LIVE_SYNC_TESTS=1 to run local live sync integration tests"
        return
    stack = LiquidRegtestStack(allow_pull=LIVE_SYNC_PULL)
    try:
        stack.start()
    except DockerUnavailable as exc:
        if REQUIRE_LIQUID_REGTEST:
            raise AssertionError(str(exc)) from exc
        _skip_reason = str(exc)
        return
    _stack = stack


def tearDownModule() -> None:
    global _stack
    if _stack:
        _stack.stop()
        _stack = None


class LiveLiquidTestCase(unittest.TestCase):
    def setUp(self) -> None:
        if _stack is None:
            self.skipTest(_skip_reason or "liquid regtest stack not available")
        self.stack = _stack
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-liquid-live-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"

    def run(self, result=None):
        if result is None:
            result = self.defaultTestResult()
        before_errors = list(result.errors)
        before_failures = list(result.failures)
        super().run(result)
        new_failure = (
            result.errors[len(before_errors):]
            or result.failures[len(before_failures):]
        )
        if new_failure and getattr(self, "stack", None) is not None:
            logs = self.stack.dump_logs()
            print(
                f"\n--- elementsd + electrs-liquid logs for {self.id()} ---\n{logs}\n",
                file=sys.stderr,
            )
        return result


class LiquidBlindedDescriptorWalletTest(LiveLiquidTestCase):
    def test_ct_slip77_elwpkh_descriptor_wallet_syncs(self) -> None:
        descriptors = self.stack.mint_blinded_descriptor(label="kassiber-live")

        payload, result = run_kassiber_json(self.data_root, "init")
        assert_kassiber_ok(self, payload, result, "init")
        payload, result = run_kassiber_json(
            self.data_root, "workspaces", "create", "Main"
        )
        assert_kassiber_ok(self, payload, result, "workspaces.create")
        payload, result = run_kassiber_json(
            self.data_root,
            "profiles",
            "create",
            "--workspace",
            "Main",
            "Default",
        )
        assert_kassiber_ok(self, payload, result, "profiles.create")

        payload, result = run_kassiber_json(
            self.data_root,
            "backends",
            "create",
            "liquid-local",
            "--kind",
            "electrum",
            "--url",
            self.stack.electrum_url(),
            "--chain",
            "liquid",
            "--network",
            "elementsregtest",
        )
        assert_kassiber_ok(self, payload, result, "backends.create")

        receive_path = self.data_root.parent / "receive.desc"
        change_path = self.data_root.parent / "change.desc"
        receive_path.write_text(descriptors["receive_descriptor"])
        change_path.write_text(descriptors["change_descriptor"])

        policy_asset = self.stack.policy_asset_id()

        payload, result = run_kassiber_json(
            self.data_root,
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "LiquidRegtest",
            "--kind",
            "descriptor",
            "--backend",
            "liquid-local",
            "--chain",
            "liquid",
            "--network",
            "elementsregtest",
            "--descriptor-file",
            str(receive_path),
            "--change-descriptor-file",
            str(change_path),
            "--gap-limit",
            "5",
            "--policy-asset",
            policy_asset,
        )
        assert_kassiber_ok(self, payload, result, "wallets.create")

        derived = run_kassiber_json(
            self.data_root,
            "wallets",
            "derive",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "LiquidRegtest",
            "--count",
            "1",
        )
        assert_kassiber_ok(self, derived[0], derived[1], "wallets.derive")
        addresses = derived[0]["data"].get("addresses") or derived[0]["data"]
        receive_address = addresses[0]["address"] if isinstance(addresses, list) else addresses[0]
        self.assertTrue(receive_address, "expected at least one derived address")

        self.stack.send_to(receive_address, 1.5)

        payload, result = run_kassiber_json(
            self.data_root,
            "wallets",
            "sync",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "LiquidRegtest",
        )
        assert_kassiber_ok(self, payload, result, "wallets.sync")
        self.assertEqual(len(payload["data"]), 1)
        sync = payload["data"][0]
        self.assertEqual(sync["status"], "synced")
        self.assertEqual(sync["chain"], "liquid")
        self.assertEqual(sync["network"], "elementsregtest")
        self.assertEqual(sync["sync_mode"], "descriptor")
        self.assertGreater(sync["target_count"], 0)
        self.assertGreaterEqual(sync["imported"], 1)

        payload, result = run_kassiber_json(
            self.data_root,
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "LiquidRegtest",
        )
        assert_kassiber_ok(self, payload, result, "transactions.list")
        rows = payload["data"]
        self.assertTrue(rows, "expected at least one synced transaction")
        tx = rows[0]
        self.assertEqual(tx["direction"], "inbound")
        self.assertEqual(tx["asset"], "LBTC")
        self.assertAlmostEqual(tx["amount"], 1.5, places=8)


if __name__ == "__main__":
    unittest.main()
