import pytest

from kassiber.core.accounting import ledger, posting_batch as batch
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401


def drafts(conn, profile):
    return [ledger.create_draft(conn, profile, dict(idempotency_key=f'batch-{index}',
        period_id='2025', entry_date='2025-03-01', description=f'Entry {index}',
        lines=[dict(account_code='bank', debit_minor=100 + index), dict(account_code='sales',credit_minor=100 + index)]))['id'] for index in range(2)]


def test_batch_preview_is_no_write_and_post_is_idempotent(book):
    conn, profile, _ = book
    ids = drafts(conn, profile)
    revision = ledger.require_book(conn, profile)['revision']
    reviewed = batch.preview(conn, profile, draft_ids=ids)
    assert ledger.require_book(conn, profile)['revision'] == revision
    assert all(ledger._entry(conn, profile, key)['status'] == 'draft' for key in ids)
    args = dict(draft_ids=ids, expected_digest=reviewed['expected_digest'], expected_revision=reviewed['expected_revision'],
        idempotency_key='approved-batch',reason='Reviewed every entry')
    posted = batch.post(conn, profile, **args)
    assert posted['posted_ids'] == ids
    assert batch.post(conn, profile, **args) == posted
    assert all(ledger._entry(conn, profile, key)['status'] == 'posted' for key in ids)
    with pytest.raises(AppError):
        batch.post(conn, profile, **dict(args, draft_ids=ids[::-1]))
    with pytest.raises(Exception, match='accounting_posting_batch_retained'):
        conn.execute('DELETE FROM gl_posting_batches')


def test_any_late_failure_rolls_back_whole_batch(book, monkeypatch):
    conn, profile, _ = book
    ids = drafts(conn, profile)
    preview = batch.preview(conn, profile, draft_ids=ids)
    real_post = ledger.post_draft
    calls = []
    def fail_second(db, scope, **args):
        calls.append(args['draft_id'])
        # The first pass is the rollback-only revalidation. Fail after the
        # first real posting, proving rollback of the committing pass too.
        if len(calls) == 4:
            raise AppError('Synthetic late guard', code='accounting_close_blocked')
        return real_post(db, scope, **args)
    monkeypatch.setattr(ledger, 'post_draft', fail_second)
    with pytest.raises(AppError):
        batch.post(conn, profile, draft_ids=ids, expected_digest=preview['expected_digest'],
            expected_revision=preview['expected_revision'], idempotency_key='late-fail', reason='Reviewed')
    assert all(ledger._entry(conn, profile, key)['status'] == 'draft' for key in ids)
    assert ledger.require_book(conn, profile)['revision'] == preview['expected_revision']
    assert calls == ids + ids
    assert conn.execute('SELECT COUNT(*) FROM gl_posting_batches').fetchone()[0] == 0


def test_batch_rejects_changed_input_scope_and_revision(book):
    conn, profile, _ = book
    ids = drafts(conn, profile)
    preview = batch.preview(conn, profile, draft_ids=ids)
    for values in ([], [ids[0], ids[0]], [False], ['missing'], ids * 30):
        with pytest.raises(AppError):
            batch.preview(conn, profile, draft_ids=values)
    with pytest.raises(AppError):
        batch.post(conn, profile, draft_ids=ids[::-1], expected_digest=preview['expected_digest'],
            expected_revision=preview['expected_revision'], idempotency_key='wrong-order', reason='Reviewed')
    ledger.create_account(conn, profile, code='changed', name='Changed', kind='expense')
    with pytest.raises(AppError):
        batch.post(conn, profile, draft_ids=ids, expected_digest=preview['expected_digest'],
            expected_revision=preview['expected_revision'], idempotency_key='stale', reason='Reviewed')
