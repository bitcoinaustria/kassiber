import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from kassiber import daemon
from kassiber.core import source_funds_review
from kassiber.db import open_db
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_projection import tables, wire
from crates.reports.scripts.gen_source_funds_reads import BASE
from crates.reports.scripts.gen_source_funds_report import insert, link

ROOT = Path(__file__).resolve().parents[3]
DESTINATION = ROOT/'crates/reports/tests/data/source_funds_review.json'
SCRATCH = ROOT/'target/r5-sof-reads-5/scratch'
BASE_SQL = (ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE


def outcome(conn, command, args):
    try:
        if command.startswith('ui.'):
            data = daemon._ui_source_funds_payload_from_conn(conn, command, daemon._coerce_args_dict(None,args))
        else:
            from kassiber.cli.source_funds_review import dispatch_source_funds_review
            from types import SimpleNamespace
            data = dispatch_source_funds_review(conn,SimpleNamespace(source_funds_command=command.split('.')[-1],workspace=args.get('workspace'),profile=args.get('profile'),target_transaction=args.get('target_transaction'),recipe_file=args.get('recipe_file'),action=args.get('action'),expected_review_fingerprint=args.get('expected_review_fingerprint'),explanation=args.get('explanation')),daemon._source_funds_hooks())
        return dict(ok=data)
    except AppError as e:
        return dict(error=dict(code=e.code,message=str(e),hint=e.hint,details=e.details,retryable=e.retryable))
    except Exception as e:
        return dict(error=dict(code='internal_error',message=str(e),hint=None,details=None,retryable=False))


def cases():
    result=[]
    def add(name,args=None,sql='',command='ui.source_funds.review_context',seed=BASE_SQL):
        result.append(dict(name=name,command=command,args_json=wire({'target_transaction':'in',**(args or {})}),sql=seed+sql))
    for mode in ['minimal','labels_only','standard','full']:
        for target in ['in','out','earn','excluded','external-in','duplicate','missing']:
            add(f'{mode}-{target}',dict(target_transaction=target,reveal_mode=mode))
    for args in [dict(target_transaction=''),dict(target_transaction=' in '),dict(target_transaction=None),dict(target_transaction=42),dict(target_amount=1),dict(planned_destination=False),dict(planned_note='\ud800'),dict(planned_destination='\ud800',report_purpose='planned_exchange_sale'),dict(planned_note=[]),dict(reveal_mode={}),dict(recipient=42),dict(report_purpose=False),dict(max_depth=True),dict(max_depth=0),dict(max_depth=33),dict(max_depth=1.5),dict(max_depth='2'),dict(report_options=None),dict(report_options=[]),dict(unexpected=True),dict(planned_note='Ω'*3000),dict(planned_note='x'*16300),dict(planned_note='x'*16384),dict(report_options=dict(unknown=True)),dict(report_purpose='planned_exchange_sale',planned_destination='  Bank Ω  ',planned_note='  Note  ',target_amount='0.1',max_depth=1,recipient='r-a'),dict(report_options=dict(mask_recipient=True,reveal_overrides={'in':'hide'},omit_sections=['sources']))]:
        add('args-'+repr(args)[:100],args)
    for name,sql in {
        'stale':"UPDATE profiles SET journal_input_version=99 WHERE id='p';",
        'source-blocked':"INSERT INTO freshness_source_states(profile_id,source_type,source_key,source_label,status,blocking_reports,updated_at) VALUES('p','wallet','cold','Cold','error',1,'fixed');",
        'no-context':"DELETE FROM settings WHERE key='context_profile';",
        'provenance-edit':"UPDATE source_funds_sources SET description='changed' WHERE id='s-a';",
        'evidence-edit':"UPDATE attachments SET sha256='changed' WHERE id='a-a';",
        'rejected-link':"UPDATE source_funds_links SET state='rejected' WHERE id='l-a';",
        'attestation':"UPDATE source_funds_sources SET source_type='opening_balance_attestation' WHERE id='s-a';",
        'missing-evidence':"DELETE FROM source_funds_source_attachments;DELETE FROM source_funds_link_attachments;DELETE FROM attachments;",
        'link-truncation':''.join(link(f'extra-{i:03}','s-a','in',1,source=True,state='rejected') for i in range(202)),
        'evidence-truncation':''.join(insert('attachments',dict(id=f'extra-{i:03}',workspace_id='ws',profile_id='p',transaction_id='in',attachment_type='file',label='Local',stored_relpath='never-open',created_at='fixed')) for i in range(202)),
    }.items():
        add(name,sql=sql)
    reports=json.loads((ROOT/'crates/reports/tests/data/source_funds_report.json').read_text())
    for name in ['custody-current','custody-grouped','stale-custody','reconciliation-current','structural-current','stale-structural','privacy-fee-reference-bug']:
        case=next(c for c in reports if c['name']==name)
        add('report-'+name,json.loads(case['args_json']),seed=case['sql'])
        if name in ['custody-current','structural-current']:
            sql="UPDATE source_funds_links SET state='suggested';"
            if name=='custody-current':sql+="UPDATE transactions SET occurred_at='2023-01-01T00:00:00Z' WHERE id='out';"
            add('eligible-'+name,json.loads(case['args_json']),seed=case['sql'],sql=sql)
            if name=='structural-current':
                manual=sql+"INSERT INTO source_funds_links(id,workspace_id,profile_id,from_source_id,from_transaction_id,to_transaction_id,link_type,state,confidence,method,asset,allocation_amount,allocation_policy,explanation,uses_chain_observation,chain_data_confirmed,created_at,updated_at,from_asset,from_allocation_amount) SELECT 'manual-copy',workspace_id,profile_id,from_source_id,from_transaction_id,to_transaction_id,link_type,'suggested',confidence,'manual',asset,allocation_amount,allocation_policy,explanation,uses_chain_observation,chain_data_confirmed,created_at,updated_at,from_asset,from_allocation_amount FROM source_funds_links WHERE method='utxo_spend' LIMIT 1;"
                add('eligible-structural-with-manual',json.loads(case['args_json']),seed=case['sql'],sql=manual)
            if name=='custody-current':
                beyond=sql+"UPDATE source_funds_links SET id='zz-eligible';"+''.join(link(f'extra-{i:03}','s-a','in',1,source=True,state='rejected') for i in range(202))
                add('eligible-beyond-inspection',json.loads(case['args_json']),seed=case['sql'],sql=beyond)
            for variant,extra in {
                'weak':"UPDATE source_funds_links SET confidence='weak';",
                'chain':"UPDATE source_funds_links SET uses_chain_observation=1;",
                'allocation':"UPDATE source_funds_links SET allocation_amount=1;",
                'from-allocation':"UPDATE source_funds_links SET from_allocation_amount=1;",
                'fractional-allocation':"UPDATE source_funds_links SET allocation_amount=allocation_amount+0.5;",
                'fractional-from':"UPDATE source_funds_links SET from_allocation_amount=from_allocation_amount+0.5;",
                'excluded':"UPDATE transactions SET excluded=1 WHERE id='out';",
                'chronology':"UPDATE transactions SET occurred_at='2030-01-01' WHERE direction='outbound';",
                'strong':"UPDATE source_funds_links SET confidence='strong';",
                'provider':"UPDATE source_funds_links SET method='provider_id';",
                'rejected-pair':"INSERT INTO source_funds_links SELECT 'rejected-copy',workspace_id,profile_id,from_source_id,from_transaction_id,to_transaction_id,link_type,'rejected',confidence,method,asset,allocation_amount,allocation_policy,explanation,uses_chain_observation,chain_data_confirmed,created_at,updated_at,from_asset,from_allocation_amount FROM source_funds_links LIMIT 1;",
            }.items():
                add(f'eligible-{name}-{variant}',json.loads(case['args_json']),seed=case['sql'],sql=sql+extra)
    for args in [None,[],False,{},dict(target_transaction='in',recipe={})]:
        result.append(dict(name='raw-'+repr(args),command='ui.source_funds.review_context',args_json=wire(args),sql=BASE_SQL))
    with tempfile.TemporaryDirectory(dir=SCRATCH) as temporary:
        conn=open_db(Path(temporary)/'data');conn.executescript(BASE_SQL);conn.commit()
        inspection=outcome(conn,'ui.source_funds.review_context',{'target_transaction':'in'})['ok']
        request=dict(target_transaction='in',action='attach_evidence',recipe=inspection['recipe'],expected_review_fingerprint=inspection['review_fingerprint'])
        for action in ['attach_evidence','connect_wallet','import_history','invalid',None,42]:
            add('request-'+str(action),dict(request,action=action),command='ui.source_funds.request_input')
        for explanation in [None,'  Evidence Ω\n note\t  ','', '   ','x'*1000,'x'*1001,'bad\x00',False,[], '\U0001f600'*1000]:
            add('explanation-'+repr(explanation)[:35],dict(request,explanation=explanation),command='ui.source_funds.request_input')
        for fingerprint in [None,'','A'*64,'0'*63,'0'*65,'0'*64,42]:
            add('fingerprint-'+str(fingerprint),dict(request,expected_review_fingerprint=fingerprint),command='ui.source_funds.request_input')
        for recipe in [None,[],{'unknown':True},{'max_depth':True},{'report_options':{}},inspection['recipe']]:
            add('request-recipe-'+repr(recipe)[:35],dict(request,recipe=recipe),command='ui.source_funds.request_input')
        for field in ['action','target_transaction','expected_review_fingerprint']:
            args={k:v for k,v in request.items() if k!=field}
            result.append(dict(name='request-missing-'+field,command='ui.source_funds.request_input',args_json=wire(args),sql=BASE_SQL))
        add('request-unknown',dict(request,unknown=True),command='ui.source_funds.request_input')
        add('request-stale-evidence',request,sql="UPDATE attachments SET sha256='changed' WHERE id='a-a';",command='ui.source_funds.request_input')
        for name,extra in {
            'no-missing-input':"DELETE FROM source_funds_links WHERE id='l-z';UPDATE source_funds_links SET from_allocation_amount=allocation_amount WHERE id='l-a';",
            'truncated-links':''.join(link(f'extra-{i:03}','s-a','in',1,source=True,state='rejected') for i in range(202)),
            'truncated-evidence':''.join(insert('attachments',dict(id=f'extra-{i:03}',workspace_id='ws',profile_id='p',transaction_id='in',attachment_type='file',label='Local',stored_relpath='never-open',created_at='fixed')) for i in range(202)),
        }.items():
            conn.executescript('SAVEPOINT request_case;'+extra)
            reviewed=outcome(conn,'ui.source_funds.review_context',{'target_transaction':'in'})['ok']
            add('request-'+name,dict(target_transaction='in',action='attach_evidence',recipe=reviewed['recipe'],expected_review_fingerprint=reviewed['review_fingerprint']),sql=extra,command='ui.source_funds.request_input')
            conn.execute('ROLLBACK TO request_case');conn.execute('RELEASE request_case')
        conn.close()
    return result


def evaluate(fixtures):
    with tempfile.TemporaryDirectory(dir=SCRATCH) as temporary:
        conn=open_db(Path(temporary)/'data');result=[]
        for case in fixtures:
            conn.executescript('SAVEPOINT review_case;'+case['sql'])
            before=tables(conn)
            expected=outcome(conn,case['command'],json.loads(case['args_json']))
            assert tables(conn)==before,case['name']
            if case['name'] in ['eligible-custody-current','eligible-structural-current']:
                assert expected['ok']['bulk_review']['eligible_link_ids'],case['name']
            if case['name']=='eligible-structural-with-manual':
                eligible=expected['ok']['bulk_review']['eligible_link_ids']
                assert eligible and 'manual-copy' not in eligible,case['name']
            result.append(wire(expected))
            conn.execute('ROLLBACK TO review_case');conn.execute('RELEASE review_case')
        conn.close()
    return result


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--write',action='store_true');parser.add_argument('--child',action='store_true');args=parser.parse_args()
    SCRATCH.mkdir(parents=True,exist_ok=True)
    fixtures=cases()
    if args.child:
        print(wire(evaluate(fixtures)));return
    variants=[]
    for seed in ['0','1','2']:
        child=subprocess.run([sys.executable,'-m','crates.reports.scripts.gen_source_funds_review','--child'],cwd=ROOT,env={**os.environ,'PYTHONHASHSEED':seed},capture_output=True,text=True,timeout=180)
        assert child.returncode==0,child.stderr
        assert not child.stderr,child.stderr
        variants.append(json.loads(child.stdout))
    for i,case in enumerate(fixtures):case['expected_variants']=list(dict.fromkeys(v[i] for v in variants))
    generated=json.dumps(fixtures,ensure_ascii=True,indent=2)+'\n'
    if args.write:DESTINATION.write_text(generated)
    elif DESTINATION.read_text()!=generated:raise SystemExit('source-funds review vectors differ')
    print(f'{len(fixtures)} review/input cases verified across seeds 0, 1 and 2; all 114 tables unchanged')


if __name__=='__main__':main()
