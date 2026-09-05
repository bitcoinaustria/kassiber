from uuid import uuid4

import pytest

from kassiber.core.accounting import cashbook as cash, evidence, ledger
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401


@pytest.fixture
def cash_book(book):
    conn, profile, _ = book
    cash.ensure_schema(conn)
    for code, kind in [('cash', 'asset'), ('receivable', 'asset'), ('cost', 'expense'), ('loan', 'liability')]:
        ledger.create_account(conn, profile, code=code, name=code, kind=kind)
    source = evidence.retain_evidence(conn, profile, content=b'Synthetic reviewed count/payment evidence', media_type='text/plain', name='Test')
    for code, role in [('cash', 'cash'), ('bank', 'bank'), ('loan', 'loan')]:
        cash.select_account(conn, profile, account_code=code, role=role, effective_from='2025-01-01', reason='Reviewed chart', idempotency_key=code)
    return conn, profile, source['id']


def post(conn, profile, lines, date='2025-02-01'):
    draft = ledger.create_draft(conn, profile, dict(idempotency_key=uuid4().hex, period_id='2025',
        entry_date=date, description='Synthetic posted payment', lines=lines))
    return ledger.post_draft(conn, profile, draft_id=draft['id'], expected_digest=draft['payload_digest'])


def classify(cash_book, entry, classification='income', amount=1000, cash_index=0, offset_index=1, key=None):
    conn, profile, evidence_id = cash_book
    return cash.classify_flow(conn, profile, cash_line_id=entry['lines'][cash_index]['id'], offset_line_id=entry['lines'][offset_index]['id'],
        amount_minor=amount, classification=classification, evidence_id=evidence_id, locator='Page 1', reason='Human checked payment purpose', idempotency_key=key or uuid4().hex)


def test_explicit_counts_never_turn_missing_into_zero_and_compare_same_date(cash_book):
    conn, profile, source = cash_book
    result = cash.reconciliation(conn, profile, as_of='2025-02-01')
    assert result['rows'][0]['counted_minor'] is None
    assert result['rows'][0]['difference_minor'] is None
    assert not result['complete']
    count = cash.retain_count(conn, profile, account_code='cash', count_date='2025-02-01', counted_minor=0,
        evidence_id=source, locator='Count sheet', reason='Counted at day end', idempotency_key='zero')
    assert cash.reconciliation(conn, profile, as_of='2025-02-01')['complete']
    later = cash.reconciliation(conn, profile, as_of='2025-02-02')['rows'][0]
    assert later['counted_minor'] is None and later['latest_count_date'] == '2025-02-01'
    cash.void_record(conn, profile, record_kind='counts', record_id=count['id'], reason='Wrong count', idempotency_key='void-count')
    assert not cash.reconciliation(conn, profile, as_of='2025-02-01')['complete']
    with pytest.raises(Exception, match='accounting_cash_retained'):
        conn.execute('DELETE FROM gl_cash_counts')


def test_cash_income_is_not_accrual_profit_and_asset_statement_is_gl(cash_book):
    conn, profile, source = cash_book
    post(conn, profile, [dict(account_code='receivable', debit_minor=5000), dict(account_code='sales', credit_minor=5000)])
    receipt = post(conn, profile, [dict(account_code='cash', debit_minor=2000), dict(account_code='receivable', credit_minor=2000)])
    classify(cash_book, receipt, amount=2000)
    result = cash.report(conn, profile, start_date='2025-01-01', end_date='2025-12-31')
    assert result['income_minor'] == 2000 and result['complete']
    assert ledger.financial_statements(conn, profile, period_id='2025')['profit_minor'] == 5000
    assets = cash.asset_statement(conn, profile, as_of='2025-12-31')
    assert assets['assets_minor'] == 5000 and assets['net_assets_minor'] == 5000
    assert assets['accounting_regime'] == 'accrual'
    cash.retain_count(conn, profile, account_code='cash', count_date='2025-12-31', counted_minor=1999,
        evidence_id=source, locator='Sheet 1', reason='Actual count', idempotency_key='count')
    count = cash.reconciliation(conn, profile, as_of='2025-12-31')['rows'][0]
    assert count['difference_minor'] == -1 and count['status'] == 'count_mismatch'


