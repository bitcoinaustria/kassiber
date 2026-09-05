import hashlib
import json
import sqlite3

import pytest

from kassiber.core.accounting import jurisdiction as j, ledger
from kassiber.core.accounting import tax_workpapers as tax
from kassiber.errors import AppError
from tests.test_accounting_evidence import accounting_db, retained


@pytest.fixture
def tax_db(accounting_db):
    tax.ensure_schema(accounting_db)
    for profile in ('p', 'other'):
        ledger.create_period(accounting_db, profile, period_id='2025', start_date='2025-01-01', end_date='2025-12-31')
    return accounting_db


def create(conn, profile='p'):
    return tax.create_workpaper(conn, profile, period_id='2025', pack_id=j.AT_PACK_ID, idempotency_key='create')


def review(value, *, money=False):
    return dict(state='reviewed_input', reason='Synthetic reviewed fact', **{'value_minor' if money else 'value': value})


def complete_patch():
    pack = j.get_pack(j.AT_PACK_ID)
    facts = {item['id']: review(True) for item in pack['facts']}
    facts.update(liability=review('unlimited'), section7_3=review(False), entity_type=review('association'),
                 tax_scope_review=review('Synthetic unconditionally taxable test entity; no additional sources'),
                 specialist_review=review('All exceptional cases reviewed as absent in synthetic fixture'),
                 group_parent=review(False), required_annexes=review([]), capital_election=review(False))
    fields = {}
    for definition in pack['forms']['K2']['fields']:
        identifier = definition['id']
        if definition['required']:
            fields['main.' + identifier] = review('Synthetic test identity')
        else:
            fields['main.' + identifier] = dict(state='not_applicable', reason='Explicitly reviewed absent in synthetic fixture')
    return dict(facts=facts, field_reviews=fields)


def complete(conn):
    workpaper = create(conn)
    result = tax.review_workpaper(conn, 'p', workpaper_id=workpaper['id'], expected_revision=1,
        patch=complete_patch(), reason='Synthetic complete review', idempotency_key='complete')
    ledger.close_period(conn, 'p', period_id='2025', expected_revision=ledger.require_book(conn, 'p')['revision'])
    return result


def test_empty_book_full_review_close_finalize_export_roundtrip(tax_db):
    workpaper = complete(tax_db)
    preview = tax.preview_workpaper(tax_db, 'p', workpaper_id=workpaper['id'])
    assert preview['blockers'] == []
    assert preview['ready']
    receipt = tax.finalize_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=2, expected_digest=preview['input_digest'])
    final = tax.export_workpaper(tax_db, 'p', final_id=receipt['final_id'], confirm_plaintext=True)
    assert not final['stale']
    assert not final['report']['filed']
    assert not final['verification_levels']['tax_liability_certified']
    assert final['report_digest'] == ledger.digest(final['report'])
    assert '<table>' in final['html']
    assert tax.finalize_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=2, expected_digest=preview['input_digest'])['final_id'] == final['final_id']


