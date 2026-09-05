"""Actual captured Bitcoin sources, reviewed local assignments and task posting."""
import pytest

from kassiber.core.accounting import artifacts, ledger, projection, sources, tasks
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_projection import prepared
from tests.test_accounting_tasks import approve


def task_for(conn, profile, args, *, events=None, key='bitcoin-task', **extra):
    selected = events if events is not None else [{k: args[k] for k in ('event_id', 'binding_id', 'category')}]
    payload = dict(period_id='2025', statement_ids=[], idempotency_key=key,
        projection=dict(artifact_id=args['artifact_id'], policy_id=args['policy_id'], events=selected))
    return tasks.execute(conn, profile, 'task-create', {**payload, **extra})


@pytest.mark.parametrize('quantity', [1, 100_000_000_000])
def test_raw_source_task_prepare_and_post_no_existing_draft_needed(book, quantity):
    conn, profile, args = prepared(book, quantity=quantity)
    task = task_for(conn, profile, args)
    assert task['source_count'] == 1 and task['coverage'][0]['source_kind'] == 'bitcoin'
    assert task['spec']['draft_ids'] == []
    revision = ledger.require_book(conn, profile)['revision']
    first = tasks.preview(conn, profile, task['id'], 'prepare')
    assert first['expected_digest'] == tasks.preview(conn, profile, task['id'], 'prepare')['expected_digest']
    assert not conn.execute('SELECT 1 FROM gl_entries').fetchone()
    assert not conn.execute('SELECT 1 FROM gl_projection_proposals').fetchone()
    assert ledger.require_book(conn, profile)['revision'] == revision
    result, retry_payload = approve(conn, profile, task['id'], 'prepare')
    assert len(result['result']['projection_ids']) == 1
    if quantity == 1:
        assert result['result']['draft_ids'] == []
    saved = projection.get_proposal(conn, profile, result['result']['projection_ids'][0])
    assert saved['proposal']['quantitative_posting']['quantity_msat'] == quantity
    assert tasks.execute(conn, profile, 'task-apply', retry_payload)['receipt']['id'] == result['receipt']['id']
    assert tasks.get(conn, profile, task['id'])['next_step'] == 'post'
    published, retry_payload = approve(conn, profile, task['id'], 'post')
    assert published['task']['coverage'][0]['status'] == 'posted'
    assert tasks.execute(conn, profile, 'task-apply', retry_payload)['already_applied']
    assert conn.execute('SELECT count(*) FROM gl_projection_publications').fetchone()[0] == 1
    assert not projection.validate_close(conn, profile, '2025-01-01', '2025-12-31')['blockers']


def assignment(conn, profile, task, args, *, key='answer'):
    payload = dict(task_id=task['id'], event_id=args['event_id'], binding_id=args['binding_id'],
        category=args['category'], reason='Locally reviewed acquisition classification')
    reviewed = tasks.execute(conn, profile, 'task-projection-assign-preview', payload)
    return {**payload, 'kind': 'projection', 'confirmed': True, 'idempotency_key': key,
        'expected_digest': reviewed['expected_digest'], 'expected_revision': reviewed['expected_revision']}


def test_missing_category_answer_resolves_same_task_but_cannot_rewrite_prepared_source(book):
    conn, profile, args = prepared(book)
    task = task_for(conn, profile, args, events=[{'event_id': args['event_id']}])
    assert task['coverage'][0]['exception'] == 'accounting_task_projection_category_required'
    original = conn.execute('SELECT spec_json FROM gl_accounting_tasks WHERE id=?', (task['id'],)).fetchone()[0]
    payload = assignment(conn, profile, task, args)
    other_answer = {**payload, 'idempotency_key': 'concurrent-answer'}
    assigned = tasks.execute(conn, profile, 'task-source-assign', payload)
    assert assigned['task']['id'] == task['id'] and assigned['task']['coverage'][0]['status'] == 'ready'
    assert conn.execute('SELECT spec_json FROM gl_accounting_tasks WHERE id=?', (task['id'],)).fetchone()[0] == original
    assert tasks.execute(conn, profile, 'task-source-assign', payload)['already_applied']
    with pytest.raises(AppError) as error:
        tasks.execute(conn, profile, 'task-source-assign', other_answer)
    assert error.value.code == 'accounting_stale_approval'
    approve(conn, profile, task['id'], 'prepare')
    with pytest.raises(AppError) as error:
        assignment(conn, profile, task, args, key='rewrite-prepared')
    assert error.value.code == 'accounting_task_projection_already_prepared'
    approve(conn, profile, task['id'], 'post')
    with pytest.raises(AppError) as error:
        assignment(conn, profile, task, args, key='rewrite-posted')
    assert error.value.code == 'accounting_task_projection_already_prepared'
    assert tasks.execute(conn, profile, 'task-source-assign', payload)['already_applied']


