#!/usr/bin/env python3
"""Fixed-source payout creation oracle: exact outputs and complete database rows."""
from contextlib import ExitStack
import datetime
import hashlib
import json
import tempfile
import uuid
from unittest.mock import patch
from export_pairs import ROOT, TABLES, seed, dump, terms, migration, components, handlers, open_db, AppError

def payout(out='d',**kw):return ('transfers.payouts.create',dict(tx_out=out,payout_asset='ETH',payout_amount='0.000000008',**{k:v for k,v in kw.items() if k not in ('payout_asset','payout_amount')} ) | {k:v for k,v in kw.items() if k in ('payout_asset','payout_amount')})
def delete(id):return ('transfers.payouts.delete',dict(payout_id=id))
def main():
    scenarios={
        'single':[payout(notes=' € 🚀 ',payout_fiat_value='1.2345'),payout(),delete('@0'),payout(payout_occurred_at='2025-02-02T01:02:03+01:00')],
        'same':[payout(payout_asset='BTC'),delete('@0'),payout(payout_asset='BTC',policy='taxable')],
        'fee':[payout('a'),delete('@0'),payout('f')],
        'native':[payout('j',out_amount='0.000000008'),delete('@0')],
        'split':[payout(out_amount='0.000000008'),delete('@0')],
        'scope':[payout(profile_scope='other'),payout('d-ext'),payout()],
        'gui':[payout(authored_source='gui',notes='gui',counterparty='party',payout_external_id='ref')],
        'generic':[payout(__sql=["UPDATE profiles SET tax_country='generic',bitcoin_rail_carrying_value=0 WHERE id='p'"]),delete('@0'),payout(policy='carrying-value'),payout(payout_asset='LBTC')],
        'validation':[payout('absent'),payout(kind='bad'),payout(policy='bad'),payout(payout_asset=''),payout(payout_amount='0'),payout(payout_amount='bad'),payout(out_amount='1'),payout(payout_fiat_value='-1'),payout(payout_occurred_at='bad'),payout(notes=12),payout(counterparty=12),payout(payout_external_id=12),payout(payout_amount='0.000000000001'),payout(payout_amount='0.000000000005')],
    }
    scenarios['late-failure']=[]
    for field in ('payout_amount','out_amount','payout_fiat_value'):
        scenarios['edge-'+field]=[payout(**{field:v}) for v in (None,'',' ',True,'NaN','Infinity','-Infinity','bad','１_２e-11','1e9999','-0','0.000000000005')]
    for field in ('payout_amount','payout_fiat_value'):
        for i,v in enumerate(('Infinity','NaN','1e9999','1e20','１_２e-11','0.1','-0','1e-20')):
            scenarios[f'isolated-{field}-{i}']=[payout(**{field:v})]
    for tx in ('zero','neg'):
        scenarios['boundary-'+tx]=[payout(tx)]
    for i,v in enumerate(('banana','NaN0','sNaN12','i_nf','0.00009999999999999999','0.0001','9999999999999999','-0.0')):
        scenarios[f'fiat-format-{i}']=[payout(payout_fiat_value=v)]
    result=[];states={}
    for name,operations in scenarios.items():
        with tempfile.TemporaryDirectory(prefix='kassiber-pair-oracle-') as root:
            c=open_db(root);sql=seed(c);cases=[];ids={}
            if name == 'late-failure':
                version=c.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
                operations=[payout(__sql=[f"CREATE TEMP TRIGGER refuse_payout_creation BEFORE UPDATE OF journal_input_version ON profiles WHEN NEW.id='p' AND NEW.journal_input_version={version+2} BEGIN SELECT RAISE(ABORT,'late payout creation invalidation'); END"])]
            tables=[r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
            untouched={t:dump(c,t) for t in tables if t not in TABLES}
            for i,(operation,raw) in enumerate(operations):
                args={k:ids.get(v,v) if isinstance(v,str) else v for k,v in raw.items()}
                profile=args.pop('profile_scope','p')
                setup=args.pop('__sql',[])
                for statement in setup:c.execute(statement)
                c.commit()
                untouched={t:dump(c,t) for t in tables if t not in TABLES}
                before={t:dump(c,t) for t in tables}
                override=args.pop('__now',None)
                override_id=args.pop('__id',None)
                id=str(uuid.uuid5(uuid.NAMESPACE_DNS,f'pair-oracle-{name}-{i}'));now=(datetime.datetime(2025,6,1,tzinfo=datetime.timezone.utc)+datetime.timedelta(hours=i)).isoformat().replace('+00:00','Z')
                id=override_id or id
                now=override or now
                with ExitStack() as stack:
                    stack.enter_context(patch.object(terms.uuid,'uuid4',return_value=uuid.UUID(id)))
                    for module,field in [(terms,'now_iso'),(migration,'now_iso'),(components,'_now_iso')]:stack.enter_context(patch.object(module,field,return_value=now))
                    try:
                        if operation=='transfers.payouts.create':
                            payload={k:v for k,v in args.items() if k != 'tx_out'}
                            out=handlers.create_direct_swap_payout(c,'w',profile,args.get('tx_out'),**payload)
                            ids[f'@{i}']=out['id']
                        else:out=handlers.delete_direct_swap_payout(c,'w',profile,**args)
                        expected={'result':out}
                    except Exception as e:
                        c.rollback()
                        expected={'error':dict(code=e.code if isinstance(e,AppError) else 'internal_error',message=str(e),hint=getattr(e,'hint',None),details=getattr(e,'details',None))}
                if name == 'late-failure':
                    assert expected['error']['message'] == 'late payout creation invalidation'
                    assert {t:dump(c,t) for t in tables} == before
                rows={}
                for table in TABLES:
                    data=dump(c,table);encoded=json.dumps(data,sort_keys=True,ensure_ascii=True);key=hashlib.sha256(encoded.encode()).hexdigest();states[key]=data;rows[table]=key
                for table,before in untouched.items():assert dump(c,table)==before,(name,i,table)
                cases.append(dict(operation=operation,args=args,setup=setup,profile=profile,id=id,now=now,**expected,rows=rows))
            result.append(dict(name=name,sql=sql,cases=cases))
            c.close()
    (ROOT/'crates/transfers/tests/payout_create.json').write_text(json.dumps(dict(states=states,scenarios=result),indent=2,ensure_ascii=True)+'\n')
    print({s['name']:{'operations':len(s['cases']),'success':sum('result' in c for c in s['cases'])} for s in result})
if __name__=='__main__':main()
