"""Real SQLCipher task preparation, approval, replay and complete population."""
import pytest

from kassiber.core.accounting import bank, ledger, tasks
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401


def setup(conn, profile_id, count=1):
    tasks.ensure_schema(conn)
    statement = bank.import_statement(conn, profile_id, account_code='bank', statement_id='task-source',
        start_date='2025-01-01', end_date='2025-12-31',
        csv_text='row_id,date,amount_minor,description\n' + ''.join(f'{i},2025-02-03,100,Membership\n' for i in range(count)))
    task = tasks.execute(conn, profile_id, 'task-create',
        {'period_id': '2025', 'statement_ids': [statement['id']], 'idempotency_key': 'task'})
    return task, statement


def rule(conn, profile_id):
    return tasks.execute(conn, profile_id, 'rule-create', dict(idempotency_key='rule', account_code='bank',
        direction='in', description_exact='Membership', counter_account_code='sales', reason='Reviewed receipts', confirmed=True))


def approve(conn, profile_id, task_id, step, key=None):
    preview = tasks.execute(conn, profile_id, 'task-preview', dict(task_id=task_id, step=step))
    payload = dict(task_id=task_id, step=step, expected_revision=preview['expected_revision'],
        expected_digest=preview['expected_digest'], confirmed=True, idempotency_key=key or step)
    return tasks.execute(conn, profile_id, 'task-apply', payload), payload


def test_full_population_prepares_posts_and_resumes_without_duplicates(book):
    conn, profile_id, _ = book
    task, _ = setup(conn, profile_id, 123)
    assert task['source_count'] == 123
    assert len(task['exceptions']) == 123
    assert task['next_step'] is None
    rule(conn, profile_id)
    preview = tasks.preview(conn, profile_id, task['id'], 'prepare')
    assert len(preview['proposals']) == 123
    assert conn.execute('SELECT count(*) FROM gl_entries').fetchone()[0] == 0
    prepared, request = approve(conn, profile_id, task['id'], 'prepare')
    assert len(prepared['result']['draft_ids']) == 123
    assert tasks.execute(conn, profile_id, 'task-apply', request)['already_applied']
    conn.commit()
    posted, request = approve(conn, profile_id, task['id'], 'post')
    assert len(posted['result']['posted_ids']) == 123
    assert all(r['status'] == 'posted' for r in posted['task']['coverage'])
    assert tasks.execute(conn, profile_id, 'task-apply', request)['already_applied']
    assert conn.execute('SELECT count(*) FROM gl_entries').fetchone()[0] == 123
    assert conn.execute('SELECT count(*) FROM gl_bank_allocations').fetchone()[0] == 123
    assert len(tasks.get(conn, profile_id, task['id'])['receipts']) == 2


def test_same_sources_in_another_task_do_not_generate_another_entry(book):
    conn, profile_id, _ = book
    task, statement = setup(conn, profile_id)
    rule(conn, profile_id)
    approve(conn, profile_id, task['id'], 'prepare')
    other = tasks.execute(conn, profile_id, 'task-create',
        dict(period_id='2025', statement_ids=[statement['id']], idempotency_key='other'))
    assert other['coverage'][0]['status'] == 'draft'
    assert not tasks.preview(conn, profile_id, other['id'], 'prepare')['ready']
    approve(conn, profile_id, other['id'], 'post')
    assert tasks.get(conn, profile_id, task['id'])['coverage'][0]['status'] == 'posted'
    assert conn.execute('SELECT count(*) FROM gl_entries').fetchone()[0] == 1


@pytest.mark.parametrize('change', ['rule', 'book', 'journal', 'cancel'])
def test_changed_authority_refuses_old_approval(book, change):
    conn, profile_id, _ = book
    task, _ = setup(conn, profile_id)
    assignment = rule(conn, profile_id)
    preview = tasks.preview(conn, profile_id, task['id'], 'prepare')
    if change == 'rule':
        tasks.execute(conn, profile_id, 'rule-revoke', dict(rule_id=assignment['id'], reason='Changed assignment'))
    elif change == 'book':
        ledger.create_account(conn, profile_id, code='another', name='Another', kind='expense')
    elif change == 'journal':
        conn.execute('UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id=?', (profile_id,))
    else:
        tasks.execute(conn, profile_id, 'task-cancel', dict(task_id=task['id'], reason='Cancelled'))
    with pytest.raises(AppError, match='explicitly reviewed') as exc:
        tasks.execute(conn, profile_id, 'task-apply', dict(task_id=task['id'], step='prepare',
            expected_revision=preview['expected_revision'], expected_digest=preview['expected_digest'], confirmed=True, idempotency_key='apply'))
    assert exc.value.code == 'accounting_stale_approval'
    assert conn.execute('SELECT count(*) FROM gl_entries').fetchone()[0] == 0


