"""Synthetic loopback Coinbase protocol fixture, never a live exchange integration.

The genuine Kassiber connection/import path performs signed HTTP requests and
pagination. Callers may reuse actual regtest transaction references; the API
ledger itself is synthetic and cannot attest chain ownership. The HTTP server
stops after the case, so its persisted backend is explicitly a stopped fixture.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import re
import secrets
import threading
import time
from decimal import Decimal
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

from kassiber import backends
from kassiber.cli.handlers import import_exchange_api
from kassiber.core import accounts
from kassiber.db import get_setting, open_db, set_setting
from kassiber.msat import MSAT_PER_BTC
from tests.integration.env import no_egress_guard


class CoinbaseApiFixture:
    """Small strict API fixture with disposable credentials and safe request audit."""

    def __init__(self, transactions: list[dict]):
        self.transactions = json.loads(json.dumps(transactions))
        self.api_key = secrets.token_hex(16)
        self.api_secret = secrets.token_hex(32)
        self.requests: list[dict] = []
        self.url = ""
        self._server = None
        self._thread = None

    def __enter__(self):
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass  # Never log headers, signatures, or disposable credentials.

            def do_GET(self):
                timestamp = self.headers.get("CB-ACCESS-TIMESTAMP", "")
                signature = hmac.new(
                    fixture.api_secret.encode(),
                    f"{timestamp}GET{self.path}".encode(), hashlib.sha256,
                ).hexdigest()
                fresh = timestamp.isdecimal() and abs(time.time() - int(timestamp)) < 60
                authenticated = (
                    fresh
                    and hmac.compare_digest(self.headers.get("CB-ACCESS-KEY", ""), fixture.api_key)
                    and hmac.compare_digest(self.headers.get("CB-ACCESS-SIGN", ""), signature)
                )
                if not authenticated:
                    status, payload = 401, {"errors": [{"id": "authentication_error"}]}
                else:
                    status, payload = fixture._page(self.path)
                fixture.requests.append({"path": self.path, "status": status})
                body = json.dumps(payload).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._server.daemon_threads = True
        self.url = f"http://127.0.0.1:{self._server.server_port}"
        self._thread = threading.Thread(target=self._server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, *_args):
        if self._server:
            self._server.shutdown()
            self._server.server_close()
        if self._thread:
            self._thread.join(timeout=3)

    def _page(self, raw_path):
        parsed = urlsplit(raw_path)
        query = parse_qs(parsed.query)
        page_text = query.get("page", ["1"])
        if set(query) - {"page"} or len(page_text) != 1 or not page_text[0].isdecimal():
            return 400, {"errors": [{"id": "invalid_page"}]}
        page = int(page_text[0])
        if parsed.path == "/v2/accounts":
            rows = [{"id": "fixture-eur", "currency": {"code": "EUR"}},
                    {"id": "fixture-btc", "currency": {"code": "BTC"}}]
        elif parsed.path == "/v2/accounts/fixture-btc/transactions":
            rows = self.transactions
        else:
            return 404, {"errors": [{"id": "not_found"}]}
        if page < 1 or page > max(1, len(rows)):
            return 400, {"errors": [{"id": "invalid_page"}]}
        next_uri = f"{parsed.path}?page={page + 1}" if page < len(rows) else None
        return 200, {"data": rows[page - 1:page], "pagination": {"next_uri": next_uri}}


def _positive_btc(value):
    amount = Decimal(str(value))
    if not amount.is_finite() or amount <= 0 or amount * 100000000 != (amount * 100000000).to_integral_value():
        raise ValueError("Fixture quantities must be positive exact Bitcoin satoshi amounts")
    return amount


def run_exchange_api_case(
    data_root: Path,
    workspace_label: str,
    withdrawal_txid: str,
    deposit_txid: str,
    *,
    withdrawal_btc="0.006",
    deposit_btc="0.002",
    times: dict | None = None,
) -> dict:
    """Import and refresh a disposable synthetic API book using real CLI handlers.

    Supplied txids are references, not verified here. The demo orchestrator must
    obtain them from Core; standalone tests use explicitly synthetic references.
    Pass a separate temporary data root and retain only the returned report.
    No report-ready or complete custody claim is made for this isolated ledger.
    """
    if any(not re.fullmatch(r"[0-9a-f]{64}", txid) for txid in (withdrawal_txid, deposit_txid)):
        raise ValueError("Expected canonical regtest transaction references")
    withdrawal = _positive_btc(withdrawal_btc)
    deposit = _positive_btc(deposit_btc)
    timestamps = {
        "buy_time": "2025-01-01T00:00:00Z", "withdrawal_time": "2025-01-02T00:00:00Z",
        "deposit_time": "2025-01-03T00:00:00Z", "sell_time": "2025-01-04T00:00:00Z",
        **(times or {}),
    }
    transactions = [
        {"id": "fixture-buy", "type": "buy", "created_at": timestamps["buy_time"],
         "amount": {"amount": "0.01", "currency": "BTC"},
         "native_amount": {"amount": "1000", "currency": "EUR"}, "buy": {"commission": "1"}},
        {"id": "fixture-withdrawal", "type": "send", "created_at": timestamps["withdrawal_time"],
         "amount": {"amount": str(-withdrawal), "currency": "BTC"}, "network": {"hash": withdrawal_txid}},
        {"id": "fixture-deposit", "type": "exchange_deposit", "created_at": timestamps["deposit_time"],
         "amount": {"amount": str(deposit), "currency": "BTC"}, "network": {"hash": deposit_txid}},
        {"id": "fixture-sell", "type": "sell", "created_at": timestamps["sell_time"],
         "amount": {"amount": "-0.001", "currency": "BTC"},
         "native_amount": {"amount": "149", "currency": "EUR"}},
    ]
    conn = open_db(data_root)
    prior_context = {key: get_setting(conn, key) for key in ("context_workspace", "context_profile")}
    try:
        workspace = conn.execute("SELECT * FROM workspaces WHERE label = ?", (workspace_label,)).fetchone()
        if workspace is None:
            workspace = accounts.create_workspace(conn, workspace_label)
        profile = accounts.create_profile(
            conn, workspace["id"], "Coinbase API protocol fixture (stopped)", "EUR", "FIFO", "generic", 365,
        )
        backend_name = "coinbase-protocol-" + profile["id"][:8]
        with no_egress_guard(enabled=True), CoinbaseApiFixture(transactions) as provider:
            accounts.create_backend(
                conn, backend_name, "coinbase", provider.url,
                token=provider.api_key, auth_header=provider.api_secret, timeout=3,
                notes="Synthetic loopback protocol validation fixture. Server stops after validation; not a live exchange connection.",
            )
            runtime = backends.merge_db_backends(conn, {
                "env_file": str(data_root / "unused.env"), "backends": {},
                "bootstrap_backends": {}, "dotenv_backends": [],
                "process_env_overrides": {"backends": {}, "default_backend": False},
            })
            outcomes = [import_exchange_api(
                conn, runtime, workspace["id"], profile["id"], backend_name,
                expected_backend_kind="coinbase",
            ) for _ in range(2)]
            requests = list(provider.requests)
        rows = conn.execute(
            "SELECT id, wallet_id, external_id, direction, amount, fee FROM transactions WHERE profile_id = ? ORDER BY occurred_at",
            (profile["id"],),
        ).fetchall()
        balance_msat = sum(row["amount"] if row["direction"] == "inbound" else -row["amount"] - row["fee"] for row in rows)
        expected = Decimal("0.01") - withdrawal + deposit - Decimal("0.001")
        actual = Decimal(balance_msat) / MSAT_PER_BTC
        if len(rows) != 4 or outcomes[1]["imported"] != 0 or actual != expected:
            raise AssertionError("Synthetic exchange refresh did not preserve the expected source ledger")
        if len(requests) != 12 or any(item["status"] != 200 for item in requests):
            raise AssertionError("Synthetic exchange pagination/authentication coverage incomplete")
        return {
            "id": "coinbase-api-protocol", "evidence_kind": "synthetic_coinbase_api_protocol",
            "book_scope": "separate_disposable_protocol_book", "interactive_demo_profile": False,
            "title": "Coinbase API protocol validation (local fixture, stopped)",
            "profile_id": profile["id"], "profile": profile["label"], "wallet_id": rows[0]["wallet_id"],
            "transaction_ids": [row["id"] for row in rows],
            "txids": {"withdrawal": withdrawal_txid, "deposit": deposit_txid},
            "transaction_count": len(rows), "first_imported": outcomes[0]["imported"],
            "refresh_added": outcomes[1]["imported"], "authenticated_requests": len(requests),
            "pages_per_refresh": 6, "source_ledger_balance_btc": format(actual, "f"),
            "withdrawal_btc": str(withdrawal), "deposit_btc": str(deposit), "provider_running": False,
            "limitations": [
                "Synthetic provider validates the repository adapter protocol, not live Coinbase compatibility.",
                "Separate alternate protocol profile; do not add this ledger to the main demo book.",
                "Profile and transaction IDs are validation references, not links to the interactive demo; the caller discards this temporary book.",
                "API transaction references do not establish native blockchain authority.",
                "Movement fees are zero in this fixture; no exchange withdrawal fee coverage is claimed.",
                "No native wallet is connected in this profile; source-ledger balance is not a custody or tax report.",
                "The local API server has stopped; this fixture connection cannot perform ongoing refreshes.",
            ],
        }
    finally:
        for key, value in prior_context.items():
            if value is None:
                conn.execute("DELETE FROM settings WHERE key = ?", (key,))
            else:
                set_setting(conn, key, value)
        conn.commit()
        conn.close()
