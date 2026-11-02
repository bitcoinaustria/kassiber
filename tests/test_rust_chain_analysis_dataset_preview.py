"""Native local dataset preview, staged grants and preview receipts against unchanged Python."""
from __future__ import annotations

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

from tests.rust_sync_env import child_env
from tests.test_rust_chain_analysis_psbt_reads import OWNER, READS, Session

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('chain_analysis_dataset_preview_vectors', ROOT / 'crates/wallets/scripts/generate_chain_analysis_dataset_preview.py')
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)
REQUIRED = sorted(REFERENCE.MANIFEST)
OPTIONAL = ['source_url', 'observed_at', 'valid_from', 'valid_until', 'category', 'expected_active_id']


def sorted_manifests(value):
    """Pin the reference's hash-seed manifest order to the native sorted order."""
    if isinstance(value, list):
        return [sorted_manifests(v) for v in value]
    if not isinstance(value, dict):
        return value
    value = {k: sorted_manifests(v) for k, v in value.items()}
    if set(value) == set(REQUIRED + OPTIONAL):
        value = {k: value[k] for k in REQUIRED + OPTIONAL}
    return value


def test_dataset_preview_envelopes_and_all_112_tables_match_unchanged_python():
    cases = REFERENCE.vectors()
    assert cases == json.loads(REFERENCE.DEST.read_text())
    assert len(cases) >= 200
    assert all(s['state']['table_count'] == 112 for c in cases for s in c['expected'])
    kinds = Counter(s['result']['kind'] for c in cases for s in c['expected'])
    assert kinds['ui.chain_analysis.datasets.preview'] >= 60 and kinds['error'] >= 100


def test_dataset_manifest_hashseed_variants_are_recorded_without_changing_python():
    from tests.rust_port.repro_dataset_manifest_hashseed import variants
    recorded = json.loads((ROOT / 'crates/wallets/tests/data/chain_analysis_dataset_manifest_hashseeds.json').read_text())
    assert variants() == recorded
    assert len({tuple(v['manifest_keys']) for v in recorded.values()}) > 1
    assert len({v['canonical'] for v in recorded.values()}) == 1


@pytest.mark.parametrize('offline', [False, True])
def test_native_dataset_preview_envelopes_all_tables_without_network_consent(tmp_path, offline):
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


