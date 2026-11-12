"""Excluded transaction IDs must still match the asset being compared."""

from kassiber.cli.handlers import _report_hooks
from kassiber.core.cross_check import cross_check
from kassiber.db import open_db
from tests.test_custody_authored_migration import _scope, _tx


def test_excluded_shared_id_finds_the_correct_asset_outside_the_time_window(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        for ident, asset in (("first", "LBTC"), ("second", "BTC")):
            _tx(conn, ident, "btc", "inbound", asset, 100_000_000_000, "2025-01-01T00:00:00Z")
        conn.execute("UPDATE transactions SET excluded=1, external_id='shared'")
        path = tmp_path / "tool.csv"
        path.write_text("Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Date,Tx-ID\nDeposit,1,BTC,,,,,Test,2025-02-01,shared\n")
        result = cross_check(conn, "ws", "profile", _report_hooks(), tool="cointracking", file_path=path)
        row, = [row for row in result["rows"] if row["tool"]]
        assert row["status"] == "excluded_in_kassiber"
        assert row["kassiber"]["ref"] == "second"
    finally:
        conn.close()
