import argparse
from itertools import count
import json
from pathlib import Path
import tempfile
from unittest.mock import patch
from uuid import UUID

from kassiber import daemon
from kassiber.core import source_funds as sof
from kassiber.db import open_db
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_cases import dump, STAMP
from crates.reports.scripts.gen_source_funds_suggest import cases as previews, structural, BASE_SQL

ROOT=Path(__file__).resolve().parents[3]
DESTINATION=ROOT/'crates/reports/tests/data/source_funds_passes.json'
SCRATCH=ROOT/'target/r5-sof-mutations-5/generator'


def outcome(conn,case):
    calls=[];ids=count(1)
    def now():
        calls.append('now')
        if case.get('fail_now')==calls.count('now'):raise AppError('late source clock refusal',code='validation')
        return STAMP
    def uuid():
        calls.append('uuid');return UUID(f'00000000-0000-4000-8000-{next(ids):012d}')
    with patch.object(sof,'_now',now),patch.object(sof.uuid,'uuid4',uuid):
        command,args=case['command'],case['args']
        try:
            if command.startswith('ui.'):
                data=daemon._ui_source_funds_payload_from_conn(conn,command,daemon._coerce_args_dict(None,args))
            elif command.endswith('assemble'):
                data=sof.assemble_history(conn,args.get('workspace'),args.get('profile'),daemon._source_funds_hooks(),target_transaction_ref=args['target_transaction'],include_broad_hints=args.get('include_broad_hints',False),max_passes=args.get('max_passes',8))
            else:
                data=sof.suggest_links(conn,args.get('workspace'),args.get('profile'),daemon._source_funds_hooks(),target_transaction_ref=args.get('target_transaction'),include_broad_hints=args.get('include_broad_hints',False),max_suggestions=args.get('max_suggestions',500))
            result=dict(ok=data);envelope=build_envelope(command,data)
        except Exception as error:
            conn.rollback()
            fields=dict(code=error.code,message=str(error),hint=error.hint,details=error.details,retryable=error.retryable) if isinstance(error,AppError) else dict(code='internal_error',message=str(error),hint=None,details=None,retryable=False)
            result=dict(error=fields);envelope=build_error_envelope(**fields)
    result.update(calls=calls,cli=json.dumps(envelope,indent=2)+'\n',daemon=json.dumps(envelope,separators=(',',':'))+'\n')
    return result


