#!/usr/bin/env python3
"""Freeze Bull's wallet-history parser branches; coordinator parity uses the daemon oracle."""
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
from generate_exchange_vectors import clean

DEST = Path(__file__).resolve().parents[1] / 'tests/vectors/python_bull_wallet_imports.json'
HEADER = ['date','type','direction','amount_sats','amount_btc','fee_sats','status','txid','network','address','swap_id','preimage',
          'total_swap_fees_sats','send_network','receive_network','send_txid','receive_txid']
BASE = dict(zip(HEADER, ['2025-01-02T01:02:03Z','onchain','incoming','150000','0.0015','200','confirmed','ab'*32,'bitcoin','bc1q',
                         '','','','','','','']))
PREIMAGE = '11' * 32


def inputs():
    cases = {'empty': '', 'header-only': ','.join(HEADER) + '\n', 'blank-header': '\nx\n', 'utf8': b'\xff'}

    def add(name, changes=None, *, header=None, rows=None, dialect=None):
        header = HEADER if header is None else header
        output = io.StringIO(newline='')
        writer = csv.writer(output, **(dialect or {}))
        writer.writerow(header)
        for row in rows if rows is not None else [{**BASE, **(changes or {})}]:
            writer.writerow([row.get(k, row.get(k.strip(), '')) for k in header])
        cases[name] = output.getvalue()

    add('base')
    cases['bom'] = '\ufeff' + cases['base']
    add('semicolon', dialect={'delimiter': ';'})
    for key in HEADER:
        add('missing-' + key, header=[k for k in HEADER if k != key])
    add('missing-all-but-date', header=['date'])
    for style, transform in [('upper', str.upper), ('spaces', lambda k: ' \t' + k + '  '), ('long-s', lambda k: k.replace('s', 'ſ'))]:
        add('headers-' + style, header=[transform(k) for k in HEADER], rows=[{transform(k): v for k, v in BASE.items()}])
    for v in ('failed', 'FAILED', ' expired ', 'refunded', 'pending', 'Completed', 'confirmed', ''):
        add('status-' + v, {'status': v})
    for v in ('incoming', 'Inbound', ' received ', 'receive', 'outgoing', 'OUTBOUND', 'sent', 'send', 'self', 'SELF', 'sideways', '', 'ſend'):
        add('direction-' + v, {'direction': v})
    for key in ('amount_sats', 'fee_sats'):
        for v in ('0', '-0', '-150', '1.0', '1.5', '1E+3', '1e-3', '($1,234)', '1,234', ' 7 ', '', 'NaN', '-NaN', 'sNaN', 'Infinity', '-Infinity', 'bad', '9' * 30, '１２'):
            add(f'number-{key}-{v}', {key: v})
    for kind in ('onchain', 'liquid', 'lightning_send', 'lightning_receive', 'payjoin_send', 'payjoin_receive', 'chain_swap', 'mystery', '', 'LIQUID'):
        add('type-' + kind, {'type': kind, 'network': ''})
        add('type-network-' + kind, {'type': kind})
    for network in ('bitcoin', 'BTC', 'onchain', 'liquid', 'lbtc', 'LiquidV1', 'lightning', 'LN', ' ln ', 'doge', 'chain'):
        add('network-' + network, {'network': network})
    add('swap-in', {'type': 'chain_swap', 'network': '', 'receive_network': 'liquid', 'send_network': 'bitcoin'})
    add('swap-out', {'type': 'chain_swap', 'direction': 'outgoing', 'network': '', 'receive_network': 'liquid', 'send_network': 'lightning'})
    add('swap-none', {'type': 'chain_swap', 'network': ''})
    for name, changes in {
        'txid-receive': {'txid': '', 'receive_txid': 'r1', 'send_txid': 's1', 'swap_id': 'w1'},
        'txid-send': {'txid': ' ', 'send_txid': ' s1 ', 'swap_id': 'w1'},
        'txid-swap': {'txid': '', 'swap_id': 'w1'},
        'txid-fallback': {'txid': ''},
        'txid-fallback-empty': {'txid': '', 'date': '', 'type': '', 'direction': 'in', 'amount_sats': '5'},
        'txid-fallback-spaces': {'txid': '', 'date': ' ', 'type': ' '},
    }.items():
        add(name, changes)
    for kind in ('lightning_send', 'lightning_receive', 'onchain'):
        add('preimage-' + kind, {'type': kind, 'preimage': PREIMAGE, 'txid': 'swap-ln'})
        add('preimage-upper-' + kind, {'type': kind, 'preimage': ' ' + PREIMAGE.upper() + ' '})
        add('preimage-spaced-' + kind, {'type': kind, 'preimage': '11 ' * 21 + '1'})
        add('preimage-short-' + kind, {'type': kind, 'preimage': '11', 'txid': 'CD' * 32})
        add('preimage-bad-' + kind, {'type': kind, 'preimage': 'zz' * 32, 'txid': ' ' + 'ef' * 32 + ' '})
        add('preimage-blank-' + kind, {'type': kind, 'preimage': ' ', 'txid': 'not-hex'})
    add('preimage-duplicate', header=HEADER + [' preimage '], changes={'preimage': PREIMAGE, ' preimage ': 'x'})
    add('extra', header=HEADER + ['extra'], changes={'extra': 'kept'})
    cases['short-row'] = ','.join(HEADER) + '\n2025-01-02,onchain,incoming,5\n'
    cases['unheaded-extra'] = cases['base'].rstrip() + ',ignored\n'
    add('mixed', rows=[
        BASE,
        {**BASE, 'type': 'liquid', 'network': '', 'txid': 'l1', 'direction': 'outgoing', 'amount_sats': '-5000', 'fee_sats': '1.0', 'status': 'completed'},
        {**BASE, 'type': 'lightning_receive', 'network': '', 'txid': '', 'swap_id': 'sw', 'preimage': PREIMAGE, 'fee_sats': ''},
        {**BASE, 'status': 'failed'}, {**BASE, 'direction': 'self'},
        {**BASE, 'type': 'payjoin_send', 'direction': 'sent', 'network': '', 'txid': 'pj'},
    ])
    for stamp in ('2025-01-02', 'invalid', ''):
        add('date-' + repr(stamp), {'date': stamp})
    return cases


