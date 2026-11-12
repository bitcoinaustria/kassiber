import argparse
import csv
import io
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
from kassiber.core.imports import normalize_import_record
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.importers import load_import_records, _COINTRACKING_TYPES
from kassiber.msat import btc_to_msat
from generate_exchange_vectors import clean

DEST = Path(__file__).resolve().parents[1] / 'tests/vectors/python_cointracking_imports.json'
BASE = {'Type':'Trade','Buy Amount':'0.00200000','Buy Currency':'BTC','Sell Amount':'100.00','Sell Currency':'EUR','Fee':'1.00','Fee Currency':'EUR','Exchange':'Kraken','Trade-Group':'savings','Comment':'buy','Date':'01.02.2025 01:02:03','Tx-ID (optional)':'ct-buy'}

def inputs():
    cases = {'empty':'','blank':'\n \n','utf8':b'\xff','utf16':','.join(BASE).encode('utf-16')}
    def add(name, changes=None, *, header=None, rows=None, **dialect):
        columns = list(BASE) if header is None else header
        out = io.StringIO(newline='')
        writer = csv.writer(out, **dialect)
        writer.writerow(columns)
        for row in rows if rows is not None else [{**BASE, **(changes or {})}]:
            writer.writerow([row.get(k, '') for k in columns])
        cases[name] = out.getvalue()
    add('base')
    add('header-only', rows=[])
    cases['bom'] = '\ufeff'+cases['base']
    cases['blank-prefix'] = '\n\n'+cases['base']
    cases['double-bom'] = '\ufeff'+cases['bom']
    cases['bad-bom'] = b'\xef\xbb\xbf\xff'
    for delimiter in (';', '\t', '|'):
        add('delimiter-'+repr(delimiter), delimiter=delimiter)
    for quote in ("'", '"'):
        add('quote-'+quote, {'Comment':f'one, "two"\r\nnext {quote}word{quote}'}, quotechar=quote)
    cases['extra-unheaded'] = cases['base'].rstrip()+',ignored\n'
    cases['short-row'] = ','.join(BASE)+'\nDeposit,1,BTC\n'
    cases['field-limit'] = cases['base'].replace('savings','x'*131073)
    for key in BASE:
        add('missing-'+key, header=[k for k in BASE if k != key])
        for blank in ('',' ','\t'):
            add('blank-'+key+repr(blank), {key:blank})
    for style, transform in (('lower', str.lower), ('upper', str.upper), ('spaces',lambda k:' '+k+' '), ('unicode',lambda k:k.replace(' ','\u00a0\t')), ('casefold',lambda k:k.replace('s','ſ').replace('fi','ﬁ'))):
        add('headers-'+style, header=[transform(k) for k in BASE],rows=[{transform(k):v for k,v in BASE.items()}])
    for extra in (['DATE','date'],['date','DATE'],[' Date ','Date'],['Date',' Date ']):
        add('collision-'+repr(extra), header=list(BASE)+extra, changes={extra[0]:'bad',extra[1]:'2025-01-03'})
    for field in ('Buy Amount','Sell Amount','Fee'):
        for value in ('0','-0','-1','1e-12','0.000000000005','0.000000000015','1.23456789012345678901234567890123456','($1,234.50)','1.234,56','0,05','1,234,567','1.234.567',' 12 LBTC ','１２.３４','١_٢e-٣','NaN','sNaN','Infinity','-Infinity','bad','1e309'):
            add('number-'+field+value,{field:value})
    for key in ('Buy Currency','Sell Currency','Fee Currency'):
        for value in ('BTC','btc',' XBT ','XXBT','LBTC','L-BTC','EUR','USD','ETH',''):
            add('asset-'+key+value,{key:value})
    for kind in [*_COINTRACKING_TYPES,'Trade','Transfer','Receive Loan',' ', ' STAKING ', 'ſtolen']:
        add('type-in-'+kind,{'Type':kind,'Sell Currency':'','Sell Amount':'','Fee':'','Fee Currency':''})
        add('type-out-'+kind,{'Type':kind,'Buy Currency':'','Buy Amount':'','Sell Currency':'BTC','Sell Amount':'0.002','Fee':'','Fee Currency':''})
    for value in ('2025-01-02','20250102','2025-W01-4','2025-01-02T01:02:03.123456Z','2025-01-02T00:00 Z','1.2.2025','１.２.２０２５','01.02.222','invalid',''):
        add('date-'+value,{'Date':value})
    add('no-id',{'Tx-ID (optional)':''})
    add('repeat', rows=[BASE,BASE])
    add('idless-repeat',rows=[{**BASE,'Tx-ID (optional)':''}]*2)
    add('idless-comments',rows=[{**BASE,'Tx-ID (optional)':''},{**BASE,'Tx-ID (optional)':'','Comment':'changed'}])
    add('standalone',{'Type':'Other Fee','Buy Amount':'','Buy Currency':'','Sell Amount':'','Sell Currency':'','Fee':'0.00001','Fee Currency':'BTC'})
    add('standalone-no-type',{'Type':'','Buy Amount':'','Buy Currency':'','Sell Amount':'','Sell Currency':'','Fee':'0.00001','Fee Currency':'BTC'})
    add('skip-invalid',{'Type':'bad','Buy Currency':'ETH','Sell Currency':'USD','Buy Amount':'bad','Date':'bad','Fee':'0'})
    add('skip-fee-invalid',{'Type':'bad','Buy Currency':'ETH','Sell Currency':'USD','Buy Amount':'bad','Date':'bad','Fee':'bad'})
    add('sell-net',{'Buy Amount':'100','Buy Currency':'EUR','Sell Amount':'0.002','Sell Currency':'BTC','Fee':'101','Fee Currency':'EUR'})
    add('failed-batch',rows=[BASE,{**BASE,'Date':'invalid'}])
    import random
    randomizer = random.Random(2102)
    for index in range(40):
        rows = [{**BASE, 'Comment':randomizer.choice(['plain', 'two, pieces', 'double "quote"', "single 'quote'", 'line\nnext', '\t whitespace ']), 'Tx-ID (optional)':randomizer.choice(['', 'source-id']), 'Buy Amount':randomizer.choice(['0.002', '0,002', '0.000000000015'])} for _ in range(randomizer.randrange(1, 5))]
        add(f'seeded-dialect-{index}', rows=rows, delimiter=randomizer.choice([',',';','\t','|']), quotechar=randomizer.choice(['"', "'"]), lineterminator=randomizer.choice(['\n','\r\n','\r']))
    from tests.test_tax_platform_migration import COINTRACKING_CSV
    cases['smoke'] = COINTRACKING_CSV
    return cases

