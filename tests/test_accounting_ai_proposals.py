import json

import pytest

from kassiber.core.accounting import ai_proposals as proposals, document_text, evidence, ledger
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401


@pytest.fixture
def suggested(book):
    conn, profile, _ = book
    source = evidence.retain_evidence(conn, profile, content=b"Invoice 42 Total EUR 10.00", media_type="text/plain", name="Invoice")
    extraction = document_text.extract(conn, profile, evidence_id=source['id'])
    binding = {'profile_id': profile, 'book_revision': ledger.require_book(conn, profile)['revision'],
        'provider': {'name':'synthetic', 'model':'test', 'kind':'local'},
        'selection': {'extractions':[{'id':extraction['id'],'pages':[1],'fields':[]}],
            'include_chart':True,'period_ids':['2025']}}
    candidate = {'schema_version':1,'explanation':'Synthetic example for review',
        'entries':[{'period_id':'2025','entry_date':'2025-02-01','description':'Reviewed synthetic sale',
            'lines':[{'account_code':'bank','debit_minor':'1000','credit_minor':'0'},
                     {'account_code':'sales','debit_minor':'0','credit_minor':'1000'}], 'evidence_ids':[source['id']]}],
        'document_reviews':[{'extraction_id':extraction['id'],'fields':{'document_number':'42'},
            'spans':{'document_number':{'page':1,'start':8,'end':10}}}]}
    return conn, profile, binding, candidate


def test_preview_uses_normal_guards_without_surviving_writes(suggested):
    conn, profile, binding, candidate = suggested
    reviewed = proposals.preview(conn, profile, candidate=candidate, binding=binding)
    assert reviewed['plan']['entries'][0]['payload']['lines'][0]['debit_minor'] == '1000'
    for table in ('gl_entries','gl_evidence_field_reviews','gl_ai_proposal_applications'):
        assert conn.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0] == 0
    assert ledger.require_book(conn, profile)['revision'] == binding['book_revision']
    applied = proposals.apply(conn, profile, candidate=candidate, binding=binding,
        expected_digest=reviewed['expected_digest'], reason='Human checked source and both entry lines')
    assert applied['posted'] is False
    assert len(applied['draft_ids']) == len(applied['document_review_ids']) == 1
    assert ledger._entry(conn, profile, applied['draft_ids'][0])['status'] == 'draft'
    assert conn.execute('SELECT COUNT(*) FROM gl_ai_proposal_applications').fetchone()[0] == 1
    with pytest.raises(Exception, match='accounting_ai_proposal_retained'):
        conn.execute('DELETE FROM gl_ai_proposal_applications')


@pytest.mark.parametrize('tamper', ['balance','account','period','evidence','page','span','amount','extra'])
def test_hostile_model_output_cannot_bypass_commands_or_disclosed_sources(suggested, tamper):
    conn, profile, binding, candidate = suggested
    if tamper == 'balance': candidate['entries'][0]['lines'][0]['debit_minor'] = '1001'
    elif tamper == 'account': candidate['entries'][0]['lines'][0]['account_code'] = 'missing'
    elif tamper == 'period': candidate['entries'][0]['period_id'] = '2030'
    elif tamper == 'evidence': candidate['entries'][0]['evidence_ids'] = ['unselected']
    elif tamper == 'page': candidate['document_reviews'][0]['spans']['document_number']['page'] = 2
    elif tamper == 'span': candidate['document_reviews'][0]['spans']['document_number']['end'] = 999
    elif tamper == 'amount': candidate['entries'][0]['lines'][0]['debit_minor'] = True
    else: candidate['entries'][0]['post_immediately'] = True
    with pytest.raises(AppError):
        proposals.preview(conn, profile, candidate=candidate, binding=binding)
    assert ledger.require_book(conn, profile)['revision'] == binding['book_revision']
    assert conn.execute('SELECT COUNT(*) FROM gl_entries').fetchone()[0] == 0


def test_changed_payload_and_book_revision_revoke_application(suggested):
    conn, profile, binding, candidate = suggested
    preview = proposals.preview(conn, profile, candidate=candidate, binding=binding)
    candidate['entries'][0]['description'] = 'Changed after review'
    with pytest.raises(AppError) as exc:
        proposals.apply(conn, profile, candidate=candidate, binding=binding, expected_digest=preview['expected_digest'], reason='Reviewed')
    assert exc.value.code == 'accounting_stale_approval'
    ledger.create_account(conn, profile, code='later', name='Later', kind='asset')
    with pytest.raises(AppError):
        proposals.preview(conn, profile, candidate=candidate, binding=binding)


def test_unstructured_or_oversized_result_is_explanation_only(suggested):
    _, _, _, candidate = suggested
    assert proposals.decode_result(json.dumps(candidate)) == candidate
    for content in ('ordinary explanation', '{}', 'x' * (proposals.MAX_RESULT_BYTES + 1), '{"schema_version":true}'):
        assert proposals.decode_result(content) is None


def test_redacted_offsets_never_become_false_original_source_spans(book):
    conn, profile, _ = book
    source = evidence.retain_evidence(conn, profile,
        content=b'token=abcdefghijklmnopqrstuvwxyz1234567890\nInvoice 42 Total EUR 10.00',
        media_type='text/plain', name='Redacted example')
    extraction = document_text.extract(conn, profile, evidence_id=source['id'])
    binding = {'profile_id':profile, 'book_revision':ledger.require_book(conn,profile)['revision'],
        'provider':{'name':'test'}, 'selection':{'extractions':[{'id':extraction['id'],'pages':[1],'fields':[]}]}}
    candidate = {'schema_version':1,'explanation':'Review', 'entries':[], 'document_reviews':[
        {'extraction_id':extraction['id'],'fields':{'document_number':'42'},
         'spans':{'document_number':{'page':1,'start':25,'end':27}}}]}
    with pytest.raises(AppError) as exc:
        proposals.preview(conn, profile, candidate=candidate, binding=binding)
    assert exc.value.code == 'accounting_ai_source_redacted'
    assert conn.execute('SELECT COUNT(*) FROM gl_evidence_field_reviews').fetchone()[0] == 0
