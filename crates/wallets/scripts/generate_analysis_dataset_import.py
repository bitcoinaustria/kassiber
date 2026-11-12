"""Unchanged-Python oracle for local attribution imports and incomplete-pack cleanup.

Sources are synthetic bytes held in the corpus; nothing is read from or sent to
anywhere else. Job-mode actions record the progress receipts and cancellation
that the daemon's background start variants report.
"""
from __future__ import annotations

import base64
import copy
import io
import json
from pathlib import Path
import sys
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_durable import capture
from generate_analysis_index import SEED_TABLE_COUNT
from generate_analysis_ops import NOW, Sources
from generate_analysis_watches import database
from kassiber.core import chain_analysis_api as api, chain_analysis_datasets as datasets
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError

DEST = ROOT / 'crates/wallets/tests/data/analysis_dataset_import.json'
A, B = 'a1' * 32, 'b2' * 32


def manifest(**kwargs):
    return {'dataset_key': 'example.pack', 'name': 'Example', 'version': '1', 'chain': 'bitcoin', 'network': 'main', 'source': 'Synthetic fixture',
            'license': 'CC0', 'attribution_method': 'declared', 'visibility': 'public', **kwargs}


def evaluate(case, conn, resolve=None):
    source = Sources()
    results = []
    for action in case['actions']:
        if resolve is not None and callable(action.get('args')):
            action['args'] = action['args'](results)
        operation = action['operation']
        progress = None
        if operation == 'sql':
            conn.executescript(action['sql'])
            value = None
        elif operation in {'begin', 'rollback'}:
            conn.execute('BEGIN') if operation == 'begin' else conn.rollback()
            value = None
        else:
            identity = SimpleNamespace(uuid4=source.label_uuid)
            with patch.object(datasets, 'now_iso', return_value=NOW), patch.object(datasets, 'uuid', identity):
                profile = case.get('profile', 'p')
                args = copy.deepcopy(action.get('args'))
                stream = io.BytesIO(base64.b64decode(action.get('source', '')))
                try:
                    if operation == 'datasets.import':
                        payload = api.dispatch(conn, 'ui.chain_analysis.datasets.import', args, profile=profile, source_stream=stream)
                    elif operation.endswith('.job'):
                        progress = []
                        limit = action.get('cancel_after')
                        cancelled = lambda: limit is not None and len(progress) >= limit
                        if operation == 'datasets.import.job':
                            payload = datasets.import_dataset(conn, profile, args['manifest'], stream, expected_sha256=args.get('expected_sha256'),
                                                              format=args.get('format', 'csv'), adapter=args.get('adapter', 'generic'),
                                                              progress=progress.append, cancelled=cancelled)
                        else:
                            payload = datasets.discard_dataset(conn, profile, args['id'], progress=progress.append, cancelled=cancelled)
                    else:
                        payload = api.dispatch(conn, 'ui.chain_analysis.' + operation, args, profile=profile)
                    value = build_envelope('ui.chain_analysis.' + operation.removesuffix('.job'), payload)
                except AppError as e:
                    value = build_error_envelope(e.code, str(e), e.details, e.hint, e.retryable)
                except (RuntimeError, OverflowError, UnicodeEncodeError, ValueError, TypeError) as e:
                    value = build_error_envelope('internal_error', str(e))
        state = capture(conn)
        assert state['table_count'] == SEED_TABLE_COUNT
        record = {'result': value, 'state': state, 'in_transaction': conn.in_transaction}
        if progress is not None:
            record['progress'] = progress
        results.append(record)
    return results


def data(results, index):
    value = results[index]['result']
    assert value['kind'] != 'error', value
    return value['data']


def text(rows):
    return base64.b64encode(rows.encode()).decode()


