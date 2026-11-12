import argparse
import csv
from datetime import date, datetime, timedelta, time
import io
import json
import re
from pathlib import Path
import sys
import tempfile
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

from kassiber import importers as oracle
from kassiber.core.imports import normalize_import_record
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.msat import btc_to_msat
from crates.wallets.scripts.generate_import_vectors import clean

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / 'tests/vectors/python_ledger_imports.json'
TABLES = ROOT / 'src/file_import/ledger_tables.json'
BASE = {'Type': 'Buy', 'Date': '2025-01-02', 'Received Amount': '0.002', 'Received Asset': 'BTC', 'Sent Amount': '190', 'Sent Asset': 'EUR', 'Fee Amount': '2', 'Fee Asset': 'EUR', 'Note': ' bought ', 'Tx-ID': 'ledger-id'}


def tables():
    aliases = {}
    for field in oracle.LEDGER_COLUMN_PLAN_FIELDS:
        key = '_BYO_' + field.upper()
        aliases[field] = sorted(getattr(oracle, key))
    return {
        'aliases': aliases,
        'types': oracle._GENERIC_LEDGER_TYPES,
        'display': oracle.GENERIC_LEDGER_DISPLAY_TYPES,
        'columns': oracle.LEDGER_COLUMN_PLAN_FIELDS,
        'asset_fields': oracle.LEDGER_ASSET_HINT_FIELDS,
        'plan_fields': sorted(oracle.LEDGER_PLAN_FIELDS),
        'hint_codes': sorted(oracle.LEDGER_ASSET_HINT_CODES),
        'header_assets': oracle._BYO_HEADER_ASSET_ALIASES,
        'received_tokens': sorted(oracle._BYO_RECEIVED_HEADER_TOKENS),
        'sent_tokens': sorted(oracle._BYO_SENT_HEADER_TOKENS),
        'type_values': oracle._BYO_TYPE_VALUE_MAP,
        'inbound': sorted(oracle._BYO_INBOUND_VALUES),
        'outbound': sorted(oracle._BYO_OUTBOUND_VALUES),
    }


def xlsx(sheets, *, active=0, epoch1904=False, stored=False, rewrite=None, chart_first=False):
    from openpyxl import Workbook
    from openpyxl.utils.datetime import CALENDAR_MAC_1904
    book = Workbook()
    book.remove(book.active)
    if epoch1904:
        book.epoch = CALENDAR_MAC_1904
    for name, rows in sheets:
        sheet = book.create_sheet(name)
        for row in rows:
            sheet.append(row)
    if chart_first:
        from openpyxl.chart import BarChart, Reference
        chart = BarChart()
        chart.add_data(Reference(book.worksheets[0], min_col=2, min_row=1, max_row=2), titles_from_data=True)
        chart_sheet = book.create_chartsheet('Chart')
        chart_sheet.add_chart(chart)
        book._sheets.insert(0, book._sheets.pop())
    book.active = active
    book.properties.created = book.properties.modified = datetime(2025, 1, 1)
    output = io.BytesIO()
    book.save(output)
    frozen = io.BytesIO()
    with zipfile.ZipFile(output) as source, zipfile.ZipFile(frozen, 'w', compression=zipfile.ZIP_STORED if stored else zipfile.ZIP_DEFLATED) as dest:
        entries = {name: source.read(name) for name in source.namelist()}
        entries['docProps/core.xml'] = re.sub(rb'(<dcterms:modified[^>]*>)[^<]+',rb'\g<1>2025-01-01T00:00:00Z',entries['docProps/core.xml'])
        if rewrite:
            rewrite(entries)
        for name, content in sorted(entries.items()):
            info = zipfile.ZipInfo(name, (2025, 1, 1, 0, 0, 0))
            info.compress_type = dest.compression
            dest.writestr(info, content)
    return frozen.getvalue()


