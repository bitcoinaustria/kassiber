import argparse
import csv
import io
import json
from pathlib import Path
import random
import tempfile
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))

from kassiber.core.imports import normalize_import_record
from kassiber.errors import AppError
from kassiber.importers import load_import_records
from kassiber.msat import btc_to_msat
from kassiber.fingerprints import make_transaction_fingerprint
from crates.wallets.scripts.generate_import_vectors import clean

DEST = Path(__file__).resolve().parents[1] / 'tests/vectors/python_binance_imports.json'
AUTO = {'timestamp UTC': '2025-01-02 03:04:05', 'base asset symbol': 'BTC', 'quote asset amount + symbol': '100.00 EUR', 'trading fee (in quote asset)': '1.00 EUR', 'base asset amount + symbol': '0.002 BTC', 'source of funds': 'Spot Wallet'}
DIVIDEND = {'id': 'reward', 'amount': '0.00001', 'asset': 'BTC', 'divTime': '1735776000123', 'enInfo': 'Mining reward'}


def inputs():
    cases = {'empty': '', 'blank': '\n', 'bad-utf8': b'\xff'}
    def add(name, changes=None, base=AUTO, rows=None, header=None, **dialect):
        output = io.StringIO(newline='')
        writer = csv.DictWriter(output, header or list(base), extrasaction='ignore', lineterminator='\n', **dialect)
        writer.writeheader()
        writer.writerows(rows if rows is not None else [{**base, **(changes or {})}])
        cases[name] = output.getvalue()
    add('autoinvest')
    add('dividend', base=DIVIDEND)
    add('header-only', rows=[])
    add('dividend-header-only', base=DIVIDEND, rows=[])
    add('unsupported', base={'date': '2025-01-02', 'amount': '1'})
    for base_name, base in [('auto', AUTO), ('dividend', DIVIDEND)]:
        for key in base:
            for value in ['', ' ', '\t']:
                add(f'{base_name}-blank-{key}-{value!r}', {key: value}, base=base)
            add(f'{base_name}-missing-{key}', base=base, header=[k for k in base if k != key])
        for label, transform in [('upper', str.upper), ('lower', str.lower), ('spaces', lambda k: ' ' + k + ' '), ('unicode', lambda k: k.replace(' ', '\u00a0\t')), ('casefold', lambda k: k.replace('s', 'ſ'))]:
            row = {transform(k): v for k, v in base.items()}
            add(f'{base_name}-headers-{label}', base=row)
        for ending in ('\r', '\r\n'):
            cases[f'{base_name}-ending-{ending!r}'] = cases['autoinvest' if base_name == 'auto' else 'dividend'].replace('\n', ending)
        cases[f'{base_name}-bom'] = '\ufeff' + cases['autoinvest' if base_name == 'auto' else 'dividend']
        add(f'{base_name}-duplicate', base=base, rows=[base, base])
        add(f'{base_name}-short', base=base, rows=[])
        cases[f'{base_name}-short'] += ','.join(list(base.values())[:2]) + '\n'
        cases[f'{base_name}-extra'] = cases['autoinvest' if base_name == 'auto' else 'dividend'].rstrip() + ',unheaded\n'
    for key in ['base asset symbol', 'asset']:
        for asset in ['btc', ' XBT ', 'XXBT', 'LBTC', 'L-BTC', 'ETH', 'SATS', 'ſbtc', '']:
            add(f'asset-{key}-{asset!r}', {key: asset}, base=DIVIDEND if key == 'asset' else AUTO)
    for value in ['0', '-0', '-1', '0.000000000005', '0.000000000015', '1e-40', '1.23456789012345678901234567890123456', '1,234.50', '($1,234.50)', '１２.３４', '١_٢e-٣', 'NaN', 'sNaN', 'Infinity', '-Infinity', 'bad', '1e309']:
        for key in ['quote asset amount + symbol', 'base asset amount + symbol', 'trading fee (in quote asset)']:
            add(f'auto-number-{key}-{value}', {key: value + (' BTC' if key == 'base asset amount + symbol' else ' EUR')})
        add('dividend-number-' + value, {'amount': value}, base=DIVIDEND)
    for value in ['100 USD', '100 USDT', '100 BTC', '100 PLN', '100 eur', '100', 'EUR', '--', '100\u00a0EUR', '100 EUR trailing']:
        add('quote-' + value, {'quote asset amount + symbol': value})
    for value in ['--', '-', '0', '1', '1 USD', '1 BTC', '1 EUR extra', '1\u00a0EUR', 'EUR EUR']:
        add('fee-' + value, {'trading fee (in quote asset)': value})
    add('mismatched-base', {'base asset symbol': 'BTC', 'base asset amount + symbol': '0.002 ETH'})
    add('fallback-base', {'base asset symbol': 'ETH'})
    add('skip-nonbtc', {'base asset symbol': 'ETH', 'base asset amount + symbol': '1 ETH'})
    add('skip-invalid-quote', {'base asset symbol': 'ETH', 'base asset amount + symbol': '1 ETH', 'quote asset amount + symbol': 'bad'})
    add('zero-base', {'base asset amount + symbol': '0 BTC'})
    add('negative-base', {'base asset amount + symbol': '-0.002 BTC'})
    for value in ['0', '-0.0005', '-0.0006', '0.0005', '999.9999995', '999.9995', '1000.0005', '-999.9995', '1735776000123.456', '1735776000999.9999', 'bad', 'NaN', 'sNaN', 'Infinity', '1e30', '1e20', '-1e20', '253402300799999.9999', '-62135596800000.0001', '253402300800000', '-62135596801000', '１２３', '1_000', '']:
        add('epoch-' + value, {'divTime': value}, base=DIVIDEND)
    for value in ['reward', 'MINING', 'ｍining', 'mıning', 'mine\nmining', 'ordinary', '']:
        add('info-' + value, {'enInfo': value}, base=DIVIDEND)
    add('timestamp-fallback', base={**DIVIDEND, 'divTime': '', 'timestamp': '2025-01-02'})
    add('bad-date', {'timestamp UTC': 'invalid'})
    add('partial-failure', rows=[AUTO, {**AUTO, 'timestamp UTC': 'invalid'}])
    add('schema-precedence', base={**AUTO, **DIVIDEND})
    add('fee-alias', base={**AUTO, 'Trading Fee': '2 EUR'})
    for key in ['timestamp UTC', 'base asset symbol', 'asset', 'enInfo']:
        base = DIVIDEND if key in DIVIDEND else AUTO
        for value in ['', 'other']:
            add(f'collision-{key}-{value!r}', base={**base, key.upper(): value})
    randomizer = random.Random(2108)
    for index in range(24):
        base = randomizer.choice([AUTO, DIVIDEND])
        rows = [{**base, ('source of funds' if base is AUTO else 'enInfo'): randomizer.choice(['two, parts', 'double "quote"', 'line\r\nnext', 'plain'])} for _ in range(randomizer.randrange(1, 4))]
        add(f'seeded-{index}', base=base, rows=rows)
    return cases


def vectors():
    result = []
    with tempfile.TemporaryDirectory(prefix='binance-vectors-') as root:
        file = Path(root) / 'input.csv'
        for name, content in inputs().items():
            file.write_bytes(content.encode() if isinstance(content, str) else content)
            case = {'name': 'binance:' + name, 'format': 'binance_supplemental_csv', **({'text': content} if isinstance(content, str) else {'bytes': list(content)})}
            try:
                records = []
                for row in load_import_records(str(file), 'binance_supplemental_csv'):
                    record = normalize_import_record(row, 'file:binance_supplemental_csv')
                    fingerprint = make_transaction_fingerprint('wallet', record['external_id'], record['occurred_at'], record['direction'], record['asset'], record['amount'], record['fee'])
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
            raise SystemExit('Binance vectors drifted')
    else:
        DEST.write_text(value)
