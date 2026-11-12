import copy
import json
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import report_explanation, ui_snapshot
from kassiber.core.journal_calculation import encode_calculation, PRICING_KEYS
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, open_db
from kassiber.errors import AppError
from crates.reports.scripts.gen_exit_tax_vectors import FixedDateTime

OUT = ROOT / 'crates/reports/tests/data/gain_reads.json'
REF = dict(database_id='0123456789abcdef0123456789abcdef', workspace_id='ws', profile_id='p', input_version=7, processed_at='2026-01-01T00:00:00Z', entry_id='z')


def quote(value):
    return "'" + value.replace("'", "''") + "'"


def calculation():
    def source(tx, date, asset='BTC'):
        return dict(asset=asset, crypto_fee_msat=123, crypto_fee_msat_exact='123', engine_event_id='event €', fiat_fee_exact='0.00001', occurred_at=date,
                    pricing={k: 'manual' if k == 'pricing_method' else None for k in PRICING_KEYS}, spot_price_exact='30000.123', transaction_id=tx)
    fragments = []
    for quantity, basis, proceeds, gain in [(100000000, '10', '12', '2'), (1, '0.00000001', '0.34', '0.33999999')]:
        fragments.append(dict(acquisition_basis_exact='10000.0000000000000000001', acquisition_quantity_msat=9007199254740993, acquisition_quantity_msat_exact='9007199254740993', basis_authority='engine', cost_basis_exact=basis, event=source('out','2024-12-31T23:30:00Z'), gain_loss_exact=gain, lot=source('in','2024-01-01T00:00:00Z'), proceeds_exact=proceeds, quantity_msat=quantity, quantity_msat_exact=str(quantity), unit_basis_override_exact=None))
    return dict(engine='kassiber-tax', fragments=fragments, method='fifo', schema_version=1)


def stored(calc=None, compact=True, entry='z'):
    calc = calculation() if calc is None else calc
    legacy, encoded = encode_calculation(calc) if compact else (json.dumps(calc, sort_keys=True), None)
    return f"UPDATE journal_entries SET calculation_json={quote(legacy) if legacy else 'NULL'},calculation_compact={quote(encoded) if encoded else 'NULL'},cost_basis_exact='10.00000001',proceeds_exact='12.34',gain_loss_exact='2.33999999' WHERE id={quote(entry)};"


BASE = stored()


