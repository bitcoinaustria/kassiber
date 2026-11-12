#!/usr/bin/env python3
"""Freeze Bull's Python parser branches; coordinator parity uses the book oracle."""
import argparse
import csv
import io
import json
import tempfile
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
from kassiber.core.imports import normalize_import_record
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.importers import load_import_records
from kassiber.msat import btc_to_msat
from tests.test_cli_smoke import _BULLBITCOIN_ORDERS_CSV
from generate_exchange_vectors import clean

DEST = Path(__file__).resolve().parents[1] / 'tests/vectors/python_bull_imports.json'
HEADER = ['ORDER_STATUS','PAYIN_STATUS','PAYOUT_STATUS','PAYIN_AMOUNT','PAYIN_CURRENCY','PAYOUT_AMOUNT','PAYOUT_CURRENCY','COMPLETED_AT (UTC)','SENT_AT (UTC)','CREATED_AT (UTC)','TRANSACTION_ID','EXCHANGE_RATE_AMOUNT','ORDER_ID','ORDER_NUMBER','PAYIN_METHOD','PAYOUT_METHOD']
BASE = dict(zip(HEADER,['completed','completed','completed','100.00','EUR','0.002','BTC','2025-01-02T01:02:03+01:00','','','ab'*32,'','order-1','number-1','wire','onchain']))

def inputs():
    cases = {'smoke':_BULLBITCOIN_ORDERS_CSV,'empty':'','header-only':','.join(HEADER)+'\n','blank-header':'\nx\n','utf8':b'\xff','utf16':','.join(HEADER).encode('utf-16')}
    def add(name,changes=None,*,header=None,rows=None,dialect=None):
        header=HEADER if header is None else header
        output=io.StringIO(newline='');writer=csv.writer(output,**(dialect or {}));writer.writerow(header)
        for row in rows if rows is not None else [{**BASE,**(changes or {})}]: writer.writerow([row.get(k,row.get(k.strip(),'')) for k in header])
        cases[name]=output.getvalue()
    add('base');cases['bom']='\ufeff'+cases['base'];add('semicolon',dialect={'delimiter':';'});add('tab',dialect={'delimiter':'\t'})
    for key in HEADER:
        add('missing-'+key,header=[k for k in HEADER if k!=key])
        for blank in ('',' ','\t'):add('blank-'+key+repr(blank),{key:blank})
    for style,transform in [('lower',str.lower),('upper',str.upper),('spaces',lambda k:' \t'+k+'  '),('whitespace',lambda k:k.replace(' ','\u00a0\t')),('long-s',lambda k:k.replace('S','ſ')),('ligature',lambda k:k.replace('ST','ﬆ'))]:
        add('headers-'+style,header=[transform(k) for k in HEADER],rows=[{transform(k):v for k,v in BASE.items()}])
    for aliases in (['TRANSACTION_ID','transaction_id'],['transaction_id','TRANSACTION_ID'],[' TRANSACTION_ID ','TRANSACTION_ID'],['TRANSACTION_ID',' TRANSACTION_ID ']):
        add('collision-'+repr(aliases),header=HEADER+aliases,changes={aliases[0]:'first',aliases[1]:'last'})
    for key in ('ORDER_STATUS','PAYIN_STATUS','PAYOUT_STATUS'):
        for v in ('pending','cancelled','Completed',' COMPLETED ','completed\u00a0','COMPLETED!'):add('status-'+key+v,{key:v})
    for key in ('PAYIN_AMOUNT','PAYOUT_AMOUNT','EXCHANGE_RATE_AMOUNT'):
        for v in ('0','-0','-1','1e-12','0.000000000005','0.0000000000049','1.23456789012345678901234567890123456','($1,234.50)','( -2 )',' 0.002 LBTC ','$ EUR 1,234.50','１２.３４','١_٢e-٣','NaN','-NaN42','sNaN','Infinity','-Infinity','bad','1e309'):
            add('number-'+key+v,{key:v})
    for currency in ('BTC','btc','XBT','XXBT','LBTC','L-BTC','EUR','USD','ETH',''):
        for key in ('PAYIN_CURRENCY','PAYOUT_CURRENCY'):add('currency-'+key+currency,{key:currency})
    sell={**BASE,'PAYIN_AMOUNT':'0.002','PAYIN_CURRENCY':'LBTC','PAYOUT_AMOUNT':'100','PAYOUT_CURRENCY':'EUR'}
    add('sell',rows=[sell]);add('explicit-rate',{'EXCHANGE_RATE_AMOUNT':'49999.12345678901234567890123456789'})
    add('both-crypto',{'PAYIN_CURRENCY':'BTC'});add('both-fiat',{'PAYOUT_CURRENCY':'USD'})
    for stamp in ('2025-01-02','20250102','2025-W01-4','2025-01-02T01:02:03.123456Z','2025-01-02T00:00 Z','2025-01-02T01:02:03+01:30','invalid',''):
        add('date-'+stamp,{'COMPLETED_AT (UTC)':stamp})
    add('sent-fallback',{'COMPLETED_AT (UTC)':'','SENT_AT (UTC)':'2025-01-03','CREATED_AT (UTC)':'invalid'})
    add('completed-priority',{'SENT_AT (UTC)':'2025-01-03','CREATED_AT (UTC)':'2025-01-04'})
    add('created-fallback',{'COMPLETED_AT (UTC)':'','CREATED_AT (UTC)':'2025-01-04'})
    add('order-number',{'ORDER_ID':''});add('order-txid',{'ORDER_ID':'','ORDER_NUMBER':''})
    add('quote',{'PAYIN_METHOD':'wire, "quoted"\r\nline'})
    add('extra',header=HEADER+['extra'],changes={'extra':'ignored'});cases['unheaded-extra']=cases['base'].rstrip()+',ignored\n'
    cases['short-row']=','.join(HEADER)+'\ncompleted\n';cases['field-limit']=cases['base'].replace('wire','x'*131073)
    add('repeat',rows=[BASE,BASE,{**BASE,'PAYIN_METHOD':'updated'}]);add('failed-batch',rows=[BASE,{**BASE,'COMPLETED_AT (UTC)':'invalid'}])
    return cases

