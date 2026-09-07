import sqlite3

import pytest

from kassiber.core.accounting import ledger as gl
from kassiber.core.accounting.schema import ensure_schema
from kassiber.errors import AppError


@pytest.fixture
def conn(tmp_path):
    sqlcipher = pytest.importorskip('sqlcipher3.dbapi2')
    db = sqlcipher.connect(str(tmp_path / 'book.db'))
    db.row_factory = sqlcipher.Row
    db.execute("PRAGMA key='test-only-key'")
    db.execute('PRAGMA foreign_keys=ON')
    db.execute('CREATE TABLE profiles(id TEXT PRIMARY KEY)')
    db.executemany('INSERT INTO profiles VALUES(?)', [('p',), ('other',)])
    ensure_schema(db)
    gl.configure_book(db, 'p', currency='EUR', timezone='Europe/Vienna')
    for code, name, kind in [('1000','Bank','asset'), ('2000','Capital','equity'),
                             ('4000','Sales','income'), ('5000','Costs','expense')]:
        gl.create_account(db, 'p', code=code, name=name, kind=kind)
    gl.create_period(db, 'p', period_id='2025', start_date='2025-01-01', end_date='2025-12-31')
    db.commit()
    yield db
    db.close()


def payload(key='sale', amount=10000, *, period='2025', entry_date='2025-02-01', account='4000', entry_kind='normal'):
    return dict(idempotency_key=key, period_id=period, entry_date=entry_date, description='Test booking', entry_kind=entry_kind,
                lines=[dict(account_code='1000', debit_minor=amount), dict(account_code=account, credit_minor=amount)])


def post(conn, data=None):
    draft = gl.create_draft(conn, 'p', data or payload())
    return gl.post_draft(conn, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])


def test_plain_and_unkeyed_cipher_are_rejected(tmp_path):
    db = sqlite3.connect(':memory:')
    with pytest.raises(AppError, match='encrypted'):
        gl.require_encrypted(db)
    db.close()
    sqlcipher = pytest.importorskip('sqlcipher3.dbapi2')
    db = sqlcipher.connect(str(tmp_path / 'unkeyed.db'))
    db.execute('CREATE TABLE plain(x)')
    with pytest.raises(AppError, match='encrypted'):
        gl.require_encrypted(db)
    db.close()


@pytest.mark.parametrize('field', ['reversal_of', 'account_code'])
@pytest.mark.parametrize('value', [[], {}, 42, True])
def test_posting_identifiers_reject_json_non_text(conn, field, value):
    data = payload()
    if field == 'account_code':
        data['lines'][0][field] = value
    else:
        data[field] = value
    with pytest.raises(AppError):
        gl.create_draft(conn, 'p', data)
    assert conn.execute('SELECT COUNT(*) FROM gl_entries').fetchone()[0] == 0


def test_statements_gate_encryption_before_period_lookup():
    db = sqlite3.connect(':memory:')
    with pytest.raises(AppError) as exc:
        gl.financial_statements(db, 'p', period_id='2025')
    assert exc.value.code == 'accounting_requires_encryption'
    db.close()


class CipherStatusConnection:
    """Keep actual codec/schema behavior while emulating a status pragma."""

    def __init__(self, conn, status=None):
        self.conn = conn
        self.status = status

    def execute(self, sql, *args):
        if sql == 'PRAGMA cipher_status':
            return self
        return self.conn.execute(sql, *args)

    def fetchone(self):
        return None if self.status is None else (self.status,)


def test_keyed_older_cipher_without_status_is_accepted(conn):
    gl.require_encrypted(CipherStatusConnection(conn))


def test_failed_status_never_falls_back_to_valid_salt(conn):
    with pytest.raises(AppError) as error:
        gl.require_encrypted(CipherStatusConnection(conn, '0'))
    assert error.value.code == 'accounting_requires_encryption'


