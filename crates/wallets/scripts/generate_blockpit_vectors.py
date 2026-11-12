import argparse
import csv
import io
import json
import random
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
from kassiber.core.imports import normalize_import_record
from kassiber.errors import AppError
from kassiber.fingerprints import make_transaction_fingerprint
from kassiber.importers import load_import_records, _BLOCKPIT_TYPES
from kassiber.msat import btc_to_msat
from generate_exchange_vectors import clean

DEST = Path(__file__).resolve().parents[1] / 'tests/vectors/python_blockpit_imports.json'
BASE = {'Date (UTC)': '01.02.2025 01:02:03', 'Integration Name': 'Kraken', 'Label': 'Trade', 'Outgoing Asset': 'EUR', 'Outgoing Amount': '100.00', 'Incoming Asset': 'BTC', 'Incoming Amount': '0.00200000', 'Fee Asset': 'EUR', 'Fee Amount': '1.00', 'Comments': 'buy', 'Trx. ID': 'bp-buy', 'Source Type': 'API', 'Source Name': 'Kraken API'}


def inputs():
    cases = {'empty': '', 'blank': '\n \n', 'utf8': b'\xff', 'utf16': ','.join(BASE).encode('utf-16')}

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
    cases['bom'] = '\ufeff' + cases['base']
    cases['blank-prefix'] = '\n\n' + cases['base']
    cases['double-bom'] = '\ufeff' + cases['bom']
    cases['bad-bom'] = b'\xef\xbb\xbf\xff'
    for delimiter in (';', '\t', '|'):
        add('delimiter-' + repr(delimiter), delimiter=delimiter)
    for quote in ("'", '"'):
        add('quote-' + quote, {'Comments': f'one, "two"\r\nnext {quote}word{quote}'}, quotechar=quote)
    cases['extra-unheaded'] = cases['base'].rstrip() + ',ignored\n'
    cases['short-row'] = ','.join(BASE) + '\n2025-01-02,Source,Deposit,,,BTC,1\n'
    cases['field-limit'] = cases['base'].replace('Kraken API', 'x' * 131073)
    for key in BASE:
        add('missing-' + key, header=[k for k in BASE if k != key])
        for blank in ('', ' ', '\t'):
            add('blank-' + key + repr(blank), {key: blank})
    for style, transform in (('lower', str.lower), ('upper', str.upper), ('spaces', lambda k: ' ' + k + ' '), ('unicode', lambda k: k.replace(' ', '\u00a0\t')), ('casefold', lambda k: k.replace('s', 'ſ').replace('fi', 'ﬁ'))):
        add('headers-' + style, header=[transform(k) for k in BASE], rows=[{transform(k): v for k, v in BASE.items()}])
    for key, alternatives in (('Date (UTC)', ['Date']), ('Fee Asset', ['Fee Asset (optional)']), ('Fee Amount', ['Fee Amount (optional)']), ('Comments', ['Comment', 'Comment (optional)']), ('Trx. ID', ['Trx. ID (optional)', 'Transaction ID'])):
        for alias in alternatives:
            renamed = {alias if k == key else k: v for k, v in BASE.items()}
            add('alias-' + alias, header=list(renamed), rows=[renamed])
            for value in ('', 'alternative'):
                add('collision-' + alias + repr(value), header=list(BASE) + [alias], changes={key: value, alias: BASE[key]})
    optional = {k + ' (optional)' if k in {'Fee Asset', 'Fee Amount', 'Comments', 'Trx. ID'} else k: v for k, v in BASE.items()}
    optional['Comment (optional)'] = optional.pop('Comments (optional)')
    add('manual-template', header=list(optional), rows=[optional])
    for extra in (['LABEL', 'label'], ['label', 'LABEL'], [' Label ', 'Label'], ['Label', ' Label ']):
        add('collision-' + repr(extra), header=list(BASE) + extra, changes={extra[0]: 'bad', extra[1]: 'Trade'})
    for field in ('Incoming Amount', 'Outgoing Amount', 'Fee Amount'):
        for value in ('0', '-0', '-1', '1e-12', '0.000000000005', '0.000000000015', '1.23456789012345678901234567890123456', '($1,234.50)', '1.234,56', '0,05', '1,234,567', '1.234.567', ' 12 LBTC ', '１２.３４', '١_٢e-٣', 'NaN', 'sNaN', 'Infinity', '-Infinity', 'bad', '1e309'):
            add('number-' + field + value, {field: value})
    for key in ('Incoming Asset', 'Outgoing Asset', 'Fee Asset'):
        for value in ('BTC', 'btc', ' XBT ', 'XXBT', 'LBTC', 'L-BTC', 'SATS', 'EUR', 'USD', 'ETH', ''):
            add('asset-' + key + value, {key: value})
    for kind in [*_BLOCKPIT_TYPES, 'Trade', 'Transfer', 'Receive Loan', 'spend', 'donation', 'fork', ' ', ' STAKING ', 'ſteuerfrei (ein)', 'GEBÜHR']:
        add('type-in-' + kind, {'Label': kind, 'Outgoing Asset': '', 'Outgoing Amount': '', 'Fee Amount': '', 'Fee Asset': ''})
        add('type-out-' + kind, {'Label': kind, 'Incoming Asset': '', 'Incoming Amount': '', 'Outgoing Asset': 'BTC', 'Outgoing Amount': '0.002', 'Fee Amount': '', 'Fee Asset': ''})
    for value in ('2025-01-02', '20250102', '2025-W01-4', '2025-01-02T01:02:03.123456Z', '2025-01-02T00:00 Z', '1.2.2025', '１.２.２０２５', '01.02.222', 'invalid', ''):
        add('date-' + value, {'Date (UTC)': value})
    add('no-id', {'Trx. ID': ''})
    add('repeat', rows=[BASE, BASE])
    add('idless-repeat', rows=[{**BASE, 'Trx. ID': ''}] * 2)
    add('idless-comments', rows=[{**BASE, 'Trx. ID': ''}, {**BASE, 'Trx. ID': '', 'Comments': 'changed'}])
    add('standalone', {'Label': 'Fee', 'Incoming Amount': '', 'Incoming Asset': '', 'Outgoing Amount': '', 'Outgoing Asset': '', 'Fee Amount': '0.00001', 'Fee Asset': 'BTC'})
    add('standalone-no-type', {'Label': '', 'Incoming Amount': '', 'Incoming Asset': '', 'Outgoing Amount': '', 'Outgoing Asset': '', 'Fee Amount': '0.00001', 'Fee Asset': 'BTC'})
    add('skip-invalid', {'Label': 'bad', 'Incoming Asset': 'ETH', 'Outgoing Asset': 'USD', 'Incoming Amount': 'bad', 'Date (UTC)': 'bad', 'Fee Amount': '0'})
    add('skip-fee-invalid', {'Label': 'bad', 'Incoming Asset': 'ETH', 'Outgoing Asset': 'USD', 'Incoming Amount': 'bad', 'Date (UTC)': 'bad', 'Fee Amount': 'bad'})
    add('sell-net', {'Incoming Amount': '100', 'Incoming Asset': 'EUR', 'Outgoing Amount': '0.002', 'Outgoing Asset': 'BTC', 'Fee Amount': '101', 'Fee Asset': 'EUR'})
    add('failed-batch', rows=[BASE, {**BASE, 'Date (UTC)': 'invalid'}])
    add('bare-cr', lineterminator='\r')
    randomizer = random.Random(2103)
    for index in range(40):
        rows = [{**BASE, 'Comments': randomizer.choice(['plain', 'two, pieces', 'double "quote"', "single 'quote'", 'line\nnext', '\t whitespace ']), 'Trx. ID': randomizer.choice(['', 'source-id']), 'Incoming Amount': randomizer.choice(['0.002', '0,002', '0.000000000015'])} for _ in range(randomizer.randrange(1, 5))]
        add(f'seeded-dialect-{index}', rows=rows, delimiter=randomizer.choice([',', ';', '\t', '|']), quotechar=randomizer.choice(['"', "'"]), lineterminator=randomizer.choice(['\n', '\r\n', '\r']))
    from tests.test_tax_platform_migration import BLOCKPIT_CSV
    cases['smoke'] = BLOCKPIT_CSV
    return cases


