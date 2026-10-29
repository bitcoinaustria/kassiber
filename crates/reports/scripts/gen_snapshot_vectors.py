#!/usr/bin/env python3
"""Freeze phase-4 snapshot writes against Python on disposable books."""
from pathlib import Path
import json
import tempfile
from unittest.mock import patch

from kassiber.core import custody_filed_reports as reports
from kassiber.db import open_db
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'crates/reports/tests/data/snapshot_writes.json'
STAMP = '2026-01-02T03:04:05Z'
TABLES = ('filed_report_snapshots', 'filed_report_chain_captures',
          'filed_report_chain_dependencies', 'custody_filed_report_impacts',
          'custody_filed_report_impact_resolutions', 'filed_report_chain_impacts',
          'filed_report_chain_resolutions')
BASE = dict(report_kind='full-report.csv', report_state='saved',
            period_start_year=2024, content_sha256='a' * 64)
SAVED = """
INSERT INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,
period_start_year,period_end_year,content_sha256,classification_summary_json,gain_summary_json,
report_scope_json,authored_source,notes,created_at)
VALUES('saved','ws','p','full-report.csv','saved',2024,2025,
'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
'{"income":{"amount_msat":1001,"count":1}}','{"gain_loss_exact":"1.2300","status":"final"}',
'{"wallet_ids":["cold"]}','user','sealed','2025-01-01T00:00:00Z');
INSERT INTO filed_report_chain_captures(snapshot_id,profile_id,captured_at) VALUES('saved','p','original');
INSERT INTO filed_report_chain_dependencies(snapshot_id,profile_id,transaction_id,wallet_id,chain,network,txid,observation_json)
VALUES('saved','p','in','cold','bitcoin','main','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
'{"confirmed":true,"domain_id":"sealed","quantity_hash":"frozen","status":"observed"}');
INSERT INTO custody_filed_report_impacts(id,workspace_id,profile_id,filed_report_snapshot_id,
component_id,review_id,gap_id,affected_period_start_year,affected_period_end_year,
after_gain_summary_json,amendment_warning,created_at)
VALUES('impact','ws','p','saved','component','review','gap',2024,2025,
'{"status":"pending_journal_rebuild"}','review required','impact-time');
INSERT INTO custody_filed_report_impact_resolutions(id,workspace_id,profile_id,impact_id,rebuilt_at,
after_gain_summary_json,classification_changed,gain_changed,amendment_status,created_at)
VALUES('resolution','ws','p','impact','rebuilt','{"gain_loss_exact":"2.3400"}',0,1,'saved_report_changed','resolution-time');
INSERT INTO filed_report_chain_impacts(id,snapshot_id,profile_id,transaction_id,inbox_id,created_at)
VALUES('chain-impact','saved','p','in','inbox','chain-impact-time');
INSERT INTO filed_report_chain_resolutions(impact_id,rebuilt_at,summary_json,created_at)
VALUES('chain-impact','rebuilt','{"amendment_status":"review_required"}','chain-resolution-time');
"""


