from decimal import Decimal

import pytest

from kassiber.core.accounting import artifacts, ledger, projection, sources
from kassiber.core.wallets import create_wallet
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401


def prepared(book, quantity=100_000_000_000, disposal=False, historical=False):
    conn, scope, _ = book
    sources.ensure_schema(conn)
    artifacts.ensure_schema(conn)
    projection.ensure_schema(conn)
    for code, kind in [('btc','asset'),('income','income'),('gain','income'),('fees','expense')]:
        ledger.create_account(conn, scope, code=code, name=code, kind=kind)
    policy = projection.configure_policy(conn, scope, period_id='2025', asset_accounts={'BTC':'btc'},
        settlement_account='bank', income_account='income', capital_account='capital',
        gain_account='gain', fee_account='fees', acknowledge_tax_book_basis=True, reason='Reviewed for this book and fiscal year')
    workspace = conn.execute('SELECT workspace_id FROM profiles WHERE id=?', (scope,)).fetchone()[0]
    wallet = create_wallet(conn, workspace, scope, 'Projection source', 'custom')
    conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,
        occurred_at,direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
        VALUES('acquisition',?,?,?,'source','fingerprint','2025-01-01T12:00:00Z','inbound',
        'BTC',?,0,'EUR','100','{}','2025-01-01T12:00:00Z','buy')''', (workspace,scope,wallet['id'],quantity))
    if historical:
        conn.execute("UPDATE transactions SET occurred_at='2024-01-01T12:00:00Z' WHERE id='acquisition'")
    if disposal:
        conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,
            occurred_at,direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
            VALUES('disposal',?,?,?,'sale-source','sale-fingerprint','2025-02-01T12:00:00Z','outbound',
            'BTC',50000000000,1000000000,'EUR','200','{}','2025-02-01T12:00:00Z','sell')''', (workspace,scope,wallet['id']))
    snapshot = sources.capture_sources(conn, scope)
    artifact = artifacts.capture_calculation(conn, scope, snapshot_id=snapshot['id'], period_id='2025',boundary='opening' if historical else 'closing')
    event_id, mapping = next((event_id, mapping) for event_id,mapping in artifact['capture']['inputs']['source_event_map'].items()
                            if mapping['journal_transaction_id'] == ('disposal' if disposal else 'acquisition'))
    binding = sources.bind_sources(conn, scope, snapshot_id=snapshot['id'], expected_digest=snapshot['input_digest'],
        economic_id='reviewed-acquisition',role='recognition',reason='Reviewed',idempotency_key='source-bind',
        claims=[dict(source_id=mapping['source_id'],**slice_) for slice_ in mapping['claim_slices']])
    args = dict(policy_id=policy['id'],artifact_id=artifact['id'],binding_id=binding['id'],event_id=event_id,
                category='disposal' if disposal else 'purchase',period_id='2025',idempotency_key='proposal')
    conn.commit()
    return conn, scope, args


def test_real_acquisition_proposal_is_balanced_posted_and_idempotent(book):
    conn, scope, args = prepared(book)
    proposal = projection.create_proposal(conn,scope,**args)
    exact=proposal['proposal']['quantitative_posting']
    assert {key:value for key,value in exact.items() if key!='currency_rounding'} == dict(asset='BTC',quantity_msat=100_000_000_000,basis_exact='100',
        account_code='btc',location='inventory',book_value_minor=10000)
    assert exact['currency_rounding'][0]['remainder_minor']==0
    assert proposal['proposal']['lines'] == [dict(account_code='bank',debit_minor=0,credit_minor=10000),dict(account_code='btc',debit_minor=10000,credit_minor=0)]
    assert projection.create_proposal(conn,scope,**args)['id'] == proposal['id']
    posted = projection.post_proposal(conn,scope,proposal_id=proposal['id'],expected_digest=proposal['payload_digest'])
    assert posted['published']
    assert ledger._entry(conn,scope,proposal['draft_id'])['status'] == 'posted'
    assert projection.post_proposal(conn,scope,proposal_id=proposal['id'],expected_digest=proposal['payload_digest'])['published']
    check=projection.validate_close(conn,scope,'2025-01-01','2025-12-31')
    assert not check['blockers']
    assert check['external_completeness_verified'] is False
    with pytest.raises(AppError) as exc:
        sources.void_binding(conn,scope,binding_id=args['binding_id'],reason='Wrong',idempotency_key='void')
    assert exc.value.code == 'accounting_projection_in_use'


