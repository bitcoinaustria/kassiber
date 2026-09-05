"""Independent production-path regressions; no mocked source/engine authority."""
import pytest

from kassiber.core.accounting import artifacts, ledger, projection, sources, valuation
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_projection import prepared
from tests.test_accounting_valuation import impaired


def test_cannot_reverse_impairment_under_a_published_write_back(book):
    conn, profile, _, impairment = impaired(book)
    restored = valuation.create_valuation(conn, profile, **{**impairment['valuation']['request'],
        'adjustment_minor':2000, 'valuation_kind':'write_back', 'idempotency_key':'restore'})
    valuation.post_valuation(conn, profile, valuation_id=restored['id'], expected_digest=restored['payload_digest'])
    with pytest.raises(AppError):
        ledger.reverse_entry(conn, profile, entry_id=impairment['draft_id'], entry_date='2025-12-31',
            period_id='2025', idempotency_key='reverse-impairment-only', reason='Correct impairment')


def test_close_requires_review_after_source_date_correction_even_when_totals_match(book):
    conn, profile, args = prepared(book)
    proposal = projection.create_proposal(conn, profile, **args)
    projection.post_proposal(conn, profile, proposal_id=proposal['id'], expected_digest=proposal['payload_digest'])
    # A source correction must leave the posted snapshot immutable but require
    # review; recomputing equal year-end totals is not approval of the old date.
    conn.execute("UPDATE transactions SET occurred_at='2025-02-01T12:00:00Z' WHERE id='acquisition'")
    snapshot = sources.capture_sources(conn, profile)
    artifacts.capture_calculation(conn, profile, snapshot_id=snapshot['id'], period_id='2025')
    check = projection.validate_close(conn, profile, '2025-01-01', '2025-12-31')
    assert check['source_coverage']['stale_bindings']
    assert projection.get_proposal(conn, profile, proposal['id'])['entry_date'] == '2025-01-01'
    assert check['blockers'], 'Equal totals must not silently approve stale source provenance'


def test_excluding_last_posted_source_cannot_disable_projection_close_controls(book):
    conn, profile, args = prepared(book)
    proposal = projection.create_proposal(conn, profile, **args)
    projection.post_proposal(conn, profile, proposal_id=proposal['id'], expected_digest=proposal['payload_digest'])
    conn.execute("UPDATE transactions SET excluded=1 WHERE id='acquisition'")
    assert sources.source_coverage(conn, profile)['stale_bindings']
    check = projection.validate_close(conn, profile, '2025-01-01', '2025-12-31')
    assert check['required'] and check['blockers']


