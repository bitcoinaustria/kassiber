"""Exact local additive task scope, retained history and stale agent consent."""
import json

import pytest

from kassiber import daemon_accounting_tasks as adapter
from kassiber.core.accounting import evidence, ledger, tasks
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_tasks import approve, assign_document, evidence_task, reviewed_document, rule, setup


def amendment(conn, profile, task, document, *, key='amend'):
    selection = dict(task_id=task['id'], period_id='2025', evidence_ids=[document['id']],
                     reason='Locally reviewed newly supplied invoice')
    preview = tasks.execute(conn, profile, 'task-amend-preview', selection)
    return dict(**selection, expected_digest=preview['expected_digest'],
                expected_revision=preview['expected_revision'], idempotency_key=key, confirmed=True)


def test_new_document_joins_same_task_preserving_history_and_staling_agent_consent(book):
    conn, profile, _ = book
    task, _ = setup(conn, profile)
    rule(conn, profile)
    prepared, original_request = approve(conn, profile, task['id'], 'prepare')
    original_spec = conn.execute('SELECT spec_json FROM gl_accounting_tasks WHERE id=?', (task['id'],)).fetchone()[0]
    prior_receipts = tasks.get(conn, profile, task['id'])['receipts']
    grants = adapter.TaskApprovals()
    old = adapter.execute(conn, profile, 'ui.accounting.task_preview', {'task_id': task['id'], 'step': 'post'}, grants)
    document, review = reviewed_document(conn, profile)
    with pytest.raises(AppError) as denied:
        assign_document(conn, profile, task, document, review)
    assert denied.value.code == 'accounting_task_source_scope'
    payload = amendment(conn, profile, task, document)
    result = tasks.execute(conn, profile, 'task-amend', payload)
    current = result['task']
    assert current['id'] == task['id'] and current['scope_revision'] == 1
    assert current['source_count'] == 2 and current['state'] == 'attention'
    assert current['receipts'][:-1] == prior_receipts
    assert conn.execute('SELECT spec_json FROM gl_accounting_tasks WHERE id=?', (task['id'],)).fetchone()[0] == original_spec
    assert ledger.require_book(conn, profile)['revision'] == payload['expected_revision'] + 1
    with pytest.raises(AppError) as stale:
        adapter.consent_preview(conn, profile, {'task_id': task['id'], 'approval_id': old['approval_id']}, grants)
    assert stale.value.code == 'accounting_stale_approval'
    retry = tasks.execute(conn, profile, 'task-amend', payload)
    assert retry['already_applied'] and retry['receipt'] == result['receipt']
    assert retry['task']['scope_revision'] == 1
    assert tasks.execute(conn, profile, 'task-apply', original_request)['receipt']['id'] == prepared['receipt']['id']
    assign_document(conn, profile, task, document, review)
    approve(conn, profile, task['id'], 'prepare', 'prepare-new-document')
    posted, _ = approve(conn, profile, task['id'], 'post')
    assert len(posted['result']['posted_ids']) == 2
    assert all(row['status'] == 'posted' for row in posted['task']['coverage'])
    with pytest.raises(Exception, match='accounting_task_retained'):
        conn.execute('UPDATE gl_accounting_task_receipts SET result_json=? WHERE id=?', ('{}', result['receipt']['id']))


@pytest.mark.parametrize('change', ['book', 'journal', 'task_receipt', 'reason', 'selection', 'revision_bool'])
def test_amendment_exact_binding_rejects_changes(book, change):
    conn, profile, _ = book
    task, _ = setup(conn, profile)
    rule(conn, profile)
    document, _ = reviewed_document(conn, profile)
    payload = amendment(conn, profile, task, document)
    if change == 'book':
        ledger.create_account(conn, profile, code='later', name='Later', kind='expense')
    elif change == 'journal':
        conn.execute('UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id=?', (profile,))
    elif change == 'task_receipt':
        approve(conn, profile, task['id'], 'prepare')
    elif change == 'reason':
        payload['reason'] = 'Changed approval intent'
    elif change == 'selection':
        other = evidence.retain_evidence(conn, profile, content=b'Different new invoice', media_type='text/plain', name='Other')
        payload['evidence_ids'] = [other['id']]
    else:
        payload['expected_revision'] = True
    with pytest.raises(AppError) as error:
        tasks.execute(conn, profile, 'task-amend', payload)
    assert error.value.code == 'accounting_stale_approval'
    assert tasks.get(conn, profile, task['id'])['scope_revision'] == 0


