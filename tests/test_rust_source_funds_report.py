"""Complete provenance report bytes, database invariance and native routing."""
import json
import os
from pathlib import Path
import subprocess

import pytest

from kassiber.db import open_db
from tests.test_rust_source_funds_reads import ROOT, SERVICE, RUST_BIN, book, state

CASES = json.loads((ROOT/'crates/reports/tests/data/source_funds_report.json').read_text())


@pytest.mark.skipif(not SERVICE, reason='requires native source-funds service probe')
def test_source_funds_complete_report_service_bytes_and_all_tables(tmp_path):
    for index,case in enumerate(CASES):
        root=tmp_path/str(index)
        conn=open_db(root)
        conn.executescript(case['sql'])
        conn.commit()
        database=Path(conn.execute('PRAGMA database_list').fetchone()[2])
        conn.close()
        before=state(root)
        request=dict(database=str(database),command=case['command'],args_json=case['args_json'])
        native=subprocess.run([SERVICE],input=json.dumps(request)+'\n',capture_output=True,text=True,check=True,timeout=180)
        assert not native.stderr,case['name']
        expected=[json.dumps(json.loads(v))+'\n' for v in case['expected_variants']]
        assert native.stdout in expected,(case['name'],native.stdout[:1000])
        assert state(root)==before,case['name']


@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('variant',['ready','stale','source-blocked','no-context'])
def test_source_funds_complete_report_cli_bytes_files_tables_and_zero_fallbacks(tmp_path,monkeypatch,variant):
    root,_,env,call=book(tmp_path,monkeypatch,variant)
    before=state(root)
    count=0
    recipes=[[],['--target-amount','0.1'],['--purpose','planned_exchange_sale','--planned-destination','Exchange','--planned-note','Sale'],['--recipient','r-a'],['--reveal-mode','minimal'],['--reveal-mode','full','--diagram-detail','detailed','--amount-precision','sats','--mask-recipient','--omit-section','flow_links','--reveal-override','in=hide'],['--max-depth','1'],['--target-transaction','missing'],['--target-transaction','duplicate'],['--target-amount','bad'],['--recipient','missing']]
    for flags in [[],['--machine'],['--format','json'],['--format','table'],['--format','plain'],['--format','csv']]:
        for recipe in recipes:
            args=[*flags,'reports','source-funds','--target-transaction','in',*recipe]
            native=call(*args)
            reference=call(*args,reference=True)
            assert (native.returncode,native.stdout,native.stderr)==(reference.returncode,reference.stdout,reference.stderr),args
            assert state(root)==before
            count+=1
    for format in ['json','plain']:
        destination=tmp_path/('report.'+format)
        args=['--format',format,'--output',str(destination),'reports','source-funds','--target-transaction','in']
        native=call(*args)
        payload=destination.read_bytes() if destination.exists() else None
        destination.unlink(missing_ok=True)
        reference=call(*args,reference=True)
        assert (native.returncode,native.stdout,native.stderr)==(reference.returncode,reference.stdout,reference.stderr)
        assert (destination.read_bytes() if destination.exists() else None)==payload
        assert state(root)==before
        count+=1
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert len(served)==count==68
    assert all(item==dict(command='reports.source-funds',served_by='rust') for item in served)


@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('variant',['ready','stale','source-blocked','no-context'])
def test_source_funds_complete_report_daemon_bytes_tables_and_zero_fallbacks(tmp_path,monkeypatch,variant):
    root,_,env,call=book(tmp_path,monkeypatch,variant)
    before=state(root)
    recipes=[None,{},False,[],dict(target_transaction='missing'),dict(target_transaction='duplicate')]
    recipes += [dict(target_transaction='in',**extra) for extra in [{},dict(target_amount='0.1'),dict(reveal_mode='minimal'),dict(reveal_mode='full'),dict(report_purpose='planned_exchange_sale',planned_destination='Exchange'),dict(recipient='r-a'),dict(recipient='missing'),dict(max_depth=1),dict(max_depth='2'),dict(report_options=dict(diagram_detail='detailed',mask_recipient=True,omit_sections=['flow_links']))]]
    wire=''.join(json.dumps(dict(request_id=i,kind='ui.source_funds.preview',args=args))+'\n' for i,args in enumerate(recipes))+'{"request_id":"done","kind":"daemon.shutdown"}\n'
    native=call('daemon',wire=wire.encode())
    assert state(root)==before
    reference=call('daemon',reference=True,wire=wire.encode())
    assert state(root)==before
    assert native.returncode==reference.returncode==0
    assert native.stderr==reference.stderr==b''
    assert native.stdout.splitlines()[1:]==reference.stdout.splitlines()[1:]
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served==[dict(command='daemon',served_by='rust')]
