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
DEST=Path(__file__).resolve().parents[1]/'tests/vectors/python_strike_imports.json'
HEADER=['Reference', 'Date & Time (UTC)', 'Transaction Type', 'Amount EUR', 'Fee EUR', 'Amount BTC', 'Fee BTC', 'BTC Price', 'Cost Basis (EUR)', 'Destination', 'Description', 'Transaction Hash', 'Note']
BASE={'Reference': 'row-1', 'Date & Time (UTC)': 'Jan 02 2025 01:02:03', 'Transaction Type': 'Buy', 'Amount EUR': '-100', 'Fee EUR': '1.20', 'Amount BTC': '0.002', 'Fee BTC': '0.00001', 'BTC Price': '', 'Cost Basis (EUR)': '', 'Destination': 'balance', 'Description': 'purchase', 'Transaction Hash': 'abababababababababababababababababababababababababababababababab', 'Note': 'note'}
NUMBERS=['Amount EUR', 'Fee EUR', 'Amount BTC', 'Fee BTC', 'BTC Price', 'Cost Basis (EUR)']
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
    from tests.test_cli_smoke import _STRIKE_CSV
    cases['smoke']=_STRIKE_CSV
    for kind in ('buy','purchase','sell','receive','received','send','sent','withdraw','withdrawal','unknown type',' BUY ',''):
        for amount in ('0.002','-0.002'):add('kind-'+kind+amount,{'Transaction Type':kind,'Amount BTC':amount})
    for key in NUMBERS:
        for dash in ('-','–','—',' - '):add('dash-'+key+dash,{key:dash})
    for stamp in ('January 2 2025 1:2:3','JAN 02 2025 01:02:03','Feb 29 2024 00:00:00','Feb 29 2025 00:00:00','Jan 02 25 01:02:03','Jan 02 0001 01:02:03','Jan 02 2025 +1:02:03','Jan 02 2025 0001:02:03','2025-01-02T01:02:03+01:00','2025-01-02','invalid',''):
        add('date-'+stamp,{'Date & Time (UTC)':stamp})
    for stamp in ('Aprıl 2 2025 1:2:3','Aprİl 2 2025 1:2:3','Auguﬆ 2 2025 1:2:3','Jan ２ ２０２５ １:２:３','Jan ０２ 2025 01:02:03','Jan 02 2_25 01:02:03','Jan 02 2025 01:+2:03','Jan 02 2025 01:002:03','Jan 02 2025 01:02:-3'):
        add('date-edge-'+stamp,{'Date & Time (UTC)':stamp})
    for month in ('Jan','January','Feb','February','Mar','March','Apr','April','May','Jun','June','Jul','July','Aug','August','Sep','September','Oct','October','Nov','November','Dec','December'):
        add('date-month-'+month,{'Date & Time (UTC)':month+' 02 2025 01:02:03'})
    add('quoted-note',{'Note':'note, "quoted"\r\nnext'})
    add('cost-currency-only',header=[k for k in HEADER if k not in ('Amount EUR','Fee EUR')],changes={'Cost Basis (EUR)':'101'})
    add('date-long-s',{'Date & Time (UTC)':'ſep 02 2025 01:02:03'})
    add('fiat-dual-precedence',header=HEADER+['Amount (EUR)','Fee (EUR)','Cost Basis EUR'],changes={'Amount (EUR)':'200','Fee (EUR)':'5','Cost Basis (EUR)':'300','Cost Basis EUR':'400'})
    add('cost-dual-precedence',header=HEADER+['Cost Basis EUR'],changes={'Amount EUR':'','Cost Basis (EUR)':'300','Cost Basis EUR':'400'})
    for stamp in ('Jan 2 ２０２５ １:２:３','Jan 1２ 2025 0１:0２:0３','Jan ０２ 2025 01:02:03'):
        add('date-digit-regex-'+stamp,{'Date & Time (UTC)':stamp})
    for dest in ('Lightning','lightning:abc','LNBC1invoice','lntb1invoice','lnbcrt1invoice','address',''):
        add('destination-'+dest,{'Destination':dest})
    for txhash in ('ab'*32, 'AB'*32, 'not-a-hash',''):
        add('lightning-'+txhash,{'Transaction Type':'Lightning Receive','Transaction Hash':txhash})
        add('onchain-'+txhash,{'Transaction Type':'Receive','Transaction Hash':txhash})
    for kind in ('ſell','ſend'):
        add('unicode-kind-'+kind,{'Transaction Type':kind,'Amount BTC':'-0.002'})
    add('lightning-spaced-hash',{'Destination':'lightning','Transaction Hash':'ab'*31+'  '})
    add('rate-only',{'Amount EUR':'','BTC Price':'50000'});add('cost-basis',{'Amount EUR':'','Cost Basis (EUR)':'101'})
    add('no-price',{'Amount EUR':'','Fee EUR':'','Cost Basis (EUR)':''})
    add('sell-floor',{'Amount BTC':'-0.002','Transaction Type':'Sell','Amount EUR':'1','Fee EUR':'2'})
    for currency in ('USD','EUR','CHF','GBP','BTC','LBTC','UTC'):
        mapping={k:k.replace('EUR',currency) for k in HEADER};add('fiat-'+currency,header=[mapping[k] for k in HEADER],rows=[{mapping[k]:v for k,v in BASE.items()}])
    mapping={k:k.replace('Amount EUR','Amount (EUR)').replace('Fee EUR','Fee (EUR)').replace('Cost Basis (EUR)','Cost Basis EUR') for k in HEADER}
    add('parentheses',header=list(mapping.values()),rows=[{mapping[k]:v for k,v in BASE.items()}])
    add('currency-order',header=HEADER+['Amount USD'],changes={'Amount USD':'200'})
    add('currency-order-reversed',header=['Amount USD']+HEADER,changes={'Amount USD':'200'})
    add('repeat',rows=[BASE,BASE,{**BASE,'Note':'updated'}]);add('failed-batch',rows=[BASE,{**BASE,'Date & Time (UTC)':'invalid'}])
    return cases

def vectors():
    cases=[]
    with tempfile.TemporaryDirectory(prefix='kassiber-strike-vectors-') as root:
        path=Path(root)/'input.csv'
        for name,content in inputs().items():
            data=content.encode() if isinstance(content,str) else content;path.write_bytes(data)
            case={'name':'strike:'+name,'format':'strike_csv',**({'text':content} if isinstance(content,str) else {'bytes':list(data)})}
            try:
                records=[]
                for row in load_import_records(str(path),'strike_csv'):
                    r=normalize_import_record(row,'file:strike_csv');fp=make_transaction_fingerprint('wallet',r['external_id'],r['occurred_at'],r['direction'],r['asset'],r['amount'],r['fee'])
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
