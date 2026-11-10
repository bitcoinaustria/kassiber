"""Local full-dump pricing oracle; all database copies stay under target/."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import sqlite3
from unittest.mock import patch

from kassiber.cli.handlers import auto_price_transactions_from_rates_cache
from tests.rust_port.journal_persistence_oracle import dump

ROOT = Path(__file__).resolve().parents[2]
PATH = ROOT / "crates/journals/tests/data/prebuild-pricing.json"
CLOCK = "2026-10-31T12:00:00Z"


def seed(conn):
    conn.execute("BEGIN")
    manifest = json.loads((ROOT / "crates/store/fixtures/current-schema.json").read_text())
    for kind in ("table", "index", "view", "trigger"):
        for obj in manifest["objects"]:
            if obj[0] == kind and obj[3]:
                conn.execute(obj[3])
    conn.execute("INSERT INTO chain_index_clock(id,revision) VALUES(1,0)")
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','W',?)", (CLOCK,))
    for profile in ("p", "other"):
        conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES(?,'w',?,'EUR',?)", (profile, profile, CLOCK))
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES(?,'w',?,?,'custom',?)", (profile, profile, profile, CLOCK))
    conn.commit()


def transaction(conn, id="t", **overrides):
    row = dict(id=id, workspace_id="w", profile_id="p", wallet_id="p", fingerprint=id,
               occurred_at="2025-01-01T12:00:00Z", confirmed_at=None, direction="inbound",
               asset="BTC", amount=1234567890123, fiat_currency=None, created_at=CLOCK)
    row.update(overrides)
    conn.execute(f"INSERT INTO transactions({','.join(row)}) VALUES({','.join('?' for _ in row)})", tuple(row.values()))


def rate(conn, **overrides):
    row = dict(pair="BTC-EUR", timestamp="2025-01-01T00:00:00Z", rate=43210.123456789,
               rate_exact="43210.123456789012345678901234567890123456789",
               source="coinbase-exchange", fetched_at=CLOCK, granularity="minute", method="sample")
    row.update(overrides)
    conn.execute(f"INSERT INTO rates_cache({','.join(row)}) VALUES({','.join('?' for _ in row)})", tuple(row.values()))


def refusal(error):
    return dict(family=type(error).__name__, message=str(error), code=getattr(error, "code", None),
                hint=getattr(error, "hint", None), details=getattr(error, "details", None),
                retryable=getattr(error, "retryable", None), error_class=None, invariant=None,
                cause=refusal(error.__cause__) if error.__cause__ else None)


def capture(conn, name):
    conn.commit()
    before = dump(conn)
    conn.execute("BEGIN IMMEDIATE")
    conn.execute("SAVEPOINT prebuild_oracle")
    error = None
    count = None
    with ExitStack() as stack:
        for target in ("socket.getaddrinfo", "socket.socket.connect", "socket.socket.connect_ex", "socket.socket.sendto"):
            stack.enter_context(patch(target, side_effect=AssertionError("pricing attempted network I/O")))
        try:
            count = auto_price_transactions_from_rates_cache(conn, {"id": "p", "fiat_currency": "EUR"})
        except Exception as exc:
            error = refusal(exc)
            conn.execute("ROLLBACK TO prebuild_oracle")
    conn.execute("RELEASE prebuild_oracle")
    expected = dump(conn)
    conn.rollback()
    return dict(name=name, before=before, expected=expected, count=count, error=error)


def vectors():
    output = []
    cases = {
        "cache-miss": ({}, None),
        "live-enabled-cache-miss": ({}, None),
        "provider-minute": ({}, {}),
        "confirmed-price": ({"confirmed_at": "2025-01-02T00:00:00Z"}, {}),
        "zero-amount": ({"amount": 0}, {}),
        "negative-amount": ({"amount": -1}, {}),
        "excluded": ({"excluded": 1}, {}),
        "other-profile": ({"profile_id": "other", "wallet_id": "other"}, {}),
        "unsupported-asset": ({"asset": "ETH"}, {}),
        "lbtc": ({"asset": " lbtc "}, {}),
        "l-btc-unsupported": ({"asset": "L-BTC"}, {}),
        "unsupported-fiat": ({"fiat_currency": "CAD"}, {}),
        "import-price": ({"fiat_rate": 1, "fiat_price_source": "import"}, {}),
        "manual-price": ({"fiat_value_exact": "123.00", "pricing_source_kind": "manual_override"}, {}),
        "legacy-enrichment": ({"fiat_rate": 1, "fiat_price_source": "rates_cache"}, {}),
        "legacy-exact-rate": ({"fiat_rate_exact": "1.23000000000000000000000000000000000000000001", "fiat_price_source": "rates_cache"}, {}),
        "legacy-exact-value": ({"fiat_value_exact": "3.4500", "fiat_price_source": "rates_cache"}, {}),
        "legacy-provenance-kept": ({"fiat_rate": 1, "fiat_price_source": "rates_cache", "pricing_source_kind": "fmv_provider"}, {}),
        "legacy-quality-kept": ({"fiat_rate": 1, "fiat_price_source": "rates_cache", "pricing_quality": "exact"}, {}),
        "negative-price": ({"fiat_rate": -1, "fiat_value": -3}, {}),
        "zero-price": ({"fiat_rate": 0, "fiat_value": 0}, {}),
        "empty-exact": ({"fiat_rate_exact": "", "fiat_value_exact": ""}, {}),
        "legacy-missing-cache": ({"fiat_rate": 1, "fiat_price_source": "rates_cache"}, None),
        "manual-cache": ({}, {"source": "manual", "granularity": "daily"}),
        "coarse-cache": ({}, {"granularity": " DAILY "}),
        "unknown-cache": ({}, {"granularity": None, "method": None}),
        "real-cache": ({}, {"rate_exact": None}),
        "empty-cache-exact": ({}, {"rate_exact": ""}),
        "invalid-exact": ({"fiat_rate_exact": "bad"}, {}),
        "invalid-cache": ({}, {"rate_exact": "bad"}),
        "invalid-date": ({"occurred_at": "bad"}, {}),
        "invalid-month": ({"occurred_at": "2025-13-01"}, {}),
        "invalid-day": ({"occurred_at": "2025-02-30"}, {}),
        "invalid-hour": ({"occurred_at": "2025-01-01T25:00:00Z"}, {}),
        "invalid-minute": ({"occurred_at": "2025-01-01T00:60:00Z"}, {}),
        "invalid-second": ({"occurred_at": "2025-01-01T00:00:60Z"}, {}),
        "invalid-year": ({"occurred_at": "0000-01-01"}, {}),
        "invalid-time-with-month": ({"occurred_at": "2025-13-01Tbad"}, {}),
        "date-overflow": ({"occurred_at": "0001-01-01T00:00:00+01:00"}, {}),
        "unsupported-invalid-date": ({"asset": "ETH", "occurred_at": "bad"}, {}),
        "offset-time": ({"occurred_at": "2025-01-01T01:00:00+01:00"}, {}),
        "fractional-time": ({"occurred_at": "2025-01-01T00:00:00.500000Z"}, {"timestamp": "2025-01-01T00:00:00Z"}),
        "rate-offset": ({"occurred_at": "2025-01-01T00:00:00Z"}, {"timestamp": "2025-01-01T01:00:00+01:00"}),
        "invalid-stored-rate-date": ({}, {"timestamp": "bad"}),
        "small-quantity": ({"amount": 1}, {}),
        "large-quantity": ({"amount": 9223372036854775807}, {}),
        "signed-zero": ({"fiat_rate_exact": "-0.00"}, {}),
        "unicode-decimal": ({"fiat_rate_exact": "\u0661.\u0662\u0663_0", "fiat_price_source": "rates_cache"}, {}),
        "rate-overflow": ({}, {"rate_exact": "1e999999"}),
        "rate-tiny": ({}, {"rate_exact": "1e-40"}),
        "quiet-nan": ({}, {"rate_exact": "-NaN001"}),
        "signaling-nan": ({}, {"rate_exact": "sNaN12"}),
        "clear-old-reference": ({"pricing_external_ref": "old-evidence"}, {}),
        "rate-context-overflow": ({}, {"rate_exact": "1e1000000"}),
        "rate-extreme-overflow": ({}, {"rate_exact": "1e999999999999999999"}),
        "rate-construction-overflow": ({}, {"rate_exact": "1e1000000000000000000"}),
        "rate-subnormal-underflow": ({}, {"rate_exact": "1e-1000031"}),
        "rate-large-coefficient": ({}, {"rate_exact": '0.' + '1' * 5000}),
        "rate-large-nan-payload": ({}, {"rate_exact": 'sNaN' + '1' * 5000}),
        "legacy-real-amount": ({"amount": 1.75}, {}),
        "legacy-negative-real-amount": ({"amount": -1.75}, {}),
        "legacy-wide-real-amount": ({"amount": 1e20}, {}),
        "legacy-text-amount": ({"amount": '1_000'}, {}),
        "legacy-text-amount-cache-miss": ({"amount": '1_000'}, None),
        "legacy-text-amount-unsupported": ({"amount": '1_000', "asset": 'ETH'}, {}),
        "legacy-amount-skipped-for-existing-value": ({"amount": '1_000', "fiat_value_exact": '1', "fiat_price_source": 'rates_cache'}, {}),
        "value-derived-rate": ({"fiat_value_exact": '1', "fiat_price_source": 'rates_cache'}, {"rate_exact": '', "rate": ''}),
        "legacy-text-amount-value-derived-rate": ({"amount": '1_000', "fiat_value_exact": '1', "fiat_price_source": 'rates_cache'}, {"rate_exact": '', "rate": ''}),
        "empty-rate-and-value": ({"amount": '1_000'}, {"rate_exact": '', "rate": ''}),
        "binary-exact-cache": ({}, {"rate_exact": b'123'}),
        "binary-empty-cache": ({}, {"rate_exact": b''}),
        "binary-single-quote-cache": ({}, {"rate_exact": b"it's"}),
        "binary-all-bytes-cache": ({}, {"rate_exact": bytes(range(256))}),
        "binary-numeric-cache": ({}, {"rate_exact": None, "rate": b'123'}),
        "binary-exact-rate": ({"fiat_rate_exact": b'1', "fiat_price_source": 'rates_cache'}, {}),
        "binary-exact-value": ({"fiat_value_exact": b'1', "fiat_price_source": 'rates_cache'}, {}),
        "binary-numeric-rate": ({"fiat_rate": b'1', "fiat_price_source": 'rates_cache'}, {"rate_exact": None}),
        "binary-numeric-value": ({"fiat_value": b'1', "fiat_price_source": 'rates_cache', "amount": 0}, {}),
        "binary-quantity": ({"amount": b'1'}, {}),
        "binary-cache-ignored": ({"fiat_rate_exact": '1', "fiat_price_source": 'rates_cache'}, {"rate_exact": b'bad'}),
        "positive-infinity": ({}, {"rate_exact": 'Infinity'}),
        "negative-infinity": ({}, {"rate_exact": '-Infinity'}),
        "signaling-nan-without-quantity": ({"amount": 0}, {"rate_exact": 'sNaN12'}),
        "positive-infinite-real-cache": ({}, {"rate_exact": None, "rate": float('inf')}),
        "negative-infinite-real-cache": ({}, {"rate_exact": None, "rate": float('-inf')}),
        "infinite-real-quantity": ({"amount": float('inf')}, {}),
        "value-derived-quiet-nan": ({"fiat_value_exact": 'NaN12', "fiat_price_source": 'rates_cache'}, {"rate_exact": '', "rate": ''}),
        "value-derived-signaling-nan": ({"fiat_value_exact": 'sNaN12', "fiat_price_source": 'rates_cache'}, {"rate_exact": '', "rate": ''}),
        "value-derived-positive-infinity": ({"fiat_value_exact": 'Infinity', "fiat_price_source": 'rates_cache'}, {"rate_exact": '', "rate": ''}),
        "value-derived-negative-infinity": ({"fiat_value_exact": '-Infinity', "fiat_price_source": 'rates_cache'}, {"rate_exact": '', "rate": ''}),
        "invalid-date-separator": ({"occurred_at": '2025-01x01'}, {}),
        "invalid-century-leap-day": ({"occurred_at": '1900-02-29'}, {}),
        "invalid-four-century-leap-day": ({"occurred_at": '2000-02-30'}, {}),
        "invalid-four-century-leap-hour": ({"occurred_at": '2000-02-29T25:00:00Z'}, {}),
        "invalid-month-short-time": ({"occurred_at": '2025-13-01T12'}, {}),
        "invalid-basic-hour": ({"occurred_at": '20000101T24'}, {}),
        "value-derived-overflow": ({"amount": 1, "fiat_value_exact": '1e999999', "fiat_price_source": 'rates_cache'}, {"rate_exact": '', "rate": ''}),
        # Round 5 price bounds (d444f4db5, f823b4729): an unusable cached
        # valuation marks the row unsupported instead of storing a price.
        "rate-tax-limit": ({}, {"rate_exact": "1e19"}),
        "rate-mirror-limit": ({}, {"rate_exact": "9999999999999999999.5"}),
        "rate-integer-mirror-limit": ({"amount": 100000000000}, {"rate_exact": "9999999999999999999"}),
        "derived-value-limit": ({}, {"rate_exact": "1e18"}),
        "derived-value-tiny": ({"amount": 1}, {"rate_exact": "1e-12"}),
        "derived-rate-limit": ({"amount": 1, "fiat_value_exact": "1e18", "fiat_price_source": 'rates_cache'}, {"rate_exact": '', "rate": ''}),
        "value-limit": ({"fiat_value_exact": "1e19", "fiat_price_source": 'rates_cache'}, {}),
        "stored-rate-limit": ({"fiat_rate_exact": "1e19", "fiat_price_source": 'rates_cache'}, {}),
        "unsupported-then-priced": ({}, {"rate_exact": "1e18"}),
        # A stored value and a cached rate are each in range, but one implies
        # an out-of-range partner for the quantity.
        "implied-rate-limit": ({"amount": 1, "fiat_value_exact": "1e18", "fiat_price_source": 'rates_cache'}, {}),
        "implied-value-limit": ({"amount": 1000000000000000, "fiat_value_exact": "1", "fiat_price_source": 'rates_cache'}, {"rate_exact": "1e18"}),
        # The rate is bounded before the stored value is read.
        "bounded-rate-before-invalid-value": ({"fiat_value_exact": "bad", "fiat_price_source": 'rates_cache'}, {"rate_exact": "NaN"}),
        "stored-real-rate-yields-to-cache": ({"fiat_price_source": 'rates_cache', "fiat_rate": 1e19, "pricing_provider": "kept", "pricing_method": "old"}, {}),
        # Group B's edge cases: the rate and value bounds apart from the
        # valuations they imply, canonical zero, and absolute amounts.
        "rate-limit-small-amount": ({"amount": 1}, {"rate_exact": '1e19'}),
        "rate-float-mirror-small-amount": ({"amount": 1}, {"rate_exact": '9999999999999999999.5'}),
        "derived-value-floor": ({"amount": 1}, {"rate_exact": '1e-13'}),
        "value-limit-zero-amount": ({"amount": 0, "fiat_value_exact": '1e19', "fiat_price_source": 'rates_cache'}, {"rate_exact": '', "rate": ''}),
        "derived-rate-floor": ({"amount": 9223372036854775807, "fiat_value_exact": '1e-12', "fiat_price_source": 'rates_cache'}, {"rate_exact": '', "rate": ''}),
        "zero-rate-canonical": ({}, {"rate_exact": '0.000'}),
        "negative-derived-amount": ({"amount": -9223372036854775807, "fiat_value_exact": '1e-12', "fiat_price_source": 'rates_cache'}, {}),
    }
    for name, (tx, cached) in cases.items():
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        seed(conn)
        transaction(conn, **tx)
        if cached is not None:
            rate(conn, **cached)
        if name == "live-enabled-cache-miss":
            conn.execute("INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p',?)",
                         ('{"source_classes":{"market_rates":true}}',))
            conn.execute("INSERT INTO settings(key,value) VALUES('market_rate_provider','coinbase-exchange')")
        if name == "confirmed-price":
            rate(conn, timestamp="2025-01-02T00:00:00Z", rate_exact="90000")
        if name == "manual-cache":
            rate(conn, source="a-provider", rate_exact="1", fetched_at="9999")
        if name == "invalid-exact":
            transaction(conn, id="earlier", occurred_at="2025-01-01T01:00:00Z")
        if name == "unsupported-then-priced":
            transaction(conn, id="later", occurred_at="2025-01-01T13:00:00Z", amount=100000)
        if name == "binary-exact-cache":
            transaction(conn, id="earlier", occurred_at="2025-01-01T01:00:00Z",
                        fiat_rate_exact='1', fiat_price_source='rates_cache')
        output.append(capture(conn, name))
        conn.close()
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    data = vectors()
    base = data[0]["before"]
    packed = dict(base=base, cases=[dict(case, **{
        side: {table: item["rows"] for table, item in case[side].items() if item != base[table]}
        for side in ("before", "expected")}) for case in data])
    text = json.dumps(packed, ensure_ascii=True, separators=(",", ":"), allow_nan=False) + "\n"
    if args.write:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
    print(f"{len(data)} complete pricing dumps match Python")


if __name__ == "__main__":
    main()