def vectors():
    corpus = []

    def add(name, actions, **kwargs):
        case = {'group': 'dataset-import', 'name': name, 'encrypted': False, 'sql': '', 'actions': actions, **kwargs}
        conn = database(case['sql'], case['encrypted'])
        try:
            case['expected'] = evaluate(case, conn, resolve=True)
        finally:
            conn.close()
        corpus.append(case)

    def act(op, args=None, source='', **kwargs):
        return {'operation': op, 'args': args, 'source': source, **kwargs}

    def imp(source, sha=None, **kwargs):
        args = {'manifest': manifest(**kwargs.pop('manifest', {})), 'expected_sha256': sha, **kwargs}
        return act('datasets.import', args, text(source) if isinstance(source, str) else source)

    csv = f'subject,label,category,source_url,observed_at\n{A},Exchange One,exchange,https://example.com/a,2024-01-02\n{B}:1,Mixer Two,mixing,,\n'
    preview = datasets.preview_dataset(manifest(), io.BytesIO(csv.encode()))
    sha = preview['sha256']
    add('csv-generic', [imp(csv, sha)])
    add('csv-without-hash', [imp(csv, None)])
    jsonl = '\n'.join(json.dumps(row) for row in [{'Subject': A, 'Label': 'Pool', 'Category': 'mining', 'valid_from': '2024-01-01', 'valid_until': '2025-01-01T00:00:00+02:00'},
                                                   {'address': A, 'entity': 'Svc', 'original_category': 'payment', 'chain': 'bitcoin', 'network': 'mainnet'}]) + '\n\n'
    add('jsonl', [imp(jsonl, datasets.preview_dataset(manifest(), io.BytesIO(jsonl.encode()), format='jsonl')['sha256'], format='jsonl')])
    maru = f'hashAdd,exchange,gambling,mining,service,historic\n{A},Kraken,,,,\n{B},,,Pool,,\n'
    add('maru92', [imp(maru, None, adapter='maru92')])
    quoted = f'﻿subject,label\r\n{A},"Line\r\nbreak, ""quoted"""\r\n'
    commas = f'subject,label\n{A},"Comma, ""quoted"""\n'
    add('quoted-bom-crlf', [imp(quoted, None), imp(commas, None, manifest={'version': '2'})])
    for i, bad in enumerate([f'subject,label\n{A},"open\n', f'subject,label\n{A},x"y\n', f'subject,label\n{A}\n', f'subject,label\n{A},x,extra\n', 'subject,subject\nx,y\n',
                             'label\nX\n', '', f'subject,label\n{A},\n', 'subject,label\nnot-a-subject,X\n', f'subject,label,chain\n{A},X,liquid\n',
                             f'subject,label,valid_from,valid_until\n{A},X,2025-01-01,2024-01-01\n', f'subject,label,source_url\n{A},X,ftp://bad\n',
                             f'subject,label\n{A},X\r{A},Y\n', b'\xff\xfe', f'subject,label\n{A},' + 'x' * 70000 + '\n']):
        add(f'malformed-{i}', [imp(bad if isinstance(bad, str) else base64.b64encode(bad).decode(), None)])
    add('jsonl-malformed', [imp('{"subject": 1}\n[1]\n', None, format='jsonl'), imp('{"a":1,"A":2}\n', None, format='jsonl'), imp('nope\n', None, format='jsonl')])
    for i, args in enumerate([{'manifest': manifest()}, {'manifest': manifest(), 'expected_sha256': 'A' * 64}, {'manifest': manifest(), 'expected_sha256': sha, 'format': 'xml'},
                              {'manifest': manifest(), 'expected_sha256': sha, 'adapter': 'other'}, {'manifest': {'name': 'x'}, 'expected_sha256': sha},
                              {'manifest': manifest(visibility='team'), 'expected_sha256': sha}, {'manifest': manifest(dataset_key='bad key'), 'expected_sha256': sha},
                              {'manifest': manifest(), 'expected_sha256': sha, 'unknown': 1}, [], None,
                              {'manifest': manifest(), 'expected_sha256': sha, 'expected_scope': {'workspace_id': 'ws', 'profile_id': 'other'}}]):
        add(f'arguments-{i}', [act('datasets.import', args, text(csv))])
    add('hash-changed', [imp(csv, 'f' * 64)])
    replaced = lambda r: {'manifest': manifest(version='2', expected_active_id=data(r, 0)['id']), 'expected_sha256': None}
    add('replacement', [imp(csv, sha), act('datasets.import', replaced, text(csv)), imp(csv, None, manifest={'version': '2'}),
                        act('datasets.import', lambda r: {'manifest': manifest(version='3', expected_active_id=data(r, 0)['id']), 'expected_sha256': None}, text(csv))])
    many = 'subject,label\n' + ''.join(f'{n:064x},Entity {n}\n' for n in range(2001))
    add('batched-job', [act('datasets.import.job', {'manifest': manifest()}, text(many))])
    add('cancelled-job', [act('datasets.import.job', {'manifest': manifest()}, text(many), cancel_after=1),
                          act('datasets.discard.job', {'id': '00000000-0000-4000-8000-000000000001'}),
                          act('datasets.discard', {'id': '00000000-0000-4000-8000-000000000001'})])
    add('cancelled-before-activation', [act('datasets.import.job', {'manifest': manifest()}, text(csv), cancel_after=1)])
    add('discard-job-cancel', [act('datasets.import.job', {'manifest': manifest()}, text(many), cancel_after=1),
                               act('datasets.discard.job', {'id': '00000000-0000-4000-8000-000000000001'}, cancel_after=1),
                               act('datasets.discard.job', {'id': '00000000-0000-4000-8000-000000000001'})])
    add('caller-busy', [{'operation': 'begin'}, imp(csv, sha), {'operation': 'rollback'}])
    add('missing-book', [act('datasets.import.job', {'manifest': manifest()}, text(csv))], profile='absent')
    return corpus


if __name__ == '__main__':
    DEST.write_text(json.dumps(vectors(), ensure_ascii=True, separators=(',', ':')) + '\n')
