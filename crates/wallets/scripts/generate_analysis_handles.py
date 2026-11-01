from __future__ import annotations
import json
from collections import OrderedDict
from contextlib import contextmanager
from pathlib import Path
import sys
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT));sys.path.insert(0,str(Path(__file__).parent))
from generate_analysis_views import materialize, database
from generate_analysis_sources import stored, inventory, TX
from kassiber.core import chain_analysis_ai as ai
from kassiber.core.chain_analysis.projection import _View
from kassiber.errors import AppError

DEST=ROOT/'crates/wallets/tests/data/analysis_handles.json'


def evaluate(case,conn):
    issued={};results=[]
    @contextmanager
    def read_index(conn,profile):
        state=dict(conn.execute('SELECT * FROM chain_index_state WHERE profile_id=?',(profile,)).fetchone())
        yield _View(conn,profile,state)
    def expand(value):
        if isinstance(value,dict):return {k:expand(v) for k,v in value.items()}
        if isinstance(value,list):return [expand(v) for v in value]
        return issued.get(value[1:],value) if isinstance(value,str) and value.startswith('$') else value
    with patch.object(ai,'_PROCESS_KEY',bytes([7])*32),patch.object(ai,'_REFERENCES',OrderedDict()),patch.object(ai,'_REFERENCE_CAPACITY',int(case.get('capacity','32768'))),patch.object(ai,'read_index',read_index):
        for action in case['actions']:
            scope=action.get('book','book-p').encode()
            try:
                op=action['operation']
                if op=='issue':
                    result=ai._reference(scope,action['value']);issued[action.get('name','ref')]=result
                elif op=='restart':
                    ai._PROCESS_KEY=bytes([8])*32;ai._REFERENCES.clear();result=None
                elif op=='clear':
                    ai._REFERENCES.clear();result=None
                elif op=='decode':
                    with patch.object(ai,'_scope',return_value=scope):result=ai.decode_ai_args(conn,'p',expand(action['args']))
                else:raise AssertionError(op)
            except AppError as error:result={'error':error.code}
            results.append(result)
    return results


def vectors():
    sql=materialize(stored()+inventory())
    cases=[]
    values=[TX,f'tx:{TX}',f'out:{TX}:0',f'{TX}:0','script:0014'+'22'*20,'missing','雪','private-fingerprint']
    def add(name,actions,**extra):
        case={'name':name,'operation':'handles','sql':sql,'actions':actions,**extra}
        conn=database(sql)
        try:case['expected']=evaluate(case,conn)
        finally:conn.close()
        cases.append(case)
    for i,value in enumerate(values):
        for name,args in [('subject',{'subject':'$ref'}),('target',{'target':'$ref'}),('nested',{'query':{'subject':'$ref'},'targets':[{'target':'$ref'}],'subject':['$ref']}),('other-field',{'node_id':'$ref','subject':value}),('book-switch',{'subject':'$ref'})]:
            add(f'{i}-{name}',[{'operation':'issue','value':value},{'operation':'decode','args':args,**({'book':'book-other'} if name=='book-switch' else {})}])
    for op in ['restart','clear']:
        add(op,[{'operation':'issue','value':TX},{'operation':op},{'operation':'decode','args':{'subject':'$ref'}}])
    add('unissued',[{'operation':'decode','args':{'subject':ai._REFERENCE_PREFIX+'0'*64}}])
    for value in [None,[],1,'broken']:
        add(f'invalid-{value}',[{'operation':'decode','args':value}])
    add('eviction',[{'operation':'issue','value':TX,'name':'one'},{'operation':'issue','value':f'tx:{TX}','name':'two'},{'operation':'decode','args':{'subject':'$one'}},{'operation':'issue','value':f'out:{TX}:0','name':'three'},{'operation':'decode','args':{'subject':'$one'}},{'operation':'decode','args':{'subject':'$two'}},{'operation':'decode','args':{'subject':'$three'}}],capacity='2')
    return cases

if __name__=='__main__':
    cases=vectors();DEST.write_text(json.dumps(cases,indent=2)+'\n');print(f'{len(cases)} handle cases')