def cases():
    result = []
    def explain(name, sql='', reference=REF):
        result.append((name, 'reports.explain-capital-gain', dict(reference=json.dumps(reference)), sql))
    explain('compact')
    explain('legacy', stored(compact=False))
    explain('unavailable', 'UPDATE journal_entries SET calculation_compact=NULL;')
    for field in ['cost_basis_exact','proceeds_exact','gain_loss_exact']:
        explain('missing-' + field, f'UPDATE journal_entries SET {field}=NULL;')
        explain('mismatch-' + field, f"UPDATE journal_entries SET {field}='999';")
    explain('mismatch-quantity', "UPDATE journal_entries SET quantity=2 WHERE id='z';")
    for name, raw in [('bad-json','{'),('bad-version','[1,"e","m",[],[]]'),('bool-version','[true,"e","m",[],[]]'),('bad-width','[2,"e","m",[],[[1]]]'),('bad-source-index','[2,"e","m",[],[[100000001,"10.00000001","12.34","2.33999999",null,null,null,null,0,null]]]')]:
        explain(name, f'UPDATE journal_entries SET calculation_compact={quote(raw)};')
    for field in ['cost_basis_exact','proceeds_exact','gain_loss_exact']:
        calc=calculation();calc['fragments'][0][field]='bad';explain('corrupt-'+field,stored(calc))
    calc=calculation();calc['fragments'][0]['event']=None;calc['fragments'][0]['lot']=None
    calc['fragments'][1]['event']=None;calc['fragments'][1]['lot']=None
    explain('no-source-dates',stored(calc))
    calc=calculation();calc['fragments'][0]['lot']['pricing']['pricing_method']='carrying_value';calc['fragments'][0]['lot']['transaction_id']='earn';calc['fragments'][0]['lot']['asset']='LBTC'
    explain('inherited-missing-source',stored(calc))
    explain('inherited-source', stored(calc) + stored(entry='a') + "UPDATE journal_entries SET transaction_id='out',asset='BTC' WHERE id='a';")
    inherited=calculation();inherited['fragments'][0]['lot']['pricing']['pricing_method']='carrying_value';inherited['fragments'][0]['lot']['transaction_id']='earn';inherited['fragments'][0]['lot']['asset']='LBTC'
    explain('inherited-cycle',stored(calc)+stored(inherited,entry='a'))
    explain('compact-precedes-corrupt-legacy', "UPDATE journal_entries SET calculation_json='{' WHERE id='z';")
    explain('corrupt-compact-precedes-legacy',stored(compact=False)+"UPDATE journal_entries SET calculation_compact='{' WHERE id='z';")
    for label, change in [('bad-source-width',lambda x:x[3][0].pop()),('bad-source-fee',lambda x:x[3][0].__setitem__(6,True)),('bool-quantity',lambda x:x[4][0].__setitem__(0,True)),('bool-index',lambda x:x[4][0].__setitem__(8,True)),('negative-index',lambda x:x[4][0].__setitem__(8,-1)),('bad-acquired',lambda x:x[4][0].__setitem__(7,False))]:
        encoded=json.loads(encode_calculation(calculation())[1]);change(encoded)
        explain(label,"UPDATE journal_entries SET calculation_compact="+quote(json.dumps(encoded))+" WHERE id='z';")
    for label,change in [('missing-event',lambda c:c['fragments'][0].pop('event')),('null-pricing',lambda c:c['fragments'][0]['lot'].__setitem__('pricing',None)),('number-lot',lambda c:c['fragments'][0].__setitem__('lot',1))]:
        calc=calculation();change(calc);explain(label,stored(calc,compact=False))
    columns=('decision_id workspace_id profile_id source_transaction_id target_transaction_id source_observation_hash source_start_msat source_end_msat target_observation_hash target_start_msat target_end_msat source_wallet_id target_wallet_id source_network target_network source_rail target_rail source_asset target_asset state basis_state reason occurred_at target_occurred_at created_at').split()
    def decisions(count):
        select=["printf('%064d',x+1000)" if c=='decision_id' else "'2024-03-01T00:00:00Z'" if c=='target_occurred_at' else "'cold'" if c=='target_wallet_id' else "'BTC'" if c=='target_asset' else 'd.'+c for c in columns]
        return f"WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<{count}) INSERT INTO journal_custody_decisions({','.join(columns)}) SELECT {','.join(select)} FROM n CROSS JOIN journal_custody_decisions d WHERE d.decision_id=printf('%064d',1);"
    explain('custody-500-record-bound',decisions(501))
    explain('inherited-100-record-bound',stored(calc:=calculation())+decisions(101)+stored(entry='a'))
    calc=calculation();calc['fragments'][0]['lot']['pricing']['pricing_method']='carrying_value';calc['fragments'][0]['lot']['transaction_id']='earn';calc['fragments'][0]['lot']['asset']='LBTC'
    explain('inherited-100-record-bound-material',stored(calc)+"WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<101) INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,cost_basis_exact,proceeds_exact,gain_loss_exact,calculation_compact,created_at) SELECT printf('source%03d',x),'ws','p','out','cold','2024-01-01T00:00:00Z','disposal','BTC',-100000001,'10.00000001','12.34','2.33999999',"+quote(encode_calculation(calculation())[1])+",'fixed' FROM n;")
    explain('income-entry',reference={**REF,'entry_id':'income'})
    from decimal import Decimal, localcontext
    calc=calculation();calc['fragments'][0]['cost_basis_exact']='0.12345678901234567890123456789012'
    with localcontext() as context:
        context.prec=32
        total=sum((Decimal(f['cost_basis_exact']) for f in calc['fragments']),Decimal(0))
    explain('precision-32',stored(calc)+"UPDATE journal_entries SET cost_basis_exact="+quote(str(total))+" WHERE id='z';")
    calc=calculation();calc['fragments'][0]['lot']['pricing']['pricing_method']='carrying_value';calc['fragments'][0]['lot']['transaction_id']='earn';calc['fragments'][0]['lot']['asset']='LBTC'
    explain('inherited-ineligible',stored(calc)+"UPDATE journal_custody_decisions SET basis_state='blocked_by_prior_custody_basis';")
    chain=[]
    for index in range(10):
        chain.append(f"INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,excluded,occurred_at,created_at) VALUES('hop{index}','ws','p','cold','hop{index}','BTC','inbound',100000001,0,1,'2024-01-01T00:00:00Z','fixed');")
    for index in range(9):
        encoded=calculation();encoded['fragments'][0]['lot']['pricing']['pricing_method']='carrying_value';encoded['fragments'][0]['lot']['transaction_id']=f'hop{index+1}'
        chain.append(f"INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,created_at) VALUES('entry-hop{index+1}','ws','p','hop{index+1}','cold','2024-01-01T00:00:00Z','disposal','BTC',-100000001,'fixed');")
        chain.append(stored(encoded,entry=f'entry-hop{index+1}'))
        select=[quote(f'{index+2000:064d}') if c=='decision_id' else quote(f'hop{index+1}') if c=='source_transaction_id' else quote(f'hop{index}') if c=='target_transaction_id' else "'cold'" if c in ('target_wallet_id','source_wallet_id') else "'BTC'" if c in ('source_asset','target_asset') else 'd.'+c for c in columns]
        chain.append(f"INSERT INTO journal_custody_decisions({','.join(columns)}) SELECT {','.join(select)} FROM journal_custody_decisions d WHERE d.decision_id=printf('%064d',1);")
    calc=calculation();calc['fragments'][0]['lot']['pricing']['pricing_method']='carrying_value';calc['fragments'][0]['lot']['transaction_id']='hop0'
    explain('inherited-eight-depth-bound',stored(calc)+''.join(chain))
    explain('component-blocked',"INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at) VALUES('incomplete','lineage','ws','p',1,'migration','active','fixed');")
    explain('custody-at-cutoff',"UPDATE journal_custody_decisions SET target_occurred_at='2024-12-31T23:30:00Z';")
    for label,change in [('number-event',lambda c:c['fragments'][0].__setitem__('event',1)),('event-missing-transaction',lambda c:c['fragments'][0]['event'].pop('transaction_id')),('event-missing-date',lambda c:c['fragments'][0]['event'].pop('occurred_at')),('carrying-missing-asset',lambda c:(c['fragments'][0]['lot']['pricing'].__setitem__('pricing_method','carrying_value'),c['fragments'][0]['lot'].pop('asset')))]:
        calc=calculation();change(calc);explain(label,stored(calc,compact=False))
    explain('freshness-current-reference',"UPDATE profiles SET journal_input_version=8 WHERE id='p';",{**REF,'input_version':8})
    explain('unprocessed-current-reference',"UPDATE profiles SET last_processed_at=NULL WHERE id='p';",{**REF,'processed_at':None})
    for label,raw in [('float',1.1),('bool',True)]:
        calc=calculation();calc['fragments'][0]['cost_basis_exact']=raw
        with localcontext() as context:
            context.prec=32
            total=sum((Decimal(f['cost_basis_exact']) for f in calc['fragments']),Decimal(0))
        explain('legacy-number-'+label,stored(calc,compact=False)+"UPDATE journal_entries SET cost_basis_exact="+quote(str(total))+" WHERE id='z';")
    calc=calculation();calc['fragments'][0]['quantity_msat']=100000000.0
    explain('legacy-float-quantity',stored(calc,compact=False))
    calc=calculation();calc['fragments'][0]['cost_basis_exact']='\u200310.00\u2003'
    explain('legacy-decimal-whitespace',stored(calc,compact=False))
    for key in ['journal_input_version','last_processed_input_version','last_processed_tx_count']:
        explain('invalid-profile-'+key,f"UPDATE profiles SET {key}='bad' WHERE id='p';")
    explain('invalid-unprocessed-profile-count',"UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count='bad' WHERE id='p';",{**REF,'processed_at':None})
    explain('invalid-stale-profile-version',"UPDATE profiles SET last_processed_tx_count=0,last_processed_input_version='bad' WHERE id='p';")
    explain('float-profile-versions',"UPDATE profiles SET journal_input_version=7.9,last_processed_input_version=7.1,last_processed_tx_count=4.5 WHERE id='p'; UPDATE profiles SET journal_semantics_version="+str(JOURNAL_CALCULATION_SEMANTICS_VERSION)+" WHERE id='p';")
    explain('old-semantics',"UPDATE profiles SET required_journal_semantics_version=2,journal_semantics_version=1 WHERE id='p';")
    for field, value in [('database_id','0'*32),('workspace_id','other'),('profile_id','other'),('input_version',8),('processed_at','old'),('entry_id','absent')]:
        explain('stale-'+field,reference={**REF,field:value})
    for name, value in [('null',None),('list',[]),('empty',{}),('bool-version',{**REF,'input_version':True}),('number-entry',{**REF,'entry_id':1}),('extra',{**REF,'extra':1})]:
        explain('invalid-'+name,reference=value)
    for name, sql in [('stale',"UPDATE profiles SET journal_input_version=8 WHERE id='p';"),('unprocessed',"UPDATE profiles SET last_processed_at=NULL WHERE id='p';"),('missing-entry',"DELETE FROM journal_entries WHERE id='z';"),('nontaxable',"UPDATE transactions SET taxability_override=0 WHERE id='out';"),('swap',"UPDATE journal_entries SET at_category='neu_swap' WHERE id='z';"),('unsupported-country',"UPDATE profiles SET tax_country='xx' WHERE id='p';"),('invalid-identity',"DELETE FROM settings WHERE key='database_instance_id';"),('custody-quantity',"INSERT INTO journal_quantity_issues(issue_id,workspace_id,profile_id,issue_type,state,asset,amount_msat,reason,created_at) VALUES('issue','ws','p','shortfall','custody_suspense','BTC',1,'review','fixed');")]:
        explain(name,sql)
    explain('stale-precedes-custody', "UPDATE profiles SET journal_input_version=8 WHERE id='p';" )
    for reference in ['bad','', '{"entry_id":']:
        result.append(('invalid-json-'+str(len(reference)),'reports.explain-capital-gain',dict(reference=reference),''))
    for sqlname, sql in [('base',''),('at',"UPDATE profiles SET tax_country='at',gains_algorithm='HIFO' WHERE id='p';"),('stale',"UPDATE profiles SET journal_input_version=8 WHERE id='p';"),('unprocessed',"UPDATE profiles SET last_processed_at=NULL WHERE id='p';"),('no-context',"DELETE FROM settings WHERE key='context_profile';"),('crossed-context',"UPDATE settings SET value='missing' WHERE key='context_workspace';"),('empty', 'DELETE FROM journal_entries; DELETE FROM transactions;'),('long',"UPDATE journal_entries SET capital_gains_type='long';"),('income-zero',"UPDATE journal_entries SET gain_loss=0,proceeds=0 WHERE entry_type='income';"),('large-msat',"UPDATE journal_entries SET quantity=-9007199254740993 WHERE id='z';"),('null-proceeds',"UPDATE journal_entries SET proceeds=NULL WHERE id='z';"),('null-category',"UPDATE journal_entries SET at_category=NULL,at_kennzahl=NULL;"),('invalid-date',"UPDATE journal_entries SET occurred_at='broken' WHERE id='z';"),('offset',"UPDATE profiles SET tax_country='at' WHERE id='p'; UPDATE journal_entries SET occurred_at='2024-01-01T00:30:00+02:00' WHERE id='z';"),('out-of-bounds',"UPDATE journal_entries SET occurred_at='2008-01-01T00:00:00Z' WHERE id='z';"),('cap',"WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<205) INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,cost_basis,proceeds,gain_loss,created_at) SELECT printf('cap%03d',x),'ws','p','out','cold','2024-01-01T00:00:00Z','disposal','BTC',-x,x,x+1,1,printf('%03d',x) FROM n;")]:
        for year in [None,2024,2025]:
            result.append((f'desktop-{sqlname}-{year}','ui.reports.capital_gains',dict(year=year),sql))
    # Main sync 1 (#706): the Reports screen payload and the crypto Kennzahlen
    # from 1 March 2022 (Europe/Vienna).
    at="UPDATE profiles SET tax_country='at' WHERE id='p';"
    conversions="INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,target_transaction_id,source_asset,target_asset,source_amount_msat,target_amount_msat,basis_state,occurred_at,target_occurred_at,created_at) VALUES(printf('%064d',6),'ws','p','conversion','out','earn','BTC','LBTC',5000,4999,'eligible','2024-12-31T23:30:00Z','2025-01-02T00:00:00Z','d'),(printf('%064d',7),'ws','p','conversion','out','in','BTC','BTC',5001,5001,'eligible','2024-12-31T23:30:00Z','2025-01-02T00:00:00Z','e');"
    links="WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<{count}) INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,excluded,occurred_at,created_at) SELECT printf('tx%03d',x),'ws','p','cold',printf('tx%03d',x),'BTC','outbound',x,0,0,'2025-06-01T00:00:00Z','fixed' FROM n; UPDATE profiles SET last_processed_tx_count=last_processed_tx_count+{count} WHERE id='p';"+"WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<{count}) INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,cost_basis,proceeds,gain_loss,at_category,at_kennzahl,capital_gains_type,created_at) SELECT printf('link%03d',x),'ws','p',printf('tx%03d',x),'cold','2025-06-01T00:00:00Z','disposal','BTC',-x,0.1,0.25,0.15,'neu_gain',174,'short_term',printf('%03d',x) FROM n;"
    for sqlname, sql, years in [
        ('at-fields', at+"UPDATE journal_entries SET at_category='alt_spekulation',at_kennzahl=801 WHERE id='z'; UPDATE journal_entries SET at_category='neu_loss',at_kennzahl=176,gain_loss=-0.5 WHERE id='first';", [None,2025]),
        ('at-swaps', at+conversions, [2025]),
        ('generic-swaps', conversions, [2024]),
        ('at-old-rules', at+"UPDATE journal_entries SET occurred_at='2022-02-10T10:00:00Z' WHERE id IN ('a','swap'); UPDATE journal_entries SET occurred_at='2022-05-10T10:00:00Z' WHERE id IN ('income','z','first');", [None,2022]),
        ('at-boundary', at+"UPDATE journal_entries SET occurred_at='2022-02-28T22:59:59Z' WHERE id='a'; UPDATE journal_entries SET occurred_at='2022-02-28T23:00:00Z' WHERE id='income';", [2022]),
        ('at-after-boundary', at+"UPDATE journal_entries SET occurred_at='2022-02-28T23:00:00Z' WHERE id IN ('a','income');", [2022]),
        ('at-before-e1kv', at+"UPDATE journal_entries SET occurred_at='2021-06-01T10:00:00Z'; UPDATE journal_entries SET at_category='alt_spekulation',at_kennzahl=801 WHERE id='z';", [None,2021]),
        ('at-usd', "UPDATE profiles SET tax_country='at',fiat_currency='USD' WHERE id='p';", [2025]),
        ('at-lower-eur', at+"UPDATE profiles SET fiat_currency='eur' WHERE id='p';", [2025]),
        ('at-net-loss', at+"UPDATE journal_entries SET at_category='neu_loss',at_kennzahl=176,gain_loss=-5 WHERE id='a';", [2025]),
        ('at-links-128', at+links.format(count=127), [2025]),
        ('at-links-129', at+links.format(count=128), [2025]),
        ('generic-links-129', links.format(count=129), [2025]),
        ('generic-long-days', "UPDATE profiles SET tax_long_term_days=730 WHERE id='p';", [2024]),
        ('generic-zero-days', "UPDATE profiles SET tax_long_term_days=0 WHERE id='p';", [2024]),
        ('generic-negative-days', "UPDATE profiles SET tax_long_term_days=-1 WHERE id='p';", [2024]),
        ('generic-text-days', "UPDATE profiles SET tax_long_term_days=' 400 ' WHERE id='p';", [2024]),
        ('country-empty', "UPDATE profiles SET tax_country='' WHERE id='p';", [2024]),
        ('country-padded-at', "UPDATE profiles SET tax_country=' at' WHERE id='p';", [2024]),
        ('country-unsupported', "UPDATE profiles SET tax_country='xx' WHERE id='p';", [2024]),
        ('fiat-empty', "UPDATE profiles SET fiat_currency='' WHERE id='p';", [2024]),
        ('generic-income-long', "UPDATE journal_entries SET capital_gains_type='LONG' WHERE id='a'; UPDATE journal_entries SET gain_loss=0 WHERE id='income';", [2024,2025]),
        ('half-cent', "UPDATE journal_entries SET cost_basis=0.005,proceeds=0.015,gain_loss=0.01 WHERE id IN ('a','z');", [2025]),
        ('kennzahl-text', at+"UPDATE journal_entries SET at_kennzahl='174' WHERE id='a';", [2025]),
    ]:
        for year in years:
            result.append((f'desktop-{sqlname}-{year}','ui.reports.capital_gains',dict(year=year),sql))
    for year in ['2024',' ２０２４ ',2024.9,True,False,2008,2101,[],{},'bad']:
        result.append(('desktop-year-'+repr(year),'ui.reports.capital_gains',dict(year=year),''))
    for missing in [False,True]:
        for label,raw in [('null',None),('list',[]),('number',1),('empty',{}),('valid',dict(reference=REF)),('stale',dict(reference={**REF,'input_version':-1}))]:
            result.append((f'daemon-reference-{missing}-{label}','ui.reports.explain_capital_gain',dict(raw_args=raw),"DELETE FROM settings WHERE key='context_profile';" if missing else ''))
    return result


