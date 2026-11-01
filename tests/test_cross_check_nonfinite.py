"""Comparison reads refuse nonfinite amounts before arithmetic or publication."""

import pytest

from kassiber.core.cross_check import _tool_rows
from kassiber.core.transaction_formats import _journal_values
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_custody_authored_migration import NOW, _scope, _tx


@pytest.mark.parametrize("value", ["NaN", "-NaN12", "sNaN", "Infinity", "-Infinity"])
@pytest.mark.parametrize("field", ["amount", "value"])
def test_nonfinite_tool_amount_or_cash_is_a_reviewable_refusal(tmp_path, value, field):
    path = tmp_path / "tool.csv"
    path.write_text("Type,Buy Amount,Buy Currency,Sell Amount,Sell Currency,Fee,Fee Currency,Exchange,Date,Buy Value\n" +
                    f"Deposit,{value if field == 'amount' else '1'},BTC,,,,,Tool,2025-01-01,{value if field == 'value' else '1'}\n"
                    "Deposit,2,BTC,,,,,Tool,2025-01-02,200\n")
    row, healthy = _tool_rows("cointracking", path, "EUR")[0]
    assert row.error
    assert not row.comparable
    assert row.index == 1
    assert healthy.comparable and healthy.index == 2
    assert healthy.value == 200


def test_nonfinite_stored_journal_cash_cannot_reach_comparison_totals(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        _tx(conn, "tx", "btc", "inbound", "BTC", 1000, NOW)
        conn.execute("INSERT INTO journal_entries(id,workspace_id,profile_id,wallet_id,transaction_id,entry_type,occurred_at,asset,quantity,proceeds_exact,created_at) VALUES('j','ws','profile','btc','tx','disposal',?,'BTC',1000,'NaN12',?)", (NOW, NOW))
        with pytest.raises(AppError, match="finite"):
            _journal_values(conn, "profile")
    finally:
        conn.close()
