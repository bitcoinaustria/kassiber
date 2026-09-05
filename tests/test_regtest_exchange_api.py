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

from kassiber.core import accounts
from kassiber.core.exchange_imports import fetch_coinbase_records
from kassiber.db import get_setting, open_db
from kassiber.errors import AppError
from tests.integration.env import no_egress_guard
from tests.integration.regtest_exchange_api import CoinbaseApiFixture, run_exchange_api_case


class RegtestExchangeApiTest(unittest.TestCase):
    def test_real_signed_paginated_import_and_refresh_preserve_source_ledger(self):
        with tempfile.TemporaryDirectory() as tmp, no_egress_guard(enabled=True):
            conn = open_db(Path(tmp))
            workspace = accounts.create_workspace(conn, "Protocol test")
            main_profile = accounts.create_profile(conn, workspace["id"], "Main demo", "EUR", "FIFO", "generic", 365)
            conn.close()
            report = run_exchange_api_case(Path(tmp), "Protocol test", "ab" * 32, "cd" * 32)
            self.assertEqual(report["source_ledger_balance_btc"], "0.005")
            self.assertEqual(report["transaction_count"], 4)
            self.assertEqual(report["refresh_added"], 0)
            self.assertEqual(report["authenticated_requests"], 12)
            self.assertEqual(report["pages_per_refresh"], 6)
            self.assertFalse(report["provider_running"])
            self.assertEqual(report["evidence_kind"], "synthetic_coinbase_api_protocol")
            conn = open_db(Path(tmp))
            try:
                self.assertEqual(get_setting(conn, "context_profile"), main_profile["id"])
                self.assertNotEqual(report["profile_id"], main_profile["id"])
                rows = conn.execute(
                    "SELECT external_id, direction, amount, fee FROM transactions WHERE profile_id = ?",
                    (report["profile_id"],),
                ).fetchall()
                amounts = {row["external_id"]: (row["direction"], row["amount"], row["fee"]) for row in rows}
                self.assertEqual(amounts["ab" * 32], ("outbound", 600000000, 0))
                self.assertEqual(amounts["cd" * 32], ("inbound", 200000000, 0))
                self.assertEqual(conn.execute("SELECT COUNT(*) FROM chain_observation_provenance").fetchone()[0], 0)
                backend = conn.execute("SELECT token, auth_header FROM backends").fetchone()
                self.assertNotIn(backend["token"], json.dumps(report))
                self.assertNotIn(backend["auth_header"], json.dumps(report))
                buy = conn.execute("SELECT fiat_value_exact, pricing_provider FROM transactions WHERE external_id = 'coinbase:fixture-buy'").fetchone()
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
        with no_egress_guard(enabled=True), CoinbaseApiFixture([]) as provider:
            with self.assertRaises(AppError):
                fetch_coinbase_records({
                    "url": provider.url, "token": provider.api_key,
                    "auth_header": "incorrect-disposable-secret", "timeout": 3,
                })
            self.assertEqual(provider.requests, [{"path": "/v2/accounts", "status": 401}])


if __name__ == "__main__":
    unittest.main()
