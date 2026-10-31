"""Historical exit-tax output uses dated evidence, never current balances."""

import warnings

import openpyxl
import pytest

from kassiber.cli.handlers import _report_hooks
from kassiber.core import exit_tax, reports
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.test_exit_tax import _conn_with_rate, _profile, _state


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path / "book")
    conn.executescript("""
        INSERT INTO workspaces(id,label,created_at) VALUES('ws','Books','now');
        INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,
            gains_algorithm,last_processed_at,last_processed_tx_count,created_at)
            VALUES('p','ws','Departing','EUR','at','HIFO','2026-01-01',2,'now');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
            VALUES('cold','ws','p','Cold','manual','{}','now'),
                  ('hot','ws','p','Hot','manual','{}','now');
        INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,
            fingerprint,occurred_at,direction,asset,amount,fiat_rate,fiat_rate_exact,
            raw_json,created_at)
            VALUES('buy','ws','p','cold','buy','buy','2023-12-31T23:30:00Z',
                'inbound','BTC',100000000000,30000,'30000.125','{}','now'),
                ('future','ws','p','cold','future','future','2025-01-01T00:00:00Z',
                'inbound','BTC',20000000000,60000,'60000.123','{}','now');
        INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,
            wallet_id,occurred_at,entry_type,asset,quantity,fiat_value,created_at)
            VALUES('buy','ws','p','buy','cold','2023-12-31T23:30:00Z',
                'acquisition','BTC',100000000000,30000,'now'),
                ('future','ws','p','future','cold','2025-01-01T00:00:00Z',
                'acquisition','BTC',20000000000,12000,'now');
        INSERT INTO journal_wallet_holdings(id,workspace_id,profile_id,wallet_id,
            wallet_label,asset,quantity,cost_basis,created_at)
            VALUES('wh','ws','p','cold','Cold','BTC',500000000000,42000,'now');
    """)
    try:
        yield conn
    finally:
        conn.close()


def _report(conn):
    return reports.report_exit_tax(
        conn, "ws", "p", _report_hooks(), departure_date="2024-01-01"
    )


@pytest.mark.parametrize("country", ["at", "generic"])
def test_report_names_the_books_selected_method(country):
    with _conn_with_rate(60000) as conn:
        profile = {**_profile(country), "gains_algorithm": " HIFO "}
        report = exit_tax.compute_deemed_disposal(conn, profile, _state())
    assert report["method"] == "hifo"
    assert "Accounting method: hifo" in exit_tax.format_exit_tax_lines(report)


@pytest.mark.parametrize(("occurred_at", "expected_sats"), [
    ("2024-01-02T00:30:00+02:00", 100000000),  # Jan 1 in UTC
    ("2024-01-01T23:30:00-02:00", 0),          # Jan 2 in UTC
    ("2024-01-01T23:59:59Z", 100000000),
    ("2024-01-01T23:59:59.000001Z", 100000000),
    ("2024-01-02T00:00:00Z", 0),
])
def test_departure_cutoff_compares_exact_instants(book, occurred_at, expected_sats):
    book.execute("UPDATE journal_entries SET occurred_at=? WHERE id='buy'", (occurred_at,))
    assert _report(book)["totals"]["neuQuantitySats"] == expected_sats


def test_transaction_fallback_is_bounded_and_discloses_its_source_time(book):
    # Lexically later offset timestamp is actually earlier than the UTC quote.
    book.execute("UPDATE transactions SET occurred_at='2024-01-01T00:30:00+02:00' WHERE id='future'")
    report = _report(book)
    assert report["fmvSource"] == [{
        "asset": "BTC", "rate": 30000.125, "pair": "BTC-EUR",
        "asOf": "2023-12-31T23:30:00Z", "source": "transaction",
    }]
    assert report["totals"]["neuMarketValue"] == 30000.13


def test_future_only_transaction_price_leaves_historical_estimate_unpriced(book):
    book.execute("UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL WHERE id='buy'")
    report = _report(book)
    assert report["fmvSource"][0]["source"] == "missing"
    assert report["totals"]["neuMarketValue"] is None
    assert report["totals"]["estimatedTax"] is None


def test_excluded_transaction_cannot_supply_a_historical_price(book):
    book.execute("UPDATE transactions SET excluded=1 WHERE id='buy'")
    book.execute("UPDATE profiles SET last_processed_tx_count=1")
    assert _report(book)["fmvSource"][0]["source"] == "missing"


