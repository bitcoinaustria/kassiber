"""Frozen Python consolidation decisions, errors and collaborative helper edges."""
from copy import deepcopy
import json
from pathlib import Path
import sys

from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from kassiber.core.custody_snapshot import decode_owned_index
from kassiber.core.ownership_transfers import derive_multi_source_consolidations, _complete_collaborative_destinations
from kassiber.core.onchain import parse_ownership_tx
from tests.custody_graph_generator import graph_snapshot
from tests.custody_graph_vectors import match
from tests.custody_snapshot import to_jsonable

PATH = Path(__file__).parent / 'fixtures/custody_consolidation_cases.json'
SCENARIOS = (
    'bitcoin', 'liquid', 'absent_index', 'empty_index', 'empty_rows', 'single_source',
    'handled_source', 'handled_receipt', 'duplicate_wallet', 'graphless', 'no_scope',
    'synthetic', 'synthetic_fanout', 'synthetic_cross', 'synthetic_payout', 'synthetic_owned',
    'different_fees', 'normalized', 'mixed_normalized', 'normalized_zero', 'normalized_zero_fee',
    'fee_string', 'fee_unicode', 'fee_bad', 'fee_type', 'amount_bad', 'amount_type',
    'nonpositive_source', 'graph_conflict', 'unknown_input_asset', 'different_input_asset',
    'unknown_output_asset', 'different_output_asset', 'fee_output', 'unknown_value',
    'external', 'external_zero', 'change', 'shared_change', 'ambiguous_destination',
    'no_destination', 'unproven_source', 'foreign_input', 'shared_foreign_input', 'empty_inputs',
    'conservation', 'missing_wallet', 'zero_destination', 'negative_destination',
    'duplicate_receipt', 'wrong_receipt', 'zero_receipt', 'synthetic_receipts',
    'offgroup_near', 'offgroup_exact', 'offgroup_different', 'offgroup_handled',
    'offgroup_other_wallet', 'offgroup_other_asset', 'offgroup_outside',
    'offgroup_bad_amount', 'offgroup_wrong_network', 'offgroup_unknown_identity',
    'two_destinations', 'two_synthetic', 'mixed_destinations', 'sink_order', 'sender_order',
    'split_fee', 'normalized_split_fee', 'value_pricing', 'exact_pricing', 'pricing_error',
    'unbounded', 'huge_sum', 'exact_msat', 'uppercase_asset', 'duplicate_wallet_ref',
    'duplicate_source_id', 'empty_source_id', 'negative_capacity', 'group_order',
    'collaborative', 'collab_stale', 'collab_fee', 'collab_duplicate_wallet',
    'collab_ambiguous_input', 'collab_unknown_input', 'collab_ambiguous_output',
    'collab_unknown_output', 'collab_missing_wallet', 'collab_residual',
    'collab_wrong_direction', 'collab_wrong_amount', 'collab_zero_delta',
    'collab_no_receipt', 'collab_zero_source', 'conflict_before_fee', 'handled_before_fee',
    'fee_before_script', 'scope_before_amount', 'liquid_component_display', 'single_conserving', 'duplicate_wallet_owned', 'incomplete_extra_input', 'normalized_fee_gate', 'zero_extra_sink', 'collab_stale_graph', 'collab_bad_version', 'offgroup_boundary', 'aggregate_destination', 'collab_zero_change', 'unknown_value_before_script', 'no_destination_before_input', 'canonical_fallback', 'uppercase_txid',
)


