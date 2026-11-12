"""Historical inbound coverage compares complete bytes and all 114 tables."""
import json
from pathlib import Path
import subprocess

import pytest

from kassiber.db import open_db
from tests.test_rust_source_funds_reads import ROOT, SERVICE, RUST_BIN, book, state

CASES=json.loads((ROOT/'crates/reports/tests/data/source_funds_coverage.json').read_text())


@pytest.mark.skipif(not SERVICE,reason='requires native report service probe')
def test_source_funds_coverage_service_bytes_and_all_tables(tmp_path):
    for index,case in enumerate(CASES):
        root=tmp_path/str(index)
        conn=open_db(root);conn.executescript(case['sql']);conn.commit()
        database=Path(conn.execute('PRAGMA database_list').fetchone()[2]);conn.close()
        before=state(root)
        result=subprocess.run([SERVICE],input=json.dumps(dict(database=str(database),command=case['command'],args_json=case['args_json']))+'\n',capture_output=True,text=True,check=True,timeout=180)
        assert result.stderr==''
        assert result.stdout==json.dumps(json.loads(case['expected_json']))+'\n',case['name']
        assert state(root)==before,case['name']


@pytest.mark.skipif(not RUST_BIN,reason='requires native test-harness binary')
@pytest.mark.parametrize('variant',['ready','stale','source-blocked','no-context'])
def test_source_funds_coverage_cli_bytes_files_all_tables_and_zero_fallbacks(tmp_path,monkeypatch,variant):
    root,_,env,call=book(tmp_path,monkeypatch,variant);before=state(root);count=0
    def compare(args,output=None):
        nonlocal count
        native=call(*args)
        payload=output.read_bytes() if output and output.exists() else None
        if output:output.unlink(missing_ok=True)
        reference=call(*args,reference=True)
        assert (native.returncode,native.stdout,native.stderr)==(reference.returncode,reference.stdout,reference.stderr),args
        if output:assert (output.read_bytes() if output.exists() else None)==payload
        assert state(root)==before
        count+=1
    for flags in [[],['--machine'],['--format','json'],['--format','table'],['--format','plain'],['--format','csv']]:
        compare([*flags,'source-funds','coverage'])
    for cap,depth in [(0,0),(-1,99),(1,1),(2,64),(2000,8)]:
        compare(['--machine','source-funds','coverage','--max-transactions',str(cap),'--max-depth',str(depth)])
    for profile in ['Other','missing']:
        compare(['--machine','source-funds','coverage','--profile',profile])
    for depth in [-1,2,99]:
        compare(['--machine','source-funds','coverage','--max-depth',str(depth)])
    for format in ['json','plain']:
        output=tmp_path/('coverage.'+format)
        compare(['--format',format,'--output',str(output),'source-funds','coverage','--max-transactions','1'],output)
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert len(served)==count==18
    assert served==[dict(command='source-funds.coverage',served_by='rust')]*count


@pytest.mark.skipif(not RUST_BIN,reason='requires native test-harness binary')
@pytest.mark.parametrize('variant',['ready','stale','source-blocked','no-context'])
def test_source_funds_coverage_daemon_bytes_all_tables_and_zero_fallbacks(tmp_path,monkeypatch,variant):
    root,_,env,call=book(tmp_path,monkeypatch,variant);before=state(root)
    values=[None,False,True,0,-1,1,2,50_001,1.5,'2',10**40]
    args=[None,{},[],False,dict(unknown=True,profile='missing')]
    args += [dict(max_transactions=v) for v in values]
    args += [dict(max_depth=v) for v in values]
    requests=[dict(request_id=i,kind='ui.source_funds.coverage',args=a) for i,a in enumerate(args)]
    wire=''.join(json.dumps(r)+'\n' for r in requests)+'{"request_id":"done","kind":"daemon.shutdown"}\n'
    native=call('daemon',wire=wire.encode());assert state(root)==before
    reference=call('daemon',reference=True,wire=wire.encode());assert state(root)==before
    assert native.returncode==reference.returncode==0
    assert native.stderr==reference.stderr==b''
    assert native.stdout.splitlines()[1:]==reference.stdout.splitlines()[1:]
    assert len(native.stdout.splitlines())==len(requests)+2==29
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served==[dict(command='daemon',served_by='rust')]