def vectors():
    cases = []
    with tempfile.TemporaryDirectory(prefix='kassiber-bull-wallet-vectors-') as root:
        path = Path(root) / 'input.csv'
        for name, content in inputs().items():
            data = content.encode() if isinstance(content, str) else content
            path.write_bytes(data)
            case = {'name': 'bull-wallet:' + name, 'format': 'bullbitcoin_wallet_csv',
                    **({'text': content} if isinstance(content, str) else {'bytes': list(data)})}
            try:
                records, networks = [], []
                for row in load_import_records(str(path), 'bullbitcoin_wallet_csv'):
                    networks.append(row['_bullbitcoin_wallet_network'])
                    r = normalize_import_record(row, 'file:bullbitcoin_wallet_csv')
                    fp = make_transaction_fingerprint('wallet', r['external_id'], r['occurred_at'], r['direction'], r['asset'], r['amount'], r['fee'])
                    r['amount'] = btc_to_msat(r['amount'])
                    r['fee'] = btc_to_msat(r['fee'])
                    if any(not -(1 << 63) <= r[k] < (1 << 63) for k in ('amount', 'fee')):
                        raise OverflowError('Python int too large to convert to SQLite INTEGER')
                    assert r.pop('match_existing_ignore_fee') is True
                    for k in ('exchange_evidence_match_by_economics', 'exchange_evidence_match_time_tolerance_seconds'):
                        r.pop(k)
                    r['amount_includes_fee'] = int(r['amount_includes_fee'])
                    records.append({'fields': r, 'fingerprint': fp})
                case['records'] = clean(records)
                case['networks'] = networks
            except Exception as exc:
                case['error'] = {'code': exc.code if isinstance(exc, AppError) else 'internal_error', 'message': str(exc),
                                 'hint': exc.hint if isinstance(exc, AppError) else None}
            cases.append(case)
    return cases


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    text = json.dumps(vectors(), ensure_ascii=True, indent=2) + '\n'
    if args.check:
        if DEST.read_text() != text:
            raise SystemExit('Bull wallet import vectors drifted')
    else:
        DEST.write_text(text)
