"""Independent cash report boundary regressions with real SQLCipher/GL rows."""
import pytest

from kassiber.core.accounting import cashbook as cash, ledger
from kassiber.core.accounts import create_profile
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_cashbook import post


@pytest.fixture
def cash_basis(book):
    conn, previous, _ = book
    workspace = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (previous,)).fetchone()[0]
    profile = create_profile(conn,workspace,'Cash basis adversarial','EUR','FIFO','generic',365)['id']
    ledger.configure_book(conn,profile,currency='EUR',timezone='Europe/Vienna',accounting_regime='cash_basis')
    for code,kind in [('bank','asset'),('sales','income'),('equity','equity')]:
        ledger.create_account(conn,profile,code=code,name=code,kind=kind)
    ledger.create_period(conn,profile,period_id='2025',start_date='2025-01-01',end_date='2025-12-31')
    return conn,profile


def select_later(conn,profile):
    return cash.select_account(conn,profile,account_code='bank',role='bank',effective_from='2025-07-01',
        reason='Reviewed bank selection',idempotency_key='bank-selection')


def test_later_account_selection_cannot_hide_earlier_current_period_payments(cash_basis):
    conn,profile = cash_basis
    payment = post(conn,profile,[dict(account_code='bank',debit_minor=1000),dict(account_code='sales',credit_minor=1000)],date='2025-01-15')
    select_later(conn,profile)
    check = cash.validate_close(conn,profile,'2025-01-01','2025-12-31')
    assert not check['complete'] and not check['report']['complete']
    assert check['blockers'] == [dict(code='accounting_cash_selection_gap',count=1)]
    assert check['report']['coverage_gaps'] == [dict(line_id=payment['lines'][0]['id'],entry_id=payment['id'],
        account_code='bank',occurred_on='2025-01-15',effective_from='2025-07-01')]
    assert check['report']['rows'] == []  # No retroactive classification invented.
    with pytest.raises(AppError) as error:
        ledger.close_period(conn,profile,period_id='2025',expected_revision=ledger.require_book(conn,profile)['revision'])
    assert error.value.code == 'accounting_close_blocked'
    assert any(row['code']=='accounting_cash_selection_gap' for row in error.value.details['blockers'])


def test_initial_opening_balance_is_not_a_missing_cash_payment(cash_basis):
    conn,profile = cash_basis
    draft = ledger.create_draft(conn,profile,dict(period_id='2025',entry_date='2025-01-01',entry_kind='opening',
        description='Reviewed initial opening',idempotency_key='opening',
        lines=[dict(account_code='bank',debit_minor=1000),dict(account_code='equity',credit_minor=1000)]))
    ledger.post_draft(conn,profile,draft_id=draft['id'],expected_digest=draft['payload_digest'])
    select_later(conn,profile)
    check = cash.validate_close(conn,profile,'2025-01-01','2025-12-31')
    assert check['complete'] and check['report']['complete']
    assert check['report']['coverage_gaps'] == [] and check['report']['income_minor'] == 0


def test_activity_before_report_interval_does_not_create_a_false_coverage_gap(cash_basis):
    conn,profile = cash_basis
    post(conn,profile,[dict(account_code='bank',debit_minor=1000),dict(account_code='sales',credit_minor=1000)],date='2025-01-15')
    select_later(conn,profile)
    report = cash.report(conn,profile,start_date='2025-07-01',end_date='2025-12-31')
    assert report['complete'] and report['coverage_gaps'] == []
    ledger.create_period(conn,profile,period_id='2026',start_date='2026-01-01',end_date='2026-12-31')
    check = cash.validate_close(conn,profile,'2026-01-01','2026-12-31')
    assert check['complete'] and check['report']['coverage_gaps'] == []


def test_optional_accrual_cash_report_shows_gap_without_forcing_cash_basis_close(book):
    conn,profile,_ = book
    post(conn,profile,[dict(account_code='bank',debit_minor=1000),dict(account_code='sales',credit_minor=1000)],date='2025-01-15')
    select_later(conn,profile)
    check = cash.validate_close(conn,profile,'2025-01-01','2025-12-31')
    assert not check['required'] and check['complete']
    assert not check['report']['complete'] and check['report']['coverage_gaps']
