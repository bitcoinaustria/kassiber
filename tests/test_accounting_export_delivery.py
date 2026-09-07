"""Actual approved export -> private local transport -> exact verified CLI file."""
import copy
from contextlib import contextmanager
import hashlib
import io
import json
import os
import queue
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import daemon, daemon_accounting_tasks as adapter, diagnostics
from kassiber.cli import accounting_consent, accounting_delivery as delivery, chat
from kassiber.cli.main import build_parser
from kassiber.core.accounting import tasks
from kassiber.db import resolve_database_path
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import get_row_class, open_encrypted
from tests.test_accounting_agent_outcomes import agent_run, task_steps
from tests.test_accounting_integration import book, post  # noqa: F401
from tests.test_accounting_tasks import approve
from tests.test_cli_accounting_consent import Tty, run_consent


@pytest.fixture
def exported(book):
    conn, profile, _ = book
    posted = post(conn, profile)
    task = tasks.execute(conn, profile, 'task-create', dict(period_id='2025', statement_ids=[],
        draft_ids=[posted['id']], idempotency_key='delivery-task'))
    approve(conn, profile, task['id'], 'close')
    grants = adapter.TaskApprovals()
    preview = adapter.execute(conn, profile, 'ui.accounting.task_preview',
        dict(task_id=task['id'], step='export_close'), grants)
    args = dict(task_id=task['id'], approval_id=preview['approval_id'], idempotency_key='delivery-export')
    consent = dict(call_id='export-call', name='ui.accounting.task_apply', arguments_preview=args,
        accounting_task_preview=adapter.consent_preview(conn, profile, args, grants))
    sideband = {}
    result = adapter.execute(conn, profile, 'ui.accounting.task_apply', args, grants, local_export=sideband)
    return task, consent, dict(call_id='export-call', ok=True, envelope={'data': result}, **{delivery.SIDEBAND: sideband})


def sink_for(exported, path):
    task, consent, _ = exported
    sink = delivery.Delivery([(task['id'], 'export_close', str(path))])
    sink.approve('export-call', consent)
    return sink


def test_exact_artifact_is_saved_verified_exclusively_and_retry_is_idempotent(exported, tmp_path):
    task, consent, result = exported
    path = tmp_path / 'close.json'
    sink = sink_for(exported, path)
    sink.consume(result)
    assert sink.receipts[-1]['file_verified'] is True
    assert path.read_bytes() == result[delivery.SIDEBAND]['artifact_json'].encode()
    assert os.stat(path).st_mode & 0o777 == 0o600
    assert result['envelope']['data']['file_saved'] is False
    original = path.stat().st_ino
    sink.consume(result)  # Duplicate event cannot reuse consumed local consent.
    assert len(sink.receipts) == 1
    sink.approve('retry', consent)
    sink.consume({**result, 'call_id': 'retry'})
    assert sink.receipts[-1]['file_verified'] and path.stat().st_ino == original
    output = io.StringIO()
    sink.render(output)
    assert 'LOCAL EXPORT RECEIPT (not model text)' in output.getvalue()
    assert task['id'] not in output.getvalue()  # Receipt names the retained artifact.


@pytest.mark.parametrize('mutation', ['unapproved', 'other_call', 'task', 'step', 'hash', 'identity', 'inner_hash', 'failed', 'missing'])
def test_denied_mismatched_or_tampered_result_never_writes(exported, tmp_path, mutation):
    path = tmp_path / 'close.json'
    sink = sink_for(exported, path)
    result = copy.deepcopy(exported[2])
    value = result[delivery.SIDEBAND]
    if mutation == 'unapproved':
        sink.pending.clear()
    elif mutation == 'other_call':
        result['call_id'] = 'other'
    elif mutation in ('task', 'step', 'hash'):
        value[{'task': 'task_id', 'step': 'step', 'hash': 'sha256'}[mutation]] = 'changed'
    elif mutation in ('identity', 'inner_hash'):
        artifact = json.loads(value['artifact_json'])
        artifact['id' if mutation == 'identity' else 'snapshot_json'] = 'tampered'
        value['artifact_json'] = json.dumps(artifact)
        value['sha256'] = hashlib.sha256(value['artifact_json'].encode()).hexdigest()
    elif mutation == 'failed':
        result['ok'] = False
    else:
        result.pop(delivery.SIDEBAND)
    sink.consume(result)
    assert not path.exists()
    assert not sink.receipts or sink.receipts[-1]['file_saved'] is False