def test_older_unkeyed_cipher_and_wrong_key_are_rejected(tmp_path):
    sqlcipher = pytest.importorskip('sqlcipher3.dbapi2')
    for driver in (sqlite3, sqlcipher):
        db = driver.connect(':memory:')
        db.execute('CREATE TABLE plain(x)')
        with pytest.raises(AppError) as error:
            gl.require_encrypted(CipherStatusConnection(db))
        assert error.value.code == 'accounting_requires_encryption'
        db.close()

    path = str(tmp_path / 'key-validation.db')
    db = sqlcipher.connect(path)
    db.execute("PRAGMA key='test-only-key'")
    db.execute('CREATE TABLE protected(x)')
    db.commit()
    db.close()
    db = sqlcipher.connect(path)
    db.execute("PRAGMA key='different-test-key'")
    # A configured codec returns salt even with the wrong key. The actual
    # encrypted-schema read must reject it on both current and older versions.
    assert db.execute('PRAGMA cipher_salt').fetchone()
    with pytest.raises(AppError) as error:
        gl.require_encrypted(CipherStatusConnection(db))
    assert error.value.code == 'accounting_requires_encryption'
    db.close()


def test_post_atomic_exact_and_retry(conn):
    draft = gl.create_draft(conn, 'p', payload(amount=2**53 + 5))
    assert gl.trial_balance(conn, 'p')['debit_minor'] == 0
    entry = gl.post_draft(conn, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])
    assert entry['status'] == 'posted'
    assert gl.trial_balance(conn, 'p')['debit_minor'] == 2**53 + 5
    assert gl.create_draft(conn, 'p', payload(amount=2**53 + 5))['id'] == entry['id']
    assert gl.post_draft(conn, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])['id'] == entry['id']
    with pytest.raises(AppError, match='different payload'):
        gl.create_draft(conn, 'p', payload(amount=1))


@pytest.mark.parametrize('amount', [True, -1, 0, 1.1, '100', 2**63])
def test_bad_amounts_leave_no_partial_draft(conn, amount):
    with pytest.raises(AppError):
        gl.create_draft(conn, 'p', payload(amount=amount))
    assert conn.execute('SELECT count(*) FROM gl_entries').fetchone()[0] == 0


def test_aggregate_overflow_and_unbalanced(conn):
    data = payload(amount=2**62)
    data['lines'] *= 2
    with pytest.raises(AppError, match='64-bit'):
        gl.create_draft(conn, 'p', data)
    data = payload()
    data['lines'][1]['credit_minor'] = 1
    with pytest.raises(AppError, match='equal'):
        gl.create_draft(conn, 'p', data)


def test_mutation_protection_raw_sql(conn):
    entry = post(conn)
    statements = [
        ("UPDATE gl_entries SET description='changed' WHERE id=?", (entry['id'],)),
        ('DELETE FROM gl_entries WHERE id=?', (entry['id'],)),
        ('UPDATE gl_lines SET debit_minor=123 WHERE id=?', (entry['lines'][0]['id'],)),
        ('DELETE FROM gl_lines WHERE entry_id=?', (entry['id'],)),
        ("INSERT INTO gl_lines VALUES('rogue',?,99,'p','1000','Bank','asset',1,0)", (entry['id'],)),
        ("UPDATE gl_accounts SET name='Rewritten' WHERE code='1000'", ()),
        ("DELETE FROM profiles WHERE id='p'", ()),
    ]
    for sql, params in statements:
        with pytest.raises(Exception):
            conn.execute(sql, params)
    assert gl.trial_balance(conn, 'p')['debit_minor'] == 10000


def test_stale_approval_and_cross_book(conn):
    draft = gl.create_draft(conn, 'p', payload())
    with pytest.raises(AppError, match='approval'):
        gl.post_draft(conn, 'p', draft_id=draft['id'], expected_digest='wrong')
    gl.configure_book(conn, 'other', currency='EUR', timezone='UTC')
    with pytest.raises(AppError, match='not found'):
        gl.post_draft(conn, 'other', draft_id=draft['id'], expected_digest=draft['payload_digest'])
    conn.execute("UPDATE gl_entries SET description='Tampered' WHERE id=?", (draft['id'],))
    with pytest.raises(AppError, match='approval'):
        gl.post_draft(conn, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])


