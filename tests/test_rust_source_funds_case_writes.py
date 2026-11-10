import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess

import pytest

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_cases import dump, outcome, ROOT

SERVICE = os.environ.get('KASSIBER_SOF_CASE_SERVICE_BIN')
FIXTURE = json.loads((ROOT / 'crates/reports/tests/data/source_funds_cases.json').read_text())


@pytest.fixture(scope='module', params=[False, True], ids=['sqlite', 'sqlcipher'])
def case_template(tmp_path_factory, request):
    encrypted = request.param
    passphrase = 'synthetic-case-passphrase' if encrypted else None
    root = tmp_path_factory.mktemp('case-template')
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


@pytest.mark.skipif(not SERVICE, reason='requires native frozen-case persistence probe')
@pytest.mark.parametrize('case', FIXTURE['cases'], ids=lambda case: case['name'])
def test_frozen_case_persistence_bytes_and_all_112_tables(tmp_path, case, case_template):
    template, passphrase, connect = case_template
    database = tmp_path / 'native.sqlite'
    reference_path = tmp_path / 'reference.sqlite'
    shutil.copy2(template, database)
    shutil.copy2(template, reference_path)
    reference = connect(reference_path)
    native = connect(database)
    assert dump(native) == dump(reference)
    native.close()
    reference.executescript(case['sql'])
    expected = outcome(reference, case)
    assert expected == case['expected']
    expected_tables = dump(reference)
    reference.close()
    request = dict(database=str(database), passphrase=passphrase, case=case)
    result = subprocess.run([SERVICE], input=json.dumps(request)+'\n', capture_output=True,
                            text=True, check=True, timeout=180)
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
