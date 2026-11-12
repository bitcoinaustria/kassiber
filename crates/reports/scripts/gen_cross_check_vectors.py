"""Freeze diagnostic reads and render inputs from the unchanged Python reference."""
import copy
import csv
from dataclasses import replace
import io
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber import importers
from kassiber.cli.handlers import _report_hooks
from kassiber.core import cross_check
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.rust_port.xlsx import RecordingWorkbook, STAMP

OUT = ROOT / 'crates/reports/tests/data/cross_check.json'
BASE = "UPDATE transactions SET external_id=id;"
CT = ['Type','Buy Amount','Buy Currency','Sell Amount','Sell Currency','Fee','Fee Currency','Exchange','Tx-ID','Date','Buy Value','Sell Value']
BP = ['Date (UTC)','Integration Name','Label','Outgoing Asset','Outgoing Amount','Incoming Asset','Incoming Amount','Fee Asset','Fee Amount','Trx. ID']


def encode(rows, tool='cointracking', delimiter=',', quotechar='"', header=None):
    stream = io.StringIO(newline='')
    writer = csv.DictWriter(stream, fieldnames=header or (CT if tool=='cointracking' else BP), delimiter=delimiter, quotechar=quotechar, extrasaction='ignore')
    writer.writeheader(); writer.writerows(rows)
    return stream.getvalue()


def ct(**changes):
    return {'Type':'Deposit','Buy Amount':'1.23456789001','Buy Currency':'BTC','Date':'2024-01-01T00:00:00Z','Exchange':'Cold source','Tx-ID':'in',**changes}


def bp(**changes):
    return {'Date (UTC)':'2024-01-01T00:00:00Z','Integration Name':'Cold source','Label':'Deposit','Incoming Asset':'BTC','Incoming Amount':'1.23456789001','Trx. ID':'in',**changes}


