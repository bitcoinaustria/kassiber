import argparse
import csv
import io
import json
from pathlib import Path
import random
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

from kassiber.core.imports import normalize_import_record
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.importers import load_import_records
from kassiber.msat import btc_to_msat
from crates.wallets.scripts.generate_import_vectors import clean

DEST = Path(__file__).resolve().parents[1] / 'tests/vectors/python_btcpay_imports.json'
BASE = {'TransactionId': 'ab' * 32, 'Timestamp': '2025-01-02 03:04:05', 'Currency': 'BTC', 'Amount': '-0.002 BTC', 'Comment': ' imported note ', 'Labels': 'Shop, two words, Shop'}


def inputs():
    cases = []
    def add(name, rows=None, format='json', content=None, **changes):
        if content is None:
            rows = rows if rows is not None else [{**BASE, **changes}]
            if format == 'json':
                content = json.dumps(rows, ensure_ascii=True)
            else:
                output = io.StringIO(newline='')
                writer = csv.DictWriter(output, list(rows[0]) if rows else list(BASE), lineterminator='\n')
                writer.writeheader()
                writer.writerows(rows)
                content = output.getvalue()
        cases.append({'name': 'btcpay:' + format + ':' + name, 'format': 'btcpay_' + format, **({'bytes': list(content)} if isinstance(content, bytes) else {'text': content})})
    for format in ['csv', 'json']:
        add('base', format=format)
        add('empty', rows=[], format=format)
        add('duplicate', rows=[BASE, BASE], format=format)
        add('partial-normalization-failure', rows=[BASE, {**BASE, 'Timestamp': 'bad'}], format=format)
        add('partial-parser-failure', rows=[BASE, {**BASE, 'Amount': 'bad'}], format=format)
        for key in BASE:
            add('missing-' + key, rows=[{k:v for k,v in BASE.items() if k != key}], format=format)
            for value in ['', ' ', '\t']:
                add('blank-' + key + repr(value), format=format, **{key:value})
        for field in ['TransactionId', 'Timestamp', 'Currency', 'Amount', 'Comment', 'Labels']:
            for value in ['0', 'True', 'two\nlines', 'two\r\nlines', 'İſß']:
                add('text-' + field + repr(value), format=format, **{field:value})
        for amount in ['0', '-0', '-0.0 BTC', '0.000000000005 BTC', '-0.000000000015 BTC', '1.23456789012345678901234567890123456 BTC', '1e-40 BTC', '1e20 BTC', '1e999999 BTC', 'NaN BTC', 'sNaN BTC', '-Infinity BTC', '0.2 btc', '0.2 Btc', '0.2 XBT', '1,000 BTC', '１２.３４ BTC', '١_٢e-٣ BTC', 'BTC']:
            add('amount-' + amount, format=format, Amount=amount)
        for currency, amount in [('l-btc', '1 LBTC'), ('LBTC', '-1 lbtc'), ('XBT', '1 XBT'), ('', '1 BTC'), (' ', '1 BTC'), ('eur', '1 EUR'), ('ab'*32, '1 ' + 'ab'*32)]:
            add('currency-' + currency, format=format, Currency=currency, Amount=amount)
        for field in ['confirmed_at', 'ConfirmedAt', 'Transaction Id']:
            add('alias-' + field, rows=[{**BASE, 'TransactionId':'', field: '2025-01-03' if 'At' in field or 'at' in field else 'alias'}], format=format)
        add('label-collision', format=format, Labels='A B,a-b,A  B, İ,İ')
        add('no-comment-labels', format=format, Comment='', Labels='')
        add('bad-utf8', content=b'\xff', format=format)
    for field in ['Amount', 'TransactionId', 'Timestamp', 'Currency', 'Comment', 'Labels', 'confirmed_at']:
        for value in [None, False, True, 0, 1, 1.5, [], {}, ['one', {'text':'two'}, None, False], {'a':None, 'b':{'text':'two'}, 'c':{'other':1}, 'd':''}]:
            add('typed-' + field + repr(value), **{field:value})
    for value in [None, False, 1, 'row', [], {}, {'transactions':[]}]:
        add('top-' + repr(value), content=json.dumps(value))
    for value in [None, False, 1, 'row', []]:
        add('row-' + repr(value), rows=[value])
    for labels in [[{}, {'text':None}, {'text':''}, {'text':0}, {'text':False}], {'fallback':{}, 'empty':None, '':None}, {'x':{'text':[1]}, 'y':{'text':{'a':1}}}]:
        add('labels-' + repr(labels), Labels=labels)
    for field in ['TransactionId', 'Comment', 'Labels']:
        add('surrogate-' + field, **{field:' İ\ud800 '})
    for name, currency, amount in [
        ('plain', ' İ\ud800 ', '1'),
        ('uppercase-expansion', 'ß\ud800', '1 SS\ud800'),
        ('lowercase-context', 'ΟΣ\ud800', '1 ος\ud800'),
        ('replacement-collision', '\ud800', '1 �'),
        ('matching-suffix', '\ud800', '1 \ud800'),
        ('invalid-amount', '\ud800', 'invalid'),
    ]:
        add('currency-surrogate-' + name, Currency=currency, Amount=amount)
    add('precedence', rows=[{**BASE, 'Transaction Id':'alternate', 'confirmed_at':'2025-01-03', 'ConfirmedAt':'2025-01-04', 'amount':99, 'fee':1, 'fiat_rate':123}])
    add('label-reference-quirk', Labels={'fallback':{}})
    add('whitespace-id-reference-quirk', TransactionId=' ' + BASE['TransactionId'] + ' ')
    for format in ['csv', 'json']:
        base = next(c['text'] for c in cases if c['name'] == 'btcpay:' + format + ':base')
        add('bom', content='\ufeff' + base, format=format)
        add('bare-cr', content=base.replace('\n', '\r'), format=format)
        add('crlf', content=base.replace('\n', '\r\n'), format=format)
        add('invalid-json' if format == 'json' else 'extra-column', content='[\r invalid]' if format == 'json' else base.rstrip() + ',extra\n', format=format)
    randomizer = random.Random(2109)
    for index in range(24):
        rows = [{**BASE, 'TransactionId':str(index), 'Amount':randomizer.choice(['0.002 BTC', '-0.003 BTC']), 'Labels':randomizer.choice(['two, parts', 'quote "one"', 'two\r\nlines']), 'Comment':randomizer.choice(['', 'note', 'two\nlines'])} for _ in range(randomizer.randrange(1,4))]
        add('seeded-' + str(index), rows=rows, format=randomizer.choice(['csv', 'json']))
    return cases


