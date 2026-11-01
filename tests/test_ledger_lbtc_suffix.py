"""Importer regression recorded by the Rust port."""

from kassiber.importers import load_generic_ledger_records
from decimal import Decimal
import pytest


def test_native_ledger_accepts_lbtc_amount_suffix(tmp_path):
    path = tmp_path / "ledger.csv"
    path.write_text("Type,Date,Received Amount,Received Asset\nDeposit,2025-01-02,1 LBTC,LBTC\n")
    row, = load_generic_ledger_records(path)
    assert row["amount"] == 1
    assert row["asset"] == "LBTC"


@pytest.mark.parametrize("suffix,asset,expected", [
    ("L-BTC", "L-BTC", Decimal("0.5")),
    ("l-btc", "LBTC", Decimal("0.5")),
    ("sats", "sats", Decimal("0.000000005")),
])
def test_amount_suffixes_match_asset_aliases_case_insensitively(tmp_path, suffix, asset, expected):
    path = tmp_path / "ledger.csv"
    path.write_text(f"Type,Date,Received Amount,Received Asset\nDeposit,2025-01-02,0.5 {suffix},{asset}\n")
    row, = load_generic_ledger_records(path)
    assert row["amount"] == expected
    assert row["asset"] == ("BTC" if asset == "sats" else "LBTC")
