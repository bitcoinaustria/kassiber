"""Raw books with successful composed producers and separate event identities."""
from copy import deepcopy
import json
from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from tests.custody_graph_generator import graph_snapshot
from tests.custody_fanout_generator import fanout_snapshot
from tests.custody_graph_vectors import match

PIPELINE_SCENARIOS=(
    'bitcoin_consolidation','liquid_consolidation','bitcoin_fanout','liquid_fanout',
    'bitcoin_ownership','liquid_ownership','bitcoin_mixed','liquid_mixed',
    'bitcoin_normalized','bitcoin_collaborative','bitcoin_residual','bitcoin_blocked',
)


def _fanout(seed,liquid):
    book=fanout_snapshot(seed,case='liquid_graph' if liquid else 'bitcoin_graph')
    chain,network=('liquid','liquid') if liquid else ('bitcoin','main')
    raw=json.loads(book['transactions'][0]['raw_json'])
    book['owned_index']=dict(by_script={},by_outpoint={},by_scoped_outpoint=[],txid_wallets={},txid_wallets_by_scope=[])
    book['owned_index']['by_script'][raw['vin'][0]['prevout']['scriptpubkey']]=[match('wallet-a',chain=chain,network=network)]
    # The raw graph was emitted from target construction order, before row sort.
    for output,source in zip(raw['vout'],book['receipt_sources']):
        book['owned_index']['by_script'][output['scriptpubkey']]=[match(source['wallet_id'],chain=chain,network=network)]
    return book


def pipeline_snapshot(seed,*,case=None):
    case=case or PIPELINE_SCENARIOS[seed%len(PIPELINE_SCENARIOS)]
    liquid=case.startswith('liquid')
    if case.endswith('fanout'):book=_fanout(seed,liquid)
    elif case.endswith('mixed'):
        parts=[graph_snapshot(seed*3,case='liquid_consolidation' if liquid else 'bitcoin_consolidation'),_fanout(seed*3+1,liquid),graph_snapshot(seed*3+2,case='liquid_ownership' if liquid else 'bitcoin_ownership')]
        # All Liquid events share a domain; fan-out's fixture uses main Liquid.
        if liquid:
            for part in (parts[0],parts[2]):
                for row in part['transactions']:
                    raw=json.loads(row['raw_json']);raw['network']='liquid';row['raw_json']=json.dumps(raw);row['config_json']='{"chain":"liquid","network":"liquid"}'
                for matches in list(part['owned_index']['by_script'].values())+list(part['owned_index']['by_outpoint'].values()):
                    for m in matches:m['network']='liquidv1'
                for entry in part['owned_index']['by_scoped_outpoint']+part['owned_index']['txid_wallets_by_scope']:
                    entry['key'][1]='liquidv1'
                    for m in entry['matches']:
                        if isinstance(m,dict):m['network']='liquidv1'
        book=deepcopy(parts[0]);book['transactions']=[];book['receipt_sources']=[]
        for position,part in enumerate(parts):
            for row in part['transactions']:
                row['id']=f'{position}:{row["id"]}'
                book['transactions'].append(row)
            book['receipt_sources'].extend(part['receipt_sources'])
            if position:
                for key in ('by_script','by_outpoint','txid_wallets'):book['owned_index'][key].update(part['owned_index'][key])
                for key in ('by_scoped_outpoint','txid_wallets_by_scope'):book['owned_index'][key].extend(part['owned_index'][key])
    elif case.endswith('collaborative'):
        from tests.custody_consolidation_vectors import books
        book=deepcopy(next(book for name,book,_ in books() if name=='collaborative'))
        for row in book['transactions']:row['occurred_at']=f'2026-01-{seed%28+1:02d}T00:00:00Z'
    else:
        book=graph_snapshot(seed,case=('liquid_' if liquid else 'bitcoin_')+('consolidation' if case.endswith(('consolidation','normalized')) else 'ownership'))
    if case.endswith('normalized'):
        for row in book['transactions']:
            if row['direction']=='outbound':row.update(amount=row['amount']+row['fee'],fee=0,amount_includes_fee=0,observation_fee_attribution='implicit_wallet_delta')
            raw=json.loads(row['raw_json'])
            raw['observer_owned_scripts']=[script for script,matches in book['owned_index']['by_script'].items() if matches[0]['wallet_id']==row['wallet_id']]
            row['raw_json']=json.dumps(raw)
    if case.endswith('residual'):book['transactions'][0]['amount']+=10_000_000
    if case.endswith('blocked'):
        raw=json.loads(book['transactions'][0]['raw_json'])
        script=raw['vout'][0]['scriptpubkey']
        book['owned_index']['by_script'][script].append(match('wallet-b'))
        source=book['transactions'][0]
        book['transactions'].append(dict(source,id='receipt',direction='inbound',wallet_id='wallet-c',amount=source['amount']-1000,fee=0))
    for row in book['transactions']:
        row.pop('custody_native_fee_normalized',None)
        row['observation_graph_hash']=canonical_graph_hash(row['raw_json'])
        row['observation_quantity_hash']=canonical_observed_quantity_hash(row)
    book['transactions'].sort(key=lambda r:(r['occurred_at'],r['created_at'],r['id']))
    book['gap_worklist']['rows']=deepcopy(book['transactions'])
    return book
