"""Cross-check service, artifact and real transport parity with the Python oracle."""
import base64
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
from unittest.mock import patch

import pytest

from kassiber.db import open_db
from crates.reports.scripts.gen_cross_check_vectors import outcome, encode, ct, bp
from tests.rust_port.xlsx import RecordingWorkbook, design_content
from tests.test_rust_report_exports import _dump
from tests.test_rust_xlsx_exports import _wire
from tests.test_rust_austrian_xlsx import _clocked_book
from tests.rust_port.book_schema import TABLES

ROOT = Path(__file__).resolve().parents[1]
SERVICE = os.environ.get('KASSIBER_XLSX_SERVICE_BIN')
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
CASES = json.loads((ROOT/'crates/reports/tests/data/cross_check.json').read_text())


def state(root):
    result = _dump(root,{})
    assert tuple(sorted(result))==TABLES
    return json.dumps(result,sort_keys=True,separators=(',',':')).encode()


@pytest.mark.skipif(not SERVICE,reason='requires native cross-check service probe')
@pytest.mark.parametrize('case',CASES,ids=lambda c:c['name'])
@pytest.mark.parametrize('workbook',[False,True],ids=['read','xlsx'])
def test_cross_check_service_bytes_workbooks_and_all_database_tables(tmp_path,case,workbook):
    root=tmp_path/'data';conn=open_db(root)
    conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+case['sql']);conn.commit()
    database=Path(conn.execute('PRAGMA database_list').fetchone()[2]);conn.close()
    before=database.read_bytes()
    path=tmp_path/'export.csv';path.write_text(case['csv'],newline='')
    target=tmp_path/'comparison.xlsx';args=json.loads(case['args_json'])
    conn=sqlite3.connect(database);conn.row_factory=sqlite3.Row
    with patch('xlsxwriter.Workbook',RecordingWorkbook):
        expected=outcome(conn,args,path,target if workbook else None)
    conn.close()
    content=None
    if workbook and 'ok' in expected:
        assert expected['ok']['bytes']==target.stat().st_size
        assert 'report_snapshot' not in expected['ok']
        content=design_content(target,False)
    expected_exists=target.exists()
    expected_artifact=target.read_bytes() if expected_exists and content is None else None
    expected_state=state(root)
    database.write_bytes(before);target.unlink(missing_ok=True)
    request=dict(database=str(database),command='reports.cross-check',args_json=json.dumps(dict(args,file=str(path),xlsx=str(target) if workbook else None)))
    result=subprocess.run([SERVICE],input=json.dumps(request)+'\n',capture_output=True,text=True,timeout=180,check=True)
    assert not result.stderr
    actual=json.loads(result.stdout)
    assert target.exists()==expected_exists
    if content is not None:
        assert list(actual['ok'])==list(expected['ok'])
        assert actual['ok']['bytes']==target.stat().st_size
        assert design_content(target,True)==content
    else:
        if expected_exists:
            assert target.read_bytes()==expected_artifact
    wire=lambda value:json.dumps(value,sort_keys=True,separators=(',',':')).encode()
    assert (_wire(wire(actual)) if workbook else wire(actual))==(_wire(wire(expected)) if workbook else wire(expected))
    assert state(root)==expected_state


@pytest.mark.skipif(not RUST_BIN,reason='requires Rust test-harness binary')
@pytest.mark.parametrize('country,variant',[('generic','ready'),('at','ready'),('at','stale'),('at','raw-hex')])
def test_cross_check_cli_bytes_workbooks_all_tables_and_zero_fallbacks(tmp_path,monkeypatch,country,variant):
    root,database,env,call=_clocked_book(tmp_path,monkeypatch,country,variant)
    env['HOME']=os.environ['HOME']
    before=database.read_bytes();path=tmp_path/'export.csv';target=tmp_path/'cross.xlsx'
    options=[[],['--year','2024'],['--year','2008'],['--wallet','cold'],['--wallet','missing'],['--source','Cold source'],['--source','absent'],['--include-tool-wallets']]
    def sequence(reference):
        database.write_bytes(before);outcomes=[]
        for tool in ['cointracking','blockpit']:
            path.write_text(encode([ct()] if tool=='cointracking' else [bp()],tool),newline='')
            for flags in [[],['--machine'],['--format','json'],['--format','table'],['--format','plain'],['--format','csv']]:
                result=call(*flags,'reports','cross-check','--tool',tool,'--file',str(path),reference=reference)
                outcomes.append((result.returncode,result.stdout,result.stderr,state(root)))
            for extra in options:
                target.unlink(missing_ok=True)
                result=call('--machine','reports','cross-check','--tool',tool,'--file',str(path),'--xlsx',str(target),*extra,reference=reference)
                payload=json.loads(result.stdout)
                if payload['kind']!='error':
                    assert payload['data']['bytes']==target.stat().st_size
                    assert 'report_snapshot' not in payload['data']
                outcomes.append((result.returncode,_wire(result.stdout),result.stderr,design_content(target,not reference) if target.exists() else None,state(root)))
        return outcomes
    actual=sequence(False);expected=sequence(True)
    for i,(a,e) in enumerate(zip(actual,expected)):
        assert a==e,(i,a[:3],e[:3])
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert len(served)==28
    assert all(c==dict(command='reports.cross-check',served_by='rust') for c in served)


