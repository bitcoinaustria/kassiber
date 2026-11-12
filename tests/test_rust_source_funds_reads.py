"""Stored provenance read parity, complete database invariance and native routing."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess

import pytest

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_reads import BASE, outcome
from tests.test_rust_report_exports import _book, _dump
from tests.rust_port.book_schema import TABLES

ROOT = Path(__file__).resolve().parents[1]
SERVICE = os.environ.get('KASSIBER_XLSX_SERVICE_BIN')
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
CASES = json.loads((ROOT/'crates/reports/tests/data/source_funds_reads.json').read_text())


def state(root):
    tables = _dump(root, {})
    assert tuple(sorted(tables)) == TABLES
    return json.dumps(tables, sort_keys=True, separators=(',', ':')).encode()


@pytest.mark.skipif(not SERVICE, reason='requires native source-funds service probe')
@pytest.mark.parametrize('case', CASES, ids=lambda case: case['name'])
def test_source_funds_stored_service_bytes_and_all_112_tables(tmp_path, case):
    root = tmp_path/'data'
    conn = open_db(root)
    conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+case['sql'])
    conn.commit()
    database = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    conn.close()
    before = state(root)
    conn = sqlite3.connect(database)
    conn.row_factory = sqlite3.Row
    expected = outcome(conn, case['command'], json.loads(case['args_json']))
    conn.close()
    assert state(root) == before
    request = dict(database=str(database), command=case['command'], args_json=case['args_json'])
    result = subprocess.run([SERVICE], input=json.dumps(request)+'\n', capture_output=True, text=True, timeout=180, check=True)
    assert not result.stderr
    actual = json.loads(result.stdout)
    wire = lambda value: json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
    assert wire(actual) == wire(expected)
    assert wire(actual) == case['expected_json'].encode()
    assert state(root) == before


@pytest.mark.skipif(not SERVICE, reason='requires native source-funds service probe')
def test_source_funds_stored_sqlcipher_bytes_and_all_112_tables(tmp_path):
    from sqlcipher3 import dbapi2
    root = tmp_path/'encrypted'
    passphrase = 'synthetic-source-funds-passphrase'
    conn = open_db(root, passphrase=passphrase)
    conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE)
    conn.commit()
    database = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    conn.close()
    def connect():
        c = dbapi2.connect(database)
        c.execute('PRAGMA key='+repr(passphrase))
        c.executescript('PRAGMA cipher_compatibility=4;PRAGMA kdf_iter=256000;PRAGMA cipher_page_size=4096;')
        c.row_factory = dbapi2.Row
        return c
    def dump(c):
        tables = [row[0] for row in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
        assert tuple(sorted(tables)) == TABLES
        records = {table: sorted([dict(row) for row in c.execute(f'SELECT * FROM "{table}"')], key=lambda row: json.dumps(row, sort_keys=True)) for table in tables}
        return json.dumps(records, sort_keys=True, separators=(',', ':')).encode()
    conn = connect()
    before = dump(conn)
    commands = [(f'source-funds.{family}.list', {}) for family in ['sources','links','cases','recipients']]
    commands += [(f'ui.source_funds.{family}.list', {'include_inactive':True}) for family in ['sources','links','cases','recipients']]
    commands += [('ui.source_funds.evidence.list', {'limit':1, 'cursor':'1'}), ('ui.source_funds.links.list', {'target_transaction':'duplicate'})]
    expected = [outcome(conn, command, args) for command, args in commands]
    assert dump(conn) == before
    conn.close()
    requests = [dict(database=str(database), passphrase=passphrase, command=command, args_json=json.dumps(args)) for command,args in commands]
    result = subprocess.run([SERVICE], input=''.join(json.dumps(request)+'\n' for request in requests), capture_output=True, text=True, timeout=180, check=True)
    assert not result.stderr
    wire = lambda value: json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
    assert [wire(json.loads(line)) for line in result.stdout.splitlines()] == [wire(item) for item in expected]
    conn = connect()
    assert dump(conn) == before
    conn.close()


def book(tmp_path, monkeypatch, variant):
    root, database, env, call = _book(tmp_path, monkeypatch, 'generic', 'FIFO')
    conn = open_db(root)
    conn.executescript(BASE)
    if variant == 'stale':
        conn.execute("UPDATE profiles SET journal_input_version=999 WHERE id='p'")
    elif variant == 'source-blocked':
        conn.execute("INSERT INTO freshness_source_states(profile_id,source_type,source_key,source_label,status,blocking_reports,updated_at) VALUES('p','wallet','cold','Cold','error',1,'fixed')")
    elif variant == 'no-context':
        conn.execute("DELETE FROM settings WHERE key='context_profile'")
    conn.commit()
    conn.close()
    conn = open_db(root)
    conn.close()
    return root, database, env, call


@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('variant', ['ready', 'stale', 'source-blocked', 'no-context'])
def test_source_funds_stored_cli_bytes_output_files_all_tables_and_zero_fallbacks(tmp_path, monkeypatch, variant):
    root, _, env, call = book(tmp_path, monkeypatch, variant)
    before = state(root)
    count = 0
    families = ['sources', 'links', 'cases', 'recipients']
    for family in families:
        for flags in [[], ['--machine'], ['--format', 'json'], ['--format', 'table'], ['--format', 'plain'], ['--format', 'csv']]:
            native = call(*flags, 'source-funds', family, 'list')
            reference = call(*flags, 'source-funds', family, 'list', reference=True)
            assert (native.returncode, native.stdout, native.stderr) == (reference.returncode, reference.stdout, reference.stderr)
            assert state(root) == before
            count += 1
        destination = tmp_path/(family+'.json')
        args = ['--machine', '--output', str(destination), 'source-funds', family, 'list', '--profile', 'Book']
        native = call(*args)
        expected_file = destination.read_bytes() if destination.exists() else None
        destination.unlink(missing_ok=True)
        reference = call(*args, reference=True)
        assert (native.returncode, native.stdout, native.stderr) == (reference.returncode, reference.stdout, reference.stderr)
        assert (destination.read_bytes() if destination.exists() else None) == expected_file
        assert state(root) == before
        count += 1
    for extra in [[], ['--target-transaction', 'external-in'], ['--target-transaction', 'duplicate'], ['--target-transaction', 'missing'], ['--state', 'reviewed'], ['--state', 'rejected'], ['--state', 'suggested'], ['--profile', 'Other'], ['--profile', 'missing']]:
        args = ['--machine', 'source-funds', 'links', 'list', *extra]
        native = call(*args)
        reference = call(*args, reference=True)
        assert (native.returncode, native.stdout, native.stderr) == (reference.returncode, reference.stdout, reference.stderr)
        assert state(root) == before
        count += 1
    served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert len(served) == count == 37
    assert all(item['served_by'] == 'rust' and item['command'] in {f'source-funds.{f}.list' for f in families} for item in served)


@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('variant', ['ready', 'stale', 'source-blocked', 'no-context'])
def test_source_funds_stored_daemon_bytes_all_tables_and_zero_fallbacks(tmp_path, monkeypatch, variant):
    root, _, env, call = book(tmp_path, monkeypatch, variant)
    before = state(root)
    requests = []
    for family in ['sources', 'links', 'cases', 'recipients']:
        kind = f'ui.source_funds.{family}.list'
        for args in [None, {}, [], False, {'profile':'Other','workspace':'missing','unknown':True}, {'target_transaction':' external-in ','state':' accepted '}, {'target_transaction':'duplicate'}, {'target_transaction':'missing'}, {'state':'unknown'}, {'include_inactive':True}]:
            requests.append(dict(kind=kind, args=args))
    for args in [None, {}, {'limit':1}, {'limit':1,'cursor':'1'}, {'limit':1,'cursor':'2'}, {'cursor':'٣'}, {'cursor':'2147483648'}, {'limit':False}, {'limit':' ２ '}, {'limit':1.9}, {'unknown':1}, [], False]:
        requests.append(dict(kind='ui.source_funds.evidence.list', args=args))
    wire = b''.join((json.dumps(dict(request_id=i,**request))+'\n').encode() for i,request in enumerate(requests))+b'{"request_id":"done","kind":"daemon.shutdown"}\n'
    def session(reference):
        result = call('daemon', reference=reference, wire=wire)
        assert (result.returncode, result.stderr) == (0, b'')
        assert state(root) == before
        return result.stdout.splitlines()[1:]
    actual = session(False)
    expected = session(True)
    assert len(actual) == len(expected) == len(requests)+1
    for index, (a, e) in enumerate(zip(actual, expected)):
        assert a == e, (index, requests[index] if index<len(requests) else 'shutdown', a, e)
    served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served == [dict(command='daemon', served_by='rust')]


def native_debug_wire(line):
    envelope = json.loads(line)
    assert envelope['kind'] == 'error'
    assert envelope['error']['code'] == 'internal_error'
    debug = envelope['error']['debug']
    assert isinstance(debug, str)
    field = b'"debug":' + json.dumps(debug).encode()
    assert line.count(field) == 1
    return line.replace(field, b'"debug":"<native-stack>"', 1)


def test_source_funds_internal_debug_normalization_preserves_other_wire_bytes():
    envelope = dict(kind='error', schema_version=1, error=dict(
        code='internal_error', message='planted', hint=None, details={'planted':True},
        retryable=False, debug='stack'), request_id='cursor')
    wire = lambda value: json.dumps(value, separators=(',', ':')).encode()
    expected = native_debug_wire(wire(envelope))
    for key in ['message', 'hint', 'details', 'retryable']:
        changed = json.loads(wire(envelope))
        changed['error'][key] = 'changed'
        assert native_debug_wire(wire(changed)) != expected
    for key in ['schema_version', 'request_id']:
        changed = {**envelope, key:'changed'}
        assert native_debug_wire(wire(changed)) != expected
    for key, value in [('debug', None), ('code', 'validation')]:
        changed = json.loads(wire(envelope))
        changed['error'][key] = value
        with pytest.raises(AssertionError):
            native_debug_wire(wire(changed))
    assert native_debug_wire(wire(envelope).replace(b',', b', ', 1)) != expected


@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('cursor', ['²', '0'*4301], ids=['nondecimal-digit', 'oversized-integer'])
def test_source_funds_stored_internal_error_wire_parity(tmp_path, monkeypatch, cursor):
    root, _, env, call = book(tmp_path, monkeypatch, 'ready')
    before = state(root)
    requests = [dict(request_id='cursor', kind='ui.source_funds.evidence.list', args=dict(cursor=cursor)),
                dict(request_id='done', kind='daemon.shutdown')]
    wire = ''.join(json.dumps(request)+'\n' for request in requests).encode()
    native = call('daemon', wire=wire)
    assert state(root) == before
    reference = call('daemon', reference=True, wire=wire)
    assert state(root) == before
    served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served == [dict(command='daemon', served_by='rust')]
    from tests.test_rust_source_funds_diagnostics import _compare
    _compare(native, reference, daemon=True, debug=False)
