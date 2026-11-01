import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess

import pytest

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_batch import outcome, ROOT
from crates.reports.scripts.gen_source_funds_cases import dump

SERVICE = os.environ.get('KASSIBER_SOF_MUTATION_SERVICE_BIN')
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
FIXTURE = json.loads((ROOT / 'crates/reports/tests/data/source_funds_batch.json').read_text())


@pytest.fixture(scope='module', params=[False, True], ids=['sqlite', 'sqlcipher'])
def source_template(tmp_path_factory, request):
    encrypted = request.param
    passphrase = 'synthetic-source-passphrase' if encrypted else None
    root = tmp_path_factory.mktemp('source-template')
    conn = open_db(root / 'data', passphrase=passphrase)
    conn.executescript('BEGIN;'+FIXTURE['base_sql']+'COMMIT;')
    conn.execute('PRAGMA wal_checkpoint(TRUNCATE)')
    conn.execute('PRAGMA journal_mode=DELETE')
    database = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    conn.close()
    def connect(path):
        if encrypted:
            from sqlcipher3 import dbapi2 as driver
        else:
            driver = sqlite3
        connection = driver.connect(path)
        connection.row_factory = driver.Row
        if passphrase is not None:
            connection.execute('PRAGMA key='+repr(passphrase))
            connection.executescript('PRAGMA cipher_log_level=NONE;PRAGMA cipher_compatibility=4;PRAGMA kdf_iter=256000;PRAGMA cipher_page_size=4096;')
        connection.execute('PRAGMA foreign_keys=ON')
        return connection
    return database, passphrase, connect


@pytest.mark.skipif(not SERVICE, reason='requires native source mutation probe')
@pytest.mark.parametrize('case', FIXTURE['cases'], ids=lambda case: case['name'])
def test_batch_mutation_bytes_managed_files_and_all_112_tables(tmp_path, case, source_template):
    template, passphrase, connect = source_template
    database, reference_path = tmp_path / 'native.sqlite', tmp_path / 'reference.sqlite'
    shutil.copy2(template, database)
    shutil.copy2(template, reference_path)
    evidence = tmp_path / 'not-read/secret.pdf'
    evidence.parent.mkdir()
    evidence.write_bytes(b'synthetic managed evidence\x00\xff')
    before_file = evidence.read_bytes(), hashlib.sha256(evidence.read_bytes()).hexdigest()
    reference = connect(reference_path)
    reference.executescript(case['sql'])
    expected = outcome(reference, case)
    frozen = case.get('expected_cipher', case['expected']) if passphrase else case['expected']
    assert expected == frozen
    if case['name'].endswith('-deferred'):
        reference.executescript('DROP TRIGGER refuse_commit; DROP TABLE commit_failure;')
    expected_tables = dump(reference)
    reference.close()
    request = dict(database=str(database), passphrase=passphrase, case=case)
    result = subprocess.run([SERVICE], input=json.dumps(request)+'\n', capture_output=True, text=True, check=True, timeout=180)
    assert not result.stderr
    wire = lambda value: json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
    assert result.stdout.encode() == wire(expected)+b'\n'
    native = connect(database)
    if case['name'].endswith('-deferred'):
        native.executescript('DROP TRIGGER refuse_commit; DROP TABLE commit_failure;')
    assert wire(dump(native)) == wire(expected_tables)
    native.close()
    assert (evidence.read_bytes(), hashlib.sha256(evidence.read_bytes()).hexdigest()) == before_file
    assert sorted(path.relative_to(tmp_path).as_posix() for path in tmp_path.rglob('*') if path.is_file()) == ['native.sqlite', 'not-read/secret.pdf', 'reference.sqlite']



