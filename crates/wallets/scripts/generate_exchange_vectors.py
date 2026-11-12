#!/usr/bin/env python3
"""Freeze reference exchange parser branches and normalized storage fields."""
import argparse
import csv
import io
import json
import math
import tempfile
from pathlib import Path

from kassiber.core.imports import normalize_import_record
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.importers import load_import_records
from kassiber.msat import btc_to_msat
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
from tests.test_cli_smoke import _RIVER_CSV

DEST = Path(__file__).resolve().parents[1] / 'tests/vectors/python_exchange_imports.json'
HEADER = ['Date','Sent Amount','Sent Currency','Received Amount','Received Currency','Fee Amount','Fee Currency','Bitcoin Price Amount','Bitcoin Price Currency','Transaction ID','Reference Code','Transaction Type','Tag','Method','Source','Destination','Payment Hash','payment_hash']
BASE = {'Date':'2025-01-02T01:02:03+01:00','Sent Amount':'100.00','Sent Currency':'EUR','Received Amount':'0.002','Received Currency':'BTC','Fee Amount':'1.00','Fee Currency':'EUR','Bitcoin Price Amount':'50000','Bitcoin Price Currency':'EUR','Reference Code':'river-buy','Transaction Type':'Buy','Tag':'Buy','Source':'bank','Destination':'balance','Method':'wire'}
CASES = {}
def add(name, changes=None, *, header=None, rows=None, dialect=None):
    columns = header or HEADER
    stream = io.StringIO(newline='')
    writer = csv.writer(stream, **(dialect or {}))
    writer.writerow(columns)
    for row in rows if rows is not None else [{**BASE, **(changes or {})}]:
        writer.writerow([row.get(key, row.get(key.strip(), '')) for key in columns])
    CASES['river:'+name] = stream.getvalue()

CASES['river:smoke'] = _RIVER_CSV
CASES['river:smoke-eur'] = _RIVER_CSV.replace('USD','EUR')
CASES['river:empty'] = ''
CASES['river:header-only'] = ','.join(HEADER)+'\n'
CASES['river:blank-header'] = '\nrow\n'
CASES['river:utf8'] = b'\xff'
CASES['river:utf16'] = ','.join(HEADER).encode('utf-16')
add('base')
add('unspaced-header-refusal', header=[k.replace(' ', '') for k in HEADER], rows=[{k.replace(' ', ''):v for k,v in BASE.items()}])
CASES['river:bom'] = '\ufeff'+CASES['river:base']
add('tab', dialect={'delimiter':'\t'})
add('semicolon', dialect={'delimiter':';'})
add('extra', header=HEADER+['extra'], changes={'extra':'ignored'})
CASES['river:unheaded-extra'] = CASES['river:base'].rstrip() + ',ignored\n'
CASES['river:short-row'] = ','.join(HEADER)+'\n2025-01-02,1,BTC\n'
for key in ('Date','Sent Amount','Sent Currency','Received Amount','Received Currency'):
    add('missing-'+key, header=[k for k in HEADER if k!=key])
for style in ('lower','upper','spaces','unicode','long-s','ligature'):
    def transform(key):
        if style=='lower': return key.lower()
        if style=='upper': return key.upper()
        if style=='spaces': return ' \t'+key+'  '
        if style=='unicode': return key.replace(' ', '\u00a0\t')
        if style=='long-s': return key.replace('S','ſ').replace('s','ſ')
        return key.replace('st','ﬆ').replace('St','ﬆ')
    add('header-'+style, header=[transform(k) for k in HEADER], rows=[{transform(k):v for k,v in BASE.items()}])
for extras in (['DATE','date'], ['date','DATE'], [' Date ','Date'], ['Date',' Date ']):
    add('collision-'+repr(extras), header=HEADER+extras, changes={extras[0]:'wrong', extras[1]:'2025-01-03'})
for key in ('Date','Sent Amount','Sent Currency','Received Amount','Received Currency','Fee Amount','Fee Currency','Bitcoin Price Amount','Bitcoin Price Currency','Transaction ID','Reference Code','Transaction Type','Tag','Method','Source','Destination','Payment Hash'):
    for value in ('',' ','  \t '): add('blank-'+key+'-'+repr(value), {key:value})
for field in ('Sent Amount','Received Amount','Fee Amount','Bitcoin Price Amount'):
    for value in ('0','-0','-1','1e-12','0.000000000005','0.0000000000049','1.23456789012345678901234567890123456','($1,234.50)','( -2 )',' 12 XBT ',' 12 LBTC ','$ EUR 1,234.50','１２.３４','١_٢e-٣','NaN','-NaN42','sNaN','Infinity','-Infinity','bad','1e309'):
        add('number-'+field+'-'+value, {field:value})
