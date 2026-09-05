from decimal import Decimal

import pytest

from kassiber.core.accounting import artifacts,evidence,ledger,projection,sources,valuation
from kassiber.errors import AppError
from tests.test_accounting_projection import prepared
from tests.test_accounting_integration import book  # noqa: F401


def impaired(book,adjustment=-2000):
    conn,scope,args=prepared(book)
    valuation.ensure_schema(conn)
    original=projection.create_proposal(conn,scope,**args)
    projection.post_proposal(conn,scope,proposal_id=original['id'],expected_digest=original['payload_digest'])
    proof=evidence.retain_evidence(conn,scope,content=b'Synthetic reviewed valuation support',media_type='text/plain',name='Valuation evidence')
    record=valuation.create_valuation(conn,scope,policy_id=args['policy_id'],artifact_id=args['artifact_id'],period_id='2025',
        effective_date='2025-12-31',asset='BTC',adjustment_minor=adjustment,evidence_id=proof['id'],offset_account='fees',
        valuation_kind='impairment',reason='Reviewed book-only impairment; tax basis unchanged',idempotency_key='impairment')
    valuation.post_valuation(conn,scope,valuation_id=record['id'],expected_digest=record['payload_digest'])
    return conn,scope,args,record


def later_disposal(conn,scope,args,*,new_acquisition=False):
    ledger.create_period(conn,scope,period_id='2026',start_date='2026-01-01',end_date='2026-12-31')
    old=projection.get_policy(conn,scope,args['policy_id'])['policy']
    policy=projection.configure_policy(conn,scope,period_id='2026',asset_accounts=old['asset_accounts'],
        settlement_account='bank',income_account='income',capital_account='capital',gain_account='gain',fee_account='fees',
        acknowledge_tax_book_basis=True,reason='Reviewed next-year carrying policy')
    transaction=ledger._row(conn,"SELECT * FROM transactions WHERE id='acquisition'")
    if new_acquisition:
        conn.execute("UPDATE profiles SET gains_algorithm='LIFO' WHERE id=?",(scope,))
        conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,
            occurred_at,direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
            VALUES('new-buy',?,?,?,'new-buy','new-buy-fingerprint','2026-01-01T12:00:00Z','inbound','BTC',100000000000,0,'EUR','300','{}','2026-01-01T12:00:00Z','buy')''',
            (transaction['workspace_id'],scope,transaction['wallet_id']))
    conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,
        occurred_at,direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
        VALUES('later-sale',?,?,?,'later-sale','later-sale-fingerprint','2026-02-01T12:00:00Z','outbound','BTC',50000000000,0,'EUR','400','{}','2026-02-01T12:00:00Z','sell')''',
        (transaction['workspace_id'],scope,transaction['wallet_id']))
    snapshot=sources.capture_sources(conn,scope)
    artifact=artifacts.capture_calculation(conn,scope,snapshot_id=snapshot['id'],period_id='2026')
    results=[]
    for anchor,category in ([('new-buy','purchase')] if new_acquisition else [])+[('later-sale','disposal')]:
        event_id,mapping=next((identity,mapping) for identity,mapping in artifact['capture']['inputs']['source_event_map'].items() if mapping['journal_transaction_id']==anchor)
        binding=sources.bind_sources(conn,scope,snapshot_id=snapshot['id'],expected_digest=snapshot['input_digest'],economic_id=anchor,
            role='recognition',reason='Reviewed event',idempotency_key=anchor,
            claims=[dict(source_id=mapping['source_id'],**claim) for claim in mapping['claim_slices']])
        proposal=projection.create_proposal(conn,scope,policy_id=policy['id'],artifact_id=artifact['id'],binding_id=binding['id'],
            event_id=event_id,category=category,period_id='2026',idempotency_key='proposal-'+anchor)
        projection.post_proposal(conn,scope,proposal_id=proposal['id'],expected_digest=proposal['payload_digest'])
        results.append(proposal)
    return results[-1]