def test_denied_confirmation_never_prepares(book):
    conn, profile_id, _ = book
    task, _ = setup(conn, profile_id)
    rule(conn, profile_id)
    preview = tasks.preview(conn, profile_id, task['id'], 'prepare')
    with pytest.raises(AppError) as exc:
        tasks.execute(conn, profile_id, 'task-apply', dict(task_id=task['id'], step='prepare',
            expected_revision=preview['expected_revision'], expected_digest=preview['expected_digest'], confirmed=False, idempotency_key='denied'))
    assert exc.value.code == 'accounting_task_consent_required'
    assert conn.execute('SELECT count(*) FROM gl_entries').fetchone()[0] == 0


def test_scope_selection_required_and_cross_book_refused(book):
    conn, profile_id, _ = book
    task, _ = setup(conn, profile_id)
    with pytest.raises(AppError):
        tasks.execute(conn, profile_id, 'task-create', dict(period_id='2025', idempotency_key='implicit'))
    from kassiber.core.accounts import create_profile
    workspace = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (profile_id,)).fetchone()[0]
    other = create_profile(conn, workspace, 'Other', 'EUR', 'FIFO', 'generic', 365)['id']
    ledger.configure_book(conn, other, currency='EUR', timezone='Europe/Vienna')
    with pytest.raises(AppError) as exc:
        tasks.get(conn, other, task['id'])
    assert exc.value.code == 'accounting_task_not_found'


def reviewed_document(conn, profile_id, *, total=100, vat=0):
    from kassiber.core.accounting import document_text, evidence
    retained = evidence.retain_evidence(conn, profile_id, content=b'Test invoice for task', media_type='text/plain', name='Invoice')
    extraction = document_text.transcribe(conn, profile_id, evidence_id=retained['id'], pages=['Invoice 1'], reason='Reviewed source')
    reviewed = document_text.review_fields(conn, profile_id, extraction_id=extraction['id'], expected_digest=extraction['content_digest'],
        previous_id=None, fields=dict(total_minor=total, vat_minor=vat, issued_date='2025-02-03', currency='EUR', minor_unit_exponent=2),
        spans={}, reason='Verified source facts')
    return retained, reviewed


def evidence_task(conn, profile_id, evidence_ids):
    tasks.ensure_schema(conn)
    return tasks.execute(conn, profile_id, 'task-create', dict(period_id='2025', statement_ids=[], evidence_ids=evidence_ids, idempotency_key='documents'))


def assign_document(conn, profile_id, task, doc, review, **extra):
    return tasks.execute(conn, profile_id, 'task-source-assign', dict(task_id=task['id'], evidence_id=doc['id'],
        kind='evidence_posting', extraction_id=review['id'], expected_review_digest=review['review']['content_digest'],
        debit_account_code='bank', credit_account_code='sales', reason='Reviewed explicit assignment', confirmed=True,
        idempotency_key='assignment', **extra))


def test_reviewed_evidence_preparation_deduplicates_identical_bytes(book):
    from kassiber.core.accounting import evidence
    conn, profile_id, _ = book
    doc, review = reviewed_document(conn, profile_id)
    copy = evidence.retain_evidence(conn, profile_id, content=b'Test invoice for task', media_type='text/plain', name='Duplicate invoice')
    task = evidence_task(conn, profile_id, [doc['id'], copy['id']])
    assign_document(conn, profile_id, task, doc, review)
    preview = tasks.preview(conn, profile_id, task['id'], 'prepare')
    assert preview['source_count'] == 2
    assert len(preview['proposals']) == 1
    approve(conn, profile_id, task['id'], 'prepare')
    posted, _ = approve(conn, profile_id, task['id'], 'post')
    assert len(posted['result']['posted_ids']) == 1
    assert len(posted['task']['coverage']) == 2
    assert all(r['status'] == 'posted' for r in posted['task']['coverage'])


