import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess

import pytest

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_cases import dump
from crates.reports.scripts.gen_source_funds_passes import outcome, ROOT

SERVICE=os.environ.get('KASSIBER_SOF_MUTATION_SERVICE_BIN')
RUST_BIN=os.environ.get('KASSIBER_RUST_BIN')
CASES=json.loads((ROOT/'crates/reports/tests/data/source_funds_passes.json').read_text())


@pytest.fixture(scope='module',params=[False,True],ids=['sqlite','sqlcipher'])
def passes_template(tmp_path_factory,request):
    passphrase='synthetic-passes-passphrase' if request.param else None
    root=tmp_path_factory.mktemp('passes-template');conn=open_db(root/'data',passphrase=passphrase)
    conn.execute('PRAGMA wal_checkpoint(TRUNCATE)');conn.execute('PRAGMA journal_mode=DELETE')
    database=Path(conn.execute('PRAGMA database_list').fetchone()[2]);conn.close()
    def connect(path):
        if passphrase:
            from sqlcipher3 import dbapi2 as driver
        else:driver=sqlite3
        c=driver.connect(path);c.row_factory=driver.Row
        if passphrase:c.execute('PRAGMA key='+repr(passphrase));c.executescript('PRAGMA cipher_log_level=NONE;PRAGMA cipher_compatibility=4;PRAGMA kdf_iter=256000;PRAGMA cipher_page_size=4096;')
        c.execute('PRAGMA foreign_keys=ON');return c
    return database,passphrase,connect


@pytest.mark.skipif(not SERVICE,reason='requires native mutation probe')
@pytest.mark.parametrize('case',CASES,ids=lambda c:c['name'])
def test_passes_service_bytes_hashes_all_112_tables_and_commits(tmp_path,case,passes_template):
    template,passphrase,connect=passes_template
    native,reference=tmp_path/'native.sqlite',tmp_path/'reference.sqlite'
    for path in [native,reference]:shutil.copy2(template,path)
    evidence=tmp_path/'not-read/secret.pdf';evidence.parent.mkdir();evidence.write_bytes(b'synthetic persisted evidence\0\xff')
    before=evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest()
    seeded={**case,'sql':'PRAGMA synchronous=OFF;'+case['sql']+'PRAGMA synchronous=FULL;'}
    conn=connect(reference);conn.executescript(seeded['sql']);expected=outcome(conn,case)
    assert expected==case['expected'],case['name']
    expected_tables=dump(conn);conn.close()
    result=subprocess.run([SERVICE],input=json.dumps(dict(database=str(native),passphrase=passphrase,case=seeded))+'\n',capture_output=True,text=True,check=True,timeout=180)
    assert result.stderr==''
    wire=lambda v:json.dumps(v,sort_keys=True,separators=(',',':'))
    assert result.stdout==wire(expected)+'\n'
    conn=connect(native);assert wire(dump(conn))==wire(expected_tables);conn.close()
    assert (evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest())==before
    assert sorted(p.relative_to(tmp_path).as_posix() for p in tmp_path.rglob('*') if p.is_file())==['native.sqlite','not-read/secret.pdf','reference.sqlite']


