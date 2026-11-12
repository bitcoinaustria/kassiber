"""The Rust binary's signed, paginated Coinbase import against a loopback fixture.

The fixture (`regtest_exchange_api.py`) verifies the HMAC signature of every
request; nothing reaches a real exchange.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import tempfile
import time
import unittest
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from tests.integration import cli
from tests.integration.book import open_book
from tests.integration.env import no_egress_guard
from tests.integration.regtest_exchange_api import CoinbaseApiFixture, run_exchange_api_case


def _cli(data_root: Path, *args, check: bool = True):
    result = cli.run(["--data-root", str(data_root), "--machine", *args],
                     capture_output=True, text=True, check=False, timeout=300)
    if check and result.returncode != 0:
        raise AssertionError(f"kassiber {args[:2]} failed\nstdout={result.stdout}\nstderr={result.stderr}")
    return result


@unittest.skipUnless(cli.configured(), cli.NO_BINARY)
class RegtestExchangeApiTest(unittest.TestCase):
    def test_real_signed_paginated_import_and_refresh_preserve_source_ledger(self):
        with tempfile.TemporaryDirectory() as tmp, no_egress_guard(enabled=True):
            root = Path(tmp)
            _cli(root, "init")
            _cli(root, "workspaces", "create", "Protocol test")
            main_profile = json.loads(_cli(
                root, "profiles", "create", "Main demo", "--workspace", "Protocol test",
                "--fiat-currency", "EUR", "--gains-algorithm", "FIFO", "--tax-country", "generic",
                "--tax-long-term-days", "365",
            ).stdout)["data"]
            report = run_exchange_api_case(root, "Protocol test", "ab" * 32, "cd" * 32)
            self.assertEqual(report["source_ledger_balance_btc"], "0.005")
            self.assertEqual(report["transaction_count"], 4)
            self.assertEqual(report["refresh_added"], 0)
            self.assertEqual(report["authenticated_requests"], 12)
            self.assertEqual(report["pages_per_refresh"], 6)
            self.assertFalse(report["provider_running"])
            self.assertEqual(report["evidence_kind"], "synthetic_coinbase_api_protocol")
            conn = open_book(root, readonly=True)
            try:
                # The case books into its own profile and leaves the main one
                # untouched. (`profiles create` selects the new profile, so the
                # demo selects its own again afterwards, `run_cli context set`.)
                self.assertNotEqual(report["profile_id"], main_profile["id"])
                self.assertEqual(conn.execute(
                    "SELECT COUNT(*) FROM transactions WHERE profile_id = ?", (main_profile["id"],),
                ).fetchone()[0], 0)
                rows = conn.execute(
                    "SELECT external_id, direction, amount, fee FROM transactions WHERE profile_id = ?",
                    (report["profile_id"],),
                ).fetchall()
                amounts = {row["external_id"]: (row["direction"], row["amount"], row["fee"]) for row in rows}
                self.assertEqual(amounts["ab" * 32], ("outbound", 600000000, 0))
                self.assertEqual(amounts["cd" * 32], ("inbound", 200000000, 0))
                self.assertEqual(conn.execute("SELECT COUNT(*) FROM chain_observation_provenance").fetchone()[0], 0)
                backend = conn.execute("SELECT token, auth_header FROM backends WHERE kind = 'coinbase'").fetchone()
                self.assertTrue(backend["token"] and backend["auth_header"])
                self.assertNotIn(backend["token"], json.dumps(report))
                self.assertNotIn(backend["auth_header"], json.dumps(report))
                buy = conn.execute(
                    "SELECT fiat_value_exact, pricing_provider FROM transactions WHERE external_id = 'coinbase:fixture-buy'"
                ).fetchone()
                self.assertEqual(buy["fiat_value_exact"], "1001")
                self.assertEqual(buy["pricing_provider"], "Coinbase")
            finally:
                conn.close()

    def test_fixture_checks_signature_of_exact_pagination_query(self):
        with no_egress_guard(enabled=True), CoinbaseApiFixture([]) as provider:
            timestamp = str(int(time.time()))
            path = "/v2/accounts?page=2"
            signature = hmac.new(
                provider.api_secret.encode(), f"{timestamp}GET/v2/accounts".encode(), hashlib.sha256,
            ).hexdigest()
            request = Request(provider.url + path, headers={
                "CB-ACCESS-KEY": provider.api_key,
                "CB-ACCESS-SIGN": signature,
                "CB-ACCESS-TIMESTAMP": timestamp,
            })
            with self.assertRaises(HTTPError) as raised:
                urlopen(request, timeout=3)
            self.assertEqual(raised.exception.code, 401)
            self.assertEqual(provider.requests, [{"path": path, "status": 401}])

    def test_genuine_fetcher_does_not_import_with_invalid_credentials(self):
        with tempfile.TemporaryDirectory() as tmp, no_egress_guard(enabled=True), \
                CoinbaseApiFixture([]) as provider:
            root = Path(tmp)
            _cli(root, "init")
            _cli(root, "workspaces", "create", "Protocol test")
            _cli(root, "profiles", "create", "Main", "--workspace", "Protocol test",
                 "--fiat-currency", "EUR", "--tax-country", "generic")
            _cli(root, "backends", "create", "coinbase-bad", "--kind", "coinbase", "--url", provider.url,
                 "--timeout", "3",
                 "--token-fd", cli.InputFd(provider.api_key.encode()),
                 "--auth-header-fd", cli.InputFd(b"incorrect-disposable-secret"))
            refused = _cli(root, "wallets", "sync-coinbase", "--backend", "coinbase-bad", check=False)
            self.assertNotEqual(refused.returncode, 0, refused.stdout)
            self.assertEqual(json.loads(refused.stdout)["kind"], "error")
            self.assertEqual(provider.requests, [{"path": "/v2/accounts", "status": 401}])
            conn = open_book(root, readonly=True)
            try:
                self.assertEqual(conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0], 0)
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
