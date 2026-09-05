"""Adversarial result-token tests: real SQLCipher commands, no AI/network calls."""
import copy
import json
from types import SimpleNamespace

import pytest

from kassiber import daemon, daemon_accounting_ai as bridge
from kassiber.core.accounting import ai_proposals, document_text, evidence, ledger
from kassiber.core.accounting.ai_context import DisclosureGrants
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401


@pytest.fixture
def result(book, monkeypatch):
    conn, profile, root = book
    source = evidence.retain_evidence(conn, profile, content=b'Invoice 42 Total EUR 10.00', name='Synthetic invoice', media_type='text/plain')
    extraction = document_text.extract(conn, profile, evidence_id=source['id'])
    provider = dict(name='Local', kind='local', base_url='http://127.0.0.1:9999/v1', updated_at='1')
    monkeypatch.setattr(bridge, 'resolve_ai_provider', lambda conn, name: provider)
    ctx = SimpleNamespace(conn=conn, data_root=str(root), ownership_generation='generation-1',
        accounting_ai_grants=DisclosureGrants(), active_ai_chats=daemon.ActiveAiChats(), db_passphrase=None)
    selection = dict(extractions=[dict(id=extraction['id'], pages=[1], fields=[])], include_chart=True, period_ids=['2025'])
    preview = bridge.dispatch(ctx, 'ui.accounting.ai_preview', dict(profile_id=profile, payload=dict(
        provider='Local', model='test-model', selection=selection, question='Suggest a reviewed draft', purpose='draft_entry')))
    validated = daemon._ai_chat_args(dict(provider='Local', model='test-model', messages=[dict(role='user', content='Suggest a reviewed draft')],
        accounting_context=dict(profile_id=profile, token=preview['token'], expected_digest=preview['expected_digest'], confirm=True)))
    disclosed = bridge.prepare(ctx, validated, provider)
    candidate = dict(schema_version=1, explanation='Synthetic review', entries=[dict(period_id='2025', entry_date='2025-02-01',
        description='Synthetic draft', lines=[dict(account_code='bank', debit_minor='1000', credit_minor='0'),
            dict(account_code='sales', debit_minor='0', credit_minor='1000')], evidence_ids=[source['id']])],
        document_reviews=[dict(extraction_id=extraction['id'], fields=dict(document_number='42'), spans=dict(document_number=dict(page=1,start=8,end=10)))])
    token = bridge.buffer_result(ctx, disclosed, json.dumps(candidate))
    conn.commit()
    return ctx, profile, provider, disclosed, candidate, token


def action(result, suffix='preview', **payload):
    ctx, profile, _, _, _, token = result
    return bridge.dispatch(ctx, f'ui.accounting.ai_result_{suffix}', dict(profile_id=profile, payload=dict(result_token=token, **payload)))


def counts(conn):
    return tuple(conn.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]
        for table in ('gl_entries','gl_evidence_field_reviews','gl_ai_proposal_applications'))


def test_result_preview_is_read_only_then_explicit_apply_creates_only_drafts_once(result):
    ctx, profile, _, disclosed, _, token = result
    reviewed = action(result)
    assert counts(ctx.conn) == (0,0,0)
    assert ledger.require_book(ctx.conn, profile)['revision'] == disclosed['binding']['book_revision']
    assert ctx.accounting_ai_grants._results[token]['disclosed'] == {'binding': disclosed['binding']}
    assert reviewed['plan']['entries'][0]['payload']['lines'][0]['debit_minor'] == '1000'
    applied = action(result, 'apply', expected_digest=reviewed['expected_digest'], confirm=True, reason='Human checked every field and line')
    assert applied['posted'] is False and counts(ctx.conn) == (1,1,1)
    assert ctx.conn.execute('SELECT status FROM gl_entries').fetchone()[0] == 'draft'
    assert token not in ctx.accounting_ai_grants._results
    with pytest.raises(AppError) as error:
        action(result, 'apply', expected_digest=reviewed['expected_digest'], confirm=True, reason='Retry')
    assert error.value.code == 'accounting_ai_grant_expired'


@pytest.mark.parametrize('confirmation', [False, None, 1, 'true', [], {}])
def test_apply_requires_exact_true_without_writes_or_token_consumption(result, confirmation):
    reviewed = action(result)
    with pytest.raises(AppError) as error:
        action(result, 'apply', expected_digest=reviewed['expected_digest'], confirm=confirmation, reason='Reviewed')
    assert error.value.code == 'accounting_ai_consent_required'
    assert counts(result[0].conn) == (0,0,0)
    assert result[-1] in result[0].accounting_ai_grants._results