@pytest.mark.skipif(not RUST_BIN,reason='requires native test-harness binary')
@pytest.mark.parametrize('encrypted',[False,True],ids=['sqlite','sqlcipher'])
@pytest.mark.parametrize('transport',['cli','daemon'])
def test_passes_process_bytes_hashes_all_112_tables_and_zero_fallbacks(tmp_path,monkeypatch,encrypted,transport):
    import sys
    from uuid import UUID
    from tests import cli_runner
    from tests.test_rust_report_exports import _book,REFERENCE,_normalize
    from tests.test_rust_source_funds_diagnostics import _compare
    root,database,env,call=_book(tmp_path,monkeypatch,'generic','FIFO')
    conn=open_db(tmp_path/'empty-template');conn.execute('PRAGMA wal_checkpoint(TRUNCATE)');conn.execute('PRAGMA journal_mode=DELETE');empty_database=Path(conn.execute('PRAGMA database_list').fetchone()[2]);conn.close();shutil.copy2(empty_database,database)
    if encrypted:
        from sqlcipher3 import dbapi2
        conn=dbapi2.connect(database);output=tmp_path/'encrypted.sqlite'
        conn.execute('ATTACH DATABASE ? AS encrypted KEY ?',(str(output),'synthetic-passes-passphrase'));conn.execute("SELECT sqlcipher_export('encrypted')");conn.execute('DETACH DATABASE encrypted');conn.close();os.replace(output,database)
    def connect():
        if encrypted:
            from sqlcipher3 import dbapi2 as driver
        else:driver=sqlite3
        c=driver.connect(database);c.row_factory=driver.Row
        if encrypted:c.execute("PRAGMA key='synthetic-passes-passphrase'")
        c.execute('PRAGMA foreign_keys=ON');return c
    baseline=tmp_path/'baseline.sqlite';shutil.copy2(database,baseline)
    program=REFERENCE.replace('raise SystemExit(cli.main(sys.argv[1:]))','from kassiber.core import source_funds\nsource_funds._now=lambda:stamp\nraise SystemExit(cli.main(sys.argv[1:]))')
    original_prefix=cli_runner.prefix
    monkeypatch.setattr(cli_runner,'prefix',lambda *,reference=False,python_options=():(sys.executable,'-c',program) if reference else original_prefix(python_options=python_options))
    flags=['--db-passphrase-fd',cli_runner.InputFd(b'synthetic-passes-passphrase')] if encrypted else []
    evidence=root/'attachments/not-read/secret.pdf';evidence.parent.mkdir(parents=True);evidence.write_bytes(b'process evidence\0\xff');file_before=evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest()
    names=['suggest-provider-default','suggest-cap-rollback','assemble-provider-default-1','assemble-provider-default-2','assemble-structural-current-1','assemble-structural-current-8']
    for name in names:
        case=next(c for c in CASES if c['name']==name);results=[];states=[]
        for reference in [False,True]:
            shutil.copy2(baseline,database);conn=connect();conn.executescript('PRAGMA synchronous=OFF;'+case['sql']);conn.execute("UPDATE source_funds_cases SET target_external_id='frozen external' WHERE target_external_id IS NULL");conn.commit();conn.close()
            args=case['args'];command=case['command'].split('.')[-1]
            if transport=='cli':
                argv=['--machine','source-funds',command]
                for key in ['target_transaction','max_suggestions','max_passes']:
                    if key in args:argv.extend(['--'+key.replace('_','-'),str(args[key])])
                if args.get('include_broad_hints'):argv.append('--include-broad-hints')
                result=call(*flags,*argv,reference=reference)
            else:
                requests=[dict(request_id='pass',kind='ui.source_funds.'+command,args=args),dict(request_id='done',kind='daemon.shutdown')]
                result=call(*flags,'daemon',reference=reference,wire=''.join(json.dumps(r)+'\n' for r in requests).encode())
            conn=connect();states.append(dump(conn));conn.close();results.append(result)
            assert (evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest())==file_before
        mapping={};reference_rows={tuple(r.get(k) for k in ['from_transaction_id','to_transaction_id','link_type','method']):r for r in states[1]['source_funds_links']}
        for row in states[0]['source_funds_links']:
            other=reference_rows[tuple(row.get(k) for k in ['from_transaction_id','to_transaction_id','link_type','method'])]
            if row['id']!=other['id']:
                assert UUID(row['id']).version==UUID(other['id']).version==4;mapping[row['id']]=other['id']
        results[0].stdout=_normalize(results[0].stdout,mapping)
        assert results[0].returncode==results[1].returncode,(name,results[0].stdout,results[1].stdout)
        _compare(*results,daemon=transport=='daemon',debug=False)
        assert json.loads(_normalize(json.dumps(states[0],sort_keys=True).encode(),mapping))==states[1],name
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert len(served)==len(names)
    assert all(r['served_by']=='rust' and r['command'] in ['source-funds.suggest','source-funds.assemble','daemon'] for r in served)