def cases():
    rows=[ct(),ct(Type='Spend', **{'Buy Amount':'','Buy Currency':'','Sell Amount':'0.00123456789','Sell Currency':'BTC','Fee':'0.00000000456','Fee Currency':'BTC','Tx-ID':'out','Date':'2024-12-31T23:30:00Z'}),ct(Type='Interest Income', **{'Buy Amount':'0.00000789001','Buy Currency':'LBTC','Tx-ID':'earn','Date':'2025-01-02T00:00:00Z','Exchange':'Hot source','Buy Value':'0.123'}),ct(**{'Buy Amount':'0.00000999999','Tx-ID':'excluded','Date':'2026-02-01T00:00:00Z'}),ct(Type='Futures',**{'Tx-ID':'bad'}),ct(**{'Tx-ID':'only-tool','Buy Amount':'2','Date':'2025-02-05T00:00:00Z'})]
    result=[]
    def add(name, values=None, args=None, sql='', tool='cointracking', text=None, **dialect):
        result.append(dict(name=name,tool=tool,csv=text if text is not None else encode(values if values is not None else rows,tool,**dialect),args_json=json.dumps(dict(tool=tool,**(args or {}))),sql=BASE+sql))
    add('baseline')
    add('bare-cr-lines',text=encode(rows).replace('\r\n','\r'))
    add('blockpit',[bp(),bp(Label='Fee',**{'Incoming Asset':'','Incoming Amount':'','Fee Asset':'BTC','Fee Amount':'0.000001','Trx. ID':'out'}),bp(Label='Non-taxable (out)',**{'Incoming Asset':'','Incoming Amount':'','Outgoing Asset':'BTC','Outgoing Amount':'0.00000100001','Trx. ID':'no-tax','Date (UTC)':'2025-02-01T00:00:00Z'})],tool='blockpit')
    for tool,mapping in [('cointracking',importers._COINTRACKING_TYPES),('blockpit',importers._BLOCKPIT_TYPES)]:
        for kind,mapped in mapping.items():
            direction=importers._GENERIC_LEDGER_TYPES[mapped.lower()][0]
            value=ct(Type=kind) if tool=='cointracking' else bp(Label=kind)
            if direction=='outbound':
                value.update({'Buy Amount':'','Buy Currency':'','Sell Amount':'0.00123456789','Sell Currency':'BTC','Tx-ID':'out'} if tool=='cointracking' else {'Incoming Asset':'','Incoming Amount':'','Outgoing Asset':'BTC','Outgoing Amount':'0.00123456789','Trx. ID':'out'})
            add(tool+'-type-'+kind,[value],tool=tool)
    for tool in ['cointracking','blockpit']:
        for delim in [',',';','\t','|']:
            for quote in ['"',"'"]:
                add(f'{tool}-dialect-{ord(delim)}-{ord(quote)}',[ct(Exchange='quoted " x, y; z|\nΩ') if tool=='cointracking' else bp(**{'Integration Name':'quoted " x, y; z|\nΩ'})],tool=tool,delimiter=delim,quotechar=quote)
    for name,args in [('at-year',{'year':2025}),('generic-year',{'year':2024}),('invalid-year',{'year':2008}),('float-year',{'year':2024.9}),('unicode-year',{'year':' ２０２４ '}),('bad-year',{'year':'bad'}),('one-wallet',{'wallets':['cold']}),('wallet-order',{'wallets':['hot','cold','hot']}),('empty-wallets',{'wallets':[]}),('missing-wallet',{'wallets':['missing']}),('source-filter',{'tool_sources':['Hot source']}),('empty-source-filter',{'tool_sources':[]}),('include-tool-wallets',{'include_tool_wallets':True})]:
        add(name,args=args,sql="UPDATE profiles SET tax_country='at' WHERE id='p';" if name=='at-year' else '')
    changes=[('stale',"UPDATE profiles SET journal_input_version=99;"),('unprocessed',"UPDATE profiles SET last_processed_at=NULL;"),('tool-wallet',"UPDATE wallets SET kind='cointracking' WHERE id='cold';"),('included-fee',"UPDATE transactions SET amount_includes_fee=1 WHERE id='out';"),('non-taxable-income',"UPDATE transactions SET taxability_override=0 WHERE id='earn';"),('no-entries','DELETE FROM journal_entries;'),('mixed-transfer',"INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,created_at) VALUES('move','ws','p','out','cold','2024-12-31T23:30:00Z','transfer_out','BTC',-1000,'fixed');"),('other-assets',"UPDATE transactions SET asset='USDT' WHERE id='no-tax'; UPDATE transactions SET asset='abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd' WHERE id='earn';"),('quarantine-read','UPDATE journal_quarantines SET reason=\'custody_quantity_unresolved\';'),('invalid-date',"UPDATE transactions SET occurred_at='broken' WHERE id='in';"),('invalid-internal-date',"UPDATE transactions SET occurred_at='broken' WHERE id='out';"),('tax-country-whitespace',"UPDATE profiles SET tax_country=' at ';"),('no-context',"DELETE FROM settings WHERE key='context_profile';"),('empty','DELETE FROM journal_entries; DELETE FROM transactions;')]
    for name,sql in changes:add(name,sql=sql)
    for field,value in [('Buy Amount','1.23456789999'),('Buy Amount','1.23456790001'),('Buy Amount','1.234567890005'),('Buy Amount','9.007199254740993'),('Buy Amount','(1,23456789001)'),('Buy Amount','bad'),('Buy Amount',''),('Buy Currency','ETH'),('Sell Currency','ETH'),('Sell Currency','BTC'),('Fee','bad'),('Fee','1'),('Date',''),('Date','bad'),('Date','31.12.2024 23:59:59'),('Date','2024-01-01T00:00:00.123456Z'),('Date','2024-W01-1T00:00:00Z'),('Date','20240101'),('Tx-ID',''),('Type',''),('Type','Trade'),('Type','Sell'),('Buy Value','99999,99')]:
        add(f'cell-{field}-{value}',[ct(**{field:value})])
    for fee in ['0','0.00000000455','0.00000001456','0.00000002']:
        add('outbound-fee-'+fee,[rows[1]|{'Fee':fee}])
    add('folded-fee',[rows[1]|{'Sell Amount':'0.00123458789','Fee':''}],sql="UPDATE transactions SET fee=2000 WHERE id='out';")
    add('same-total-split',[rows[1]|{'Sell Amount':'0.00123455789','Fee':'0.00000001456'}])
    add('cash-fee',[ct(**{'Sell Amount':'30000.005','Sell Currency':'EUR','Fee':'1.005','Fee Currency':'EUR'})])
    add('foreign-cash-value',[ct(**{'Sell Amount':'999','Sell Currency':'USD','Buy Value':'123,45'})])
    add('fee-needs-cash',[ct(**{'Sell Currency':'EUR','Fee':'1','Fee Currency':'EUR'})])
    add('standalone-other-fee',[ct(**{'Buy Currency':'ETH','Buy Amount':'1','Fee':'0.001','Fee Currency':'LBTC'})])
    add('standalone-liquid-fee',[ct(Type='Fee',**{'Buy Currency':'','Buy Amount':'','Fee':'0.001','Fee Currency':'LBTC','Tx-ID':'liquid-fee'})])
    add('invalid-other-fee',[ct(**{'Buy Currency':'ETH','Fee':'junk'})])
    add('duplicate-id',[ct(),ct(),ct(**{'Tx-ID':'IN'})])
    add('excluded-exact-before-probable',[ct(**{'Tx-ID':'excluded'})])
    add('probable-conflicting-chain-ids',[ct(**{'Tx-ID':'b'*64})],sql="UPDATE transactions SET external_id='"+'a'*64+"' WHERE id='in';")
    add('probable-after-26h',[ct(**{'Tx-ID':'','Date':'2024-01-02T02:00:00Z'})])
    add('probable-outside-26h',[ct(**{'Tx-ID':'','Date':'2024-01-02T02:00:00.000001Z'})])
    add('probable-before-26h',[ct(**{'Tx-ID':'','Date':'2023-12-30T22:00:00Z'})])
    add('probable-before-outside-26h',[ct(**{'Tx-ID':'','Date':'2023-12-30T21:59:59.999999Z'})])
    shifted=[ct(**{'Date':'2024-01-01T02:00:00Z'}),rows[1]|{'Date':'2025-01-01T01:30:00Z'},rows[2]|{'Date':'2025-01-02T02:00:00Z'}]
    add('clock-offset',shifted)
    add('clock-offset-filtered-year',shifted,args={'year':2024})
    add('clock-offset-no-majority',[*shifted,rows[1]|{'Tx-ID':'no-tax','Date':'2025-02-01T00:00:00Z'}])
    add('offset-not-whole-hour',[r|{'Date':r['Date'].replace(':00Z',':31Z')} for r in shifted])
    add('year-two-minute-drift',[rows[1]|{'Date':'2024-12-31T23:01:00Z'}],sql="UPDATE profiles SET tax_country='at'; UPDATE transactions SET occurred_at='2024-12-31T22:59:00Z' WHERE id='out';")
    add('invalid-interior-date',sql="UPDATE transactions SET occurred_at='2024-bad' WHERE id='out';")
    add('excluded-id-asset-blind',[ct(**{'Tx-ID':'shared','Date':'2024-01-01T00:00:00Z','Buy Amount':'0.00000999999'})],sql="UPDATE transactions SET excluded=1,external_id='shared' WHERE id IN ('earn','excluded');")
    add('active-id-nearest',[ct(**{'Tx-ID':'shared','Buy Amount':'2'})],sql="UPDATE transactions SET external_id='shared' WHERE id='in'; UPDATE transactions SET direction='inbound',external_id='shared',amount=200000000000 WHERE id='no-tax';")
    add('control-whitespace',[ct(Type='\x1c Deposit\x1f',Exchange='\x1cCold source\x1f',**{'Tx-ID':'\x1cin\x1f','Buy Amount':'\x1c1.23456789001\x1f'})])
    for date in ['３１.１２.２０２４ 23:59:59','٣١.١٢.٢٠٢٤ 23:59:59','٣١.١٢.2024 23:59:59','+1.1.2024']:
        add('european-date-'+date,[ct(Date=date)])
    add('unsupported-asset-hex',[ct(**{'Sell Currency':'ABCDEF'*10+'ABCD'})])
    for field in ['Buy Amount','Fee','Buy Value','Sell Amount']:
        for special in ['NaN','sNaN','Infinity','-Infinity']:
            add(f'nonfinite-{field}-{special}',[ct(**{field:special,'Fee Currency':'BTC',**({'Sell Currency':'EUR'} if field=='Sell Amount' else {})})])
    add('nan-valued-comparison',[ct(**{'Buy Value':'NaN'})],sql="UPDATE transactions SET fiat_value_exact='1',fiat_currency='EUR' WHERE id='in';")
    add('nan-valued-payload',[ct(**{'Buy Value':'NaN12'})])
    add('nan-valued-negative-payload',[ct(**{'Buy Value':'-NaN12'})])
    add('nan-income-value',[ct(Type='Income',**{'Buy Value':'NaN'})])
    for special in ['NaN','NaN12','-NaN12','sNaN','Infinity','-Infinity']:
        add('journal-proceeds-'+special,sql=f"UPDATE journal_entries SET proceeds_exact='{special}' WHERE id='hidden';")
    add('journal-income-NaN',sql="UPDATE journal_entries SET fiat_value_exact='NaN' WHERE id='income';")
    # Empty exact journal cash falls back to the numeric value (141eabc5f).
    add('journal-proceeds-empty-exact',sql="UPDATE journal_entries SET proceeds_exact='' WHERE id='hidden';")
    add('journal-income-empty-exact',sql="UPDATE journal_entries SET fiat_value_exact='' WHERE id='income';")
    # A non-finite stored value refuses the comparison (f3a0289be).
    for tx in ['in','out','earn','excluded']:
        add('stored-value-NaN-'+tx,sql=f"UPDATE transactions SET fiat_value_exact='NaN',fiat_currency='EUR' WHERE id='{tx}';")
    for date in ['0001-01-01T00:00:00+01:00','9999-12-31T23:59:59-01:00','0001-01-01T00:00:00Z']:
        add('date-bound-'+date,[ct(Date=date)])
    for date in ['0001-01-01T00:00:00Z','9999-12-31T23:59:59Z']:
        add('probable-date-bound-'+date,[ct(Date=date,**{'Tx-ID':''})])
    for value in [float('inf'),float('-inf'),float('nan')]:add('year-nonfinite-'+str(value),args={'year':value})
    for value in ['1','1.005','1.01','100','101','101.0001']:
        add('money-tolerance-'+value,[ct(**{'Buy Value':value})],sql="UPDATE transactions SET fiat_value_exact='100',fiat_currency='EUR' WHERE id='in';")
    add('income-unvalued',[ct(Type='Income')])
    add('journal-income-authority',sql="UPDATE transactions SET fiat_value_exact='500',fiat_currency='EUR' WHERE id='earn';")
    add('neutral-unpaired-category',sql="UPDATE journal_entries SET at_category='neu_swap' WHERE id='hidden';")
    add('invalid-bounds-date-whitespace',sql="UPDATE transactions SET occurred_at=' 2024-01-01T00:00:00Z ' WHERE id='in';")
    for n in [2,3]:add('clock-minimum-'+str(n),shifted[:n])
    add('clock-cap',[r|{'Date':r['Date'].replace('02:00:00','15:00:00').replace('01:30:00','14:30:00')} for r in shifted])
    for text in ['', 'Summary\n123\n', '\ufeff'+encode(rows), '\r\n'+encode(rows), encode(rows).replace(',Cold source,',', Cold source,'), encode([ct()]).replace('Buy Amount',' Buy\u00a0 Amount '), encode([ct()]).replace('Buy Amount','Date')]:
        add('raw-'+str(len(result)),text=text)
    return result


