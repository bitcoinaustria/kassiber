import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

from kassiber.db import open_db
from tests.test_rust_report_exports import _book, _dump
from tests.test_rust_legacy_privacy import REFERENCE
from tests.rust_port.book_schema import TABLES

ROOT = Path(__file__).resolve().parents[1]
pytestmark = pytest.mark.skipif(not os.environ.get('KASSIBER_RUST_BIN'),reason='requires Rust test-harness binary')
KIND = 'ui.privacy_hygiene.snapshot'


def state(data):
    result = _dump(data,{})
    assert tuple(sorted(result)) == TABLES
    return REFERENCE.canonical(result).encode()


def routes(env, commands):
    rows = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert rows and all(row['served_by'] == 'rust' for row in rows),rows
    assert [row['command'] for row in rows] == commands,rows
    return len(rows)


def seed(tmp_path,monkeypatch,name,offline):
    data,_,env,call = _book(tmp_path,monkeypatch,'generic','FIFO')
    if offline:
        env['KASSIBER_NO_EGRESS'] = '1'
    else:
        env.pop('KASSIBER_NO_EGRESS',None)
    assert call('--machine','status',reference=True).returncode == 0
    case = next(case for case in json.loads(REFERENCE.DEST.read_text()) if case['name'] == name)
    conn = open_db(data)
    conn.executescript(case['sql'])
    conn.commit()
    conn.close()
    conn = open_db(data)
    conn.close()
    return data,env,call


@pytest.mark.parametrize('offline',[False,True])
def test_legacy_daemon_bytes_all_tables_zero_fallback(tmp_path,monkeypatch,offline):
    total = 0
    names = ['common-input-5','inventory-51','other-marker-1-main-' + REFERENCE.TXID,
        'cached-low-r-and-sighash','ordering-limit-10','empty-profile']
    for index,name in enumerate(names):
        root = tmp_path / str(index)
        root.mkdir()
        with pytest.MonkeyPatch.context() as patch:
            data,env,call = seed(root,patch,name,offline)
            before = state(data)
            values = [None,{}, {'wallet':'cold'},{'wallet':'Cold'},{'transaction':'legacy'},
                {'transaction':REFERENCE.TXID.upper()},{'limit':1},{'limit':'٢'},
                {'limit':200},{'limit':201},{'limit':0},{'limit':False},{'limit':'bad'}, {'limit':{'z':2,'a':1}}, {'limit':[]},
                {'wallet':'missing'},{'transaction':'missing'},{'z':1,'a':2},[],False,'',3,
                {'wallet':{'z':2,'a':1}}, {'transaction':[2,1]}]
            requests = [dict(kind=KIND,request_id=i,args=value) for i,value in enumerate(values,1)]
            requests.extend([
                dict(kind=KIND,request_id='scope-match',args={'expected_scope':{'workspace_id':'ws','profile_id':'p'}}),
                dict(kind=KIND,request_id='scope-stale',args={'expected_scope':{'workspace_id':'ws','profile_id':'wrong'}}),
                dict(kind=KIND,request_id=None),dict(kind=KIND),
            ])
            wire = b''.join((json.dumps(request)+'\n').encode() for request in requests)
            expected = call('daemon',reference=True,wire=wire)
            assert state(data) == before
            actual = call('daemon',wire=wire)
            assert actual.returncode == expected.returncode == 0,(name,actual.stderr,expected.stderr)
            assert actual.stderr == expected.stderr == b''
            native = actual.stdout.splitlines(keepends=True)
            python = expected.stdout.splitlines(keepends=True)
            assert KIND in json.loads(native[0])['data']['supported_kinds']
            assert len(native) == len(python) == len(requests)+1
            assert native[1:] == python[1:],(name,actual.stdout,expected.stdout)
            for line in native[1:]:
                response = json.loads(line)
                if 'data' in response:
                    assert response['data']['meta']['engine'] == 'privacy_hygiene_phase1'
                else:
                    assert response['error']['code'] in {'validation','not_found','stale_context'}
            assert state(data) == before,name
            assert routes(env,['daemon']) == 1
            total += len(requests)
    print(json.dumps({'offline':offline,'legacy_daemon_requests':total,'rust_daemon_sessions':len(names),'python_fallbacks':0,'tables_per_comparison':len(TABLES)},sort_keys=True))