def test_reviewed_impairment_closes_with_book_tax_difference_and_immutable_evidence(book):
    conn,scope,args,record=impaired(book)
    check=projection.validate_close(conn,scope,'2025-01-01','2025-12-31')
    assert not check['blockers']
    assert check['reconciliation'][0]['book_basis_exact']=='80'
    assert check['book_adjustments']['remaining_by_asset_minor']=={'BTC':-2000}
    assert check['book_adjustments']['tax_adjustment_minor']==0
    artifact=artifacts.get_calculation(conn,scope,args['artifact_id'])
    assert Decimal(artifact['capture']['assets'][0]['open_positions'][0]['basis_exact'])==100
    for sql in ['UPDATE gl_book_valuations SET payload_json=\'{}\'',
                'DELETE FROM gl_valuation_publications','INSERT OR REPLACE INTO gl_book_valuations SELECT * FROM gl_book_valuations']:
        with pytest.raises(Exception,match='accounting_valuation_retained'):
            conn.execute(sql)
    assert valuation.get_valuation(conn,scope,record['id'])['published']


def test_partial_disposal_releases_only_consumed_quantity_and_preserves_tax_basis(book):
    conn,scope,args,record=impaired(book)
    proposal=later_disposal(conn,scope,args)
    posting=proposal['proposal']['quantitative_posting']
    assert Decimal(posting['basis_exact'])==-40
    assert posting['book_adjustment_minor']==1000
    assert valuation.controls(conn,scope,'2026-12-31')['remaining_by_asset_minor']=={'BTC':-1000}
    assert not projection.validate_close(conn,scope,'2026-01-01','2026-12-31')['blockers']
    with pytest.raises(AppError) as exc:
        valuation.require_reversible(conn,scope,record['draft_id'])
    assert exc.value.code=='accounting_valuation_in_use'


def test_later_unimpaired_lot_does_not_release_old_impairment_under_lifo(book):
    conn,scope,args,_=impaired(book)
    proposal=later_disposal(conn,scope,args,new_acquisition=True)
    assert Decimal(proposal['proposal']['quantitative_posting']['basis_exact'])==-150
    assert 'book_adjustment_minor' not in proposal['proposal']['quantitative_posting']
    assert valuation.controls(conn,scope,'2026-12-31')['remaining_by_asset_minor']=={'BTC':-2000}
    assert not projection.validate_close(conn,scope,'2026-01-01','2026-12-31')['blockers']


def test_integer_scope_allocation_preserves_sign_and_every_cent():
    assert valuation._allocate(-2,{'a':1,'b':1,'c':1})=={'a':-1,'b':-1,'c':0}
    from kassiber.core.accounting.valuation_releases import _ratio
    cumulative=[_ratio(-1,quantity,3) for quantity in range(4)]
    assert cumulative==[0,0,-1,-1]


@pytest.mark.parametrize('invalid',[[],{},None,20250101,'2025-02-30','20250101'])
def test_valuation_date_requires_canonical_typed_date(book,invalid):
    conn,scope,args=prepared(book)
    valuation.ensure_schema(conn)
    with pytest.raises(AppError):
        valuation.create_valuation(conn,scope,policy_id=args['policy_id'],artifact_id=args['artifact_id'],period_id='2025',
            effective_date=invalid,asset='BTC',adjustment_minor=-1,evidence_id='unused',offset_account='fees',
            valuation_kind='impairment',reason='Invalid date',idempotency_key='invalid-date')


def test_individual_lot_cannot_be_negative_even_if_total_would_be_positive(book):
    conn,scope,_=prepared(book)
    valuation.ensure_schema(conn)
    positions=[dict(lot_id='cheap',basis_exact='1'),dict(lot_id='expensive',basis_exact='100')]
    with pytest.raises(AppError) as exc:
        valuation._validate_lot_amounts(conn,scope,'2025-12-31','BTC',positions,
            {'cheap':-200,'expensive':-200},'impairment',2)
    assert exc.value.code=='accounting_valuation_lot_amount'


def test_write_back_cannot_create_unreviewed_revaluation(book):
    conn,scope,args,record=impaired(book)
    saved=record['valuation']['request']
    with pytest.raises(AppError) as exc:
        valuation.create_valuation(conn,scope,**{**saved,'adjustment_minor':2001,'valuation_kind':'write_back',
            'idempotency_key':'excessive-writeback'})
    assert exc.value.code=='accounting_valuation_write_back_ceiling'
    result=valuation.create_valuation(conn,scope,**{**saved,'adjustment_minor':2000,'valuation_kind':'write_back',
            'idempotency_key':'valid-writeback'})
    valuation.post_valuation(conn,scope,valuation_id=result['id'],expected_digest=result['payload_digest'])
    assert valuation.controls(conn,scope,'2025-12-31')['remaining_by_asset_minor']=={'BTC':0}
    assert not projection.validate_close(conn,scope,'2025-01-01','2025-12-31')['blockers']


