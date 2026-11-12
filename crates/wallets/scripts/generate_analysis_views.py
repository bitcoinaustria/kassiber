from __future__ import annotations

import json
from pathlib import Path
import sys
import uuid
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_index import database
from generate_analysis_sources import stored, cache, inventory, label, dataset, raw, update, TX, INSTANT
from generate_analysis_ingestion import q
from kassiber.core.chain_analysis.projection_store import synchronize
from kassiber.core.chain_analysis.projection import _View
from kassiber.core.chain_analysis.query import query_index
from kassiber.core.chain_analysis.index import thaw
from kassiber.errors import AppError

DEST = ROOT / 'crates/wallets/tests/data/analysis_views.json'


def materialize(sql, profile='p'):
    conn = database(sql)
    try:
        with patch('kassiber.core.chain_analysis.projection_store.uuid.uuid4', return_value=uuid.UUID(int=1)), patch('kassiber.core.chain_analysis.projection_store.now_iso', return_value=INSTANT), patch('kassiber.core.chain_analysis_datasets.now_iso', return_value=INSTANT):
            synchronize(conn, profile)
        tables = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'chain_index_%' ORDER BY name")]
        result = sql
        for table in tables:
            result += f'DELETE FROM {table};'
            for row in sorted(conn.execute(f'SELECT * FROM {table}'), key=lambda row: tuple(q(v) for v in row)):
                result += f'INSERT INTO {table} VALUES(' + ','.join(q(v) for v in row) + ');'
        return result
    finally:
        conn.close()


def evaluate(case, conn):
    state = dict(conn.execute('SELECT * FROM chain_index_state WHERE profile_id=?', (case.get('profile','p'),)).fetchone())
    view = _View(conn, case.get('profile','p'), state, case.get('observer','owner'))
    if case.get('upgrade'):
        view = view.for_observer('owner')
    if case.get('query') is not None:
        try:
            from kassiber.core.chain_analysis.query import normalize_query
            return query_index(view, normalize_query(case['query']))
        except AppError as error:
            return {'error': {'code': error.code, 'message': str(error), 'details': error.details, 'retryable': error.retryable}}
    nodes = set(view.nodes)
    owned = view.owned_output_ids()
    from kassiber.core.chain_analysis.analytics import analyze_index
    analytics = analyze_index(view, {'observer':case.get('observer','owner'), 'include_hypotheses':True, '_subject_node_ids':view.profile_seeds, '_selected_edge_ids':list(view.edges)}, nodes)
    return {
        'analytics':thaw(analytics),
        'snapshot_id':view.snapshot_id, 'coverage':thaw(view.coverage),
        'nodes': {key: thaw(view.nodes[key]) for key in view.nodes},
        'edges': {key: thaw(view.edges[key]) for key in view.edges},
        'facts':{key: thaw(view.transaction_facts[key]) for key in view.transaction_facts},
        'outputs':{key: thaw(view.output_facts[key]) for key in view.output_facts},
        'seeds':list(view.seed_nodes(case.get('seed_query',{}))),
        'findings':thaw(view.findings_for_nodes(nodes)), 'labels':thaw(view.labels_for_nodes(nodes)),
        'label_count':len(view.labels), 'available':len(view.available_transaction_ids()),
        'owned':sorted(owned), 'owned_count':len(owned),
        'domain_count':len({(view.nodes[i]['chain'],view.nodes[i]['network']) for i in owned if i in view.nodes}),
        'subjects':{key:list(view.subjects[key]) for key in view.subjects},
        'has_subject':{subject:view.has_subject(subject) for subject in ['missing',TX, 'script:0014'+'22'*20, *case.get('subjects',[])]},
    }


def vectors():
    cases=[]
    sources=[('seed',''),('stored',stored()),('inventory',inventory()),('all',stored()+cache()+inventory()),('derived-ambiguous',stored()+inventory()),('labels',stored()+label()),('datasets',stored()+dataset()),('private-dataset',stored()+dataset(visibility='private')),('all-labels',stored()+label()+dataset()),('ambiguous-owner',stored()+inventory()+inventory(id='utxo2',wallet_id='hot')),('unsafe',stored()+inventory()+cache(payload_json=raw(vout=[]))),('liquid',stored(asset='LBTC')+update('wallets', {'config_json':'{"network":"liquidv1"}'}, "id='cold'"))]
    for name,sql in sources:
        sql=materialize(sql)
        if name == 'derived-ambiguous':
            sql += "UPDATE chain_index_nodes SET output_json=json_set(output_json,'$.ownership_ambiguous',1) WHERE output_json IS NOT NULL;"
        for observer in ['owner','public','disclosed']:
            variants=[{}, {'query':{'observer':observer}}, {'query':{'observer':observer,'mode':'trace','subject':TX}}, {'query':{'observer':observer,'mode':'path','subject':TX,'target':TX}}, {'query':{'observer':observer,'mode':'trace','subject':'missing'}}, {'seed_query':{'chain':'bitcoin','network':'regtest'}}]
            if observer=='public': variants.append({'upgrade':True})
            for number,options in enumerate(variants):
                case={'name':f'{name}-{observer}-{number}', 'operation':'views', 'sql':sql, 'observer':observer, **options}
                conn=database(sql)
                try:case['expected']=evaluate(case,conn)
                finally:conn.close()
                cases.append(case)
    return cases

if __name__=='__main__':
    cases=vectors()
    DEST.write_text(json.dumps(cases,ensure_ascii=True,indent=2)+'\n')
    print(f'{len(cases)} view cases')
