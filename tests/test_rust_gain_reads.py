import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys

import pytest

from kassiber.core.report_explanation import result_reference
from kassiber.core.custody_journal import projection_freshness
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, open_db
from crates.reports.scripts.gen_gain_vectors import BASE, outcome
from tests import cli_runner
from tests.test_rust_report_exports import _dump, REFERENCE, STAMP
from tests.rust_port.book_schema import TABLES

ROOT=Path(__file__).resolve().parents[1]
SERVICE=os.environ.get('KASSIBER_XLSX_SERVICE_BIN')
RUST_BIN=os.environ.get('KASSIBER_RUST_BIN')
CASES=json.loads((ROOT/'crates/reports/tests/data/gain_reads.json').read_text())


def state(root):
    return json.dumps(_dump(root,{}),sort_keys=True,separators=(',',':')).encode()


@pytest.mark.skipif(not SERVICE,reason='requires native service probe')
@pytest.mark.parametrize('case',CASES,ids=lambda c:c['name'])
def test_gain_service_bytes_and_every_database_table(tmp_path,case):
    root=tmp_path/'data';conn=open_db(root)
    conn.executescript(((ROOT/'crates/reports/tests/data/book.sql').read_text() if case.get('base',True) else '')+case['sql']);conn.commit()
    database=Path(conn.execute('PRAGMA database_list').fetchone()[2]);conn.close()
    before=database.read_bytes()
    conn=sqlite3.connect(database);conn.row_factory=sqlite3.Row
    expected=outcome(conn,case['command'],json.loads(case['args_json']));conn.close()
    expected_state=state(root)
    database.write_bytes(before)
    request=dict(database=str(database),command=case['command'],args_json=case['args_json'])
    result=subprocess.run([SERVICE],input=json.dumps(request)+'\n',capture_output=True,text=True,timeout=180,check=True)
    assert not result.stderr
    actual=json.loads(result.stdout)
    assert json.dumps(actual,sort_keys=True,separators=(',',':')).encode()==json.dumps(expected,sort_keys=True,separators=(',',':')).encode()
    assert state(root)==expected_state
    assert tuple(sorted(_dump(root,{})))==TABLES


def book(tmp_path,monkeypatch,variant='ready'):
    root=tmp_path/'data';conn=open_db(root)
    conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE)
    database=Path(conn.execute('PRAGMA database_list').fetchone()[2]);conn.close()
    conn=open_db(root)
    conn.execute("UPDATE profiles SET last_processed_at=?,last_processed_tx_count=4,last_processed_input_version=journal_input_version WHERE id='p'",(STAMP,))
    conn.execute("UPDATE profiles SET journal_semantics_version=? WHERE id='p'",
                 (JOURNAL_CALCULATION_SEMANTICS_VERSION,))
    assert projection_freshness(conn, 'p')['is_current']
    if variant=='at':
        conn.execute("UPDATE profiles SET tax_country='at',gains_algorithm='HIFO' WHERE id='p'")
    if variant=='legacy':
        from crates.reports.scripts.gen_gain_vectors import stored
        conn.executescript(stored(compact=False))
    if variant=='stale':
        conn.execute("UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p'")
    if variant=='no-context':
        conn.execute("DELETE FROM settings WHERE key='context_profile'")
    profile=conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
    reference=result_reference(conn,profile,'z');conn.commit();conn.close()
    owners=tmp_path/'owners';owners.mkdir(mode=0o700)
    env={**os.environ,'KASSIBER_RUST_TEST_OWNER_ROOT':str(owners),'KASSIBER_RUST_TEST_REPORT_TIME':STAMP,
        'KASSIBER_HYBRID_RUST_BIN':str(Path(RUST_BIN).resolve()),'KASSIBER_HYBRID_LOG':str(tmp_path/'served.jsonl'),
        'KASSIBER_HYBRID_RUST_DAEMON':'1','KASSIBER_TEST_NO_EGRESS':'strict','KASSIBER_DISABLE_UPDATE_CHECK':'1',
        'KASSIBER_OFFLINE_PREFERENCE_FILE':str(tmp_path/'offline.json'),'XDG_CONFIG_HOME':str(tmp_path/'config'),
        'XDG_DATA_HOME':str(tmp_path/'xdg-data'),'XDG_STATE_HOME':str(tmp_path/'state')}
    monkeypatch.setenv('KASSIBER_TEST_CLI',json.dumps([sys.executable,str(ROOT/'tests/rust_port/hybrid_cli.py')]))
    original=cli_runner.prefix
    code=REFERENCE.replace('reports.datetime = FixedDateTime','from kassiber.core import ui_snapshot\nui_snapshot.datetime = FixedDateTime\nreports.datetime = FixedDateTime')
    monkeypatch.setattr(cli_runner,'prefix',lambda *,reference=False,python_options=():
        (sys.executable,'-c',code) if reference else original(python_options=python_options))
    def call(*args,reference=False,wire=None):
        return cli_runner.run(['--data-root',root,*args],reference=reference,env=env,input=wire,capture_output=True,timeout=180)
    return root,database,reference,env,call


@pytest.mark.skipif(not RUST_BIN,reason='requires Rust test-harness binary')
@pytest.mark.parametrize('variant',['ready','legacy','at','stale'])
def test_gain_cli_bytes_all_tables_and_zero_fallbacks(tmp_path,monkeypatch,variant):
    root,database,reference,env,call=book(tmp_path,monkeypatch,variant)
    before=database.read_bytes()
    references=[reference,{**reference,'input_version':-1},{**reference,'entry_id':'missing'},None,{},[],{**reference,'input_version':True}]
    def sequence(is_reference):
        database.write_bytes(before);result=[]
        for ref in [*map(json.dumps,references),'broken']:
            for flags in [[],['--machine'],['--format','json'],['--format','table'],['--format','plain'],['--format','csv']]:
                item=call(*flags,'reports','explain-capital-gain','--reference',ref,reference=is_reference)
                result.append((item.returncode,item.stdout,item.stderr,state(root)))
        return result
    assert sequence(False)==sequence(True)
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert len(served)==48
    assert all(c==dict(command='reports.explain-capital-gain',served_by='rust') for c in served)


@pytest.mark.skipif(not RUST_BIN,reason='requires Rust test-harness binary')
@pytest.mark.parametrize('variant',['ready','legacy','at','stale','no-context'])
def test_gain_daemon_bytes_all_tables_and_zero_fallbacks(tmp_path,monkeypatch,variant):
    root,database,reference,env,call=book(tmp_path,monkeypatch,variant)
    before=database.read_bytes()
    requests=[]
    for args in [None,{},[],1,dict(reference=reference),dict(reference={**reference,'entry_id':'missing'}),dict(reference={**reference,'processed_at':'old'}),dict(reference={**reference,'input_version':True}),dict(reference=reference,workspace='ignored',extra=1)]:
        requests.append(dict(kind='ui.reports.explain_capital_gain',args=args))
    wire=b''.join((json.dumps(dict(request_id=i,**r))+'\n').encode() for i,r in enumerate(requests))+b'{"request_id":"done","kind":"daemon.shutdown"}\n'
    def session(is_reference):
        database.write_bytes(before)
        result=call('daemon',reference=is_reference,wire=wire)
        assert (result.returncode,result.stderr)==(0,b'')
        return result.stdout.splitlines()[1:],state(root)
    actual=session(False);expected=session(True)
    assert len(actual[0])==len(expected[0])
    for index,(a,e) in enumerate(zip(actual[0],expected[0])):
        assert a==e,(index,a,e)
    assert actual[1]==expected[1]
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served==[dict(command='daemon',served_by='rust')]
