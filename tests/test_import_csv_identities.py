"""Overlapping exports dedupe economics independently of their file row."""

import csv
from decimal import Decimal

import pytest

from kassiber import importers


CASES = [
    ("binance_supplemental_csv", {
        "timestamp UTC": "2025-01-01 10:00:00", "base asset symbol": "BTC",
        "quote asset amount + symbol": "100.00 EUR", "trading fee (in quote asset)": "1.00 EUR",
        "base asset amount + symbol": "0.002 BTC", "source of funds": "Spot Wallet",
    }, "timestamp UTC"),
    ("binance_supplemental_csv", {"id": "", "amount": "0.002", "asset": "BTC",
                                    "divTime": "", "timestamp": "2025-01-01 10:00:00"}, "timestamp"),
    ("coinfinity_csv", {"Order ID": "", "Type": "sell", "Date": "2025-01-01T10:00:00Z",
                        "Amount EUR": "100", "Amount Crypto": "0.002", "Crypto": "BTC",
                        "Rate EUR": "50000", "Mining Fee Crypto": "0", "Total Fee EUR": "1",
                        "Transaction": ""}, "Date"),
    ("pocketbitcoin_csv", {"type": "exchange", "date": "2025-01-01T10:00:00Z", "reference": "",
                           "price.currency": "EUR", "price.amount": "50000", "cost.currency": "EUR",
                           "cost.amount": "100", "fee.currency": "EUR", "fee.amount": "1",
                           "value.currency": "BTC", "value.amount": "0.002"}, "date"),
    ("ledgerlive_csv", {"Operation Date": "2025-01-01T10:00:00Z", "Currency Ticker": "BTC",
                        "Operation Type": "IN", "Operation Amount": "0.002", "Operation Fees": "0",
                        "Operation Hash": "", "Account Name": "Wallet"}, "Operation Date"),
]


def _write(path, rows):
    with path.open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


@pytest.mark.parametrize("input_format,january,date_column", CASES)
def test_csv_synthetic_ids_survive_new_rows_and_count_identical_economic_repeats(tmp_path, input_format, january, date_column):
    february = {**january, date_column: january[date_column].replace("01-01", "02-01")}
    jan, janfeb, repeats = (tmp_path / name for name in ("jan.csv", "janfeb.csv", "repeats.csv"))
    _write(jan, [january])
    _write(janfeb, [february, january])
    _write(repeats, [january, february, january])
    original, = importers.load_import_records(jan, input_format)
    expanded = importers.load_import_records(janfeb, input_format)
    duplicated = importers.load_import_records(repeats, input_format)
    assert expanded[1]["txid"] == original["txid"]
    assert duplicated[0]["txid"] == original["txid"]
    assert duplicated[1]["txid"] == expanded[0]["txid"]
    assert len({row["txid"] for row in duplicated}) == 3


def test_binance_overlap_does_not_duplicate_holdings_or_acquisition_value(tmp_path):
    from kassiber.cli import handlers
    from kassiber.core.imports import insert_wallet_records
    from kassiber.db import open_db
    from tests.test_custody_authored_migration import _scope

    fmt, january, column = CASES[0]
    conn = open_db(tmp_path / "book")
    try:
        _scope(conn)
        conn.commit()
        profile = conn.execute("SELECT * FROM profiles").fetchone()
        wallet = conn.execute("SELECT * FROM wallets WHERE id='btc'").fetchone()
        for name, rows, inserted in (
            ("jan", [january], 1),
            ("janfeb", [{**january, column: "2025-02-01 10:00:00"}, january], 1),
            ("repeats", [january, january], 1),
        ):
            path = tmp_path / f"{name}.csv"
            _write(path, rows)
            result = insert_wallet_records(conn, profile, wallet, importers.load_import_records(path, fmt),
                                           "Binance", handlers._import_coordinator_hooks())
            assert result["imported"] == inserted
        stored = conn.execute("SELECT amount, fiat_value_exact FROM transactions").fetchall()
        assert len(stored) == 3
        assert sum(row["amount"] for row in stored) == 600_000_000
        assert sum(Decimal(row["fiat_value_exact"]) for row in stored) == 303
    finally:
        conn.close()


def test_binance_same_time_and_bitcoin_amount_with_different_cash_is_distinct(tmp_path):
    fmt, row, _ = CASES[0]
    path = tmp_path / "different-prices.csv"
    _write(path, [row, {**row, "quote asset amount + symbol": "200 EUR"}])
    first, second = importers.load_import_records(path, fmt)
    assert first["txid"].rsplit(":", 1)[0] != second["txid"].rsplit(":", 1)[0]
    assert (first["fiat_value"], second["fiat_value"]) == (101, 201)


@pytest.mark.parametrize("case_index,id_column,expected", [
    (1, "id", "binance:real:economic:provider"),
    (2, "Order ID", "coinfinity:real:economic:provider"),
    (3, "reference", "pocketbitcoin:real:economic:provider"),
    (4, "Operation Hash", "real:economic:provider"),
])
def test_explicit_provider_ids_are_never_occurrence_numbered(tmp_path, case_index, id_column, expected):
    fmt, row, _ = CASES[case_index]
    row = {**row, id_column: "real:economic:provider"}
    path = tmp_path / "provider.csv"
    _write(path, [row, row])
    first, second = importers.load_import_records(path, fmt)
    assert first["txid"] == second["txid"] == expected
