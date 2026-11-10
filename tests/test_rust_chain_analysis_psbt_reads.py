"""Native PSBT reads, staged sources and analysis receipts against unchanged Python."""
from __future__ import annotations

import base64
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import time

import pytest

from tests.rust_sync_parity import fixed_elapsed_ms
from tests.rust_sync_env import child_env

ROOT = Path(__file__).resolve().parents[1]


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


REFERENCE = _load('chain_analysis_psbt_reads_vectors', 'crates/wallets/scripts/generate_chain_analysis_psbt_reads.py')
READS = _load('chain_analysis_reads_vectors_for_psbt', 'crates/wallets/scripts/generate_chain_analysis_reads.py')
OWNER = ('import os,sys; from pathlib import Path; from kassiber.operator import project; '
         'project._owner_lock_root=lambda:Path(os.environ["KASSIBER_RUST_TEST_OWNER_ROOT"]); '
         'from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))')


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True)


def test_psbt_read_envelopes_and_all_112_tables_match_unchanged_python():
    cases = REFERENCE.vectors()
    assert cases == json.loads(REFERENCE.DEST.read_text())
    assert len(cases) >= 250
    assert all(s['state']['table_count'] == 112 for c in cases for s in c['expected'])
    statuses = Counter(check['status'] for c in cases for s in c['expected'] if s['result']['kind'] != 'error'
                       for check in s['result']['data'].get('payjoin', {}).get('checks', []))
    assert set(statuses) == {'passed', 'failed', 'unavailable'}


@pytest.mark.parametrize('offline', [False, True])
def test_native_psbt_read_envelopes_all_tables_without_network_consent(tmp_path, offline):
    executable = os.environ.get('KASSIBER_CHAIN_ANALYSIS_READS_BIN')
    if not executable:
        pytest.skip('requires chain_analysis_reads_fixture test-harness build')
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    if offline:
        env['KASSIBER_NO_EGRESS'] = '1'
    cases = json.loads(REFERENCE.DEST.read_text())
    result = subprocess.run([executable], input=json.dumps(cases).encode(), env=env, capture_output=True, check=True, timeout=300)
    assert result.stderr == b''
    assert result.stdout == READS.canonical([c['expected'] for c in cases]).encode() + b'\n'


def test_psbt_read_mutation_catalog_stays_applicable(tmp_path):
    path = ROOT / 'crates/wallets/scripts/chain_analysis_psbt_reads_mutations.json'
    entries = json.loads(path.read_text())
    result = subprocess.run([sys.executable, str(ROOT / 'crates/wallets/scripts/record_mutation_check.py'), '--mutations', str(path), '--check'],
                            env=child_env(tmp_path), check=True, capture_output=True, timeout=30)
    assert result.stdout == f'{len(entries)} entries apply\n'.encode()
    source = (ROOT / 'crates/wallets/src/analysis_index/reads/tests.rs').read_text()
    assert entries and all(f'fn {test}()' in source for entry in entries for test in entry['expect_failing'])


class Session:
    """One interactive JSONL daemon; requests are answered in order."""

    def __init__(self, argv, env):
        self.process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, cwd=ROOT)
        self.ready = self.process.stdout.readline()
        self.count = 0

    def call(self, kind, args=None):
        self.count += 1
        request = {'request_id': self.count, 'kind': kind}
        if args is not None:
            request['args'] = args
        self.process.stdin.write((json.dumps(request) + '\n').encode())
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        assert line, self.process.stderr.read()[-2000:]
        return line

    def close(self):
        self.call('daemon.shutdown')
        self.process.stdin.close()
        assert self.process.wait(timeout=60) == 0
        return self.process.stderr.read()


def entropy_heavy_psbt():
    from embit.psbt import PSBT
    from embit.script import Script
    from embit.transaction import Transaction, TransactionInput, TransactionOutput
    script = Script(REFERENCE.SCRIPT)
    tx = Transaction(vin=[TransactionInput(bytes([i + 1]) * 32, 0) for i in range(12)],
                     vout=[TransactionOutput(9_900 + i, script) for i in range(12)])
    psbt = PSBT(tx)
    for i, row in enumerate(psbt.inputs):
        row.witness_utxo = TransactionOutput(10_000 + i, script)
    return psbt.to_base64()