@pytest.mark.parametrize('offset', ['bank', 'loan', 'capital'])
def test_internal_transfer_loan_and_equity_cannot_be_income(cash_book, offset):
    conn, profile, _ = cash_book
    entry = post(conn, profile, [dict(account_code='cash', debit_minor=1000), dict(account_code=offset, credit_minor=1000)])
    with pytest.raises(AppError):
        classify(cash_book, entry)
    classify(cash_book, entry, 'non_result')
    result = cash.report(conn, profile, start_date='2025-01-01', end_date='2025-12-31')
    assert result['complete'] and result['income_minor'] == result['expenditure_minor'] == 0
    if offset == 'bank':
        assert sum(row['amount_minor'] for row in result['rows']) == 0
        with pytest.raises(AppError, match='unallocated'):
            classify(cash_book, entry, 'non_result', cash_index=1, offset_index=0)


def test_mixed_payment_requires_partial_review_without_duplicate_endpoint_coverage(cash_book):
    conn, profile, _ = cash_book
    entry = post(conn, profile, [dict(account_code='cash', debit_minor=1500),
        dict(account_code='sales', credit_minor=1000), dict(account_code='loan', credit_minor=500)])
    allocation = classify(cash_book, entry, amount=400, key='partial')
    assert classify(cash_book, entry, amount=400, key='partial')['id'] == allocation['id']
    report = cash.report(conn, profile, start_date='2025-01-01', end_date='2025-12-31')
    assert not report['complete'] and report['unclassified'][0]['remaining_minor'] == 1100
    with pytest.raises(AppError, match='unallocated'):
        classify(cash_book, entry, amount=601)
    classify(cash_book, entry, amount=600)
    classify(cash_book, entry, 'non_result', amount=500, offset_index=2)
    result = cash.report(conn, profile, start_date='2025-01-01', end_date='2025-12-31')
    assert result['complete'] and result['income_minor'] == 1000


def test_reversals_keep_original_asof_and_inverse_at_reversal_date(cash_book):
    conn, profile, _ = cash_book
    entry = post(conn, profile, [dict(account_code='cash', debit_minor=1000), dict(account_code='sales', credit_minor=1000)])
    classify(cash_book, entry)
    reversed_entry = ledger.reverse_entry(conn, profile, entry_id=entry['id'], entry_date='2025-03-01', period_id='2025', idempotency_key='reverse', reason='Reviewed reversal')
    early = cash.report(conn, profile, start_date='2025-01-01', end_date='2025-02-28')
    late = cash.report(conn, profile, start_date='2025-03-01', end_date='2025-03-31')
    annual = cash.report(conn, profile, start_date='2025-01-01', end_date='2025-12-31')
    assert early['income_minor'] == 1000 and early['complete']
    assert late['income_minor'] == -1000 and late['complete']
    assert annual['income_minor'] == 0 and annual['complete']
    with pytest.raises(AppError):
        classify(cash_book, reversed_entry, 'non_result')


def test_exact_large_amount_and_open_period_guard(cash_book):
    conn, profile, source = cash_book
    value = 9007199254740997
    entry = post(conn, profile, [dict(account_code='cash', debit_minor=value), dict(account_code='sales', credit_minor=value)])
    allocation = classify(cash_book, entry, amount=value)
    assert cash.report(conn, profile, start_date='2025-01-01', end_date='2025-12-31')['income_minor'] == value
    # Isolate supporting-record close guard without exercising unrelated close controls.
    conn.execute("UPDATE gl_periods SET state='closed' WHERE profile_id=? AND id='2025'", (profile,))
    with pytest.raises(AppError) as error:
        cash.void_record(conn, profile, record_kind='flows', record_id=allocation['id'], reason='Later correction', idempotency_key='void')
    assert error.value.code == 'accounting_period_closed'
    for amount in (None, True, 1.2, -1, 2**63):
        with pytest.raises(AppError):
            cash.retain_count(conn, profile, account_code='cash', count_date='2025-02-01', counted_minor=amount,
                evidence_id=source, locator='Sheet', reason='Count', idempotency_key=uuid4().hex)


