"""Stored provenance quantities cannot be silently rounded for display."""

from types import SimpleNamespace

import pytest

from kassiber.cli.handlers import _report_hooks
from kassiber.core import source_funds
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_custody_authored_migration import NOW, _scope, _tx


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path / "data")
    _scope(conn)
    _tx(conn, "target", "btc", "inbound", "BTC", 1000, NOW)
    conn.execute("""INSERT INTO source_funds_sources(
        id,workspace_id,profile_id,source_type,label,asset,amount,created_at,updated_at)
        VALUES('source','ws','profile','fiat_purchase','Source','BTC',1000,?,?)""", (NOW, NOW))
    conn.execute("""INSERT INTO source_funds_links(
        id,workspace_id,profile_id,from_source_id,to_transaction_id,link_type,
        asset,allocation_amount,from_allocation_amount,created_at,updated_at)
        VALUES('link','ws','profile','source','target','manual_source','BTC',1000,1000,?,?)""", (NOW, NOW))
    source_funds.save_case_snapshot(conn, "ws", "profile", "target", 1000, "BTC", "minimal", "blocked", {})
    conn.commit()
    try:
        yield conn
    finally:
        conn.close()


READS = [
    ("source_funds_sources", "amount", source_funds.list_sources, "amount_msat"),
    ("source_funds_links", "allocation_amount", source_funds.list_links, "allocation_amount_msat"),
    ("source_funds_links", "from_allocation_amount", source_funds.list_links, "from_allocation_amount_msat"),
    ("source_funds_cases", "target_amount", source_funds.list_cases, "target_amount_msat"),
]


@pytest.mark.parametrize("table,column,read,field", READS)
@pytest.mark.parametrize("value", [1.5, -0.5])
def test_fractional_stored_provenance_amount_refuses_without_mutation(book, table, column, read, field, value):
    book.execute(f"UPDATE {table} SET {column}=?", (value,))
    book.commit()
    before = list(book.iterdump())
    hooks = SimpleNamespace(resolve_scope=_report_hooks().resolve_scope)
    with pytest.raises(AppError, match="whole millisatoshis") as raised:
        read(book, "ws", "profile", hooks)
    assert raised.value.code == "validation"
    assert list(book.iterdump()) == before


@pytest.mark.parametrize("table,column,read,field", READS)
@pytest.mark.parametrize("value", [0, 1, 2**53 + 1])
def test_whole_stored_provenance_amount_preserves_exact_msat(book, table, column, read, field, value):
    book.execute(f"UPDATE {table} SET {column}=?", (value,))
    hooks = SimpleNamespace(resolve_scope=_report_hooks().resolve_scope)
    row, = read(book, "ws", "profile", hooks)
    assert row[field] == value
