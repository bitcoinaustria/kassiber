"""Deterministic tool/consent outcome proofs, not a model or human-time pilot.

Only provider transport and dispatch scheduling are scripted. Domain operations,
scope validation, advertised schemas, human consent events and persistence are real.
Explicit intake/assignment steps are counted separately, never called agent work.
"""
import hashlib
import io
import json
import time
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import daemon
from kassiber.ai.tools import get_tool
from kassiber.core.accounting import bank, evidence, ledger, tasks
from kassiber.db import open_db
from tests.test_accounting_agent_tasks import runtime_for
from tests.test_accounting_integration import book  # noqa: F401


def _data(results):
    return results[-1]['envelope']['data']


def agent_run(conn, profile_id, root, operations, *, decisions=(), capabilities=('accounting_tasks',), require_success=False):
    runtime = runtime_for(conn, profile_id, root)
    chats = daemon.ActiveAiChats()
    _, active = chats.register('outcome-fixture')
    events, calls, results = [], [], []
    approvals = iter(decisions)
    started = time.perf_counter()

    class LocalReview(io.StringIO):
        def write(self, text):
            length = super().write(text)
            for line in text.splitlines():
                if not line.strip():
                    continue
                event = json.loads(line)
                events.append(event)
                if event['kind'] == 'ai.chat.tool_consent_required':
                    assert active.consent.record(event['data']['call_id'], next(approvals))
            return length

    def provider(_rid, _client, _validated, context, offered, _out, _cancel):
        results[:] = [json.loads(item['output']) for item in context.input_items if item.get('type') == 'function_call_output']
        if require_success:
            assert all(result.get('ok') is True for result in results), json.dumps({'calls': calls, 'results': results}, sort_keys=True)
        if len(calls) == len(operations):
            return daemon.AiToolTurnResult([], 'Authoritative task results inspected.', '', 'stop', [])
        kind, arguments = operations[len(calls)]
        entry = get_tool(kind)
        assert entry.provider_name in {tool['name'] for tool in offered}
        payload = arguments(results) if callable(arguments) else arguments
        calls.append((kind, payload))
        return daemon.AiToolTurnResult([{'id': str(len(calls)), 'function': {
            'name': entry.provider_name, 'arguments': json.dumps(payload)}}], '', '', 'tool_calls', [])

    validated = daemon._ai_chat_args({'model': 'scripted-outcome-fixture', 'tools_enabled': True,
        'persist': False, 'tool_loop_max_iterations': 32,
        'messages': [{'role': 'user', 'content': 'Continue this explicitly selected accounting or portfolio review task.'}],
        'screen_context': {'route': '/assistant', 'capabilities': list(capabilities)}})
    with patch.object(daemon, '_run_on_daemon_main_thread', side_effect=lambda _, callback: callback(conn)), \
         patch.object(daemon, '_stream_ai_chat_tool_turn', provider):
        daemon._run_ai_chat_tool_loop('outcome-fixture', SimpleNamespace(last_provider_session_id=None),
            {'name': 'synthetic-local', 'kind': 'local', 'base_url': 'http://localhost'}, validated,
            daemon._OutputChannel(LocalReview()), active, runtime, chats)
    assert len(calls) == len(operations)
    return {'calls': calls, 'results': results, 'events': events, 'elapsed_seconds': time.perf_counter() - started,
            'approval_count': sum(event['kind'] == 'ai.chat.tool_consent_required' for event in events),
            'active_user_time_seconds': None, 'provider_inference_measured': False}


def task_steps(task_id, *steps, key_prefix='outcome-'):
    def apply_arguments(results, step):
        data = _data(results)
        assert 'approval_id' in data, (step, json.dumps(results, sort_keys=True))
        return {'task_id': task_id, 'approval_id': data['approval_id'], 'idempotency_key': key_prefix + step}

    calls = []
    for step in steps:
        calls.append(('ui.accounting.task_preview', {'task_id': task_id, 'step': step}))
        calls.append(('ui.accounting.task_apply', lambda results, step=step: apply_arguments(results, step)))
    calls.append(('ui.accounting.task_get', {'task_id': task_id}))
    return calls