def test_two_fiscal_periods_one_assessment_all_closes_and_balances(accounting_db):
    conn = accounting_db
    tax.ensure_schema(conn)
    for identifier, start, end in (('h1', '2025-01-01', '2025-06-30'), ('h2', '2025-07-01', '2025-12-31')):
        ledger.create_period(conn, 'p', period_id=identifier, start_date=start, end_date=end)
        draft = ledger.create_draft(conn, 'p', {'period_id':identifier, 'entry_date':end, 'description':'Synthetic income',
            'idempotency_key':identifier, 'lines':[{'account_code':'bank','debit_minor':100}, {'account_code':'sales','credit_minor':100}]})
        ledger.post_draft(conn, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])
        if identifier == 'h1':
            ledger.close_period(conn, 'p', period_id=identifier, expected_revision=ledger.require_book(conn, 'p')['revision'])
    paper = tax.create_workpaper(conn, 'p', period_id='h1', pack_id=j.AT_PACK_ID, idempotency_key='create')
    patch = complete_patch()
    patch['field_reviews']['main.660'] = review(200, money=True)
    patch['mappings'] = [dict(id='annual', field_key='main.660', account_code='sales', basis='movement', amount_minor=-200, multiplier=-1, reason='Reviewed annual income')]
    tax.review_workpaper(conn, 'p', workpaper_id=paper['id'], expected_revision=1, patch=patch, reason='Reviewed full year', idempotency_key='review')
    preview = tax.preview_workpaper(conn, 'p', workpaper_id=paper['id'])
    assert preview['book_profit_minor'] == 200
    assert [b['target'] for b in preview['blockers']] == ['h2']
    assert len(preview['binding']['assessment_periods']) == 2
    with pytest.raises(AppError, match='assessment-year'):
        tax.create_workpaper(conn, 'p', period_id='h2', pack_id=j.AT_PACK_ID, idempotency_key='duplicate-year')
    ledger.close_period(conn, 'p', period_id='h2', expected_revision=ledger.require_book(conn, 'p')['revision'])
    preview = tax.preview_workpaper(conn, 'p', workpaper_id=paper['id'])
    assert preview['ready']
    final = tax.finalize_workpaper(conn, 'p', workpaper_id=paper['id'], expected_revision=2, expected_digest=preview['input_digest'])
    ledger.reopen_period(conn, 'p', period_id='h2', reason='Reviewed later period correction', expected_revision=ledger.require_book(conn, 'p')['revision'])
    assert tax.export_workpaper(conn, 'p', final_id=final['final_id'], confirm_plaintext=True)['stale']


def test_complete_every_selected_annex_can_finalize_without_placeholder_blocks(tax_db):
    paper = create(tax_db)
    pack = j.get_pack(j.AT_PACK_ID)
    patch = complete_patch()
    forms = [key for key in pack['forms'] if key != 'K2']
    patch['facts']['required_annexes'] = review(forms)
    patch['annex_instances'] = paper['state']['annex_instances'] + [dict(id=form, form_id=form, label='Synthetic ' + form) for form in forms]
    for form in forms:
        for field in pack['forms'][form]['fields']:
            patch['field_reviews'][form + '.' + field['id']] = review('Synthetic identity') if field['required'] else dict(state='not_applicable', reason='Reviewed absent in synthetic full-annex fixture')
    patch['field_reviews']['K2a.INCOME_CLASS'] = review('GW')
    patch['field_reviews']['K11.INCOME_CLASS'] = review('GW')
    patch['field_reviews']['K12.BEH_10A4'] = review(True)
    patch['field_reviews']['K12a.WJA_12A'] = review('2025-01-01')
    patch['field_reviews']['K12a.WJE_12A'] = review('2025-12-31')
    tax.review_workpaper(tax_db, 'p', workpaper_id=paper['id'], expected_revision=1, patch=patch, reason='Reviewed full synthetic annex set', idempotency_key='all-annexes')
    ledger.close_period(tax_db, 'p', period_id='2025', expected_revision=ledger.require_book(tax_db, 'p')['revision'])
    preview = tax.preview_workpaper(tax_db, 'p', workpaper_id=paper['id'])
    assert preview['blockers'] == []
    assert len(preview['forms']) == 7
    assert sum(len(form['fields']) for form in preview['forms']) > 300
    final = tax.finalize_workpaper(tax_db, 'p', workpaper_id=paper['id'], expected_revision=2, expected_digest=preview['input_digest'])
    assert not tax.export_workpaper(tax_db, 'p', final_id=final['final_id'], confirm_plaintext=True)['stale']


def test_unknown_values_are_not_zeros_and_legal_route_is_explicit(tax_db):
    workpaper = create(tax_db)
    preview = tax.preview_workpaper(tax_db, 'p', workpaper_id=workpaper['id'])
    assert not preview['ready']
    assert any(b['code'] == 'ledger_not_closed' for b in preview['blockers'])
    assert preview['forms'][0]['fields']['610']['state'] == 'blocked'
    with pytest.raises(AppError, match='unresolved'):
        tax.finalize_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=1, expected_digest=preview['input_digest'])
    tax.review_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=1,
        patch={'facts': {'liability': review('limited')}}, reason='Reviewed foreign organization', idempotency_key='limited')
    assert any(b['code'] == 'different_form_route' for b in tax.preview_workpaper(tax_db, 'p', workpaper_id=workpaper['id'])['blockers'])