@pytest.mark.parametrize('offline',[False,True])
def test_legacy_registration_preserves_configuration_report_cli(tmp_path,monkeypatch,offline):
    data,env,call = seed(tmp_path,monkeypatch,'common-input-5',offline)
    before = state(data)
    count = 0
    for flags in [('--machine',),('--format','json'),('--format','table'),('--format','plain'),('--format','csv')]:
        args = (*flags,'reports','privacy-hygiene')
        expected = call(*args,reference=True)
        actual = call(*args)
        assert (actual.returncode,actual.stdout,actual.stderr) == (expected.returncode,expected.stdout,expected.stderr)
        assert state(data) == before
        count += 1
    output = tmp_path / 'configuration.json'
    args = ('--machine','--output',str(output),'reports','privacy-hygiene')
    expected = call(*args,reference=True)
    saved = output.read_bytes()
    actual = call(*args)
    assert (actual.returncode,actual.stdout,actual.stderr) == (expected.returncode,expected.stdout,expected.stderr)
    assert output.read_bytes() == saved
    assert state(data) == before
    count += 1
    assert routes(env,['reports.privacy-hygiene']*count) == count
    print(json.dumps({'offline':offline,'reports.privacy-hygiene':count,'python_fallbacks':0,'tables_per_comparison':len(TABLES)},sort_keys=True))


@pytest.mark.parametrize('debug',[False,True])
def test_legacy_corrupt_amount_uses_native_unhandled_diagnostics(tmp_path,monkeypatch,debug):
    data,env,call = seed(tmp_path,monkeypatch,'invalid-amount-bad-None',False)
    before = state(data)
    requests = [dict(kind=KIND,request_id=1,args={'unknown':True}),dict(kind=KIND,request_id=2),dict(kind=KIND,request_id=3,args=False)]
    wire = b''.join((json.dumps(request)+'\n').encode() for request in requests)
    flags = ('--debug',) if debug else ()
    expected = call(*flags,'daemon',reference=True,wire=wire)
    actual = call(*flags,'daemon',wire=wire)
    assert actual.returncode == expected.returncode == 0
    native = actual.stdout.splitlines(keepends=True)
    python = expected.stdout.splitlines(keepends=True)
    assert len(native) == len(python) == 4
    assert native[1] == python[1] and native[3] == python[3]
    a,b = json.loads(native[2]),json.loads(python[2])
    assert a['error']['code'] == b['error']['code'] == 'internal_error'
    assert a['error']['message'] == b['error']['message'] == "invalid literal for int() with base 10: 'bad'"
    assert a['error']['debug'] == ''
    assert isinstance(b['error']['debug'],str) and 'ValueError' in b['error']['debug']
    debug_bytes = json.dumps(b['error']['debug'],ensure_ascii=False).encode()
    assert python[2].count(debug_bytes) == 1
    assert native[2] == python[2].replace(debug_bytes,b'""',1)
    assert expected.stderr.startswith(b'Traceback (most recent call last):\n')
    assert expected.stderr.endswith(b"ValueError: invalid literal for int() with base 10: 'bad'\n")
    assert actual.stderr == b''
    assert state(data) == before
    assert routes(env,['daemon']) == 1
    print(json.dumps({'debug':debug,'native_unhandled_sessions':1,'legacy_daemon_requests':3,'python_fallbacks':0,'tables_per_comparison':len(TABLES)},sort_keys=True))


def test_legacy_transport_mutations_apply_and_name_process_killers(tmp_path):
    result = subprocess.run(
        [sys.executable, str(ROOT / "crates/reports/scripts/legacy_privacy_transport_mutation_check.py"), "--check"],
        capture_output=True, check=True, timeout=30,
    )
    entries = json.loads((ROOT / "crates/reports/scripts/legacy_privacy_transport_mutations.json").read_text())
    assert result.stdout == f"{len(entries)} entries apply\n".encode()
    source = (ROOT / "crates/kassiber/tests/legacy_privacy_transport.rs").read_text()
    for entry in entries:
        assert all(f"fn {name}()" in source for name in entry["expect_failing"])
