import pytest

from kassiber.core.accounting import bank, ledger, sources
from kassiber.errors import AppError
from tests.test_accounting_evidence import accounting_db  # noqa: F401
from tests.test_accounting_integration import book  # noqa: F401


@pytest.fixture
def sourced(accounting_db):
    conn = accounting_db
    sources.ensure_schema(conn)
    for column in ('document_type', 'external_ref', 'issued_at', 'due_at', 'fiat_currency',
                   'fiat_value_exact', 'review_state', 'updated_at'):
        conn.execute(f'ALTER TABLE external_documents ADD COLUMN {column} TEXT')
    conn.execute("""INSERT INTO external_documents(id,profile_id,document_type,external_ref,issued_at,
        fiat_currency,fiat_value_exact,review_state) VALUES('invoice','p','invoice','INV-1',
        '2026-03-01','EUR','2.00','reviewed')""")
    bank.import_statement(conn, 'p', account_code='bank', statement_id='bank-1',
                          start_date='2026-03-01', end_date='2026-03-31',
                          csv_text='row_id,date,amount_minor,description\na,2026-03-01,100,First\nb,2026-03-02,100,Second\n')
    conn.commit()
    return conn


def bind(conn, snapshot, claims=None, **overrides):
    record = next(r for r in snapshot['snapshot']['sources'] if r['kind'] == 'document')
    args = dict(snapshot_id=snapshot['id'], expected_digest=snapshot['input_digest'], economic_id='invoice:invoice',
                role='recognition', claims=claims or [dict(source_id=record['source_id'], start_atomic=0, end_atomic=100)],
                reason='Reviewed invoice capture', idempotency_key='bind')
    return sources.bind_sources(conn, 'p', **{**args, **overrides})


def test_capture_is_exact_scoped_immutable_and_caller_owned(sourced):
    snapshot = sources.capture_sources(sourced, 'p')
    assert len(snapshot['snapshot']['sources']) == 3
    revision = ledger.require_book(sourced, 'p')['revision']
    assert sources.capture_sources(sourced, 'p')['id'] == snapshot['id']
    assert ledger.require_book(sourced, 'p')['revision'] == revision
    with pytest.raises(AppError):
        sources.get_snapshot(sourced, 'other', snapshot['id'])
    with pytest.raises(Exception, match='accounting_source_retained'):
        sourced.execute('UPDATE gl_source_snapshots SET payload_json=?', ('{}',))
    with pytest.raises(Exception, match='accounting_source_retained'):
        sourced.execute('INSERT OR REPLACE INTO gl_source_snapshots SELECT * FROM gl_source_snapshots')
    sourced.rollback()
    assert sourced.execute('SELECT COUNT(*) FROM gl_source_snapshots').fetchone()[0] == 0


def test_partial_claims_remainders_duplicate_and_void_replacement(sourced):
    snapshot = sources.capture_sources(sourced, 'p')
    binding = bind(sourced, snapshot)
    assert bind(sourced, snapshot)['id'] == binding['id']
    coverage = sources.source_coverage(sourced, 'p')
    covered = next(r for r in coverage['rows'] if r['assigned_atomic'])
    assert covered['amount_atomic'] == 200 and covered['remaining_atomic'] == 100
    with pytest.raises(AppError) as error:
        bind(sourced, snapshot, idempotency_key='double', economic_id='other-invoice')
    assert error.value.code == 'accounting_source_claim_overlap'
    with pytest.raises(AppError) as error:
        bind(sourced, snapshot, reason='changed')
    assert error.value.code == 'accounting_idempotency_conflict'
    result = sources.void_binding(sourced, 'p', binding_id=binding['id'], reason='Wrong allocation', idempotency_key='void')
    assert result['voided']
    assert sources.void_binding(sourced, 'p', binding_id=binding['id'], reason='Wrong allocation', idempotency_key='void')['voided']
    replacement = bind(sourced, snapshot, idempotency_key='replacement')
    assert replacement['id'] != binding['id']