def invoke(conn, command, args):
    if command == 'ui.reports.capital_gains':
        with patch.object(ui_snapshot,'datetime',FixedDateTime):
            return ui_snapshot.build_capital_gains_snapshot(conn,args.get('year'))
    from kassiber.core.repo import resolve_scope
    if command=='ui.reports.explain_capital_gain':
        _,profile=ui_snapshot._active_context_and_profile(conn)
        if profile is None:
            raise AppError('Select a book before explaining a report',code='not_found')
        args=args.get('raw_args',args)
        if args is not None and not isinstance(args,dict):
            raise AppError('daemon args must be an object',code='validation',details=dict(type=type(args).__name__))
        args=args or {}
    else:
        _,profile=resolve_scope(conn,args.get('workspace'),args.get('profile'))
    reference=args.get('reference')
    if command.startswith('reports.'):
        try:
            reference=json.loads(reference)
        except ValueError as exc:
            raise AppError('Result reference must be JSON',code='validation') from exc
    return report_explanation.explain_capital_gain(conn,profile,reference)


def outcome(conn, command, args):
    try:
        return dict(ok=invoke(conn,command,args))
    except AppError as e:
        return dict(error=dict(code=e.code,message=str(e),hint=e.hint,details=e.details,retryable=e.retryable))
    except (ValueError,TypeError,OverflowError,AttributeError,KeyError) as e:
        return dict(error=dict(code='internal_error',message=str(e),hint=None,details=None,retryable=False))