def scenario(session, files):
    records = []

    def keep(line, mask=()):
        value = sorted_manifests(json.loads(line))
        for path in mask:
            target = value
            for key in path[:-1]:
                target = target.get(key) if isinstance(target, dict) else None
            if isinstance(target, dict) and path[-1] in target:
                target[path[-1]] = '<varies>'
        if isinstance(value, dict):
            value.pop('request_id', None)
        records.append(json.dumps(value, ensure_ascii=False))
        return json.loads(line)

    def stage(path, purpose='dataset'):
        value = keep(session.call('internal.chain_analysis.stage', {'source_file': str(path), 'purpose': purpose}), [('data', 'source_token')])
        return value.get('data', {}).get('source_token')

    def settle(job):
        for _ in range(3000):
            value = json.loads(session.call('ui.chain_analysis.jobs.get', {'job_id': job}))
            if value.get('data', {}).get('status') != 'running':
                return value
            time.sleep(0.01)
        raise AssertionError('receipt did not settle')

    manifest = REFERENCE.MANIFEST
    good = stage(files['good'])
    jsonl = stage(files['jsonl'])
    psbt = stage(files['good'], 'psbt')
    bad = stage(files['bad'])
    for args in [{'manifest': manifest, 'source_token': good}, {'manifest': manifest, 'source_token': jsonl, 'format': 'jsonl'},
                 {'manifest': manifest, 'source_token': psbt}, {'manifest': manifest, 'source_token': bad},
                 {'manifest': manifest}, {'source_token': good}, {'manifest': manifest, 'source_token': good, 'format': 'xml'},
                 {'manifest': {**manifest, 'name': ''}, 'source_token': good},
                 {'manifest': manifest, 'source_token': good, 'expected_scope': {'workspace_id': 'ws', 'profile_id': 'x'}}]:
        keep(session.call('ui.chain_analysis.datasets.preview', args))
    for args in [{'manifest': manifest, 'source_token': good}, {'manifest': manifest, 'source_token': jsonl, 'format': 'jsonl', 'expected_sha256': 'x'},
                 {'manifest': manifest, 'source_token': bad}, {'manifest': manifest, 'source_token': good, 'adapter': 'other'}]:
        line = session.call('ui.chain_analysis.datasets.preview.start', args)
        keep(line, [('data', 'job_id'), ('data', 'status'), ('data', 'progress'), ('data', 'result'), ('data', 'elapsed_ms'), ('data', 'error_code')])
        settled = settle(json.loads(line)['data']['job_id'])
        keep(json.dumps(settled), [('data', 'job_id'), ('data', 'elapsed_ms')])
    for args in [{'manifest': manifest}, {'manifest': {**manifest, 'visibility': 'x'}, 'source_token': good},
                 {'manifest': manifest, 'source_token': psbt}, {'manifest': manifest, 'source_token': 'missing'},
                 {'manifest': manifest, 'source_token': good, 'extra': 1}]:
        keep(session.call('ui.chain_analysis.datasets.preview.start', args))
    # A changed file is refused; a lock forgets every grant and receipt.
    files['good'].write_bytes(files['good'].read_bytes() + b'\n')
    keep(session.call('ui.chain_analysis.datasets.preview', {'manifest': manifest, 'source_token': good}))
    keep(session.call('daemon.lock'))
    keep(session.call('ui.chain_analysis.datasets.preview', {'manifest': manifest, 'source_token': jsonl, 'format': 'jsonl'}))
    return records