def test_scope_encryption_and_same_year_pack_guard(tax_db):
    conn = tax_db
    workpaper = create(conn)
    with pytest.raises(AppError):
        tax.get_workpaper(conn, 'other', workpaper_id=workpaper['id'])
    with pytest.raises(AppError):
        tax.create_workpaper(conn, 'p', period_id='2026', pack_id=j.AT_PACK_ID, idempotency_key='wrongyear')
    with pytest.raises(AppError):
        tax.execute(conn, 'p', 'tax-create', {'period_id':'2025', 'pack_id': j.TEST_PACK_ID, 'idempotency_key':'test'})
    with pytest.raises(AppError, match='encrypted'):
        tax.list_workpapers(sqlite3.connect(':memory:'), 'p')
    foreign_evidence = retained(conn, 'other')
    patch = {'field_reviews': {'main.660': dict(**review(5, money=True), evidence_ids=[foreign_evidence])}}
    with pytest.raises(AppError, match='this book'):
        tax.review_workpaper(conn, 'p', workpaper_id=workpaper['id'], expected_revision=1,
            patch=patch, reason='Cross profile should fail', idempotency_key='cross')
    assert tax.get_workpaper(conn, 'p', workpaper_id=workpaper['id'])['revision'] == 1


@pytest.mark.parametrize('value', [None, True, False, '100', 1.01, 10**15, -(10**15)])
def test_strict_money_rejects_entire_bug_class_atomically(tax_db, value):
    workpaper = create(tax_db)
    with pytest.raises(AppError):
        tax.review_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=1,
            patch={'field_reviews': {'main.660': review(value, money=True)}}, reason='Invalid minor value', idempotency_key='invalid')
    assert tax.get_workpaper(tax_db, 'p', workpaper_id=workpaper['id'])['revision'] == 1


def test_revision_guards_idempotency_and_client_derived_claim(tax_db):
    workpaper = create(tax_db)
    args = dict(workpaper_id=workpaper['id'], expected_revision=1,
        patch={'field_reviews': {'main.660': review(100, money=True)}}, reason='Review a value', idempotency_key='review')
    assert tax.review_workpaper(tax_db, 'p', **args)['revision'] == 2
    assert tax.review_workpaper(tax_db, 'p', **args)['revision'] == 2
    with pytest.raises(AppError, match='Idempotency'):
        tax.review_workpaper(tax_db, 'p', **{**args, 'reason': 'Different'})
    with pytest.raises(AppError, match='changed'):
        tax.review_workpaper(tax_db, 'p', **{**args, 'idempotency_key': 'stale'})
    with pytest.raises(AppError, match='deterministic'):
        tax.review_workpaper(tax_db, 'p', **{**args, 'expected_revision': 2, 'idempotency_key':'smuggle',
            'patch': {'field_reviews': {'main.660': dict(state='derived', value_minor=1, reason='Untrusted client claim')}}})


def test_immutable_records_replace_and_raw_cross_scope_refs(tax_db):
    workpaper = create(tax_db)
    foreign_evidence = retained(tax_db, 'other')
    for table in ('gl_tax_workpapers', 'gl_tax_revisions'):
        with pytest.raises(Exception, match='retained'):
            tax_db.execute(f'DELETE FROM {table}')
        with pytest.raises(Exception, match='replacement'):
            tax_db.execute(f'INSERT OR REPLACE INTO {table} SELECT * FROM {table}')
    with pytest.raises(Exception, match='scope'):
        tax_db.execute('INSERT INTO gl_tax_evidence_refs VALUES(?,?,?)', ('p', workpaper['revision_id'], foreign_evidence))