def build(scratch):
    result=[]
    for name,command,args,sql in cases():
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            conn=open_db(Path(directory)/'data')
            conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE+sql)
            conn.commit()
            expected=outcome(conn,command,args)
            result.append(dict(name=name,command=command,sql=BASE+sql,args_json=json.dumps(args),expected=expected,expected_json=json.dumps(expected,sort_keys=True,separators=(',',':'))))
            conn.close()
    import itertools
    import uuid
    from tests.test_report_explanation import _book
    for method,country in [('fifo','generic'),('lifo','generic'),('hifo','generic'),('lofo','generic'),('moving_average_at','at')]:
        with tempfile.TemporaryDirectory(dir=scratch) as directory, patch('uuid.uuid4',side_effect=lambda counter=itertools.count(1):uuid.UUID(int=next(counter))):
            conn,profile,reference=_book(Path(directory)/'data',method,country)
            conn.execute("UPDATE profiles SET last_processed_at='2026-01-01T00:00:00Z'")
            conn.execute("UPDATE profiles SET journal_semantics_version=?",(JOURNAL_CALCULATION_SEMANTICS_VERSION,))
            conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('database_instance_id','0123456789abcdef0123456789abcdef'),('context_workspace','w'),('context_profile','p')")
            conn.execute("UPDATE journal_entries SET created_at='fixed'")
            conn.commit()
            profile=conn.execute("SELECT * FROM profiles").fetchone()
            reference=report_explanation.result_reference(conn,profile,reference['entry_id'])
            tables=['workspaces','profiles','accounts','wallets','transactions','journal_entries','journal_quarantines','journal_custody_decisions','journal_custody_economic_relations','settings']
            inserts=[line for line in conn.iterdump() if line.startswith('INSERT INTO ')]
            sql='\n'.join(line.replace('INSERT INTO','INSERT OR REPLACE INTO',1) if table=='settings' else line for table in tables for line in inserts if line.split('"')[1]==table)
            for command,args in [('reports.explain-capital-gain',dict(reference=json.dumps(reference))),('ui.reports.capital_gains',dict(year=2024))]:
                expected=outcome(conn,command,args)
                result.append(dict(name='engine-'+method+'-'+command,base=False,command=command,sql=sql,args_json=json.dumps(args),expected=expected,expected_json=json.dumps(expected,sort_keys=True,separators=(',',':'))))
            conn.close()
    return result


if __name__=='__main__':
    scratch=ROOT/'target/r5-reports15/scratch';scratch.mkdir(parents=True,exist_ok=True)
    content='[\n'+',\n'.join(json.dumps(case,ensure_ascii=True,separators=(',',':')) for case in build(scratch))+'\n]\n'
    if '--check' in sys.argv:
        assert OUT.read_text()==content,'gain reads drifted'
    else:
        OUT.write_text(content)
