import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess

import pytest

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_saves import outcome, ROOT
from crates.reports.scripts.gen_source_funds_cases import dump

SERVICE = os.environ.get('KASSIBER_SOF_MUTATION_SERVICE_BIN')
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
FIXTURE = json.loads((ROOT / 'crates/reports/tests/data/source_funds_saves.json').read_text())


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
def test_case_save_bytes_managed_hashes_reports_and_all_112_tables(tmp_path, case, source_template):
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
    expected_tables = dump(reference)
    reference.close()
    request = dict(database=str(database), passphrase=passphrase, case=case)
    result = subprocess.run([SERVICE], input=json.dumps(request)+'\n', capture_output=True, text=True, check=True, timeout=180)
    assert not result.stderr
    wire = lambda value: json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
    # A refusal that ends the transaction: the reference reports its own
    # cleanup failure, the native save the refusal (docs/reference/rust-shell.md,
    # tests/rust_port/repro_source_funds_save_rollback_masks_refusal.py).
    native_expected = wire(expected).replace(b'no such savepoint: source_funds_case_snapshot', b'synthetic late refusal')
    assert result.stdout.encode() == native_expected+b'\n'
    native = connect(database)
    assert wire(dump(native)) == wire(expected_tables)
    native.close()
    assert (evidence.read_bytes(), hashlib.sha256(evidence.read_bytes()).hexdigest()) == before_file
    assert sorted(path.relative_to(tmp_path).as_posix() for path in tmp_path.rglob('*') if path.is_file()) == ['native.sqlite', 'not-read/secret.pdf', 'reference.sqlite']



@pytest.mark.skipif(not RUST_BIN,reason='requires Rust test-harness binary')
@pytest.mark.parametrize('encrypted',[False,True],ids=['sqlite','sqlcipher'])
@pytest.mark.parametrize('transport',['cli','daemon'])
def test_case_save_process_bytes_frozen_snapshots_hashes_all_tables_and_zero_fallbacks(tmp_path,monkeypatch,encrypted,transport):
    import sys
    from tests import cli_runner
    from tests.test_rust_report_exports import _book, REFERENCE, _normalize
    from tests.test_rust_source_funds_diagnostics import _compare
    from crates.reports.scripts.gen_source_funds_saves import READY
    from crates.reports.scripts.gen_source_funds_reads import BASE
    root,database,env,call=_book(tmp_path,monkeypatch,'generic','FIFO')
    conn=open_db(root);conn.executescript(BASE+READY)
    conn.execute("UPDATE source_funds_cases SET target_external_id='frozen external' WHERE target_external_id IS NULL")
    conn.commit();conn.execute('PRAGMA wal_checkpoint(TRUNCATE)');conn.execute('PRAGMA journal_mode=DELETE');conn.close()
    if encrypted:
        from sqlcipher3 import dbapi2
        conn=dbapi2.connect(database);output=tmp_path/'encrypted.sqlite'
        conn.execute('ATTACH DATABASE ? AS encrypted KEY ?',(str(output),'synthetic-source-passphrase'))
        conn.execute("SELECT sqlcipher_export('encrypted')");conn.execute('DETACH DATABASE encrypted');conn.close();os.replace(output,database)
    def connect():
        if encrypted:
            from sqlcipher3 import dbapi2 as driver
        else:driver=sqlite3
        conn=driver.connect(database);conn.row_factory=driver.Row
        if encrypted:conn.execute("PRAGMA key='synthetic-source-passphrase'")
        conn.execute('PRAGMA foreign_keys=ON');return conn
    baseline=tmp_path/'baseline.sqlite';shutil.copy2(database,baseline)
    program=REFERENCE.replace('raise SystemExit(cli.main(sys.argv[1:]))','from kassiber.core import source_funds\nsource_funds._now=lambda:stamp\nraise SystemExit(cli.main(sys.argv[1:]))')
    original_prefix=cli_runner.prefix
    monkeypatch.setattr(cli_runner,'prefix',lambda *,reference=False,python_options=(): (sys.executable,'-c',program) if reference else original_prefix(python_options=python_options))
    flags=['--db-passphrase-fd',cli_runner.InputFd(b'synthetic-source-passphrase')] if encrypted else []
    evidence=root/'not-read/secret.pdf';evidence.parent.mkdir();evidence.write_bytes(b'synthetic case evidence\x00\xff')
    original=evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest()
    def normalized(records):
        ids={row['id']:'new-case' for row in records['source_funds_cases'] if row['id'] not in {'c-a','c-z','c-other'}}
        ids.update({row['id']:'new-snapshot' for row in records['source_funds_snapshots'] if row['case_id'] in ids})
        return json.loads(_normalize(json.dumps(records,sort_keys=True).encode(),ids)),ids
    if transport=='cli':
        operations=[[],['--recipient','r-a','--reveal-mode','minimal','--case-label','Frozen Ω'],['--target-amount','0.1','--purpose','planned_exchange_sale','--planned-destination','Exchange','--diagram-detail','detailed','--amount-precision','sats','--mask-recipient'],['--target-transaction','missing'],['--recipient','missing'],['--target-amount','0']]
    else:
        from kassiber.core import source_funds_review as review
        from kassiber import daemon
        conn=connect();_,profile=daemon.resolve_scope(conn,None,None)
        fp=review.review_context(conn,profile,daemon._source_funds_hooks(),target_transaction='in')['review_fingerprint'];conn.close()
        operations=[dict(_kind='ui.source_funds.review_context'),dict(_kind='ui.source_funds.review_context',recipient='r-a',reveal_mode='minimal'),dict(_kind='ui.source_funds.review_context',max_depth=True),{},dict(case_label='Frozen Ω',recipient='r-a',reveal_mode='minimal'),dict(expected_review_fingerprint=fp,case_label='Reviewed Ω'),dict(expected_review_fingerprint='0'*64),dict(expected_review_fingerprint=None),dict(target_transaction='missing'),dict(case_label=1),dict(recipient='missing'),dict(target_transaction=''),dict(report_options=dict(mask_recipient=True,amount_precision='sats'))]
    for operation in operations:
        results=[];states=[]
        for reference in [False,True]:
            shutil.copy2(baseline,database)
            if transport=='cli':
                result=call(*flags,'--machine','reports','source-funds','--target-transaction','in','--save-case',*operation,reference=reference)
            else:
                args={'target_transaction':'in',**operation}
                kind=args.pop('_kind','ui.source_funds.cases.save')
                wire=json.dumps(dict(request_id='save',kind=kind,args=args)).encode()+b'\n{"request_id":"done","kind":"daemon.shutdown"}\n'
                result=call(*flags,'daemon',wire=wire,reference=reference)
            conn=connect();records=dump(conn);conn.close()
            state,ids=normalized(records);states.append(state)
            result.stdout=_normalize(result.stdout,ids)
            results.append(result)
            assert (evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest())==original
            for row in state['source_funds_cases']:
                if row['id']=='new-case':
                    frozen=json.loads(row['snapshot_json'])
                    assert 'case' not in frozen
                    assert 'diagrams' in frozen
                    assert row['snapshot_hash']==hashlib.sha256(json.dumps(frozen,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        _compare(*results,transport=='daemon',False)
        assert states[0]==states[1],operation
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served==[dict(command='daemon' if transport=='daemon' else 'reports.source-funds',served_by='rust')]*len(operations)
