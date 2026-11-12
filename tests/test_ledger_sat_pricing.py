"""Importer regression recorded by the Rust port."""

from decimal import Decimal

from kassiber.importers import load_generic_ledger_records


def test_sats_column_uses_bitcoin_quantity_for_unit_price(tmp_path):
    path = tmp_path / "ledger.csv"
    path.write_text("Date,Sats Amount,Price\n2025-01-02,100000,0.2\n")
    row, = load_generic_ledger_records(path)
    assert row["amount"] == Decimal("0.001")
    assert row["fiat_value"] == Decimal("0.0002")
    assert row["fiat_rate"] == Decimal("0.2")
