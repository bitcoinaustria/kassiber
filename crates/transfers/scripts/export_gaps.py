#!/usr/bin/env python3
"""Fixed-source gap authoring oracle with full DB rows and late failure rollback."""
from contextlib import ExitStack
import argparse
import datetime
import hashlib
import json
import tempfile
import uuid
from unittest.mock import patch
from export_pairs import ROOT, STAMP, TABLES as COMPONENT_TABLES, dump, components, open_db, AppError
from kassiber.core import custody_gaps as gaps, custody_gap_reviews as reviews, custody_filed_reports as filed
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION

TABLES = COMPONENT_TABLES + ('custody_gap_reviews','custody_gap_review_relation_sets','custody_gap_review_transactions','custody_filed_report_impacts')

def op(action='create', apply=False, **kw):
    return ('transfers.gaps.apply' if apply else 'transfers.gaps.plan', dict(action=action, **kw))

def both(action='create', **kw):
    return [op(action, **kw),op(action,apply=True,**kw)]

def seed(c, name):
    sql=["INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z')", "INSERT INTO profiles(id,workspace_id,label,created_at,last_processed_at,last_processed_tx_count,ownership_review_counts_json) VALUES('p','w','Book','2025-01-01T00:00:00Z','2025-01-01T00:00:00Z',42,'{\"owned\":7}')", "INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('other','w','Other','2025-01-01T00:00:00Z')"]
    for wallet in ('source','target'):
        sql.append(f"INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('{wallet}','w','p','{wallet}','custom','{STAMP}')")
    rows=[('out','source','outbound',1000,10,0,'2020-01-01T00:00:00Z'),('in','target','inbound',900,0,0,'2021-01-01T00:00:00Z'),('later','target','outbound',50,0,0,'2023-01-01T00:00:00Z')]
    if name=='excess':rows[1]=('in','target','inbound',1100,0,0,'2021-01-01T00:00:00Z')
    if name=='included':rows[0]=('out','source','outbound',1010,10,1,'2020-01-01T00:00:00Z')
    if name in ('split','split-large'):rows=[('out','source','outbound',600,10,0,'2020-01-01T00:00:00Z'),('out2','source','outbound',400,5,0,'2020-01-02T00:00:00Z'),('in','target','inbound',550,0,0,'2021-01-01T00:00:00Z'),('in2','target','inbound',350,0,0,'2021-01-02T00:00:00Z')]
    if name=='split-large':
        rows=[(id,wallet,direction,amount*10**16,fee,included,occurred) for id,wallet,direction,amount,fee,included,occurred in rows]
    for id,wallet,direction,amount,fee,included,occurred in rows:
        sql.append(f"INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at,privacy_boundary) VALUES('{id}','w','p','{wallet}','fp-{id}','ext-{id}','BTC','{direction}',{amount},{fee},{included},'{occurred}','withdrawal','{{}}','{STAMP}','coinjoin')")
    if name in ('filed','late-impact'):
        for year,state in ((2020,'filed'),(2021,'saved'),(2023,'filed'),(2024,'saved')):
            sql.append(f"INSERT INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,period_start_year,period_end_year,content_sha256,classification_summary_json,gain_summary_json,report_scope_json,authored_source,created_at) VALUES('filed-{year}','w','p','tax','{state}',{year},{year},'{('ab'*32)}','{{\"external\":{{\"count\":1,\"amount_msat\":1000}}}}','{{\"gain_loss_exact\":\"42.00\"}}','{{}}','cli','{STAMP}')")
    if name in ('large','large-untyped'):
        if name=='large-untyped':sql.append("UPDATE transactions SET privacy_boundary=NULL")
        sql.append("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<50000) INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,kind,raw_json,created_at) SELECT 'filler-'||i,'w','p','source','filler-fp-'||i,'filler-ext-'||i,'BTC','neutral',1,0,'2019-01-01T00:00:00Z','fee','{}','2019-01-01T00:00:00Z' FROM n")
    if name in ('large','large-untyped'):sql.append('DELETE FROM chain_index_dirty')
    if name.startswith('current') or name=='stale-inputs':
        sql.append("UPDATE profiles SET last_processed_at='2025-01-01T00:00:00Z',last_processed_tx_count=3,last_processed_input_version=journal_input_version WHERE id='p'")
        sql.append(f"UPDATE profiles SET journal_semantics_version={JOURNAL_CALCULATION_SEMANTICS_VERSION} WHERE id='p'")
        ids='{}' if name=='current-malformed' else '["out"]'
        version='journal_input_version+1' if name=='stale-inputs' else 'journal_input_version'
        sql.append(f"INSERT INTO journal_custody_gap_inputs(workspace_id,profile_id,input_version,ignored_ids_json,accounting_ignored_ids_json,created_at) SELECT 'w','p',{version},'{ids}','[]','2025-01-01T00:00:00Z' FROM profiles WHERE id='p'")
    for q in sql:c.execute(q)
    c.commit()
    return sql

