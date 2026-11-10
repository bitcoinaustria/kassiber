"""R4-13's unregistered native pipeline against complete Python process dumps."""
from contextlib import ExitStack
import itertools
import json
import os
from pathlib import Path
import selectors
import shutil
import sqlite3
import struct
import subprocess
import tempfile
import uuid
from unittest.mock import patch

import pytest

from kassiber.cli import handlers
from kassiber.core import custody_journal, custody_filed_reports, filed_report_chain
from tests.rust_port.journal_persistence_oracle import CLOCK, dump
from tests.rust_port.journal_prebuild_oracle import rate, refusal
from tests.test_journal_storage_format import _book
from tests.rust_fixture_binary import checked

ROOT = Path(__file__).resolve().parents[1]
BINARY = checked(os.environ.get("KASSIBER_TEST_JOURNAL_PROCESS_BIN", ROOT / "target/debug/examples/journal-process"),
                 "cargo build -p kassiber-journals --example journal-process --features test-harness --offline --locked")
pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason="requires journal-process test-harness example")


@pytest.fixture
def prepared_book_root():
    parent = ROOT / 'target/r4-13/tests'
    parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='prepared-', dir=parent) as root:
        yield Path(root)


def scalar_exact(a, b):
    if type(a) is not type(b):
        return False
    if isinstance(a, float):
        return struct.pack('!d', a) == struct.pack('!d', b)
    if isinstance(a, list):
        return len(a) == len(b) and all(scalar_exact(x, y) for x, y in zip(a, b))
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(scalar_exact(a[k], b[k]) for k in a)
    return a == b


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("country", ["generic", "at"])
@pytest.mark.parametrize("mode", ["unchanged", "changed", "blocked", "empty", "cached-price", "legacy-price"])
def test_native_prepared_pipeline_matches_live_payload_progress_and_complete_dump(prepared_book_root, cipher, country, mode):
    tmp_path = prepared_book_root
    # The native probe explicitly accepts disposable books under target/ only.
    assert tmp_path.resolve().is_relative_to((ROOT / 'target').resolve())
    key = 'synthetic-r4-13-only' if cipher else None
    conn = _book(tmp_path / 'python', country=country, passphrase=key)
    try:
        if mode == 'changed':
            conn.execute("UPDATE transactions SET fiat_rate=12345,fiat_rate_exact='12345' WHERE id='a'")
        elif mode == 'blocked':
            conn.execute("UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL WHERE id='a'")
        elif mode == 'empty':
            conn.execute('DELETE FROM transactions')
        elif mode == 'cached-price':
            conn.execute("UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,"
                         "pricing_source_kind=NULL,pricing_quality=NULL,pricing_provider=NULL WHERE id='a'")
            rate(conn, timestamp='2024-01-01T00:00:00Z')
        elif mode == 'legacy-price':
            conn.execute("UPDATE transactions SET fiat_price_source='rates_cache',"
                         "pricing_source_kind=NULL,pricing_quality=NULL,pricing_provider=NULL WHERE id='a'")
        conn.commit()
        before = dump(conn)
        profile = dict(conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone())
        path = Path(conn.execute('PRAGMA database_list').fetchone()[2])
        conn.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchall()
        native = tmp_path / 'native.sqlite'
        shutil.copy2(path, native)
        ids = []
        counter = itertools.count(1)

        def identity():
            value = uuid.UUID(int=next(counter), version=4)
            ids.append(str(value))
            return value

        original_store = custody_journal.store_ledger_state
        original_price = handlers.auto_price_transactions_from_rates_cache

        def store(*args, **kwargs):
            with patch.object(uuid, 'uuid4', identity):
                return original_store(*args, **kwargs)

        def price(*args, **kwargs):
            # Resolution and both real repair stages must be no-ops. Pricing
            # is native for the two active cases and also checked on no-ops.
            assert scalar_exact(dump(conn), before)
            return original_price(*args, **kwargs)

        phases = []
        with ExitStack() as stack:
            for module in (custody_journal, custody_filed_reports, filed_report_chain):
                stack.enter_context(patch.object(module, 'now_iso', lambda: CLOCK))
            stack.enter_context(patch.object(custody_journal, 'store_ledger_state', store))
            stack.enter_context(patch.object(handlers, 'auto_price_transactions_from_rates_cache', price))
            expected = handlers.process_journals(conn, 'w', 'p', progress_observer=phases.append)
        request = dict(database=str(native), passphrase=key, native_prebuild=True,
                       profile=profile, clock=CLOCK, ids=ids, dump=True)
        result = subprocess.run([str(BINARY)], input=json.dumps(request) + '\n', text=True,
                                capture_output=True, check=True, timeout=120)
        assert not result.stderr
        actual = json.loads(result.stdout)
        assert actual['ok'], actual
        assert actual['payload'] == expected
        assert expected['auto_priced'] == (1 if mode == 'cached-price' else 0)
        assert [{'phase': p['phase']} for p in actual['phases']] == phases
        expected_dump = dump(conn)
        assert actual['dump'].keys() == expected_dump.keys()
        for table in expected_dump:
            assert scalar_exact(actual['dump'][table], expected_dump[table]), table
    finally:
        conn.close()


