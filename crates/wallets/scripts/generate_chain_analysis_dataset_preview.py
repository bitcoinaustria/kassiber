"""Freeze unchanged Python local dataset preview envelopes.

The reference builds normalized manifests by iterating a ``set`` of required
fields, so their key order follows ``PYTHONHASHSEED``. This oracle pins the
sorted order the native port uses; the unpatched variants are recorded by
``tests/rust_port/repro_dataset_manifest_hashseed.py``.
"""
from __future__ import annotations

import io
import json
from pathlib import Path
import sys
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_durable import capture, database
from kassiber.core import chain_analysis_api as api
from kassiber.core import chain_analysis_datasets as datasets
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError

DEST = ROOT / 'crates/wallets/tests/data/chain_analysis_dataset_preview.json'
TX = 'a' * 64
ADDRESS = 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4'
MANIFEST = {'dataset_key': 'pack-1', 'name': 'Pack', 'version': '1', 'chain': 'bitcoin', 'network': 'main',
            'source': 'Local file', 'license': 'CC0', 'attribution_method': 'published', 'visibility': 'public'}


def sorted_required():
    return patch.object(datasets, '_MANIFEST_REQUIRED', dict.fromkeys(sorted(datasets._MANIFEST_REQUIRED)).keys())


def evaluate(case, conn=None):
    own = conn is None
    conn = database(case.get('sql', '')) if own else conn
    result = []
    try:
        with sorted_required():
            for action in case['actions']:
                kind = 'ui.chain_analysis.' + action['operation']
                stream = io.BytesIO(bytes.fromhex(action['stream'])) if 'stream' in action else None
                try:
                    value = build_envelope(kind, api.dispatch(conn, kind, action.get('args', {}), source_stream=stream))
                except AppError as error:
                    value = build_error_envelope(error.code, str(error), details=error.details, hint=error.hint, retryable=error.retryable)
                result.append({'wire': json.dumps(value, separators=(',', ':')), 'result': value, 'state': capture(conn), 'in_transaction': conn.in_transaction})
        return result
    finally:
        if own:
            conn.close()


def csv(header, rows, end='\n'):
    return (end.join([header, *rows]) + end).encode()


def jsonl(rows):
    return ''.join(json.dumps(row) + '\n' for row in rows).encode()


