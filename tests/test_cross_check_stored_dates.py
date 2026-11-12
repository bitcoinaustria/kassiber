"""Comparison reports must not invent an instant for malformed book dates."""

import pytest

from kassiber.cli.handlers import _report_hooks
from kassiber.core.cross_check import cross_check
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_custody_authored_migration import NOW, _scope, _tx


@pytest.mark.parametrize("date", ["", "2025-01-bad"])
@pytest.mark.parametrize("year", [None, 2025])
def test_invalid_stored_date_refuses_instead_of_becoming_year_one(tmp_path, date, year):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        _tx(conn, "target", "btc", "inbound", "BTC", 1000, NOW)
        conn.execute("UPDATE transactions SET occurred_at=?", (date,))
        conn.commit()
        before = list(conn.iterdump())
        path = tmp_path / "tool.csv"
        path.write_text("Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Date\n")
        with pytest.raises(AppError, match="Stored comparison dates") as raised:
            cross_check(conn, "ws", "profile", _report_hooks(), tool="cointracking", file_path=path, year=year)
        assert raised.value.code == "validation"
        assert list(conn.iterdump()) == before
    finally:
        conn.close()
