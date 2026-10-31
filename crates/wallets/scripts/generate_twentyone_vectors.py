#!/usr/bin/env python3
"""Freeze the unchanged Python provider parser and its normalization branches."""
import argparse
import csv
import io
import json
import tempfile
from pathlib import Path
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[3]))
from kassiber.core.imports import normalize_import_record
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.importers import load_import_records
from kassiber.msat import btc_to_msat
from generate_exchange_vectors import clean
DEST=Path(__file__).resolve().parents[1]/'tests/vectors/python_twentyone_imports.json'
HEADER=['id', 'transaction_date', 'buy_asset', 'buy_amount', 'sell_asset', 'sell_amount', 'fee_asset', 'fee_amount', 'transaction_type', 'exchange_name', 'depot_name', 'note', 'linked_transaction']
BASE={'id': 'row-1', 'transaction_date': '01.06.222 03:00:42', 'buy_asset': 'BTC', 'buy_amount': '0.002', 'sell_asset': 'EUR', 'sell_amount': '100.00', 'fee_asset': 'EUR', 'fee_amount': '1.20', 'transaction_type': 'trade', 'exchange_name': '21bitcoin', 'depot_name': 'main', 'note': 'purchase', 'linked_transaction': 'ignored'}
NUMBERS=['buy_amount', 'sell_amount', 'fee_amount']
def inputs():
    cases={'empty':'','header-only':','.join(HEADER)+'\n','blank-header':'\nx\n','utf8':b'\xff','utf16':','.join(HEADER).encode('utf-16')}
    def add(name,changes=None,*,header=None,rows=None,dialect=None):
        header=HEADER if header is None else header
        stream=io.StringIO(newline='');writer=csv.writer(stream,**(dialect or {}));writer.writerow(header)
        for row in rows if rows is not None else [{**BASE,**(changes or {})}]:writer.writerow([row.get(k,row.get(k.strip(),'')) for k in header])
        cases[name]=stream.getvalue()
    add('base');cases['bom']='\ufeff'+cases['base'];add('tab',dialect={'delimiter':'\t'});add('semicolon',dialect={'delimiter':';'})
    for key in HEADER:
        add('missing-'+key,header=[k for k in HEADER if k!=key])
        for blank in ('',' ','\t'):add('blank-'+key+repr(blank),{key:blank})
    for style,transform in [('lower',str.lower),('upper',str.upper),('spaces',lambda k:' \t'+k+'  '),('unicode',lambda k:k.replace(' ','\u00a0\t')),('long-s',lambda k:k.replace('s','ſ'))]:
        add('header-'+style,header=[transform(k) for k in HEADER],rows=[{transform(k):v for k,v in BASE.items()}])
    add('extra',header=HEADER+['extra'],changes={'extra':'ignored'});cases['unheaded-extra']=cases['base'].rstrip()+',ignored\n'
    cases['blank-unheaded-extra']=','.join(HEADER)+'\n'+','*len(HEADER)+'\n'
    cases['short-row']=','.join(HEADER)+'\nfirst\n'
    cases['field-limit']=cases['base'].rstrip()+','+'x'*131073+'\n'
    for key in NUMBERS:
        for v in ('0','-0','-1','1e-12','0.000000000005','0.0000000000049','1.23456789012345678901234567890123456','($1,234.50)',' 0.002 LBTC ','１２.３４','١_٢e-٣','NaN','sNaN','Infinity','-Infinity','bad','1e309'):
            add('number-'+key+v,{key:v})
    from tests.test_cli_smoke import _TWENTYONEBITCOIN_TRANSACTIONS_CSV
    cases['smoke']=_TWENTYONEBITCOIN_TRANSACTIONS_CSV
    add('sell',{'buy_asset':'EUR','buy_amount':'100','sell_asset':'BTC','sell_amount':'0.002'})
    add('counterparty-other',{'exchange_name':'Other broker'})
    add('sell-kind-overrides',{'buy_asset':'EUR','buy_amount':'100','sell_asset':'BTC','sell_amount':'0.002','transaction_type':'withdrawal'})
    add('deposit',{'sell_asset':'','sell_amount':'','fee_asset':'BTC','transaction_type':'deposit'})
    add('withdrawal',{'buy_asset':'','buy_amount':'','sell_asset':'BTC','sell_amount':'0.002','fee_asset':'BTC','fee_amount':'0.00001','transaction_type':'withdrawal'})
    add('blank-rows',rows=[{k:'' for k in HEADER},BASE,{k:' \t' for k in HEADER}])
    for stamp in ('01.01.25','01.01.99','01.01.-1','01.01.+1','01.01.2+1','01.01.0001','31.12.9999 23:59:59','01.01.222 01:02:03.987','01.01.2025 01:02','31.02.2025','2025-01-02T01:02:03+01:00','2025-01-02','invalid','01.01.2147483648','01.01.99999999999999999999999999999999'):
        add('date-'+stamp,{'transaction_date':stamp})
    for stamp in ('01.01.xx','xx.01.2025','01.xx.2025','01.01.2025 x:02:03','01.01.2025 01:02:x','01.01.-2147483649','999999999999999999999.01.2025','01.01.2025 01:02:2147483648','01.01.2025 01:02:-2147483649','01.01.2025 01:02:03:04'):
        add('date-edge-'+stamp,{'transaction_date':stamp})
    for key in ('buy_asset','sell_asset','fee_asset'):
        for asset in ('BTC','btc','XBT','XXBT','LBTC','L-BTC','USD','EUR','ETH',''):add('currency-'+key+asset,{key:asset})
    for kind in ('deposit','trade','withdrawal','income','new type',' TRADE ',''):
        add('in-kind-'+kind,{'sell_asset':'','sell_amount':'','transaction_type':kind})
        add('out-kind-'+kind,{'buy_asset':'','buy_amount':'','sell_asset':'BTC','sell_amount':'0.002','transaction_type':kind})
    add('fiat-only',{'buy_asset':'USD'});add('both-btc',{'sell_asset':'LBTC'})
    add('sell-no-value',{'buy_asset':'EUR','buy_amount':'','sell_asset':'BTC','sell_amount':'0.002'})
    add('buy-no-value',{'sell_asset':'EUR','sell_amount':''})
    add('quote',{'note':'purchase, "quoted"\r\nline'})
    for aliases in (['id','ID'],['ID','id'],[' id ','id'],['id',' id ']):add('collision-'+repr(aliases),header=HEADER+aliases,changes={aliases[0]:'first',aliases[1]:'last'})
    add('repeat',rows=[BASE,BASE,{**BASE,'note':'updated'}]);add('failed-batch',rows=[BASE,{**BASE,'transaction_date':'invalid'}])
    return cases

def vectors():
    cases=[]
    with tempfile.TemporaryDirectory(prefix='kassiber-twentyone-vectors-') as root:
        path=Path(root)/'input.csv'
        for name,content in inputs().items():
            data=content.encode() if isinstance(content,str) else content;path.write_bytes(data)
            case={'name':'twentyone:'+name,'format':'21bitcoin_csv',**({'text':content} if isinstance(content,str) else {'bytes':list(data)})}
            try:
                records=[]
                for row in load_import_records(str(path),'21bitcoin_csv'):
                    r=normalize_import_record(row,'file:21bitcoin_csv');fp=make_transaction_fingerprint('wallet',r['external_id'],r['occurred_at'],r['direction'],r['asset'],r['amount'],r['fee'])
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
        if DEST.read_text()!=text:raise SystemExit('Provider import vectors drifted')
    else:DEST.write_text(text)