def test_reversal_and_historical_profit(conn):
    entry = post(conn)
    reversed_entry = gl.reverse_entry(conn, 'p', entry_id=entry['id'], entry_date='2025-03-01',
                                      period_id='2025', idempotency_key='reverse', reason='Correction')
    assert reversed_entry['reversal_of'] == entry['id']
    assert gl.financial_statements(conn, 'p', period_id='2025')['profit_minor'] == 0
    assert len(gl.journal(conn, 'p')) == 2
    assert gl.trial_balance(conn, 'p', as_of='2025-02-28')['rows'][0]['balance_minor'] == 10000


def reversal_payload(entry, key='reverse'):
    return dict(idempotency_key=key, period_id='2025', entry_date='2025-03-01',
                description='Correction', entry_kind='reversal', reversal_of=entry['id'],
                lines=[dict(account_code=line['account_code'], debit_minor=line['credit_minor'],
                            credit_minor=line['debit_minor']) for line in entry['lines']])


@pytest.mark.parametrize('posted', [False, True])
def test_duplicate_reversal_is_typed_and_retry_is_idempotent(conn, posted):
    entry = post(conn)
    data = reversal_payload(entry)
    reversal = gl.create_draft(conn, 'p', data)
    if posted:
        gl.post_draft(conn, 'p', draft_id=reversal['id'], expected_digest=reversal['payload_digest'])
    revision = gl.require_book(conn, 'p')['revision']
    assert gl.create_draft(conn, 'p', data)['id'] == reversal['id']
    with pytest.raises(AppError) as conflict:
        gl.create_draft(conn, 'p', {**data, 'description': 'Different correction'})
    assert conflict.value.code == 'accounting_idempotency_conflict'
    with pytest.raises(AppError) as duplicate:
        gl.create_draft(conn, 'p', reversal_payload(entry, 'another-reversal'))
    assert duplicate.value.code == 'accounting_already_reversed'
    with pytest.raises(AppError) as duplicate:
        gl.reverse_entry(conn, 'p', entry_id=entry['id'], entry_date='2025-03-01',
                         period_id='2025', idempotency_key='another-reversal', reason='Correction')
    assert duplicate.value.code == 'accounting_already_reversed'
    assert gl.require_book(conn, 'p')['revision'] == revision
    assert conn.execute('SELECT count(*) FROM gl_entries').fetchone()[0] == 2
    with pytest.raises(pytest.importorskip('sqlcipher3.dbapi2').IntegrityError, match='UNIQUE'):
        conn.execute("""INSERT INTO gl_entries(
            id,profile_id,period_id,entry_date,description,entry_kind,status,idempotency_key,
            payload_digest,source_ref,reversal_of,created_at)
            SELECT 'raw-duplicate',profile_id,period_id,entry_date,description,entry_kind,
            'draft','raw-duplicate',payload_digest,source_ref,reversal_of,created_at
            FROM gl_entries WHERE id=?""", (reversal['id'],))
    assert gl.reverse_entry(conn, 'p', entry_id=entry['id'], entry_date='2025-03-01',
                            period_id='2025', idempotency_key='reverse', reason='Correction')['id'] == reversal['id']
    posted_revision = gl.require_book(conn, 'p')['revision']
    assert gl.reverse_entry(conn, 'p', entry_id=entry['id'], entry_date='2025-03-01',
                            period_id='2025', idempotency_key='reverse', reason='Correction')['id'] == reversal['id']
    assert gl.require_book(conn, 'p')['revision'] == posted_revision