for currency in ('BTC','btc','XBT','XXBT','L-BTC','LBTC','EUR','USD','ETH',''):
    for key in ('Sent Currency','Received Currency','Fee Currency','Bitcoin Price Currency'):
        add('currency-'+key+'-'+currency,{key:currency})
for kind in ('Automatic Withdrawal','Cash Deposit','Cash Withdrawal','Mining Payout','Referral Reward','Bitcoin Interest on Cash','Unknown Type','', '  İNCOME  '):
    add('kind-'+kind, {'Sent Currency':'BTC','Sent Amount':'0.002','Received Currency':'','Transaction Type':kind, 'Tag':kind})
for fee in ('1','100','101','-101','NaN','Infinity'):
    add('sell-fee-'+fee, {'Sent Currency':'BTC','Sent Amount':'0.002','Received Currency':'EUR','Received Amount':'100','Fee Amount':fee})
for amount in ('0','NaN','Infinity'):
    add('zero-price-'+amount, {'Sent Currency':'BTC','Sent Amount':amount,'Received Currency':'','Bitcoin Price Amount':'0'})
for stamp in ('2025-01-02','20250102','2025-W01-4','2025-01-02T01:02:03.123456Z','2025-01-02T00:00 Z','2025-01-02T01:02:03+01:30','invalid',''):
    add('date-'+stamp, {'Date':stamp})
for h in ('ab'*32,'AB'*32,'ab'*31,'a '+'b'*62,'ab'*15+'\x0b\x0c\t '+'cd'*15,'bad',''):
    add('hash-'+h, {'Payment Hash':h})
add('quoting', {'Method':'comma, "quote"\r\nline','Tag':'Reward Type'})
CASES['river:field-limit'] = CASES['river:base'].replace('wire','x'*131073)
add('repeated',rows=[BASE,BASE,{**BASE,'Tag':'Changed','Method':'updated'}])
add('failed-batch',rows=[BASE,{**BASE,'Date':'invalid'}])

add('inbound-unpriced', {'Sent Currency':'','Bitcoin Price Amount':''})
add('default-description', {'Sent Currency':'','Bitcoin Price Amount':'','Transaction Type':'','Tag':'','Method':'','Source':'','Destination':''})
add('nonfinite-zero-price', {'Sent Currency':'','Received Amount':'0','Bitcoin Price Amount':'NaN'})
add('double-infinity', {'Sent Currency':'','Received Amount':'Infinity','Bitcoin Price Amount':'Infinity'})
add('contextual-tag', {'Transaction Type':'','Tag':'Σ'})

def clean(value):
    if isinstance(value,float) and math.isnan(value): return None
    if isinstance(value,float) and not math.isfinite(value): return {'$float':str(value)}
    if isinstance(value,dict): return {k:clean(v) for k,v in value.items()}
    if isinstance(value,list): return [clean(v) for v in value]
    return value

def vectors():
    cases=[]
    with tempfile.TemporaryDirectory(prefix='kassiber-exchange-vectors-') as root:
        path=Path(root)/'input.csv'
        for name,content in CASES.items():
            data=content.encode() if isinstance(content,str) else content
            path.write_bytes(data)
            fmt='river_csv'
            case={'name':name,'format':fmt,**({'text':content} if isinstance(content,str) else {'bytes':list(data)})}
            try:
                records=[]
                for row in load_import_records(str(path),fmt):
                    r=normalize_import_record(row,'file:'+fmt)
                    fp=make_transaction_fingerprint('wallet',r['external_id'],r['occurred_at'],r['direction'],r['asset'],r['amount'],r['fee'])
                    r['amount']=btc_to_msat(r['amount']);r['fee']=btc_to_msat(r['fee'])
                    if any(not -(1<<63)<=r[k]<(1<<63) for k in ('amount','fee')): raise OverflowError('Python int too large to convert to SQLite INTEGER')
                    for k in ('exchange_evidence_match_by_economics','exchange_evidence_match_time_tolerance_seconds','match_existing_ignore_fee'):r.pop(k)
                    r['amount_includes_fee']=int(r['amount_includes_fee'])
                    records.append({'fields':r,'fingerprint':fp})
                case['records']=clean(records)
            except Exception as exc:
                case['error']={'code':exc.code if isinstance(exc,AppError) else 'internal_error','message':str(exc),'hint':exc.hint if isinstance(exc,AppError) else None}
            cases.append(case)
    return cases

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--check',action='store_true');args=parser.parse_args()
    text=json.dumps(vectors(),ensure_ascii=True,indent=2)+'\n'
    if args.check:
        if DEST.read_text()!=text:raise SystemExit('Exchange import vectors drifted')
    else:DEST.write_text(text)