def vectors():
    result = []
    with tempfile.TemporaryDirectory(prefix='blockpit-vectors-') as root:
        file = Path(root) / 'input.csv'
        for name, content in inputs().items():
            file.write_bytes(content.encode() if isinstance(content, str) else content)
            case = {'name': 'blockpit:' + name, 'format': 'blockpit_csv', **({'text': content} if isinstance(content, str) else {'bytes': list(content)})}
            try:
                records = []
                for row in load_import_records(str(file), 'blockpit_csv'):
                    record = normalize_import_record(row, 'file:blockpit_csv')
                    fingerprint = make_transaction_fingerprint('wallet', record['external_id'] or row['_tax_platform_row_identity'], record['occurred_at'], record['direction'], record['asset'], record['amount'], record['fee'])
                    record['amount'] = btc_to_msat(record['amount'])
                    record['fee'] = btc_to_msat(record['fee'])
                    if any(not -(1 << 63) <= record[k] < (1 << 63) for k in ('amount', 'fee')):
                        raise OverflowError('Python int too large to convert to SQLite INTEGER')
                    for key in ('exchange_evidence_match_by_economics', 'exchange_evidence_match_time_tolerance_seconds', 'match_existing_ignore_fee'):
                        record.pop(key)
                    record['amount_includes_fee'] = int(record['amount_includes_fee'])
                    records.append({'fields': record, 'fingerprint': fingerprint})
                case['records'] = clean(records)
            except Exception as exc:
                case['error'] = {'code': exc.code if isinstance(exc, AppError) else 'internal_error', 'message': str(exc), 'hint': exc.hint if isinstance(exc, AppError) else None}
            result.append(case)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    value = json.dumps(vectors(), ensure_ascii=True, indent=2) + '\n'
    if args.check:
        if DEST.read_text() != value:
            raise SystemExit('Blockpit vectors drifted')
    else:
        DEST.write_text(value)