def test_source_scope_and_same_entry_are_enforced(cash_book):
    conn, profile, source = cash_book
    one = post(conn, profile, [dict(account_code='cash', debit_minor=1000), dict(account_code='sales', credit_minor=1000)])
    two = post(conn, profile, [dict(account_code='cash', debit_minor=1000), dict(account_code='sales', credit_minor=1000)])
    with pytest.raises(AppError):
        cash.classify_flow(conn, profile, cash_line_id=one['lines'][0]['id'], offset_line_id=two['lines'][1]['id'],
            amount_minor=1000, classification='income', evidence_id=source, locator='Sheet', reason='Review', idempotency_key='bad')
    with pytest.raises(AppError):
        cash.retain_count(conn, profile, account_code='cash', count_date='2025-02-01', counted_minor=10,
            evidence_id='other-book-source', locator='Sheet', reason='Count', idempotency_key='bad-evidence')
    assert conn.execute('SELECT COUNT(*) FROM gl_cash_flows').fetchone()[0] == 0


def test_optional_basis_is_not_automatically_selected(book):
    conn, profile, _ = book
    cash.ensure_schema(conn)
    result = cash.report(conn, profile, start_date='2025-01-01', end_date='2025-12-31')
    assert result['configured'] is False and result['income_minor'] is None and not result['complete']


def test_sql_scope_and_allocation_budget_guards(cash_book):
    conn, profile, source = cash_book
    entry = post(conn, profile, [dict(account_code='cash', debit_minor=1000), dict(account_code='sales', credit_minor=1000)])
    first = classify(cash_book, entry)
    with pytest.raises(Exception, match='accounting_cash_allocation_exceeded'):
        conn.execute('INSERT INTO gl_cash_flows VALUES(?,?,?,?,?,?,?,?,?,?,?)',
            (uuid4().hex, profile, first['cash_line_id'], first['offset_line_id'], 1, 'income', source, 'Sheet', 'Extra', 'extra', 'digest'))
    with pytest.raises(Exception, match='accounting_cash_retained'):
        conn.execute('UPDATE gl_cash_flows SET amount_minor=1')


def test_accrual_close_has_no_forced_cash_basis_but_selected_counts_gate_close(book):
    conn, profile, _ = book
    cash.ensure_schema(conn)
    before = ledger.require_book(conn, profile)['revision']
    checks = cash.validate_close(conn, profile, '2025-01-01', '2025-12-31')
    assert checks['complete'] and not checks['required'] and not checks['configured']
    assert ledger.require_book(conn, profile)['revision'] == before
    cash.select_account(conn, profile, account_code='bank', role='cash', effective_from='2025-01-01', reason='Reviewed physical cash ledger', idempotency_key='choose')
    checks = cash.validate_close(conn, profile, '2025-01-01', '2025-12-31')
    assert not checks['complete'] and checks['blockers'][0]['code'] == 'accounting_cash_missing_count'


def test_cash_basis_requires_selection_and_complete_payment_coverage(book):
    from kassiber.core.accounts import create_profile
    conn, previous, _ = book
    workspace = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (previous,)).fetchone()[0]
    profile = create_profile(conn, workspace, 'Cash-basis test', 'EUR', 'FIFO', 'generic', 365)['id']
    ledger.configure_book(conn, profile, currency='EUR', timezone='Europe/Vienna', accounting_regime='cash_basis')
    ledger.create_account(conn, profile, code='bank', name='Bank', kind='asset')
    ledger.create_account(conn, profile, code='sales', name='Sales', kind='income')
    ledger.create_period(conn, profile, period_id='2025', start_date='2025-01-01', end_date='2025-12-31')
    cash.ensure_schema(conn)
    source = evidence.retain_evidence(conn, profile, content=b'Cash basis source', media_type='text/plain', name='Test')
    checks = cash.validate_close(conn, profile, '2025-01-01', '2025-12-31')
    assert checks['required'] and checks['blockers'] == [{'code': 'accounting_cash_basis_missing'}]
    cash.select_account(conn, profile, account_code='bank', role='bank', effective_from='2025-01-01', reason='Reviewed all payment accounts', idempotency_key='bank')
    entry = post(conn, profile, [dict(account_code='bank', debit_minor=1000), dict(account_code='sales', credit_minor=1000)])
    checks = cash.validate_close(conn, profile, '2025-01-01', '2025-12-31')
    assert checks['blockers'] == [{'code': 'accounting_cash_flows_unclassified', 'count': 1}]
    classify((conn, profile, source['id']), entry)
    checks = cash.validate_close(conn, profile, '2025-01-01', '2025-12-31')
    assert checks['complete'] and checks['report']['income_minor'] == 1000