@pytest.mark.parametrize("timestamp", [None, "2025-01-01T00:00:00Z"])
def test_undated_or_future_state_fallback_cannot_price_a_past_departure(timestamp):
    state = _state()
    state["latest_rate_timestamps"] = {"BTC": timestamp}
    with _conn_with_rate() as conn:
        report = exit_tax.compute_deemed_disposal(
            conn, _profile(), state, departure_date="2024-01-01"
        )
    assert report["totals"]["estimatedTax"] is None


def test_wallet_context_replays_cutoff_transfers_and_ignores_current_holdings(book):
    book.executescript("""
        INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,
            wallet_id,occurred_at,entry_type,asset,quantity,created_at)
            VALUES('out','ws','p','buy','cold','2024-01-01T00:00:00Z',
                'transfer_out','BTC',-40000000000,'now'),
                ('in','ws','p','buy','hot','2024-01-01T00:00:00Z',
                'transfer_in','BTC',39000000000,'now'),
                ('fee','ws','p','buy','cold','2024-01-01T00:00:00Z',
                'transfer_fee','BTC',-1000000000,'now'),
                ('earn','ws','p','buy','cold','2023-12-31T23:30:00Z',
                'income','BTC',100000000000,'now');
    """)
    report = _report(book)
    assert report["walletHoldings"] == [
        {"wallet": "Cold", "asset": "BTC", "quantitySats": 60000000, "marketValue": 18000.08},
        {"wallet": "Hot", "asset": "BTC", "quantitySats": 39000000, "marketValue": 11700.05},
    ]
    assert report["totals"]["neuQuantitySats"] == 99000000


def test_wallet_only_asset_gets_its_cached_quote(book):
    book.executescript("""
        INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,
            wallet_id,occurred_at,entry_type,asset,quantity,created_at)
            VALUES('rail','ws','p','buy','hot','2024-01-01T00:00:00Z',
                'transfer_in','LBTC',10000000000,'now');
        INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at)
            VALUES('BTC-EUR','2024-01-01T12:00:00Z',40000,'manual','now');
    """)
    report = _report(book)
    assert report["walletHoldings"][-1] == {
        "wallet": "Hot", "asset": "LBTC", "quantitySats": 10000000, "marketValue": 4000,
    }
    assert any(quote["asset"] == "LBTC" and quote["source"] == "cache" for quote in report["fmvSource"])


@pytest.mark.parametrize("check", ["title", "merge", "quarantine"])
def test_xlsx_keeps_assumptions_title_and_quarantine_disclosure(book, tmp_path, check):
    book.execute("INSERT INTO journal_quarantines VALUES('future','ws','p','missing_spot_price','{}','now')")
    artifact = tmp_path / "exit.xlsx"
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        reports.export_exit_tax_xlsx_report(
            book, "ws", "p", artifact, _report_hooks(), departure_date="2024-01-01"
        )
    workbook = openpyxl.load_workbook(artifact)
    try:
        if check == "merge":
            assert not any("single cell" in str(w.message).lower() for w in caught)
        if check == "title":
            assert workbook["Assumptions"]["A1"].value == "Assumptions & Steuerberater review gate"
        notes = [cell.value for row in workbook["Assumptions"] for cell in row if cell.value]
        if check == "quarantine":
            assert any("1 quarantined transaction(s)" in note and "incomplete" in note for note in notes)
    finally:
        workbook.close()


@pytest.mark.parametrize("format", ["pdf", "xlsx"])
def test_exit_estimate_does_not_register_realized_journal_snapshot(book, tmp_path, format):
    exporter = getattr(reports, f"export_exit_tax_{format}_report")
    artifact = tmp_path / f"exit.{format}"
    result = exporter(book, "ws", "p", artifact, _report_hooks(), departure_date="2024-01-01")
    assert artifact.is_file()
    assert "report_snapshot" not in result
    assert book.execute("SELECT COUNT(*) FROM filed_report_snapshots").fetchone()[0] == 0
    assert book.execute("SELECT COUNT(*) FROM filed_report_chain_captures").fetchone()[0] == 0


def test_exit_estimate_still_requires_fresh_journals(book):
    book.execute("UPDATE profiles SET last_processed_at=NULL")
    with pytest.raises(AppError, match="fresh journals"):
        _report(book)
