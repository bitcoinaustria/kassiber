import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess

import pytest

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_links import outcome, ROOT
from crates.reports.scripts.gen_source_funds_cases import dump

SERVICE = os.environ.get('KASSIBER_SOF_MUTATION_SERVICE_BIN')
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
FIXTURE = json.loads((ROOT / 'crates/reports/tests/data/source_funds_links.json').read_text())


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
def test_link_mutation_bytes_managed_files_and_all_112_tables(tmp_path, case, source_template):
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
@pytest.mark.parametrize('transport', ['cli', 'daemon'])
def test_link_mutation_process_envelopes_files_tables_and_zero_fallbacks(tmp_path, monkeypatch, transport):
    import sys
    from tests import cli_runner
    from tests.test_rust_report_exports import _book, REFERENCE, _normalize
    root, database, env, call = _book(tmp_path, monkeypatch, 'generic', 'FIFO')
    conn = open_db(root)
    from crates.reports.scripts.gen_source_funds_reads import BASE
    conn.executescript(BASE)
    conn.execute("UPDATE source_funds_links SET from_transaction_id='in',to_transaction_id='out',link_type='self_transfer',asset='BTC',from_asset='BTC',allocation_amount=100001,from_allocation_amount=100002 WHERE id='l-z'")
    conn.commit()
    conn.execute('PRAGMA wal_checkpoint(TRUNCATE)')
    conn.execute('PRAGMA journal_mode=DELETE')
    conn.close()
    baseline = tmp_path / 'baseline.sqlite'
    shutil.copy2(database, baseline)
    # Links created at the pinned time list by id, so both sides number them
    # in creation order; random ids made the daemon list order a coin flip.
    env['KASSIBER_RUST_TEST_REPORT_IDS'] = 'sequential'
    reference_program = REFERENCE.replace('raise SystemExit(cli.main(sys.argv[1:]))',
        'from kassiber.core import source_funds\nsource_funds._now = lambda: stamp\n'
        'import itertools, types\nids = itertools.count(1)\n'
        "source_funds.uuid = types.SimpleNamespace(uuid4=lambda: '00000000-0000-4000-8000-%012x' % next(ids))\n"
        'raise SystemExit(cli.main(sys.argv[1:]))')
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
        generated = sorted((row for row in records['source_funds_links'] if row['id'] not in {'l-a','l-z','l-old','l-other'}), key=lambda row: (row['profile_id'],row['to_transaction_id'],row['method'],row['link_type'],row['from_source_id'] or '',row['from_transaction_id'] or ''))
        ids = {row['id']: f'generated-link-{index}' for index,row in enumerate(generated)}
        assert len(ids) == len(set(ids.values())) == len(generated)
        normalized = json.loads(_normalize(json.dumps(records, sort_keys=True).encode(),ids))
        return {table:sorted(rows,key=lambda row:json.dumps(row,sort_keys=True)) for table,rows in normalized.items()},ids
    if transport == 'cli':
        operations = [
            ['create','--from-transaction','in','--to-transaction','out','--type','self_transfer','--allocation-amount','0.00000100001','--from-amount','0.00000100002','--attachment','a-a','--attachment','a-z'],
            ['create','--from-source','Purchase Ω','--to-transaction','out','--type','manual_source','--state','suggested','--confidence','weak','--method','exact method','--allocation-amount','0.00000000001'],
            ['create','--from-transaction','in','--to-transaction','out','--type','self_transfer','--uses-chain-observation','--chain-data-confirmed'],
            ['create','--from-source','s-z','--to-transaction','out','--type','missing_history','--attachment','a-a','--attachment','missing'],
            ['create','--from-source','s-other','--to-transaction','out','--type','manual_source'],
            ['create','--from-transaction','out','--to-transaction','in','--type','self_transfer'],
            ['create','--to-transaction','out','--type','self_transfer'],
            ['create','--from-transaction','in','--to-transaction','out','--type','self_transfer','--allocation-amount','2'],
            ['review','--link','l-z','--state','reviewed'],
            ['review','--link','l-z','--state','rejected'],
            ['review','--link','l-z','--state','rejected','--allocation-amount','0'],
            ['review','--link','l-z','--allocation-amount','2'],
            ['review','--link','l-z','--explanation','','--no-chain-observation','--unconfirmed-chain-data'],
            ['review','--link','l-z','--allocation-amount','0.00000000001','--from-amount','0.00000000002','--allocation-policy','explicit'],
            ['review','--link','l-z'],
            ['review','--link','l-other','--state','reviewed'],
            ['attach','--link','l-z','--attachment','a-a'],
            ['attach','--link','l-a','--attachment','a-a'],
            ['attach','--link','l-z','--attachment','a-other'],
            ['attach','--link','missing','--attachment','a-a'],
        ]
        for operation in operations:
            outputs,tables = [],[]
            for reference in [False,True]:
                shutil.copy2(baseline,database)
                result = call('--machine','source-funds','links',*operation,reference=reference)
                records,ids = state()
                outputs.append((result.returncode,_normalize(result.stdout,ids),result.stderr))
                tables.append(records)
            assert outputs[0] == outputs[1],operation
            assert tables[0] == tables[1],operation
            assert (evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest()) == original_file
        served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
        assert len(served) == len(operations)
        assert all(row['served_by'] == 'rust' and row['command'] in {'source-funds.links.create','source-funds.links.review','source-funds.links.attach'} for row in served)
    else:
        requests = [
            ('ui.source_funds.links.create',dict(from_transaction='in',to_transaction='out',link_type='self_transfer',allocation_amount='0.00000100001',from_allocation_amount='0.00000100002',attachment_ids=['a-a','a-z'])),
            ('ui.source_funds.links.create',dict(from_source='Purchase Ω',to_transaction='out',link_type='manual_source',method='other',allocation_amount='0.00000000001',uses_chain_observation=True,chain_data_confirmed=True)),
            ('ui.source_funds.links.create',dict(from_transaction='in',to_transaction='out',link_type='self_transfer',attachment_ids=['missing'])),
            ('ui.source_funds.links.create',dict(from_source='s-z',to_transaction='out',link_type='missing_history',attachment_ids=['a-a','missing'])),
            ('ui.source_funds.links.review',dict(link=' l-z ',state='accepted',expected_allocation_amount_msat=100001,expected_from_allocation_amount_msat=100002)),
            ('ui.source_funds.links.review',dict(link='l-z',state='reviewed',expected_allocation_amount_msat=0)),
            ('ui.source_funds.links.review',dict(link='l-z',state='reviewed',expected_from_allocation_amount_msat=0)),
            ('ui.source_funds.links.review',dict(link='l-z',state='reject',allocation_amount='0')),
            ('ui.source_funds.links.review',dict(link='l-z',allocation_amount='2')),
            ('ui.source_funds.links.review',dict(link='l-z',allocation_amount='0.00000000001',from_allocation_amount='0.00000000002',allocation_policy='explicit',explanation='',uses_chain_observation=False,chain_data_confirmed=False)),
            ('ui.source_funds.links.review',dict(link='l-z',state='rejected')),
            ('ui.source_funds.links.attach',dict(link=' l-z ',attachment_id=' a-a ')),
            ('ui.source_funds.links.attach',dict(link='l-a',attachment_id='a-a')),
            ('ui.source_funds.links.attach',dict(link='l-z',attachment_id='a-other')),
            ('ui.source_funds.links.attach',dict(link='l-other',attachment_id='a-a')),
            ('ui.source_funds.links.create',[]),
            ('ui.source_funds.links.review',dict(link='')),
            ('ui.source_funds.links.list',{}),
            ('ui.source_funds.cases.list',{}),
        ]
        wire = ''.join(json.dumps(dict(request_id=i,kind=kind,args=args))+'\n' for i,(kind,args) in enumerate(requests)).encode()+b'{"request_id":"done","kind":"daemon.shutdown"}\n'
        outputs,tables = [],[]
        for reference in [False,True]:
            shutil.copy2(baseline,database)
            result = call('daemon',reference=reference,wire=wire)
            records,ids = state()
            outputs.append((result.returncode,_normalize(b'\n'.join(result.stdout.splitlines()[1:]),ids),result.stderr))
            tables.append(records)
        assert outputs[0] == outputs[1]
        assert tables[0] == tables[1]
        assert (evidence.read_bytes(),hashlib.sha256(evidence.read_bytes()).hexdigest()) == original_file
        assert [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()] == [dict(command='daemon',served_by='rust')]