@pytest.mark.parametrize('change', ['book','provider_url','provider_model','provider_ack','ownership','root','scope'])
@pytest.mark.parametrize('suffix', ['preview','apply'])
def test_result_tokens_recheck_scope_provider_and_book_revision(result, change, suffix):
    ctx, profile, provider, _, _, _ = result
    approved = action(result)
    approval = dict(expected_digest=approved['expected_digest'], confirm=True, reason='Reviewed') if suffix == 'apply' else {}
    if change == 'book': ledger.create_account(ctx.conn, profile, code='new', name='New', kind='expense')
    elif change == 'provider_url': provider['base_url'] = 'https://changed.invalid/v1'
    elif change == 'provider_model': provider['updated_at'] = 'new-config'
    elif change == 'provider_ack': provider.update(kind='remote', acknowledged_at=None)
    elif change == 'ownership': ctx.ownership_generation = 'next-generation'
    elif change == 'root': ctx.data_root += '-different-project'
    else:
        with pytest.raises(AppError) as error:
            bridge.dispatch(ctx, f'ui.accounting.ai_result_{suffix}', dict(profile_id='other-book',payload=dict(result_token=result[-1], **approval)))
        assert error.value.code == 'accounting_scope_changed'
        return
    with pytest.raises(AppError):
        action(result, suffix, **approval)
    assert counts(ctx.conn) == (0,0,0)


def test_expiry_and_lock_purge_result_tokens(result, monkeypatch):
    ctx, _, _, disclosed, candidate, token = result
    expires = ctx.accounting_ai_grants._results[token]['expires']
    with monkeypatch.context() as patch:
        patch.setattr('time.monotonic', lambda: expires)
        with pytest.raises(AppError) as error:
            action(result)
        assert error.value.code == 'accounting_ai_grant_expired'
    assert token not in ctx.accounting_ai_grants._results
    new_token = bridge.buffer_result(ctx, disclosed, json.dumps(candidate))
    assert new_token in ctx.accounting_ai_grants._results
    daemon._clear_unlocked_passphrase(ctx)
    assert ctx.accounting_ai_grants._results == {}
    old = ctx.conn
    ctx.conn = None
    try:
        with pytest.raises(AppError) as error:
            bridge.buffer_result(ctx, disclosed, json.dumps(candidate))
        assert error.value.code == 'passphrase_required'
    finally:
        ctx.conn = old


def test_result_digest_failure_and_late_command_failure_roll_back_all_writes(result, monkeypatch):
    ctx, profile, _, disclosed, _, token = result
    reviewed = action(result)
    with pytest.raises(AppError):
        action(result, 'apply', expected_digest='not-reviewed', confirm=True, reason='Reviewed')
    assert counts(ctx.conn) == (0,0,0)
    original = ai_proposals._apply_commands
    calls = 0
    def fail_after_actual_commands(*args, **kwargs):
        nonlocal calls
        calls += 1
        receipt = original(*args, **kwargs)
        if calls == 2:
            raise AppError('Injected after all commands', code='synthetic_failure')
        return receipt
    monkeypatch.setattr(ai_proposals, '_apply_commands', fail_after_actual_commands)
    with pytest.raises(AppError) as error:
        action(result, 'apply', expected_digest=reviewed['expected_digest'], confirm=True, reason='Reviewed')
    assert error.value.code == 'synthetic_failure'
    assert counts(ctx.conn) == (0,0,0)
    assert ledger.require_book(ctx.conn, profile)['revision'] == disclosed['binding']['book_revision']
    assert token in ctx.accounting_ai_grants._results


def test_buffer_is_bounded_and_invalid_model_content_never_creates_a_token(result):
    ctx, _, _, disclosed, candidate, first = result
    for invalid in ('prose only','{}','x'* (ai_proposals.MAX_RESULT_BYTES+1)):
        assert bridge.buffer_result(ctx, disclosed, invalid) is None
    for _ in range(16):
        bridge.buffer_result(ctx, disclosed, json.dumps(candidate))
    assert len(ctx.accounting_ai_grants._results) == 16
    assert first not in ctx.accounting_ai_grants._results
    hostile = copy.deepcopy(candidate)
    hostile['entries'][0]['lines'][0]['debit_minor'] = '1001'
    token = bridge.buffer_result(ctx, disclosed, json.dumps(hostile))
    with pytest.raises(AppError):
        bridge.dispatch(ctx, 'ui.accounting.ai_result_preview', dict(profile_id=result[1],payload=dict(result_token=token)))
    assert counts(ctx.conn) == (0,0,0)


@pytest.mark.parametrize('extra', [{'candidate':{}}, {'profile_id':'other'}, {'post':True}, {'result_token':1}])
def test_result_boundary_rejects_injected_fields_or_wrong_token_types(result, extra):
    ctx, profile, _, _, _, token = result
    with pytest.raises(AppError) as error:
        bridge.dispatch(ctx, 'ui.accounting.ai_result_preview', dict(profile_id=profile,payload={**dict(result_token=token),**extra}))
    assert error.value.code == 'accounting_invalid_fields'
    assert counts(ctx.conn) == (0,0,0)