def test_impaired_basis_survives_cross_asset_transit_receipt_and_partial_disposal(book):
    from kassiber.core import custody_components
    from kassiber.core.wallets import create_wallet
    conn,scope,_,_=impaired(book)
    ledger.create_period(conn,scope,period_id='2026',start_date='2026-01-01',end_date='2026-12-31')
    original=ledger._row(conn,"SELECT * FROM transactions WHERE id='acquisition'")
    wallet=create_wallet(conn,original['workspace_id'],scope,'Liquid valuation destination','custom')
    for identifier,asset,direction,day,wallet_id,amount,kind in (
        ('dispatch','BTC','outbound','2026-01-02',original['wallet_id'],100000000000,'transfer'),
        ('receipt','LBTC','inbound','2026-01-03',wallet['id'],100000000000,'transfer'),
        ('liquid-sale','LBTC','outbound','2026-02-01',wallet['id'],50000000000,'sell')):
        conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,
            direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
            VALUES(?,?,?,?,?,?,?,?,?,?,0,'EUR','100','{}',?,?)''',
            (identifier,original['workspace_id'],scope,wallet_id,identifier,'fp-'+identifier,day+'T12:00:00Z',
             direction,asset,amount,day+'T12:00:00Z',kind))
    legs=[dict(id=role,role=role,transaction_id=identifier,wallet_id=wallet_id,rail=rail,chain=rail,
        network='regtest' if rail=='bitcoin' else 'elementsregtest',asset=asset,exposure='bitcoin',
        conservation_unit='msat',amount_msat=100000000000) for role,identifier,asset,rail,wallet_id in (
        ('source','dispatch','BTC','bitcoin',original['wallet_id']),('destination','receipt','LBTC','liquid',wallet['id']))]
    component=custody_components.create_component(conn,workspace_id=original['workspace_id'],profile_id=scope,
        component_type='manual_bridge',legs=legs,
        allocations=[dict(source_leg_id='source',sink_leg_id='destination',source_amount_msat=100000000000,sink_amount_msat=100000000000)],
        evidence_kind='manual_claim',evidence_grade='reviewed',notes='Synthetic valuation carry',conversion_policy='carrying-value',conversion_reviewed=True)
    custody_components.activate_component(conn,component['id'])
    for code in ('lbtc','transit'):
        ledger.create_account(conn,scope,code=code,name=code,kind='asset')
    policy=projection.configure_policy(conn,scope,period_id='2026',asset_accounts={'BTC':'btc','LBTC':'lbtc'},
        transit_accounts={'LBTC':'transit'},settlement_account='bank',income_account='income',capital_account='capital',
        gain_account='gain',fee_account='fees',acknowledge_tax_book_basis=True,reason='Reviewed carrying policy')
    snapshot=sources.capture_sources(conn,scope)
    artifact=artifacts.capture_calculation(conn,scope,snapshot_id=snapshot['id'],period_id='2026')
    for anchor,category in [('dispatch','transfer_dispatch'),('receipt','transfer_receipt'),('liquid-sale','disposal')]:
        event,mapping=next((event,mapping) for event,mapping in artifact['capture']['inputs']['source_event_map'].items()
            if mapping['journal_transaction_id']==anchor)
        binding=sources.bind_sources(conn,scope,snapshot_id=snapshot['id'],expected_digest=snapshot['input_digest'],
            economic_id=anchor,role='recognition' if category=='disposal' else 'settlement',reason='Reviewed event',
            idempotency_key='binding-'+anchor,claims=[dict(source_id=mapping['source_id'],**claim) for claim in mapping['claim_slices']])
        proposal=projection.create_proposal(conn,scope,policy_id=policy['id'],artifact_id=artifact['id'],binding_id=binding['id'],
            event_id=event,category=category,period_id='2026',idempotency_key='proposal-'+anchor)
        projection.post_proposal(conn,scope,proposal_id=proposal['id'],expected_digest=proposal['payload_digest'])
    check=projection.validate_close(conn,scope,'2026-01-01','2026-12-31')
    assert not check['blockers']
    liquid=next(item for item in check['reconciliation'] if item['asset']=='LBTC')
    assert Decimal(liquid['tax_basis_exact'])==50
    assert Decimal(liquid['book_basis_exact'])==40
    assert check['book_adjustments']['remaining_by_asset_minor']=={'BTC':0,'LBTC':-1000}
    assert conn.execute("SELECT SUM(debit_minor-credit_minor) FROM gl_lines WHERE account_code='transit'").fetchone()[0]==0