def test_source_revision_cannot_evade_an_existing_claim(sourced):
    snapshot = sources.capture_sources(sourced, 'p')
    binding = bind(sourced, snapshot)
    sourced.execute("UPDATE external_documents SET fiat_value_exact='3.00' WHERE id='invoice'")
    with pytest.raises(AppError) as error:
        sources.require_current(sourced, 'p', snapshot['id'])
    assert error.value.code == 'accounting_source_stale'
    new = sources.capture_sources(sourced, 'p')
    old_source = next(r for r in snapshot['snapshot']['sources'] if r['kind'] == 'document')
    new_source = next(r for r in new['snapshot']['sources'] if r['kind'] == 'document')
    assert old_source['source_id'] == new_source['source_id']
    assert old_source['source_digest'] != new_source['source_digest']
    assert sources.source_coverage(sourced, 'p')['stale_bindings'][0]['binding_id'] == binding['id']
    with pytest.raises(AppError) as error:
        bind(sourced, new, idempotency_key='repeat-after-source-change')
    assert error.value.code == 'accounting_source_claim_overlap'


@pytest.mark.parametrize('start,end', [(True, 2), (0, 0), (0, 201), (-1, 1), (0, 2**63)])
def test_invalid_claim_leaves_no_binding(sourced, start, end):
    snapshot = sources.capture_sources(sourced, 'p')
    doc = next(r for r in snapshot['snapshot']['sources'] if r['kind'] == 'document')
    with pytest.raises(AppError):
        bind(sourced, snapshot, [dict(source_id=doc['source_id'], start_atomic=start, end_atomic=end)])
    assert sourced.execute('SELECT COUNT(*) FROM gl_source_bindings').fetchone()[0] == 0


def test_closed_period_and_raw_sql_claim_bypass_rejected(sourced):
    snapshot = sources.capture_sources(sourced, 'p')
    binding = bind(sourced, snapshot)
    with pytest.raises(Exception, match='accounting_source_claim_invalid'):
        sourced.execute("""INSERT INTO gl_source_claims VALUES('raw','p',?,'unknown','unknown','minor',0,1)""", (binding['id'],))
    sourced.execute("UPDATE gl_periods SET state='closed' WHERE profile_id='p'")
    with pytest.raises(AppError) as error:
        sources.void_binding(sourced, 'p', binding_id=binding['id'], reason='Changed', idempotency_key='void')
    assert error.value.code == 'accounting_period_closed'
    record = next(r for r in snapshot['snapshot']['sources'] if r['kind'] == 'bank')
    with pytest.raises(AppError) as error:
        bind(sourced, snapshot, [dict(source_id=record['source_id'], start_atomic=0, end_atomic=100)], idempotency_key='late')
    assert error.value.code == 'accounting_period_closed'


def test_document_requires_exact_same_currency_value(sourced):
    sourced.execute("UPDATE external_documents SET fiat_value_exact='0.001' WHERE id='invoice'")
    result = sources.preview_sources(sourced, 'p')
    assert not any(r['kind'] == 'document' for r in result['sources'])
    assert result['blockers'] == [dict(code='accounting_document_needs_valuation', document_id='invoice')]


def test_real_custody_capture_keeps_tiny_quantities_without_running_rp2(book, monkeypatch):
    from kassiber.core.wallets import create_wallet
    from kassiber.core.engines import GenericRP2TaxEngine

    conn, scope, _root = book
    sources.ensure_schema(conn)
    workspace_id = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (scope,)).fetchone()[0]
    wallet = create_wallet(conn, workspace_id, scope, 'Capture source', 'custom')
    conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,
        occurred_at,direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at)
        VALUES('tiny',?,?,?,'tiny-source','tiny-fingerprint','2025-01-01T23:30:00Z','inbound',
        'BTC',1,0,'EUR','1','{}','2025-01-01T23:30:00Z')''', (workspace_id, scope, wallet['id']))
    def no_engine(*args, **kwargs):
        raise AssertionError('Source capture must not invoke the cost-basis engine')
    monkeypatch.setattr(GenericRP2TaxEngine, 'build_ledger_state', no_engine)
    snapshot = sources.capture_sources(conn, scope)
    source = snapshot['snapshot']['sources'][0]
    assert source['unit'] == 'msat' and source['amount_atomic'] == 1
    assert source['occurred_on'] == '2025-01-02'
    assert source['facts']['anchor_transaction_id'] == 'tiny'
    assert source['facts']['prices']['fiat_rate_exact'] == '1'
    assert not conn.execute('SELECT 1 FROM gl_entries WHERE profile_id=?', (scope,)).fetchone()
