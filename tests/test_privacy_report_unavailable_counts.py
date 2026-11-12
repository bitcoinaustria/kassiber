"""An unavailable local count is unknown, even if other counts are exact."""

import pytest

from kassiber.cli.handlers import _report_hooks
from kassiber.core.reports import report_privacy_hygiene
from kassiber.db import open_db
from tests.test_custody_authored_migration import _scope


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path / "data")
    _scope(conn)
    conn.commit()
    try:
        yield conn
    finally:
        conn.close()


@pytest.mark.parametrize("table,column,group,field,code", [
    ("wallet_utxos", "address", "inventory", "utxos_with_address_count", "utxo_address_count_unavailable"),
    ("transactions", "raw_json", "transactions", "transaction_raw_json_count", "transaction_raw_json_count_unavailable"),
    ("journal_quarantines", "reason", "journals", "privacy_quarantine_count", "privacy_quarantine_count_unavailable"),
])
def test_unavailable_count_cannot_claim_exact_zero(book, table, column, group, field, code):
    book.execute(f"ALTER TABLE {table} RENAME COLUMN {column} TO unavailable_{column}")
    before = list(book.iterdump())
    result = report_privacy_hygiene(book, "ws", "profile", _report_hooks())
    assert result["facts"][group][field] is None
    assert result["facts"][group]["evidence_level"] == "unknown"
    assert any(row["code"] == code and row["evidence_level"] == "unknown" for row in result["limitations"])
    assert list(book.iterdump()) == before


def test_unavailable_active_count_cannot_claim_no_transactions(book):
    book.execute("ALTER TABLE transactions RENAME COLUMN excluded TO unavailable_excluded")
    result = report_privacy_hygiene(book, "ws", "profile", _report_hooks())
    assert result["summary"]["active_transaction_count"] is None
    assert result["facts"]["transactions"]["evidence_level"] == "unknown"
    assert result["facts"]["journals"]["status"] == "unknown"
    assert result["facts"]["journals"]["needs_processing"] is None
    assert result["facts"]["journals"]["evidence_level"] == "unknown"


def test_known_empty_counts_remain_exact_zero(book):
    result = report_privacy_hygiene(book, "ws", "profile", _report_hooks())
    assert result["facts"]["inventory"]["utxos_with_address_count"] == 0
    assert result["facts"]["inventory"]["evidence_level"] == "exact"
    assert result["facts"]["transactions"]["transaction_raw_json_count"] == 0
    assert result["facts"]["transactions"]["evidence_level"] == "exact"
    assert result["facts"]["journals"]["status"] == "no_transactions"
    assert result["facts"]["journals"]["evidence_level"] == "exact"
