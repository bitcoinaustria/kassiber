"""Importer regression recorded by the Rust port."""

from kassiber.importers import load_generic_ledger_records
from decimal import Decimal
import pytest
from kassiber.errors import AppError


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


@pytest.mark.parametrize("amount,asset", [
    ("1000 sats", "BTC"), ("1000 SATS", "BTC"), ("1000 sat", "BTC"),
    ("0.5 L-BTC", "BTC"), ("0.5 BTC", "SATS"), ("0.5 BTC", "LBTC"),
    ("100 EUR", "BTC"),
])
@pytest.mark.parametrize("leg", ["Received", "Sent", "Fee"])
def test_mismatched_amount_suffix_cannot_change_the_declared_unit(tmp_path, amount, asset, leg):
    path = tmp_path / "ledger.csv"
    if leg == "Fee":
        header = "Type,Date,Received Amount,Received Asset,Fee Amount,Fee Asset"
        row = f"Deposit,2025-01-02,1,BTC,{amount},{asset}"
    else:
        header = f"Type,Date,{leg} Amount,{leg} Asset"
        row = f"{'Deposit' if leg == 'Received' else 'Withdrawal'},2025-01-02,{amount},{asset}"
    path.write_text(header + "\n" + row + "\n")
    with pytest.raises(AppError) as raised:
        load_generic_ledger_records(path)
    assert raised.value.code == "validation"
    assert "row" in str(raised.value).lower()


def test_satoshi_aliases_preserve_exact_integer_msat_and_fee_fallback(tmp_path):
    from kassiber.core.imports import normalize_import_record
    from kassiber.msat import btc_to_msat
    path = tmp_path / "ledger.csv"
    path.write_text("Type,Date,Sent Amount,Sent Asset,Fee Amount\nWithdrawal,2025-01-02,1000 SATS,SAT,1 sat\n")
    row, = load_generic_ledger_records(path)
    normalized = normalize_import_record(row, source_label="ledger")
    assert btc_to_msat(normalized["amount"]) == 1_000_000
    assert btc_to_msat(normalized["fee"]) == 1_000
