import pytest

from kassiber.core.accounting import ledger
from kassiber.core.accounting.commands import execute
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401


def test_readiness_no_write_and_exact_post_guard(book):
    conn, profile, _ = book
    before = ledger.require_book(conn, profile)['revision']
    ready = execute(conn, profile, 'close-readiness', {'period_id': '2025'})
    assert ready['ready'] and ready['revision'] == before
    assert not ready['external_completeness_verified'] and not ready['tax_filing_ready']
    draft = ledger.create_draft(conn, profile, dict(period_id='2025', entry_date='2025-01-02',
        description='Pending reviewed receipt', idempotency_key='readiness-draft',
        lines=[dict(account_code='bank', debit_minor=100), dict(account_code='sales', credit_minor=100)]))
    blocked = ledger.close_readiness(conn, profile, period_id='2025')
    assert not blocked['ready']
    assert blocked['blockers'] == [{'kind': 'unposted_drafts', 'count': 1}]
    assert ledger.require_book(conn, profile)['revision'] == blocked['revision']
    with pytest.raises(AppError) as stale:
        ledger.close_period(conn, profile, period_id='2025', expected_revision=before)
    assert stale.value.code == 'accounting_stale_approval'
    with pytest.raises(AppError) as current:
        ledger.close_period(conn, profile, period_id='2025', expected_revision=blocked['revision'])
    assert current.value.details['blockers'] == blocked['blockers']
    ledger.post_draft(conn, profile, draft_id=draft['id'], expected_digest=draft['payload_digest'])
    assert ledger.close_readiness(conn, profile, period_id='2025')['ready']