@pytest.mark.parametrize('obstacle', ['existing', 'leaf_symlink', 'parent_symlink', 'missing_parent', 'write_error'])
def test_filesystem_failure_preserves_existing_data_and_reports_partial_state(exported, tmp_path, obstacle):
    path = tmp_path / 'close.json'
    original = tmp_path / 'original'
    original.write_bytes(b'never replace me')
    if obstacle == 'existing':
        path.write_bytes(b'existing')
    elif obstacle == 'leaf_symlink':
        path.symlink_to(original)
    elif obstacle == 'parent_symlink':
        (tmp_path / 'alias').symlink_to(tmp_path, target_is_directory=True)
        path = tmp_path / 'alias' / 'close.json'
    elif obstacle == 'missing_parent':
        path = tmp_path / 'absent' / 'close.json'
    sink = sink_for(exported, path)
    with patch.object(delivery.os, 'link', side_effect=OSError('secret local path')) if obstacle == 'write_error' else patch.object(delivery, '_fail', wraps=delivery._fail):
        sink.consume(exported[2])
    receipt = sink.receipts[-1]
    assert receipt['artifact_prepared'] and not receipt['file_saved']
    assert 'secret local path' not in json.dumps(receipt)
    assert original.read_bytes() == b'never replace me'
    if obstacle == 'existing':
        assert path.read_bytes() == b'existing'
    assert not list(tmp_path.glob('.kassiber-export-*'))


def test_local_destination_requires_exact_once_consent_and_never_enters_chat_arguments(exported, tmp_path):
    task, consent, result = exported
    path = tmp_path / 'private-close.json'
    for answer in ('n\n', 'c\n'):
        sink = delivery.Delivery([(task['id'], 'export_close', str(path))])
        sent, output, _ = run_consent(consent, answer=answer, _accounting_delivery=sink)
        assert str(path) in output and str(path) not in json.dumps(sent)
        sink.consume(result)
        assert not path.exists()
    sink = delivery.Delivery([(task['id'], 'export_close', str(path))])
    sent, output, _ = run_consent(consent, _accounting_delivery=sink, yes=True)
    assert sent['args']['decision'] == 'allow_once' and 'plaintext' in output
    sink.consume(result)
    assert path.exists()
    args = build_parser().parse_args(['chat', '--accounting-export', task['id'], 'export_close', str(path), '--prompt', 'Continue'])
    args._accounting_delivery = sink
    assert str(path) not in json.dumps(chat._build_chat_args(args, []))
    assert str(path) not in json.dumps(diagnostics._argument_summary(args))


def test_transcript_and_machine_stream_drop_artifact_while_local_sink_receives_it(exported, tmp_path):
    _, consent, result = exported
    event = dict(kind='ai.chat.tool_result', request_id='request', data=result)
    client = chat._DaemonChatClient.__new__(chat._DaemonChatClient)
    client._read_timeout_seconds, client._stdout_queue, client._transcript = 1, queue.Queue(), io.StringIO()
    client._stdout_queue.put(json.dumps(event))
    assert delivery.SIDEBAND in client.read()['data']
    assert delivery.SIDEBAND not in client._transcript.getvalue()
    class Client:
        def __init__(self):
            self.events = iter([dict(kind='ai.chat.tool_consent_required', data=consent), event,
                               dict(kind='ai.chat.delta', data={'delta': {'content': 'Untrusted assistant claim'}}),
                               dict(kind='ai.chat', data={'finish_reason': 'stop'})])
        def send(self, record):
            if record['kind'] == 'ai.chat':
                self.request = record['request_id']
        def read(self):
            return dict(next(self.events), request_id=self.request)
    path = tmp_path / 'saved.json'
    sink = delivery.Delivery([(exported[0]['id'], 'export_close', str(path))])
    args = SimpleNamespace(model='fake', _accounting_delivery=sink)
    output, raw = io.StringIO(), io.StringIO()
    answer = chat._run_turn(Client(), args, [], stdin=Tty('y\n'), out=output, chrome=output, render=True, stream_out=raw)
    assert path.exists()
    assert output.getvalue().index('Untrusted assistant claim') < output.getvalue().index('LOCAL EXPORT RECEIPT (not model text)')
    assert delivery.SIDEBAND not in raw.getvalue()
    assert 'synthetic-private-ledger-marker' not in raw.getvalue()
    assert result[delivery.SIDEBAND]['sha256'] not in raw.getvalue()
    assert str(path) not in json.dumps(answer.tool_calls)