def vectors():
    result = []
    with tempfile.TemporaryDirectory(prefix='btcpay-vectors-') as root:
        file = Path(root) / 'input.csv'
        for case in inputs():
            file.write_bytes(bytes(case['bytes']) if 'bytes' in case else case['text'].encode())
            try:
                records = []
                for row in load_import_records(str(file), case['format']):
                    record = normalize_import_record(row, 'file:' + case['format'])
                    fingerprint = make_transaction_fingerprint('wallet', record['external_id'], record['occurred_at'], record['direction'], record['asset'], record['amount'], record['fee'])
                    record['amount'] = btc_to_msat(record['amount'])
                    record['fee'] = btc_to_msat(record['fee'])
                    if any(not -(1 << 63) <= record[k] < (1 << 63) for k in ('amount', 'fee')):
                        raise OverflowError('Python int too large to convert to SQLite INTEGER')
                    for key in ('exchange_evidence_match_by_economics', 'exchange_evidence_match_time_tolerance_seconds', 'match_existing_ignore_fee'):
                        record.pop(key)
                    record['amount_includes_fee'] = int(record['amount_includes_fee'])
                    import sqlite3
                    values = [None] * 34
                    values[4] = record['external_id']
                    values[9] = record['asset']
                    values[31] = record['description']
                    if isinstance(record['description'], bool):
                        record['description'] = int(record['description'])
                    with sqlite3.connect(':memory:') as conn:
                        conn.execute('SELECT ' + ','.join('?' for _ in values), values)
                    records.append({'fields': record, 'fingerprint':fingerprint})
                case['records'] = clean(records)
            except Exception as exc:
                case['error'] = {'code':exc.code if isinstance(exc, AppError) else 'internal_error', 'message':str(exc), 'hint':exc.hint if isinstance(exc, AppError) else None}
            result.append(case)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    value = json.dumps(vectors(), ensure_ascii=True, indent=2) + '\n'
    if args.check:
        if DEST.read_text() != value:
            raise SystemExit('BTCPay vectors drifted')
    else:
        DEST.write_text(value)
