"""Freeze report saves and reviewed inspection admission from unchanged Python."""
import argparse
import json
import sqlite3
import tempfile
from pathlib import Path
from unittest.mock import patch

from kassiber import daemon
from kassiber.core import source_funds as sof, source_funds_review as review
from kassiber.db import open_db
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_cases import ROOT, STAMP, dump
from crates.reports.scripts.gen_source_funds_reads import BASE

DESTINATION=ROOT/'crates/reports/tests/data/source_funds_saves.json'
BASE_SQL=(ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE
READY="DELETE FROM source_funds_links WHERE id='l-z';UPDATE source_funds_links SET from_allocation_amount=allocation_amount WHERE id='l-a';"


def outcome(conn,case):
    calls=[]
    ids=iter(case.get('ids',['new-case','new-snapshot']))
    def identity():
        calls.append('uuid');return next(ids)
    def now():
        calls.append('now');return STAMP
    with patch.object(sof.uuid,'uuid4',identity),patch.object(sof,'_now',now):
        try:
            command,args=case['command'],case['args']
            if command=='helper.review_context':
                _,profile=daemon.resolve_scope(conn,None,None)
                data=review.review_context(conn,profile,daemon._source_funds_hooks(),target_transaction=args.get('target_transaction'),recipe=args.get('recipe'))
            elif command.startswith('ui.'):
                data=daemon._ui_source_funds_payload_from_conn(conn,command,daemon._coerce_args_dict(None,args))
            else:
                data=sof.build_report(conn,args.get('workspace'),args.get('profile'),daemon._source_funds_hooks(),
                    target_transaction_ref=args.get('target_transaction'),target_amount=args.get('target_amount'),
                    report_purpose=args.get('purpose') or 'existing_transaction',planned_destination=args.get('planned_destination'),planned_note=args.get('planned_note'),
                    reveal_mode=args.get('reveal_mode'),max_depth=args.get('max_depth',8),recipient_ref=args.get('recipient'),
                    include_diagrams=True,save_case=True,case_label=args.get('case_label'),
                    report_options=dict(diagram_detail=args.get('diagram_detail'),amount_precision=args.get('amount_precision'),mask_recipient=args.get('mask_recipient'),omit_sections=args.get('omit_section'),reveal_overrides=dict(item.split('=',1) for item in args.get('reveal_override',[]) if '=' in item)))
                if args.get('format') in ('table','plain'):
                    data='\n'.join(sof.build_report_lines(data,daemon._source_funds_hooks()))
            result=dict(ok=data);envelope=build_envelope(command,data)
        except Exception as error:
            conn.rollback()
            fields=dict(code=error.code if isinstance(error,AppError) else 'internal_error',message=str(error),hint=error.hint if isinstance(error,AppError) else None,details=error.details if isinstance(error,AppError) else None,retryable=error.retryable if isinstance(error,AppError) else False)
            result=dict(error=fields);envelope=build_error_envelope(**fields)
    result.update(calls=calls,cli=json.dumps(envelope,indent=2)+'\n',daemon=json.dumps(envelope,separators=(',',':'))+'\n')
    return result


def cases():
    result=[]
    def add(name,args=None,sql='',command='ui.source_funds.cases.save',**extra):
        result.append(dict(name=name,command=command,args={'target_transaction':'in',**(args or {})},sql=sql,ids=['new-case','new-snapshot'],**extra))
    for mode in ['minimal','standard','full']:
        for recipient in [None,'r-a']:
            add(f'save-{mode}-{recipient}',dict(reveal_mode=mode,recipient=recipient,case_label='Case Ω'),READY)
    for name,sql in [('blocked',''),('stale',"UPDATE profiles SET journal_input_version=99 WHERE id='p';"),('ready',READY),('unknown',READY+"UPDATE source_funds_sources SET source_type='unknown' WHERE id='s-a';"),('frozen-external',READY+"UPDATE transactions SET external_id='secret external' WHERE id='in';"),('null-external',READY+"UPDATE transactions SET external_id=NULL WHERE id='in';")]:
        add(name,sql=sql)
    add('cli-save',dict(save_case=True,case_label='CLI case'),READY,command='reports.source-funds')
    add('cli-plain',dict(save_case=True,format='plain'),READY,command='reports.source-funds')
    add('cli-planned',dict(save_case=True,purpose='planned_exchange_sale',planned_destination='  Exchange Ω ',planned_note=' note ',recipient='r-a',target_amount='0.1',diagram_detail='detailed',amount_precision='sats',mask_recipient=True,omit_section=['flow_links'],reveal_override=['in=hide']),READY,command='reports.source-funds')
    add('desktop-planned',dict(report_purpose='planned_exchange_sale',planned_destination='Exchange',planned_note='Note',recipient='r-a',target_amount='0.1',report_options=dict(diagram_detail='detailed',amount_precision='sats',mask_recipient=True,omit_sections=['flow_links'],reveal_overrides={'in':'hide'})),READY)
    for args in [dict(target_transaction='missing'),dict(target_transaction='duplicate'),dict(target_transaction=''),dict(target_transaction=None),dict(target_transaction=1),dict(case_label=1),dict(case_label=False),dict(target_amount='bad'),dict(target_amount='0'),dict(recipient='missing'),dict(recipient='r-i'),dict(reveal_mode='bad'),dict(report_purpose='bad'),dict(max_depth='2'),dict(report_options=[]),dict(target_transaction='  in  ',case_label='')]:
        add('argument-'+str(args),args,READY)
    for label,sql in [('case-collision',"INSERT INTO source_funds_cases(id,workspace_id,profile_id,target_transaction_id,target_amount,asset,label,reveal_mode,status,snapshot_hash,snapshot_json,created_at,updated_at) VALUES('new-case','ws','p','in',1,'BTC','old','minimal','blocked','old','{}','fixed','fixed');"),('snapshot-collision',"INSERT INTO source_funds_snapshots(id,case_id,snapshot_hash,snapshot_json,created_at) VALUES('new-snapshot','c-a','old','{}','fixed');")]:
        add(label,sql=READY+sql)
    for table in ['source_funds_cases','source_funds_snapshots']:
        for action in ['ABORT','ROLLBACK']:
            add(table+'-'+action,sql=READY+f"CREATE TRIGGER refuse_save BEFORE INSERT ON {table} BEGIN SELECT RAISE({action},'synthetic late refusal'); END;")
    for recipe in [None,{},dict(target_amount='0.1'),dict(recipient='r-a'),dict(reveal_mode='full',report_options=dict(mask_recipient=True)),dict(report_purpose='planned_exchange_sale',planned_destination='Exchange',planned_note='Note'),dict(max_depth=1),dict(unknown='x'),[],True,dict(target_amount=1),dict(max_depth=True),dict(max_depth=0),dict(max_depth=33),dict(report_options=[]),dict(planned_note='x'*17000)]:
        add('inspection-'+str(recipe)[:60],dict(recipe=recipe),READY,command='helper.review_context')
    for fp in [None,'','0'*63,'A'*64,True,123,'z'*64]:
        add('fingerprint-'+str(fp),dict(expected_review_fingerprint=fp),READY)
    for change,sql in [('current',''),('label',"UPDATE source_funds_sources SET label='Changed' WHERE id='s-a';"),('evidence',"UPDATE attachments SET sha256='changed' WHERE id='a-a';"),('input-version',"UPDATE profiles SET journal_input_version=99 WHERE id='p';"),('link-timestamp',"UPDATE source_funds_links SET updated_at='changed' WHERE id='l-a';"),('target',"UPDATE transactions SET description='changed' WHERE id='in';"),('unrelated',"UPDATE transactions SET description='unrelated' WHERE id='earn';")]:
        add('reviewed-'+change,dict(case_label='Reviewed Ω'),READY+sql,fingerprint_sql=READY)
    add('reviewed-options',dict(case_label='Reviewed options',reveal_mode='minimal',recipient='r-a',target_amount='0.1',report_options=dict(mask_recipient=True)),READY,fingerprint_sql=READY)
    add('late-deferred',sql=READY+"PRAGMA defer_foreign_keys=ON;CREATE TRIGGER defer_save AFTER INSERT ON source_funds_cases BEGIN INSERT INTO source_funds_snapshots(id,case_id,snapshot_hash,snapshot_json,created_at) VALUES('late','missing','hash','{}','fixed');END;")
    from crates.reports.scripts.gen_source_funds_report import link
    truncated=READY+''.join(link('overflow-'+str(i),'s-a','in',1,source=True,state='rejected') for i in range(201))
    add('reviewed-truncated',sql=truncated,fingerprint_sql=truncated)
    add('inspection-truncated',sql=truncated,command='helper.review_context')
    for recipe in [{},dict(target_amount='0.1'),dict(recipient='r-a'),dict(reveal_mode='full',report_options=dict(mask_recipient=True)),dict(report_purpose='planned_exchange_sale',planned_destination='Exchange'),dict(unknown='x'),dict(target_transaction='excluded'),dict(target_transaction=None),dict(target_transaction=1),dict(max_depth=True),dict(max_depth=33)]:
        add('context-daemon-'+str(recipe),recipe,READY,command='ui.source_funds.review_context')
    return result


def generate():
    scratch=ROOT/'target/r5-sof-mutations-4/generator-data';scratch.mkdir(parents=True,exist_ok=True)
    result=cases()
    with tempfile.TemporaryDirectory(dir=scratch) as directory:
        template=open_db(Path(directory)/'data');template.executescript(BASE_SQL);template.commit()
        for case in result:
            conn=sqlite3.connect(':memory:');conn.row_factory=sqlite3.Row;template.backup(conn);conn.execute('PRAGMA foreign_keys=ON')
            if 'fingerprint_sql' in case:
                conn.executescript(case['fingerprint_sql']);conn.commit()
                _,profile=daemon.resolve_scope(conn,None,None)
                recipe={k:v for k,v in case['args'].items() if k in review.RECIPE_KEYS}
                context=review.review_context(conn,profile,daemon._source_funds_hooks(),target_transaction=case['args']['target_transaction'],recipe=recipe)
                case['args']['expected_review_fingerprint']=context['review_fingerprint']
                conn.close();conn=sqlite3.connect(':memory:');conn.row_factory=sqlite3.Row;template.backup(conn);conn.execute('PRAGMA foreign_keys=ON')
            conn.executescript(case['sql']);conn.commit();before=dump(conn)
            case['expected']=outcome(conn,case);after=dump(conn)
            written=['source_funds_cases','source_funds_snapshots']
            assert {k:v for k,v in before.items() if k not in written}=={k:v for k,v in after.items() if k not in written},case['name']
            if 'error' in case['expected'] or case['command']=='helper.review_context':assert before==after,case['name']
            case['tables']={k:after[k] for k in written};conn.close()
        template.close()
    return json.dumps(dict(base_sql=BASE_SQL,cases=result),ensure_ascii=True,indent=2)+'\n'


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--write',action='store_true');args=parser.parse_args();generated=generate()
    if args.write:DESTINATION.write_text(generated)
    elif DESTINATION.read_text()!=generated:raise SystemExit('source-funds save vectors differ')
    print(f'{len(json.loads(generated)["cases"])} source-funds save cases verified')

if __name__=='__main__':main()
