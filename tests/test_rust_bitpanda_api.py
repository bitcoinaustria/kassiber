"""Bitpanda API sync (Rust only): every schema table, plaintext and SQLCipher.

Bitpanda has no Python reference importer, so the book each sync leaves is
pinned here: which tables change, the rows written, and identical bytes on
both storage formats. The exchange fixture driver runs the Rust sync against
a loopback fake of Bitpanda's documented shapes
(`tests/rust_port/exchange_api.py`, docs/reference/bitpanda.md).
"""
import json
import subprocess
import uuid

import pytest

from tests.rust_port.exchange_api import Bitpanda, serve
from tests.rust_sync_env import child_env
from tests.test_rust_exchange_api import TABLES, books, connect, driver  # noqa: F401 - driver is a fixture
from tests.test_rust_store_migrations import normalized_dump

TIME = "2026-11-08T12:00:00Z"
PASSPHRASE = "synthetic-exchange-book"
IDS = [str(uuid.UUID(int=500 + n)) for n in range(64)]
REQUESTS = [
    {"method": "GET", "path": "/v1/trading-limits", "params": []},
    {"method": "GET", "path": "/v1/assets", "params": [("page_size", "100"), ("symbol", "BTC")]},
    {"method": "GET", "path": "/v1/currencies", "params": []},
    {"method": "GET", "path": "/v1/operations", "params": [("page_size", "100")]},
    {"method": "GET", "path": "/v1/operations", "params": [("cursor", "c2"), ("page_size", "100")]},
    {"method": "GET", "path": "/v1/operations", "params": [("cursor", "c4"), ("page_size", "100")]},
    {"method": "GET", "path": "/v1/operations", "params": [("cursor", "c6"), ("page_size", "100")]},
]
# external_id, direction, kind, amount (msat), fee (msat), fiat value, pricing method, occurred_at
# The new wallet and its rows, the journal invalidation on the profile and the
# chain index's dirty marks; nothing else.
CHANGED = ["chain_index_clock", "chain_index_dirty", "profiles", "transactions", "wallets"]
ROWS = [
    ("bitpanda:t-buy", "inbound", "buy", 1_000_000_000, 0, "401.49", "bitpanda_api", "2024-01-02T09:00:00Z"),
    ("bitpanda:t-deposit", "inbound", "deposit", 500_000_000, 0, None, None, "2024-01-03T00:00:00Z"),
    ("bitpanda:t-withdrawal", "outbound", "withdrawal", 400_000_000, 10_000_000, None, None, "2024-01-05T00:00:00Z"),
    ("bitpanda:t-sell", "outbound", "sell", 200_000_000, 0, "84.00", "bitpanda_api", "2024-01-06T00:00:00Z"),
    ("bitpanda:t-reward", "inbound", "income", 1_000_000, 0, None, None, "2024-01-07T00:00:00Z"),
]


def sync(driver, tmp_path, path, encrypted, backend):
    request = {"book": str(path), "backend": backend, "wallet": "", "ids": IDS, "timestamp": TIME,
               "expected": "bitpanda"}
    if encrypted:
        request["passphrase"] = PASSPHRASE
    result = subprocess.run([str(driver)], input=json.dumps(request).encode(), capture_output=True,
                            env=child_env(tmp_path), timeout=120)
    assert result.returncode == 0 and result.stderr == b"", result.stderr.decode()
    return json.loads(result.stdout)


def dump(path, encrypted):
    conn = connect(path, encrypted)
    try:
        tables = normalized_dump(conn)[1]
    finally:
        conn.close()
    assert len(tables) == TABLES
    return tables


def changed(before, after):
    return sorted(table for table in before if before[table] != after[table])


def bitpanda_backend(backend):
    # Bitpanda authenticates with the key alone.
    return {key: value for key, value in backend.items() if key != "auth_header"}


def test_sync_resync_and_refusal_write_the_same_tables_on_both_formats(driver, tmp_path):
    books_after = {}
    for encrypted in (False, True):
        root = tmp_path / ("sqlcipher" if encrypted else "sqlite")
        root.mkdir()
        path = books(root, encrypted)[1]
        exchange = Bitpanda()
        with serve(exchange) as backend:
            backend = bitpanda_backend(backend)
            before = dump(path, encrypted)
            first = sync(driver, root, path, encrypted, backend)
            assert first["kind"] == "wallets.sync-bitpanda", first
            data = first["data"]
            assert (data["imported"], data["fetched"], data["input_format"]) == (5, 5, "bitpanda_api")
            assert data["skipped"] == {"fiat_only_operations": 1, "non_btc_operations": 1, "internal_btc_moves": 0}
            assert data["backend_kind"] == "bitpanda" and data["wallet"] == "Bitpanda"
            assert exchange.requests == REQUESTS
            after = dump(path, encrypted)
            assert changed(before, after) == CHANGED, changed(before, after)
            conn = connect(path, encrypted)
            rows = [tuple(row) for row in conn.execute(
                "SELECT external_id, direction, kind, amount, fee, fiat_value_exact, pricing_method, occurred_at "
                "FROM transactions ORDER BY occurred_at")]
            wallet = tuple(conn.execute("SELECT label, kind, config_json FROM wallets WHERE kind='bitpanda'").fetchone())
            conn.close()
            assert rows == ROWS
            assert wallet == ("Bitpanda", "bitpanda", '{"backend": "fixture"}')

            # A re-sync imports nothing new and leaves the rows as they were.
            exchange.reset()
            second = sync(driver, root, path, encrypted, backend)
            assert second["data"]["imported"] == 0, second
            again = dump(path, encrypted)
            assert again["transactions"] == after["transactions"]
            assert again["wallets"] == after["wallets"]

            # A key that can trade is refused after the scope check alone,
            # without touching the book.
            exchange.reset()
            exchange.limits = (404, {"error": {"code": "not_found"}})
            refused = sync(driver, root, path, encrypted, backend)
            assert refused["error"]["code"] == "auth_error", refused
            assert [r["path"] for r in exchange.requests] == ["/v1/trading-limits"]
            assert dump(path, encrypted) == again
        books_after[encrypted] = (after, again)
    assert books_after[False] == books_after[True]