def outcome(conn,args,path,target=None):
    options=dict(tool=args['tool'],file_path=path,wallet_ref=args.get('wallet'),wallet_refs=args.get('wallets'),year=args.get('year'),tool_sources=args.get('tool_sources'),include_tool_wallets=args.get('include_tool_wallets',False))
    try:
        if target is None:
            return dict(ok=cross_check.public_cross_check(cross_check.cross_check(conn,args.get('workspace'),args.get('profile'),_report_hooks(),**options)))
        return dict(ok=cross_check.export_cross_check_xlsx(conn,args.get('workspace'),args.get('profile'),target,replace(_report_hooks(),now_iso=lambda:STAMP),**options))
    except AppError as e:
        return dict(error=dict(code=e.code,message=str(e),hint=e.hint,details=e.details,retryable=e.retryable))
    except (ValueError,TypeError,OverflowError,AttributeError,ArithmeticError,csv.Error) as e:
        return dict(error=dict(code='internal_error',message=str(e),hint=None,details=None,retryable=False))


def build(root):
    with tempfile.TemporaryDirectory(dir=root) as directory:
        seed=open_db(Path(directory)/'data')
        database=Path(seed.execute('PRAGMA database_list').fetchone()[2])
        seed.commit();seed.close();template=database.read_bytes()
    result=[]
    for case in cases():
        with tempfile.TemporaryDirectory(dir=root) as directory:
            directory=Path(directory);database=directory/'book.sqlite';database.write_bytes(template)
            conn=sqlite3.connect(database);conn.row_factory=sqlite3.Row
            conn.execute('PRAGMA foreign_keys=ON')
            conn.executescript('BEGIN;\n'+(ROOT/'crates/reports/tests/data/book.sql').read_text()+case['sql'])
            path=directory/'export.csv';path.write_text(case['csv'],newline='')
            expected=outcome(conn,json.loads(case['args_json']),path)
            document=None
            if 'ok' in expected:
                captured=[]
                def factory(*args,**kwargs):
                    book=RecordingWorkbook(*args,**kwargs);captured.append(book.document);return book
                with patch('xlsxwriter.Workbook',factory):
                    exported=outcome(conn,json.loads(case['args_json']),path,directory/'report.xlsx')
                if 'ok' in exported:
                    document=captured[0]
                else:
                    case=dict(**case,workbook_error=exported['error'])
            conn.close()
            result.append(dict(**case,expected_json=json.dumps(expected,sort_keys=True,separators=(',',':')),document=document))
    return result


if __name__=='__main__':
    scratch=ROOT/'target/r5-reports16/scratch';scratch.mkdir(parents=True,exist_ok=True)
    content='[\n'+',\n'.join(json.dumps(case,ensure_ascii=True,separators=(',',':')) for case in build(scratch))+'\n]\n'
    if '--check' in sys.argv:
        assert OUT.read_text()==content,'cross-check vectors drifted'
    else:
        OUT.write_text(content)