def inputs():
    cases = []
    def add(name, rows=None, *, text=None, extension='.csv', column_map=None, limit=200, **changes):
        if text is None:
            rows = rows if rows is not None else [{**BASE, **changes}]
            output = io.StringIO(newline='')
            writer = csv.DictWriter(output, list(rows[0]) if rows else list(BASE), lineterminator='\n')
            writer.writeheader()
            writer.writerows(rows)
            text = output.getvalue()
        cases.append({'name':'ledger:' + name, 'extension':extension, 'column_map':column_map, 'limit':limit, **({'bytes':list(text)} if isinstance(text, bytes) else {'text':text})})
    add('base')
    add('empty', text='')
    add('header-only', rows=[])
    add('unrecognized', text='mystery,other\na,b\n')
    add('preamble', text='Person,Example\n' + ','.join(BASE) + '\n' + ','.join(BASE.values()) + '\n')
    add('duplicate', rows=[BASE, BASE])
    add('bad-later', rows=[BASE, {**BASE, 'Date':'invalid'}])
    for kind, (direction, _) in oracle._GENERIC_LEDGER_TYPES.items():
        add('type-' + kind, rows=[{**BASE, 'Type':kind, **({'Received Amount':'190', 'Received Asset':'USD', 'Sent Amount':'0.002', 'Sent Asset':'BTC', 'Fee Asset':'USD'} if direction == 'outbound' else {})}])
    for field in BASE:
        add('missing-' + field, rows=[{key:value for key,value in BASE.items() if key != field}])
    for field in ['Type', 'Date', 'Received Amount', 'Sent Amount', 'Fee Amount', 'Received Asset', 'Sent Asset', 'Fee Asset']:
        for value in ['', ' ', 'invalid']:
            add('invalid-' + field + repr(value), **{field:value})
    for value in ['(0,002)', '1.234,56', '1,234.56', '1,234,567', '1.234.567', '1\u00a0234,5', '$0.002', '１２.３４', '١_٢e-٣', '-0', '0', '0.000000000005', '1e-40', '1e20', 'NaN', 'sNaN', '-Infinity']:
        add('number-' + value, **{'Received Amount':value})
    for value in ['2.1.2025', '02.01.2025 12:34:56', '2025-01-02T03:04:05+02:00', '2025-02-30', '١.٢.٢٠٢٥', '0', '20250102']:
        add('date-' + value, Date=value)
    for asset in ['SAT', 'SATS', 'XBT', 'l-btc', 'LIQUID-BTC', 'ETH']:
        add('asset-' + asset, **{'Received Asset':asset, 'Sent Amount':'', 'Sent Asset':'', 'Fee Amount':'2', 'Fee Asset':''})
    add('both-bitcoin', **{'Sent Asset':'LBTC'})
    add('cross-asset', **{'Sent Asset':'USDT'})
    add('wrong-direction', Type='Sell')
    add('fiat-fee-no-cash', **{'Sent Amount':'', 'Sent Asset':''})
    add('fiat-fee-conflict', **{'Fee Asset':'USD'})
    add('sell-fee-exceeds-value', rows=[{**BASE, 'Type':'Sell', 'Received Amount':'1', 'Received Asset':'EUR', 'Sent Amount':'0.002', 'Sent Asset':'BTC', 'Fee Amount':'2'}])
    add('explicit-value', **{'Sent Amount':'', 'Sent Asset':'', 'Fee Asset':'BTC', 'Fee Amount':'0.00001', 'Fiat Value':'123,45'})
    add('linkage', **{'Payment Hash':'AB'*32, 'Swap Refund Funding Tx-ID':'CD'*32, 'Swap Refund Funding Vout':'2'})
    add('bad-linkage', **{'Payment Hash':'bad', 'Swap Refund Funding Vout':'bad'})
    add('no-id', **{'Tx-ID':''})
    add('lbtc-suffix-reference-bug',**{'Received Amount':'1 LBTC','Received Asset':'LBTC'})
    # 8f (11b553bca): the hyphenated Liquid suffix, and a long s matching s
    # case-insensitively as Python's re.IGNORECASE does.
    add('l-btc-suffix',**{'Received Amount':'1 L-BTC','Received Asset':'LBTC'})
    add('long-s-sats-suffix',**{'Received Amount':'1000 \u017fats','Received Asset':'BTC'})
    add('python-control-whitespace',Type='gift\x1creceived')
    add('aliases', rows=[{'Transaction Type':'Buy', 'Timestamp':'2025-01-02', 'Buy Amount':'0.002', 'Buy Cur.':'BTC', 'Sell Amount':'190', 'Sell Cur.':'EUR', 'Fee':'2', 'Fee Cur.':'EUR', 'Description':'note', 'Transaction ID':'alias'}])
    for delimiter in [',', ';', '\t', '|']:
        output = io.StringIO(newline='')
        writer = csv.writer(output, delimiter=delimiter, lineterminator='\n')
        writer.writerows([list(BASE), list(BASE.values())])
        add('delimiter-' + repr(delimiter), text=output.getvalue(), extension='.tsv' if delimiter == '\t' else '.csv')
    for newline in ['\r', '\r\n']:
        add('newline-' + repr(newline), text=cases[0]['text'].replace('\n', newline))
    add('bom', text='\ufeff' + cases[0]['text'])
    add('bad-utf8', text=b'\xff')
    for name, header, row in [
        ('signed', ['Date','Amount'], ['2025-01-02','-0.002']),
        ('german', ['Datum','Typ','Menge','Währung','Wert','Gebühr','Fee Asset'], ['2.1.2025','Kauf','0,002','EUR','190','2','EUR']),
        ('sats', ['Date','Sats Amount','Price'], ['2025-01-02','100000','0.2']),
        ('cash-legs', ['Time','Bought BTC','Sold EUR'], ['2025-01-02','0.002','190']),
        ('liquid', ['Date','Received Liquid Bitcoin','Sent USD'], ['2025-01-02','0.002','190']),
        ('both', ['Date','Received BTC','Sent LBTC'], ['2025-01-02','0.002','0.003']),
        ('type-single', ['Date','Type','Amount'], ['2025-01-02','Sell','0.002']),
        ('unreadable-type', ['Date','Typ','Amount'], ['2025-01-02','Überweisung','0.002']),
        ('junk-number', ['Date','Amount'], ['2025-01-02','junk']),
    ]:
        add('byo-' + name, text=','.join(header) + '\n' + ','.join('"' + x + '"' for x in row) + '\n')
    for direction in ['in','out','Income','Mining','', 'inbound', 'Ausgang']:
        add('byo-direction-' + direction, rows=[{'Date':'2025-01-02','Direction':direction,'Amount':'0.002'}])
    for field, aliases in tables()['aliases'].items():
        for alias in aliases:
            row = {'date':'2025-01-02', 'amount':'0.002'}
            if field in ['date','amount']:
                row = {alias: '2025-01-02' if field == 'date' else '0.002', ('amount' if field == 'date' else 'date'):'0.002' if field == 'date' else '2025-01-02'}
            else:
                row[alias] = {'type':'Deposit','direction':'in','received':'0.002','sent':'0.002','asset':'BTC','fee':'0.00001','fiat_rate':'123','fiat_value':'123','fiat_currency':'EUR'}.get(field,'')
            add('alias-' + field + '-' + alias, rows=[row])
    text = 'When,Units,Side,Memo\n2025-01-02,0.002,ACQUIRE,note\n'
    mapping = {'date':'When','amount':'Units','type':'Side','note':'Memo','type_map':{'ACQUIRE':'Buy'}}
    add('mapping', text=text, column_map=mapping)
    for names in [('acquire','ACQUIRE'),('ACQUIRE','acquire')]:
        add('mapping-type-order-' + names[0], text=text, column_map={**mapping,'type_map':{names[0]:'Buy',names[1]:'Deposit'}})
    for name, mapping in [
        ('non-object', []), ('unknown', {'typo':'Units'}), ('missing-column', {'date':'Absent','amount':'Units'}),
        ('duplicate', {'date':'When','amount':'When'}), ('unusable', {'date':'When'}),
        ('bad-hint', {'date':'When','amount':'Units','amount_header_asset':'ETH'}),
        ('null-hint', {'date':'When','amount':'Units','amount_header_asset':None}),
        ('bad-type-map', {'date':'When','amount':'Units','type_map':[]}),
        ('bad-type-target', {'date':'When','amount':'Units','type_map':{'x':'Made up'}}),
        ('blank-type-target', {'date':'When','amount':'Units','type_map':{'x':''}}),
        ('typed-type-target', {'date':'When','amount':'Units','type_map':{'x':True}}),
    ]:
        add('mapping-' + name, text=text, column_map=mapping)
    for hint in [None,'SATS','LBTC','BTC']:
        add('mapping-contradiction-' + repr(hint), text='Date,BTC Amount\n2025-01-02,0.002\n', column_map={'date':'Date','amount':'BTC Amount','amount_header_asset':hint})
    for limit in [0,1,2,-1]:
        add('preview-limit-' + str(limit), rows=[BASE,{**BASE,'Type':'bad'}, {**BASE,'Tx-ID':'second'}, {**BASE,'Date':'bad'}], limit=limit)
    native = [list(BASE), list(BASE.values())]
    for name, sheets, options in [
        ('native', [('Transactions',native)], {}),
        ('stored', [('Transactions',native)], {'stored':True}),
        ('multi-sheet', [('Legend',[['about']]), ('Transactions',native)], {}),
        ('active-fallback', [('Legend',[['about']]), ('Arbitrary',[['Date','Amount'], ['2025-01-02',0.002]])], {'active':1}),
        ('chartsheet-active', [('Arbitrary',[['Date','Amount'], ['2025-01-02',0.002]])], {'chart_first':True}),
        ('chartsheet-index', [('Arbitrary',[['Date','Amount'], ['2025-01-02',0.002]])], {'chart_first':True,'active':1}),
        ('preamble', [('Transactions',[['Person','Example'],*native])], {}),
        ('dates', [('Transactions',[['Type','Date','Received Amount','Received Asset'],['Deposit',datetime(2025,1,2,3,4,5),0.002,'BTC'], ['Deposit',date(2025,1,3),0.003,'BTC']])], {}),
        ('epoch1904', [('Transactions',[['Type','Date','Received Amount','Received Asset'],['Deposit',datetime(2025,1,2),0.002,'BTC']])], {'epoch1904':True}),
        ('typed', [('Transactions',[['Type','Date','Received Amount','Received Asset','Note','Tx-ID'],['Deposit','2025-01-02',0.002,'BTC',True,123]])], {}),
        ('formula', [('Transactions',[['Type','Date','Received Amount','Received Asset'],['Deposit','2025-01-02','=1/2','BTC']])], {}),
        ('time', [('Transactions',[['Type','Date','Received Amount','Received Asset'],['Deposit',time(12,34),0.002,'BTC']])], {}),
        ('duration', [('Transactions',[['Type','Date','Received Amount','Received Asset'],['Deposit',timedelta(days=2),0.002,'BTC']])], {}),
        ('empty', [('Transactions',[])], {}),
    ]:
        add('xlsx-' + name, text=xlsx(sheets, **options), extension='.xlsx')
    for name, mutate in [
        ('shared-strings', lambda entries: entries.update({'xl/sharedStrings.xml':b'<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>Deposit</t></si></sst>', 'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'].replace(b'<c r="A2" t="inlineStr"><is><t>Buy</t></is></c>', b'<c r="A2" t="s"><v>0</v></c>')})),
        ('missing-content-types', lambda entries: entries.pop('[Content_Types].xml')),
        ('missing-workbook', lambda entries: entries.pop('xl/workbook.xml')),
        ('bad-cell-number', lambda entries: entries.update({'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'].replace(b'<c r="C2" t="inlineStr"><is><t>0.002</t></is></c>', b'<c r="C2" t="n"><v>junk</v></c>')})),
        ('bad-cell-boolean', lambda entries: entries.update({'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'].replace(b'<c r="C2" t="inlineStr"><is><t>0.002</t></is></c>', b'<c r="C2" t="b"><v>junk</v></c>')})),
        ('bad-cell-coordinate', lambda entries: entries.update({'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'].replace(b'r="C2"', b'r="JUNK"')})),
        ('bad-cell-style', lambda entries: entries.update({'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'].replace(b'r="C2"', b'r="C2" s="junk"')})),
        ('sheet-truncated', lambda entries: entries.update({'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'][:-10]})),
        ('sheet-mismatched-tag', lambda entries: entries.update({'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'].replace(b'</row>', b'</wrong>', 1)})),
        ('sheet-undefined-entity', lambda entries: entries.update({'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'].replace(b'Buy', b'&evil;')})),
        ('workbook-truncated', lambda entries: entries.update({'xl/workbook.xml':entries['xl/workbook.xml'][:-10]})),
        ('manifest-truncated', lambda entries: entries.update({'[Content_Types].xml':entries['[Content_Types].xml'][:-10]})),
        ('sheet-invalid-name', lambda entries: entries.update({'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'].replace(b'<worksheet ', b'<1worksheet ', 1)})),
        ('serial-rounds-to-next-day', lambda entries: entries.update({'xl/styles.xml':entries['xl/styles.xml'].replace(b'numFmtId="0"', b'numFmtId="14"'), 'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'].replace(b'<c r="B2" t="inlineStr"><is><t>2025-01-02</t></is></c>', b'<c r="B2" t="n"><v>0.999999999999</v></c>')})),
    ]:
        add('xlsx-' + name, text=xlsx([('Transactions',native)], rewrite=mutate), extension='.xlsx')
    def shared(entries):
        entries['[Content_Types].xml']=entries['[Content_Types].xml'].replace(b'</Types>',b'<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>')
        entries['xl/sharedStrings.xml']=b'<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><r><t>Bu</t></r><r><t>y</t></r></si></sst>'
        entries['xl/worksheets/sheet1.xml']=entries['xl/worksheets/sheet1.xml'].replace(b'<c r="A2" t="inlineStr"><is><t>Buy</t></is></c>',b'<c r="A2" t="s"><v>0</v></c>')
    add('xlsx-valid-shared-rich-text',text=xlsx([('Transactions',native)],rewrite=shared),extension='.xlsx')
    def utf16(entries):
        for name,value in list(entries.items()):
            if name.endswith(('.xml','.rels')):
                entries[name]=value.decode().encode('utf-16')
    add('xlsx-utf16',text=xlsx([('Transactions',native)],rewrite=utf16),extension='.xlsx')
    add('xlsm-native',text=xlsx([('Transactions',native)]),extension='.xlsm')
    add('xlsx-cached-formula',text=xlsx([('Transactions',native)],rewrite=lambda entries:entries.update({'xl/worksheets/sheet1.xml':entries['xl/worksheets/sheet1.xml'].replace(b'<c r="C2" t="inlineStr"><is><t>0.002</t></is></c>',b'<c r="C2"><f>1/500</f><v>0.002</v></c>')})),extension='.xlsx')
    add('xlsx-not-zip', text=b'not a zip', extension='.xlsx')
    add('xlsx-truncated', text=xlsx([('Transactions',native)])[:-20], extension='.xlsx')
    overlapping = bytearray(xlsx([('Transactions', native)]))
    central = int.from_bytes(overlapping[-6:-2], 'little')
    compressed = int.from_bytes(overlapping[central + 20:central + 24], 'little')
    overlapping[central + 20:central + 24] = (compressed + 1).to_bytes(4, 'little')
    add('xlsx-overlapping-member', text=bytes(overlapping), extension='.xlsx')
    return cases