@pytest.mark.parametrize('change', ['source', 'policy', 'binding'])
def test_changed_source_authority_invalidates_old_prepare(book, change):
    conn, profile, args = prepared(book)
    task = task_for(conn, profile, args)
    preview = tasks.preview(conn, profile, task['id'], 'prepare')
    if change == 'source':
        conn.execute("UPDATE transactions SET fiat_rate_exact='101' WHERE id='acquisition'")
    elif change == 'policy':
        conn.execute("UPDATE profiles SET gains_algorithm='LIFO' WHERE id=?", (profile,))
    else:
        sources.void_binding(conn, profile, binding_id=args['binding_id'], reason='Withdraw interpretation', idempotency_key='void')
    with pytest.raises(AppError) as error:
        tasks.execute(conn, profile, 'task-apply', dict(task_id=task['id'], step='prepare', expected_revision=preview['expected_revision'],
            expected_digest=preview['expected_digest'], confirmed=True, idempotency_key='prepare'))
    assert error.value.code == 'accounting_stale_approval'
    assert not conn.execute('SELECT 1 FROM gl_projection_proposals').fetchone()


def test_projection_task_selection_is_exact_and_period_bound(book):
    conn, profile, args = prepared(book)
    for events in ([{'event_id': 'unselected'}], [{'event_id': args['event_id']}] * 2):
        with pytest.raises(AppError):
            task_for(conn, profile, args, events=events)
    ledger.create_period(conn, profile, period_id='2026', start_date='2026-01-01', end_date='2026-12-31')
    with pytest.raises(AppError):
        tasks.execute(conn, profile, 'task-create', dict(period_id='2026', statement_ids=[], idempotency_key='wrong-period',
            projection=dict(artifact_id=args['artifact_id'], policy_id=args['policy_id'], events=[{'event_id': args['event_id']}])) )
    task = task_for(conn, profile, args)
    assert task['period_id'] == '2025'
    assert task['spec']['projection']['events'] == [{key: args[key] for key in ('event_id', 'binding_id', 'category')}]


def test_unlike_binding_role_remains_exception_not_automatic_revenue(book):
    conn, profile, args = prepared(book)
    binding = sources.get_binding(conn, profile, args['binding_id'])
    snapshot = sources.get_snapshot(conn, profile, binding['snapshot_id'])
    sources.void_binding(conn, profile, binding_id=binding['id'], reason='Settlement only', idempotency_key='void')
    settlement = sources.bind_sources(conn, profile, snapshot_id=snapshot['id'], expected_digest=snapshot['input_digest'],
        economic_id='payment-only', role='settlement', reason='Reviewed payment, not recognition', idempotency_key='settlement',
        claims=[{key: row[key] for key in ('source_id', 'start_atomic', 'end_atomic')} for row in binding['claims']])
    task = task_for(conn, profile, {**args, 'binding_id': settlement['id']})
    assert task['coverage'][0]['exception'] == 'accounting_projection_binding'
    assert not tasks.preview(conn, profile, task['id'], 'prepare')['ready']


def bind_events(conn, profile, snapshot, artifact, categories):
    from kassiber.core.accounting import projection_views
    rows = projection_views.list_events(conn, profile, artifact_id=artifact['id'], period_id='2025')['rows']
    result = []
    for row in rows:
        category = categories.get(row['event_id'], row['categories'][0])
        binding = sources.bind_sources(conn, profile, snapshot_id=snapshot['id'], expected_digest=snapshot['input_digest'],
            economic_id=row['event_id'], role='settlement' if category == 'custody_move' else 'recognition',
            claims=row['claims'], reason='Reviewed exact source slices', idempotency_key='bind-' + row['event_id'])
        result.append(dict(event_id=row['event_id'], binding_id=binding['id'], category=category))
    return result