def build():
    specs = []
    def add(name, override=None, *, sql='', args=None, workspace='ws', profile='p', cli=False):
        specs.append(dict(name=name, sql=sql, args=(BASE | (override or {})) if args is None else args,
                          workspace=workspace, profile=profile, cli=cli))
    add('basic')
    add('normalized', dict(report_kind='  FULL.Report_1-CSV ', report_state=' FILED ',
                          content_sha256=' A' + 'A'*63 + ' ', notes=' \u2003Reviewed € 🪙 \u001c '))
    add('whitespace-note', dict(notes=' \u2003 '))
    add('empty-note', dict(notes=''))
    add('exact-summaries', dict(classification_summary={'Income': {'count': 2, 'amount_msat': 1001},
                          ' fee ': {}, 'income': {'count': 4}}, gain_summary=dict(
                          status=' FINAL ', fiat_currency=' eur ', proceeds_exact=' 1.2300000 ',
                          cost_basis_exact='-0.000', gain_loss_exact='1E+40'), report_scope=dict(
                          wallet_ids=[' hot ', 'cold', 'cold'], occurred_at_start='2024-01-01T00:00:00Z',
                          occurred_at_end='2025-01-01T00:00:00Z')))
    add('scope-empty',dict(report_scope={'wallet_ids':[], 'occurred_at_start':None,'occurred_at_end':''}))
    add('scope-shape-only',dict(report_scope={'occurred_at_start':'2024-99-99T99:99:99Z'}))
    add('scope-unicode-digits',dict(report_scope={'occurred_at_start':'２０２４-０１-０１T００:００:００Z'}))
    add('scope-surrogate-wallet',dict(report_scope={'wallet_ids':[' \ud800 ','z']}))
    add('classification-surrogate',dict(classification_summary={'income\ud800':{}}))
    add('scope-surrogate-time',dict(report_scope={'occurred_at_start':'2024-01-01T00:00:00Z\ud800'}))
    add('classification-large-msat',dict(classification_summary={'income':{'count':10**40,'amount_msat':10**100}}))
    add('duplicate-categories-normalize',dict(classification_summary={'Income':{'count':1},'income':{'count':2}}))
    for field,values in {
        'report_kind':[None,'','A'*65,'.a','a b','é','a' * 64],
        'report_state':[None,'','pending','FILED'],
        'period_start_year':[None,True,False,1899,10000,'bad','２０２４','2_024',2024.9,10**60],
        'period_end_year':[True,1899,2023,10000,2024,9999],
        'content_sha256':[None,'','g'*64,'a'*63,'a'*65],
        'authored_source':['user','cli','gui','ai_tool',' USER ','daemon',None],
        'notes':['x'*1000,'🪙'*1000,'x'*1001],
    }.items():
        for i,value in enumerate(values):add(f'{field}-{i}',{field:value})
    for label,value in [('wrong-type',[]),('too-many',{f'x{i}':{} for i in range(33)}),
                        ('bad-category',{'bad category':{}}),('not-mapping',{'income':0}),
                        ('unknown',{'income':{'extra':0}}),('bool-count',{'income':{'count':True}}),
                        ('bool-amount',{'income':{'amount_msat':False}}),
                        ('negative-count',{'income':{'count':-1}}),('negative-amount',{'income':{'amount_msat':-1}}),
                        ('float',{'income':{'count':1.0}}),('text',{'income':{'amount_msat':'1'}})]:
        add('classification-'+label,dict(classification_summary=value))
    for label,value in [('wrong-type',[]),('unknown',{'extra':0}),('currency',{'fiat_currency':'EU'}),
                        ('currency-digit',{'fiat_currency':'EUR1'}),('currency-long',{'fiat_currency':'A'*13}),
                        ('currency-max',{'fiat_currency':'A'*12}),('status',{'status':'draft'}),
                        ('pending',{'status':'pending_journal_rebuild'})]:
        add('gains-'+label,dict(gain_summary=value))
    for field in ('proceeds_exact','cost_basis_exact','gain_loss_exact'):
        for i,value in enumerate(('bad','NaN','-sNaN42','Infinity','-Inf',0,None,'0','1_000.01','１２３.４５','Inf42','--Inf','_NaN_','NaN１２','+NaN0','1\ud800')):
            add(f'gain-{field}-{i}',dict(gain_summary={field:value}))
    for label,value in [('wrong-type',[]),('unknown',{'extra':0}),('wallet-type',{'wallet_ids':'cold'}),
                        ('wallet-null',{'wallet_ids':None}),('wallet-many',{'wallet_ids':['a']*257}),
                        ('wallet-empty',{'wallet_ids':['']}),('wallet-null-item',{'wallet_ids':[None]}),
                        ('wallet-long',{'wallet_ids':['a'*129]}),('wallet-max',{'wallet_ids':['🪙'*128]}),
                        ('wallet-coerce',{'wallet_ids':[1,True,'hot']}),
                        ('timestamp',{'occurred_at_start':'2024-01-01'}),
                        ('fraction',{'occurred_at_end':'2024-01-01T00:00:00.1Z'}),
                        ('order',{'occurred_at_start':'2025-01-01T00:00:00Z','occurred_at_end':'2024-01-01T00:00:00Z'})]:
        add('scope-'+label,dict(report_scope=value))
    add('wrong-book',workspace='missing')
    add('missing-profile',profile='missing')
    add('stale-allowed',sql='UPDATE profiles SET last_processed_at=NULL;')
    filed = dict(report_state='filed',saved_snapshot_id='saved')
    add('sealed-file',args=filed,sql=SAVED)
    add('sealed-file-stale-current-inputs',args=filed,sql=SAVED + "UPDATE profiles SET last_processed_at=NULL; DELETE FROM journal_entries;")
    add('sealed-empty-dependencies',args=filed,sql=SAVED + 'DELETE FROM filed_report_chain_dependencies;')
    add('sealed-missing-capture',args=filed,sql=SAVED + 'DELETE FROM filed_report_chain_captures;')
    add('sealed-not-saved',args=filed,sql=SAVED.replace("'full-report.csv','saved'","'full-report.csv','filed'"))
    add('sealed-other-profile',args=filed,sql=SAVED,profile='other')
    add('sealed-other-workspace',args=filed,sql=SAVED,workspace='missing')
    add('sealed-only-source-profile-dependencies',args=filed,sql=SAVED + "INSERT INTO filed_report_chain_dependencies SELECT snapshot_id,'other','foreign',wallet_id,chain,network,txid,observation_json FROM filed_report_chain_dependencies;")
    for field,value in [('report_kind','other'),('period_start_year',2025),('period_end_year',2026),
                        ('content_sha256','c'*64),('classification_summary',{}),('gain_summary',{}),('report_scope',{})]:
        add('sealed-mismatch-'+field,args=filed | {field:value},sql=SAVED)
    add('sealed-equal-reordered',args=filed | dict(classification_summary={'income':{'count':1,'amount_msat':1001}},
        gain_summary={'status':'final','gain_loss_exact':'1.2300'},report_scope={'wallet_ids':['cold','cold']}),sql=SAVED)
    for i,value in enumerate(('',False,1,' saved ', 'missing')):
        add('saved-reference-'+str(i),args=filed | {'saved_snapshot_id':value},sql=SAVED)
    add('saved-reference-for-save',args=dict(report_state='saved',saved_snapshot_id='saved'),sql=SAVED)
    add('capture-copy-rollback',args=filed,sql=SAVED + "CREATE TRIGGER reject_capture BEFORE INSERT ON filed_report_chain_captures BEGIN SELECT RAISE(ABORT,'capture_rejected'); END;")
    add('dependency-copy-rollback',args=filed,sql=SAVED + "CREATE TRIGGER reject_dependency BEFORE INSERT ON filed_report_chain_dependencies BEGIN SELECT RAISE(ABORT,'dependency_rejected'); END;")
    add('existing-id-append-only',sql=SAVED.split("INSERT INTO filed_report_chain_captures")[0].replace("'saved','ws','p'","'new','ws','p'"))
    for field in ('classification_summary_json','gain_summary_json','report_scope_json'):
        for i,value in enumerate(('bad','[]','true','1','null','{}')):
            add(f'cli-{field}-{i}',args={**BASE,'state':'saved',field:value},cli=True)
    add("cli-json-integer-limit",args={**BASE,"state":"saved","classification_summary_json":"[]","gain_summary_json":"1"*4301},cli=True)
    result=[]
    for spec in specs:
        with tempfile.TemporaryDirectory(prefix='kassiber-snapshot-oracle-') as root:
            conn=open_db(root)
            conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + spec['sql'])
            args=spec['args'].copy()
            try:
                if spec['cli']:
                    for field in ('classification_summary','gain_summary','report_scope'):
                        try: args[field]=json.loads(args.pop(field+'_json')) if field+'_json' in args else None
                        except json.JSONDecodeError as exc:raise AppError('filed snapshot summaries must be JSON objects',code='validation') from exc
                    if any(args[field] is not None and not isinstance(args[field],dict) for field in ('classification_summary','gain_summary','report_scope')):
                        raise AppError('filed snapshot summaries must be JSON objects',code='validation')
                    args['report_state']=args.pop('state')
                    args['authored_source']='cli'
                with patch('kassiber.core.filed_report_chain.now_iso',return_value=STAMP):
                    payload=reports.create_filed_report_snapshot(conn,workspace_id=spec['workspace'],profile_id=spec['profile'],
                        snapshot_id='new',created_at=STAMP,**args)
                expected={'ok':payload}
            except AppError as exc:
                expected={'error':dict(code=exc.code,message=str(exc),hint=exc.hint,details=exc.details,retryable=exc.retryable)}
            except Exception as exc:
                expected={'error':dict(code='internal_error',message=str(exc),hint=None,details=None,retryable=False)}
            spec['expected']=expected
            spec['tables']={table:[dict(row) for row in conn.execute(f'SELECT * FROM {table} ORDER BY 1,2,3')] for table in TABLES}
            conn.close()
            result.append(spec)
    return json.dumps(result,ensure_ascii=True,indent=2)+'\n'

if __name__=='__main__':
    import sys
    content=build()
    if '--check' in sys.argv:assert OUT.read_text()==content,'snapshot vectors drifted'
    else:OUT.write_text(content)
