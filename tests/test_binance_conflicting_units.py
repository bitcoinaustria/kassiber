"""Contradictory Bitcoin asset declarations cannot price a guessed quantity."""

import pytest

from kassiber.errors import AppError
from kassiber.importers import normalize_binance_supplemental_autoinvest_record


@pytest.mark.parametrize("declared,amount", [("BTC", "0.002 ETH"), ("ETH", "0.002 BTC"), ("BTC", "0.002 LBTC")])
def test_binance_refuses_conflicting_base_asset_symbols(declared, amount):
    with pytest.raises(AppError, match="conflict"):
        normalize_binance_supplemental_autoinvest_record({"base asset symbol": declared, "base asset amount + symbol": amount, "quote asset amount + symbol": "100 EUR", "trading fee (in quote asset)": "0 EUR", "timestamp UTC": "2025-01-01"})
