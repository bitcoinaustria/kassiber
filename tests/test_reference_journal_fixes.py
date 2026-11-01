"""Regressions discovered while comparing the Python and Rust journals."""

from decimal import Decimal

from kassiber.cli.handlers import auto_price_transactions_from_rates_cache
from kassiber.core import rates
from kassiber.db import open_db
from tests.test_source_overlap import _seed_book, _tx, _wallet


def test_auto_pricing_accepts_every_documented_bitcoin_rail_alias(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        profile = _seed_book(conn)
        _wallet(conn, "wallet", "Wallet", "csv", {})
        rates.set_manual_rate(conn, "BTC-EUR", "2026-01-01T00:00:00Z", "50000")
        for asset in ("BTC", "LBTC", "L-BTC"):
            _tx(conn, asset, "wallet", asset)
            conn.execute(
                "UPDATE transactions SET asset=?, fiat_rate=NULL, fiat_value=NULL WHERE id=?",
                (asset, asset),
            )
        assert auto_price_transactions_from_rates_cache(conn, profile) == 3
        for row in conn.execute("SELECT * FROM transactions"):
            assert row["fiat_rate_exact"] == "50000"
            assert Decimal(row["fiat_value_exact"]) == 500
            assert row["pricing_pair"] == "BTC-EUR"
    finally:
        conn.close()