@pytest.mark.parametrize('decision', ['deny', 'allow_once'])
def test_real_tool_loop_releases_only_after_consent_and_never_to_provider(exported, book, decision):
    conn, profile, root = book
    task = exported[0]
    with patch.object(daemon, '_record_ai_tool_usage', wraps=daemon._record_ai_tool_usage) as usage, \
         patch.object(daemon, '_update_review_checkpoint', wraps=daemon._update_review_checkpoint) as checkpoint:
        run = agent_run(conn, profile, root, task_steps(task['id'], 'export_close'), decisions=[decision])
    releases = [event['data'][delivery.SIDEBAND] for event in run['events'] if delivery.SIDEBAND in event.get('data', {})]
    assert len(releases) == int(decision == 'allow_once')
    serialized = json.dumps(run['results'])
    serialized += json.dumps([call.args[-1] for call in usage.call_args_list + checkpoint.call_args_list])
    assert delivery.SIDEBAND not in serialized and 'snapshot_json' not in serialized
    assert 'synthetic-private-ledger-marker' not in serialized
    if releases:
        assert releases[0]['sha256'] not in serialized


@pytest.mark.parametrize('change', ['stale', 'cancel', 'expired'])
def test_changed_export_approval_never_releases_bytes(exported, book, change):
    conn, profile, _ = book
    task = exported[0]
    grants = adapter.TaskApprovals()
    preview = adapter.execute(conn, profile, 'ui.accounting.task_preview', dict(task_id=task['id'], step='export_close'), grants)
    args = dict(task_id=task['id'], approval_id=preview['approval_id'], idempotency_key='new-export')
    if change == 'cancel':
        tasks.execute(conn, profile, 'task-cancel', dict(task_id=task['id'], reason='stop'))
    elif change == 'expired':
        grants.pending[preview['approval_id']]['expires'] = 0
    else:
        grants.pending[preview['approval_id']]['expected_digest'] = '0' * 64
    released = {}
    with pytest.raises(AppError):
        adapter.execute(conn, profile, 'ui.accounting.task_apply', args, grants, local_export=released)
    assert not released


@pytest.mark.parametrize('mutation', ['valid', 'report', 'html', 'stale'])
def test_real_tax_json_preserves_rendering_and_verifies_both_commitments(book, tmp_path, mutation):
    from kassiber.core.accounting import jurisdiction, tax_workpapers
    from tests.test_accounting_tax_workpapers import complete_patch
    conn, profile, _ = book
    paper = tax_workpapers.create_workpaper(conn, profile, period_id='2025', pack_id=jurisdiction.AT_PACK_ID, idempotency_key='tax')
    tax_workpapers.review_workpaper(conn, profile, workpaper_id=paper['id'], expected_revision=1,
        patch=complete_patch(), reason='Synthetic local review', idempotency_key='review')
    task = tasks.execute(conn, profile, 'task-create', dict(period_id='2025', statement_ids=[],
        tax_workpaper_id=paper['id'], idempotency_key='tax-task'))
    approve(conn, profile, task['id'], 'close')
    approve(conn, profile, task['id'], 'tax_finalize')
    grants = adapter.TaskApprovals()
    preview = adapter.execute(conn, profile, 'ui.accounting.task_preview', dict(task_id=task['id'], step='export_tax'), grants)
    args = dict(task_id=task['id'], approval_id=preview['approval_id'], idempotency_key='export')
    consent = dict(accounting_task_preview=adapter.consent_preview(conn, profile, args, grants))
    sideband = {}
    adapter.execute(conn, profile, 'ui.accounting.task_apply', args, grants, local_export=sideband)
    artifact = json.loads(sideband['artifact_json'])
    assert '<table>' in artifact['html'] and artifact['verification_contract']
    assert artifact['report']['filed'] is False
    if mutation == 'report':
        artifact['report_json'] += ' '
    elif mutation == 'html':
        artifact['html'] += 'tampered'
    elif mutation == 'stale':
        artifact['stale'] = True
    sideband['artifact_json'] = json.dumps(artifact)
    sideband['sha256'] = hashlib.sha256(sideband['artifact_json'].encode()).hexdigest()
    path = tmp_path / 'tax.json'
    sink = delivery.Delivery([(task['id'], 'export_tax', str(path))])
    sink.approve('tax-call', consent)
    sink.consume(dict(call_id='tax-call', ok=True, **{delivery.SIDEBAND: sideband}))
    assert path.exists() == (mutation == 'valid')
    assert sink.receipts[-1]['file_verified'] == (mutation == 'valid')


