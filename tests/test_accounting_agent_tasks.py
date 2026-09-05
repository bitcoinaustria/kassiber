"""Scripted provider transport; real encrypted task/consent/ledger operations."""
import io
import json
import queue
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import daemon, daemon_accounting_tasks as adapter
from kassiber.ai.tools import select_tool_capabilities, get_tool, tool_capabilities
from kassiber.core.accounting import ledger, tasks
from kassiber.db import set_setting
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_tasks import setup, rule


def runtime_for(conn, profile_id, root):
    workspace = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (profile_id,)).fetchone()[0]
    set_setting(conn, 'context_workspace', workspace)
    set_setting(conn, 'context_profile', profile_id)
    conn.commit()
    return daemon.AiToolRuntime(str(root), {}, queue.Queue(),
        {'scope_workspace_id': workspace, 'scope_profile_id': profile_id, 'provider_kind': 'local'})


@pytest.mark.parametrize('decision', ['allow_once', 'deny'])
def test_real_agent_task_review_consent_and_redacted_continuation(book, decision):
    conn, profile_id, root = book
    task, _ = setup(conn, profile_id, 3)
    rule(conn, profile_id)
    runtime = runtime_for(conn, profile_id, root)
    active_chats = daemon.ActiveAiChats()
    _, active = active_chats.register('accounting-agent')
    events, outbound = [], []
    class Desktop(io.StringIO):
        def write(self, text):
            result = super().write(text)
            for line in text.splitlines():
                if not line.strip():
                    continue
                event = json.loads(line)
                events.append(event)
                if event['kind'] == 'ai.chat.tool_consent_required':
                    assert event['data']['accounting_task_preview']['status'] == 'ready'
                    assert active.consent.record(event['data']['call_id'], decision)
            return result
    sequence = ['task_get', 'task_preview', 'task_apply', 'task_get']
    calls = []
    def provider(_rid, _client, _validated, context, offered, _out, _cancel):
        results = [json.loads(item['output']) for item in context.input_items if item.get('type') == 'function_call_output']
        outbound[:] = results
        if len(calls) == len(sequence):
            return daemon.AiToolTurnResult([], 'Task state checked.', '', 'stop', [])
        action = sequence[len(calls)]
        arguments = {'task_id': task['id']}
        if action == 'task_preview':
            arguments['step'] = 'prepare'
        elif action == 'task_apply':
            arguments.update(approval_id=results[-1]['envelope']['data']['approval_id'], idempotency_key='agent-prepare')
        entry = get_tool('ui.accounting.' + action)
        assert entry.provider_name in {tool['name'] for tool in offered}
        calls.append(action)
        return daemon.AiToolTurnResult([{'id': str(len(calls)), 'function': {
            'name': entry.provider_name, 'arguments': json.dumps(arguments)}}], '', '', 'tool_calls', [])
    validated = daemon._ai_chat_args({'model': 'scripted-test', 'tools_enabled': True, 'persist': False,
        'messages': [{'role': 'user', 'content': 'Continue accounting task ' + task['id']}],
        'screen_context': {'route': '/assistant', 'capabilities': ['accounting_tasks']}})
    with patch.object(daemon, '_run_on_daemon_main_thread', side_effect=lambda _, callback: callback(conn)), \
         patch.object(daemon, '_stream_ai_chat_tool_turn', provider):
        daemon._run_ai_chat_tool_loop('accounting-agent', SimpleNamespace(last_provider_session_id=None),
            {'name': 'fake-local', 'kind': 'local', 'base_url': 'http://localhost'}, validated,
            daemon._OutputChannel(Desktop()), active, runtime, active_chats)
    assert calls == sequence
    disclosed = json.dumps(outbound)
    for forbidden in ('Membership', 'Reviewed receipts', 'sales', 'counter_account_code', 'description', 'expected_digest', 'amount_minor', 'proposals'):
        assert forbidden not in disclosed
    previews = [event['data']['accounting_task_preview'] for event in events if event['kind'] == 'ai.chat.tool_consent_required']
    assert len(previews[0]['preview']['proposals']) == 3
    assert 'Membership' in json.dumps(previews[0])
    assert conn.execute('SELECT COUNT(*) FROM gl_entries').fetchone()[0] == (3 if decision == 'allow_once' else 0)
    assert conn.execute("SELECT COUNT(*) FROM gl_entries WHERE status='posted'").fetchone()[0] == 0


def test_review_handles_expire_are_one_use_and_cannot_cross_chat_or_book(book):
    conn, profile, _ = book
    task, _ = setup(conn, profile)
    rule(conn, profile)
    grants = adapter.TaskApprovals()
    preview = adapter.execute(conn, profile, 'ui.accounting.task_preview', {'task_id': task['id'], 'step': 'prepare'}, grants)
    args = {'task_id': task['id'], 'approval_id': preview['approval_id'], 'idempotency_key': 'apply'}
    for destination, authority in [('other', grants), (profile, adapter.TaskApprovals())]:
        with pytest.raises(AppError):
            adapter.execute(conn, destination, 'ui.accounting.task_apply', args, authority)
    adapter.execute(conn, profile, 'ui.accounting.task_apply', args, grants)
    with pytest.raises(AppError):
        adapter.execute(conn, profile, 'ui.accounting.task_apply', args, grants)
    assert conn.execute('SELECT COUNT(*) FROM gl_entries').fetchone()[0] == 1
    preview = adapter.execute(conn, profile, 'ui.accounting.task_preview', {'task_id': task['id'], 'step': 'post'}, grants)
    grants.pending[preview['approval_id']]['expires'] = 0
    with pytest.raises(AppError):
        adapter.consent_preview(conn, profile, {'task_id': task['id'], 'approval_id': preview['approval_id']}, grants)


def test_stale_preview_and_provider_errors_never_disclose_financial_fields(book):
    conn, profile, _ = book
    task, _ = setup(conn, profile)
    rule(conn, profile)
    grants = adapter.TaskApprovals()
    prepared = adapter.execute(conn, profile, 'ui.accounting.task_preview', {'task_id': task['id'], 'step': 'prepare'}, grants)
    ledger.create_account(conn, profile, code='new', name='Sensitive counterparty', kind='expense')
    with pytest.raises(AppError):
        adapter.consent_preview(conn, profile, {'task_id': task['id'], 'approval_id': prepared['approval_id']}, grants)
    with patch.object(tasks, 'execute', side_effect=AppError('Sensitive counterparty', code='accounting_task_failed', details={'amount': '999999'})):
        with pytest.raises(AppError) as exc:
            adapter.execute(conn, profile, 'ui.accounting.task_get', {'task_id': task['id']}, grants)
    assert 'Sensitive' not in str(exc.value)
    assert not exc.value.details


def test_task_tools_are_specialist_only_and_once_only():
    assert tool_capabilities(get_tool('ui.accounting.task_get')) == {'accounting_tasks'}
    assert 'accounting_tasks' not in select_tool_capabilities([{'role': 'user', 'content': 'Show my portfolio'}])
    assert {'ui.accounting.task_apply', 'ui.accounting.task_cancel'} <= daemon.AI_TOOL_ONCE_ONLY_CONSENT
