"""Actual Bitcoin task previews through the interactive CLI approval boundary."""
import copy

import pytest

from kassiber import daemon_accounting_tasks as adapter
from kassiber.core.accounting import tasks
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_projection import prepared
from tests.test_accounting_task_projection import task_for
from tests.test_cli_accounting_consent import run_consent


def reviewed_step(conn, profile, task, grants, step):
    preview = adapter.execute(conn, profile, 'ui.accounting.task_preview', dict(task_id=task['id'], step=step), grants)
    arguments = dict(task_id=task['id'], approval_id=preview['approval_id'], idempotency_key=step)
    return dict(call_id=step, name='ui.accounting.task_apply', needs_consent=True, arguments_preview=arguments,
        accounting_task_preview=adapter.consent_preview(conn, profile, arguments, grants))


def apply_after_actual_cli_consent(conn, profile, grants, data, answer='y\n'):
    sent, output, _ = run_consent(data, answer=answer, yes=True)
    if sent.get('args', {}).get('decision') == 'allow_once':
        adapter.execute(conn, profile, 'ui.accounting.task_apply', data['arguments_preview'], grants)
    return sent, output


@pytest.mark.parametrize('quantity', [1, 100_000_000_000])
@pytest.mark.parametrize('step', ['prepare', 'post'])
@pytest.mark.parametrize('answer', ['y\n', 'n\n'])
def test_bitcoin_real_cli_once_consent_and_denial(book, quantity, step, answer):
    conn, profile, request = prepared(book, quantity=quantity)
    task = task_for(conn, profile, request)
    grants = adapter.TaskApprovals()
    if step == 'post':
        sent, _ = apply_after_actual_cli_consent(conn, profile, grants, reviewed_step(conn, profile, task, grants, 'prepare'))
        assert sent['args']['decision'] == 'allow_once'
    data = reviewed_step(conn, profile, task, grants, step)
    sent, output = apply_after_actual_cli_consent(conn, profile, grants, data, answer)
    allowed = answer == 'y\n'
    assert sent['args']['decision'] == ('allow_once' if allowed else 'deny')
    assert 'quantitative_posting' in output and 'basis_exact' in output
    assert request['event_id'] in output and request['binding_id'] in output
    assert '"period_id": "2025"' in output
    assert conn.execute('SELECT COUNT(*) FROM gl_projection_proposals').fetchone()[0] == int(allowed or step == 'post')
    assert conn.execute('SELECT COUNT(*) FROM gl_projection_publications').fetchone()[0] == int(allowed and step == 'post')
    if quantity == 1:
        assert conn.execute('SELECT COUNT(*) FROM gl_entries').fetchone()[0] == 0


@pytest.mark.parametrize('step', ['prepare', 'post'])
@pytest.mark.parametrize('damage', ['missing', 'request', 'period', 'event', 'quantity', 'basis', 'rounding', 'policy', 'line', 'empty_lines', 'extra'])
def test_malformed_bitcoin_effects_are_not_blanket_approved(book, step, damage):
    conn, profile, request = prepared(book)
    task = task_for(conn, profile, request)
    grants = adapter.TaskApprovals()
    if step == 'post':
        apply_after_actual_cli_consent(conn, profile, grants, reviewed_step(conn, profile, task, grants, 'prepare'))
    data = copy.deepcopy(reviewed_step(conn, profile, task, grants, step))
    preview = data['accounting_task_preview']['preview']
    row = preview['proposals'][0] if step == 'prepare' else preview['detail']['projections'][0]
    projected = row['projection']
    if damage == 'missing':
        row.pop('projection')
    elif damage == 'request':
        projected['request']['category'] = 'arbitrary_action'
    elif damage == 'period':
        projected['request']['period_id'] = '2026'
    elif damage == 'event':
        projected['request']['event_id'] = 'other-source'
    elif damage == 'quantity':
        projected['quantitative_posting']['quantity_msat'] = 0.5
    elif damage == 'basis':
        projected['quantitative_posting']['basis_exact'] = 'NaN'
    elif damage == 'rounding':
        projected['quantitative_posting']['currency_rounding'] = []
    elif damage == 'policy':
        projected['policy_digest'] = 'missing'
    elif damage == 'line':
        projected['lines'][0]['debit_minor'] = 0.1
    elif damage == 'empty_lines':
        projected['lines'] = []
    else:
        projected['unreviewed_payload'] = {'amount': '1000'}
    sent, output, _ = run_consent(data, answer='y\n', yes=True)
    assert sent['args']['decision'] == 'deny'
    assert 'complete, current local preview is unavailable' in output
    assert request['event_id'] not in output
    assert conn.execute('SELECT COUNT(*) FROM gl_projection_publications').fetchone()[0] == 0


def test_missing_post_identity_and_empty_non_projection_post_are_denied(book):
    conn, profile, request = prepared(book, quantity=1)
    task = task_for(conn, profile, request)
    grants = adapter.TaskApprovals()
    apply_after_actual_cli_consent(conn, profile, grants, reviewed_step(conn, profile, task, grants, 'prepare'))
    original = reviewed_step(conn, profile, task, grants, 'post')
    for missing in ('proposal_id', 'proposal_digest', 'artifact_digest'):
        data = copy.deepcopy(original)
        data['accounting_task_preview']['preview']['detail']['projections'][0].pop(missing)
        assert run_consent(data)[0]['args']['decision'] == 'deny'
    data = copy.deepcopy(original)
    data['accounting_task_preview']['preview']['detail']['projections'] = []
    assert run_consent(data)[0]['args']['decision'] == 'deny'
