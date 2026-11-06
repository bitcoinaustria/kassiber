import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess

import pytest

from kassiber.db import open_db
from tests.test_rust_report_exports import _book, _dump
from crates.reports.scripts.gen_platform_csv_vectors import reference
from tests.rust_port.book_schema import TABLES

ROOT = Path(__file__).resolve().parents[1]
SERVICE = os.environ.get('KASSIBER_XLSX_SERVICE_BIN')
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
CASES = json.loads((ROOT/'crates/reports/tests/data/platform_csv.json').read_text())


def state(root):
    tables = _dump(root,{})
    assert tuple(sorted(tables)) == TABLES
    return json.dumps(tables,sort_keys=True,separators=(',',':')).encode()


@pytest.mark.skipif(not SERVICE, reason='requires native report service probe')
@pytest.mark.parametrize('case', CASES, ids=lambda c:c['name'])
def test_platform_csv_service_bytes_metadata_and_all_112_tables(tmp_path,case):
    root=tmp_path/'data'
    conn=open_db(root)
    conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+case['sql'])
    conn.commit()
    database=Path(conn.execute('PRAGMA database_list').fetchone()[2])
    conn.close()
    before=database.read_bytes()
    target=tmp_path/'platform.csv'
    conn=sqlite3.connect(database)
    conn.row_factory=sqlite3.Row
    expected=reference(conn,target,json.loads(case['args_json']))
    conn.close()
    content=target.read_bytes() if target.exists() else None
    expected_state=state(root)
    database.write_bytes(before)
    target.unlink(missing_ok=True)
    request=dict(database=str(database),command='reports.export-platform-csv',args_json=case['args_json'],path=str(target))
    result=subprocess.run([SERVICE],input=json.dumps(request)+'\n',text=True,capture_output=True,check=True,timeout=120)
    assert result.stderr == ''
    actual=json.loads(result.stdout)
    assert json.dumps(actual,ensure_ascii=True).encode()==json.dumps(expected,ensure_ascii=True).encode()
    assert (target.read_bytes() if target.exists() else None)==content
    assert state(root)==expected_state


@pytest.mark.skipif(not RUST_BIN, reason='requires native harness')
@pytest.mark.parametrize('country,variant',[('generic','ready'),('at','ready'),('at','stale'),('at','raw-hex')])
def test_platform_csv_cli_bytes_and_native_routing(tmp_path,monkeypatch,country,variant):
    root,database,env,call=_book(tmp_path,monkeypatch,country,'MOVING_AVERAGE_AT' if country=='at' else 'FIFO',variant)
    conn=sqlite3.connect(database)
    conn.executescript("DELETE FROM journal_quarantines; DELETE FROM journal_entries WHERE id='swap';")
    conn.close()
    before=database.read_bytes()
    target=tmp_path/'platform.csv'
    options=[[],['--wallet','Hot'],['--wallet','Cold','--wallet','Hot'],['--from','2024-01-01','--to','2024-12-31'],['--period','last-year'],['--period','today','--from','2024-01-01'],['--wallet','missing'],['--from','']]
    def sequence(reference):
        database.write_bytes(before)
        outcomes=[]
        for platform in ['cointracking','blockpit']:
            for extra in options:
                target.unlink(missing_ok=True)
                result=call('--machine','transactions','export','--export-format',platform+'-csv','--file',str(target),*extra,reference=reference)
                if result.returncode==0:
                    payload=json.loads(result.stdout)['data']
                    assert 'report_snapshot' not in payload
                    assert payload['bytes']==target.stat().st_size
                outcomes.append((result.returncode,result.stdout,result.stderr,target.read_bytes() if target.exists() else None,state(root)))
        return outcomes
    assert sequence(False)==sequence(True)
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served==[dict(command='transactions.export',served_by='rust')]*16


@pytest.mark.skipif(not RUST_BIN, reason='requires native harness')
@pytest.mark.parametrize('blocked',[False,True,'empty'])
def test_platform_csv_daemon_bytes_collisions_validation_and_native_routing(tmp_path,monkeypatch,blocked):
    root,database,env,call=_book(tmp_path,monkeypatch,'at','MOVING_AVERAGE_AT')
    conn=sqlite3.connect(database)
    conn.executescript("DELETE FROM journal_entries WHERE id='swap';")
    if not blocked:
        conn.execute('DELETE FROM journal_quarantines')
    conn.commit()
    conn.close()
    if blocked == 'empty':
        blank=open_db(tmp_path/'blank')
        blank_database=Path(blank.execute('PRAGMA database_list').fetchone()[2])
        blank.close()
        database.write_bytes(blank_database.read_bytes())
    before=database.read_bytes()
    exports=tmp_path/'exports'
    args=[dict(platform=p,**a) for p in ['cointracking',' BLOCKPIT '] for a in [{},{},dict(wallet='Hot'),dict(wallets=['Cold','Hot']),dict(range='custom',**{'from':'2024-01-01','to':'2024-12-31','timezone':'Europe/Vienna'}),dict(wallets=[]),dict(wallet='Cold',wallets=[]),dict(range=''),dict(timezone=42),dict(wallets=42),dict(wallet=False),dict(wallet='missing'),dict(z=1,a=2)]]
    args += [{},{'platform':False},{'platform':'bad'}, {'platform':'bad','range':''},{'platform':'blockpit','workspace':'ignored'}]
    wire=b''.join((json.dumps(dict(request_id=i,kind='ui.transactions.export_platform_csv',args=a))+'\n').encode() for i,a in enumerate(args))
    wire += b'{"request_id":"done","kind":"daemon.shutdown"}\n'
    def session(reference):
        database.write_bytes(before)
        shutil.rmtree(exports,ignore_errors=True)
        reserved=exports/'reports/kassiber-cointracking-20260102-030405-2.csv'
        reserved.mkdir(parents=True)
        result=call('daemon',reference=reference,wire=wire)
        assert (result.returncode,result.stderr)==(0,b'')
        ready,*records=result.stdout.splitlines()
        if not reference:
            assert 'ui.transactions.export_platform_csv' in json.loads(ready)['data']['supported_kinds']
        artifacts={str(p.relative_to(exports)):p.read_bytes() for p in exports.rglob('*.csv') if p.is_file()}
        assert reserved.is_dir()
        return records,artifacts,state(root)
    assert session(False)==session(True)
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served==[dict(command='daemon',served_by='rust')]