def test_locked_main_thread_refuses_export_without_release(exported, book):
    from tests.test_accounting_agent_tasks import runtime_for
    conn, profile, root = book
    task = exported[0]
    runtime = runtime_for(conn, profile, root)
    preview = adapter.execute(conn, profile, 'ui.accounting.task_preview', dict(task_id=task['id'], step='export_close'), runtime.accounting_task_approvals)
    call = daemon.ParsedAiToolCall('locked', 'ui.accounting.task_apply', dict(task_id=task['id'],
        approval_id=preview['approval_id'], idempotency_key='locked-export'))
    def locked(_runtime, callback):
        response = queue.Queue()
        runtime.main_thread_tasks.put(daemon._DaemonMainThreadTask(callback=callback, response=response, request_id=None))
        daemon._drain_daemon_main_thread_tasks(SimpleNamespace(conn=None, main_thread_tasks=runtime.main_thread_tasks))
        ok, value = response.get_nowait()
        assert not ok
        raise value
    with patch.object(daemon, '_run_on_daemon_main_thread', side_effect=locked):
        result = daemon._execute_mutating_ai_tool(call, runtime)
    assert result['reason'] == 'passphrase_required'
    assert delivery.SIDEBAND not in result


def test_oversized_sideband_leaves_prepared_receipt_without_releasing_bytes(exported, book):
    conn, profile, _ = book
    task = exported[0]
    grants = adapter.TaskApprovals()
    preview = adapter.execute(conn, profile, 'ui.accounting.task_preview', dict(task_id=task['id'], step='export_close'), grants)
    args = dict(task_id=task['id'], approval_id=preview['approval_id'], idempotency_key='oversized-export')
    released = {}
    with patch.object(adapter, 'MAX_LOCAL_EXPORT_BYTES', 32):
        result = adapter.execute(conn, profile, 'ui.accounting.task_apply', args, grants, local_export=released)
    assert result['delivery_code'] == 'accounting_export_too_large'
    assert not released and result['file_saved'] is False
    assert tasks.get(conn, profile, task['id'])['receipts'][-1]['result']['artifact_state'] == 'prepared'


def test_cancelled_transport_reports_unknown_preparation_and_requires_fresh_approval(exported, tmp_path):
    path = tmp_path / 'cancelled.json'
    sink = sink_for(exported, path)
    out = io.StringIO()
    sink.render(out)
    assert 'accounting_export_not_delivered' in out.getvalue()
    assert '"artifact_prepared": null' in out.getvalue()
    sink.consume(exported[2])
    assert not path.exists() and not sink.pending


@pytest.mark.parametrize('stage', ['directory_fsync', 'temporary_cleanup'])
def test_post_publication_failure_reports_unknown_saved_state_and_safe_retry(exported, tmp_path, stage):
    path = tmp_path / 'partial.json'
    sink = sink_for(exported, path)
    if stage == 'directory_fsync':
        failure = patch.object(delivery.os, 'fsync', side_effect=[None, OSError('directory durability unknown')])
    else:
        failure = patch.object(delivery.os, 'unlink', side_effect=OSError('temporary cleanup failed'))
    with failure:
        sink.consume(exported[2])
    assert path.exists()
    assert sink.receipts[-1]['file_saved'] is None and sink.receipts[-1]['may_exist'] is True
    sink.approve('retry', exported[1])
    sink.consume({**exported[2], 'call_id': 'retry'})
    assert sink.receipts[-1]['file_saved'] is True and sink.receipts[-1]['file_verified'] is True


