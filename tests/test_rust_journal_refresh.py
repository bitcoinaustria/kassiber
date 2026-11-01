"""Unregistered native refresh: exact result bytes, lifecycle tapes and full books."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess

import pytest

from tests.rust_port.journal_pipeline_oracle import restore
from tests.rust_port.journal_refresh_oracle import PATH, packed_vectors
from tests.rust_port.journal_persistence_oracle import dump
from tests.test_rust_journal_process import prepared_book_root, scalar_exact, _native_vectors

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get('KASSIBER_TEST_JOURNAL_PROCESS_BIN', ROOT / 'target/debug/examples/journal-process'))
pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason='requires journal-process test-harness example')


def cases():
    packed = json.loads(PATH.read_text())
    for case in packed['cases']:
        yield dict(case, **{
            side: {table: dict(data, rows=case[side].get(table, data['rows']))
                   for table, data in packed['base'].items()}
            for side in ('before', 'expected')})


def test_refresh_vectors_rebuild_from_the_python_reference():
    actual = json.dumps(packed_vectors(), ensure_ascii=True, separators=(',', ':'), allow_nan=False) + '\n'
    assert PATH.read_text() == actual


@pytest.mark.parametrize('cipher', [False, True])
def test_refresh_exact_ordered_results_phases_and_complete_books(prepared_book_root, cipher):
    if cipher:
        import sqlcipher3 as driver
    else:
        driver = sqlite3
    for index, case in enumerate(cases()):
        path = prepared_book_root / f'refresh-{index}.sqlite3'
        conn = driver.connect(path)
        try:
            if cipher:
                conn.execute("PRAGMA key='synthetic-r4-13-only'")
            conn.row_factory = driver.Row
            restore(conn, case['before'])
            for statement in case['schema_sql']:
                conn.execute(statement)
            conn.commit()
            assert scalar_exact(dump(conn), case['before'])
        finally:
            conn.close()
        request = dict(database=str(path), passphrase='synthetic-r4-13-only' if cipher else None,
                       native_prebuild=True, refresh=True, profile=case['profile'],
                       auto_pair=case['auto_pair'], skip_rebuild_when_current=case['skip_rebuild_when_current'],
                       clock=case['clock'], ticks=case['ticks'], ids=case['ids'], dump=True)
        result = subprocess.run([str(BINARY)], input=json.dumps(request) + '\n', text=True,
                                capture_output=True, check=True, timeout=120)
        assert not result.stderr
        actual = json.loads(result.stdout)
        assert actual['ok'] == (case['error'] is None), (case['name'], actual)
        if case['error'] is None:
            assert scalar_exact(actual['payload'], case['result']), case['name']
            assert actual['payload_json'] == case['payload_json'], case['name']
        else:
            assert actual['refusal'] == case['error'], case['name']
        assert [{'phase': p['phase']} for p in actual['phases']] == case['phases'], case['name']
        assert scalar_exact(actual['dump'], case['expected']), case['name']
        assert actual['clock_reads'] == len(case['ticks']), case['name']
        assert actual['uuid_reads'] == len(case['ids']), case['name']
        assert actual['build_clock_reads'] == case['precise_reads'], case['name']


def test_plain_process_preserves_the_complete_result_insertion_order():
    for case in _native_vectors():
        request = dict(before=case['before'], native_prebuild=True, profile=case['profile'],
                       clock=case['clock'], ticks=case['ticks'], ids=case['ids'])
        result = subprocess.run([str(BINARY)], input=json.dumps(request) + '\n', text=True,
                                capture_output=True, check=True, timeout=120)
        assert not result.stderr
        actual = json.loads(result.stdout)
        if case['error'] is None:
            assert actual['ok'], (case['name'], actual)
            assert actual['payload_json'] == json.dumps(case['result']), case['name']
        else:
            assert actual['refusal'] == case['error'], case['name']


def test_refresh_oracle_has_active_pairing_failure_and_commit_witnesses():
    corpus = list(cases())
    assert any(c['result'] and c['result'].get('auto_pair', {}).get('applied', 0) > 0 for c in corpus)
    assert any(sum(p['phase'] == 'building' for p in c['phases']) == 2 for c in corpus)
    assert any(c['result'] and c['result'].get('rebuilt') is False for c in corpus)
    forced = next(c for c in corpus if c['name'] == 'applied-pair-keeps-current-markers')
    assert forced['skip_rebuild_when_current'] is True
    assert forced['result']['auto_pair']['applied'] > 0
    assert forced['result']['rebuilt'] is True
    assert sum(p['phase'] == 'building' for p in forced['phases']) == 1
    for current in (0, 1):
        skipped = next(c for c in corpus if c['name'] == f'pairing-cache-failure-current{current}')
        assert skipped['result']['auto_pair']['skipped'] is True
        assert skipped['result']['auto_pair']['error']['code'] == 'auto_pair_failed'
        failed = next(c for c in corpus if c['name'] == f'second-rebuild-failure-current{current}')
        assert failed['error']['family'] == 'IntegrityError'
        assert (failed['before'] == failed['expected']) == bool(current)