@pytest.mark.parametrize('prepare_first', [False, True])
def test_changed_document_review_is_named_exception_not_silent_repricing(book, prepare_first):
    from kassiber.core.accounting import document_text
    conn, profile_id, _ = book
    doc, review = reviewed_document(conn, profile_id)
    task = evidence_task(conn, profile_id, [doc['id']])
    assigned = assign_document(conn, profile_id, task, doc, review)
    if prepare_first:
        approve(conn, profile_id, task['id'], 'prepare')
    replacement = document_text.review_fields(conn, profile_id, extraction_id=review['id'], expected_digest=review['content_digest'],
        previous_id=review['review']['id'], fields=review['review']['fields'], spans={}, reason='Same values independently verified')
    updated = tasks.get(conn, profile_id, task['id'])
    assert updated['exceptions'][0]['exception'] == 'accounting_document_review_changed'
    tasks.execute(conn, profile_id, 'task-source-assign', dict(task_id=task['id'], evidence_id=doc['id'],
        kind='evidence_posting', extraction_id=review['id'], expected_review_digest=replacement['review']['content_digest'],
        debit_account_code='bank', credit_account_code='sales', reason='Current review', confirmed=True,
        idempotency_key='reassignment', previous_id=assigned['id']))
    assert not tasks.get(conn, profile_id, task['id'])['exceptions']


@pytest.mark.parametrize('total,vat', [(-100, 0), (100, 20), (0, 0)])
def test_refunds_tax_splits_and_zero_values_require_manual_semantics(book, total, vat):
    conn, profile_id, _ = book
    doc, review = reviewed_document(conn, profile_id, total=total, vat=vat)
    task = evidence_task(conn, profile_id, [doc['id']])
    with pytest.raises(AppError) as exc:
        assign_document(conn, profile_id, task, doc, review)
    assert exc.value.code == 'accounting_document_split_required'
    assert conn.execute('SELECT count(*) FROM gl_entries').fetchone()[0] == 0


def test_invoice_bank_evidence_alias_never_posts_second_expense(book):
    conn, profile_id, _ = book
    initial, statement = setup(conn, profile_id)
    doc, _ = reviewed_document(conn, profile_id)
    task = tasks.execute(conn, profile_id, 'task-create', dict(period_id='2025', statement_ids=[statement['id']],
        evidence_ids=[doc['id']], idempotency_key='combined'))
    row_id = initial['coverage'][0]['source_id']
    tasks.execute(conn, profile_id, 'task-source-assign', dict(task_id=task['id'], evidence_id=doc['id'], kind='bank_evidence',
        bank_row_id=row_id, reason='Invoice supporting this payment', confirmed=True, idempotency_key='link'))
    rule(conn, profile_id)
    assert len(tasks.preview(conn, profile_id, task['id'], 'prepare')['proposals']) == 1
    approve(conn, profile_id, task['id'], 'prepare')
    posted, _ = approve(conn, profile_id, task['id'], 'post')
    assert len(posted['result']['posted_ids']) == 1
    assert not posted['task']['exceptions']


def test_existing_unallocated_entry_prevents_blind_second_bank_post(book):
    from tests.test_accounting_integration import post
    conn, profile_id, _ = book
    task, _ = setup(conn, profile_id)
    rule(conn, profile_id)
    post(conn, profile_id, amount='100')
    current = tasks.get(conn, profile_id, task['id'])
    assert current['exceptions'][0]['exception'] == 'possible_existing_entry'
    assert not tasks.preview(conn, profile_id, task['id'], 'prepare')['ready']


def test_empty_period_close_and_real_package_export_are_separate_approvals(book):
    from kassiber.core.accounting.package import verify_package
    conn, profile_id, _ = book
    task = evidence_task(conn, profile_id, [])
    closed, _ = approve(conn, profile_id, task['id'], 'close')
    assert closed['result']['action'] == 'close'
    preview = tasks.preview(conn, profile_id, task['id'], 'export_close')
    payload = dict(task_id=task['id'], step='export_close', expected_digest=preview['expected_digest'],
        expected_revision=preview['expected_revision'], idempotency_key='export', confirmed=True)
    with pytest.raises(AppError):
        tasks.execute(conn, profile_id, 'task-apply', payload)
    exported = tasks.execute(conn, profile_id, 'task-apply', {**payload, 'confirm_plaintext': True})
    assert exported['receipt']['result']['artifact_state'] == 'prepared'
    verify_package(exported['result'])
    assert 'snapshot_json' not in exported['receipt']['result']
    retried = tasks.execute(conn, profile_id, 'task-apply', {**payload, 'confirm_plaintext': True})
    assert retried['already_applied']
    assert retried['result']['snapshot_digest'] == exported['result']['snapshot_digest']
    assert retried['task']['state'] == 'completed'


