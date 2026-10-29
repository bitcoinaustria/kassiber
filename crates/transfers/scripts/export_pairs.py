#!/usr/bin/env python3
"""Deterministic pair/revision/refusal oracle, including every application table."""
from contextlib import ExitStack
import ast
import datetime
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import uuid
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
from kassiber.cli import handlers
from kassiber.core import custody_authored_migration as migration, custody_components as components, custody_review_terms as terms
from kassiber.db import open_db
from kassiber.errors import AppError

STAMP='2025-01-01T00:00:00Z'
TABLES=('chain_index_clock','chain_index_dirty','profiles','custody_components','custody_component_legs','custody_component_allocations','custody_component_economic_terms','custody_component_transaction_memberships','custody_authored_evidence_snapshots','custody_component_evidence_commitments')

def dump(c,table):
    quoted='"'+table.replace('"','""')+'"'
    rows=[dict(r) for r in c.execute(f'SELECT * FROM {quoted}')]
    return sorted(rows,key=lambda r:json.dumps(r,sort_keys=True,ensure_ascii=True))

def seed(c):
    sql=["INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z')", "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at,last_processed_at,last_processed_tx_count,ownership_review_counts_json) VALUES('p','w','Book','EUR','at','2025-01-01T00:00:00Z','2025-01-01T00:00:00Z',42,'{\"owned\":7}')", "INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('other','w','Other','2025-01-01T00:00:00Z')"]
    for wallet in ('source','target','liquid'):
        sql.append(f"INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('{wallet}','w','p','{wallet}','custom','{STAMP}')")
    for id,wallet,direction,amount,asset,fee,includes,external,raw in [
        ('a','source','outbound',1000,'BTC',10,0,'a-ext','{}'),('b','target','inbound',600,'BTC',0,0,'b-ext','{}'),('c','target','inbound',400,'BTC',0,0,'c-ext','{}'),('d','source','outbound',1000,'BTC',0,0,'d-ext','{}'),('e','target','inbound',1000,'BTC',0,0,'e-ext','{}'),('f','source','outbound',1010,'BTC',10,1,'f-ext','{}'),('g','liquid','inbound',800,'LBTC',0,0,'g-ext','{}'),('h','target','inbound',700,'ETH',0,0,'h-ext','{}'),('j','source','outbound',1000,'BTC',0,0,'j-ext','{}'),('k','target','inbound',1000,'BTC',0,0,'k-ext','{}'),('l','target','inbound',200,'BTC',0,0,'j-ext','{}'),('zero','source','outbound',0,'BTC',0,0,'zero-ext','{}'),('neg','source','outbound',10,'BTC',20,1,'neg-ext','{}'),('bad','target','inbound',1000,'BTC',1,1,'bad-ext','{}'),('excluded','target','inbound',1000,'BTC',0,0,'excluded-ext','{}'),('net','target','inbound',1000,'BTC',0,0,'net-ext','{"network":"regtest"}'),('contradiction','target','inbound',1000,'BTC',0,0,'contradiction-ext','{"chain":"liquid"}'),
    ]:
        vals=(id,wallet,f'fp-{id}',external,asset,direction,amount,fee,includes,raw)
        sql.append("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at) VALUES(%s,'w','p',%s,%s,%s,%s,%s,%s,%s,%s,'2025-02-01T00:00:00Z','deposit',%s,'2025-01-01T00:00:00Z')"%tuple("'"+x.replace("'","''")+"'" if isinstance(x,str) else str(x) for x in vals))
    sql.append("UPDATE transactions SET excluded=1 WHERE id='excluded'")
    for q in sql:c.execute(q)
    c.commit()
    return sql

def pair(out='d',inbound='e',**kw):return ('transfers.pair',dict(tx_out=out,tx_in=inbound,**kw))
def update(id,**kw):return ('transfers.update',dict(pair_id=id,**kw))
def unpair(id):return ('transfers.unpair',dict(pair_id=id))