def error(exc, path):
    details = exc.details if isinstance(exc, AppError) else None
    return {'code':exc.code if isinstance(exc,AppError) else 'internal_error','message':str(exc).replace(str(path),'{file}'),'hint':exc.hint if isinstance(exc,AppError) else None,'details':details,'details_json':json.dumps(details)}


def vectors():
    results = []
    with tempfile.TemporaryDirectory(prefix='ledger-vectors-') as root:
        for case in inputs():
            case['format'] = 'generic_ledger'
            case['column_map_json'] = json.dumps(case['column_map'])
            path = Path(root) / ('input' + case['extension'])
            path.write_bytes(bytes(case['bytes']) if 'bytes' in case else case['text'].encode())
            try:
                case['preview'] = clean(oracle.preview_generic_ledger_records(path, limit=case['limit'], column_map=case['column_map']))
            except Exception as exc:
                case['preview_error'] = error(exc, path)
            try:
                records = []
                for source in oracle.load_generic_ledger_records(path, column_map=case['column_map']):
                    record = normalize_import_record(source, 'file:generic_ledger')
                    fingerprint = make_transaction_fingerprint('wallet',record['external_id'],record['occurred_at'],record['direction'],record['asset'],record['amount'],record['fee'])
                    record['amount'] = btc_to_msat(record['amount'])
                    record['fee'] = btc_to_msat(record['fee'])
                    if any(not -(1<<63) <= record[key] < 1<<63 for key in ['amount','fee']):
                        raise OverflowError('Python int too large to convert to SQLite INTEGER')
                    for key in ['exchange_evidence_match_by_economics','exchange_evidence_match_time_tolerance_seconds','match_existing_ignore_fee']:
                        record.pop(key)
                    record['amount_includes_fee'] = int(record['amount_includes_fee'])
                    records.append({'fields':record, 'fingerprint':fingerprint})
                case['records'] = clean(records)
            except Exception as exc:
                case['error'] = error(exc, path)
            results.append(case)
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    for path, value in [(TABLES,tables()), (DEST,vectors())]:
        content = json.dumps(value, ensure_ascii=True, indent=2) + '\n'
        if args.check:
            if path.read_text() != content:
                raise SystemExit(f'Ledger reference drift: {path}')
        else:
            path.write_text(content)