def test_zero_currency_posting_keeps_quantity_and_dated_reversal(book):
    conn, scope, args = prepared(book,quantity=1)
    proposal = projection.create_proposal(conn,scope,**args)
    assert proposal['draft_id'] is None and proposal['proposal']['lines'] == []
    assert proposal['proposal']['quantitative_posting']['quantity_msat'] == 1
    projection.post_proposal(conn,scope,proposal_id=proposal['id'],expected_digest=proposal['payload_digest'])
    assert not projection.validate_close(conn,scope,'2025-01-01','2025-12-31')['blockers']
    assert conn.execute('SELECT COUNT(*) FROM gl_entries WHERE profile_id=?',(scope,)).fetchone()[0] == 0
    void_args = dict(proposal_id=proposal['id'],expected_digest=proposal['payload_digest'],entry_date='2025-02-01',period_id='2025',reason='Reclassify')
    assert projection.void_quantity_proposal(conn,scope,**void_args)['voided']
    assert any(item['code']=='accounting_projection_reconciliation' for item in projection.validate_close(conn,scope,'2025-01-01','2025-12-31')['blockers'])
    assert projection.void_quantity_proposal(conn,scope,**void_args)['voided']
    with pytest.raises(AppError) as exc:
        projection.void_quantity_proposal(conn,scope,**{**void_args,'reason':'Different'})
    assert exc.value.code == 'accounting_idempotency_conflict'


def test_source_change_rejects_stale_posting_and_keeps_proposal_history(book):
    conn, scope, args = prepared(book)
    proposal = projection.create_proposal(conn,scope,**args)
    conn.execute("UPDATE transactions SET fiat_rate_exact='101' WHERE id='acquisition'")
    with pytest.raises(AppError) as exc:
        projection.post_proposal(conn,scope,proposal_id=proposal['id'],expected_digest=proposal['payload_digest'])
    assert exc.value.code == 'accounting_source_stale'
    assert projection.get_proposal(conn,scope,proposal['id'])['published'] is False
    assert ledger._entry(conn,scope,proposal['draft_id'])['status'] == 'draft'


def test_cross_book_duplicate_wrong_policy_and_sql_mutation_rejected(book):
    conn, scope, args = prepared(book)
    proposal = projection.create_proposal(conn,scope,**args)
    for override, expected in [({'idempotency_key':'other'},'accounting_projection_duplicate'),
                               ({'category':'capital'},'accounting_idempotency_conflict')]:
        with pytest.raises(AppError) as exc:
            projection.create_proposal(conn,scope,**{**args,**override})
        assert exc.value.code == expected
    with pytest.raises(AppError):
        projection.get_proposal(conn,'other',proposal['id'])
    for sql in ['UPDATE gl_projection_proposals SET payload_json=\'{}\'',
                'DELETE FROM gl_projection_policies',
                'INSERT OR REPLACE INTO gl_projection_proposals SELECT * FROM gl_projection_proposals']:
        with pytest.raises(Exception,match='accounting_projection_(retained|duplicate)'):
            conn.execute(sql)


def test_disposal_uses_execution_basis_and_separate_fee_claim_budget(book):
    conn,scope,args = prepared(book,disposal=True)
    proposal = projection.create_proposal(conn,scope,**args)
    assert proposal['proposal']['quantitative_posting']['quantity_msat'] == -51_000_000_000
    assert Decimal(proposal['proposal']['quantitative_posting']['basis_exact']) == -51
    assert proposal['proposal']['lines'] == [dict(account_code='bank',debit_minor=10000,credit_minor=0),
        dict(account_code='btc',debit_minor=0,credit_minor=5100),dict(account_code='fees',debit_minor=100,credit_minor=0),
        dict(account_code='gain',debit_minor=0,credit_minor=5000)]


def test_opening_migrates_historical_basis_and_explicit_cash_without_double_entry(book):
    from kassiber.core.accounting import opening
    conn,scope,args=prepared(book,historical=True)
    proposal=opening.create_opening_proposal(conn,scope,policy_id=args['policy_id'],artifact_id=args['artifact_id'],
        binding_id=args['binding_id'],period_id='2025',idempotency_key='opening',
        additional_balances=[dict(account_code='bank',balance_minor=2500)])
    draft=ledger._entry(conn,scope,proposal['draft_id'])
    assert draft['entry_kind']=='opening' and draft['entry_date']=='2025-01-01'
    assert proposal['proposal']['lines']==[dict(account_code='bank',debit_minor=2500,credit_minor=0),
        dict(account_code='btc',debit_minor=10000,credit_minor=0),dict(account_code='capital',debit_minor=0,credit_minor=12500)]
    projection.post_proposal(conn,scope,proposal_id=proposal['id'],expected_digest=proposal['payload_digest'])
    snapshot=sources.capture_sources(conn,scope)
    artifacts.capture_calculation(conn,scope,snapshot_id=snapshot['id'],period_id='2025')
    assert not projection.validate_close(conn,scope,'2025-01-01','2025-12-31')['blockers']