def test_reopen_and_new_review_stale_final_never_overwrite(tax_db):
    workpaper = complete(tax_db)
    preview = tax.preview_workpaper(tax_db, 'p', workpaper_id=workpaper['id'])
    final = tax.finalize_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=2, expected_digest=preview['input_digest'])
    original = tax.export_workpaper(tax_db, 'p', final_id=final['final_id'], confirm_plaintext=True)
    ledger.reopen_period(tax_db, 'p', period_id='2025', reason='Synthetic correction', expected_revision=ledger.require_book(tax_db, 'p')['revision'])
    exported = tax.export_workpaper(tax_db, 'p', final_id=final['final_id'], confirm_plaintext=True)
    assert exported['stale']
    assert exported['report_digest'] == final['report_digest']
    assert exported['report_json'] == original['report_json']
    assert hashlib.sha256(exported['report_json'].encode('utf-8')).hexdigest() == final['report_digest']
    assert exported['html_sha256'] != original['html_sha256']
    for result in (original, exported):
        assert hashlib.sha256(result['html'].encode('utf-8')).hexdigest() == result['html_sha256']
        assert final['report_digest'] in result['html']
        assert result['verification_contract']['version'] == 1
    assert 'STALE' in exported['html']
    with pytest.raises(AppError, match='changed'):
        tax.finalize_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=2, expected_digest=preview['input_digest'])


def test_ledger_mapping_budget_and_coverage_are_not_arithmetic_claims(tax_db):
    draft = ledger.create_draft(tax_db, 'p', {'idempotency_key':'income', 'period_id':'2025', 'entry_date':'2025-06-01',
        'description':'Synthetic income', 'lines':[{'account_code':'bank','debit_minor':1000},{'account_code':'sales','credit_minor':1000}]})
    ledger.post_draft(tax_db, 'p', draft_id=draft['id'], expected_digest=draft['payload_digest'])
    workpaper = complete(tax_db)
    preview = tax.preview_workpaper(tax_db, 'p', workpaper_id=workpaper['id'])
    assert not preview['verification']['ledger_source_coverage']
    assert any(b['code'] == 'unmapped_book_result' for b in preview['blockers'])
    mapping = dict(id='income', field_key='main.660', account_code='sales', basis='movement', amount_minor=-1001,
                   multiplier=-1, reason='Synthetic over-allocation')
    tax.review_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=2,
        patch={'mappings':[mapping], 'field_reviews': {'main.660': None}}, reason='Map income', idempotency_key='mapping')
    assert any(b['code'] == 'mapping_budget' for b in tax.preview_workpaper(tax_db, 'p', workpaper_id=workpaper['id'])['blockers'])


def test_annex_crypto_fields_require_negative_losses_and_reviewed_source_route(tax_db):
    workpaper = create(tax_db)
    instances = workpaper['state']['annex_instances'] + [dict(id='capital', form_id='K2kv', label='Synthetic capital')]
    tax.review_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=1,
        patch={'annex_instances': instances}, reason='Add capital source', idempotency_key='annex')
    with pytest.raises(AppError, match='negative'):
        tax.review_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=2,
            patch={'field_reviews': {'capital.175': review(300, money=True)}}, reason='Wrong sign', idempotency_key='wrong')
    tax.review_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=2,
        patch={'field_reviews': {'capital.175': review(-300, money=True)}}, reason='Correct sign', idempotency_key='correct')
    fields = tax.preview_workpaper(tax_db, 'p', workpaper_id=workpaper['id'])['forms'][1]['fields']
    assert fields['175']['value_minor'] == -300
    assert fields['POOL_WITH_KEST']['state'] == 'blocked'