def test_frontier_subcent_disposals_match_manual_chronological_projection(book):
    conn, profile, args = prepared(book)
    sources.void_binding(conn, profile, binding_id=args['binding_id'], reason='Synthetic exact subcent sequence', idempotency_key='void')
    conn.execute("UPDATE transactions SET fiat_rate_exact='0.03' WHERE id='acquisition'")
    original = ledger._row(conn, "SELECT * FROM transactions WHERE id='acquisition'")
    for identifier, day in [('sale-one', '2025-02-01'), ('sale-two', '2025-03-01')]:
        conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,
            direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
            VALUES(?,?,?,?,?,?,?,'outbound','BTC',50000000000,0,'EUR','0.10','{}',?,'sell')''',
            (identifier, original['workspace_id'], profile, original['wallet_id'], identifier, 'fp-' + identifier,
             day + 'T12:00:00Z', day + 'T12:00:00Z'))
    snapshot = sources.capture_sources(conn, profile)
    artifact = artifacts.capture_calculation(conn, profile, snapshot_id=snapshot['id'], period_id='2025')
    events = bind_events(conn, profile, snapshot, artifact, {})
    # Independent existing manual API is the oracle; no manually prepared GL
    # drafts survive into the actual task workflow.
    expected = []
    conn.execute('SAVEPOINT manual_oracle')
    try:
        for event in events:
            proposal = projection.create_proposal(conn, profile, policy_id=args['policy_id'], artifact_id=artifact['id'],
                period_id='2025', idempotency_key='manual-' + event['event_id'], **event)
            expected.append((proposal['proposal']['lines'], proposal['proposal']['quantitative_posting']['basis_exact']))
            projection.post_proposal(conn, profile, proposal_id=proposal['id'], expected_digest=proposal['payload_digest'])
    finally:
        conn.execute('ROLLBACK TO manual_oracle')
        conn.execute('RELEASE manual_oracle')
    task = task_for(conn, profile, {**args, 'artifact_id': artifact['id']}, events=list(reversed(events)))
    for index, expected_effects in enumerate(expected):
        preview = tasks.preview(conn, profile, task['id'], 'prepare')
        assert len(preview['proposals']) == 1
        prepared_result, _ = approve(conn, profile, task['id'], 'prepare', f'prepare-{index}')
        proposal = projection.get_proposal(conn, profile, prepared_result['result']['projection_ids'][0])
        assert (proposal['proposal']['lines'], proposal['proposal']['quantitative_posting']['basis_exact']) == expected_effects
        assert not tasks.preview(conn, profile, task['id'], 'prepare')['ready']
        approve(conn, profile, task['id'], 'post', f'post-{index}')
    assert all(row['status'] == 'posted' for row in tasks.get(conn, profile, task['id'])['coverage'])
    assert conn.execute("SELECT SUM(debit_minor-credit_minor) FROM gl_lines WHERE account_code='btc'").fetchone()[0] == 0
    assert not projection.validate_close(conn, profile, '2025-01-01', '2025-12-31')['blockers']


def test_native_same_asset_custody_publishes_without_fiat_draft(book):
    from kassiber.core import custody_components
    from kassiber.core.wallets import create_wallet
    conn, profile, args = prepared(book)
    sources.void_binding(conn, profile, binding_id=args['binding_id'], reason='Add reviewed custody', idempotency_key='void')
    original = ledger._row(conn, "SELECT * FROM transactions WHERE id='acquisition'")
    wallet = create_wallet(conn, original['workspace_id'], profile, 'Second custody wallet', 'custom')
    legs = []
    for identifier, direction, wallet_id, role in [('dispatch', 'outbound', original['wallet_id'], 'source'), ('receipt', 'inbound', wallet['id'], 'destination')]:
        conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,
            direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
            VALUES(?,?,?,?,?,?,'2025-02-01T12:00:00Z',?,'BTC',100000000000,0,'EUR','100','{}','2025-02-01T12:00:00Z','transfer')''',
            (identifier, original['workspace_id'], profile, wallet_id, identifier, 'fp-' + identifier, direction))
        legs.append(dict(id=role, role=role, transaction_id=identifier, wallet_id=wallet_id, rail='bitcoin', chain='bitcoin',
            network='regtest', asset='BTC', exposure='bitcoin', conservation_unit='msat', amount_msat=100000000000))
    component = custody_components.create_component(conn, workspace_id=original['workspace_id'], profile_id=profile,
        component_type='manual_bridge', legs=legs,
        allocations=[dict(source_leg_id='source', sink_leg_id='destination', source_amount_msat=100000000000, sink_amount_msat=100000000000)],
        evidence_kind='manual_claim', evidence_grade='reviewed', notes='Reviewed same-asset custody', conversion_policy='carrying-value', conversion_reviewed=True)
    custody_components.activate_component(conn, component['id'])
    snapshot = sources.capture_sources(conn, profile)
    artifact = artifacts.capture_calculation(conn, profile, snapshot_id=snapshot['id'], period_id='2025')
    events = bind_events(conn, profile, snapshot, artifact, {})
    assert any(row['category'] == 'custody_move' for row in events)
    task = task_for(conn, profile, {**args, 'artifact_id': artifact['id']}, events=events)
    approve(conn, profile, task['id'], 'prepare', 'prepare-buy')
    approve(conn, profile, task['id'], 'post', 'post-buy')
    custody, _ = approve(conn, profile, task['id'], 'prepare', 'prepare-custody')
    assert custody['result']['draft_ids'] == []
    proposal = projection.get_proposal(conn, profile, custody['result']['projection_ids'][0])
    assert proposal['proposal']['quantitative_posting']['custody_move']['crypto_sent_msat'] == 100000000000
    posted, _ = approve(conn, profile, task['id'], 'post', 'post-custody')
    assert posted['result']['posted_ids'] == [] and posted['result']['projection_ids'] == [proposal['id']]
    assert all(row['status'] == 'posted' for row in posted['task']['coverage'])
    assert conn.execute('SELECT count(*) FROM gl_projection_publications').fetchone()[0] == 2


