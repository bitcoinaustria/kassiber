import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess

import pytest

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_recipients import outcome, ROOT
from crates.reports.scripts.gen_source_funds_cases import dump

SERVICE = os.environ.get('KASSIBER_SOF_MUTATION_SERVICE_BIN')
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
FIXTURE = json.loads((ROOT / 'crates/reports/tests/data/source_funds_recipients.json').read_text())


@pytest.fixture(scope='module', params=[False, True], ids=['sqlite', 'sqlcipher'])
def recipient_template(tmp_path_factory, request):
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
def test_recipient_mutation_bytes_files_and_all_112_tables(tmp_path, case, recipient_template):
    template, passphrase, connect = recipient_template
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
    if case['name'].endswith('deferred-commit'):
        reference.executescript('DROP TRIGGER refuse_commit; DROP TABLE commit_failure;')
    expected_tables = dump(reference)
    reference.close()
    request = dict(database=str(database), passphrase=passphrase, case=case)
    result = subprocess.run([SERVICE], input=json.dumps(request)+'\n', capture_output=True, text=True, check=True, timeout=180)
    assert not result.stderr
    wire = lambda value: json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
    assert result.stdout.encode() == wire(expected)+b'\n'
    native = connect(database)
    if case['name'].endswith('deferred-commit'):
        native.executescript('DROP TRIGGER refuse_commit; DROP TABLE commit_failure;')
    assert wire(dump(native)) == wire(expected_tables)
    native.close()
    assert (evidence.read_bytes(), hashlib.sha256(evidence.read_bytes()).hexdigest()) == before_file
    assert sorted(path.relative_to(tmp_path).as_posix() for path in tmp_path.rglob('*') if path.is_file()) == ['native.sqlite', 'not-read/secret.pdf', 'reference.sqlite']


@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('transport', ['cli', 'daemon'])
def test_recipient_mutation_process_envelopes_files_tables_and_zero_fallbacks(tmp_path, monkeypatch, transport):
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
    reference_program = REFERENCE.replace('raise SystemExit(cli.main(sys.argv[1:]))',
        'from kassiber.core import source_funds_recipients\nsource_funds_recipients.now_iso = lambda: stamp\nraise SystemExit(cli.main(sys.argv[1:]))')
    original_prefix = cli_runner.prefix
    monkeypatch.setattr(cli_runner, 'prefix', lambda *, reference=False, python_options=():
        (sys.executable, '-c', reference_program) if reference else original_prefix(python_options=python_options))
    evidence = root / 'not-read/secret.pdf'
    evidence.parent.mkdir()
    evidence.write_bytes(b'synthetic evidence\x00\xff')
    original_file = evidence.read_bytes(), hashlib.sha256(evidence.read_bytes()).hexdigest()
    def state():
        conn = sqlite3.connect(database)
        conn.row_factory = sqlite3.Row
        records = dump(conn)
        conn.close()
        generated = sorted((row for row in records['source_funds_recipients'] if row['id'] not in {'r-a', 'r-z', 'r-i', 'r-other'}), key=lambda row: (row['profile_id'], row['label'], row['created_at']))
        ids = {row['id']: f'generated-recipient-{index}' for index, row in enumerate(generated)}
        assert len(ids) == len(set(ids.values())) == len(generated)
        normalized = json.loads(_normalize(json.dumps(records, sort_keys=True).encode(), ids))
        return {table: sorted(rows, key=lambda row: json.dumps(row, sort_keys=True)) for table, rows in normalized.items()}, ids
    if transport == 'cli':
        operations = [
            ['create', '--kind', 'tax_authority', '--label', '  Recipient Ω  ', '--default-reveal-mode', 'labels_only', '--notes', '  Notes Ω  '],
            ['create', '--kind', 'bank', '--label', 'alpha'],
            ['create', '--kind', 'bank', '--label', 'A retired recipient'],
            ['create', '--kind', 'exchange', '--label', 'Other recipient', '--profile', 'Other'],
            ['create', '--kind', 'bank', '--label', ''],
            ['update', '--recipient', 'r-a', '--label', 'Renamed', '--kind', 'lawyer', '--default-reveal-mode', 'full', '--notes', ''],
            ['update', '--recipient', 'alpha'],
            ['update', '--recipient', 'r-a', '--label', 'ALPHA'],
            ['update', '--recipient', 'r-i', '--label', 'Retired renamed'],
            ['update', '--recipient', 'Alpha'],
            ['delete', '--recipient', 'alpha'],
            ['delete', '--recipient', 'r-i'],
            ['delete', '--recipient', 'r-other'],
        ]
        for operation in operations:
            outputs, tables = [], []
            for reference in [False, True]:
                shutil.copy2(baseline, database)
                result = call('--machine', 'source-funds', 'recipients', *operation, reference=reference)
                records, ids = state()
                outputs.append((result.returncode, _normalize(result.stdout, ids), result.stderr))
                tables.append(records)
            assert outputs[0] == outputs[1], operation
            assert tables[0] == tables[1], operation
            assert (evidence.read_bytes(), hashlib.sha256(evidence.read_bytes()).hexdigest()) == original_file
        served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
        assert len(served) == len(operations)
        assert all(row['served_by'] == 'rust' and row['command'] in {'source-funds.recipients.create', 'source-funds.recipients.update', 'source-funds.recipients.delete'} for row in served)
    else:
        requests = [
            ('ui.source_funds.recipients.create', dict(kind=' Tax-Authority ', label='  Recipient Ω  ', default_reveal_mode='labels_only', notes='  Notes Ω  ')),
            ('ui.source_funds.recipients.create', dict(kind='bank', label='alpha')),
            ('ui.source_funds.recipients.create', dict(kind='bank', label='A retired recipient')),
            ('ui.source_funds.recipients.create', dict(kind='bank', label='')),
            ('ui.source_funds.recipients.update', dict(recipient=' r-a ', label='Renamed', kind='lawyer', default_reveal_mode='full', notes='')),
            ('ui.source_funds.recipients.update', dict(recipient='r-z', label=False, kind=42, notes=None)),
            ('ui.source_funds.recipients.update', dict(recipient='Alpha')),
            ('ui.source_funds.recipients.update', dict(recipient='r-i', label='Retired renamed')),
            ('ui.source_funds.recipients.delete', dict(recipient='Renamed')),
            ('ui.source_funds.recipients.delete', dict(recipient='r-i')),
            ('ui.source_funds.recipients.delete', dict(recipient='r-other')),
            ('ui.source_funds.recipients.delete', dict(recipient='')),
            ('ui.source_funds.recipients.create', []),
            ('ui.source_funds.recipients.list', dict(include_inactive=True)),
            ('ui.source_funds.cases.list', {}),
        ]
        wire = ''.join(json.dumps(dict(request_id=i, kind=kind, args=args))+'\n' for i, (kind, args) in enumerate(requests)).encode()+b'{"request_id":"done","kind":"daemon.shutdown"}\n'
        outputs, tables = [], []
        for reference in [False, True]:
            shutil.copy2(baseline, database)
            result = call('daemon', reference=reference, wire=wire)
            records, ids = state()
            outputs.append((result.returncode, _normalize(b'\n'.join(result.stdout.splitlines()[1:]), ids), result.stderr))
            tables.append(records)
        assert outputs[0] == outputs[1]
        assert tables[0] == tables[1]
        assert (evidence.read_bytes(), hashlib.sha256(evidence.read_bytes()).hexdigest()) == original_file
        assert [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()] == [dict(command='daemon', served_by='rust')]
