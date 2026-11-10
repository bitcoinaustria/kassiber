import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3

import pytest

from kassiber.db import open_db
from tests import cli_runner
from tests.test_rust_report_exports import _book
from crates.reports.scripts.gen_source_funds_cases import dump
from crates.reports.scripts.gen_source_funds_reads import BASE

RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
pytestmark = pytest.mark.skipif(not RUST_BIN,reason='requires Rust test-harness binary')


def _compare(native,reference,daemon,debug):
    assert native.returncode == reference.returncode
    assert native.stderr == b''
    if daemon:
        assert b'synthetic-diagnostic-secret' not in native.stdout + native.stderr
    if daemon:
        actual = native.stdout.splitlines()[1:]
        expected = reference.stdout.splitlines()[1:]
    else:
        actual = [native.stdout]
        expected = [reference.stdout]
    assert len(actual) == len(expected)
    diagnostics = False
    for native_line,reference_line in zip(actual,expected,strict=True):
        a,e = json.loads(native_line),json.loads(reference_line)
        encode = lambda value: (json.dumps(value,separators=(',',':')) if daemon else json.dumps(value,indent=2)+'\n').encode()
        assert reference_line == encode(e)
        if e.get('kind') == 'error':
            field = e['error']['debug']
            required = e['error']['code'] == 'internal_error' if daemon else debug
            assert isinstance(field,str) if required else field is None
            assert a['error']['debug'] == ('' if required else None)
            if required:
                diagnostics = True
                assert field
                e['error']['debug'] = ''
        assert native_line == encode(e), (a, e)
    if diagnostics:
        assert b'Traceback (most recent call last):' in reference.stderr
    else:
        assert reference.stderr == b''