@pytest.mark.parametrize('already_booked', [False, True])
def test_shared_bank_claim_requires_explicit_reconciliation_not_second_rule_post(book, already_booked):
    from kassiber.core.accounting import bank
    conn, profile, args = prepared(book)
    sources.void_binding(conn, profile, binding_id=args['binding_id'], reason='Review shared purchase source', idempotency_key='void')
    statement = bank.import_statement(conn, profile, account_code='bank', statement_id='purchase-payment',
        start_date='2025-01-01', end_date='2025-12-31',
        csv_text='row_id,date,amount_minor,description\npayment,2025-01-01,-10000,Purchase\n')
    snapshot = sources.capture_sources(conn, profile)
    artifact = artifacts.capture_calculation(conn, profile, snapshot_id=snapshot['id'], period_id='2025')
    event_id, mapping = next(iter(artifact['capture']['inputs']['source_event_map'].items()))
    bank_source = next(row for row in snapshot['snapshot']['sources'] if row['kind'] == 'bank')
    claims = [dict(source_id=mapping['source_id'], **row) for row in mapping['claim_slices']]
    claims.append(dict(source_id=bank_source['source_id'], start_atomic=0, end_atomic=10000))
    binding = sources.bind_sources(conn, profile, snapshot_id=snapshot['id'], expected_digest=snapshot['input_digest'],
        economic_id='one-purchase', role='recognition', claims=claims, reason='Reviewed bank payment and acquired Bitcoin are one purchase', idempotency_key='combined')
    tasks.execute(conn, profile, 'rule-create', dict(idempotency_key='purchase-rule', account_code='bank', direction='out',
        description_exact='Purchase', counter_account_code='sales', reason='Existing rule must not duplicate a claimed purchase', confirmed=True))
    row_id = bank_source['facts']['bank_row_id']
    if already_booked:
        draft = ledger.create_draft(conn, profile, dict(period_id='2025', entry_date='2025-01-01', idempotency_key='manual-bank',
            description='Previously recognized payment', lines=[dict(account_code='bank', credit_minor=10000), dict(account_code='sales', debit_minor=10000)]))
        entry = ledger.post_draft(conn, profile, draft_id=draft['id'], expected_digest=draft['payload_digest'])
        line = next(row for row in entry['lines'] if row['account_code'] == 'bank')
        bank.allocate_bank_row(conn, profile, row_id=row_id, line_id=line['id'], amount_minor=10000, idempotency_key='manual-allocation')
    task = task_for(conn, profile, {**args, 'artifact_id': artifact['id'], 'event_id': event_id, 'binding_id': binding['id']}, statement_ids=[statement['id']])
    if already_booked:
        assert task['coverage'][-1]['exception'] == 'accounting_task_bank_source_already_booked'
        assert not tasks.preview(conn, profile, task['id'], 'prepare')['ready']
        assert not conn.execute('SELECT 1 FROM gl_projection_proposals').fetchone()
        return
    assert task['coverage'][0]['exception'] == 'accounting_task_bank_source_claimed'
    prepared_result, _ = approve(conn, profile, task['id'], 'prepare')
    assert len(prepared_result['result']['draft_ids']) == 1
    posted, _ = approve(conn, profile, task['id'], 'post')
    assert posted['task']['coverage'][0]['exception'] == 'accounting_task_bank_source_claimed'
    entry = ledger._entry(conn, profile, posted['result']['posted_ids'][0])
    line = next(row for row in entry['lines'] if row['account_code'] == 'bank')
    bank.allocate_bank_row(conn, profile, row_id=row_id, line_id=line['id'], amount_minor=10000, idempotency_key='reviewed-settlement')
    current = tasks.get(conn, profile, task['id'])
    assert [row['status'] for row in current['coverage']] == ['covered', 'posted']
    assert conn.execute('SELECT count(*) FROM gl_entries').fetchone()[0] == 1