def _pricing_book(root, cipher):
    key = 'synthetic-r4-13-only' if cipher else None
    conn = _book(root, passphrase=key)
    conn.execute("UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,"
                 "pricing_source_kind=NULL,pricing_quality=NULL,pricing_provider=NULL WHERE id='a'")
    rate(conn, timestamp='2024-01-01T00:00:00Z')
    conn.commit()
    conn.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchall()
    path = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    profile = dict(conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone())
    request = dict(database=str(path), passphrase=key, native_prebuild=True,
                   profile=profile, clock=CLOCK,
                   ids=[str(uuid.UUID(int=i, version=4)) for i in range(1, 1000)], dump=True)
    return conn, request


def _native_vectors():
    from tests.rust_port.journal_native_process_oracle import PATH
    packed = json.loads(PATH.read_text())
    for case in packed['cases']:
        yield dict(case, **{
            side: {table: dict(data, rows=case[side].get(table, data['rows']))
                   for table, data in packed['base'].items()}
            for side in ('before', 'expected')})


def _native_replay(root, case, cipher, **options):
    from tests.rust_port.journal_pipeline_oracle import restore
    if cipher:
        import sqlcipher3 as driver
    else:
        driver = sqlite3
    path = root / 'native.sqlite3'
    conn = driver.connect(path)
    try:
        if cipher:
            conn.execute("PRAGMA key='synthetic-r4-13-only'")
        conn.row_factory = driver.Row
        restore(conn, case['before'])
        assert scalar_exact(dump(conn), case['before'])
    finally:
        conn.close()
    request = dict(database=str(path), passphrase='synthetic-r4-13-only' if cipher else None,
                   native_prebuild=True, profile=case['profile'], clock=case['clock'],
                   ticks=case['ticks'], ids=case['ids'], dump=True, **options)
    result = subprocess.run([str(BINARY)], input=json.dumps(request) + '\n', text=True,
                            capture_output=True, check=True, timeout=120)
    assert not result.stderr
    return json.loads(result.stdout)


def test_real_prebuild_process_vectors_rebuild_from_python():
    from tests.rust_port.journal_native_process_oracle import PATH, packed_vectors
    assert json.loads(PATH.read_text()) == packed_vectors()


def _native_vector_names():
    from tests.rust_port.journal_native_process_oracle import PATH
    return [case['name'] for case in json.loads(PATH.read_text())['cases']]


# One vector per test id, so xdist spreads the 115 replays (two SQLCipher
# key derivations each when encrypted) instead of one worker running them all.
@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('index,name', list(enumerate(_native_vector_names())),
                         ids=_native_vector_names())
