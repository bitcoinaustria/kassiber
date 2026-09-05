"""PR #428 salvage: validate Bitcoin-ledger cash legs, without asset overlays."""
from decimal import Decimal

import pytest

from kassiber.errors import AppError
from kassiber.importers import normalize_generic_ledger_record


def record(direction, currency, cash_amount="20"):
    bitcoin_side, cash_side = ("Received", "Sent") if direction == "inbound" else ("Sent", "Received")
    return {
        "Date": "2025-01-01", "Type": "Buy" if direction == "inbound" else "Sell",
        f"{bitcoin_side} Asset": "BTC", f"{bitcoin_side} Amount": "1",
        f"{cash_side} Asset": currency, f"{cash_side} Amount": cash_amount,
    }


@pytest.mark.parametrize("direction", ["inbound", "outbound"])
@pytest.mark.parametrize("currency", ["ETH", "USDT", "USDC", "EURO", "XYZ"])
@pytest.mark.parametrize("cash_amount", ["20", ""])
def test_unsupported_cash_symbols_never_become_exact_fiat(direction, currency, cash_amount):
    row = record(direction, currency, cash_amount)
    row["Fiat Value"] = "40000"  # A value cannot legitimize a non-fiat cash leg.
    with pytest.raises(AppError) as raised:
        normalize_generic_ledger_record(row, index=7)
    assert raised.value.code == "validation"
    assert f"Ledger row 7: unrecognized cash currency '{currency}'" in str(raised.value)
    assert "book currency" in raised.value.hint
    assert "legacy-holdings" not in raised.value.hint


@pytest.mark.parametrize("direction", ["inbound", "outbound"])
@pytest.mark.parametrize("currency", ["eur", "USD", " CHF ", "JPY"])
def test_supported_cash_and_matching_fiat_fees_keep_exact_execution(direction, currency):
    row = record(direction, currency, "40000")
    row.update({"Fee Amount": "20", "Fee Asset": currency})
    result = normalize_generic_ledger_record(row)
    assert result["fiat_currency"] == currency.strip().upper()
    assert result["fiat_value"] == Decimal("40020" if direction == "inbound" else "39980")
    assert result["pricing_source_kind"] == "exchange_execution"
    assert result["pricing_quality"] == "exact"


@pytest.mark.parametrize("direction", ["inbound", "outbound"])
def test_explicit_book_currency_value_without_cash_leg_remains_supported(direction):
    row = record(direction, "", "")
    row["Fiat Value"] = "40000"
    result = normalize_generic_ledger_record(row)
    assert result["fiat_currency"] is None
    assert result["fiat_value"] == Decimal("40000")
    assert result["pricing_source_kind"] is None


@pytest.mark.parametrize("direction", ["inbound", "outbound"])
@pytest.mark.parametrize("asset,amount,fee", [("BTC", "1", "0.000001"), ("SATS", "100000000", "100")])
def test_bitcoin_and_default_satoshi_fee_units_do_not_change(direction, asset, amount, fee):
    row = record(direction, "EUR", "40000")
    bitcoin_side = "Received" if direction == "inbound" else "Sent"
    row.update({f"{bitcoin_side} Asset": asset, f"{bitcoin_side} Amount": amount,
                "Fee Amount": fee, "Fee Asset": ""})
    result = normalize_generic_ledger_record(row)
    assert result["asset"] == "BTC"
    assert result["amount"] == Decimal("1")
    assert result["fee"] == Decimal("0.000001")
    assert result["fiat_value"] == Decimal("40000")