def scenario(session, files, tx):
    """The same request script for both daemons; returns comparable records."""
    records = []
    def keep(line, mask=()):
        value = fixed_elapsed_ms(json.loads(line))
        for path in mask:
            target = value
            for key in path[:-1]:
                target = target.get(key) if isinstance(target, dict) else None
                if target is None:
                    break
            if isinstance(target, dict) and path[-1] in target:
                target[path[-1]] = '<varies>'
        # Receipt polling makes request counts differ; responses stay ordered.
        if isinstance(value, dict):
            value.pop('request_id', None)
        records.append(value)
        return json.loads(line)
    def staged(line):
        value = keep(line, [('data', 'source_token')])
        return value.get('data', {}).get('source_token')
    def receipt(line):
        value = json.loads(line)
        return value.get('data', {}).get('job_id')
    def settle(job):
        for _ in range(3000):
            value = json.loads(session.call('ui.chain_analysis.jobs.get', {'job_id': job}))
            if value.get('data', {}).get('status') != 'running':
                return value
            time.sleep(0.01)
        raise AssertionError('receipt did not settle')
    def final(value, *, volatile=False):
        mask = [('data', 'job_id')]
        if volatile:
            mask += [('data', 'progress'), ('data', 'result'), ('data', 'status')]
        keep(json.dumps(value), mask)
    token = staged(session.call('internal.chain_analysis.stage', {'source_file': str(files['psbt']), 'purpose': 'psbt'}))
    after = staged(session.call('internal.chain_analysis.stage', {'source_file': str(files['after']), 'purpose': 'psbt', 'expected_scope': {'workspace_id': 'ws', 'profile_id': 'p'}}))
    dataset = staged(session.call('internal.chain_analysis.stage', {'source_file': str(files['psbt']), 'purpose': 'dataset'}))
    for args in [{'source_file': str(files['psbt']), 'purpose': 'other'}, {'source_file': str(files['empty']), 'purpose': 'psbt'},
                 {'source_file': str(files['missing']), 'purpose': 'psbt'}, {'source_file': str(files['directory']), 'purpose': 'dataset'},
                 {'source_file': 5, 'purpose': 'psbt'}, {'source_file': str(files['psbt'])},
                 {'source_file': str(files['psbt']), 'purpose': 'psbt', 'expected_scope': {'workspace_id': 'ws', 'profile_id': 'other'}},
                 {'source_file': str(files['psbt']), 'purpose': 'psbt', 'extra': True}]:
        keep(session.call('internal.chain_analysis.stage', args))
    keep(session.call('internal.chain_analysis.stage', []))
    for kind, args in [
        ('psbt.analyze', {'psbt_token': token, 'network': 'main'}),
        ('psbt.analyze', {'psbt_token': dataset, 'network': 'main'}),
        ('psbt.analyze', {'psbt_token': token, 'psbt': 'x', 'network': 'main'}),
        ('psbt.analyze', {'psbt': files['text'], 'network': 'test'}),
        ('psbt.compare', {'before_token': token, 'after_token': after, 'network': 'test'}),
        ('psbt.compare', {'before': files['original'], 'after_token': after, 'network': 'test', 'payjoin': files['constraints']}),
        ('psbt.entropy', {'psbt_token': token, 'network': 'main', 'max_states': 5}),
        ('psbt.entropy', {'psbt': files['text'], 'network': 'main', 'scenario': {'bad': 1}}),
        ('psbt.entropy.start', {'psbt': 'zz', 'network': 'main'}),
        ('psbt.entropy.start', {'psbt': files['text'], 'network': 'main', 'max_states': 0}),
        ('entropy.start', {'subject': 'absent'}),
        ('entropy.start', {'subject': tx, 'unknown': 1}),
        ('jobs.get', {}), ('jobs.get', {'job_id': 'missing'}), ('jobs.get', {'job_id': 5}),
        ('jobs.cancel', {'job_id': 'missing'}), ('jobs.get', {'job_id': 'x', 'extra': 1}),
    ]:
        keep(session.call('ui.chain_analysis.' + kind, args))
    started = []
    for kind, args in [('psbt.entropy.start', {'psbt_token': token, 'network': 'main', 'max_states': 7}),
                       ('psbt.entropy.start', {'psbt': files['text'], 'network': 'main', 'scenario': {'kind': 'coinjoin_intrafees', 'max_received_fee_msat': '1000', 'max_paid_fee_msat': '1000'}}),
                       ('entropy.start', {'subject': tx, 'max_states': 3}),
                       ('entropy.start', {'subject': tx, 'observer': 'public', 'expected_scope': {'workspace_id': 'ws', 'profile_id': 'p'}}),
                       ('entropy.start', {'subject': 'cold'})]:
        line = session.call('ui.chain_analysis.' + kind, args)
        started.append(receipt(line))
        keep(line, [('data', 'job_id'), ('data', 'status'), ('data', 'progress'), ('data', 'result'), ('data', 'error_code')])
        final(settle(started[-1]))
    final(json.loads(session.call('ui.chain_analysis.jobs.cancel', {'job_id': started[0]})))
    # Cancellation races completion; both settle without an error code.
    heavy = {'psbt': files['heavy'], 'network': 'main', 'max_states': 2000000, 'max_duration_ms': 30000}
    job = receipt(session.call('ui.chain_analysis.psbt.entropy.start', heavy))
    cancelled = json.loads(session.call('ui.chain_analysis.jobs.cancel', {'job_id': job}))
    settled = settle(job)['data']
    assert cancelled['data']['request'] == settled['request']
    assert (settled['status'], (settled['result'] or {}).get('status')) in {('cancelled', 'cancelled'), ('completed', 'model_bounded'), ('cancelled', None)}
    keep(json.dumps({'error_code': settled['error_code'], 'request': settled['request']}))
    # A lock closes the book; receipts and grants never survive it.
    keep(session.call('daemon.lock'))
    keep(session.call('ui.chain_analysis.jobs.get', {'job_id': started[1]}))
    keep(session.call('ui.chain_analysis.psbt.analyze', {'psbt_token': token, 'network': 'main'}))
    return records