@pytest.mark.parametrize('category',['purchase','income','capital'])
def test_settlement_binding_cannot_recognize_acquisition_or_revenue(book,category):
    conn,scope,args=prepared(book)
    original=sources.get_binding(conn,scope,args['binding_id'])
    snapshot=sources.get_snapshot(conn,scope,original['snapshot_id'])
    sources.void_binding(conn,scope,binding_id=original['id'],reason='Payment only',idempotency_key='void-recognition')
    replacement=sources.bind_sources(conn,scope,snapshot_id=snapshot['id'],expected_digest=snapshot['input_digest'],
        economic_id='payment-only',role='settlement',reason='Settlement evidence, not revenue',idempotency_key='settlement',
        claims=[{key:item[key] for key in ('source_id','start_atomic','end_atomic')} for item in original['claims']])
    with pytest.raises(AppError) as exc:
        projection.create_proposal(conn,scope,**{**args,'binding_id':replacement['id'],'category':category})
    assert exc.value.code=='accounting_projection_binding'
    assert not conn.execute('SELECT 1 FROM gl_entries WHERE profile_id=?',(scope,)).fetchone()


def test_subcent_source_fee_retains_tax_basis_and_explicit_currency_remainder(book):
    conn,scope,args=prepared(book)
    sources.void_binding(conn,scope,binding_id=args['binding_id'],reason='Build exact-fee fixture',idempotency_key='replace')
    conn.execute("UPDATE transactions SET fiat_rate_exact='0.03' WHERE id='acquisition'")
    original=ledger._row(conn,"SELECT * FROM transactions WHERE id='acquisition'")
    for identifier,day,quantity,fee in [('fee-sale','2025-02-01',25000000000,25000000000),('final-sale','2025-03-01',50000000000,0)]:
        conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,
            direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
            VALUES(?,?,?,?,?,?,?,'outbound','BTC',?,?,'EUR','0.10','{}',?,'sell')''',
            (identifier,original['workspace_id'],scope,original['wallet_id'],identifier,'fp-'+identifier,
             day+'T12:00:00Z',quantity,fee,day+'T12:00:00Z'))
    snapshot=sources.capture_sources(conn,scope)
    artifact=artifacts.capture_calculation(conn,scope,snapshot_id=snapshot['id'],period_id='2025')
    for anchor,category in [('acquisition','purchase'),('fee-sale','disposal'),('final-sale','disposal')]:
        event,mapping=next((event,mapping) for event,mapping in artifact['capture']['inputs']['source_event_map'].items()
            if mapping['journal_transaction_id']==anchor)
        binding=sources.bind_sources(conn,scope,snapshot_id=snapshot['id'],expected_digest=snapshot['input_digest'],
            economic_id=anchor,role='recognition',reason='Reviewed exact source and fee slices',idempotency_key='bind-'+anchor,
            claims=[dict(source_id=mapping['source_id'],**claim) for claim in mapping['claim_slices']])
        proposal=projection.create_proposal(conn,scope,**{**args,'artifact_id':artifact['id'],'binding_id':binding['id'],
            'event_id':event,'category':category,'idempotency_key':anchor})
        projection.post_proposal(conn,scope,proposal_id=proposal['id'],expected_digest=proposal['payload_digest'])
        if anchor=='fee-sale':
            exact=proposal['proposal']['quantitative_posting']
            assert Decimal(exact['basis_exact'])==Decimal('-0.015')
            assert exact['currency_rounding'][0]['remainder_minor']==1
            assert next(line for line in proposal['proposal']['lines'] if line['account_code']=='fees')['debit_minor']==1
    assert not projection.validate_close(conn,scope,'2025-01-01','2025-12-31')['blockers']
    assert conn.execute("SELECT SUM(debit_minor-credit_minor) FROM gl_lines WHERE account_code='btc'").fetchone()[0]==0
    assert artifacts.get_calculation(conn,scope,artifact['id'])==artifact