def test_full_disposal_cannot_close_with_unexplained_negative_asset_rounding_residue(book):
    conn, profile, args = prepared(book)
    sources.void_binding(conn, profile, binding_id=args['binding_id'], reason='Rebuild synthetic fixture', idempotency_key='void-initial')
    conn.execute("UPDATE transactions SET fiat_rate_exact='0.03' WHERE id='acquisition'")
    row = ledger._row(conn, "SELECT * FROM transactions WHERE id='acquisition'")
    for index in (1,2):
        conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,
            direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
            VALUES(?,?,?,?,?,?,?,'outbound','BTC',50000000000,0,'EUR','0.10','{}',?,'sell')''',
            (f'sale{index}',row['workspace_id'],profile,row['wallet_id'],f'sale{index}',f'fp{index}',
             f'2025-02-0{index}T12:00:00Z',f'2025-02-0{index}T12:00:00Z'))
    snapshot = sources.capture_sources(conn, profile)
    artifact = artifacts.capture_calculation(conn, profile, snapshot_id=snapshot['id'], period_id='2025')
    for anchor, category in [('acquisition','purchase'),('sale1','disposal'),('sale2','disposal')]:
        event, mapping = next((event,mapping) for event,mapping in artifact['capture']['inputs']['source_event_map'].items()
            if mapping['journal_transaction_id']==anchor)
        binding = sources.bind_sources(conn, profile, snapshot_id=snapshot['id'], expected_digest=snapshot['input_digest'],
            economic_id=anchor, role='recognition', reason='Reviewed synthetic event', idempotency_key='binding-'+anchor,
            claims=[dict(source_id=mapping['source_id'],**claim) for claim in mapping['claim_slices']])
        proposal = projection.create_proposal(conn, profile, **{**args,'artifact_id':artifact['id'],'binding_id':binding['id'],
            'event_id':event,'category':category,'idempotency_key':anchor})
        projection.post_proposal(conn, profile, proposal_id=proposal['id'], expected_digest=proposal['payload_digest'])
    check = projection.validate_close(conn, profile, '2025-01-01', '2025-12-31')
    balance = conn.execute("SELECT SUM(debit_minor-credit_minor) FROM gl_lines WHERE profile_id=? AND account_code='btc'", (profile,)).fetchone()[0]
    assert check['reconciliation'][0]['expected_quantity_msat'] == 0
    assert balance == 0
    assert not check['blockers']


def _opening_transit(book, *, receipt_role='settlement'):
    from kassiber.core import custody_components
    from kassiber.core.wallets import create_wallet
    from kassiber.core.accounting import opening
    conn, profile, args = prepared(book, historical=True)
    sources.void_binding(conn, profile, binding_id=args['binding_id'], reason='Prepare synthetic transfer', idempotency_key='replace-source')
    original = ledger._row(conn, "SELECT * FROM transactions WHERE id='acquisition'")
    wallet = create_wallet(conn, original['workspace_id'], profile, 'Liquid destination', 'custom')
    for identifier, asset, direction, day, wallet_id in (
        ('dispatch','BTC','outbound','2024-12-30',original['wallet_id']),
        ('receipt','LBTC','inbound','2025-01-02',wallet['id'])):
        conn.execute('''INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,
          direction,asset,amount,fee,fiat_currency,fiat_rate_exact,raw_json,created_at,kind)
          VALUES(?,?,?,?,?,?,?,?,?,100000000000,0,'EUR','100','{}',?,'transfer')''',
          (identifier,original['workspace_id'],profile,wallet_id,identifier,'fingerprint-'+identifier,day+'T12:00:00Z',direction,asset,day+'T12:00:00Z'))
    legs = []
    for role, identifier, asset, rail, wallet_id in (
        ('source','dispatch','BTC','bitcoin',original['wallet_id']),('destination','receipt','LBTC','liquid',wallet['id'])):
        legs.append(dict(id=role,role=role,transaction_id=identifier,wallet_id=wallet_id,rail=rail,
            chain=rail,network='regtest' if rail=='bitcoin' else 'elementsregtest',asset=asset,exposure='bitcoin',conservation_unit='msat',amount_msat=100000000000))
    component = custody_components.create_component(conn,workspace_id=original['workspace_id'],profile_id=profile,
        component_type='manual_bridge',legs=legs,
        allocations=[dict(source_leg_id='source',sink_leg_id='destination',source_amount_msat=100000000000,sink_amount_msat=100000000000)],
        evidence_kind='manual_claim',evidence_grade='reviewed',notes='Synthetic cross-cutoff custody',conversion_policy='carrying-value',conversion_reviewed=True)
    custody_components.activate_component(conn,component['id'])
    for code in ('lbtc','transit'):
        ledger.create_account(conn,profile,code=code,name=code,kind='asset')
    policy = projection.configure_policy(conn,profile,period_id='2025',asset_accounts={'BTC':'btc','LBTC':'lbtc'},
        transit_accounts={'LBTC':'transit'},settlement_account='bank',income_account='income',capital_account='capital',
        gain_account='gain',fee_account='fees',acknowledge_tax_book_basis=True,reason='Reviewed synthetic opening')
    snapshot = sources.capture_sources(conn,profile)
    initial = artifacts.capture_calculation(conn,profile,snapshot_id=snapshot['id'],period_id='2025',boundary='opening')
    binding = opening.bind_opening_sources(conn,profile,artifact_id=initial['id'],period_id='2025',
        expected_source_digest=snapshot['input_digest'],reason='Reviewed all pre-opening sources',idempotency_key='opening-sources')
    proposal = opening.create_opening_proposal(conn,profile,policy_id=policy['id'],artifact_id=initial['id'],binding_id=binding['id'],period_id='2025',idempotency_key='opening')
    projection.post_proposal(conn,profile,proposal_id=proposal['id'],expected_digest=proposal['payload_digest'])
    closing = artifacts.capture_calculation(conn,profile,snapshot_id=snapshot['id'],period_id='2025')
    event, mapping = next((event,mapping) for event,mapping in closing['capture']['inputs']['source_event_map'].items() if mapping['journal_transaction_id']=='receipt')
    receipt_binding = sources.bind_sources(conn,profile,snapshot_id=snapshot['id'],expected_digest=snapshot['input_digest'],
        economic_id='receipt',role=receipt_role,claims=[dict(source_id=mapping['source_id'],**claim) for claim in mapping['claim_slices']],
        reason='Reviewed arrival against opening transit',idempotency_key='receipt-source')
    receipt = projection.create_proposal(conn,profile,policy_id=policy['id'],artifact_id=closing['id'],binding_id=receipt_binding['id'],
        event_id=event,category='transfer_receipt',period_id='2025',idempotency_key='receipt')
    return conn, profile, proposal, receipt


def test_opening_cross_cutoff_receipt_settles_only_its_reviewed_transit(book):
    conn, profile, opening, receipt = _opening_transit(book)
    assert opening['proposal']['quantitative_posting']['location'] == 'transit'
    assert sources.get_binding(conn,profile,opening['binding_id'])['role'] == 'recognition'
    assert sources.get_binding(conn,profile,receipt['binding_id'])['role'] == 'settlement'
    posted = projection.post_proposal(conn,profile,proposal_id=receipt['id'],expected_digest=receipt['payload_digest'])
    assert posted['published']
    assert conn.execute("SELECT SUM(debit_minor-credit_minor) FROM gl_lines WHERE profile_id=? AND account_code='transit'", (profile,)).fetchone()[0] == 0
    check = projection.validate_close(conn,profile,'2025-01-01','2025-12-31')
    assert not check['blockers']
    with pytest.raises(AppError):
        projection.create_proposal(conn,profile,**{**receipt['proposal']['request'],'idempotency_key':'second-receipt'})
    with pytest.raises(AppError):
        ledger.reverse_entry(conn,profile,entry_id=opening['draft_id'],entry_date='2025-01-03',period_id='2025',
            idempotency_key='reverse-settled-opening',reason='Cannot erase transit origin under settled receipt')


def test_opening_receipt_cannot_use_revenue_recognition_source_role(book):
    with pytest.raises(AppError) as error:
        _opening_transit(book,receipt_role='recognition')
    assert error.value.code == 'accounting_projection_binding'