def vectors():
    result=[]
    with tempfile.TemporaryDirectory(prefix='cointracking-vectors-') as root:
        file=Path(root)/'input.csv'
        for name, content in inputs().items():
            file.write_bytes(content.encode() if isinstance(content,str) else content)
            case={'name':'cointracking:'+name,'format':'cointracking_csv',**({'text':content} if isinstance(content,str) else {'bytes':list(content)})}
            try:
                records=[]
                for row in load_import_records(str(file),'cointracking_csv'):
                    r=normalize_import_record(row,'file:cointracking_csv')
                    fp=make_transaction_fingerprint('wallet',r['external_id'] or row['_tax_platform_row_identity'],r['occurred_at'],r['direction'],r['asset'],r['amount'],r['fee'])
                    r['amount']=btc_to_msat(r['amount']);r['fee']=btc_to_msat(r['fee'])
                    if any(not -(1<<63)<=r[k]<(1<<63) for k in ('amount','fee')): raise OverflowError('Python int too large to convert to SQLite INTEGER')
                    for k in ('exchange_evidence_match_by_economics','exchange_evidence_match_time_tolerance_seconds','match_existing_ignore_fee'):r.pop(k)
                    r['amount_includes_fee']=int(r['amount_includes_fee'])
                    records.append({'fields':r,'fingerprint':fp})
                case['records']=clean(records)
            except Exception as exc:
                case['error']={'code':exc.code if isinstance(exc,AppError) else 'internal_error','message':str(exc),'hint':exc.hint if isinstance(exc,AppError) else None}
            result.append(case)
    return result

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--check',action='store_true');args=parser.parse_args()
    value=json.dumps(vectors(),ensure_ascii=True,indent=2)+'\n'
    if args.check:
        if DEST.read_text()!=value:raise SystemExit('CoinTracking vectors drifted')
    else:DEST.write_text(value)
