import pytest

from kassiber.core.accounting import evidence, ledger, workbench
from kassiber.core.accounting.commands import execute
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401


def test_workbench_is_read_only_and_scoped_with_complete_draft_count(book):
    conn, scope, _ = book
    for index in range(105):
        ledger.create_draft(conn, scope, dict(idempotency_key=str(index), period_id='2025',
            entry_date='2025-02-01', description='Synthetic local-only financial text',
            lines=[{'account_code': 'bank', 'debit_minor': 100}, {'account_code': 'sales', 'credit_minor': 100}]))
    conn.commit()
    before = conn.total_changes
    value = execute(conn, scope, 'workbench', {'period_id': '2025'})
    assert value['counts']['drafts'] == 105
    assert value['readiness']['ready'] is False
    assert value['count_scopes']['drafts'] == 'period'
    assert value['items'][0]['target'] == {
        'action': 'journal', 'payload': {'period_id': '2025', 'status': 'draft'}}
    for item in value['items']:
        target = item['target']
        assert 'view' not in target
        assert execute(conn, scope, target['action'], target['payload'])
    assert conn.total_changes == before
    assert conn.in_transaction is False
    with pytest.raises(AppError):
        workbench.snapshot(conn, 'another-book', period_id='2025')


def test_empty_workbench_does_not_claim_external_completeness(book):
    conn, scope, _ = book
    value = workbench.snapshot(conn, scope, period_id='2025')
    assert value['counts']['drafts'] == value['counts']['evidence'] == 0
    assert value['external_completeness_verified'] is False
    assert value['sources'] == {'bank_statements': [], 'truncated': False}


def test_evidence_is_honestly_book_scoped_and_inspection_keeps_caller_transaction(book):
    conn, scope, _ = book
    evidence.retain_evidence(conn, scope, content=b'Synthetic evidence', name='Private name', media_type='text/plain')
    assert conn.in_transaction
    value = workbench.snapshot(conn, scope, period_id='2025')
    assert value['counts']['evidence_unreviewed'] == 1
    assert value['count_scopes']['evidence'] == 'book'
    assert conn.in_transaction
    assert 'Private name' not in str(value)