def vectors():
    cases=[]
    with tempfile.TemporaryDirectory(prefix='kassiber-bull-vectors-') as root:
        path=Path(root)/'input.csv'
        for name,content in inputs().items():
            data=content.encode() if isinstance(content,str) else content;path.write_bytes(data)
            case={'name':'bull:'+name,'format':'bullbitcoin_csv',**({'text':content} if isinstance(content,str) else {'bytes':list(data)})}
            try:
                records=[]
                for row in load_import_records(str(path),'bullbitcoin_csv'):
                    r=normalize_import_record(row,'file:bullbitcoin_csv');fp=make_transaction_fingerprint('wallet',r['external_id'],r['occurred_at'],r['direction'],r['asset'],r['amount'],r['fee'])
                    r['amount']=btc_to_msat(r['amount']);r['fee']=btc_to_msat(r['fee'])
                    if any(not -(1<<63)<=r[k]<(1<<63) for k in ('amount','fee')): raise OverflowError('Python int too large to convert to SQLite INTEGER')
                    for k in ('exchange_evidence_match_by_economics','exchange_evidence_match_time_tolerance_seconds','match_existing_ignore_fee'): r.pop(k)
                    r['amount_includes_fee']=int(r['amount_includes_fee']);records.append({'fields':r,'fingerprint':fp})
                case['records']=clean(records)
            except Exception as exc:case['error']={'code':exc.code if isinstance(exc,AppError) else 'internal_error','message':str(exc),'hint':exc.hint if isinstance(exc,AppError) else None}
            cases.append(case)
    return cases
if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--check',action='store_true');args=parser.parse_args();text=json.dumps(vectors(),ensure_ascii=True,indent=2)+'\n'
    if args.check:
        if DEST.read_text()!=text:raise SystemExit('Bull import vectors drifted')
    else:DEST.write_text(text)
