"""Importer regression recorded by the Rust port."""

import pytest

from kassiber.importers import load_generic_ledger_records, read_tax_platform_rows


@pytest.mark.parametrize("reader", ["ledger", "cointracking"])
def test_csv_readers_accept_bare_carriage_return_records(tmp_path, reader):
    path = tmp_path / "ledger.csv"
    if reader == "ledger":
        content = "Type,Date,Received Amount,Received Asset\rDeposit,2025-01-02,1,BTC\r"
    else:
        content = "Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Date,Tx-ID\rDeposit,1,BTC,,,,,Test,2025-01-02,tx\r"
    path.write_bytes(content.encode())
    rows = load_generic_ledger_records(path) if reader == "ledger" else read_tax_platform_rows(reader, path)[1]
    assert len(rows) == 1