@contextmanager
def queued_runtime(book, *, connection=None):
    from tests.test_accounting_agent_tasks import runtime_for
    conn, profile, root = book
    runtime = runtime_for(conn, profile, root)
    def schedule(_runtime, callback):
        response = queue.Queue()
        runtime.main_thread_tasks.put(daemon._DaemonMainThreadTask(callback=callback, response=response, request_id=None))
        daemon._drain_daemon_main_thread_tasks(SimpleNamespace(conn=connection or conn, main_thread_tasks=runtime.main_thread_tasks))
        ok, result = response.get_nowait()
        if not ok:
            raise result
        return result
    with patch.object(daemon, '_run_on_daemon_main_thread', side_effect=schedule):
        yield runtime


def queued_apply(runtime, task_id, step, key):
    preview = daemon._execute_read_only_ai_tool(daemon.ParsedAiToolCall('preview-' + key,
        'ui.accounting.task_preview', dict(task_id=task_id, step=step)), runtime)
    assert preview['ok'], preview
    args = dict(task_id=task_id, approval_id=preview['envelope']['data']['approval_id'], idempotency_key=key)
    assert daemon._ai_accounting_task_consent_preview(runtime, args)['status'] == 'ready'
    return daemon._execute_mutating_ai_tool(daemon.ParsedAiToolCall(key, 'ui.accounting.task_apply', args), runtime)


def test_every_task_step_is_durable_through_actual_main_thread_queue(book):
    from kassiber.core.accounting import bank, evidence, jurisdiction, tax_workpapers
    from tests.test_accounting_tasks import rule
    from tests.test_accounting_tax_workpapers import complete_patch
    conn, profile, root = book
    controls = dict(format='kassiber-bank-control-v1', account_code='bank', statement_id='durable',
        start_date='2025-01-01', end_date='2025-12-31', opening_minor=0, closing_minor=100,
        currency='EUR', minor_unit_exponent=2)
    proof = evidence.retain_evidence(conn, profile, content=json.dumps(controls).encode(),
        name='Synthetic controls', media_type='application/json')
    statement = bank.import_statement(conn, profile, account_code='bank', statement_id='durable',
        start_date='2025-01-01', end_date='2025-12-31', opening_minor=0, closing_minor=100,
        control_evidence_id=proof['id'], control_review_reason='Synthetic verification', control_locator='Entire record',
        csv_text='row_id,date,amount_minor,description\nreceipt,2025-02-01,100,Membership\n')
    rule(conn, profile)
    cancel_task = tasks.execute(conn, profile, 'task-create', dict(period_id='2025', statement_ids=[], idempotency_key='cancel-later'))
    paper = tax_workpapers.create_workpaper(conn, profile, period_id='2025', pack_id=jurisdiction.AT_PACK_ID, idempotency_key='tax')
    review = complete_patch()
    review['field_reviews']['main.660'] = None
    review['mappings'] = [dict(id='income', field_key='main.660', account_code='sales', basis='movement',
        amount_minor=-100, multiplier=-1, reason='Explicit synthetic mapping')]
    tax_workpapers.review_workpaper(conn, profile, workpaper_id=paper['id'], expected_revision=1,
        patch=review, reason='Synthetic review', idempotency_key='review')
    # Create a task selecting the same sources and the reviewed tax working paper.
    task = tasks.execute(conn, profile, 'task-create', dict(period_id='2025', statement_ids=[statement['id']],
        tax_workpaper_id=paper['id'], idempotency_key='durable-task'))
    with queued_runtime(book) as runtime:
        for index, step in enumerate(('prepare', 'post', 'close', 'tax_finalize', 'export_close', 'export_tax'), 1):
            result = queued_apply(runtime, task['id'], step, 'durable-' + step)
            assert result['ok'], result
            assert not conn.in_transaction
            independent = open_encrypted(resolve_database_path(root), 'test-token-placeholder', row_factory=get_row_class())
            try:
                assert independent.execute('SELECT COUNT(*) FROM gl_accounting_task_receipts WHERE task_id=?', (task['id'],)).fetchone()[0] == index
                if index >= 2:
                    assert independent.execute("SELECT COUNT(*) FROM gl_entries WHERE status='posted'").fetchone()[0] >= 1
                if step in ('export_close', 'export_tax'):
                    assert delivery.SIDEBAND in result
            finally:
                independent.close()
        result = daemon._execute_mutating_ai_tool(daemon.ParsedAiToolCall('cancel', 'ui.accounting.task_cancel',
            dict(task_id=cancel_task['id'])), runtime)
        assert result['ok'] and not conn.in_transaction
    independent = open_encrypted(resolve_database_path(root), 'test-token-placeholder', row_factory=get_row_class())
    try:
        assert independent.execute('SELECT COUNT(*) FROM gl_accounting_task_cancellations').fetchone()[0] == 1
        assert independent.execute('SELECT COUNT(*) FROM gl_tax_finals').fetchone()[0] == 1
    finally:
        independent.close()


