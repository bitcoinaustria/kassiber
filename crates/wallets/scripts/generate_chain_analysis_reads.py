from __future__ import annotations

import base64
import copy
import json
from pathlib import Path
import sys
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_durable import capture, canonical, database, durable
from generate_analysis_sources import stored, inventory, dataset, label, insert, TX, BLOCK, assertion
from kassiber.core import chain_analysis_api as api
from kassiber.core.chain_analysis_cases import digest
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError

DEST = ROOT / 'crates/wallets/tests/data/chain_analysis_reads.json'


def evaluate(case, conn=None):
    own = conn is None
    conn = database(case.get('sql', '')) if own else conn
    result = []
    try:
        with patch.object(durable, 'now_iso', return_value='2026-01-01T00:00:00Z'), patch.object(durable.uuid, 'uuid4', return_value=SimpleNamespace(hex='0123456789abcdef0123456789abcdef')), patch('kassiber.core.chain_analysis_datasets.now_iso', return_value='2026-01-01T00:00:00Z'):
            for action in case['actions']:
                if action['operation'] == 'sql':
                    conn.executescript(action['sql'])
                    value = None
                elif action['operation'] in {'begin', 'rollback', 'commit'}:
                    conn.execute(action['operation'])
                    value = None
                else:
                    kind = 'ui.chain_analysis.' + action['operation']
                    try:
                        value = build_envelope(kind, api.dispatch(conn, kind, action.get('args', {})))
                    except AppError as error:
                        value = build_error_envelope(error.code, str(error), details=error.details, hint=error.hint, retryable=error.retryable)
                result.append({'wire':json.dumps(value,separators=(',',':')),'result': value, 'state': capture(conn), 'in_transaction': conn.in_transaction})
        return result
    finally:
        if own: conn.close()