def _posted_facts(conn, profile):
    return sorted(tuple(row) for row in conn.execute('''SELECT e.entry_date,l.account_code,l.debit_minor,l.credit_minor
        FROM gl_entries e JOIN gl_lines l ON l.entry_id=e.id WHERE e.profile_id=? AND e.status='posted' ''', (profile,)))


def _manual_post(conn, profile, key, amount, *, debit='bank', credit='sales', day='2025-06-15'):
    draft = ledger.create_draft(conn, profile, {'idempotency_key': key, 'period_id': '2025',
        'entry_date': day, 'description': 'Independent manual fixture', 'lines': [
            {'account_code': debit, 'debit_minor': amount}, {'account_code': credit, 'credit_minor': amount}]})
    return ledger.post_draft(conn, profile, draft_id=draft['id'], expected_digest=draft['payload_digest'])


def _selected_june_bitcoin_source(conn, profile):
    from kassiber.core.accounting import artifacts, projection, sources
    from kassiber.core.wallets import create_wallet

    for code, kind in [('btc', 'asset'), ('income', 'income'), ('gain', 'income'), ('fees', 'expense')]:
        ledger.create_account(conn, profile, code=code, name=code, kind=kind)
    policy = projection.configure_policy(conn, profile, period_id='2025', asset_accounts={'BTC': 'btc'},
        settlement_account='bank', income_account='income', capital_account='capital',
        gain_account='gain', fee_account='fees', acknowledge_tax_book_basis=True,
        reason='Explicit synthetic user accounting policy')
    workspace = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (profile,)).fetchone()[0]
    wallet = create_wallet(conn, workspace, profile, 'Synthetic June source', 'custom')
    conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,
        occurred_at,direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
        VALUES('june-acquisition',?,?,?,'june-source','june-fingerprint','2025-06-10T12:00:00Z','inbound',
        'BTC',100000000000,0,'EUR','100','{}','2025-06-10T12:00:00Z','buy')''', (workspace, profile, wallet['id']))
    # Capture after the entire selected bank/evidence population was imported.
    # Policy, source binding and selection are explicit user setup, not posting.
    snapshot = sources.capture_sources(conn, profile)
    artifact = artifacts.capture_calculation(conn, profile, snapshot_id=snapshot['id'], period_id='2025', boundary='closing')
    event_id, mapping = next((key, row) for key, row in artifact['capture']['inputs']['source_event_map'].items()
        if row['journal_transaction_id'] == 'june-acquisition')
    binding = sources.bind_sources(conn, profile, snapshot_id=snapshot['id'], expected_digest=snapshot['input_digest'],
        economic_id='reviewed-june-acquisition', role='recognition', reason='Reviewed synthetic purchase',
        idempotency_key='june-source-binding', claims=[dict(source_id=mapping['source_id'], **part) for part in mapping['claim_slices']])
    return dict(artifact_id=artifact['id'], policy_id=policy['id'],
        events=[dict(event_id=event_id, binding_id=binding['id'], category='purchase')])


def test_mixed_selected_population_agent_routines_match_manual_without_hiding_setup_or_exceptions(book, record_property):
    from kassiber.core.accounts import create_profile
    from kassiber.core.accounting import projection

    conn, profile, root = book
    manifest = {'id': 'mixed-june-v1', 'bank_rows': 100, 'reviewed_routine_rows': 96,
        'bank_exceptions': ['Unknown', 'Conflict', 'Partial', 'Needs document'],
        'selected_documents': 3, 'bank_document_aliases': 2, 'selected_raw_btc_observations': 1}
    fixture_digest = hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()
    rows = ['row_id,date,amount_minor,description']
    rows.extend(f'{i},2025-06-15,100,Membership' for i in range(96))
    rows.extend(f'{i},2025-06-15,100,{label}' for i, label in enumerate(manifest['bank_exceptions'], 96))
    statement = bank.import_statement(conn, profile, account_code='bank', statement_id='mixed-june-v1',
        start_date='2025-06-01', end_date='2025-06-30', csv_text='\n'.join(rows) + '\n')
    bank_rows = bank.reconcile_statement(conn, profile, statement['id'])['rows']
    partial = _manual_post(conn, profile, 'initial-partial', 50)
    bank.allocate_bank_row(conn, profile, row_id=next(row['id'] for row in bank_rows if row['description'] == 'Partial'),
        line_id=next(line['id'] for line in partial['lines'] if line['account_code'] == 'bank'), amount_minor=50, idempotency_key='initial-partial')
    documents = [evidence.retain_evidence(conn, profile, content=content, media_type='text/plain', name=name) for content, name in (
        (b'Synthetic duplicate representation of first membership payment', 'Original receipt'),
        (b'Synthetic duplicate representation of first membership payment', 'Duplicate receipt'),
        (b'Synthetic incomplete source requiring a missing-document answer', 'Missing supporting information'))]
    selected_bitcoin = _selected_june_bitcoin_source(conn, profile)
    assert not conn.execute('SELECT 1 FROM gl_projection_proposals').fetchone()
    task = tasks.execute(conn, profile, 'task-create', {'period_id': '2025', 'statement_ids': [statement['id']],
        'evidence_ids': [item['id'] for item in documents], 'projection': selected_bitcoin, 'idempotency_key': 'mixed-june-v1'})
    assert task['spec']['draft_ids'] == []
    for name, counter in [('Membership', 'sales'), ('Conflict', 'sales'), ('Conflict', 'capital')]:
        tasks.execute(conn, profile, 'rule-create', {'idempotency_key': name + counter, 'account_code': 'bank',
            'direction': 'in', 'description_exact': name, 'counter_account_code': counter, 'reason': 'Explicit synthetic user assignment', 'confirmed': True})
    tasks.execute(conn, profile, 'task-source-assign', {'task_id': task['id'], 'evidence_id': documents[0]['id'], 'kind': 'bank_evidence',
        'bank_row_id': next(row['id'] for row in bank_rows if row['description'] == 'Membership'),
        'reason': 'Confirmed same economic event', 'confirmed': True, 'idempotency_key': 'duplicate-link'})
    before = tasks.preview(conn, profile, task['id'], 'prepare')
    assert before['source_count'] == 104
    assert len(before['proposals']) == 97
    assert len(before['exceptions']) == 5
    assert {row['exception'] for row in before['exceptions']} == {
        'missing_assignment_rule', 'ambiguous_assignment_rule', 'partial_bank_allocation', 'evidence_assignment_required'}
    expected_routine_ids = {row['id'] for row in bank_rows if row['description'] == 'Membership'}
    assert {row['source_id'] for row in before['proposals'] if row['source_kind'] == 'bank'} == expected_routine_ids
    assert all(row['rule_id'] for row in before['proposals'] if row['source_kind'] == 'bank')
    assert not conn.execute('SELECT 1 FROM gl_projection_proposals').fetchone()
    run = agent_run(conn, profile, root, task_steps(task['id'], 'prepare', 'post'), decisions=['allow_once'] * 2, require_success=True)
    state = tasks.get(conn, profile, task['id'])
    assert len(state['coverage']) == 104 and len(state['exceptions']) == 5
    btc_row = next(row for row in state['coverage'] if row['source_kind'] == 'bitcoin')
    assert projection.get_proposal(conn, profile, btc_row['proposal_id'])['published'] is True, run['results']
    assert all(row['status'] == 'posted' for row in state['coverage'] if row['source_id'] in expected_routine_ids)
    assert conn.execute("SELECT COUNT(*) FROM gl_entries WHERE profile_id=? AND status='posted'", (profile,)).fetchone()[0] == 98
    # Separate manual ledger, built from frozen fixture facts rather than the
    # agent's proposed payloads, proves the resulting financial lines agree.
    workspace = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (profile,)).fetchone()[0]
    manual = create_profile(conn, workspace, 'Independent manual comparison', 'EUR', 'FIFO', 'generic', 365)['id']
    ledger.configure_book(conn, manual, currency='EUR', timezone='Europe/Vienna')
    for code, kind in [('bank', 'asset'), ('btc', 'asset'), ('sales', 'income')]:
        ledger.create_account(conn, manual, code=code, name=code, kind=kind)
    ledger.create_period(conn, manual, period_id='2025', start_date='2025-01-01', end_date='2025-12-31')
    _manual_post(conn, manual, 'partial', 50)
    _manual_post(conn, manual, 'btc', 10000, debit='btc', credit='bank', day='2025-06-10')
    for index in range(96):
        _manual_post(conn, manual, str(index), 100)
    assert _posted_facts(conn, profile) == _posted_facts(conn, manual)
    metrics = {**manifest, 'fixture_digest': fixture_digest, 'selected_records_accounted': 104,
        'correct_routine_proposals': 97, 'missed_selected_cases': 0, 'wrong_routine_proposals': 0,
        'routine_user_corrections': 0, 'routine_repeated_data_entries': 0, 'surfaced_exceptions': 5,
        'clarification_requests': 0,  # This run surfaces exceptions; explicit answers are exercised separately.
        'agent_approval_count': run['approval_count'], 'manual_equivalent_post_operations': 98,
        'elapsed_seconds': run['elapsed_seconds'], 'active_user_time_seconds': None,
        'provider_inference_measured': False, 'agent_btc_preparation_proven': True}
    record_property('synthetic_outcome_metrics', json.dumps(metrics, sort_keys=True))
    assert run['approval_count'] == 2


def test_agent_resumes_same_task_after_explicit_new_document_and_source_answers(book):
    from tests.test_accounting_cli_tasks import cli

    conn, profile, root = book
    statement = bank.import_statement(conn, profile, account_code='bank', statement_id='missing-document',
        start_date='2025-06-01', end_date='2025-06-30', csv_text=(
            'row_id,date,amount_minor,description\n'
            'routine,2025-06-15,100,Membership\nmissing,2025-06-15,100,Needs document\n'))
    incomplete = evidence.retain_evidence(conn, profile, content=b'Synthetic receipt lacks attribution',
        media_type='text/plain', name='Incomplete receipt')
    task = tasks.execute(conn, profile, 'task-create', dict(period_id='2025',
        statement_ids=[statement['id']], evidence_ids=[incomplete['id']], idempotency_key='missing-document'))
    tasks.execute(conn, profile, 'rule-create', dict(idempotency_key='membership', account_code='bank',
        direction='in', description_exact='Membership', counter_account_code='sales',
        reason='Explicit reviewed recurring assignment', confirmed=True))
    first = agent_run(conn, profile, root, task_steps(task['id'], 'prepare', 'post'), decisions=['allow_once'] * 2)
    prior = tasks.get(conn, profile, task['id'])
    assert {row['exception'] for row in prior['exceptions']} == {'missing_assignment_rule', 'evidence_assignment_required'}
    original_spec = conn.execute('SELECT spec_json FROM gl_accounting_tasks WHERE id=?', (task['id'],)).fetchone()[0]
    original_posted = _posted_facts(conn, profile)
    supplied = evidence.retain_evidence(conn, profile, content=b'Synthetic new supporting document confirms donation',
        media_type='text/plain', name='Newly supplied supporting document')
    missing_row = next(row for row in bank.reconcile_statement(conn, profile, statement['id'])['rows']
                       if row['description'] == 'Needs document')
    conn.commit()
    # New scope and financial interpretation are explicit local user input,
    # not model-inferred authority. Each CLI call opens a fresh persisted book.
    selection = dict(task_id=task['id'], period_id='2025', evidence_ids=[supplied['id']], reason='New missing document supplied')
    preview = cli(root, 'task-amend-preview', selection)['data']
    amendment = cli(root, 'task-amend', dict(**selection, expected_digest=preview['expected_digest'],
        expected_revision=preview['expected_revision'], confirmed=True, idempotency_key='supply-document'))['data']
    for index, item in enumerate((incomplete, supplied)):
        cli(root, 'task-source-assign', dict(task_id=task['id'], evidence_id=item['id'], kind='bank_evidence',
            bank_row_id=missing_row['id'], reason='Same reviewed bank event, not another payment',
            confirmed=True, idempotency_key=f'supplied-document-{index}'))
    cli(root, 'rule-create', dict(idempotency_key='answered-donation', account_code='bank', direction='in',
        description_exact='Needs document', counter_account_code='sales', reason='User confirms donation source', confirmed=True))
    second = agent_run(conn, profile, root, task_steps(task['id'], 'prepare', 'post', key_prefix='resumed-'),
        decisions=['allow_once'] * 2)
    conn.commit()
    final = cli(root, 'task-get', {'task_id': task['id']})['data']
    assert final['id'] == task['id'] and final['scope_revision'] == 1
    assert not final['exceptions'] and len(final['coverage']) == 4
    assert all(row['status'] in ('posted', 'covered') for row in final['coverage'])
    assert final['receipts'][:len(prior['receipts'])] == prior['receipts']
    retained_amendment = next(row for row in final['receipts'] if row['id'] == amendment['receipt']['id'])
    assert retained_amendment['result'] == amendment['receipt']['result']
    assert retained_amendment['step'] == 'amend_sources'
    assert conn.execute('SELECT spec_json FROM gl_accounting_tasks WHERE id=?', (task['id'],)).fetchone()[0] == original_spec
    assert len(_posted_facts(conn, profile)) == 2 * len(original_posted)
    assert conn.execute("SELECT COUNT(*) FROM gl_entries WHERE status='posted'").fetchone()[0] == 2
    assert first['approval_count'] == second['approval_count'] == 2


def test_private_assignment_agent_denial_restart_retry_matches_manual_without_gl(tmp_path):
    from kassiber.cli.handlers import _metadata_hooks
    from kassiber.core import custody_journal, review_workflow
    from tests.test_review_workflow import ReviewWorkflowTest

    root = tmp_path / 'private-agent'
    conn = open_db(root)
    manual = open_db(tmp_path / 'private-manual')
    operations = [
        {'type': 'price_override', 'transaction_id': 'a', 'fiat_rate': '20000', 'reason': 'Synthetic invoice reviewed'},
        {'type': 'exclude', 'transaction_id': 'b', 'reason': 'Synthetic duplicate source reviewed'},
    ]
    for target in (conn, manual):
        ReviewWorkflowTest.seed(target)
        profile = target.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
        custody_journal.store_ledger_state(target, profile, custody_journal.build_ledger_state(target, profile))
        target.commit()
    request = {'expected_input_version': 0, 'operations': operations}
    denied = agent_run(conn, 'p', root, [
        ('ui.review.cases', {}), ('ui.review.plan', request),
        ('ui.review.apply', lambda results: {'artifact': _data(results), 'idempotency_key': 'private-denied'}),
    ], decisions=['deny'], capabilities=('review', 'reports'))
    assert denied['approval_count'] == 1
    assert conn.execute('SELECT COUNT(*) FROM transaction_edit_events').fetchone()[0] == 0
    interrupted = agent_run(conn, 'p', root, [('ui.review.plan', request)], capabilities=('review',))
    artifact = _data(interrupted['results'])
    conn.close()
    conn = open_db(root)
    try:
        approved = agent_run(conn, 'p', root, [
            ('ui.review.apply', {'artifact': artifact, 'idempotency_key': 'private-approved'}),
            ('ui.reports.portfolio_summary', {}), ('ui.reports.balance_sheet', {}),
        ], decisions=['allow_once'], capabilities=('review', 'reports'))
        receipt = approved['results'][0]['envelope']['data']
        assert receipt['status'] == 'verified'
        retried = agent_run(conn, 'p', root, [('ui.review.apply', {'artifact': artifact, 'idempotency_key': 'private-approved'})],
            decisions=['allow_once'], capabilities=('review',))
        assert _data(retried['results'])['id'] == receipt['id']
        assert conn.execute('SELECT COUNT(*) FROM review_workflow_receipts').fetchone()[0] == 1
        assert conn.execute('SELECT COUNT(*) FROM transaction_edit_events').fetchone()[0] == 2
        hooks = review_workflow.ReviewHooks(metadata=_metadata_hooks())
        profile = manual.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
        plan = review_workflow.plan_review(manual, profile, operations=operations, expected_input_version=0, hooks=hooks)
        review_workflow.apply_review(manual, profile, artifact=plan, idempotency_key='manual', hooks=hooks)
        runtime_for(manual, 'p', tmp_path / 'private-manual')
        assert approved['results'][1]['envelope']['data'] == daemon.redact_ai_tool_result(daemon._reports_portfolio_summary_payload(manual))
        assert approved['results'][2]['envelope']['data'] == daemon.redact_ai_tool_result(daemon._reports_balance_sheet_payload(manual))
        assert tuple(conn.execute('SELECT quantity,cost_basis FROM journal_wallet_holdings').fetchone()) == (100000000000, 20000)
        assert ledger.snapshot(conn, 'p')['configured'] is False
        assert conn.execute('SELECT COUNT(*) FROM gl_books').fetchone()[0] == 0
    finally:
        conn.close()
        manual.close()


def test_agent_rule_reuse_nonmatch_conflict_revocation_and_stale_consent(book):
    from kassiber.errors import AppError
    from tests.test_accounting_tasks import rule

    conn, profile, root = book
    original = bank.import_statement(conn, profile, account_code='bank', statement_id='first-import',
        start_date='2025-02-01', end_date='2025-02-28',
        csv_text='row_id,date,amount_minor,description\nfirst,2025-02-03,100,Membership\n')
    initial = tasks.execute(conn, profile, 'task-create', {'period_id': '2025', 'statement_ids': [original['id']], 'idempotency_key': 'first-import'})
    reviewed_rule = rule(conn, profile)
    agent_run(conn, profile, root, task_steps(initial['id'], 'prepare', 'post'), decisions=['allow_once'] * 2)
    statement = bank.import_statement(conn, profile, account_code='bank', statement_id='next-import',
        start_date='2025-06-01', end_date='2025-06-30',
        csv_text='row_id,date,amount_minor,description\nnext,2025-06-01,100,Membership\nother,2025-06-02,200,Not membership\n')
    task = tasks.execute(conn, profile, 'task-create', {'period_id': '2025', 'statement_ids': [statement['id']], 'idempotency_key': 'next-import'})
    seen = agent_run(conn, profile, root, [('ui.accounting.task_preview', {'task_id': task['id'], 'step': 'prepare'})])
    assert _data(seen['results'])['counts'] == {'ready': 1, 'draft': 0, 'posted': 0, 'covered': 0, 'exception': 1}
    preview = tasks.preview(conn, profile, task['id'], 'prepare')
    assert [item['rule_id'] for item in preview['proposals']] == [reviewed_rule['id']]
    conflict = tasks.execute(conn, profile, 'rule-create', {'idempotency_key': 'conflict', 'account_code': 'bank',
        'direction': 'in', 'description_exact': 'Membership', 'counter_account_code': 'capital',
        'reason': 'Synthetic competing reviewed choice', 'confirmed': True})
    assert {row['exception'] for row in tasks.get(conn, profile, task['id'])['exceptions']} == {'missing_assignment_rule', 'ambiguous_assignment_rule'}
    tasks.execute(conn, profile, 'rule-revoke', {'rule_id': conflict['id'], 'reason': 'User rejects competing choice'})

    def revoke_between_preview_and_apply(results):
        # Models an explicit local correction while the assistant is waiting,
        # not a model-authorized mutation or a hidden extra tool.
        tasks.execute(conn, profile, 'rule-revoke', {'rule_id': reviewed_rule['id'], 'reason': 'User revoked before approval'})
        return {'task_id': task['id'], 'approval_id': _data(results)['approval_id'], 'idempotency_key': 'stale-rule'}

    stale = agent_run(conn, profile, root, [
        ('ui.accounting.task_preview', {'task_id': task['id'], 'step': 'prepare'}),
        ('ui.accounting.task_apply', revoke_between_preview_and_apply),
    ], decisions=['allow_once'])
    assert 'accounting_stale_approval' in json.dumps(stale['results'])
    assert conn.execute('SELECT COUNT(*) FROM gl_entries WHERE profile_id=?', (profile,)).fetchone()[0] == 1
    assert tasks.get(conn, profile, initial['id'])['coverage'][0]['status'] == 'posted'
    with pytest.raises(AppError):
        tasks.execute(conn, 'another-book', 'task-get', {'task_id': task['id']})


def test_agent_approved_close_final_k2_and_exports_are_real_retained_artifacts(book):
    from kassiber.core.accounting import jurisdiction, package, tax_workpapers
    from tests.test_accounting_tax_workpapers import complete_patch
    from tests.test_accounting_cli_tasks import cli

    conn, profile, root = book
    controls = {'format': 'kassiber-bank-control-v1', 'account_code': 'bank', 'statement_id': 'annual',
        'start_date': '2025-01-01', 'end_date': '2025-12-31', 'opening_minor': 0, 'closing_minor': 300,
        'currency': 'EUR', 'minor_unit_exponent': 2}
    control = evidence.retain_evidence(conn, profile, content=json.dumps(controls).encode(),
        name='Synthetic bank controls', media_type='application/json')
    statement = bank.import_statement(conn, profile, account_code='bank', statement_id='annual',
        start_date='2025-01-01', end_date='2025-12-31', opening_minor=0, closing_minor=300,
        control_evidence_id=control['id'], control_review_reason='Verified synthetic controls', control_locator='Entire record',
        csv_text='row_id,date,amount_minor,description\na,2025-06-01,100,Membership\nb,2025-06-02,100,Membership\nc,2025-06-03,100,Membership\n')
    tasks.execute(conn, profile, 'rule-create', {'idempotency_key': 'annual-rule', 'account_code': 'bank',
        'direction': 'in', 'description_exact': 'Membership', 'counter_account_code': 'sales',
        'reason': 'Synthetic user-reviewed allocation', 'confirmed': True})
    # Reviewed legal facts are explicit synthetic user inputs, not model inference.
    paper = tax_workpapers.create_workpaper(conn, profile, period_id='2025', pack_id=jurisdiction.AT_PACK_ID, idempotency_key='annual')
    reviewed = complete_patch()
    reviewed['field_reviews']['main.660'] = None
    reviewed['mappings'] = [{'id': 'fixture-income', 'field_key': 'main.660', 'account_code': 'sales',
        'basis': 'movement', 'amount_minor': -300, 'multiplier': -1, 'reason': 'Explicit synthetic fiscal mapping, not legal advice'}]
    tax_workpapers.review_workpaper(conn, profile, workpaper_id=paper['id'], expected_revision=1,
        patch=reviewed, reason='Synthetic manual fact review', idempotency_key='reviewed')
    task = tasks.execute(conn, profile, 'task-create', {'period_id': '2025', 'statement_ids': [statement['id']],
        'tax_workpaper_id': paper['id'], 'idempotency_key': 'annual'})
    first = agent_run(conn, profile, root, task_steps(task['id'], 'prepare', 'post', 'close'), decisions=['allow_once'] * 3)
    checked = tasks.preview(conn, profile, task['id'], 'tax_finalize')
    assert checked['ready'], checked['blockers']
    result = agent_run(conn, profile, root, task_steps(task['id'], 'tax_finalize', 'export_close', 'export_tax'), decisions=['allow_once'] * 3)
    assert first['approval_count'] + result['approval_count'] == 6
    assert _data(result['results'])['state'] == 'completed'
    receipts = tasks.get(conn, profile, task['id'])['receipts']
    assert {row['step'] for row in receipts} == {'prepare', 'post', 'close', 'tax_finalize', 'export_close', 'export_tax'}
    assert bank.reconcile_statement(conn, profile, statement['id'])['reconciled'] is True
    close_id = next(row['result']['id'] for row in receipts if row['step'] == 'close')
    close_artifact = package.export_close(conn, profile, close_id=close_id)
    assert package.verify_package(close_artifact)['ledger_arithmetic'] == 'verified'
    assert package.verify_package(close_artifact)['entries_checked'] == 3
    final_id = next(row['result']['final_id'] for row in receipts if row['step'] == 'tax_finalize')
    final = tax_workpapers.export_workpaper(conn, profile, final_id=final_id, confirm_plaintext=True)
    assert final['report_digest'] == ledger.digest(final['report'])
    assert final['stale'] is False and final['report']['filed'] is False
    assert '<table>' in final['html']
    # Explicit local CLI retrieval is not misreported as agent file delivery.
    conn.commit()
    exported = cli(root, 'tax-export', {'final_id': final_id, 'confirm_plaintext': True})['data']
    assert exported['report_digest'] == final['report_digest']
    assert all(item['envelope']['data'].get('file_saved') is False for item in result['results'])