def export_validation_order():
    """Wire field order only; validation continues to use the Rust custody stage."""
    tree=ast.parse((ROOT/'kassiber/core/custody_components.py').read_text())
    orders={}
    for function in tree.body:
        if not isinstance(function,ast.FunctionDef):continue
        bindings={target.id:node.value for node in ast.walk(function) if isinstance(node,ast.Assign) and isinstance(node.value,ast.Dict) for target in node.targets if isinstance(target,ast.Name)}
        def keys(node):
            fields=[]
            for key,value in zip(node.keys,node.values):
                if isinstance(key,ast.Constant) and isinstance(key.value,str):fields.append(key.value)
                elif key is None and isinstance(value,ast.Name) and value.id in bindings:fields.extend(keys(bindings[value.id]))
                else:raise ValueError('nonliteral validation wire field')
            return fields
        for node in ast.walk(function):
            if not isinstance(node,ast.Dict):continue
            code=next((value.value for key,value in zip(node.keys,node.values) if isinstance(key,ast.Constant) and key.value=='code' and isinstance(value,ast.Constant) and isinstance(value.value,str)),None)
            if code is not None:
                order=orders.setdefault(code,[])
                for field in keys(node):
                    if field not in order:order.append(field)
    (ROOT/'crates/custody/src/components/review_field_order.json').write_text(json.dumps(orders,indent=2,sort_keys=True)+'\n')