@pytest.mark.parametrize('change', ['cancel', 'closed', 'period', 'missing', 'empty', 'implicit', 'denied'])
def test_amendment_refuses_unreviewed_or_invalid_scope(book, change):
    conn, profile, _ = book
    task = evidence_task(conn, profile, [])
    document, _ = reviewed_document(conn, profile)
    payload = amendment(conn, profile, task, document)
    expected = 'accounting_task_source_scope'
    if change == 'cancel':
        tasks.execute(conn, profile, 'task-cancel', dict(task_id=task['id'], reason='Cancelled'))
        expected = 'accounting_task_cancelled'
    elif change == 'closed':
        ledger.close_period(conn, profile, period_id='2025', expected_revision=ledger.require_book(conn, profile)['revision'])
        expected = 'accounting_task_blocked'
    elif change == 'period':
        payload['period_id'] = '2026'
    elif change == 'missing':
        payload['evidence_ids'] = ['unretained-document']
        expected = 'accounting_evidence_not_found'
    elif change == 'empty':
        payload['evidence_ids'] = []
    elif change == 'implicit':
        payload['include_period_evidence'] = True
        expected = 'accounting_task_invalid'
    else:
        payload['confirmed'] = False
        expected = 'accounting_task_consent_required'
    with pytest.raises(AppError) as error:
        tasks.execute(conn, profile, 'task-amend', payload)
    assert error.value.code == expected
    assert tasks.get(conn, profile, task['id'])['scope_revision'] == 0


def test_scope_is_book_bound_and_retries_reject_changed_idempotency(book):
    from kassiber.core.accounts import create_profile
    conn, profile, _ = book
    task, _ = setup(conn, profile)
    document, _ = reviewed_document(conn, profile)
    payload = amendment(conn, profile, task, document)
    workspace = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (profile,)).fetchone()[0]
    other = create_profile(conn, workspace, 'Other', 'EUR', 'FIFO', 'generic', 365)['id']
    ledger.configure_book(conn, other, currency='EUR', timezone='Europe/Vienna')
    with pytest.raises(AppError) as error:
        tasks.execute(conn, other, 'task-amend', payload)
    assert error.value.code == 'accounting_task_not_found'
    foreign = evidence.retain_evidence(conn, other, content=b'Foreign invoice', media_type='text/plain', name='Foreign')
    with pytest.raises(AppError) as error:
        tasks.execute(conn, profile, 'task-amend-preview', dict(task_id=task['id'], period_id='2025', evidence_ids=[foreign['id']], reason='Wrong book'))
    assert error.value.code == 'accounting_evidence_not_found'
    tasks.execute(conn, profile, 'task-amend', payload)
    with pytest.raises(AppError) as error:
        tasks.execute(conn, profile, 'task-amend', {**payload, 'reason': 'Changed retry'})
    assert error.value.code == 'accounting_idempotency_conflict'
    with pytest.raises(AppError) as error:
        amendment(conn, profile, task, document, key='duplicate')
    assert error.value.code == 'accounting_task_source_scope'


