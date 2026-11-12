#!/usr/bin/env python3
"""Gap display oracle over authored history, canonical issues and bounded books."""
import argparse
import base64
import json
import sqlite3
import tempfile
from unittest.mock import patch
from export_pairs import ROOT, open_db, AppError
from export_component_reads import dump
from kassiber.core import custody_authored_migration, custody_gaps as gaps
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION


def template():
    target=sqlite3.connect(':memory:')
    connect=sqlite3.connect
    clock=connect(':memory:')
    def frozen_connect(*args,**kwargs):
        c=connect(*args,**kwargs)
        def strftime(*values):
            values=tuple('2025-01-01T00:00:00Z' if v=='now' else v for v in values)
            return clock.execute('SELECT strftime('+','.join('?' for _ in values)+')',values).fetchone()[0]
        c.create_function('strftime',-1,strftime)
        return c
    with tempfile.TemporaryDirectory(prefix='gap-read-template-') as root:
        # SQLite's clock and the Python one the pair fee review audit
        # (ca630b9a6) stamps on open both read the frozen instant.
        with patch('kassiber.db.sqlite3.connect',side_effect=frozen_connect), \
                patch.object(custody_authored_migration,'now_iso',return_value='2025-01-01T00:00:00Z'):
            c=open_db(root)
        c.execute("UPDATE settings SET value='00000000000000000000000000000000' WHERE key='database_instance_id'")
        c.commit();c.backup(target);c.close()
    clock.close()
    return target