@pytest.mark.parametrize('offline', [False, True])
def test_psbt_reads_and_receipts_cli_daemon_bytes_all_tables_and_zero_fallback(tmp_path, offline, monkeypatch):
    binary = os.environ.get('KASSIBER_RUST_BIN')
    if not binary:
        pytest.skip('requires native product test-harness build')
    from kassiber.db import open_db
    from tests import cli_runner
    from tests.test_rust_analysis_index import tables
    from tests.rust_port.book_schema import TABLES
    root = tmp_path / 'data'
    conn = open_db(root)
    cases = json.loads(READS.DEST.read_text())
    seed = next(c for c in cases if c['name'] == 'cases-read-0')['sql']
    conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + seed)
    conn.close()
    conn = open_db(root)
    READS.evaluate({'actions': [{'operation': 'query'}]}, conn)
    conn.commit()
    db = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    conn.close()
    snapshot = tmp_path / 'snapshot.sqlite3'
    shutil.copyfile(db, snapshot)
    owners = tmp_path / 'owners'
    owners.mkdir(mode=0o700)
    files_root = tmp_path / 'files'
    files_root.mkdir()
    vectors = {c['name']: c for c in json.loads(REFERENCE.DEST.read_text())}
    original = REFERENCE.PAYJOIN['Original PSBT']
    proposal = REFERENCE.PAYJOIN['payjoin proposal']
    text = vectors['analyze-mixed-signatures-main']['actions'][0]['args']['psbt']
    (files_root / 'a.psbt').write_bytes(base64.b64decode(text))
    (files_root / 'after.psbt').write_text(proposal)
    (files_root / 'original.psbt').write_bytes(base64.b64decode(original))
    (files_root / 'hex.psbt').write_text(base64.b64decode(text).hex())
    (files_root / 'bad.psbt').write_bytes(b'\xff\xfe')
    (files_root / 'empty.psbt').write_bytes(b'')
    constraints = {'payment_output_index': 1, 'additional_fee_output_index': 0, 'max_additional_fee_contribution_msat': '182000'}
    (files_root / 'payjoin.json').write_text(json.dumps({'kind': 'x', 'schema_version': 1, 'data': constraints}))
    (files_root / 'scenario.json').write_text(json.dumps({'kind': 'coinjoin_intrafees', 'max_received_fee_msat': '1000', 'max_paid_fee_msat': '1000'}))
    files = {'psbt': files_root / 'a.psbt', 'after': files_root / 'after.psbt', 'empty': files_root / 'empty.psbt',
             'missing': files_root / 'missing.psbt', 'directory': files_root, 'text': text, 'original': original,
             'constraints': constraints, 'heavy': entropy_heavy_psbt()}
    env = child_env(tmp_path)
    env.update({'KASSIBER_RUST_TEST_OWNER_ROOT': str(owners), 'KASSIBER_TEST_NO_EGRESS': 'strict',
                'KASSIBER_HYBRID_RUST_BIN': str(Path(binary).resolve()), 'KASSIBER_HYBRID_LOG': str(tmp_path / 'served.jsonl'),
                'KASSIBER_HYBRID_RUST_DAEMON': '1'})
    if offline:
        env['KASSIBER_NO_EGRESS'] = '1'
    monkeypatch.setenv('KASSIBER_TEST_CLI', json.dumps([sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py')]))
    original_prefix = cli_runner.prefix
    monkeypatch.setattr(cli_runner, 'prefix', lambda *, reference=False, python_options=(): (sys.executable, '-c', OWNER) if reference else original_prefix(python_options=python_options))

    def state():
        c = sqlite3.connect(db)
        try:
            # The CLI migrates the book to the current schema.
            return tables(c, TABLES)
        finally:
            c.close()

    def run(args, reference):
        shutil.copyfile(snapshot, db)
        result = cli_runner.run(['--machine', '--data-root', root, *args], reference=reference, env=env, capture_output=True, timeout=120)
        return result, state()

    f = files_root
    commands = [
        ['analyze', '--file', f / 'a.psbt', '--network', 'main'], ['analyze', '--file', f / 'hex.psbt', '--network', 'test'],
        ['analyze', '--file', f / 'after.psbt', '--network', 'regtest'], ['analyze', '--file', f / 'bad.psbt', '--network', 'main'],
        ['analyze', '--file', f / 'missing.psbt', '--network', 'main'], ['analyze', '--file', f, '--network', 'main'],
        ['analyze', '--file', f / 'empty.psbt', '--network', 'signet'],
        ['compare', '--before', f / 'original.psbt', '--after', f / 'after.psbt', '--network', 'test', '--payjoin', '@' + str(f / 'payjoin.json')],
        ['compare', '--before', f / 'original.psbt', '--after', f / 'after.psbt', '--network', 'test', '--payjoin', '{"payment_output_index": 9}'],
        ['compare', '--before', f / 'a.psbt', '--after', f / 'missing.psbt', '--network', 'main', '--payjoin', 'not json'],
        ['compare', '--before', f / 'a.psbt', '--after', f / 'a.psbt', '--network', 'main'],
        ['compare', '--before', f / 'a.psbt', '--after', f / 'a.psbt', '--network', 'main', '--payjoin', '[1]'],
        ['entropy', '--file', f / 'a.psbt', '--network', 'main'],
        ['entropy', '--file', f / 'a.psbt', '--network', 'main', '--max-states', '2', '--scenario', '@' + str(f / 'scenario.json')],
        ['entropy', '--file', f / 'a.psbt', '--network', 'main', '--max-duration-ms', '0'],
        ['entropy', '--file', f / 'bad.psbt', '--network', 'main', '--scenario', 'bad'],
        ['analyze', '--file', f / 'a.psbt', '--network', 'main', '--profile', 'missing'],
    ]
    for command in commands:
        ref, before = run(['chain-analysis', 'psbt', *command], True)
        native, after = run(['chain-analysis', 'psbt', *command], False)
        assert (native.returncode, native.stdout, native.stderr) == (ref.returncode, ref.stdout, ref.stderr), command
        assert after == before, command
    transcripts = {}
    for reference in (True, False):
        shutil.copyfile(snapshot, db)
        argv = [sys.executable, '-c', OWNER] if reference else [sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py')]
        session = Session([*argv, '--machine', '--data-root', str(root), 'daemon'], env)
        records = scenario(session, files, READS.TX)
        assert session.close() == b''
        ready = json.loads(session.ready)['data']['supported_kinds']
        transcripts[reference] = (records, state(), ready)
    (reference_records, reference_state, _), (native_records, native_state, native_ready) = transcripts[True], transcripts[False]
    (tmp_path / 'reference.json').write_text(json.dumps(reference_records, indent=1))
    (tmp_path / 'native.json').write_text(json.dumps(native_records, indent=1))
    assert len(native_records) == len(reference_records)
    for index, (a, b) in enumerate(zip(native_records, reference_records)):
        assert a == b, index
    assert native_state == reference_state
    kinds = ['psbt.analyze', 'psbt.compare', 'psbt.entropy', 'entropy.start', 'psbt.entropy.start', 'jobs.get', 'jobs.cancel']
    assert all('ui.chain_analysis.' + kind in native_ready for kind in kinds)
    assert 'internal.chain_analysis.stage' in native_ready
    calls = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    counts = Counter((c['command'], c['served_by']) for c in calls)
    assert all(c['served_by'] == 'rust' for c in calls)
    assert sum(counts.values()) == len(commands) + 1
    (tmp_path / 'served-summary.json').write_text(json.dumps({command: {'rust': n, 'python': 0} for (command, _), n in sorted(counts.items())}, indent=2) + '\n')