def test_late_batch_failure_rolls_back_every_post_allocation_and_receipt(book, monkeypatch):
    conn, profile_id, _ = book
    task, _ = setup(conn, profile_id, 123)
    rule(conn, profile_id)
    approve(conn, profile_id, task['id'], 'prepare')
    preview = tasks.preview(conn, profile_id, task['id'], 'post')
    actual = tasks.posting_batch.post
    calls = 0
    def fail_second(*args, **kwargs):
        nonlocal calls
        calls += 1
        if calls == 2:
            raise AppError('Synthetic second batch refusal', code='synthetic_refusal')
        return actual(*args, **kwargs)
    monkeypatch.setattr(tasks.posting_batch, 'post', fail_second)
    with pytest.raises(AppError):
        tasks.execute(conn, profile_id, 'task-apply', dict(task_id=task['id'], step='post',
            expected_digest=preview['expected_digest'], expected_revision=preview['expected_revision'], confirmed=True, idempotency_key='post'))
    assert conn.execute("SELECT count(*) FROM gl_entries WHERE status='posted'").fetchone()[0] == 0
    assert conn.execute('SELECT count(*) FROM gl_bank_allocations').fetchone()[0] == 0
    assert conn.execute('SELECT count(*) FROM gl_posting_batches').fetchone()[0] == 0
    assert len(tasks.get(conn, profile_id, task['id'])['receipts']) == 1


def test_restart_resumes_exact_receipts_and_remaining_work(book):
    from kassiber.db import open_db
    conn, profile_id, root = book
    task, _ = setup(conn, profile_id)
    rule(conn, profile_id)
    prepared, request = approve(conn, profile_id, task['id'], 'prepare')
    conn.commit()
    reopened = open_db(root, passphrase='test-token-placeholder')
    try:
        assert tasks.get(reopened, profile_id, task['id'])['next_step'] == 'post'
        assert tasks.execute(reopened, profile_id, 'task-apply', request)['already_applied']
        posted, _ = approve(reopened, profile_id, task['id'], 'post')
        assert posted['result']['posted_ids'] == prepared['result']['draft_ids']
    finally:
        reopened.close()


def test_statement_voiding_invalidates_old_preparation_without_changing_task(book):
    conn, profile_id, _ = book
    task, statement = setup(conn, profile_id)
    rule(conn, profile_id)
    preview = tasks.preview(conn, profile_id, task['id'], 'prepare')
    bank.void_statement(conn, profile_id, statement_id=statement['id'], reason='Wrong bank source', idempotency_key='void')
    with pytest.raises(AppError) as exc:
        tasks.execute(conn, profile_id, 'task-apply', dict(task_id=task['id'], step='prepare',
            expected_digest=preview['expected_digest'], expected_revision=preview['expected_revision'], confirmed=True, idempotency_key='prepare'))
    assert exc.value.code == 'accounting_stale_approval'
    assert tasks.get(conn, profile_id, task['id'])['exceptions'][0]['exception'] == 'statement_voided'


def test_manual_close_is_authoritatively_resumed_by_task(book):
    conn, profile_id, _ = book
    task = evidence_task(conn, profile_id, [])
    closed = ledger.close_period(conn, profile_id, period_id='2025', expected_revision=ledger.require_book(conn, profile_id)['revision'])
    preview = tasks.preview(conn, profile_id, task['id'], 'export_close')
    assert preview['ready']
    assert preview['detail']['id'] == closed['id']


def test_task_storage_refuses_plaintext_and_retained_record_replacement(book, tmp_path):
    from kassiber.db import open_db
    plain = open_db(tmp_path / 'plaintext')
    try:
        with pytest.raises(AppError) as exc:
            tasks.execute(plain, 'unknown', 'task-list', {})
        assert exc.value.code == 'accounting_requires_encryption'
    finally:
        plain.close()
    conn, profile_id, _ = book
    task, _ = setup(conn, profile_id)
    with pytest.raises(Exception, match='accounting_task_retained'):
        conn.execute('UPDATE gl_accounting_tasks SET period_id=? WHERE id=?', ('another', task['id']))
    with pytest.raises(Exception, match='accounting_task_retained'):
        conn.execute('INSERT OR REPLACE INTO gl_accounting_tasks SELECT * FROM gl_accounting_tasks WHERE id=?', (task['id'],))
