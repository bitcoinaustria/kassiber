"""AF-5: fake provider/UI transport, real private-book tools and accounting.

The scripted provider tests protocol orchestration, not model reasoning quality.
No domain functions, consent queue, portfolio calculation or persistence are mocked.
"""

import io
import json
import queue
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import daemon
from kassiber.db import open_db, resolve_database_path, set_setting


@pytest.fixture
def private_book(tmp_path):
    conn = open_db(tmp_path)
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','Private','2025')")
    conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','Private','EUR','2025')")
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wallet','w','p','Private wallet','custom','2025')")
    for identity, kind in [('acquisition', 'buy'), ('ambiguous', 'receive')]:
        conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,
            occurred_at,direction,asset,amount,fee,kind,raw_json,created_at)
            VALUES(?,'w','p','wallet',?,'2025-01-01T12:00:00Z','inbound','BTC',100000000000,0,?,'{}','2025')''',
            (identity, identity, kind))
    set_setting(conn, 'context_workspace', 'w')
    set_setting(conn, 'context_profile', 'p')
    conn.commit()
    runtime = daemon.AiToolRuntime(data_root=str(tmp_path), runtime_config={}, main_thread_tasks=queue.Queue(),
        maintenance_state={'scope_workspace_id': 'w', 'scope_profile_id': 'p', 'provider_kind': 'local'})
    # Only replace the cross-thread scheduling shim; execute every callback on
    # the real connection, including the actual scoped tool dispatch guards.
    with patch.object(daemon, '_run_on_daemon_main_thread', side_effect=lambda runtime, callback: callback(conn)):
        yield conn, runtime, tmp_path
    conn.close()


class _ConsentingDesktop(io.StringIO):
    """Simulate UI consent via the real pending-call queue, not a patched wait."""

    def __init__(self, active, decision, interrupt_after_apply=False):
        super().__init__()
        self.active = active
        self.decision = decision
        self.interrupt_after_apply = interrupt_after_apply
        self.events = []

    def write(self, text):
        result = super().write(text)
        for line in text.splitlines():
            if not line.strip():
                continue
            event = json.loads(line)
            self.events.append(event)
            if event['kind'] == 'ai.chat.tool_consent_required':
                assert self.active.consent.record(event['data']['call_id'], self.decision)
            if (self.interrupt_after_apply and event['kind'] == 'ai.chat.tool_result'
                    and event['data'].get('envelope', {}).get('kind') == 'ui.review.apply'):
                assert event['data']['ok']
                self.active.cancel_event.set()
        return result


def _conversation(runtime, steps, *, decision='allow_once', interrupt=False):
    chats = daemon.ActiveAiChats()
    _, active = chats.register('private-workflow')
    output = _ConsentingDesktop(active, decision, interrupt)
    seen = []
    delivered = []
    validated = daemon._ai_chat_args({'model': 'scripted-test-provider', 'tools_enabled': True,
        'messages': [{'role': 'user', 'content': (
            'Review quarantine. I verified acquisition at EUR 20000 per BTC. '
            'Do not guess the ambiguous receipt. Ask before changes, then show my portfolio.')}],
        'persist': False})

    def provider_transport(_rid, _client, _validated, context, offered, _out, _cancel):
        results = [json.loads(item['output']) for item in context.input_items
                   if item.get('type') == 'function_call_output']
        delivered[:] = results
        if len(seen) == len(steps):
            return daemon.AiToolTurnResult([], 'The ambiguous receipt still needs evidence.', '', 'stop', [])
        name, arguments = steps[len(seen)]
        arguments = arguments(results) if callable(arguments) else arguments
        entry = daemon.get_tool(name)
        assert entry.provider_name in {tool['name'] for tool in offered}
        seen.append((name, arguments))
        call = {'id': f'private-{len(seen)}', 'function': {
            'name': entry.provider_name, 'arguments': json.dumps(arguments)}}
        return daemon.AiToolTurnResult([call], '', '', 'tool_calls', [])

    with patch.object(daemon, '_stream_ai_chat_tool_turn', provider_transport):
        daemon._run_ai_chat_tool_loop('private-workflow', SimpleNamespace(last_provider_session_id=None),
            {'name': 'scripted-local-provider', 'kind': 'local', 'base_url': 'http://localhost'},
            validated, daemon._OutputChannel(output), active, runtime, chats)
    return output.events, seen, delivered


def _review_steps():
    return [
        ('ui.review.cases', {'limit': 100}),
        ('ui.review.plan', lambda results: {
            'expected_input_version': results[-1]['envelope']['data']['input_version'],
            'operations': [{'type': 'price_override', 'transaction_id': 'acquisition',
                            'fiat_rate': '20000', 'reason': 'User verified acquisition evidence'}]}),
        ('ui.review.apply', lambda results: {
            'artifact': results[-1]['envelope']['data'], 'idempotency_key': 'private-correction'}),
        ('ui.reports.portfolio_summary', {}),
    ]


def _assert_private(conn, root):
    assert conn.execute('SELECT COUNT(*) FROM gl_books').fetchone()[0] == 0
    assert conn.execute('SELECT COUNT(*) FROM gl_entries').fetchone()[0] == 0
    assert conn.execute('SELECT COUNT(*) FROM ai_chat_messages').fetchone()[0] == 0
    assert resolve_database_path(root).read_bytes()[:16] == b'SQLite format 3\x00'
    ambiguous = conn.execute("SELECT excluded,fiat_rate_exact FROM transactions WHERE id='ambiguous'").fetchone()
    assert not ambiguous['excluded']
    assert ambiguous['fiat_rate_exact'] is None
    assert conn.execute("SELECT 1 FROM journal_quarantines WHERE transaction_id='ambiguous'").fetchone()


@pytest.mark.parametrize('decision', ['allow_once', 'deny'])
def test_private_review_consent_then_real_portfolio_preserves_ambiguous_case(private_book, decision):
    conn, runtime, root = private_book
    events, seen, delivered = _conversation(runtime, _review_steps(), decision=decision)
    assert [name for name, _ in seen] == [name for name, _ in _review_steps()]
    consent = [event for event in events if event['kind'] == 'ai.chat.tool_consent_required']
    assert len(consent) == 1
    assert consent[0]['data']['review_preview']['status'] == 'ready'
    assert delivered[-1]['ok']
    portfolio = delivered[-1]['envelope']['data']
    approved = decision == 'allow_once'
    assert conn.execute('SELECT COUNT(*) FROM review_workflow_receipts').fetchone()[0] == int(approved)
    assert conn.execute('SELECT COUNT(*) FROM transaction_edit_events').fetchone()[0] == int(approved)
    if approved:
        assert delivered[2]['envelope']['data']['verification']['quarantine_count'] == 1
        assert not delivered[2]['envelope']['data']['verification']['report_ready']
        assert portfolio['rows'][0]['quantity_msat'] == 100000000000
        assert portfolio['rows'][0]['cost_basis'] == 20000
    else:
        assert delivered[2] == {'ok': False, 'reason': 'user_denied'}
        assert portfolio['rows'] == []
        assert conn.execute("SELECT fiat_rate_exact FROM transactions WHERE id='acquisition'").fetchone()[0] is None
    assert events[-1]['kind'] == 'ai.chat'
    assert events[-1]['data']['finish_reason'] == 'stop'
    _assert_private(conn, root)


def test_interrupted_private_review_resumes_from_receipt_without_duplicate_state(private_book):
    conn, runtime, root = private_book
    interrupted, seen, _ = _conversation(runtime, _review_steps(), interrupt=True)
    assert interrupted[-1]['data']['finish_reason'] == 'cancelled'
    assert [name for name, _ in seen] == ['ui.review.cases', 'ui.review.plan', 'ui.review.apply']
    original_receipt = json.loads(conn.execute('SELECT receipt_json FROM review_workflow_receipts').fetchone()[0])
    version = conn.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
    # New ordinary turn asks for the durable receipt before retrying the exact
    # same uncertain operation; old consent is not carried into the new chat.
    resumed, _, delivered = _conversation(runtime, [
        ('ui.review.receipt', {'idempotency_key': 'private-correction'}),
        ('ui.review.apply', seen[2][1]),
        ('ui.reports.portfolio_summary', {}),
    ])
    assert delivered[0]['envelope']['data']['id'] == original_receipt['id']
    assert delivered[1]['envelope']['data']['id'] == original_receipt['id']
    approvals = [event for event in resumed if event['kind'] == 'ai.chat.tool_consent_required']
    assert len(approvals) == 1
    assert approvals[0]['data']['review_preview']['status'] == 'applied'
    assert conn.execute('SELECT COUNT(*) FROM review_workflow_receipts').fetchone()[0] == 1
    assert conn.execute('SELECT COUNT(*) FROM transaction_edit_events').fetchone()[0] == 1
    assert conn.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0] == version
    assert delivered[-1]['envelope']['data']['rows'][0]['cost_basis'] == 20000
    _assert_private(conn, root)
