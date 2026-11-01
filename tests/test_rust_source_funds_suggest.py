"""Suggestion previews compare full bytes and all tables with account HOME intact."""
import ast
import csv
import io
import hashlib
import json
from pathlib import Path
import subprocess
from uuid import UUID

import pytest

from kassiber.db import open_db
from tests.test_rust_source_funds_reads import ROOT, SERVICE, RUST_BIN, book, state
from tests import test_rust_report_exports as exports
from crates.reports.scripts.gen_source_funds_suggest import pair

CASES=json.loads((ROOT/'crates/reports/tests/data/source_funds_suggest.json').read_text())


def same_bytes(native,reference):
    mapping={}
    def identities(a,b):
        if isinstance(a,dict) and isinstance(b,dict):
            if a.get('state')=='suggested' and b.get('state')=='suggested' and 'id' in a and 'id' in b and a['id']!=b['id']:
                assert UUID(a['id']).version==UUID(b['id']).version==4
                assert mapping.setdefault(a['id'],b['id'])==b['id']
            for key in a.keys()&b.keys():identities(a[key],b[key])
        elif isinstance(a,list) and isinstance(b,list):
            for x,y in zip(a,b):identities(x,y)
    def records(wire):
        try:return json.loads(wire)
        except json.JSONDecodeError:pass
        for line in wire.decode().splitlines():
            if line.startswith('links: '):return dict(links=json.loads(line[7:]))
            if line.startswith('links '):return dict(links=ast.literal_eval(line[6:].strip()))
        rows=list(csv.DictReader(io.StringIO(wire.decode())))
        if rows and 'links' in rows[0]:
            try:return dict(links=json.loads(rows[0]['links']))
            except json.JSONDecodeError:return dict(links=ast.literal_eval(rows[0]['links']))
        return {}
    identities(records(native),records(reference))
    assert len(set(mapping.values()))==len(mapping)
    for old,new in mapping.items():native=native.replace(old.encode(),new.encode())
    assert native==reference
    return mapping


@pytest.mark.skipif(not SERVICE,reason='requires native report service probe')
def test_source_funds_suggest_service_bytes_and_all_tables(tmp_path):
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
def test_source_funds_suggest_cli_bytes_files_all_tables_and_zero_fallbacks(tmp_path,monkeypatch,variant):
    monkeypatch.setattr(exports,'REFERENCE',exports.REFERENCE.replace('raise SystemExit(cli.main(sys.argv[1:]))','from kassiber.core import source_funds\nsource_funds._now=lambda: stamp\nraise SystemExit(cli.main(sys.argv[1:]))'))
    root,_,env,call=book(tmp_path,monkeypatch,variant)
    conn=open_db(root);conn.executescript(pair());conn.commit();conn.close()
    conn=open_db(root);conn.close()
    before=state(root);count=0
    def compare(args,output=None):
        nonlocal count
        args=[*args,'--dry-run']
        native=call(*args);assert state(root)==before
        payload=output.read_bytes() if output and output.exists() else None
        if output:output.unlink(missing_ok=True)
        reference=call(*args,reference=True);assert state(root)==before
        assert native.returncode==reference.returncode
        assert native.stderr==reference.stderr
        identity_map=same_bytes(native.stdout,reference.stdout)
        record=dict(args=args,returncode=native.returncode,native_stdout_sha256=hashlib.sha256(native.stdout).hexdigest(),python_stdout_sha256=hashlib.sha256(reference.stdout).hexdigest(),generated_uuid_map=identity_map)
        if output:
            expected=output.read_bytes() if output.exists() else None
            if payload is None:assert expected is None
            else:record['file_uuid_map']=same_bytes(payload,expected)
            record['output_file']=dict(native_sha256=hashlib.sha256(payload).hexdigest() if payload is not None else None,python_sha256=hashlib.sha256(expected).hexdigest() if expected is not None else None)
        with (tmp_path/'comparisons.jsonl').open('a') as artifact:artifact.write(json.dumps(record)+'\n')
        count+=1
    for flags in [[],['--machine'],['--format','json'],['--format','table'],['--format','plain'],['--format','csv']]:
        compare([*flags,'source-funds','suggest'])
    for flags in [[],['--target-transaction','suggest-in'],['--target-transaction','suggest-out'],['--target-transaction','missing'],['--target-transaction','duplicate'],['--target-transaction','in'],['--profile','Other'],['--profile','missing'],['--include-broad-hints'],['--max-suggestions','0'],['--max-suggestions','1'],['--max-suggestions','999']]:
        compare(['--machine','source-funds','suggest',*flags])
    for format in ['json','plain']:
        output=tmp_path/('suggest.'+format)
        compare(['--format',format,'--output',str(output),'source-funds','suggest'],output)
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert len(served)==count==20
    assert served==[dict(command='source-funds.suggest',served_by='rust')]*count
    probe=subprocess.run([RUST_BIN,'source-funds','suggest'],env={**env,'KASSIBER_RUST_TEST_PROBE':'1'},capture_output=True,check=False)
    assert probe.returncode==0
    assert state(root)==before