def test_native_real_prebuild_replays_complete_books_and_clock_uuid_tapes(prepared_book_root, cipher, index, name):
    case = next(itertools.islice(_native_vectors(), index, None))
    assert case['name'] == name
    root = prepared_book_root / str(index)
    root.mkdir()
    actual = _native_replay(root, case, cipher)
    assert actual['ok'] == (case['error'] is None), case['name']
    if actual['ok']:
        assert actual['payload'] == case['result'], case['name']
    else:
        assert actual['refusal'] == case['error'], case['name']
    assert scalar_exact(actual['dump'], case['expected']), case['name']
    assert actual['clock_reads'] == len(case['ticks']), case['name']
    assert actual['uuid_reads'] == len(case['ids']), case['name']
    assert [{'phase': p['phase']} for p in actual['phases']] == case['phases'], case['name']
    assert actual['build_clock_reads'] == int({'phase': 'building'} in case['phases']), case['name']


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('table,column,payload', [
    ('transactions', 'note', b'\xff\0'),
    ('transactions', 'note', b''),
    ('transactions', 'raw_json', b'{"chain":"bitcoin","network":"main"}'),
    ('wallets', 'config_json', b'{}'),
])
def test_native_real_prebuild_preserves_blob_consumers_without_v1_encoding(
        prepared_book_root, cipher, table, column, payload):
    from tests.rust_port.journal_native_process_oracle import capture
    conn = _book(prepared_book_root / 'python')
    try:
        conn.execute(f'UPDATE "{table}" SET "{column}"=?', (payload,))
        conn.commit()
        profile = dict(conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone())
        case = capture(dict(name=f'blob-{table}-{column}', profile=profile), connection=conn)
        actual = _native_replay(prepared_book_root, case, cipher)
        assert actual['ok'] == (case['error'] is None)
        assert scalar_exact(actual.get('payload') if actual['ok'] else actual['refusal'],
                            case['result'] if case['error'] is None else case['error'])
        assert scalar_exact(actual['dump'], case['expected'])
        assert [{'phase': p['phase']} for p in actual['phases']] == case['phases']
        assert actual['clock_reads'] == len(case['ticks'])
        assert actual['uuid_reads'] == len(case['ids'])
    finally:
        conn.close()


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('write', [1, 8])
def test_native_real_prebuild_storage_failure_reverts_every_active_stage(prepared_book_root, cipher, write):
    case = next(case for case in _native_vectors() if case['name'] == 'all-stages')
    actual = _native_replay(prepared_book_root, case, cipher, fail_after_write=write)
    assert not actual['ok']
    assert actual['phases'][-1]['phase'] == 'storing'
    assert scalar_exact(actual['dump'], case['before'])


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('stage', ['pricing', 'building', 'storing'])
def test_real_pricing_pipeline_refusals_restore_complete_book(prepared_book_root, cipher, stage):
    conn, request = _pricing_book(prepared_book_root / 'failure', cipher)
    try:
        if stage == 'pricing':
            # A prior row is priced before this later row refuses.
            conn.execute("UPDATE transactions SET fiat_rate_exact='invalid-price',"
                         "fiat_price_source='rates_cache',pricing_source_kind=NULL,"
                         "pricing_quality=NULL WHERE id='b'")
        elif stage == 'building':
            conn.execute("UPDATE profiles SET gains_algorithm='invalid-method' WHERE id='p'")
        else:
            request['fail_after_write'] = 15
        conn.commit()
        request['profile'] = dict(conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone())
        conn.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchall()
        before = dump(conn)
        if stage != 'storing':
            with pytest.raises(Exception) as caught:
                handlers.process_journals(conn, 'w', 'p')
            expected_error = refusal(caught.value)
            assert scalar_exact(dump(conn), before)
        result = subprocess.run([str(BINARY)], input=json.dumps(request) + '\n', text=True,
                                capture_output=True, check=True, timeout=120)
        assert not result.stderr
        actual = json.loads(result.stdout)
        assert not actual['ok'], actual
        assert actual['phases'][-1]['phase'] == stage
        assert scalar_exact(actual['dump'], before)
        if stage != 'storing':
            assert actual['refusal'] == expected_error
    finally:
        conn.close()


def _open_copy(path, key):
    if key is None:
        conn = sqlite3.connect(path)
    else:
        import sqlcipher3
        conn = sqlcipher3.connect(str(path))
        conn.execute('PRAGMA key="synthetic-r4-13-only"')
    return conn


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('write', [1, 8, 15])
def test_killed_pipeline_writer_recovers_pricing_and_every_journal_table(prepared_book_root, cipher, write):
    conn, request = _pricing_book(prepared_book_root / 'killed', cipher)
    before = dump(conn)
    conn.close()
    request['pause_after_write'] = write
    with subprocess.Popen([str(BINARY)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True) as child:
        try:
            child.stdin.write(json.dumps(request) + '\n')
            child.stdin.flush()
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                assert selector.select(timeout=120), 'native writer did not reach its kill point'
            paused = json.loads(child.stdout.readline())
            assert paused.get('paused_at') == write, paused
            assert paused['table'].startswith('journal') or paused['table'].startswith('custody')
        finally:
            child.kill()
            child.wait(timeout=30)
        assert not child.stderr.read()
    recovered = _open_copy(request['database'], request['passphrase'])
    try:
        assert recovered.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        assert scalar_exact(dump(recovered), before)
    finally:
        recovered.close()