def main():
    export_validation_order()
    scenarios={
        'single': [pair(notes=' € 🚀 '),pair(),update('@0'),update('@0',notes='new'),update('@0',notes='new'),update('@0',notes=''),update('@0',kind='coinjoin'),update('@0',kind='peg-in'),update('@0',kind='manual'),update('@0',policy='taxable'),unpair('@0'),unpair('@0'),update('@0',notes='absent'),unpair('absent')],
        'group':[pair('a','b'),pair('a','c'),update('@0',notes='changed'),update('@1',kind='coinjoin'),update('@0',kind='peg-in'),pair('f','c'),unpair('@0'),unpair('@1'),pair('d','e'),unpair('@8')],
        'conversion':[pair('f','g',kind='chain-swap'),pair('f','h'),update('@0',policy='taxable'),update('@0',policy='carrying-value'),update('@0',notes=''),unpair('@0'),pair('j','h',out_amount='0.000000008'),update('@6',notes='native residual'),unpair('@6')],
        'validation':[pair('absent','e'),pair('d','absent'),pair('e','d'),pair(kind='bad'),pair(policy='bad'),pair(pair_source='bad'),pair(policy='taxable'),pair(out_amount='1'),pair('d','h',out_amount='0'),pair('d','h',out_amount='-1'),pair('d','h',out_amount='NaN'),pair('d','h',out_amount='Infinity'),pair('d','h',out_amount='bad'),pair('d','h',out_amount='0.00000001001'),pair('zero','e'),pair('neg','e'),pair('d','bad'),pair('d','excluded'),pair('d','net'),pair('d','contradiction'),pair(notes=12),pair(confidence_at_pair=12),pair('d','h',out_amount='0.000000000001'),pair('d','h',out_amount='0.000000000005')],
        'scope': [pair(),('transfers.update',dict(pair_id='@0',notes='no',profile_scope='other')),('transfers.unpair',dict(pair_id='@0',profile_scope='other')),pair('d-ext','e-ext'),update('@0',kind='invalid'),update('@0',policy='invalid'),update('@0',notes=12)],
    }
    scenarios['refund']=[pair('d','b',kind='swap-refund'),update('@0',kind='manual'),update('@0',kind='swap-refund'),unpair('@0')]
    scenarios['generic']=[pair('d','h',__sql=["UPDATE profiles SET tax_country='generic',bitcoin_rail_carrying_value=0 WHERE id='p'"]),update('@0',policy='carrying-value'),unpair('@0'),pair('f','g'),update('@3',policy='carrying-value'),unpair('@3')]
    scenarios['retarget']=[pair(notes='base'),update('@0',notes='first',__now=STAMP),update('@0',notes='second',__now=STAMP),update('@0',notes='first',__now=STAMP),update('@0',notes='second',__now=STAMP)]
    scenarios['raw-active']=[pair(),pair('d','h',__sql=["DELETE FROM custody_component_transaction_memberships WHERE profile_id='p'"]),unpair('@0')]
    scenarios['ambiguity']=[pair('d-ext','e',__sql=["UPDATE transactions SET external_id='d-ext' WHERE id='f'"]),pair('d','e')]
    scenarios['validation'] += [pair('d','g',out_amount=v) for v in ('', '-Infinity',True,'1e9999')]
    scenarios['network']=[pair('d','net',__sql=["UPDATE transactions SET raw_json='{\"network\":\"main\"}' WHERE id='d'"]),pair('d','contradiction'),pair('d','e')]
    scenarios['separate-network-fee']=[pair('a','h'),update('@0',kind='peg-out'),unpair('@0')]
    scenarios['independent-memberships']=[pair('d','e'),pair('f','g'),unpair('@0'),unpair('@1')]
    scenarios['partial-group']=[pair('a','b'),pair('a','c',__sql=["UPDATE transactions SET amount=200 WHERE id='c'"]),update('@0',notes='retained residual'),unpair('@1'),unpair('@0')]
    scenarios['split-fee']=[pair('a','h',out_amount='0.000000008'),unpair('@0')]
    scenarios['draft']=[pair(),update('@0',notes='refused',__sql=["INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at) SELECT 'existing-draft',lineage_id,workspace_id,profile_id,revision+1,component_type,'draft',created_at FROM custody_components WHERE state='active'"])]
    scenarios['lineage']=[pair(),unpair('@0'),pair(__id='@0')]
    scenarios['ambiguous-residual']=[pair('j','h',out_amount='0.000000008',__sql=["INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at) SELECT 'l2',workspace_id,profile_id,wallet_id,'fp-l2',external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at FROM transactions WHERE id='l'"])]
    scenarios['asset-id']=[pair(__sql=["UPDATE transactions SET asset='ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789' WHERE id='d'", "UPDATE transactions SET asset='abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789' WHERE id='e'"])]
    scenarios['drift']=[pair('a','b'),pair('a','c'),update('@0',notes='drifted',__sql=["UPDATE transactions SET amount=900 WHERE id='a'"])]
    result=[];states={}
    for name,operations in scenarios.items():
        with tempfile.TemporaryDirectory(prefix='kassiber-pair-oracle-') as root:
            c=open_db(root);sql=seed(c);cases=[];ids={}
            tables=[r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
            untouched={t:dump(c,t) for t in tables if t not in TABLES}
            for i,(operation,raw) in enumerate(operations):
                args={k:ids.get(v,v) if isinstance(v,str) else v for k,v in raw.items()}
                profile=args.pop('profile_scope','p')
                setup=args.pop('__sql',[])
                for statement in setup:c.execute(statement)
                c.commit()
                untouched={t:dump(c,t) for t in tables if t not in TABLES}
                override=args.pop('__now',None)
                override_id=args.pop('__id',None)
                id=str(uuid.uuid5(uuid.NAMESPACE_DNS,f'pair-oracle-{name}-{i}'));now=(datetime.datetime(2025,6,1,tzinfo=datetime.timezone.utc)+datetime.timedelta(hours=i)).isoformat().replace('+00:00','Z')
                id=override_id or id
                now=override or now
                with ExitStack() as stack:
                    stack.enter_context(patch.object(terms.uuid,'uuid4',return_value=uuid.UUID(id)))
                    for module,field in [(terms,'now_iso'),(migration,'now_iso'),(components,'_now_iso')]:stack.enter_context(patch.object(module,field,return_value=now))
                    try:
                        if operation=='transfers.pair':
                            payload={k:v for k,v in args.items() if k not in ('tx_out','tx_in')}
                            out=handlers.create_transaction_pair(c,'w',profile,args.get('tx_out'),args.get('tx_in'),**payload)
                            ids[f'@{i}']=out['id']
                        elif operation=='transfers.update':out=handlers.update_transaction_pair(c,'w',profile,**args)
                        else:out=handlers.delete_transaction_pair(c,'w',profile,**args)
                        expected={'result':out}
                    except Exception as e:
                        c.rollback()
                        expected={'error':dict(code=e.code if isinstance(e,AppError) else 'internal_error',message=str(e),hint=getattr(e,'hint',None),details=getattr(e,'details',None))}
                rows={}
                for table in TABLES:
                    data=dump(c,table);encoded=json.dumps(data,sort_keys=True,ensure_ascii=True);key=hashlib.sha256(encoded.encode()).hexdigest();states[key]=data;rows[table]=key
                for table,before in untouched.items():assert dump(c,table)==before,(name,i,table)
                cases.append(dict(operation=operation,args=args,setup=setup,profile=profile,id=id,now=now,**expected,rows=rows))
            result.append(dict(name=name,sql=sql,cases=cases))
            c.close()
    (ROOT/'crates/transfers/tests/pairs.json').write_text(json.dumps(dict(states=states,scenarios=result),indent=2,ensure_ascii=True)+'\n')
    print({s['name']:{'operations':len(s['cases']),'success':sum('result' in c for c in s['cases'])} for s in result})
if __name__=='__main__':main()