@pytest.mark.parametrize('encrypted',[False,True],ids=['sqlite','sqlcipher'])
@pytest.mark.parametrize('transport',['cli','daemon'])
@pytest.mark.parametrize('debug',[False,True],ids=['normal','debug'])
def test_source_funds_diagnostics_pin_wire_types_redaction_rollback_and_all_112_tables(tmp_path,monkeypatch,encrypted,transport,debug):
    root,database,env,call = _book(tmp_path,monkeypatch,'generic','FIFO')
    conn = open_db(root)
    conn.executescript(BASE)
    conn.commit()
    conn.close()
    conn = open_db(root)
    conn.execute('PRAGMA wal_checkpoint(TRUNCATE)')
    conn.execute('PRAGMA journal_mode=DELETE')
    conn.close()
    passphrase = 'synthetic diagnostic passphrase' if encrypted else None
    if encrypted:
        from sqlcipher3 import dbapi2
        connection = dbapi2.connect(database)
        exported = tmp_path/'encrypted.sqlite'
        connection.execute('ATTACH DATABASE ? AS encrypted KEY ?',(str(exported),passphrase))
        connection.execute("SELECT sqlcipher_export('encrypted')")
        connection.execute('DETACH DATABASE encrypted')
        connection.close()
        os.replace(exported,database)
    def connect():
        if encrypted:
            from sqlcipher3 import dbapi2 as driver
        else:
            driver = sqlite3
        connection = driver.connect(database)
        connection.row_factory = driver.Row
        if passphrase:
            connection.execute('PRAGMA key='+repr(passphrase))
        connection.execute('PRAGMA foreign_keys=ON')
        return connection
    conn = connect()
    before = dump(conn)
    conn.close()
    baseline = tmp_path/'baseline.sqlite'
    shutil.copy2(database,baseline)
    evidence = root/'not-read/secret.pdf'
    evidence.parent.mkdir()
    evidence.write_bytes(b'synthetic diagnostic evidence\x00\xff')
    before_file = evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest()
    def unchanged():
        conn = connect()
        assert dump(conn) == before
        conn.close()
        assert (evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest()) == before_file
    flags = ['--db-passphrase-fd',cli_runner.InputFd(passphrase.encode())] if passphrase else []
    if debug:
        flags += ['--debug']
    if transport == 'cli':
        operations = [
            ['sources','create','--type','income','--label','Origin','--amount','NaN'],
            ['sources','create','--type','income','--label','Origin','--amount','Infinity'],
            ['sources','create','--type','income','--label','Origin','--amount','1','--attachment','a-a','--attachment','missing'],
            ['sources','attach','--source','s-z','--attachment','missing'],
            ['links','create','--from-transaction','in','--to-transaction','out','--type','self_transfer','--allocation-amount','NaN'],
            ['links','create','--from-transaction','in','--to-transaction','out','--type','self_transfer','--attachment','a-a','--attachment','missing'],
            ['links','review','--link','l-z','--allocation-amount','NaN'],
            ['links','attach','--link','l-z','--attachment','missing'],
            ['recipients','update','--recipient','r-a','--label','ALPHA'],
            ['sources','attach','--source','api_key=synthetic-diagnostic-secret','--attachment','a-a'],
        ]
        for operation in operations:
            results = []
            for reference in [False,True]:
                shutil.copy2(baseline,database)
                results.append(call(*flags,'--machine','source-funds',*operation,reference=reference))
                unchanged()
            _compare(*results,False,debug)
        served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
        assert len(served) == len(operations)
        assert all(row['served_by'] == 'rust' for row in served)
    else:
        requests = [
            ('ui.source_funds.sources.create',dict(source_type='income',label='Origin',amount='NaN')),
            ('ui.source_funds.sources.create',dict(source_type='income',label='Origin',amount='Infinity')),
            ('ui.source_funds.sources.create',dict(source_type='income',label='Origin',attachment_ids=['a-a','missing'])),
            ('ui.source_funds.sources.attach',dict(source='s-z',attachment_id='missing')),
            ('ui.source_funds.links.create',dict(from_transaction='in',to_transaction='out',link_type='self_transfer',allocation_amount='NaN')),
            ('ui.source_funds.links.create',dict(from_transaction='in',to_transaction='out',link_type='self_transfer',attachment_ids=['a-a','missing'])),
            ('ui.source_funds.links.review',dict(link='l-z',allocation_amount='NaN')),
            ('ui.source_funds.links.review',dict(link='l-a',expected_allocation_amount_msat=float('inf'))),
            ('ui.source_funds.links.review',dict(link='l-a',expected_allocation_amount_msat=float('-inf'))),
            ('ui.source_funds.links.review',dict(link='l-a',expected_allocation_amount_msat=float('nan'))),
            ('ui.source_funds.links.attach',dict(link='l-z',attachment_id='missing')),
            ('ui.source_funds.recipients.update',dict(recipient='r-a',label='ALPHA')),
            ('ui.source_funds.sources.attach',dict(source='api_key=synthetic-diagnostic-secret',attachment_id='a-a')),
            ('ui.source_funds.evidence.list',dict(cursor='²')),
            ('ui.source_funds.evidence.list',dict(cursor='0'*4301)),
        ]
        wire = ''.join(json.dumps(dict(request_id=i,kind=kind,args=args))+'\n' for i,(kind,args) in enumerate(requests)).encode()+b'{"request_id":"done","kind":"daemon.shutdown"}\n'
        results = []
        for reference in [False,True]:
            shutil.copy2(baseline,database)
            results.append(call(*flags,'daemon',reference=reference,wire=wire))
            unchanged()
        _compare(*results,True,debug)
        assert [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()] == [dict(command='daemon',served_by='rust')]


def test_source_funds_debug_comparison_preserves_message_bytes():
    from types import SimpleNamespace
    envelope = dict(kind='error', error=dict(code='internal_error', message='reference message', debug='reference stack'))
    reference = SimpleNamespace(returncode=1, stdout=json.dumps(envelope, indent=2).encode()+b'\n', stderr=b'Traceback (most recent call last):')
    envelope['error'].update(message='changed message', debug='')
    native = SimpleNamespace(returncode=1, stdout=json.dumps(envelope, indent=2).encode()+b'\n', stderr=b'')
    with pytest.raises(AssertionError):
        _compare(native, reference, daemon=False, debug=True)