def insert(c,table,row):
    return 'INSERT INTO "'+table+'"('+','.join('"'+k+'"' for k in row)+') VALUES('+','.join(c.execute('SELECT quote(?)',(v,)).fetchone()[0] for v in row.values())+')'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    options=parser.parse_args()
    base=template()
    base_sql=[q for q in base.iterdump() if q.startswith('INSERT INTO')]
    base_sql.sort(key=lambda q:(not q.startswith('INSERT INTO "chain_index_clock"'),q))
    output=[]
    for filename in ('gaps','gap_worklists'):
        source=json.loads((ROOT/'crates/transfers/tests'/f'{filename}.json').read_text())
        for scenario in source['scenarios']:
            chosen={-1,len(scenario['cases'])-1}
            chosen.update(i for i,case in enumerate(scenario['cases']) if case['operation'].endswith('apply') and 'result' in case)
            seen=set()
            for i in sorted(chosen):
                case=scenario['cases'][i] if i>=0 else None
                setup=[q for prior in scenario['cases'][:i+1] for q in prior.get('setup',[]) if 'CREATE TEMP TRIGGER' not in q]
                signature=json.dumps([case['rows'] if case else None,setup],sort_keys=True)
                if signature in seen:continue
                seen.add(signature)
                c=sqlite3.connect(':memory:');base.backup(c);c.row_factory=sqlite3.Row
                sql=[*base_sql,*[q for q in scenario['sql'] if 'CREATE TEMP TRIGGER' not in q],*setup]
                # Hydrate exact authored states while retaining the initial raw
                # observation SQL, including the compact 50,000-row worklists.
                for q in sql[len(base_sql):]:c.execute(q)
                c.commit();c.execute('PRAGMA foreign_keys=OFF')
                if case:
                    for table in reversed(source['tables']):
                        q='DELETE FROM "'+table+'"';c.execute(q);sql.append(q)
                    order=['chain_index_clock','profiles','custody_components','custody_component_legs','custody_authored_evidence_snapshots','custody_component_allocations','custody_component_economic_terms','custody_component_evidence_commitments','custody_component_transaction_memberships','custody_gap_reviews','custody_gap_review_relation_sets','custody_gap_review_transactions','custody_filed_report_impacts']
                    for table in order:
                        for row in source['states'][case['rows'][table]]:
                            q=insert(c,table,row);c.execute(q);sql.append(q)
                    for table in ('profiles','chain_index_clock'):
                        for row in source['states'][case['rows'][table]]:
                            fields=[k for k in row if k!='id']
                            q='UPDATE "'+table+'" SET '+','.join('"'+k+'"='+c.execute('SELECT quote(?)',(row[k],)).fetchone()[0] for k in fields)+' WHERE id='+c.execute('SELECT quote(?)',(row['id'],)).fetchone()[0]
                            c.execute(q);sql.append(q)
                    c.execute('DELETE FROM chain_index_dirty');sql.append('DELETE FROM chain_index_dirty')
                    for row in source['states'][case['rows']['chain_index_dirty']]:
                        q=insert(c,'chain_index_dirty',row);c.execute(q);sql.append(q)
                c.commit();c.execute('PRAGMA foreign_keys=ON')
                assert not c.execute('PRAGMA foreign_key_check').fetchall(),scenario['name']
                variants=[('base',[])]
                if scenario['name']=='lifecycle' and i==1:
                    variants.append(('review-conflict',["INSERT INTO custody_gap_reviews SELECT 'concurrent-conflict',workspace_id,profile_id,gap_id,revision,candidate_fingerprint,'dismissed',event_kind,component_id,authored_source,reason,snapshot_json,created_at FROM custody_gap_reviews"]))
                    current=["UPDATE profiles SET last_processed_at='2025-01-01',last_processed_tx_count=3,last_processed_input_version=journal_input_version WHERE id='p'", f"UPDATE profiles SET journal_semantics_version={JOURNAL_CALCULATION_SEMANTICS_VERSION} WHERE id='p'", "INSERT INTO journal_custody_gap_inputs(workspace_id,profile_id,input_version,ignored_ids_json,accounting_ignored_ids_json,created_at) SELECT 'w','p',journal_input_version,'[]','[]','2025-01-01' FROM profiles WHERE id='p'"]
                    variants += [('resolved-current',current),('resolved-drift',current+["UPDATE transactions SET amount=1001 WHERE id='out'"])]
                if scenario['name']=='dismiss' and i==1:
                    variants.append(('dismiss-drift',["UPDATE transactions SET amount=1001 WHERE id='out'"]))
                if scenario['name']=='lifecycle' and i==-1:
                    issue=lambda id,asset,amount:f"INSERT INTO journal_quantity_issues(issue_id,workspace_id,profile_id,issue_type,state,asset,amount_msat,reason,created_at) VALUES('{id}','w','p','missing','custody_suspense',{asset},{amount},'unknown custody','2025-01-01T00:00:00Z')"
                    variants += [
                        ('current',["UPDATE profiles SET last_processed_at='2025-01-01',last_processed_tx_count=3,last_processed_input_version=journal_input_version WHERE id='p'", f"UPDATE profiles SET journal_semantics_version={JOURNAL_CALCULATION_SEMANTICS_VERSION} WHERE id='p'"]),
                        ('issues',[issue('btc',"'BTC'",123),issue('eth',"'ETH'",456),issue('unknown','NULL','NULL')]),
                        ('unprocessed',["UPDATE profiles SET last_processed_at=NULL WHERE id='p'"]),
                        ('empty',["DELETE FROM transactions"]),
                        ('missing-issues',["DROP TABLE journal_quantity_issues"]),
                        ('competing',["INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at,privacy_boundary) SELECT 'in-copy',workspace_id,profile_id,wallet_id,'fp-copy','ext-copy',asset,direction,amount,fee,amount_includes_fee,occurred_at,kind,raw_json,created_at,privacy_boundary FROM transactions WHERE id='in'"]),
                    ]
                    variants.append(('current-issues',next(q for name,q in variants if name=='current')+next(q for name,q in variants if name=='issues')))
                for variant,extra in variants:
                    c.execute('SAVEPOINT variant')
                    for q in extra:c.execute(q)
                    operations=[('transfers.gaps.list',kw) for kw in ({},dict(limit=1),dict(limit=500),dict(limit=0),dict(limit=True),dict(limit=501),dict(cursor='0'),dict(cursor=''),dict(cursor='bad'),dict(cursor=1),dict(cursor='0\ud800'))]
                    try:
                        first=gaps.build_gap_snapshot(c,'p',limit=1)
                    except Exception:first={}
                    cursor=first.get('next_cursor')
                    if cursor:
                        operations.append(('transfers.gaps.list',dict(limit=1,cursor=cursor)))
                        payload=json.loads(base64.urlsafe_b64decode(cursor[5:]+'='*(-len(cursor[5:])%4)))
                        for changed in ([[payload[0][0],payload[0][1]+1],payload[1],payload[2]],[payload[0],999999,payload[2]],[payload[0],payload[1],'wrong'],[payload[0],payload[1],payload[2]+'\ud800']):
                            token='cgr3.'+base64.urlsafe_b64encode(json.dumps(changed,separators=(',',':')).encode()).decode().rstrip('=')
                            operations.append(('transfers.gaps.list',dict(cursor=token)))
                    ids={case['args'].get('gap_id') for case in scenario['cases']}
                    ids.update(g['gap_id'] for g in first.get('gaps',[]));ids.update(('', 'absent'))
                    operations += [('transfers.gaps.review',dict(gap_id=id)) for id in sorted(ids) if id is not None]
                    if scenario['name']=='lifecycle':
                        operations += [('transfers.gaps.review',dict(gap_id=scenario['cases'][0]['args']['gap_id']+'\ud800'))]
                    before=dump(c);cases=[]
                    for operation,args in operations:
                        try:
                            result=gaps.build_gap_snapshot(c,'p',**(dict(gap_id=args['gap_id'],limit=1) if operation.endswith('review') else args))
                            expected=dict(result=result)
                        except Exception as e:
                            expected=dict(error=dict(code=e.code if isinstance(e,AppError) else 'internal_error',message=str(e),hint=getattr(e,'hint',None),details=getattr(e,'details',None)))
                        cases.append(dict(operation=operation,args=args,profile='p',**expected))
                    assert dump(c)==before,(scenario['name'],i,variant)
                    output.append(dict(name=f"{scenario['name']}-{i}-{variant}",sql=sql+extra,cases=cases))
                    c.execute('ROLLBACK TO variant');c.execute('RELEASE variant')
                c.close()
    base.close()
    path=ROOT/'crates/transfers/tests/gap_reads.json'
    content=json.dumps(dict(scenarios=output),indent=2,ensure_ascii=True)+'\n'
    if options.check:assert path.read_text()==content,'gap_reads.json differs from Python'
    else:path.write_text(content)
    print(dict(books=len(output),operations=sum(len(s['cases']) for s in output),successes=sum('result' in c for s in output for c in s['cases'])))

if __name__=='__main__':main()
