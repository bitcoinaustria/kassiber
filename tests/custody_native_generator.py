"""Synthetic native HTLC observations; adapter cases preserve the discard boundary."""
from copy import deepcopy
from functools import lru_cache
import hashlib
import json

from tests.custody_snapshot_generator import snapshot
from tests.custody_safety_generator import stamp

NATIVE_SCENARIOS = (
    'lnd_settle','cln_settle','cln_alias_settle','nwc_untrusted','lnd_forged',
    'reverse_settle','timeout_refund','raw_refund','raw_claim','core_attestation',
    'fee_next_day','fee_unknown_time','zero_fee_next_day','utc_same_day','utc_different_day',
    'dismiss_active','dismiss_expired','dismiss_equal','dismiss_invalid','dismiss_future',
    'duplicate_ln','duplicate_chain','duplicate_occupied','duplicate_dismissed','network_mismatch','network_unknown',
    'amount_increase','amount_excess_fee','canonical_mismatch','source_untrusted','target_stale','observer_mismatch',
    'occupied_loan','occupied_channel','privacy_native','pending_claim','failed_payment','timeout_missing_proof',
    'refund_wrong_outpoint','refund_duplicate_funding','refund_partial_output','refund_batched',
    'provider_conflict','provider_same_edge','provider_only','provider_different_network',
    'policy_taxable','policy_austria','alias_asset','raw_reverse_prefix','raw_wrong_hash',
    'mixed_recorded','mixed_pipeline','mixed_samourai','mixed_channel',
)

@lru_cache(maxsize=2)
def base(role):
    from tests.test_custody_native_transitions import native_transition_rows
    rows,refs=native_transition_rows(role)
    book=deepcopy(snapshot(0))
    template=book['transactions'][0]
    book['transactions']=[dict(template, **r) for r in rows[1:]]
    for r in book['transactions']:
        # The loader normalizes configuration rather than exposing wallet material.
        for k in ('config_json','wallet_config_json'): r[k]=json.dumps({})
    book['wallets']=[dict(w,kind=next(r['wallet_kind'] for r in rows if r['wallet_id']==w['id']),config_json='{}') for w in refs.values()]
    book.update(channel_records=[],loan_legs=[],owned_index=None,receipt_sources=[],components=[],anchors={},anchor_wallets=[],book_network=None,rate_rows=[],pair_dismissals=[])
    book['profile'].update(tax_country='generic',bitcoin_rail_carrying_value=True)
    return stamp(book)


