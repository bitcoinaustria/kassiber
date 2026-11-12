"""Unregistered native refresh: exact result bytes, lifecycle tapes and full books."""
import itertools
import json
import os
from pathlib import Path
import selectors
import sqlite3
import subprocess

import pytest

from tests.rust_port.fixture_compare import assert_rebuilt
from tests.rust_port.journal_pipeline_oracle import restore
from tests.rust_port.journal_refresh_oracle import PATH, packed_vectors
from tests.rust_port.journal_persistence_oracle import dump
from tests.test_rust_journal_process import prepared_book_root, scalar_exact, _native_vectors
from tests.rust_fixture_binary import checked

ROOT = Path(__file__).resolve().parents[1]
BINARY = checked(os.environ.get('KASSIBER_TEST_JOURNAL_PROCESS_BIN', ROOT / 'target/debug/examples/journal-process'),
                 'cargo build -p kassiber-journals --example journal-process --features test-harness --offline --locked')
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
    assert_rebuilt(PATH.read_text(), actual, PATH.name)


def _case_names():
    return [case['name'] for case in json.loads(PATH.read_text())['cases']]


# One case per test id, so xdist spreads the replays across workers.
@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('index,name', list(enumerate(_case_names())), ids=_case_names())
def test_refresh_exact_ordered_results_phases_and_complete_books(prepared_book_root, cipher, index, name):
    if cipher:
        import sqlcipher3 as driver
    else:
        driver = sqlite3
    case = next(itertools.islice(cases(), index, None))
    assert case['name'] == name
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


def _restore_refresh(path, case, cipher):
    if cipher:
        import sqlcipher3 as driver
    else:
        driver = sqlite3
    conn = driver.connect(path)
    if cipher:
        conn.execute("PRAGMA key='synthetic-r4-13-only'")
    conn.row_factory = driver.Row
    conn.execute('PRAGMA journal_mode=' + ('DELETE' if cipher else 'WAL'))
    restore(conn, case['before'])
    for sql in case['schema_sql']:
        conn.execute(sql)
    conn.commit()
    return conn


def _refresh_request(path, case, cipher, **options):
    return dict(database=str(path), passphrase='synthetic-r4-13-only' if cipher else None,
                native_prebuild=True, refresh=True, profile=case['profile'], auto_pair=True,
                clock=case['clock'], ticks=case['ticks'], ids=case['ids'], dump=True, **options)


def _refresh_probe(request):
    result = subprocess.run([str(BINARY)], input=json.dumps(request) + '\n', text=True,
                            capture_output=True, check=True, timeout=120)
    assert not result.stderr
    return json.loads(result.stdout)


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('current', [False, True])
def test_reference_second_rebuild_failure_retains_exact_first_commit(prepared_book_root, cipher, current):
    from tests.rust_port.journal_refresh_oracle import run
    case = next(c for c in cases() if c['name'] == f'second-rebuild-failure-current{int(current)}')
    conn = _restore_refresh(prepared_book_root / 'reference.sqlite3', case, cipher)
    try:
        actual = run(conn, case['name'], schema_sql=case['schema_sql'])
        assert actual['error'] == case['error']
        assert scalar_exact(actual['expected'], case['expected'])
        assert scalar_exact(dump(conn), case['expected'])
        assert (scalar_exact(case['expected'], case['before'])) == current
        assert not conn.in_transaction
        assert conn.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
    finally:
        conn.close()


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('current', [False, True])
def test_refresh_pairing_faults_at_each_pass_match_reference_books_and_skipped_summary(prepared_book_root, cipher, current):
    from tests.rust_port.journal_refresh_oracle import run
    case = next(c for c in cases() if c['name'] == f'exact-current{int(current)}-skip0')
    for clock in range(1, 5):
        conn = _restore_refresh(prepared_book_root / f'python-{clock}.sqlite3', case, cipher)
        try:
            expected = run(conn, f'pairing-clock-{clock}', fail_pairing_clock=clock)
            assert expected['error'] is None
            assert expected['result']['auto_pair']['skipped'] is True
            assert not conn.in_transaction
        finally:
            conn.close()
        path = prepared_book_root / f'native-{clock}.sqlite3'
        _restore_refresh(path, case, cipher).close()
        request = _refresh_request(path, expected, cipher, fail_precise=clock + int(not current))
        actual = _refresh_probe(request)
        assert actual['ok'], actual
        assert actual['payload_json'] == expected['payload_json']
        assert scalar_exact(actual['dump'], expected['expected'])
        assert [{'phase': p['phase']} for p in actual['phases']] == expected['phases']
        assert actual['build_clock_reads'] == expected['precise_reads']
        assert actual['clock_reads'] == len(expected['ticks'])
        assert actual['uuid_reads'] == len(expected['ids'])


