import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess

import pytest

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_mutations import outcome, ROOT
from crates.reports.scripts.gen_source_funds_cases import dump

SERVICE = os.environ.get('KASSIBER_SOF_MUTATION_SERVICE_BIN')
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
FIXTURE = json.loads((ROOT / 'crates/reports/tests/data/source_funds_mutations.json').read_text())


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
def test_source_mutation_bytes_managed_files_and_all_112_tables(tmp_path, case, source_template):
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
    if case['name'] == 'deferred-commit':
        reference.executescript('DROP TRIGGER refuse_commit; DROP TABLE commit_failure;')
    expected_tables = dump(reference)
    reference.close()
    request = dict(database=str(database), passphrase=passphrase, case=case)
    result = subprocess.run([SERVICE], input=json.dumps(request)+'\n', capture_output=True, text=True, check=True, timeout=180)
    assert not result.stderr
    wire = lambda value: json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
    assert result.stdout.encode() == wire(expected)+b'\n'
    native = connect(database)
    if case['name'] == 'deferred-commit':
        native.executescript('DROP TRIGGER refuse_commit; DROP TABLE commit_failure;')
    assert wire(dump(native)) == wire(expected_tables)
    native.close()
    assert (evidence.read_bytes(), hashlib.sha256(evidence.read_bytes()).hexdigest()) == before_file
    assert sorted(path.relative_to(tmp_path).as_posix() for path in tmp_path.rglob('*') if path.is_file()) == ['native.sqlite', 'not-read/secret.pdf', 'reference.sqlite']


@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('transport', ['cli', 'daemon'])
def test_source_mutation_process_envelopes_files_tables_and_zero_fallbacks(tmp_path, monkeypatch, transport):
    import sys
    from tests import cli_runner
    from tests.test_rust_report_exports import _book, REFERENCE, _normalize
    root, database, env, call = _book(tmp_path, monkeypatch, 'generic', 'FIFO')
    conn = open_db(root)
    from crates.reports.scripts.gen_source_funds_reads import BASE
    conn.executescript(BASE)
    conn.commit()
    conn.execute('PRAGMA wal_checkpoint(TRUNCATE)')
    conn.execute('PRAGMA journal_mode=DELETE')
    conn.close()
    baseline = tmp_path / 'baseline.sqlite'
    shutil.copy2(database, baseline)
    fixed_reference = REFERENCE.replace('raise SystemExit(cli.main(sys.argv[1:]))',
        'from kassiber.core import source_funds\nsource_funds._now = lambda: stamp\nraise SystemExit(cli.main(sys.argv[1:]))')
    original_prefix = cli_runner.prefix
    monkeypatch.setattr(cli_runner, 'prefix', lambda *, reference=False, python_options=():
        (sys.executable, '-c', fixed_reference) if reference else original_prefix(python_options=python_options))
    evidence = root / 'not-read/secret.pdf'
    evidence.parent.mkdir()
    evidence.write_bytes(b'synthetic evidence\x00\xff')
    file_hash = hashlib.sha256(evidence.read_bytes()).hexdigest()
    def state():
        conn = sqlite3.connect(database)
        conn.row_factory = sqlite3.Row
        records = dump(conn)
        conn.close()
        return records
    def normalize(records):
        ids = {row['id']: 'generated-source' for row in records['source_funds_sources'] if row['id'] not in {'s-a', 's-z', 's-old', 's-other'}}
        return json.loads(_normalize(json.dumps(records, sort_keys=True).encode(), ids)), ids
    if transport == 'cli':
        operations = [
            ['create', '--type', 'income', '--label', '  Origin Ω  ', '--amount', '1.00000000001', '--attachment', 'a-a', '--attachment', 'a-z'],
            ['create', '--type', 'missing_history', '--label', 'Gap', '--asset', 'l-btc', '--fiat-value', '0', '--acquired-at', '2024-01-01'],
            ['create', '--type', 'unknown', '--label', 'Unknown', '--profile', 'Other'],
            ['create', '--type', 'gift', '--label', 'Missing evidence', '--attachment', 'a-a', '--attachment', 'missing'],
            ['create', '--type', 'gift', '--label', 'Other book evidence', '--attachment', 'a-other'],
            ['create', '--type', 'gift', '--label', ''],
            ['create', '--type', 'gift', '--label', 'Invalid amount', '--amount', '-1'],
            ['attach', '--source', 'Unknown', '--attachment', 'a-a'],
            ['attach', '--source', 's-a', '--attachment', 'a-a'],
            ['attach', '--source', 'missing', '--attachment', 'a-a'],
            ['attach', '--source', 's-z', '--attachment', 'a-other'],
        ]
        for operation in operations:
            outputs = []
            tables = []
            for reference in [False, True]:
                shutil.copy2(baseline, database)
                result = call('--machine', 'source-funds', 'sources', *operation, reference=reference)
                records, ids = normalize(state())
                outputs.append((result.returncode, _normalize(result.stdout, ids), result.stderr))
                tables.append(records)
            assert outputs[0] == outputs[1], operation
            assert tables[0] == tables[1], operation
            assert hashlib.sha256(evidence.read_bytes()).hexdigest() == file_hash
        served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
        assert len(served) == len(operations)
        assert all(row['served_by'] == 'rust' and row['command'] in {'source-funds.sources.create', 'source-funds.sources.attach'} for row in served)
    else:
        requests = [
            ('ui.source_funds.sources.create', dict(source_type='income', label='Origin Ω', amount='1.00000000001', attachment_ids=['a-a', 'a-z'])),
            ('ui.source_funds.sources.attach', dict(source=' s-z ', attachment_id=' a-a ')),
            ('ui.source_funds.sources.attach', dict(source='s-a', attachment_id='a-a')),
            ('ui.source_funds.sources.attach', dict(source='s-other', attachment_id='a-a')),
            ('ui.source_funds.sources.attach', dict(source='s-z', attachment_id='a-other')),
            ('ui.source_funds.sources.create', dict(source_type='gift', label='Rollback', attachment_ids=['a-a', 'missing'])),
            ('ui.source_funds.sources.create', dict(source_type='gift', label='Bad amount', amount='-1')),
            ('ui.source_funds.sources.create', dict(attachment_ids='wrong')),
            ('ui.source_funds.sources.create', []),
            ('ui.source_funds.sources.attach', dict(source='')),
            ('ui.source_funds.sources.list', {}),
        ]
        wire = ''.join(json.dumps(dict(request_id=i,kind=kind,args=args))+'\n' for i,(kind,args) in enumerate(requests)).encode()+b'{"request_id":"done","kind":"daemon.shutdown"}\n'
        outputs, tables = [], []
        for reference in [False, True]:
            shutil.copy2(baseline, database)
            result = call('daemon', reference=reference, wire=wire)
            records, ids = normalize(state())
            outputs.append((result.returncode, _normalize(b'\n'.join(result.stdout.splitlines()[1:]), ids), result.stderr))
            tables.append(records)
        assert outputs[0] == outputs[1]
        assert tables[0] == tables[1]
        assert hashlib.sha256(evidence.read_bytes()).hexdigest() == file_hash
        assert [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()] == [dict(command='daemon', served_by='rust')]