def native_snapshot(seed, *, case=None):
    case=case or NATIVE_SCENARIOS[seed % len(NATIVE_SCENARIOS)]
    refund=case in {'timeout_refund','raw_refund','timeout_missing_proof','refund_wrong_outpoint','refund_duplicate_funding','refund_partial_output','refund_batched','occupied_channel'}
    book=deepcopy(base('refund' if refund else 'claim'))
    out=next(r for r in book['transactions'] if r['direction']=='outbound')
    target=next(r for r in book['transactions'] if r['direction']=='inbound')
    # Vary exact integer quantities while keeping the whole funding output in step.
    increment=(seed%1000)*1000
    for r in (out,target):r['amount']+=increment
    if refund:
        raw=out['raw_json'];raw=raw if isinstance(raw,dict) else json.loads(raw)
        raw['vout'][0]['value_sats']+=increment//1000;out['raw_json']=raw
    if case.startswith('cln'):
        out.update(wallet_kind='core-lightning' if case=='cln_alias_settle' else 'coreln',kind='ln_pay' if case=='cln_alias_settle' else 'cln_pay',payment_hash_source='core_lightning')
        raw=deepcopy(out['raw_json']);raw['_kassiber_provenance']['import_source']='core-lightning';out['raw_json']=raw
    if case=='nwc_untrusted':out.update(wallet_kind='nwc',kind='nwc_pay',payment_hash_source='nwc');out['raw_json']={'chain':'lightning','network':'regtest','_kassiber_provenance':{'import_source':'nwc'}}
    if case=='lnd_forged':out['raw_json']={'chain':'lightning','network':'regtest'}
    if case=='reverse_settle':
        out['direction']='inbound';out['kind']='lnd_invoice';target['direction']='outbound';target['fee']=0
        out['amount'],target['amount']=target['amount'],out['amount']
    if case in {'raw_refund','raw_claim','raw_reverse_prefix','raw_wrong_hash'}:
        from tests.test_lwk_observer import POLICY_ASSET
        preimage=bytes.fromhex('42'*32);h=hashlib.sha256(preimage).digest()
        script=('82012088' if case=='raw_reverse_prefix' else '')+'a914'+hashlib.new('ripemd160',h).hexdigest()+'87632102'+'11'*32+'6703000010b1752103'+'22'*32+'68ac'
        raw=target['raw_json'];raw=json.loads(raw) if isinstance(raw,str) else raw
        raw.pop('htlc_spend',None)
        raw['vin'][0]['witness']=['7369676e6174757265','',script] if refund else ['7369676e6174757265',preimage.hex(),'01',script]
        target['raw_json']=raw
        if case=='raw_wrong_hash':target['payment_hash']='00'*32
    if case=='core_attestation':
        raw=json.loads(target['raw_json']);raw['observer']='bitcoinrpc';target['raw_json']=raw;target['observation_observer_kinds_json']='["bitcoinrpc"]'
    if case in {'fee_next_day','zero_fee_next_day'}:target['occurred_at']='2025-10-10T08:53:20Z'
    if case=='zero_fee_next_day':target['amount']=out['amount']
    if case=='fee_unknown_time':target['occurred_at']='unknown'
    if case=='utc_same_day':out['occurred_at']='2025-10-09T00:20:00+02:00';target['occurred_at']='2025-10-08T23:10:00Z'
    if case=='utc_different_day':out['occurred_at']='2025-10-09T00:20:00+02:00';target['occurred_at']='2025-10-09T00:30:00Z'
    if case.startswith('dismiss_'):
        book['as_of']='2026-01-01T00:00:00.123456Z'
        expiry={'dismiss_active':None,'dismiss_expired':'2025-12-31T23:00:00Z','dismiss_equal':book['as_of'],'dismiss_invalid':'bad','dismiss_future':'2026-01-01T00:00:00.123457Z'}[case]
        book['pair_dismissals']=[dict(out_transaction_id=out['id'],in_transaction_id=target['id'],expires_at=expiry)]
    if case.startswith('duplicate_'):
        row=deepcopy(target if case=='duplicate_chain' else out);row['id']='duplicate';book['transactions'].append(row)
        if case=='duplicate_occupied':book['loan_legs']=[dict(transaction_id='duplicate',role='collateral_lock')]
        if case=='duplicate_dismissed':book['pair_dismissals']=[dict(out_transaction_id='duplicate',in_transaction_id=target['id'],expires_at=None)]
    if case in {'network_mismatch','network_unknown'}:
        raw=out['raw_json'];raw['network']='main' if case=='network_mismatch' else 'not-a-network';out['raw_json']=raw
    if case=='amount_increase':target['amount']=out['amount']+1
    if case=='amount_excess_fee':target['amount']=out['amount']-2_500_001
    if case=='canonical_mismatch':out.update(amount_includes_fee=1,fee=1_000_000)
    if case=='source_untrusted':out.update(payment_hash_source='importer')
    if case=='observer_mismatch':target['observation_observer_kinds_json']='["esplora"]'
    if case=='occupied_loan':book['loan_legs']=[dict(transaction_id=out['id'],role='collateral_lock')]
    if case=='occupied_channel':
        from tests.custody_channel_generator import channel_snapshot
        channel=channel_snapshot(0,case='strict_lnd')
        book['wallets'].extend(dict(w,config_json='{}') for w in channel['wallets'])
        for r in (out,target):
            r['asset']='BTC';raw=r['raw_json'];raw=json.loads(raw) if isinstance(raw,str) else raw
            raw.update(chain='bitcoin',network='regtest');raw.pop('component',None);r['raw_json']=raw
        record=deepcopy(channel['channel_records'][0]);record.update(txid=out['external_id'],outpoint=out['external_id']+':0',amount_msat=out['amount'],network='regtest',config_json='{}');book['channel_records']=[record]
    if case=='privacy_native':target['privacy_boundary']='coinjoin'
    if case=='pending_claim':
        raw=json.loads(target['raw_json']);raw['status']={'confirmed':False};target['raw_json']=raw
    if case=='failed_payment':
        # Current adapters omit unsettled wallet rows. Keep a legacy failed
        # attempt as a negative input: it cannot establish a native transition.
        book['transactions']=[out];out['payment_hash']=None;out['raw_json']={'chain':'lightning','network':'regtest','status':'FAILED'}
    if case=='timeout_missing_proof':
        raw=json.loads(target['raw_json']);raw.pop('htlc_spend',None);target['raw_json']=raw;target['swap_refund_funding_txid']=out['external_id']
    if case=='refund_wrong_outpoint':target['swap_refund_funding_outpoint']='44'*32+':1'
    if case=='refund_duplicate_funding':book['transactions'].append(dict(out,id='duplicate-funding'))
    if case=='refund_partial_output':
        raw=out['raw_json'];raw['vout'][0]['value_sats']-=1;out['raw_json']=raw
    if case=='refund_batched':
        raw=json.loads(target['raw_json']);raw['vin'].append(dict(txid='55'*32,vout=1));target['raw_json']=raw
    if case.startswith('provider_'):
        raw=json.loads(target['raw_json']);raw.update(provider='boltz',swap_id='local-synthetic',flow='reverse',send_txid=out['external_id'],receive_txid=target['external_id']);target['raw_json']=raw
        source=out if case in {'provider_same_edge','provider_only'} else dict(out,id='provider-conflict',wallet_id='other-node',payment_hash=None)
        raw=deepcopy(source['raw_json']);raw.update(provider='boltz',swap_id='local-synthetic',flow='reverse',send_txid=source['external_id'],receive_txid=target['external_id']);source['raw_json']=raw
        if source is not out:book['transactions'].append(source)
        if case=='provider_only':out['payment_hash']=None
        if case=='provider_different_network':source['raw_json']['network']='main'
    if case=='policy_taxable':book['profile']['bitcoin_rail_carrying_value']=False
    if case=='policy_austria':book['profile']['tax_country']='at'
    if case=='alias_asset':out['asset']='XBT';target['asset']='L-BTC'
    if case.startswith('mixed_'):
        from tests.custody_interpretation_generator import recorded_snapshot,samourai_snapshot
        from tests.custody_pipeline_generator import pipeline_snapshot
        from tests.custody_channel_generator import channel_snapshot
        other={'mixed_recorded':lambda:recorded_snapshot(seed,case='bitcoin'),'mixed_pipeline':lambda:pipeline_snapshot(seed,case='bitcoin_mixed'),'mixed_samourai':lambda:samourai_snapshot(seed,case='tx0'),'mixed_channel':lambda:channel_snapshot(seed,case='strict_lnd')}[case]()
        book['transactions'].extend(other['transactions']);book['wallets'].extend(other['wallets']);book['channel_records'].extend(other['channel_records']);book['owned_index']=other['owned_index'];book['receipt_sources']=other['receipt_sources']
    for r in book['transactions']:
        if isinstance(r['raw_json'],dict):r['raw_json']=json.dumps(r['raw_json'])
    stamp(book)
    if case=='target_stale':target['observation_graph_hash']='stale'
    return book
