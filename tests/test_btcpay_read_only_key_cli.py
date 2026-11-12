"""A read-only BTCPay key degrades label enrichment through the command line.

`ReadOnlyKeyLabelsTest` (test_btcpay_ledger_and_fees) checks the decision in
process. These tests run `wallets sync` through `tests/cli_runner.py`, so
`KASSIBER_TEST_CLI` can point them at the Rust binary (which serves these
BTCPay wallet syncs natively), against a loopback Greenfield fake whose
wallet endpoint answers 403 with a `missingPermission` in both the bare and
the store-scoped form.
"""

import json
import os
import tempfile
import unittest
from pathlib import Path
from urllib.parse import urlsplit

from tests import cli_runner
from tests.test_credential_echo import EchoServer

WALLET_PATH = "/api/v1/stores/S1/payment-methods/BTC-CHAIN/wallet/transactions"


def greenfield(missing):
    """Empty provenance pages; the wallet endpoint lacks ``missing``."""

    def respond(handler, _echo):
        path = urlsplit(handler.path).path
        if path == WALLET_PATH:
            body = {"code": "missing-permission", "message": "denied", "missingPermission": missing}
            return 403, "application/json", json.dumps(body)
        if path in {"/api/v1/stores/S1/invoices", "/api/v1/stores/S1/payouts", "/api/v1/stores/S1/pull-payments"}:
            return 200, "application/json", "[]"
        return 404, "application/json", json.dumps({"code": "not-found", "message": path})

    return respond


class BtcpayReadOnlyKeyCliTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-btcpay-read-only-cli-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        # Children must not resolve the owner's real app-data root.
        self.env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
        self.env.setdefault("KASSIBER_OFFLINE_PREFERENCE_FILE", str(Path(self.tmp.name) / "offline-mode.json"))
        self.cli("init")
        self.cli("workspaces", "create", "Main")
        self.cli("profiles", "create", "Book", "--fiat-currency", "EUR")
        # Label enrichment rides on a wallet's own source; an empty file is enough.
        source = Path(self.tmp.name) / "settlement.json"
        source.write_text("[]", encoding="utf-8")
        self.cli(
            "wallets", "create", "--label", "Settlement", "--kind", "custom",
            "--source-file", str(source), "--source-format", "json",
        )

    def cli(self, *args, secret=None, check=True):
        result = cli_runner.run(
            ["--data-root", str(self.data_root), "--machine", *args],
            capture_output=True,
            text=True,
            input=None if secret is None else f"{secret}\n",
            secret_input=secret is not None,
            env=self.env,
            check=False,
        )
        if check:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def sync(self, missing):
        with EchoServer(greenfield(missing)) as server:
            self.cli("backends", "create", "shop", "--kind", "btcpay", "--url", server.url, "--token-stdin", secret="k" * 40)
            self.cli("wallets", "attach-btcpay", "--wallet", "Settlement", "--backend", "shop", "--store-id", "S1")
            result = self.cli("wallets", "sync", "--wallet", "Settlement", check=False)
            paths = [urlsplit(request["path"]).path for request in server.requests]
        self.assertIn(WALLET_PATH, paths)
        return result, json.loads(result.stdout)

    def assertReadOnly(self, missing):
        result, payload = self.sync(missing)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        (wallet,) = payload["data"]
        outcome = wallet["btcpay_provenance"]
        self.assertEqual(outcome["labels_unavailable_routes"], 1)
        (route,) = outcome["route_results"]
        self.assertEqual(route["labels"], "unavailable_read_only_key")
        self.assertEqual(route["invoice_provenance"]["status"], "synced")

    def assertFails(self, missing):
        result, payload = self.sync(missing)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(payload["error"]["code"], "auth_error")
        self.assertEqual(payload["error"]["details"]["missing_permission"], missing)

    def test_unscoped_permission_means_a_read_only_key(self):
        self.assertReadOnly("btcpay.store.canmodifystoresettings")

    def test_this_store_scoped_permission_means_a_read_only_key(self):
        self.assertReadOnly("btcpay.store.canmodifystoresettings:S1")

    def test_another_store_scope_still_fails(self):
        self.assertFails("btcpay.store.canmodifystoresettings:S2")

    def test_another_permission_scoped_to_this_store_still_fails(self):
        self.assertFails("btcpay.store.canviewstoresettings:S1")


if __name__ == "__main__":
    unittest.main()
