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
DEST=Path(__file__).resolve().parents[1]/'tests/vectors/python_pocket_imports.json'
HEADER=['type', 'date', 'reference', 'price.currency', 'price.amount', 'cost.currency', 'cost.amount', 'fee.currency', 'fee.amount', 'value.currency', 'value.amount']
BASE={'type': 'exchange', 'date': '2025-01-02', 'reference': 'order', 'price.currency': 'EUR', 'price.amount': '50000', 'cost.currency': 'EUR', 'cost.amount': '100', 'fee.currency': 'EUR', 'fee.amount': '1.20', 'value.currency': 'BTC', 'value.amount': '0.002'}
NUMBERS=['price.amount', 'cost.amount', 'fee.amount', 'value.amount']
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
    cases['short-row']=','.join(HEADER)+'\nfirst\n'
    cases['field-limit']=cases['base'].rstrip()+','+'x'*131073+'\n'
    for key in NUMBERS:
        for v in ('0','-0','-1','1e-12','0.000000000005','0.0000000000049','1.23456789012345678901234567890123456','($1,234.50)',' 0.002 LBTC ','１２.３４','١_٢e-٣','NaN','sNaN','Infinity','-Infinity','bad','1e309'):
            add('number-'+key+v,{key:v})
    from tests.test_cli_smoke import _POCKETBITCOIN_CSV
    cases['smoke']=_POCKETBITCOIN_CSV
    withdrawal={**BASE,'type':'withdrawal','date':'2025-01-03','reference':'withdraw','fee.currency':'BTC','fee.amount':'0.00001','value.amount':'0.00199'}
    add('paired',rows=[BASE,withdrawal]);add('paired-reversed',rows=[withdrawal,BASE])
    for key in ('date','fee.currency','fee.amount','value.currency','value.amount'):
        for v in ('','2025-01-01','2025-01-04','BTC','XBT','XXBT','LBTC','L-BTC','USD','0','0.00002','0.00199','NaN','Infinity','bad'):
            add('withdrawal-'+key+v,rows=[BASE,{**withdrawal,key:v}])
    add('pair-sort',rows=[{**withdrawal,'date':'2025-01-04','reference':'later'},BASE,withdrawal])
    add('pair-before-only',rows=[BASE,{**withdrawal,'date':'2025-01-01'}])
    add('pair-before-two',rows=[{**BASE,'date':'2025-01-05'},{**withdrawal,'date':'2025-01-04'},withdrawal])
    for aliases in (['date','Date',' date '],['value.amount','Value.Amount',' value.amount '],['reference','Reference',' reference ']):
        add('collision-'+repr(aliases),header=HEADER+aliases,rows=[{**BASE,aliases[0]:BASE.get(aliases[0],''),aliases[1]:BASE.get(aliases[0],''),aliases[2]:BASE.get(aliases[0],'')},withdrawal])
    add('pair-header-collision-order', header=HEADER+['Date',' date '], rows=[{**BASE,'Date':'2025-01-02',' date ':'2025-01-05'}, {**withdrawal,'Date':'2025-01-03',' date ':'2025-01-03'}, {**withdrawal,'reference':'later','Date':'2025-01-06',' date ':'2025-01-06'}])
    add('quoted-reference', {'reference':'order, "quoted"\r\nnext'})
    add('referenceless-repeat', rows=[{**BASE,'reference':''},{**BASE,'reference':''},{**BASE,'reference':'','date':'2025-02-01'}])
    add('pair-consumed',rows=[BASE,{**BASE,'reference':'second'},withdrawal])
    add('pair-two',rows=[BASE,{**BASE,'reference':'second'},withdrawal,{**withdrawal,'reference':'other','date':'2025-01-04'}])
    for stamp in ('2025-01-02T01:02:03+01:00','20250102','invalid',''):
        add('date-'+stamp,{'date':stamp})
    for key in ('cost.currency','value.currency','fee.currency'):
        for asset in ('BTC','XBT','XXBT','LBTC','L-BTC','USD','EUR','ETH',''):add('currency-'+key+asset,{key:asset})
    add('sell',{'cost.currency':'BTC','cost.amount':'0.002','value.currency':'EUR','value.amount':'100'})
    add('sell-floor',{'cost.currency':'BTC','cost.amount':'0.002','value.currency':'EUR','value.amount':'1'})
    add('no-rate',{'price.amount':''});add('fallback',{'reference':''});add('zero',{'value.amount':'0','price.amount':''})
    for tag in ('deposit','withdrawal','EXCHANGE',' exchange ','unknown',''):add('type-'+tag,{'type':tag})
    add('repeat',rows=[BASE,BASE,{**BASE,'price.amount':'50001'}]);add('failed-batch',rows=[BASE,{**BASE,'date':'invalid'}])
    return cases

def vectors():
    cases=[]
    with tempfile.TemporaryDirectory(prefix='kassiber-pocket-vectors-') as root:
        path=Path(root)/'input.csv'
        for name,content in inputs().items():
            data=content.encode() if isinstance(content,str) else content;path.write_bytes(data)
            case={'name':'pocket:'+name,'format':'pocketbitcoin_csv',**({'text':content} if isinstance(content,str) else {'bytes':list(data)})}
            try:
                records=[]
                for row in load_import_records(str(path),'pocketbitcoin_csv'):
                    r=normalize_import_record(row,'file:pocketbitcoin_csv');fp=make_transaction_fingerprint('wallet',r['external_id'],r['occurred_at'],r['direction'],r['asset'],r['amount'],r['fee'])
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