def test_amendment_rolls_back_receipt_and_revision_together(book, monkeypatch):
    conn, profile, _ = book
    task, _ = setup(conn, profile)
    document, _ = reviewed_document(conn, profile)
    payload = amendment(conn, profile, task, document)
    bump = ledger._bump
    def fail_after_bump(conn, profile_id):
        bump(conn, profile_id)
        raise AppError('Synthetic rollback', code='test_rollback')
    monkeypatch.setattr(ledger, '_bump', fail_after_bump)
    with pytest.raises(AppError):
        tasks.execute(conn, profile, 'task-amend', payload)
    assert ledger.require_book(conn, profile)['revision'] == payload['expected_revision']
    assert tasks.get(conn, profile, task['id'])['scope_revision'] == 0
    assert not tasks.get(conn, profile, task['id'])['receipts']


def test_additive_scope_limit_and_retry_after_cancel(book, monkeypatch):
    conn, profile, _ = book
    task, _ = setup(conn, profile)
    document, _ = reviewed_document(conn, profile)
    payload = amendment(conn, profile, task, document)
    monkeypatch.setattr(tasks, 'MAX_SOURCES', 1)
    with pytest.raises(AppError) as error:
        tasks.execute(conn, profile, 'task-amend', payload)
    assert error.value.code == 'accounting_task_population_limit'
    monkeypatch.setattr(tasks, 'MAX_SOURCES', 10000)
    applied = tasks.execute(conn, profile, 'task-amend', payload)
    tasks.execute(conn, profile, 'task-cancel', dict(task_id=task['id'], reason='Finished review'))
    retry = tasks.execute(conn, profile, 'task-amend', payload)
    assert retry['receipt'] == applied['receipt'] and retry['already_applied']
    assert retry['task']['state'] == 'cancelled'


def test_reopened_completed_task_amendment_preserves_old_export_not_completion(book):
    conn, profile, _ = book
    task = evidence_task(conn, profile, [])
    approve(conn, profile, task['id'], 'close')
    preview = tasks.preview(conn, profile, task['id'], 'export_close')
    exported = tasks.execute(conn, profile, 'task-apply', dict(task_id=task['id'], step='export_close',
        expected_digest=preview['expected_digest'], expected_revision=preview['expected_revision'],
        confirmed=True, confirm_plaintext=True, idempotency_key='export'))
    assert exported['task']['state'] == 'completed'
    historical = exported['task']['receipts']
    ledger.reopen_period(conn, profile, period_id='2025', reason='New evidence received',
                         expected_revision=ledger.require_book(conn, profile)['revision'])
    document, _ = reviewed_document(conn, profile)
    result = tasks.execute(conn, profile, 'task-amend', amendment(conn, profile, task, document))
    assert result['task']['state'] == 'attention'
    assert result['task']['receipts'][:-1] == historical
    current_export = tasks.preview(conn, profile, task['id'], 'export_close')
    assert not current_export['ready']
    assert {'kind': 'task_close_required'} in current_export['blockers']


def test_amendment_is_local_only_and_survives_fresh_cli_process(book):
    from kassiber.ai.tools import get_tool
    from tests.test_accounting_cli_tasks import cli
    conn, profile, root = book
    task, _ = setup(conn, profile)
    document, _ = reviewed_document(conn, profile)
    conn.commit()
    selection = dict(task_id=task['id'], period_id='2025', evidence_ids=[document['id']], reason='Private amendment text')
    preview = cli(root, 'task-amend-preview', selection)['data']
    payload = dict(**selection, expected_digest=preview['expected_digest'], expected_revision=preview['expected_revision'],
                   confirmed=True, idempotency_key='cli-amend')
    applied = cli(root, 'task-amend', payload)['data']
    retry = cli(root, 'task-amend', payload)['data']
    assert retry['already_applied'] and retry['receipt'] == applied['receipt']
    current = cli(root, 'task-get', {'task_id': task['id']})['data']
    assert current['spec']['evidence_ids'] == [document['id']]
    redacted = json.dumps(adapter.summary(current))
    for private in (document['id'], document['content_sha256'], document['name'], selection['reason']):
        assert private not in redacted
    assert get_tool('ui.accounting.task_amend') is None
    assert get_tool('ui.accounting.task_amend_preview') is None