def scenarios():
    result={
        'lifecycle':[*both(), *both('reopen',reason='mistake'), *both('revise',reason='fixed'), *both('classify_residual',classification='external_payment'), *both('reopen'), *both('revise')],
        'dismiss':[*both('dismiss',reason='not mine'),*both('dismiss',reason=''),*both(),op('dismiss'),*both('reopen'),*both('dismiss')],
        'included':[*both(),*both('classify_residual',classification='retained_custody')],
        'split':[*both(),*both('classify_residual',classification='suspense_continuation'),*both('classify_residual',classification='external_loss')],
        'excess':[*both(),*both('dismiss'),op('revise')],
        'filed':[*both(),*both('classify_residual',classification='external_gift'),*both('reopen'),*both('revise')],
        'late-review':[*both()],
        'late-impact':[*both()],
        'refusals':[op('unknown'),op('reopen'),op('classify_residual',classification='nonsense'),op('classify_residual',classification='external_payment'),op('create',gap_id='absent'),op('create',apply=True,expected_input_version=-1),op('create',apply=True,expected_input_version=True),op('create',apply=True,expected_input_version=999),*both(),op('create'),op('create',apply=True),op('revise'),op('classify_residual',classification=' EXTERNAL-DISPOSAL '),op('classify_residual',apply=True,classification=' EXTERNAL-DISPOSAL '),op('classify_residual',classification='external_gift'),*both('reopen'),op('reopen'),op('classify_residual',classification='retained_custody')],
    }
    result.update({
        'split-large':[*both(),*both('classify_residual',classification='external_loss')],
        'large':[op()],
        'large-untyped':[op()],
        'current-inputs':[op()],
        'current-malformed':[*both()],
        'stale-inputs':[*both()],
        'evidence-changed':[*both(),op('reopen',__sql=["UPDATE transactions SET amount=1001 WHERE id='out'"]),op('classify_residual',classification='external_loss')],
        'conflicting-history':[*both(),op('reopen',__sql=["INSERT INTO custody_gap_reviews SELECT 'concurrent',workspace_id,profile_id,gap_id,revision,candidate_fingerprint,action,event_kind,component_id,authored_source,reason,snapshot_json,created_at FROM custody_gap_reviews"])],
        'date-overflow':[op(__sql=["UPDATE transactions SET occurred_at='0001-01-01T00:00:00+01:00' WHERE id='out'"])],
        'digit-amount':[op(__sql=["UPDATE transactions SET amount='²' WHERE id='out'"])],
        'digit-fee':[op(__sql=["UPDATE transactions SET fee='²' WHERE id='out'"])],
        'unicode-downstream':[*both(__sql=["UPDATE transactions SET occurred_at='２０２３-01-01T00:00:00Z' WHERE id='later'"])],
        'digit-downstream':[op(__sql=["UPDATE transactions SET occurred_at='²²²²-01-01T00:00:00Z' WHERE id='later'"])],
        'early-downstream':[op(__sql=["UPDATE transactions SET occurred_at='9' WHERE id='later'"])],
        'early-source':[op(__sql=["UPDATE transactions SET occurred_at='1899-01-01T00:00:00Z' WHERE id='out'"])],
    })
    for classification in ('external_disposal','external_gift','external_loss','retained_custody','suspense_continuation'):
        result['classify-'+classification]=[*both(),*both('classify_residual',classification=classification)]
    return result

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true',help='compare regenerated bytes without writing')
    options=parser.parse_args()
    states={};out=[]
    for name,operations in scenarios().items():
        with tempfile.TemporaryDirectory(prefix='kassiber-gap-oracle-') as root:
            c=open_db(root);sql=seed(c,name);found,_=gaps.load_gap_search_result(c,'p',include_journal_claims=False);gap=max(found.candidates,key=lambda v:(v.source_total_msat,v.return_total_msat)).gap_id
            if name.startswith('late-'):
                table='custody_gap_reviews' if name=='late-review' else 'custody_filed_report_impacts'
                q=f"CREATE TEMP TRIGGER refuse_gap_write BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT,'late gap {table}'); END";c.execute(q);sql.append(q)
            tables=[r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")];cases=[]
            for i,(operation,raw) in enumerate(operations):
                args={'gap_id':gap,'authored_source':'cli',**raw}
                setup=args.pop('__sql',[])
                for q in setup:c.execute(q)
                c.commit()
                if operation.endswith('apply'):args.setdefault('expected_input_version',c.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0])
                before={t:dump(c,t) for t in tables};id=str(uuid.uuid5(uuid.NAMESPACE_DNS,f'gap-oracle-{name}-{i}'));now=(datetime.datetime(2025,6,1,tzinfo=datetime.timezone.utc)+datetime.timedelta(hours=i)).isoformat().replace('+00:00','Z')
                with ExitStack() as stack:
                    stack.enter_context(patch.object(reviews.uuid,'uuid4',return_value=uuid.UUID(id)))
                    for module,field in ((reviews,'now_iso'),(filed,'now_iso'),(components,'_now_iso')):stack.enter_context(patch.object(module,field,return_value=now))
                    try:
                        candidate=gaps.find_gap_candidate(c,'p',args['gap_id']) if args['action'] in ('create','revise','dismiss') else None
                        kw={k:v for k,v in args.items() if k!='expected_input_version'}
                        if operation.endswith('plan'):result=reviews.public_review_plan(reviews.plan_review(c,workspace_id='w',profile_id='p',candidate=candidate,**kw))
                        else:result=reviews.apply_review(c,workspace_id='w',profile_id='p',candidate=candidate,expected_input_version=args['expected_input_version'],**kw)
                        expected={'result':result}
                    except Exception as e:
                        c.rollback();expected={'error':dict(code=e.code if isinstance(e,AppError) else 'internal_error',message=str(e),hint=getattr(e,'hint',None),details=getattr(e,'details',None))}
                if operation.endswith('plan') or 'error' in expected:assert {t:dump(c,t) for t in tables}==before,(name,i,expected)
                row_states={}
                for table in TABLES:
                    data=dump(c,table);key=hashlib.sha256(json.dumps(data,sort_keys=True,ensure_ascii=True).encode()).hexdigest();states[key]=data;row_states[table]=key
                for table in tables:
                    if table not in TABLES:assert dump(c,table)==before[table],(name,i,table)
                cases.append(dict(operation=operation,args=args,setup=setup,profile='p',id=id,now=now,**expected,rows=row_states))
            out.append(dict(name=name,sql=sql,cases=cases));c.close()
    for filename,selected in (('gaps.json',[s for s in out if s['name'] not in ('large','large-untyped')]),('gap_worklists.json',[s for s in out if s['name'] in ('large','large-untyped')])):
        used={key for s in selected for case in s['cases'] for key in case['rows'].values()}
        path=ROOT/'crates/transfers/tests'/filename
        content=json.dumps(dict(tables=TABLES,states={key:state for key,state in states.items() if key in used},scenarios=selected),indent=2,ensure_ascii=True)+'\n'
        if options.check:
            if path.read_text()!=content:raise SystemExit(f'{filename} differs from the current Python oracle')
        else:path.write_text(content)
    print({s['name']:{'operations':len(s['cases']),'success':sum('result' in c for c in s['cases'])} for s in out})

if __name__=='__main__':main()