def test_interest_annex_dates_and_duplicates_are_real_close_members(tax_db):
    paper = create(tax_db)
    instances = paper['state']['annex_instances'] + [dict(id=key, form_id='K12a', label=key) for key in ('one', 'two')]
    fields = {key + '.' + field: review(value) for key in ('one', 'two')
              for field, value in (('WJA_12A', '2025-01-01'), ('WJE_12A', '2025-12-31'))}
    tax.review_workpaper(tax_db, 'p', workpaper_id=paper['id'], expected_revision=1,
        patch={'annex_instances':instances, 'field_reviews':fields}, reason='Synthetic duplicated fiscal annex', idempotency_key='duplicate')
    codes = {b['code'] for b in tax.preview_workpaper(tax_db, 'p', workpaper_id=paper['id'])['blockers']}
    assert 'duplicate_interest_period' in codes
    tax.review_workpaper(tax_db, 'p', workpaper_id=paper['id'], expected_revision=2,
        patch={'field_reviews':{'two.WJE_12A':review('2026-12-31')}}, reason='Synthetic wrong assessment year', idempotency_key='wrong-year')
    codes = {b['code'] for b in tax.preview_workpaper(tax_db, 'p', workpaper_id=paper['id'])['blockers']}
    assert {'interest_fiscal_period', 'interest_fiscal_coverage'} <= codes


def test_foreign_credit_cannot_exceed_source_burdens_or_parent_year_rate(tax_db):
    paper = create(tax_db)
    instances = paper['state']['annex_instances'] + [dict(id='foreign', form_id='K12', label='Synthetic foreign entity')]
    fields = {'foreign.' + key:review(value, money=True) for key,value in
              {'BETR_K12':10000,'HINZUBET':0,'KOESTVB':0,'QUELLST':0,'VORBEL':0,'ANRECH':2400}.items()}
    fields['foreign.BET_10A7'] = review(True)
    tax.review_workpaper(tax_db, 'p', workpaper_id=paper['id'], expected_revision=1,
        patch={'annex_instances':instances, 'field_reviews':fields}, reason='Synthetic excessive foreign credit', idempotency_key='foreign')
    codes = {b['code'] for b in tax.preview_workpaper(tax_db, 'p', workpaper_id=paper['id'])['blockers']}
    assert {'participation_credit_source_cap', 'participation_credit_cap'} <= codes


def test_carryforward_arithmetic_and_prior_evidence_required(tax_db):
    workpaper = create(tax_db)
    carry = dict(id='loss2024',kind='loss',vintage_year=2024,opening_minor=1000,addition_minor=0,
                 used_minor=300,expired_minor=0,closing_minor=700,reason='Synthetic prior assessment')
    with pytest.raises(AppError, match='evidence'):
        tax.review_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=1,
            patch={'carryforwards':[carry]}, reason='Import prior loss', idempotency_key='missing')
    carry['evidence_ids'] = [retained(tax_db)]
    tax.review_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=1,
        patch={'carryforwards':[carry]}, reason='Import evidenced prior loss', idempotency_key='valid')
    with pytest.raises(AppError, match='reconcile'):
        tax.review_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=2,
            patch={'carryforwards':[{**carry,'closing_minor':800}]}, reason='Invalid balance', idempotency_key='wrong')


def test_carry_register_must_match_form_not_just_reconcile_itself(tax_db):
    paper = complete(tax_db)
    record = dict(id='loss', kind='loss', vintage_year=2024, opening_minor=1000, addition_minor=0,
                  used_minor=300, expired_minor=0, closing_minor=700, reason='Synthetic assessed loss', evidence_ids=[retained(tax_db)])
    tax.review_workpaper(tax_db, 'p', workpaper_id=paper['id'], expected_revision=2,
        patch={'carryforwards':[record]}, reason='Import reviewed prior-year loss', idempotency_key='register')
    preview = tax.preview_workpaper(tax_db, 'p', workpaper_id=paper['id'])
    assert [b['target'] for b in preview['blockers']] == ['main.619']
    tax.review_workpaper(tax_db, 'p', workpaper_id=paper['id'], expected_revision=3,
        patch={'field_reviews':{'main.619':review(1000, money=True)}}, reason='Reconcile return to register', idempotency_key='form')
    assert tax.preview_workpaper(tax_db, 'p', workpaper_id=paper['id'])['ready']


