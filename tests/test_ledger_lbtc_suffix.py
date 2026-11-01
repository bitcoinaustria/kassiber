"""Importer regression recorded by the Rust port."""

from kassiber.importers import load_generic_ledger_records


def test_native_ledger_accepts_lbtc_amount_suffix(tmp_path):
    path = tmp_path / "ledger.csv"
    path.write_text("Type,Date,Received Amount,Received Asset\nDeposit,2025-01-02,1 LBTC,LBTC\n")
    row, = load_generic_ledger_records(path)
    assert row["amount"] == 1
    assert row["asset"] == "LBTC"