def _before_recount(prepared_book_root, case, cipher):
    """Python's book at the final rebuild's commit boundary, and the precise
    clock reads the recount after it takes.

    Since #707 the refresh recounts the cached transfer candidates in its own
    commit after the rebuild (`_recount_transfer_candidates`), so a kill
    between the two keeps the rebuild without the recount's write.
    """
    from unittest.mock import patch
    from tests.rust_port.journal_refresh_oracle import run
    conn = _restore_refresh(prepared_book_root / 'before-recount.sqlite3', case, cipher)
    try:
        with patch('kassiber.daemon_freshness._recount_transfer_candidates'):
            boundary = run(conn, case['name'])
        assert boundary['error'] is None
        return dump(conn), case['precise_reads'] - boundary['precise_reads']
    finally:
        conn.close()


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('current', [False, True])
@pytest.mark.parametrize('mode', ['fault', 'kill'])
def test_refresh_recovers_every_write_and_stage_to_python_commit_boundary(prepared_book_root, cipher, current, mode):
    corpus = list(cases())
    case = next(c for c in corpus if c['name'] == f'exact-current{int(current)}-skip0')
    partial = next(c for c in corpus if c['name'] == f'second-rebuild-failure-current{int(current)}')['expected']
    path = prepared_book_root / 'recovery.sqlite3'

    def reset():
        for suffix in ('', '-wal', '-shm', '-journal'):
            Path(str(path) + suffix).unlink(missing_ok=True)
        _restore_refresh(path, case, cipher).close()
        return path.read_bytes()

    def recovered(expected, initial):
        if cipher:
            import sqlcipher3 as driver
        else:
            driver = sqlite3
        conn = driver.connect(path)
        try:
            if cipher:
                conn.execute("PRAGMA key='synthetic-r4-13-only'")
            conn.row_factory = driver.Row
            assert conn.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
            assert scalar_exact(dump(conn), expected)
            assert not conn.in_transaction
        finally:
            conn.close()
        if expected is case['before']:
            assert path.read_bytes() == initial

    reset()
    baseline = _refresh_probe(_refresh_request(path, case, cipher))
    assert baseline['ok']
    assert scalar_exact(baseline['dump'], case['expected'])
    if current:
        first_writes = 0
    else:
        reset()
        request = _refresh_request(path, case, cipher)
        request['refresh'] = False
        first = _refresh_probe(request)
        assert first['ok']
        assert scalar_exact(first['dump'], partial)
        first_writes = first['write_count']
        assert first_writes > 0
    assert baseline['write_count'] > first_writes

    controls = [(dict(**{('fail' if mode == 'fault' else 'pause') + '_after_write': write}),
                 case['before'] if write <= first_writes or current else partial)
                for write in range(1, baseline['write_count'] + 1)]
    if mode == 'kill':
        before_recount, recount_reads = _before_recount(prepared_book_root, case, cipher)
        counts = {}
        committed = case['before']
        for phase in baseline['phases']:
            name = phase['phase']
            counts[name] = counts.get(name, 0) + 1
            if name == 'complete':
                # The final rebuild commits before the candidate recount does.
                committed = before_recount if current or counts[name] == 2 else partial
            controls.append((dict(pause_phase=name, pause_phase_occurrence=counts[name]), committed))
        # The recount's own clock reads follow the final rebuild's commit.
        rebuild_reads = baseline['build_clock_reads'] - recount_reads
        controls.extend((dict(pause_precise=precise),
                         before_recount if precise > rebuild_reads
                         else case['before'] if current or precise == 1 else partial)
                        for precise in range(1, baseline['build_clock_reads'] + 1))

    for control, expected in controls:
        initial = reset()
        request = _refresh_request(path, case, cipher, **control)
        if mode == 'fault':
            actual = _refresh_probe(request)
            assert not actual['ok'], control
            assert actual['refusal']['family'] == 'DatabaseError', control
            assert scalar_exact(actual['dump'], expected), control
        else:
            with subprocess.Popen([str(BINARY)], text=True, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, cwd=ROOT) as child:
                try:
                    child.stdin.write(json.dumps(request) + '\n')
                    child.stdin.flush()
                    with selectors.DefaultSelector() as selector:
                        selector.register(child.stdout, selectors.EVENT_READ)
                        assert selector.select(timeout=120), control
                    record = json.loads(child.stdout.readline())
                    assert any(key.startswith('paused_') for key in record), (control, record)
                finally:
                    child.kill()
                    child.wait(timeout=30)
                assert not child.stderr.read()
        recovered(expected, initial)
