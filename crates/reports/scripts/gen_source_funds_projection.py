"""Freeze full source-funds presentation bytes from the unchanged Python core.

This is a pure-projection oracle, not acceptance of native report routing.
Every database-backed reference projection checks every table. Scratch and
books stay under this checkout's target directory and the account HOME stays put.
"""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import random
import tempfile

from kassiber.core import source_funds as sof
from kassiber.daemon import _source_funds_hooks
from kassiber.db import open_db
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_reads import BASE
from tests.rust_port.book_schema import TABLES

ROOT = Path(__file__).resolve().parents[3]
DESTINATION = ROOT / 'crates/reports/tests/data/source_funds_projection.json'
SHAPE_KEYS = ('overview', 'data_sources', 'data_provenance_summary', 'flow_levels', 'simplified_flow', 'narrative')


def wire(value):
    return json.dumps(value, separators=(',', ':'), allow_nan=False)


def tables(conn):
    names = [row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    assert tuple(names) == TABLES
    return wire({name: sorted([dict(row) for row in conn.execute(f'SELECT * FROM "{name}"')], key=wire) for name in names})


def evaluate(operation, request):
    try:
        if operation == 'shape':
            result = deepcopy(request)
            sof._add_report_shape(result)
        elif operation == 'transaction':
            result = sof._tx_node(request['row'], request['mode'], request['required'], is_target=request['is_target'], overrides=request['overrides'])
        elif operation == 'source':
            result = sof._source_node(request['row'], request['mode'], request['required'])
        elif operation == 'attachment':
            result = sof._attachment_summary(request['row'], request['mode'])
        elif operation == 'options':
            result = sof._normalize_report_options(request)
        elif operation == 'context':
            result = sof._report_context(request)
        elif operation == 'reveal':
            result = sof._normalize_reveal_mode(request)
        elif operation == 'purpose':
            result = sof._normalize_report_purpose(request)
        elif operation == 'edge_explanation':
            result = sof._public_explanation(request['explanation'], request['mode'])
            if result and any(request['overrides'].get(endpoint) == 'hide' for endpoint in request['endpoint_ids']):
                result = 'Reviewed link; details withheld for a hidden transaction.'
        else:
            raise AssertionError(operation)
        return {'ok': result}
    except AppError as error:
        return {'error': dict(code=error.code, message=str(error), hint=error.hint, details=error.details, retryable=error.retryable)}
    except Exception as error:
        return {'error': dict(code='internal_error', message=str(error), hint=None, details=None, retryable=False)}


def cases(conn):
    result = []
    def add(name, operation, request):
        before = tables(conn)
        expected = evaluate(operation, request)
        assert tables(conn) == before, name
        result.append(dict(name=name, operation=operation, input_json=wire(request), expected_json=wire(expected)))

    tx = dict(conn.execute('SELECT t.*,w.label AS wallet_label,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.id=?', ('in',)).fetchone())
    source = dict(conn.execute("SELECT * FROM source_funds_sources WHERE id='s-a'").fetchone())
    for mode in sof.REVEAL_MODES:
        for target in (False, True):
            for override in ({}, {'in': 'show'}, {'in': 'hide'}, {'elsewhere': 'hide'}):
                add(f'tx-{mode}-{target}-{override}', 'transaction', dict(row=tx, mode=mode, required=123456789000, is_target=target, overrides=override))
        for row in conn.execute('SELECT * FROM attachments ORDER BY id'):
            add(f'attachment-{mode}-{row["id"]}', 'attachment', dict(row=dict(row), mode=mode))
        for source_type in sof.SOURCE_TYPES:
            row = {**source, 'source_type': source_type}
            add(f'source-{mode}-{source_type}', 'source', dict(row=row, mode=mode, required=1))
        for ids, overrides in [(['in'], {}), (['in'], {'in':'hide'}), (['in','out'], {'out':'hide'}), (['in'], {'out':'hide'})]:
            add(f'edge-{mode}-{ids}-{overrides}', 'edge_explanation', dict(explanation='outpoint 01234567:4 / payment-id private', mode=mode, endpoint_ids=ids, overrides=overrides))
    for kind in ['descriptor', 'xpub', 'samourai', 'address', 'coreln', 'lnd', 'nwc', 'phoenix', 'river', 'bullbitcoin', 'coinfinity', '21bitcoin', 'pocketbitcoin', 'strike', 'wasabi', 'custom', '', None, ' OTHER ']:
        for config in ('{}', '{"source_file":"literal.csv"}', '{"source_format":"csv"}', '{"source_file":""}', '[1]', '{broken'):
            row = {**tx, 'wallet_kind': kind, 'wallet_config_json': config}
            add(f'provenance-{kind}-{config}', 'transaction', dict(row=row, mode='labels_only', required=None, is_target=False, overrides={}))
    for quantity in [0, 1, -1, 1.5, -0.5, -1.5, 2**53+1, 2**63-1, -(2**63), ' １２３ ', 'bad']:
        row = {**tx, 'amount': quantity, 'fee': quantity, 'external_id': None, 'fiat_currency': None, 'pricing_source_kind': None, 'fiat_price_source': 'fallback', 'description': 'private', 'counterparty': 'private'}
        add(f'quantity-{quantity}', 'transaction', dict(row=row, mode='full', required=1, is_target=True, overrides={}))
        add(f'source-quantity-{quantity}', 'source', dict(row={**source, 'amount':quantity}, mode='full', required=None))
    for value in [None, '', ' STANDARD ', 'minimal', 'labels_only', 'full', 'wrong']:
        add(f'reveal-{value}', 'reveal', value)
    for value in [None, '', 'existing_transaction', ' PLANNED-EXCHANGE-SALE ', 'wrong']:
        add(f'purpose-{value}', 'purpose', value)
    for country in ['at', 'AT', 'generic', None, '', ' at ', 'K']:
        for currency in ['eur', 'EUR', 'USD', None, '']:
            add(f'context-{country}-{currency}', 'context', dict(tax_country=country, fiat_currency=currency))
    for value in [None, {}, dict(diagram_detail=' DETAILED ', amount_precision=' SATS ', mask_recipient='yes', omit_sections=['GRAPH_NODES',' flow_links ','unknown','graph_nodes'], reveal_overrides={'in':' HIDE ','out':'SHOW','':'hide','invalid':'keep'}), dict(omit_sections='flow_levels'), dict(omit_sections={'transaction_details':False}), dict(diagram_detail=42, amount_precision='unknown', reveal_overrides=['hide']), [], [1], False, 'invalid']:
        add(f'options-{value}', 'options', value)

    for mode in sof.REVEAL_MODES:
        for target in ['in','out','earn','no-tax']:
            before = tables(conn)
            report = sof.build_report(conn, 'ws', 'p', _source_funds_hooks(), target_transaction_ref=target, reveal_mode=mode)
            assert tables(conn) == before
            request = deepcopy(report)
            for key in SHAPE_KEYS:
                request.pop(key, None)
            request['allocations'].pop('route_difference_explanation', None)
            add(f'live-report-{mode}-{target}', 'shape', request)
            assert result[-1]['expected_json'] == wire({'ok':report})

    # Keep synthetic sort keys distinct: Python's simplified-flow set iteration
    # leaves equal node keys dependent on its process hash seed (separate repro).
    randomizer = random.Random(21053)
    base_tx = sof._tx_node(tx, 'standard', 1000, is_target=True)
    base_source = sof._source_node(source, 'standard', 1000)
    for case_index in range(120):
        count = case_index % 9
        nodes = [{**base_tx, 'id':'tx:target', 'transaction_id':'target', 'label':'Target', 'required_amount_msat':1000, 'required_amount':1e-8, 'fee_msat':case_index % 4, 'fiat_value':2.675}]
        edges = []
        for index in range(count):
            is_source = index % 3 == 0
            node = dict(base_source if is_source else base_tx)
            node.update(id=f'n{index}', label=f'Node {index:02}', required_amount_msat=100+index, required_amount=(100+index)/1e11)
            node.update(asset='LBTC' if case_index % 7 == 0 and index % 2 else 'BTC')
            if not is_source:
                node.update(wallet=f'Wallet {index%2}', data_provenance=['chain_sync','platform_export','manual_import'][index%3], fiat_value=[1.225, None, -0.0, 2.675][(index+case_index)%4], fiat_currency=['EUR','USD'][index%2] if case_index%2 else 'EUR')
            nodes.append(node)
            parent = node['id']
            child = 'tx:target' if index == 0 or case_index % 4 else f'n{index-1}'
            edges.append(dict(id=f'e{index}', **{'from':parent,'to':child}, link_type=['self_transfer','coinjoin','payjoin','swap'][case_index%4], asset=node['asset'], allocation_amount_msat=100+index, allocation_amount=(100+index)/1e11, from_allocation_amount_msat=100+index+(case_index%4 if index==0 else 0)))
        if case_index % 5 == 0:
            nodes.append({**base_source,'id':'unreachable','label':'Unreachable','fiat_value':None})
        if count > 3 and case_index % 11 == 0:
            edges.append(dict(edges[-1], id='cycle', **{'from':'tx:target','to':f'n{count-1}'}))
        mix = [dict(source_type=kind,asset='LBTC' if case_index%7==0 and index%2 else 'BTC',amount_msat=100+index,amount=(100+index)/1e11,percent_of_target=None if case_index%2 else (100+index)/10,count=index+1) for index,kind in enumerate(sof.SOURCE_TYPES[:case_index%9])]
        randomizer.shuffle(nodes)
        request = dict(workspace='Main', profile='Book', target=nodes[next(i for i,n in enumerate(nodes) if n['id']=='tx:target')], graph=dict(nodes=nodes,edges=edges), allocations=dict(asset='BTC',gross_matches_target=case_index%2==0,route_difference_msat=case_index%4,route_difference=(case_index%4)/1e11,gross_source_requirement=[dict(asset='BTC',amount=1.003e-8,amount_msat=1003)] if mix else []), source_mix=mix, explain_gates=dict(blockers=[{}]*(case_index%3), warnings=[{}]*(case_index%4)))
        add(f'shape-{case_index}', 'shape', request)
    # Integer division rounds the exact ratio once. Converting its two operands
    # to floats first changes allocated fiat for valid 64-bit msat quantities.
    found = 0
    while found < 40:
        denominator = randomizer.randrange(2**53, 2**63)
        numerator = randomizer.randrange(1, denominator)
        fiat = 1e14
        if round(fiat*(numerator/denominator),2) == round(fiat*(float(numerator)/float(denominator)),2):
            continue
        node = {**base_tx,'id':'tx:large','amount_msat':denominator,'amount':denominator/1e11,'required_amount_msat':numerator,'required_amount':numerator/1e11,'fiat_value':fiat,'fiat_currency':'EUR'}
        report = dict(target=node,graph=dict(nodes=[node],edges=[]),allocations=dict(asset='BTC',gross_matches_target=False,route_difference_msat=None),source_mix=[],explain_gates=dict(blockers=[],warnings=[]))
        add(f'large-exact-ratio-{found}', 'shape', report)
        found += 1
    for name, graph, target in [('no-id',dict(nodes=[],edges=[]),dict(required_amount=0,asset='BTC')), ('absent-target',dict(nodes=[base_source],edges=[]),base_tx)]:
        add(name, 'shape', dict(target=target,graph=graph,allocations=dict(asset='BTC'),source_mix=[],explain_gates=dict(blockers=[],warnings=[])))
    return result


def generate():
    scratch = ROOT/'target/r5-sof-reads-3/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='projection-oracle-', dir=scratch) as temporary:
        conn = open_db(Path(temporary)/'data')
        conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE)
        conn.commit()
        result = cases(conn)
        conn.close()
    return json.dumps(result, ensure_ascii=True, indent=2)+'\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    generated = generate()
    if args.write:
        DESTINATION.write_text(generated)
    elif DESTINATION.read_text() != generated:
        raise SystemExit('source-funds projection vectors differ; regenerate intentionally')
    print(f'{len(json.loads(generated))} source-funds projection cases verified with all {len(TABLES)} tables')


if __name__ == '__main__':
    main()