def test_task_reads_release_only_owned_transactions(exported, book):
    conn, _, _ = book
    with queued_runtime(book) as runtime:
        for inherited in (False, True):
            if inherited:
                conn.execute('BEGIN IMMEDIATE')
                conn.execute('CREATE TABLE caller_uncommitted(value TEXT)')
            get = daemon._execute_read_only_ai_tool(daemon.ParsedAiToolCall('read', 'ui.accounting.task_get',
                dict(task_id=exported[0]['id'])), runtime)
            assert get['ok'] and conn.in_transaction is inherited
            preview = daemon._execute_read_only_ai_tool(daemon.ParsedAiToolCall('preview', 'ui.accounting.task_preview',
                dict(task_id=exported[0]['id'], step='export_close')), runtime)
            args = dict(task_id=exported[0]['id'], approval_id=preview['envelope']['data']['approval_id'], idempotency_key='read')
            assert daemon._ai_accounting_task_consent_preview(runtime, args)['status'] == 'ready'
            assert conn.in_transaction is inherited
            if inherited:
                assert conn.execute("SELECT name FROM sqlite_master WHERE name='caller_uncommitted'").fetchone()
                conn.rollback()


def test_commit_failure_rolls_back_receipt_and_never_releases_export(exported, book):
    conn, _, root = book
    class FailedCommit:
        def __getattr__(self, name):
            return getattr(conn, name)
        def commit(self):
            raise OSError('private financial commit failure marker')
    with queued_runtime(book, connection=FailedCommit()) as runtime:
        result = queued_apply(runtime, exported[0]['id'], 'export_close', 'must-rollback')
    assert result['reason'] == 'accounting_task_commit_failed' and delivery.SIDEBAND not in result
    assert 'private financial' not in json.dumps(result) and not conn.in_transaction
    independent = open_encrypted(resolve_database_path(root), 'test-token-placeholder', row_factory=get_row_class())
    try:
        assert independent.execute("SELECT COUNT(*) FROM gl_accounting_task_receipts WHERE idempotency_key='must-rollback'").fetchone()[0] == 0
    finally:
        independent.close()


def test_structured_close_above_half_megabyte_uses_existing_package_bound(book, tmp_path):
    from kassiber.core.accounting import ledger
    conn, profile, _ = book
    for number in range(250):
        draft = ledger.create_draft(conn, profile, dict(idempotency_key=str(number), period_id='2025', entry_date='2025-02-01',
            description='Synthetic structured annual record ' + 'x' * 1800,
            lines=[dict(account_code='bank', debit_minor=100), dict(account_code='sales', credit_minor=100)]))
        ledger.post_draft(conn, profile, draft_id=draft['id'], expected_digest=draft['payload_digest'])
    task = tasks.execute(conn, profile, 'task-create', dict(period_id='2025', statement_ids=[], idempotency_key='large'))
    approve(conn, profile, task['id'], 'close')
    with queued_runtime(book) as runtime:
        result = queued_apply(runtime, task['id'], 'export_close', 'large-export')
    sideband = result[delivery.SIDEBAND]
    assert 512 * 1024 < len(json.dumps(sideband).encode()) <= adapter.MAX_LOCAL_EXPORT_BYTES
    artifact = json.loads(sideband['artifact_json'])
    consent = dict(accounting_task_preview={'preview': dict(id=task['id'], step='export_close',
        detail={'id': artifact['id'], 'snapshot_digest': artifact['snapshot_digest']})})
    path = tmp_path / 'annual.json'
    sink = delivery.Delivery([(task['id'], 'export_close', str(path))])
    sink.approve('large', consent)
    sink.consume(dict(result, call_id='large'))
    assert path.exists() and sink.receipts[-1]['file_verified'] is True