def test_stale_post_and_cross_book_selection_fail_closed(book):
    from kassiber.core.accounts import create_profile
    conn, profile, args = prepared(book)
    workspace = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (profile,)).fetchone()[0]
    other = create_profile(conn, workspace, 'Other', 'EUR', 'FIFO', 'generic', 365)['id']
    ledger.configure_book(conn, other, currency='EUR', timezone='Europe/Vienna')
    ledger.create_period(conn, other, period_id='2025', start_date='2025-01-01', end_date='2025-12-31')
    with pytest.raises(AppError):
        task_for(conn, other, args)
    task = task_for(conn, profile, args)
    approve(conn, profile, task['id'], 'prepare')
    preview = tasks.preview(conn, profile, task['id'], 'post')
    conn.execute("UPDATE transactions SET fiat_rate_exact='101' WHERE id='acquisition'")
    with pytest.raises(AppError) as error:
        tasks.execute(conn, profile, 'task-apply', dict(task_id=task['id'], step='post', expected_revision=preview['expected_revision'],
            expected_digest=preview['expected_digest'], confirmed=True, idempotency_key='post'))
    assert error.value.code == 'accounting_stale_approval'
    assert not conn.execute('SELECT 1 FROM gl_projection_publications').fetchone()


def test_existing_exact_projection_and_explicit_draft_selection_post_only_once(book):
    conn, profile, args = prepared(book)
    manual = projection.create_proposal(conn, profile, **args)
    task = task_for(conn, profile, args, draft_ids=[manual['draft_id']])
    assert task['next_step'] == 'post'
    result, _ = approve(conn, profile, task['id'], 'post')
    assert result['result']['posted_ids'] == [manual['draft_id']]
    assert result['result']['projection_ids'] == [manual['id']]
    assert conn.execute('SELECT count(*) FROM gl_projection_publications').fetchone()[0] == 1


def test_projection_prepare_rolls_back_when_receipt_cannot_be_retained(book):
    import sqlite3
    conn, profile, args = prepared(book)
    task = task_for(conn, profile, args)
    revision = ledger.require_book(conn, profile)['revision']
    def deny_receipt(action, table, _column, _database, _trigger):
        return sqlite3.SQLITE_DENY if action == sqlite3.SQLITE_INSERT and table == 'gl_accounting_task_receipts' else sqlite3.SQLITE_OK
    conn.set_authorizer(deny_receipt)
    try:
        with pytest.raises(Exception, match='not authorized'):
            approve(conn, profile, task['id'], 'prepare')
    finally:
        conn.set_authorizer(None)
    assert not conn.execute('SELECT 1 FROM gl_projection_proposals').fetchone()
    assert not conn.execute('SELECT 1 FROM gl_entries').fetchone()
    assert ledger.require_book(conn, profile)['revision'] == revision


def test_unknown_or_foreign_explicit_binding_is_rejected_before_task_retention(book):
    from kassiber.core.accounts import create_profile
    conn, profile, args = prepared(book)
    with pytest.raises(AppError) as error:
        task_for(conn, profile, {**args, 'binding_id': 'missing-binding'})
    assert error.value.code == 'not_found'
    assert not conn.execute('SELECT 1 FROM gl_accounting_tasks').fetchone()
    workspace = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (profile,)).fetchone()[0]
    other = create_profile(conn, workspace, 'Other binding owner', 'EUR', 'FIFO', 'generic', 365)['id']
    ledger.configure_book(conn, other, currency='EUR', timezone='Europe/Vienna')
    # Its exact ID exists but never resolves through a different book scope.
    from kassiber.core.accounting import task_projection
    with pytest.raises(AppError) as error:
        task_projection.normalize(conn, other, '2025', dict(artifact_id=args['artifact_id'], policy_id=args['policy_id'],
            events=[dict(event_id=args['event_id'], binding_id=args['binding_id'])]), 10000)
    assert error.value.code == 'not_found'
    assert not conn.execute('SELECT 1 FROM gl_accounting_tasks').fetchone()