def vectors():
    cases = []
    def read(op, args=None): return {'operation': op, 'args': {} if args is None else args}
    def add(name, actions, sql=''):
        case = {'name': name, 'sql': sql, 'actions': actions}
        case['expected'] = evaluate(case)
        cases.append(case)
    graph = stored() + inventory() + label() + dataset()
    for observer in ['owner', 'public', 'disclosed']:
        add('query-' + observer, [read('query', {'observer': observer}), read('query', {'mode':'trace','subject':TX,'observer':observer,'include_hypotheses':True}), read('query', {'mode':'path','subject':TX,'target':TX+':0','observer':observer})], graph)
    add('query-cold-warm-and-update', [read('query'),read('query'),{'operation':'sql','sql':"UPDATE transactions SET note='Changed' WHERE id='in';"},read('query')], graph)
    add('query-caller-rollback', [{'operation':'begin'},read('query'),{'operation':'rollback'},read('labels.list')], graph)
    add('query-small-budgets',[read('query',{'node_limit':1,'edge_limit':1,'depth':1})],graph)
    add('query-valid-expected-scope',[read('query',{'expected_scope':{'profile_id':'p','workspace_id':'ws'}})],graph)
    for index,args in enumerate([{'mode':'bad'}, {'subject':TX,'unknown':True}, {'mode':'trace'}, {'mode':'trace','subject':'absent'}, {'depth':True}, {'node_limit':0}, {'observer':'bad'}, {'mode':'path','subject':TX}, {'mode':'trace','subject':TX,'chain':'liquid'}, {'expected_scope':{'workspace_id':'ws','profile_id':'other'}}, {'expected_scope':None}]):
        add(f'query-validation-{index}',[read('query',args)],graph)
    for observer in ['owner','public','disclosed']:
        for subject in [TX,TX+':0','cold','absent']:
            add(f'entropy-{observer}-{subject}',[read('entropy',{'subject':subject,'observer':observer})],graph)
    add('entropy-ambiguous-domain',[read('entropy',{'subject':TX})],graph+insert('transactions',{'id':'liquid-alias','workspace_id':'ws','profile_id':'p','wallet_id':'hot','fingerprint':'liquid-alias','asset':'LBTC','amount':1,'direction':'inbound','occurred_at':'fixed','created_at':'fixed','raw_json':json.dumps({'txid':TX,'chain':'liquid','network':'main'})}))
    for index,args in enumerate([{}, {'subject':TX,'max_states':True},{'subject':TX,'max_duration_ms':30001},{'subject':TX,'max_states':1},{'subject':TX,'scenario':{'kind':'coinjoin_intrafees','max_received_fee_msat':'1000','max_paid_fee_msat':'1000'}},{'subject':TX,'scenario':{'bad':True}},{'subject':TX,'unknown':True}]):
        add(f'entropy-validation-{index}',[read('entropy',args)],graph)
    for index,args in enumerate([{}, {'limit':1},{'limit':True},{'limit':101},{'cursor':'bad'},{'cursor':base64.urlsafe_b64encode(canonical(['other','x','y']).encode()).decode()},{'unknown':True}]):
        add(f'datasets-list-{index}',[read('datasets.list',args)],dataset()+dataset(ident='private',visibility='private',created_at='new'))
    for ident in ['pack','missing']:
        add('datasets-get-'+ident,[read('datasets.get',{'id':ident})],dataset())
    claims = [{'subject':f'tx:{TX}','label':'Declared entity'} for _ in range(3)]
    packs=dataset(claims=claims)+dataset(ident='private',visibility='private')+dataset(ident='revoked',status='revoked')+dataset(ident='pending',status='staging')
    for index,args in enumerate([{'subject':TX,'chain':'bitcoin','network':'main'}, {'subject':TX,'chain':'bitcoin','network':'main','observer':'public','limit':1}, {'label':'Declared entity','dataset_id':'pack','limit':1},{'label':'Declared entity','dataset_id':'private','observer':'public'}, {'label':'Declared entity','dataset_id':'revoked'},{'label':'Declared entity','dataset_id':'pending'},{'subject':TX,'chain':'bitcoin','network':'regtest','dataset_id':'pack'},{'subject':'invalid','chain':'bitcoin','network':'main'},{'label':'x'}, {'subject':TX,'label':'x'},{'subject':TX,'chain':'bitcoin','network':'main','cursor':'bad'},{'subject':TX,'chain':'bitcoin','network':'main','limit':False}]):
        add(f'datasets-query-{index}',[read('datasets.query',args)],packs)
    for operation,args in [('datasets.list',{'limit':1}),('datasets.query',{'label':'Declared entity','dataset_id':'pack','limit':1})]:
        conn=database(packs)
        try:
            first=api.dispatch(conn,'ui.chain_analysis.'+operation,args)
            cursor=first['next_cursor']
            actions=[read(operation,args),read(operation,{**args,'cursor':cursor})]
            if operation=='datasets.query':
                actions += [{'operation':'sql','sql':"UPDATE chain_analysis_datasets SET revision=revision+1 WHERE id='pack';"},read(operation,{**args,'cursor':cursor})]
            add(operation+'-pagination',actions,packs)
        finally:conn.close()
    add('labels-ordered-and-profile-bound',[read('labels.list'), read('labels.list',{'unexpected':True})], label()+label(id='other-label',subject=BLOCK,chain='liquid',network='main',cluster_defining=1)+label(id='foreign-label',profile_id='other'))
    conn=database(graph)
    try:
        result=evaluate({'actions':[read('query')]},conn)[0]['result']['data']
    finally:conn.close()
    for kind in ['nodes','edges','findings','clusters','patterns','exposure','transaction_features']:
        key = 'subject' if kind=='transaction_features' else 'id'
        result.setdefault(kind,[]).extend([{key:'fixture-change','label':'Before'},{key:'fixture-remove','label':'Removed'}])
    case_sql=''
    for ident,title in [('first','Saved 雪'),('second','Next')]:
        value=copy.deepcopy(result)
        if ident=='second':
            value['nodes'][0]['label']='Changed'
            value['nodes'].pop()
            value['coverage']['complete']=True
            for kind in ['nodes','edges','findings','clusters','patterns','exposure','transaction_features']:
                key='subject' if kind=='transaction_features' else 'id'
                value[kind]=[row for row in value[kind] if row[key]!='fixture-remove']
                next(row for row in value[kind] if row[key]=='fixture-change')['label']='After'
                value[kind].append({key:'fixture-add','label':'Added'})
        case_sql+=insert('chain_analysis_cases',{'id':ident,'profile_id':'p','title':title,'query_json':canonical(value['query']),'result_json':canonical(value),'snapshot_id':value['snapshot_id'],'result_digest':digest(value),'created_at':ident})
    for index,(op,args) in enumerate([('cases.list',{}),('cases.list',{'limit':1}),('cases.get',{'id':'first'}),('cases.get',{'id':'missing'}),('cases.compare',{'id':'first'}),('cases.compare',{'id':'first','other_id':'second'}),('cases.get',{'id':''}),('cases.list',{'limit':False}),('cases.list',{'cursor':'bad'})]):
        add(f'cases-read-{index}',[read(op,args)],graph+case_sql)
    c=database(graph+case_sql)
    try: cursor=api.dispatch(c,'ui.chain_analysis.cases.list',{'limit':1})['next_cursor']
    finally:c.close()
    add('cases-pagination',[read('cases.list',{'limit':1,'cursor':cursor})],graph+case_sql)
    numeric_query=copy.deepcopy(result['query']); numeric_query['depth']=4.0
    add('cases-numeric-query-equality',[read('cases.get',{'id':'first'})],graph+case_sql+'UPDATE chain_analysis_cases SET query_json='+repr(canonical(numeric_query))+" WHERE id='first';")
    for column,value in [('result_json','broken'),('query_json','{}'),('result_digest','bad'),('snapshot_id','bad')]:
        add('cases-integrity-'+column,[read('cases.get',{'id':'first'})],graph+case_sql+f"UPDATE chain_analysis_cases SET {column}='{value}' WHERE id='first';")
    add('query-failed-read-rolls-back-index',[read('query',{'mode':'trace','subject':'absent'}),read('labels.list')],graph)
    from generate_analysis_mirror import captured_sql
    snapshots=json.loads((DEST.parent/'analysis_analytics.json').read_text())['snapshots']
    for name in ['diamond','private-change','peel','postmix','attribution-custody-observed']:
        retained=captured_sql(snapshots[name])+"UPDATE chain_index_state SET watermark=(SELECT revision FROM chain_index_clock WHERE id=1);"
        add('query-retained-'+name,[read('query',{'observer':observer,'include_hypotheses':True}) for observer in ['owner','public','disclosed']],retained)
    return cases


if __name__=='__main__':
    data=vectors(); DEST.write_text(json.dumps(data,ensure_ascii=True,sort_keys=True,separators=(',',':'))+'\n'); print(f'{len(data)} cases, {sum(len(c["expected"]) for c in data)} states')