@pytest.mark.parametrize('replacement_key', ['reverse', 'replacement'])
def test_discarded_reversal_can_be_replaced(conn, replacement_key):
    entry = post(conn)
    draft = gl.create_draft(conn, 'p', reversal_payload(entry))
    gl.discard_draft(conn, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])
    replacement = gl.reverse_entry(conn, 'p', entry_id=entry['id'], entry_date='2025-03-01',
                                   period_id='2025', idempotency_key=replacement_key, reason='Correction')
    assert replacement['id'] != draft['id']
    assert replacement['status'] == 'posted'
    assert gl.financial_statements(conn, 'p', period_id='2025')['profit_minor'] == 0


@pytest.mark.parametrize('closing', [False, True])
def test_reversals_cannot_be_reversed_and_profit_remains_correct(conn, closing):
    original = post(conn)
    if closing:
        original = post(conn, dict(idempotency_key='appropriation', period_id='2025',
                                  entry_date='2025-12-31', description='Result appropriation', entry_kind='closing',
                                  lines=[dict(account_code='4000', debit_minor=10000),
                                         dict(account_code='2000', credit_minor=10000)]))
    reversal = gl.reverse_entry(conn, 'p', entry_id=original['id'], entry_date='2025-12-31',
                                period_id='2025', idempotency_key='reverse', reason='Correction')
    expected_profit = 10000 if closing else 0
    revision = gl.require_book(conn, 'p')['revision']
    with pytest.raises(AppError) as error:
        gl.reverse_entry(conn, 'p', entry_id=reversal['id'], entry_date='2025-12-31',
                         period_id='2025', idempotency_key='reverse-again', reason='Undo correction')
    assert error.value.code == 'accounting_reversal_not_allowed'
    with pytest.raises(AppError) as error:
        gl.create_draft(conn, 'p', {**reversal_payload(reversal, 'draft-again'), 'entry_date': '2025-12-31'})
    assert error.value.code == 'accounting_reversal_not_allowed'
    assert gl.require_book(conn, 'p')['revision'] == revision
    assert gl.financial_statements(conn, 'p', period_id='2025')['profit_minor'] == expected_profit
    result = gl.close_period(conn, 'p', period_id='2025', expected_revision=revision)
    assert result['snapshot']['statements']['profit_minor'] == expected_profit


@pytest.mark.parametrize('dependent_review', [False, True])
def test_earlier_period_requires_explicit_reopening_of_later_periods(conn, dependent_review):
    post(conn)
    saved = gl.close_period(conn, 'p', period_id='2025', expected_revision=gl.require_book(conn, 'p')['revision'])
    if dependent_review:
        gl.create_period(conn, 'p', period_id='2026', start_date='2026-01-01', end_date='2026-12-31')
        gl.close_period(conn, 'p', period_id='2026', expected_revision=gl.require_book(conn, 'p')['revision'])
        gl.reopen_period(conn, 'p', period_id='2025', reason='Correction', expected_revision=gl.require_book(conn, 'p')['revision'])
    revision = gl.require_book(conn, 'p')['revision']
    with pytest.raises(AppError) as error:
        gl.create_period(conn, 'p', period_id='2024', start_date='2024-01-01', end_date='2024-12-31')
    assert error.value.code == 'accounting_later_period_closed'
    assert conn.execute("SELECT 1 FROM gl_periods WHERE id='2024'").fetchone() is None
    assert gl.require_book(conn, 'p')['revision'] == revision
    # Retrying an existing interval does not change history or require reopening.
    gl.create_period(conn, 'p', period_id='2025', start_date='2025-01-01', end_date='2025-12-31')
    assert gl.require_book(conn, 'p')['revision'] == revision
    target = '2026' if dependent_review else '2025'
    gl.reopen_period(conn, 'p', period_id=target, reason='Add prior fiscal period',
                     expected_revision=gl.require_book(conn, 'p')['revision'])
    gl.create_period(conn, 'p', period_id='2024', start_date='2024-01-01', end_date='2024-12-31')
    post(conn, payload(key='prior', period='2024', entry_date='2024-06-01'))
    assert gl.financial_statements(conn, 'p', period_id='2025')['profit_minor'] == 10000
    assert conn.execute('SELECT snapshot_digest FROM gl_period_events WHERE id=?', (saved['id'],)).fetchone()[0] == saved['snapshot_digest']
    with pytest.raises(AppError, match='Earlier fiscal periods'):
        gl.close_period(conn, 'p', period_id='2025', expected_revision=gl.require_book(conn, 'p')['revision'])