def cases():
    result=[]
    seeds=previews()
    for seed in seeds:
        args=json.loads(seed['args_json']);args.pop('dry_run')
        result.append(dict(name='suggest-'+seed['name'],command='source-funds.suggest',sql=seed['sql'],args=args,sequential_ids=True))
    selected=['provider-default','structural-current','structural-no-authority','structural-privacy','projection-custody-current','lineage-chain-3','lineage-ChangePassthroughTestsFalse','lineage-wallet-memo-isolation','projection-custody-stale-version','target-rejected-frontier','cap-rollback']
    for seed in seeds:
        if seed['name'] not in selected:continue
        args=json.loads(seed['args_json']);args.pop('dry_run');args.pop('max_suggestions',None);args.setdefault('target_transaction','suggest-in')
        for max_passes in [1,2,8,99,0,-1]:
            result.append(dict(name='assemble-'+seed['name']+'-'+str(max_passes),command='source-funds.assemble',sql=seed['sql'],args={**args,'max_passes':max_passes},sequential_ids=True))
    for seed in [s for s in seeds if s['name'] in ['provider-default','structural-current']]:
        for command in ['ui.source_funds.suggest','ui.source_funds.assemble']:
            for args in [None,[],False,{},dict(target_transaction=42),dict(target_transaction=' '),dict(target_transaction='missing'),dict(target_transaction='suggest-in',include_broad_hints=None),dict(target_transaction='suggest-in',include_broad_hints='yes'),dict(target_transaction='suggest-in',max_suggestions=0),dict(target_transaction='suggest-in',max_passes=0),dict(target_transaction='suggest-in',max_passes='2'),dict(target_transaction='suggest-in',max_passes=2.9)]:
                result.append(dict(name=command+'-'+seed['name']+'-'+str(len(result)),command=command,sql=seed['sql'],args=args,sequential_ids=True))
    for seed in [s for s in seeds if s['name']=='structural-current']:
        args=json.loads(seed['args_json']);args.pop('dry_run')
        for fail_now in [2,3]:
            result.append(dict(name='assemble-late-'+str(fail_now),command='source-funds.assemble',sql=seed['sql'],args=args,fail_now=fail_now,sequential_ids=True))
    from crates.reports.scripts.gen_source_funds_report import transaction
    seed=next(s for s in seeds if s['name']=='provider-default')
    for name,extra in [
        ('suggest-late-statement',transaction('extra',100000,wallet='hot',raw_json='{"trade_id":"proof"}')+"CREATE TRIGGER refuse_late BEFORE INSERT ON source_funds_links WHEN NEW.to_transaction_id='suggest-in' BEGIN SELECT RAISE(ABORT,'late suggest refusal'); END;"),
        ('suggest-deferred',"CREATE TRIGGER refuse_commit AFTER INSERT ON source_funds_links BEGIN INSERT INTO source_funds_link_attachments(link_id,attachment_id,created_at) VALUES(NEW.id,'missing-attachment','fixed'); END; PRAGMA defer_foreign_keys=ON;"),
    ]:
        result.append(dict(name=name,command='source-funds.suggest',sql=seed['sql']+extra,args={},sequential_ids=True))
    seed=next(s for s in seeds if s['name']=='structural-current')
    args=json.loads(seed['args_json']);args.pop('dry_run')
    result.append(dict(name='assemble-review-abort',command='source-funds.assemble',sql=seed['sql']+"CREATE TRIGGER refuse_review BEFORE UPDATE ON source_funds_links WHEN NEW.state='reviewed' BEGIN SELECT RAISE(ABORT,'late review refusal'); END;",args=args,sequential_ids=True))
    for seed in [s for s in seeds if s['name']=='projection-custody-current']:
        args=json.loads(seed['args_json']);args.pop('dry_run')
        changes=["UPDATE profiles SET last_processed_at=NULL WHERE id='p';", "UPDATE transactions SET raw_json='{\"trade_id\":\"new-proof\"}' WHERE id IN ('out','in');"]
        for index,change in enumerate(changes):
            result.append(dict(name='assemble-frozen-'+str(index),command='source-funds.assemble',sql=seed['sql']+"CREATE TRIGGER mutate_after_suggestion AFTER INSERT ON source_funds_links BEGIN "+change+" END;",args=args,sequential_ids=True))
    seed=next(s for s in seeds if s['name']=='projection-custody-current')
    args=json.loads(seed['args_json']);args.pop('dry_run')
    result.append(dict(name='assemble-frozen-review',command='source-funds.assemble',sql=seed['sql']+"UPDATE transactions SET occurred_at='2023-01-01T00:00:00Z' WHERE id='out';CREATE TRIGGER stale_after_suggestion AFTER INSERT ON source_funds_links BEGIN UPDATE profiles SET last_processed_at=NULL WHERE id='p'; END;",args=args,sequential_ids=True))
    from tests.test_source_funds_assembly import _authoritative
    lineage=json.loads((ROOT/'crates/reports/tests/data/source_funds_lineage.json').read_text())
    seed=next(c for c in lineage if c['name']=='ChangePassthroughTestsFalse')
    rows=[]
    for row in seed['rows']:
        row=dict(row)
        if row['direction']=='outbound':
            raw=json.loads(row['raw_json']);raw.pop('observer_owned_scripts',None);row['raw_json']=json.dumps(raw,sort_keys=True);row=_authoritative(row)
        rows.append(row)
    target=next((r['id'] for r in rows if r['id']=='spend'),rows[-1]['id'])
    result.append(dict(name='assemble-attestation',command='source-funds.assemble',sql=BASE_SQL+structural(rows,seed['owned']),args=dict(target_transaction=target),sequential_ids=True))
    return result


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--write',action='store_true');args=parser.parse_args()
    SCRATCH.mkdir(parents=True,exist_ok=True)
    fixtures=cases()
    with tempfile.TemporaryDirectory(dir=SCRATCH) as temporary:
        conn=open_db(Path(temporary)/'data')
        empty=conn.iterdump();schema='\n'.join(empty);conn.close()
        import sqlite3
        for case in fixtures:
            conn=sqlite3.connect(':memory:');conn.row_factory=sqlite3.Row;conn.executescript(schema);conn.execute('PRAGMA foreign_keys=ON');conn.executescript(case['sql']);before=dump(conn)
            case['expected']=outcome(conn,case)
            if case['name']=='assemble-attestation':assert case['expected']['ok']['chain_attestation_missing']
            if case['name']=='assemble-frozen-review':assert case['expected']['ok']['auto_reviewed']>0
            after=dump(conn);case['tables']={key:rows for key,rows in after.items() if rows!=before[key]};conn.close()
    rendered=json.dumps(fixtures,ensure_ascii=True,separators=(',',':'))+'\n'
    if args.write:DESTINATION.write_text(rendered)
    else:assert DESTINATION.read_text()==rendered
    print(len(fixtures),'frozen source-funds passes')


if __name__=='__main__':main()