@pytest.mark.skipif(not RUST_BIN,reason='requires Rust test-harness binary')
@pytest.mark.parametrize('variant',['ready','stale','blocked-source','no-context'])
def test_cross_check_daemon_bytes_workbooks_all_tables_and_zero_fallbacks(tmp_path,monkeypatch,variant):
    root,database,env,call=_clocked_book(tmp_path,monkeypatch,'at','stale' if variant=='stale' else 'ready')
    env['HOME']=os.environ['HOME']
    conn=open_db(root)
    if variant=='blocked-source':
        conn.execute("INSERT INTO freshness_source_states(profile_id,source_type,source_key,source_label,status,blocking_reports,updated_at) VALUES('p','wallet','cold','Cold','error',1,'fixed')")
    if variant=='no-context':conn.execute("DELETE FROM settings WHERE key='context_profile'")
    conn.commit();conn.close();before=database.read_bytes()
    requests=[]
    for kind in ['ui.reports.cross_check','ui.reports.export_cross_check']:
        valid=dict(tool='cointracking',filename='../export.csv',source_bytes_base64=base64.b64encode(encode([ct()]).encode()).decode())
        options=[valid,valid,valid|{'tool':'blockpit','source_bytes_base64':base64.b64encode(encode([bp()],'blockpit').encode()).decode()},valid|{'year':2024},valid|{'wallets':['cold'],'tool_sources':['Cold source']},valid|{'include_tool_wallets':True},None,[],{},valid|{'workspace':'ignored'},valid|{'year':True},valid|{'year':'2024'},valid|{'tool_sources':[1]},valid|{'filename':'export.xlsx'},valid|{'source_bytes_base64':'%%%bad'},valid|{'source_bytes_base64':'YQ='},valid|{'wallet':'cold','wallets':['hot']},valid|{'wallets':[]},valid|{'include_tool_wallets':None},valid|{'filename':7},valid|{'source_bytes_base64':base64.b64encode(b'bad').decode()}]
        requests.extend(dict(kind=kind,args=args) for args in options)
    requests += [dict(kind='ui.reports.explain_capital_gain',args={})]
    wire=b''.join((json.dumps(dict(request_id=i,**r))+'\n').encode() for i,r in enumerate(requests))+b'{"request_id":"done","kind":"daemon.shutdown"}\n'
    exports=tmp_path/'exports'
    def session(reference):
        shutil.rmtree(exports,ignore_errors=True);database.write_bytes(before)
        result=call('daemon',reference=reference,wire=wire)
        assert (result.returncode,result.stderr)==(0,b''),result.stderr
        records=result.stdout.splitlines()[1:]
        for raw in records:
            payload=json.loads(raw)
            if 'file' in payload.get('data',{}):
                assert payload['data']['bytes']==Path(payload['data']['file']).stat().st_size
                assert 'report_snapshot' not in payload['data']
        return records,state(root),{str(p.relative_to(exports)):design_content(p,not reference) for p in exports.rglob('*.xlsx')}
    actual=session(False);expected=session(True)
    for i,(a,e) in enumerate(zip(actual[0],expected[0])):
        assert _wire(a)==_wire(e),(i,a,e)
    assert len(actual[0])==len(expected[0])
    assert actual[1:]==expected[1:]
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served==[dict(command='daemon',served_by='rust')]