@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('encrypted', [False,True], ids=['sqlite','sqlcipher'])
@pytest.mark.parametrize('transport', ['cli','daemon'])
def test_batch_process_bytes_files_all_112_tables_and_zero_fallbacks(tmp_path,monkeypatch,encrypted,transport):
    import sys
    from tests import cli_runner
    from tests.test_rust_report_exports import _book, REFERENCE
    from tests.test_rust_source_funds_diagnostics import _compare
    root,database,env,call=_book(tmp_path,monkeypatch,'generic','FIFO')
    conn=open_db(root)
    from crates.reports.scripts.gen_source_funds_batch import BASE_SQL
    # The process helper has already created book.sql's identities.
    from crates.reports.scripts.gen_source_funds_reads import BASE
    conn.executescript(BASE_SQL[BASE_SQL.index(BASE):])
    conn.execute("UPDATE source_funds_cases SET target_external_id='frozen external' WHERE target_external_id IS NULL")
    conn.commit();conn.execute('PRAGMA wal_checkpoint(TRUNCATE)');conn.execute('PRAGMA journal_mode=DELETE');conn.close()
    if encrypted:
        from sqlcipher3 import dbapi2
        c=dbapi2.connect(database);output=tmp_path/'encrypted.sqlite'
        c.execute('ATTACH DATABASE ? AS encrypted KEY ?',(str(output),'synthetic-source-passphrase'))
        c.execute("SELECT sqlcipher_export('encrypted')");c.execute('DETACH DATABASE encrypted');c.close();os.replace(output,database)
    def connect():
        if encrypted:
            from sqlcipher3 import dbapi2 as driver
        else:driver=sqlite3
        c=driver.connect(database);c.row_factory=driver.Row
        if encrypted:c.execute("PRAGMA key='synthetic-source-passphrase'")
        c.execute('PRAGMA foreign_keys=ON');return c
    conn=connect();before=dump(conn);conn.close()
    baseline=tmp_path/'baseline.sqlite';shutil.copy2(database,baseline)
    program=REFERENCE.replace('raise SystemExit(cli.main(sys.argv[1:]))','from kassiber.core import source_funds\nsource_funds._now=lambda:stamp\nraise SystemExit(cli.main(sys.argv[1:]))')
    original_prefix=cli_runner.prefix
    monkeypatch.setattr(cli_runner,'prefix',lambda *,reference=False,python_options=(): (sys.executable,'-c',program) if reference else original_prefix(python_options=python_options))
    flags=['--db-passphrase-fd',cli_runner.InputFd(b'synthetic-source-passphrase')] if encrypted else []
    evidence=root/'not-read/secret.pdf';evidence.parent.mkdir();evidence.write_bytes(b'synthetic batch evidence\x00\xff')
    file_before=evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest()
    names=['current-custody','structural-valid','two-current','two-dry-run','stale-version','rejected-pair','target-True-1','target-True-3','target-alias','selection-1','selection-4','selection-7','selection-8','selection-9','args-1']
    if transport=='cli':names=names[:8]
    for name in names:
        case=next(c for c in FIXTURE['cases'] if c['name']==name)
        results=[];tables=[]
        for reference in [False,True]:
            shutil.copy2(baseline,database)
            conn=connect();conn.executescript(case['sql']);initial=dump(conn);conn.close()
            if transport=='cli':
                args=['--machine','source-funds','links','bulk-review','--target-transaction',case['args']['target_transaction']]
                if case['args'].get('dry_run'):args.append('--dry-run')
                result=call(*flags,*args,reference=reference)
            else:
                wire=json.dumps(dict(request_id='batch',kind='ui.source_funds.links.bulk_review',args=case['args'])).encode()+b'\n{"request_id":"done","kind":"daemon.shutdown"}\n'
                result=call(*flags,'daemon',reference=reference,wire=wire)
            conn=connect()
            records=dump(conn);conn.close()
            tables.append(records);results.append(result)
            if name=='two-dry-run' and transport=='cli':assert records==initial
            assert (evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest())==file_before
        _compare(*results,transport=='daemon',False)
        assert tables[0]==tables[1],name
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert len(served)==len(names)
    assert all(row==dict(command='daemon' if transport=='daemon' else 'source-funds.links.bulk-review',served_by='rust') for row in served)