@pytest.mark.parametrize('offline', [False, True])
def test_dataset_preview_cli_daemon_bytes_all_tables_and_zero_fallback(tmp_path, offline, monkeypatch):
    binary = os.environ.get('KASSIBER_RUST_BIN')
    if not binary:
        pytest.skip('requires native product test-harness build')
    from kassiber.db import open_db
    from tests import cli_runner
    from tests.test_rust_analysis_index import tables
    root = tmp_path / 'data'
    conn = open_db(root)
    conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text())
    db = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    conn.commit()
    conn.close()
    snapshot = tmp_path / 'snapshot.sqlite3'
    shutil.copyfile(db, snapshot)
    owners = tmp_path / 'owners'
    owners.mkdir(mode=0o700)
    f = tmp_path / 'files'
    f.mkdir()
    tx = 'a' * 64
    (f / 'good.csv').write_bytes(b'\xef\xbb\xbfSubject,Label,category\r\n' + b''.join(f'{i:064x},Entity "{i}",Exchange\r\n'.encode() for i in range(4500)))
    (f / 'rows.jsonl').write_text(''.join(json.dumps({'address': REFERENCE.ADDRESS, 'entity': f'E{i}', 'observed_at': '2026-01-01'}) + '\n' for i in range(12)))
    (f / 'bad.csv').write_bytes(b'subject,label\n' + tx.encode() + b',"open\n')
    (f / 'maru.jsonl').write_text(json.dumps({'hashadd': tx, 'exchange': 'X', 'mining': 'Y'}) + '\n')
    (f / 'manifest.json').write_text(json.dumps({'kind': 'k', 'schema_version': 1, 'data': REFERENCE.MANIFEST}))
    (f / 'empty.csv').write_bytes(b'')
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
            return tables(c)
        finally:
            c.close()

    def run(args, reference):
        shutil.copyfile(snapshot, db)
        result = cli_runner.run(['--machine', '--data-root', root, *args], reference=reference, env=env, capture_output=True, timeout=120)
        return result, state()

    manifest = json.dumps(REFERENCE.MANIFEST)
    commands = [
        ['--file', f / 'good.csv', '--manifest', manifest], ['--file', f / 'good.csv', '--manifest', '@' + str(f / 'manifest.json')],
        ['--file', f / 'rows.jsonl', '--manifest', manifest, '--source-format', 'jsonl', '--adapter', 'am_i_exposed'],
        ['--file', f / 'maru.jsonl', '--manifest', manifest, '--source-format', 'jsonl', '--adapter', 'maru92'],
        ['--file', f / 'bad.csv', '--manifest', manifest], ['--file', f / 'empty.csv', '--manifest', manifest],
        ['--file', f / 'missing.csv', '--manifest', manifest], ['--file', f, '--manifest', manifest],
        ['--file', f, '--manifest', 'not json'], ['--file', f / 'good.csv', '--manifest', '[]'],
        ['--file', f / 'good.csv', '--manifest', json.dumps({**REFERENCE.MANIFEST, 'source_url': 'https://u@x'})],
        ['--file', f / 'good.csv', '--manifest', manifest, '--profile', 'missing'],
    ]
    for command in commands:
        ref, before = run(['chain-analysis', 'datasets', 'preview', *command], True)
        native, after = run(['chain-analysis', 'datasets', 'preview', *command], False)
        assert (native.returncode, native.stderr) == (ref.returncode, ref.stderr), command
        parsed = json.loads(ref.stdout)
        assert json.dumps(parsed, indent=2).encode() + b'\n' == ref.stdout
        assert native.stdout == json.dumps(sorted_manifests(parsed), indent=2).encode() + b'\n', command
        assert after == before, command
    transcripts = {}
    for reference in (True, False):
        shutil.copyfile(snapshot, db)
        for name, target in [('good', 'good.csv'), ('jsonl', 'rows.jsonl'), ('bad', 'bad.csv')]:
            shutil.copyfile(f / target, tmp_path / f'session-{name}')
        files = {name: tmp_path / f'session-{name}' for name in ('good', 'jsonl', 'bad')}
        argv = [sys.executable, '-c', OWNER] if reference else [sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py')]
        session = Session([*argv, '--machine', '--data-root', str(root), 'daemon'], env)
        records = scenario(session, files)
        assert session.close() == b''
        transcripts[reference] = (records, state(), json.loads(session.ready)['data']['supported_kinds'])
    (tmp_path / 'reference.json').write_text('\n'.join(transcripts[True][0]))
    (tmp_path / 'native.json').write_text('\n'.join(transcripts[False][0]))
    assert transcripts[False][0] == transcripts[True][0]
    assert transcripts[False][1] == transcripts[True][1]
    assert {'ui.chain_analysis.datasets.preview', 'ui.chain_analysis.datasets.preview.start'} <= set(transcripts[False][2])
    calls = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    counts = Counter((c['command'], c['served_by']) for c in calls)
    assert all(c['served_by'] == 'rust' for c in calls)
    assert sum(counts.values()) == len(commands) + 1
    (tmp_path / 'served-summary.json').write_text(json.dumps({command: {'rust': n, 'python': 0} for (command, _), n in sorted(counts.items())}, indent=2) + '\n')


def test_dataset_preview_mutation_catalog_stays_applicable(tmp_path):
    path = ROOT / 'crates/wallets/scripts/chain_analysis_dataset_preview_mutations.json'
    entries = json.loads(path.read_text())
    result = subprocess.run([sys.executable, str(ROOT / 'crates/wallets/scripts/record_mutation_check.py'), '--mutations', str(path), '--check'],
                            env=child_env(tmp_path), check=True, capture_output=True, timeout=30)
    assert result.stdout == f'{len(entries)} entries apply\n'.encode()
    source = (ROOT / 'crates/wallets/src/analysis_index/reads/tests.rs').read_text()
    assert entries and all(f'fn {test}()' in source for entry in entries for test in entry['expect_failing'])
