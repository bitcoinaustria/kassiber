"""Agent-only privacy and consent boundaries above deterministic task sources."""
import json

import pytest

from kassiber import daemon_accounting_tasks as adapter
from kassiber.ai.tools import get_tool
from kassiber.core.accounting import tasks
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_projection import prepared
from tests.test_accounting_task_amendments import amendment
from tests.test_accounting_task_projection import task_for
from tests.test_accounting_tasks import approve, reviewed_document, rule, setup


def test_local_evidence_amendment_stales_existing_agent_consent(book):
    conn, profile, _ = book
    task, _ = setup(conn, profile)
    rule(conn, profile)
    approve(conn, profile, task['id'], 'prepare')
    grants = adapter.TaskApprovals()
    old = adapter.execute(conn, profile, 'ui.accounting.task_preview', {'task_id': task['id'], 'step': 'post'}, grants)
    document, _ = reviewed_document(conn, profile)
    tasks.execute(conn, profile, 'task-amend', amendment(conn, profile, task, document))
    with pytest.raises(AppError) as stale:
        adapter.consent_preview(conn, profile, {'task_id': task['id'], 'approval_id': old['approval_id']}, grants)
    assert stale.value.code == 'accounting_stale_approval'


def test_cli_amendment_keeps_evidence_private_and_grants_no_agent_scope_write(book):
    from tests.test_accounting_cli_tasks import cli
    conn, profile, root = book
    task, _ = setup(conn, profile)
    document, _ = reviewed_document(conn, profile)
    payload = amendment(conn, profile, task, document)
    conn.commit()
    cli(root, 'task-amend', payload)
    current = cli(root, 'task-get', {'task_id': task['id']})['data']
    redacted = json.dumps(adapter.summary(current))
    for private in (document['id'], document['content_sha256'], document['name'], payload['reason']):
        assert private not in redacted
    assert get_tool('ui.accounting.task_amend') is None
    assert get_tool('ui.accounting.task_amend_preview') is None


def test_projection_selection_stays_private_and_assignment_is_not_an_agent_tool(book):
    conn, profile, args = prepared(book)
    task = task_for(conn, profile, args)
    redacted = json.dumps(adapter.summary(task))
    for private in (args['event_id'], args['artifact_id'], args['binding_id'], args['policy_id']):
        assert private not in redacted
    assert get_tool('ui.accounting.task_projection_assign_preview') is None