def test_prior_final_carry_is_profile_year_vintage_and_revision_bound(tax_db, monkeypatch):
    # Synthetic prior-year pack tests the generic lineage contract, without
    # advertising or pretending to implement an official 2024 filing pack.
    real_get = j.get_pack
    prior_pack = real_get(j.AT_PACK_ID)
    prior_pack.update(pack_id='TEST-PRIOR-2024', tax_year=2024, test_only=True)
    prior_pack['digest'] = ledger.digest({key:value for key,value in prior_pack.items() if key != 'digest'})
    monkeypatch.setattr(j, 'get_pack', lambda identifier, **kwargs: prior_pack if identifier == 'TEST-PRIOR-2024' else real_get(identifier, **kwargs))
    ledger.create_period(tax_db, 'p', period_id='2024', start_date='2024-01-01', end_date='2024-12-31')
    prior = tax.create_workpaper(tax_db, 'p', period_id='2024', pack_id=prior_pack['pack_id'], idempotency_key='prior')
    patch = complete_patch()
    patch['carryforwards'] = [dict(id='loss2024', kind='loss', vintage_year=2024, opening_minor=0, addition_minor=1000,
        used_minor=0, expired_minor=0, closing_minor=1000, reason='Synthetic prior year loss')]
    tax.review_workpaper(tax_db, 'p', workpaper_id=prior['id'], expected_revision=1, patch=patch, reason='Synthetic prior-year review', idempotency_key='prior-review')
    ledger.close_period(tax_db, 'p', period_id='2024', expected_revision=ledger.require_book(tax_db, 'p')['revision'])
    preview = tax.preview_workpaper(tax_db, 'p', workpaper_id=prior['id'])
    final = tax.finalize_workpaper(tax_db, 'p', workpaper_id=prior['id'], expected_revision=2, expected_digest=preview['input_digest'])
    current = complete(tax_db)
    carry = dict(id='continue', kind='loss', vintage_year=2024, opening_minor=1000, addition_minor=0,
                 used_minor=300, expired_minor=0, closing_minor=700, source_final_id=final['final_id'],
                 source_carry_id='loss2024', reason='Synthetic exact prior final continuation')
    tax.review_workpaper(tax_db, 'p', workpaper_id=current['id'], expected_revision=2,
        patch={'carryforwards':[carry], 'field_reviews':{'main.619':review(1000, money=True)}}, reason='Continue prior final', idempotency_key='continue')
    assert tax.preview_workpaper(tax_db, 'p', workpaper_id=current['id'])['ready']
    tax.review_workpaper(tax_db, 'p', workpaper_id=prior['id'], expected_revision=2,
        patch={'facts':{'specialist_review':review('Corrected prior-year review')}}, reason='Correct prior-year facts', idempotency_key='correct')
    assert 'carry_source_stale' in {b['code'] for b in tax.preview_workpaper(tax_db, 'p', workpaper_id=current['id'])['blockers']}


def test_export_does_not_execute_html_in_reviewed_names(tax_db):
    workpaper = complete(tax_db)
    tax.review_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=2,
        patch={'field_reviews': {'main.ENTITY_NAME': review('<script>private()</script>')}}, reason='Untrusted name', idempotency_key='name')
    preview = tax.preview_workpaper(tax_db, 'p', workpaper_id=workpaper['id'])
    receipt = tax.finalize_workpaper(tax_db, 'p', workpaper_id=workpaper['id'], expected_revision=3, expected_digest=preview['input_digest'])
    exported = tax.export_workpaper(tax_db, 'p', final_id=receipt['final_id'], confirm_plaintext=True)
    assert '<script>' not in exported['html']
    assert '&lt;script&gt;' in exported['html']
    assert json.loads(tax_db.execute('SELECT report_json FROM gl_tax_finals').fetchone()[0])['final_id'] == exported['final_id']
    with pytest.raises(AppError, match='confirmation'):
        tax.export_workpaper(tax_db, 'p', final_id=receipt['final_id'])