def vectors():
    cases = []

    def add(name, data, manifest=None, **options):
        args = {'manifest': MANIFEST if manifest is None else manifest, **options}
        case = {'name': name, 'sql': '', 'actions': [{'operation': 'datasets.preview', 'args': args, 'stream': data.hex()}]}
        case['expected'] = evaluate(case)
        cases.append(case)

    good = csv('subject,label,category', [f'{TX},Exchange A,exchange', f'{ADDRESS},Mixer,Mixer'])
    add('csv-basic', good)
    for index, manifest in enumerate([
        {**MANIFEST, 'source_url': 'https://example.org/data', 'observed_at': '2026-01-01', 'valid_from': '2025-01-01T00:00:00Z',
         'valid_until': '2027-01-01T00:00:00+01:00', 'category': 'service', 'expected_active_id': 'abc'},
        {**MANIFEST, 'network': 'mainnet'}, {**MANIFEST, 'network': 'bitcoin'}, {**MANIFEST, 'network': 'testnet'},
        {**MANIFEST, 'chain': 'liquid', 'network': 'liquidv1'}, {**MANIFEST, 'chain': 'liquid', 'network': 'main'},
        {**MANIFEST, 'network': 'regtest'}, {**MANIFEST, 'network': 'signet'}, {**MANIFEST, 'name': '  Padded 雪  '},
        {**MANIFEST, 'category': None}, {**MANIFEST, 'expected_active_id': None}, {**MANIFEST, 'expected_active_id': ''},
        {**MANIFEST, 'source_url': None}, {**MANIFEST, 'source_url': ''}, {**MANIFEST, 'observed_at': ''},
        {k: v for k, v in MANIFEST.items() if k != 'license'}, {**MANIFEST, 'extra': 1}, [], 'manifest', None,
        {**MANIFEST, 'chain': 'lightning'}, {**MANIFEST, 'network': 'moon'}, {**MANIFEST, 'network': ''}, {**MANIFEST, 'network': 5},
        {**MANIFEST, 'name': ''}, {**MANIFEST, 'name': ' '}, {**MANIFEST, 'name': 'x' * 257}, {**MANIFEST, 'source': 'x' * 512},
        {**MANIFEST, 'source': 'x' * 513}, {**MANIFEST, 'license': 'a\nb'}, {**MANIFEST, 'version': 1}, {**MANIFEST, 'name': '\ud800'},
        {**MANIFEST, 'name': '', 'version': '', 'license': ''}, {**MANIFEST, 'dataset_key': '-bad'}, {**MANIFEST, 'dataset_key': 'k' * 128},
        {**MANIFEST, 'dataset_key': 'k' * 129}, {**MANIFEST, 'dataset_key': 'a b'}, {**MANIFEST, 'dataset_key': ' ok.key_1-x '},
        {**MANIFEST, 'visibility': 'Public'}, {**MANIFEST, 'visibility': 'private'},
        {**MANIFEST, 'valid_from': '2026-01-01', 'valid_until': '2026-01-01'}, {**MANIFEST, 'valid_from': '2027-01-01', 'valid_until': '2026-01-01'},
        {**MANIFEST, 'observed_at': 'yesterday'}, {**MANIFEST, 'observed_at': '2026-02-30'}, {**MANIFEST, 'observed_at': 'x' * 41},
        {**MANIFEST, 'observed_at': '2026-01-01T10:00:00.123456-05:30'}, {**MANIFEST, 'observed_at': '2026-01-01 10:00'},
        {**MANIFEST, 'observed_at': '20260101T1000'}, {**MANIFEST, 'observed_at': '0001-01-01T00:00:00+01:00'},
        {**MANIFEST, 'observed_at': '2026-01-01\ud800'}, {**MANIFEST, 'observed_at': 20260101}, {**MANIFEST, 'category': 'x' * 65},
        {**MANIFEST, 'expected_active_id': 7},
    ]):
        add(f'manifest-{index}', good, manifest)
    for index, url in enumerate(['http://example.org', 'HTTPS://Example.org/a/b', 'ftp://example.org', 'http://', 'http:example.org',
                                 'http://user@example.org', 'http://:pw@example.org', 'http://@example.org', 'http://:@example.org',
                                 'http://example.org/?', 'http://example.org/?q=1', 'http://example.org/#', 'http://example.org/#f',
                                 'http://[::1]/', 'http://[::1', 'http://::1]/', 'http://[1.2.3.4]/', 'http://[v1.x]/', 'http://[vz.x]/',
                                 'http://[fe80::1%25eth0]/', 'http://[fe80::1%eth0]/', 'http://[]/', 'http://ex\uff0fample.org',
                                 'http://exämple.org', 'http://example.org:99999', 'http://:80', '  https://example.org  ', 'https://a@b@c',
                                 'http://example.org?x', 'http://a:b', 'x' * 1025, 'mailto:x@example.org', 'h t t p://x', 'https://[::ffff:1.2.3.4]/']):
        add(f'url-{index}', good, {**MANIFEST, 'source_url': url})
    for index, options in enumerate([{'format': 'jsonl'}, {'format': 'xml'}, {'adapter': 'other'}, {'format': None}, {'adapter': ['generic']},
                                     {'format': 'csv', 'adapter': 'am_i_exposed'}, {'expected_sha256': 'x'}, {'unknown': 1}]):
        add(f'options-{index}', good, **options)
    csv_cases = {
        'bom': b'\xef\xbb\xbf' + good, 'crlf': csv('subject,label', [f'{TX},A', f'{TX}:1,B'], '\r\n'),
        'quoted': csv('subject,label,source', [f'"{TX}","A ""quoted"" label","multi\nline"']),
        'quote-comma': csv('subject,label', [f'{TX},"a,b"']), 'blank-lines': csv('subject,label', ['', f'{TX},A', '', '']),
        'no-final-newline': f'subject,label\n{TX},A'.encode(), 'empty': b'', 'header-only': b'subject,label\n',
        'blank-header': b'\nsubject,label\n', 'empty-header-cell': csv('subject,,label', [f'{TX},x,A']),
        'space-header': csv('subject, ,label', [f'{TX},x,A']), 'long-header': csv('subject,label,' + 'h' * 65, [f'{TX},A,x']),
        'many-columns': csv(','.join(['subject', 'label'] + [f'c{i}' for i in range(63)]), [','.join([TX, 'A'] + ['x'] * 63)]),
        'duplicate-header': csv('Subject,subject ,label', [f'{TX},{TX},A']), 'case-header': csv(' SUBJECT ,Label', [f'{TX},A']),
        'short-row': csv('subject,label,category', [f'{TX},A']), 'long-row': csv('subject,label', [f'{TX},A,extra']),
        'bad-quote': csv('subject,label', [f'"{TX}"x,A']), 'eof-in-quote': f'subject,label\n{TX},"open\n'.encode(),
        'cr-in-field': b'subject,label\n' + TX.encode() + b',A\rB\n', 'bare-cr-row': b'subject,label\r' + TX.encode() + b',A\n',
        'nul': csv('subject,label', [f'{TX},A\0B']), 'invalid-utf8': b'subject,label\n' + TX.encode() + b',\xff\n',
        'utf8-label': csv('subject,label', [f'{TX},Börse 雪']), 'bom-later': csv('subject,label', [f'{TX},\ufeffA']),
        'huge-record': b'subject,label\n' + TX.encode() + b',' + b'x' * 70000 + b'\n',
        'huge-quoted': b'subject,label\n' + TX.encode() + b',"' + b'x\n' * 33000 + b'"\n',
        'eleven': csv('subject,label', [f'{i:064x},Entity {i}' for i in range(11)]),
        'progress': csv('subject,label', [f'{i:064x},E' for i in range(4100)]),
        'quote-mid-field': csv('subject,label', [f'{TX},a"b']), 'quote-space': csv('subject,label', [f'{TX}, "x"']),
        'crlf-quoted': b'subject,label\r\n' + TX.encode() + b',"a\r\nb"\r\n',
        'doubled-quote': csv('subject,label', [f'{TX},"A ""quoted"" label"', f'{TX}:1,""""']),
    }
    for name, data in csv_cases.items():
        add('csv-' + name, data)
    rows = [
        [{'subject': TX, 'label': 'A'}], [{'address': ADDRESS, 'entity': 'B', 'original_category': 'Gambling'}],
        [{'hashadd': TX + ':3', 'name': 'C', 'category': 'P2P'}], [{'SUBJECT': TX, ' Label ': 'D'}],
        [{'subject': TX, 'label': 'A', 'chain': 'bitcoin', 'network': 'mainnet'}], [{'subject': TX, 'label': 'A', 'chain': 'liquid'}],
        [{'subject': TX, 'label': 'A', 'network': 'test'}], [{'subject': TX, 'label': 'A', 'chain': None, 'network': 'main'}],
        [{'subject': TX, 'label': 'A', 'chain': '', 'network': ''}], [{'subject': TX, 'label': 'A', 'chain': 'bitcoin', 'network': None}],
        [{'subject': 12, 'label': 'A'}], [{'subject': None, 'label': 'A'}], [{'label': 'A'}], [{'subject': TX}], [{'subject': TX, 'label': 5}],
        [{'subject': TX, 'label': 'A', 'category': 0}], [{'subject': TX, 'label': 'A', 'category': False}], [{'subject': TX, 'label': 'A', 'category': []}],
        [{'subject': TX, 'label': 'A', 'category': 'Unknown'}], [{'subject': TX, 'label': 'A', 'category': 'DarkNet'}], [{'subject': TX, 'label': 'A', 'category': 'self'}],
        [{'subject': TX, 'label': 'A', 'category': 'x' * 65}], [{'subject': TX, 'label': 'A', 'category': ' '}],
        [{'subject': TX, 'label': 'A', 'source': 'S', 'source_id': 'id', 'attribution_method': 'm', 'source_url': 'https://e.org',
          'observed_at': '2026-01-01', 'date_first_tx': '2020-01-01T00:00:00Z', 'valid_from': '2026-01-01', 'valid_until': '2027-01-01'}],
        [{'subject': TX, 'label': 'A', 'source': ''}], [{'subject': TX, 'label': 'A', 'source': None}], [{'subject': TX, 'label': 'A', 'source': 5}],
        [{'subject': TX, 'label': 'A', 'source_url': 'ftp://x'}], [{'subject': TX, 'label': 'A', 'observed_at': 'bad'}],
        [{'subject': TX, 'label': 'A', 'valid_from': '2027-01-01', 'valid_until': '2026-01-01'}],
        [{'subject': TX, 'label': '', 'valid_from': '2027-01-01', 'valid_until': '2026-01-01'}],
        [{'subject': 'not-a-subject', 'label': '', 'category': 'x' * 70}], [{'subject': 'bitcoin:test:' + TX, 'label': 'A'}],
        [{'subject': 'bitcoin:main:' + TX, 'label': 'A'}], [{'subject': 'script:0014' + '11' * 20, 'label': 'A'}],
        [{'subject': 'out:' + TX, 'label': 'A'}], [{'subject': '1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2', 'label': 'A'}],
        [{'subject': 'tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx', 'label': 'A'}], [{'subject': float('nan'), 'label': 'A'}],
        [{'subject': TX, 'label': 'A', 'extra': {'nested': [1, 2, {'x': None}]}}], [{'subject': TX, 'label': 'A\ud800'}],
        [{'subject': TX + '\ud800', 'label': 'A'}], [{'subject': ' ' + TX + ' ', 'label': ' A '}],
    ]
    for index, data in enumerate(rows):
        add(f'jsonl-{index}', jsonl(data), format='jsonl')
    jsonl_cases = {
        'blank': b'\n  \n' + jsonl([{'subject': TX, 'label': 'A'}]) + b'\n', 'malformed': b'{"subject":\n', 'array': b'[1]\n',
        'scalar': b'5\n', 'dup-key': b'{"subject":"x","subject":"' + TX.encode() + b'","label":"A"}\n',
        'dup-lower': b'{"Subject":"x","subject":"' + TX.encode() + b'","label":"A"}\n', 'too-many-keys': jsonl([{f'k{i}': 1 for i in range(65)}]),
        'long-key': jsonl([{'k' * 65: 1, 'subject': TX, 'label': 'A'}]), 'nan': b'{"subject":"' + TX.encode() + b'","label":"A","x":NaN}\n',
        'bom': b'\xef\xbb\xbf' + jsonl([{'subject': TX, 'label': 'A'}]), 'bom-second': jsonl([{'subject': TX, 'label': 'A'}]) + b'\xef\xbb\xbf{}\n',
        'deep': b'{"subject":"' + TX.encode() + b'","label":"A","x":' + b'[' * 50 + b']' * 50 + b'}\n',
        'no-newline': b'{"subject":"' + TX.encode() + b'","label":"A"}', 'empty-object': b'{}\n', 'only-blank': b'\n\n',
        'surrogate-key': b'{"\\ud800":1,"subject":"' + TX.encode() + b'","label":"A"}\n', 'huge': b'{"x":"' + b'y' * 70000 + b'"}\n',
        'invalid-utf8': b'{"subject":"\xff"}\n', 'big-int': b'{"subject":' + b'9' * 30 + b',"label":"A"}\n',
    }
    for name, data in jsonl_cases.items():
        add('jsonl-' + name, data, format='jsonl')
    maru = [
        [{'hashadd': TX, 'exchange': 'Kraken'}], [{'hashadd': TX, 'gambling': 'Dice', 'category': 'ignored'}],
        [{'hashadd': TX, 'mining': 'Pool', 'exchange': ''}], [{'hashadd': TX}], [{'hashadd': TX, 'exchange': 'A', 'service': 'B'}],
        [{'hashadd': TX, 'historic': 0}], [{'hashadd': TX, 'service': None, 'historic': 'Old'}],
    ]
    for index, data in enumerate(maru):
        add(f'maru92-{index}', jsonl(data), format='jsonl', adapter='maru92')
    add('maru92-csv', csv('hashadd,exchange,mining', [f'{TX},Kraken,', f'{TX}:1,,Pool']), adapter='maru92')
    add('am-i-exposed', csv('address,entity,original_category', [f'{ADDRESS},Wallet,Exchange']), adapter='am_i_exposed')
    from embit import ec, script
    from embit.liquid.addresses import address
    from embit.liquid.networks import NETWORKS
    sc = script.Script(bytes.fromhex('0014' + '45' * 20))
    blind = ec.PrivateKey(bytes.fromhex('12' * 32)).get_public_key()
    for network in ['liquidv1', 'liquidtestnet', 'elementsregtest']:
        for value in [address(sc, network=NETWORKS[network]), address(sc, blind, network=NETWORKS[network])]:
            add(f'liquid-{network}-{len(cases)}', csv('subject,label', [f'{value},L']), {**MANIFEST, 'chain': 'liquid', 'network': network})
    case = {'name': 'arguments', 'sql': '', 'actions': [
        {'operation': 'datasets.preview', 'args': {}, 'stream': good.hex()},
        {'operation': 'datasets.preview', 'args': {'manifest': MANIFEST}},
        {'operation': 'datasets.preview', 'args': {'manifest': MANIFEST, 'source_token': 'unknown'}},
        {'operation': 'datasets.preview', 'args': {'manifest': MANIFEST, 'expected_scope': {'workspace_id': 'ws', 'profile_id': 'p'}}, 'stream': good.hex()},
        {'operation': 'datasets.preview', 'args': {'manifest': MANIFEST, 'expected_scope': {'workspace_id': 'ws', 'profile_id': 'x'}}, 'stream': good.hex()},
        {'operation': 'datasets.preview.start', 'args': {'manifest': MANIFEST, 'source_token': 'x'}},
    ]}
    case['expected'] = evaluate(case)
    cases.append(case)
    return cases


if __name__ == '__main__':
    data = vectors()
    DEST.write_text(json.dumps(data, ensure_ascii=True, sort_keys=True, separators=(',', ':')) + '\n')
    print(f'{len(data)} cases, {sum(len(c["expected"]) for c in data)} states')
