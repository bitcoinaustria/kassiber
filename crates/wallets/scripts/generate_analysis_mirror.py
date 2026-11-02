from __future__ import annotations
import copy
import hashlib
import json
from collections import OrderedDict
from pathlib import Path
from types import SimpleNamespace
import sys
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT));sys.path.insert(0,str(Path(__file__).parent))
from generate_analysis_views import database, materialize
from generate_analysis_sources import stored, inventory, dataset, label
from generate_analysis_ingestion import q
from kassiber.core.chain_analysis.index import AnalysisIndex, thaw
from kassiber.core.chain_analysis.projection_store import VERSION, _save, encoded
from kassiber.core.chain_analysis.projection import _View
from kassiber.core import privacy_mirror, chain_analysis_ai

DEST=ROOT/'crates/wallets/tests/data/analysis_mirror.json'


def captured_sql(snapshot):
    conn=database()
    index=AnalysisIndex(**snapshot)
    try:
        _save(conn,'p',index,set(index.nodes))
        for row in index.labels:
            row=thaw(row)
            node_ids=row.get('node_ids') or index.subjects.get(row.get('subject'),())
            row['node_ids']=sorted(ident for ident in node_ids if ident in index.nodes and (index.nodes[ident]['chain'],index.nodes[ident]['network'])==(row.get('chain'),row.get('network')))
            conn.execute('INSERT INTO chain_index_labels VALUES(?,?,?)',('p',str(row['id']),encoded(row)))
            conn.executemany('INSERT INTO chain_index_label_nodes VALUES(?,?,?)',(('p',str(row['id']),ident) for ident in row['node_ids']))
        # A captured view is a current materialization of the durable index.
        conn.execute('INSERT INTO chain_index_state VALUES(?,?,?,?,?,?,?,?)',('p','fixture',VERSION,0,1,index.snapshot_id,encoded(index.coverage),None))
        sql=''
        for (table,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'chain_index_%' ORDER BY name"):
            sql+=f'DELETE FROM {table};'
            for row in sorted(conn.execute(f'SELECT * FROM {table}'),key=lambda row:tuple(q(v) for v in row)):
                sql+=f'INSERT INTO {table} VALUES('+','.join(q(v) for v in row)+');'
        return sql
    finally:conn.close()


def evaluate(case,conn):
    state=dict(conn.execute("SELECT * FROM chain_index_state WHERE profile_id='p'").fetchone())
    view=_View(conn,'p',state)
    times=iter([0.0,*[value/1000 for value in case.get('elapsed_ms',[])]])
    clock=lambda:next(times,case.get('elapsed_ms',[0])[-1]/1000)
    with patch.object(privacy_mirror,'time',SimpleNamespace(monotonic=clock)),patch('kassiber.core.chain_analysis.entropy.time.monotonic',return_value=0),patch.object(chain_analysis_ai,'_PROCESS_KEY',bytes([7])*32),patch.object(chain_analysis_ai,'_REFERENCES',OrderedDict()),patch.object(chain_analysis_ai,'_scope',return_value=b'book-p'):
        return privacy_mirror._mirror_snapshot(conn,'p',view,redacted=case.get('redacted',False))


def vectors():
    cases=[]
    def add(name,sql,**options):
        for redacted in [False,True]:
            case={'name':f'{name}-{redacted}','operation':'mirror','sql':sql,'redacted':redacted,**options}
            conn=database(sql)
            try:case['expected']=evaluate(case,conn)
            finally:conn.close()
            cases.append(case)
    for name,sql in [('seed',''),('stored',stored()),('owned',stored()+inventory()),('public-claims',stored()+inventory()+dataset()),('private-claims',stored()+inventory()+dataset(visibility='private')),('private-label',stored()+inventory()+label())]:
        add(name,materialize(sql))
    snapshots=json.loads((DEST.parent/'analysis_analytics.json').read_text())['snapshots']
    for name,snapshot in snapshots.items():
        if name in {'dense-labels'}:continue
        add(name,captured_sql(snapshot))
        if not name.startswith('labels-'):
            owned=copy.deepcopy(snapshot)
            for ident,fact in owned['output_facts'].items():
                fact.update(ownership_known=True,ownership_ambiguous=False)
                owned['nodes'][ident]['wallet_ids']=['cold']
            add(f'{name}-owned',captured_sql(owned))
    base=materialize(stored()+inventory())
    add('entropy-time-budget',base,elapsed_ms=[310])
    books={}
    for case in cases:
        sql=case.pop('sql')
        key=hashlib.sha256(sql.encode()).hexdigest()
        books[key]=sql
        case['book']=key
    return {'books':books,'cases':cases}

if __name__=='__main__':
    cases=vectors();DEST.write_text(json.dumps(cases,indent=2)+'\n');print(f'{len(cases["cases"])} mirror cases, {len(cases["books"])} captured books')