def test_existing_nested_reversal_draft_is_rechecked_before_posting(conn, monkeypatch):
    original = post(conn)
    reversal = gl.reverse_entry(conn, 'p', entry_id=original['id'], entry_date='2025-03-01',
                                period_id='2025', idempotency_key='reverse', reason='Correction')
    # Emulate a draft created by the previous implementation; approval must
    # still cross the current guard instead of trusting its earlier creation.
    with monkeypatch.context() as patch:
        patch.setattr(gl, '_require_reversible', lambda *_args: None)
        draft = gl.create_draft(conn, 'p', reversal_payload(reversal, 'legacy-nested'))
    revision = gl.require_book(conn, 'p')['revision']
    with pytest.raises(AppError) as error:
        gl.post_draft(conn, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])
    assert error.value.code == 'accounting_reversal_not_allowed'
    assert gl.require_book(conn, 'p')['revision'] == revision
    assert conn.execute('SELECT status FROM gl_entries WHERE id=?', (draft['id'],)).fetchone()[0] == 'draft'
    gl.discard_draft(conn, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])


def test_period_close_reopen_second_year_continuity(conn):
    post(conn, payload(key='opening', account='2000', entry_date='2025-01-01', entry_kind='opening'))
    post(conn)
    first = gl.close_period(conn, 'p', period_id='2025', expected_revision=gl.require_book(conn, 'p')['revision'])
    assert first['snapshot']['statements']['profit_minor'] == 10000
    assert len(first['snapshot']['journal']) == 2
    with pytest.raises(AppError, match='not open'):
        gl.create_draft(conn, 'p', payload(key='late'))
    gl.create_period(conn, 'p', period_id='2026', start_date='2026-01-01', end_date='2026-12-31')
    post(conn, payload(key='next', amount=5000, period='2026', entry_date='2026-02-01'))
    result = gl.financial_statements(conn, 'p', period_id='2026')
    assert result['profit_minor'] == 5000
    assert result['balance_sheet'][0]['balance_minor'] == 25000
    gl.close_period(conn, 'p', period_id='2026', expected_revision=gl.require_book(conn, 'p')['revision'])
    gl.reopen_period(conn, 'p', period_id='2025', reason='Prior-year correction', expected_revision=gl.require_book(conn, 'p')['revision'])
    assert conn.execute("SELECT state FROM gl_periods WHERE id='2026'").fetchone()[0] == 'review'
    post(conn, payload(key='correction', amount=3000))
    again = gl.close_period(conn, 'p', period_id='2025', expected_revision=gl.require_book(conn, 'p')['revision'])
    assert again['snapshot_digest'] != first['snapshot_digest']
    assert conn.execute('SELECT COUNT(*) FROM gl_period_events').fetchone()[0] == 4


def test_caller_rollback_and_schema_do_not_commit(conn):
    post(conn)
    ensure_schema(conn)
    conn.rollback()
    assert not gl.journal(conn, 'p')
    assert gl.book_status(conn, 'p')['configured']


def test_closed_period_raw_transition_and_drafts_block_close(conn):
    draft = gl.create_draft(conn, 'p', payload())
    with pytest.raises(AppError, match='drafts'):
        gl.close_period(conn, 'p', period_id='2025', expected_revision=gl.require_book(conn, 'p')['revision'])
    conn.execute("UPDATE gl_periods SET state='closed'")
    with pytest.raises(Exception, match='not open'):
        conn.execute("UPDATE gl_entries SET status='posted' WHERE id=?", (draft['id'],))