def test_real_ledger_close_blocks_missing_count_and_retains_matched_controls(cash_book):
    conn, profile, source = cash_book
    before = ledger.require_book(conn, profile)['revision']
    with pytest.raises(AppError) as error:
        ledger.close_period(conn, profile, period_id='2025', expected_revision=before)
    assert error.value.code == 'accounting_close_blocked'
    assert any(row['code'] == 'accounting_cash_missing_count' and row.get('account_code') == 'cash'
               for row in error.value.details['blockers'])
    assert conn.execute("SELECT state FROM gl_periods WHERE profile_id=? AND id='2025'", (profile,)).fetchone()[0] == 'open'
    assert ledger.require_book(conn, profile)['revision'] == before
    cash.retain_count(conn, profile, account_code='cash', count_date='2025-12-31', counted_minor=0,
        evidence_id=source, locator='Signed zero count', reason='Day end checked', idempotency_key='closing-count')
    result = ledger.close_period(conn, profile, period_id='2025', expected_revision=ledger.require_book(conn, profile)['revision'])
    assert result['snapshot']['cash_controls']['complete']
    assert result['snapshot']['cash_controls']['reconciliation']['rows'][0]['counted_minor'] == 0
    with pytest.raises(AppError):
        cash.retain_count(conn, profile, account_code='cash', count_date='2025-12-31', counted_minor=1,
            evidence_id=source, locator='Changed count', reason='Correction', idempotency_key='closed-count')


def test_future_account_selection_is_not_in_an_earlier_report(cash_book):
    conn, profile, _ = cash_book
    ledger.create_period(conn, profile, period_id='2026', start_date='2026-01-01', end_date='2026-12-31')
    ledger.create_account(conn, profile, code='later-bank', name='Later bank', kind='asset')
    cash.select_account(conn, profile, account_code='later-bank', role='bank', effective_from='2026-01-01', reason='New next-year bank', idempotency_key='later-bank')
    report = cash.report(conn, profile, start_date='2025-01-01', end_date='2025-12-31')
    assert 'later-bank' not in {row['account_code'] for row in report['selections']}


@pytest.mark.parametrize('invalid', [None, True, [], {}, 1.5, '', 'x\0y', 'x' * 201])
def test_public_identifiers_fail_with_typed_errors(cash_book, invalid):
    conn, profile, source = cash_book
    calls = [
        lambda: cash.reconciliation(conn, invalid, as_of='2025-12-31'),
        lambda: cash.reconciliation(conn, profile, as_of=invalid),
        lambda: cash.asset_statement(conn, profile, as_of=invalid),
        lambda: cash.select_account(conn, profile, account_code=invalid, role='cash', effective_from='2025-01-01', reason='Review', idempotency_key='bad'),
        lambda: cash.select_account(conn, profile, account_code='cash', role=invalid, effective_from='2025-01-01', reason='Review', idempotency_key='bad'),
        lambda: cash.retain_count(conn, profile, account_code=invalid, count_date='2025-12-31', counted_minor=0, evidence_id=source, locator='Sheet', reason='Review', idempotency_key='bad'),
        lambda: cash.retain_count(conn, profile, account_code='cash', count_date='2025-12-31', counted_minor=0, evidence_id=invalid, locator='Sheet', reason='Review', idempotency_key='bad'),
        lambda: cash.classify_flow(conn, profile, cash_line_id=invalid, offset_line_id='x', amount_minor=1, classification='income', evidence_id=source, locator='Sheet', reason='Review', idempotency_key='bad'),
        lambda: cash.void_record(conn, profile, record_kind='flows', record_id=invalid, reason='Review', idempotency_key='bad'),
        lambda: cash.void_record(conn, profile, record_kind=invalid, record_id='x', reason='Review', idempotency_key='bad'),
    ]
    for call in calls:
        with pytest.raises(AppError):
            call()