def books():
    for case in SCENARIOS:
        liquid = case in {'liquid', 'unknown_input_asset', 'different_input_asset', 'unknown_output_asset', 'different_output_asset', 'fee_output', 'liquid_component_display'}
        book = graph_snapshot(0, case='liquid_consolidation' if liquid else 'bitcoin_consolidation')
        rows = book['transactions']
        a, b = sorted((r for r in rows if r['direction'] == 'outbound'), key=lambda r:r['id'])
        receipt = next(r for r in rows if r['direction'] == 'inbound')
        raw = json.loads(a['raw_json']); index=book['owned_index']; handled=[]
        original = a['raw_json']
        scripts={m[0]['wallet_id']:s for s,m in index['by_script'].items()}
        def add_output(wallet, sats, n=None):
            output=dict(raw['vout'][0], n=n if n is not None else len(raw['vout']), scriptpubkey=scripts.get(wallet,wallet), value_sats=sats)
            raw['vout'].append(output)
            return output
        def normalize():
            a.update(fee=1000,custody_native_fee_normalized=True)
            b.update(fee=2000,custody_native_fee_normalized=True)
            raw['vin'][0]['prevout']['value_sats']=a['amount']//1000+1
            raw['vin'][1]['prevout']['value_sats']=b['amount']//1000+2
            raw['vout'][0]['value_sats']-=3;receipt['amount']-=3000
        if case=='absent_index':book['owned_index']=None
        if case=='empty_index':book['owned_index']={k:[] if isinstance(v,list) else {} for k,v in index.items()}
        if case=='empty_rows':book['transactions']=[]
        if case=='single_source':rows.remove(b)
        if case.startswith('handled_'):handled=[a['id'] if case=='handled_source' else receipt['id']]
        if case=='duplicate_wallet':b['wallet_id']=a['wallet_id']
        if case=='graphless':raw={'txid':raw['txid']}
        if case=='no_scope':raw={};[r.update(external_id='provider') for r in rows]
        if case.startswith('synthetic') and case!='synthetic_receipts':a['id']={'synthetic':'multi-consol:', 'synthetic_fanout':'recorded-fanout:', 'synthetic_cross':'cross-split:', 'synthetic_payout':'direct-payout:', 'synthetic_owned':'owned-derive:'}[case]+'a'
        if case=='different_fees':b['fee']+=1000
        if case.startswith('normalized') or case=='negative_capacity':
            normalize()
            if case=='normalized_zero':a['amount']=0;receipt['amount']=b['amount'];raw['vout'][0]['value_sats']=b['amount']//1000;raw['vin'][0]['prevout']['value_sats']=1
            if case=='normalized_zero_fee':a['amount']=a['fee']=0
            if case=='normalized_split_fee':pass
            if case=='negative_capacity':
                a['amount']=-1;a['fee']=1;b['amount']+=1
                receipt['amount']=a['amount']+b['amount'];raw['vout'][0]['value_sats']=receipt['amount']//1000
        if case=='mixed_normalized':a['custody_native_fee_normalized']=True
        if case in {'fee_string','fee_unicode'}:a['fee']=b['fee']='３０００' if case=='fee_unicode' else '3000'
        if case in {'fee_bad','fee_type'}:a['fee']='bad' if case=='fee_bad' else [1]
        if case in {'amount_bad','amount_type'}:a['amount']='bad' if case=='amount_bad' else [1]
        if case=='nonpositive_source':a['amount']=0
        if case=='graph_conflict':raw['ownership_graph']=dict(raw,vin=[dict(raw['vin'][0],prevout=dict(raw['vin'][0]['prevout'],value_sats=1))])
        if case=='unknown_input_asset':raw['vin'][0]['prevout'].update(asset_id=None,asset=None)
        if case=='different_input_asset':raw['vin'][0]['prevout']['asset_id']='ee'*32
        if case=='unknown_output_asset':raw['vout'][0].update(asset_id=None,asset=None)
        if case=='different_output_asset':raw['vout'][0]['asset_id']='ee'*32
        if case=='fee_output':raw['vout'].append(dict(raw['vout'][-1],n=5))
        if case=='unknown_value':raw['vout'][0].pop('value_sats')
        if case in {'external','external_zero'}:add_output('abcd',1 if case=='external' else 0)
        if case in {'change','shared_change'}:
            add_output('wallet-a',5)
            if case=='shared_change':index['by_script'][scripts['wallet-a']].append(match('wallet-c'))
        if case=='ambiguous_destination':index['by_script'][scripts['wallet-c']].append(match('wallet-d'))
        if case=='no_destination':raw['vout'][0]['scriptpubkey']=scripts['wallet-a']
        if case=='unproven_source':index['by_outpoint']={};index['by_scoped_outpoint']=[];index['by_script'].pop(scripts['wallet-b'])
        if case=='foreign_input':raw['vin'].append(dict(txid='ee'*32,vout=0,prevout=dict(scriptpubkey='abcd',value_sats=1)))
        if case=='shared_foreign_input':index['by_scoped_outpoint'][0]['matches'].append(match('wallet-d'))
        if case=='empty_inputs':raw['vin']=[]
        if case=='conservation':a['amount']+=1
        if case=='missing_wallet':book['wallets']=[w for w in book['wallets'] if w['id']!='wallet-c']
        if case in {'zero_destination','negative_destination'}:raw['vout'][0]['value_sats']=0 if case=='zero_destination' else -1;b['amount']=a['amount']=0
        if case=='duplicate_receipt':rows.append(dict(receipt,id='receipt-copy'))
        if case=='wrong_receipt':receipt['amount']+=1
        if case=='zero_receipt':receipt['amount']=0
        if case=='synthetic_receipts':rows.remove(receipt)
        if case.startswith('offgroup_'):
            other=dict(receipt,id='offgroup',external_id='provider',raw_json='{}')
            rows.append(other)
            if case=='offgroup_near':other['amount']+=1
            if case=='offgroup_different':other['external_id']='ee'*32
            if case=='offgroup_handled':handled=[other['id']]
            if case=='offgroup_other_wallet':other['wallet_id']='wallet-b'
            if case=='offgroup_other_asset':other['asset']='OTHER'
            if case=='offgroup_outside':other['amount']+=max(receipt['amount']//100,2500000)+1
            if case=='offgroup_boundary':other['amount']+=max(receipt['amount']//100,2500000)
            if case=='offgroup_bad_amount':other['amount']='bad'
            if case=='offgroup_wrong_network':other.update(external_id=a['external_id'],config_json='{"chain":"bitcoin","network":"test"}')
            if case=='offgroup_unknown_identity':other['raw_json']=json.dumps({'txid':'ee'*32});other['external_id']=a['external_id']
        if case in {'two_destinations','two_synthetic','mixed_destinations','sink_order','sender_order','split_fee','normalized_split_fee','exact_msat'}:
            book['wallets'].append(dict(book['wallets'][0],id='wallet-d',label='D'))
            script_d='0014'+'99'*20;index['by_script'][script_d]=[match('wallet-d')];scripts['wallet-d']=script_d
            total=raw['vout'][0]['value_sats']
            split=1 if case in {'split_fee','normalized_split_fee'} else total//2
            raw['vout'][0]['value_sats']=split
            add_output('wallet-d',total-split)
            receipt['amount']=split*1000
            if case!='mixed_destinations':rows.append(dict(receipt,id='receipt-d',wallet_id='wallet-d',amount=(total-split)*1000))
            if case=='two_synthetic':book['transactions']=[a,b]
            if case=='sink_order':raw['vout'].reverse()
            if case=='sender_order':rows.reverse()
            if case=='exact_msat':a['amount']+=1;b['amount']-=1
        if case in {'value_pricing','exact_pricing','pricing_error'}:
            a['fiat_value']='bad' if case=='pricing_error' else '12.50'
            if case=='exact_pricing':a['fiat_value_exact']='25.5'
        if case in {'unbounded','huge_sum'}:
            extra=10**80 if case=='unbounded' else 2**62
            for r in (a,b):r['amount']+=extra*1000
            raw['vout'][0]['value_sats']+=extra*2;receipt['amount']+=extra*2000
        if case=='uppercase_asset':receipt['asset']='btc'
        if case=='duplicate_wallet_ref':book['wallets'].append(dict(next(w for w in book['wallets'] if w['id']=='wallet-c'),label='Last'))
        if case=='duplicate_source_id':b['id']=a['id']
        if case=='empty_source_id':a['id']=''
        if case=='group_order':
            rows.extend([dict(r,id='z'+r['id'],external_id='ef'*32,fee='bad',raw_json=json.dumps(dict(raw,txid='ef'*32))) for r in (a,b)])
            a['fee']=[1]
        if case.startswith('collab'):
            for r in (a,b):r['fee']=0
            for entry,r in zip(raw['vin'],(a,b)):entry['prevout']['value_sats']=r['amount']//1000
            raw['vout'][0]['value_sats']=sum(r['amount'] for r in (a,b))//1000;receipt['amount']=raw['vout'][0]['value_sats']*1000
            raw['vin'].append(dict(txid='ee'*32,vout=0,prevout=dict(scriptpubkey='abcd',value_sats=100)))
            add_output('abcd',100)
            if case=='collab_fee':receipt['fee']=1
            if case=='collab_duplicate_wallet':rows.append(dict(receipt,id='copy'))
            if case=='collab_ambiguous_input':index['by_scoped_outpoint'][0]['matches'].append(match('wallet-c'))
            if case=='collab_unknown_input':
                raw['vin'].append(dict(txid='ff'*32,vout=5,prevout=deepcopy(raw['vin'][0]['prevout'])))
                raw['vin'][0]['prevout'].pop('value_sats')
            if case=='collab_ambiguous_output':index['by_script'][scripts['wallet-c']].append(match('wallet-d'))
            if case=='collab_unknown_output':
                add_output('wallet-c',raw['vout'][0]['value_sats'])
                raw['vout'][0].pop('value_sats')
            if case=='collab_missing_wallet':rows.remove(receipt)
            if case=='collab_residual':raw['vout'][0]['value_sats']+=1;receipt['amount']+=1000
            if case=='collab_wrong_direction':receipt['direction']='unknown'
            if case=='collab_wrong_amount':receipt['amount']+=1
            if case=='collab_zero_delta':add_output('wallet-a',raw['vin'][0]['prevout']['value_sats'])
            if case=='collab_no_receipt':receipt['amount']=0
            if case=='collab_zero_source':a['amount']=0
        if case in {'conflict_before_fee','handled_before_fee','fee_before_script','scope_before_amount'}:
            a['fee']='bad'
            if case=='conflict_before_fee':raw['ownership_graph']=dict(raw,vin=[dict(raw['vin'][0],prevout=dict(raw['vin'][0]['prevout'],value_sats=1))])
            if case=='handled_before_fee':handled=[receipt['id']]
            if case=='fee_before_script':raw['vout'][0]['scriptpubkey']=[1]
            if case=='scope_before_amount':a.update(amount='bad',config_json='{"chain":"bitcoin","network":"test"}');raw['network']='test'
        if case=='liquid_component_display':
            raw['component'].pop('asset_id')
            for entry in raw['vin']:entry['prevout'].pop('asset_id')
            for output in raw['vout']:output.pop('asset_id')
        if case=='single_conserving':
            rows.remove(b);raw['vin']=raw['vin'][:1];receipt['amount']=a['amount'];raw['vout'][0]['value_sats']=a['amount']//1000
        if case=='duplicate_wallet_owned':
            b['wallet_id']=a['wallet_id']
            index['by_scoped_outpoint'][1]['matches']=deepcopy(index['by_scoped_outpoint'][0]['matches'])
            raw['vin'][1]['prevout']['scriptpubkey']=scripts['wallet-a']
        if case=='nonpositive_source':
            receipt['amount']=b['amount']+b['fee'];raw['vout'][0]['value_sats']=receipt['amount']//1000
        if case=='incomplete_extra_input':
            raw['chain']='liquid';raw['network']='elementsregtest';raw['component']={'asset_id':'ef'*32,'asset':'LBTC'}
            for r in rows:r.update(asset='LBTC',config_json='{"chain":"liquid","network":"elementsregtest"}')
            for matches in index['by_script'].values():
                for m in matches:m.update(chain='liquid',network='elementsregtest')
            for entry in index['by_scoped_outpoint']:
                entry['key'][:2]=['liquid','elementsregtest']
                for m in entry['matches']:m.update(chain='liquid',network='elementsregtest')
            for entry in raw['vin']:entry['prevout'].update(asset_id='ef'*32,asset='LBTC')
            for output in raw['vout']:output.update(asset_id='ef'*32,asset='LBTC')
            raw['vin'].append(dict(txid='ee'*32,vout=5,prevout=dict(scriptpubkey=scripts['wallet-a'],value_sats=1)))
        if case=='normalized_fee_gate':
            normalize();a['fee']=0;b.update(amount=0,fee=1000)
            raw['vin'][0]['prevout']['value_sats']=a['amount']//1000
            raw['vin'][1]['prevout']['value_sats']=1
            add_output('wallet-b',1)
            raw['vout'][0]['value_sats']=a['amount']//1000;receipt['amount']=a['amount']
            raw['vin'].append(dict(txid='ee'*32,vout=0,prevout=dict(scriptpubkey='abcd',value_sats=10)))
            add_output('abcd',10)
        if case=='zero_extra_sink':
            rows.remove(receipt)
            book['wallets'].append(dict(book['wallets'][0],id='wallet-d'))
            index['by_script']['abba']=[match('wallet-d')]
            add_output('abba',0)
        if case=='aggregate_destination':
            raw['vout'][0]['value_sats']-=1;add_output('wallet-c',1)
        if case=='collab_zero_change':
            index['by_script']['abba']=[match('wallet-d')]
            raw['vin'].append(dict(txid='ee'*32,vout=1,prevout=dict(scriptpubkey='abba',value_sats=1)))
            add_output('abba',1)
        if case=='unknown_value_before_script':
            raw['vout'][0].pop('value_sats')
            add_output('abcd',1)['scriptpubkey']=[1]
        if case=='no_destination_before_input':
            raw['vout'][0]['scriptpubkey']=scripts['wallet-a']
            index['by_outpoint']={};index['by_scoped_outpoint']=[]
            raw['vin'][0]['prevout']['scriptpubkey']=[1]
        if case=='canonical_fallback':raw['txid']='provider'
        if case=='uppercase_txid':raw['txid']=raw['txid'].upper()
        for row in book['transactions']:
            if row['raw_json']==original:row['raw_json']=json.dumps(raw)
            try:
                row['observation_graph_hash']=canonical_graph_hash(row['raw_json'])
                row['observation_quantity_hash']=canonical_observed_quantity_hash(row)
            except (TypeError,ValueError,ArithmeticError):pass
        if case=='collab_stale':receipt['observation_quantity_hash']='stale'
        if case=='collab_stale_graph':receipt['observation_graph_hash']='stale'
        if case=='collab_bad_version':receipt['observation_authority_version']=2
        yield case,book,handled


def vectors():
    output=[]
    for case, book, handled in books():
        raw_book=deepcopy(book)
        for row in raw_book['transactions']:row.pop('custody_native_fee_normalized',None)
        vector=dict(case=case,input=raw_book,rows=book['transactions'],handled_ids=handled)
        try:
            result=derive_multi_source_consolidations(book['transactions'],index=decode_owned_index(book['owned_index']),wallet_refs_by_id={str(w['id']):w for w in book['wallets']},already_paired_ids=set(handled))
            vector['expected']={k:sorted(v) if isinstance(v,set) else v for k,v in vars(result).items()}
        except Exception as exc:vector['error']=type(exc).__name__
        output.append(to_jsonable(vector,redact_config=False))
    return output


def helper_vectors():
    output=[]
    for case,book,_handled in books():
        if not case.startswith('collab'):
            continue
        rows=book['transactions']
        parsed=parse_ownership_tx(rows[0]['raw_json'])
        scope=('bitcoin','main')
        try:
            expected=_complete_collaborative_destinations(rows,parsed,decode_owned_index(book['owned_index']),scope)
            output.append(dict(case=case,input=book,parsed_json=json.dumps(parsed),scope=scope,expected=expected))
        except Exception as exc:
            output.append(dict(case=case,input=book,parsed_json=json.dumps(parsed),scope=scope,error=type(exc).__name__))
    return to_jsonable(output,redact_config=False)


if __name__=='__main__':
    for path,values in ((PATH,vectors()),(PATH.with_name('custody_consolidation_helpers.json'),helper_vectors())):
        text=json.dumps(values,indent=2,ensure_ascii=True,sort_keys=True)+'\n'
        if '--write' in sys.argv:path.write_text(text)
        else:assert path.read_text()==text