def test_unknown_quantity_fields_and_duplicate_opening(conn):
    data = payload()
    data['quantity_msat'] = 1
    with pytest.raises(AppError, match='Unsupported posting fields'):
        gl.create_draft(conn, 'p', data)
    post(conn)
    with pytest.raises(AppError, match='precede'):
        gl.create_draft(conn, 'p', payload(key='opening', entry_kind='opening', account='2000', entry_date='2025-01-01'))


def test_preopening_entry_and_preexisting_draft_fail_closed(conn):
    gl.create_period(conn, 'p', period_id='2024', start_date='2024-01-01', end_date='2024-12-31')
    old = gl.create_draft(conn, 'p', payload(key='old', period='2024', entry_date='2024-01-01'))
    post(conn, payload(key='opening', entry_kind='opening', account='2000', entry_date='2025-01-01'))
    with pytest.raises(AppError, match='predates'):
        gl.post_draft(conn, 'p', draft_id=old['id'], expected_digest=old['payload_digest'])
    with pytest.raises(AppError, match='predates'):
        gl.create_draft(conn, 'p', payload(key='old-again', period='2024', entry_date='2024-06-01'))


def test_replace_cannot_bypass_retention(conn):
    entry = post(conn)
    conn.execute('PRAGMA recursive_triggers=OFF')
    for sql in (
        "INSERT OR REPLACE INTO gl_accounts VALUES('p','1000','Rewritten','expense')",
        "INSERT OR REPLACE INTO gl_periods VALUES('p','2025','2025-01-01','2025-12-31','open',0)",
        "INSERT OR REPLACE INTO gl_lines SELECT id,entry_id,position,profile_id,account_code,account_name,account_kind,debit_minor,credit_minor FROM gl_lines LIMIT 1",
        "INSERT OR REPLACE INTO gl_entries SELECT id,profile_id,period_id,entry_date,description,entry_kind,'draft',idempotency_key,payload_digest,source_ref,reversal_of,created_at,posted_at FROM gl_entries LIMIT 1",
    ):
        with pytest.raises(Exception, match='replacement forbidden|posted lines immutable'):
            conn.execute(sql)
    assert gl.journal(conn, 'p')[0]['id'] == entry['id']


def test_result_appropriation_preserves_period_profit(conn):
    post(conn)
    close = dict(idempotency_key='appropriation', period_id='2025', entry_date='2025-12-31',
                 description='Transfer result to retained earnings', entry_kind='closing',
                 lines=[dict(account_code='4000', debit_minor=10000), dict(account_code='2000', credit_minor=10000)])
    post(conn, close)
    statements = gl.financial_statements(conn, 'p', period_id='2025')
    assert statements['profit_minor'] == 10000
    assert statements['unappropriated_result_minor'] == 0
    assert statements['balanced']
    bad = payload(key='hide-revenue', entry_kind='closing', entry_date='2025-12-31')
    draft = gl.create_draft(conn, 'p', bad)
    with pytest.raises(AppError, match='only clear'):
        gl.post_draft(conn, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])
    gl.discard_draft(conn, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])
    assert not gl.journal(conn, 'p', status='draft')


def test_sqlite_writes_remain_uncommitted_until_caller_commits(conn):
    sqlcipher = pytest.importorskip('sqlcipher3.dbapi2')
    filename = conn.execute('PRAGMA database_list').fetchone()[2]
    other = sqlcipher.connect(filename, timeout=0.01)
    other.execute("PRAGMA key='test-only-key'")
    other.row_factory = sqlcipher.Row
    post(conn)
    assert other.execute('SELECT COUNT(*) FROM gl_entries').fetchone()[0] == 0
    with pytest.raises(Exception, match='locked'):
        gl.create_draft(other, 'p', payload(key='concurrent'))
    conn.commit()
    assert other.execute('SELECT COUNT(*) FROM gl_entries').fetchone()[0] == 1
    other.close()
