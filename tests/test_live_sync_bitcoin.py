"""Opt-in live Bitcoin Core regtest wallet-sync tests.

Skipped by default. Set ``KASSIBER_LIVE_SYNC_TESTS=1`` to enable. Use
``KASSIBER_REQUIRE_BITCOIN_REGTEST=1`` to turn "Docker not available" into a
hard failure instead of a skip.

A single bitcoind container is shared across the module's tests
(``setUpModule`` / ``tearDownModule``). Each test gets its own temporary
Kassiber data root so state does not bleed between tests.
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
    REQUIRE_BITCOIN_REGTEST,
    assert_kassiber_ok,
    run_kassiber_json,
)
from tests.live_sync.bitcoin import BitcoinRegtestStack


_stack: BitcoinRegtestStack | None = None
_skip_reason: str | None = None


def setUpModule() -> None:
    global _stack, _skip_reason
    if not LIVE_SYNC_TESTS:
        _skip_reason = "set KASSIBER_LIVE_SYNC_TESTS=1 to run local live sync integration tests"
        return
    stack = BitcoinRegtestStack(allow_pull=LIVE_SYNC_PULL)
    try:
        stack.start()
    except DockerUnavailable as exc:
        if REQUIRE_BITCOIN_REGTEST:
            raise AssertionError(str(exc)) from exc
        _skip_reason = str(exc)
        return
    _stack = stack


def tearDownModule() -> None:
    global _stack
    if _stack:
        _stack.stop()
        _stack = None


class LiveBitcoinTestCase(unittest.TestCase):
    def setUp(self) -> None:
        if _stack is None:
            self.skipTest(_skip_reason or "bitcoin regtest stack not available")
        self.stack = _stack
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-bitcoin-live-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        self._failed = False

    def run(self, result=None):  # capture failure state for log dump
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
            print(f"\n--- bitcoind logs for {self.id()} ---\n{logs}\n", file=sys.stderr)
        return result

    # ------------------------------------------------------------------ helpers

    def _bootstrap_workspace(self, backend_name: str = "regtest-core") -> None:
        payload, result = run_kassiber_json(self.data_root, "init")
        assert_kassiber_ok(self, payload, result, "init")
        payload, result = run_kassiber_json(self.data_root, "workspaces", "create", "Main")
        assert_kassiber_ok(self, payload, result, "workspaces.create")
        payload, result = run_kassiber_json(
            self.data_root, "profiles", "create", "--workspace", "Main", "Default"
        )
        assert_kassiber_ok(self, payload, result, "profiles.create")
        payload, result = run_kassiber_json(
            self.data_root,
            "backends",
            "create",
            backend_name,
            "--kind",
            "bitcoinrpc",
            "--url",
            self.stack.rpc.url,
            "--chain",
            "bitcoin",
            "--network",
            "regtest",
            "--username",
            self.stack.username,
            "--password",
            self.stack.password,
            "--wallet-prefix",
            "live-regtest",
        )
        assert_kassiber_ok(self, payload, result, "backends.create")


class BitcoinRegtestAddressWalletTest(LiveBitcoinTestCase):
    def test_address_wallet_syncs_real_receive_and_is_idempotent(self) -> None:
        watch_address = self.stack.new_watch_address()
        txid = self.stack.send_to(watch_address, 0.25)

        self._bootstrap_workspace()
        payload, result = run_kassiber_json(
            self.data_root,
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "OnchainRegtest",
            "--kind",
            "address",
            "--backend",
            "regtest-core",
            "--chain",
            "bitcoin",
            "--network",
            "regtest",
            "--address",
            watch_address,
        )
        assert_kassiber_ok(self, payload, result, "wallets.create")

        payload, result = run_kassiber_json(
            self.data_root,
            "wallets",
            "sync",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "OnchainRegtest",
        )
        assert_kassiber_ok(self, payload, result, "wallets.sync")
        self.assertEqual(len(payload["data"]), 1)
        first = payload["data"][0]
        self.assertEqual(first["status"], "synced")
        self.assertEqual(first["backend_kind"], "bitcoinrpc")
        self.assertEqual(first["chain"], "bitcoin")
        self.assertEqual(first["network"], "regtest")
        self.assertEqual(first["sync_mode"], "addresses")
        self.assertEqual(first["imported"], 1)
        self.assertEqual(first["skipped"], 0)
        self.assertEqual(first["imported_addresses"], 1)

        payload, result = run_kassiber_json(
            self.data_root,
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "OnchainRegtest",
        )
        assert_kassiber_ok(self, payload, result, "transactions.list")
        self.assertEqual(len(payload["data"]), 1)
        tx = payload["data"][0]
        self.assertEqual(tx["external_id"], txid)
        self.assertEqual(tx["direction"], "inbound")
        self.assertEqual(tx["asset"], "BTC")
        self.assertAlmostEqual(tx["amount"], 0.25, places=8)
        self.assertEqual(tx["fee"], 0.0)
        self.assertTrue(tx["confirmed_at"])

        # Idempotency: second sync must not re-import or double-count.
        payload, result = run_kassiber_json(
            self.data_root,
            "wallets",
            "sync",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "OnchainRegtest",
        )
        assert_kassiber_ok(self, payload, result, "wallets.sync")
        second = payload["data"][0]
        self.assertEqual(second["imported"], 0)
        self.assertEqual(second["skipped"], 1)
        self.assertEqual(second["imported_addresses"], 0)


if __name__ == "__main__":
    unittest.main()
