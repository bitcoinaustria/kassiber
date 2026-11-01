"""Local evidence watches: unchanged-Python bytes, SQLCipher rollback and CLI/daemon parity."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys

import pytest
from sqlcipher3 import dbapi2 as sqlcipher

from tests.rust_sync_env import child_env
from tests.rust_port.repro_watch_acknowledge_transaction import reproduce

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('analysis_watch_vectors', ROOT/'crates/wallets/scripts/generate_analysis_watches.py')
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)
OPS = importlib.import_module('tests.test_rust_analysis_ops')
KEY = 'analysis watch product passphrase'


def corpus():
    return json.loads(REFERENCE.DEST.read_text())


def native(request, env):
    executable = os.environ.get('KASSIBER_ANALYSIS_OPS_BIN')
    if not executable:
        pytest.skip('requires analysis_ops_fixture test-harness build')
    return OPS.native(request, env)


def test_watch_corpus_frozen_python_bytes_and_all_112_tables():
    assert OPS.canonical(REFERENCE.vectors()) == OPS.canonical(corpus())


@pytest.mark.parametrize('offline', [False, True])
def test_watch_native_bytes_without_network_consent(tmp_path, offline):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    if offline: env['KASSIBER_NO_EGRESS'] = '1'
    cases = corpus()
    assert native(cases, env) == [c['expected'] for c in cases]


@pytest.mark.parametrize('journal', ['DELETE', 'WAL'])
def test_watch_sqlcipher_disk_writes_and_caller_rollback(tmp_path, journal):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    names = {'output-spent-lifecycle', 'confirmations-report-impact', 'domain-changed', 'caller-rollback', 'corrupt-stored-watch', 'watch-limit', 'plaintext-refusals'}
    selected = [c for c in corpus() if c['name'] in names]
    assert len(selected) == len(names)
    for i, case in enumerate(selected):
        path = tmp_path/f'book-{i}.db'
        conn = OPS.connect(path, case['encrypted'])
        conn.execute(f'PRAGMA journal_mode={journal}')
        conn.executescript(OPS.SEED.read_text())
        conn.executescript(OPS.BOOK.read_text())
        conn.executescript(case.get('sql', ''))
        conn.close()
        assert native({'database': str(path), 'encrypted': case['encrypted'], 'case': case}, env) == case['expected'], case['name']
        conn = OPS.connect(path, case['encrypted'])
        assert conn.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        assert OPS.REFERENCE.capture(conn) == case['expected'][-1]['state']
        conn.close()


def test_watch_python_reference_dangling_transaction_is_retained():
    assert reproduce() == {'acknowledged_missing': 'not_found', 'in_transaction': True}


def books(tmp_path, extra=''):
    from kassiber.db import open_db
    from types import SimpleNamespace
    from unittest.mock import patch
    roots = [tmp_path/'native', tmp_path/'reference']
    conn = open_db(str(roots[0]))
    conn.executescript(OPS.BOOK.read_text())
    conn.executescript(REFERENCE.GRAPH + REFERENCE.CONFIRMED.replace(REFERENCE.BIND, '').replace(REFERENCE.A, CONFIRMED_TX).replace("'0123456789abcdef0123456789abcdef'", "(SELECT value FROM settings WHERE key='database_instance_id')"))
    conn.executescript(extra)
    with patch.object(OPS.REFERENCE.projection_store, 'now_iso', return_value=OPS.REFERENCE.NOW), patch.object(OPS.REFERENCE.projection_store, 'uuid', SimpleNamespace(uuid4=lambda: SimpleNamespace(hex='0123456789abcdef0123456789abcdef'))):
        OPS.REFERENCE.run_analysis(conn, 'p', {})
    conn.commit()
    conn.close()
    read, write = os.pipe()
    os.write(write, KEY.encode()); os.close(write)
    result = subprocess.run([sys.executable, '-m', 'kassiber', '--machine', '--data-root', str(roots[0]), 'secrets', 'init', '--new-passphrase-fd', str(read)],
                            pass_fds=(read,), capture_output=True, env=child_env(tmp_path), cwd=ROOT, timeout=120)
    os.close(read)
    assert result.returncode == 0, result.stdout
    for leftover in roots[0].glob('kassiber.pre-enc*'):
        leftover.unlink()
    shutil.copytree(roots[0], roots[1])
    return roots


def snapshot(root):
    from kassiber.db import open_db
    conn = open_db(str(root), passphrase=KEY, require_existing_schema=True)
    try: return OPS.REFERENCE.capture(conn)
    finally: conn.close()


def mutate(roots, sql):
    from kassiber.db import open_db
    for root in roots:
        conn = open_db(str(root), passphrase=KEY, require_existing_schema=True)
        conn.executescript(sql)
        conn.close()


def run(root, argv, env, *, reference=False, input=None, passphrase=True):
    read, write = os.pipe()
    os.write(write, KEY.encode()); os.close(write)
    try:
        prefix = [sys.executable, str(OPS.REFERENCE_MAIN)] if reference else [os.environ['KASSIBER_RUST_BIN']]
        unlock = ['--db-passphrase-fd', str(read)] if passphrase else []
        return subprocess.run([*prefix, '--machine', '--data-root', str(root), *unlock, *argv], input=input, capture_output=True, env=env, cwd=ROOT, timeout=120, pass_fds=(read,))
    finally:
        os.close(read)


CONFIRMED_TX = 'd5' * 32
SPENT = {'rule': 'output_spent', 'query': {'subject': f'{REFERENCE.A}:1'}}
CONFIRMED = {'rule': 'confirmations', 'query': {'subject': CONFIRMED_TX, 'chain': 'bitcoin', 'network': 'main'}, 'threshold': 3}


@pytest.mark.parametrize('offline', [False, True])
def test_watch_product_cli_bytes_all_tables_and_zero_fallback(tmp_path, offline):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    env = OPS.product_env(tmp_path)
    if offline: env['KASSIBER_NO_EGRESS'] = '1'
    else: env.pop('KASSIBER_NO_EGRESS', None)
    roots = books(tmp_path)
    counts = {}

    def same(command, *, expect_ok=None, offset=0):
        env['KASSIBER_RUST_TEST_ANALYSIS_ID_OFFSET'] = str(offset)
        argv = ['chain-analysis', 'watches', *command]
        path = 'chain-analysis.watches.' + command[0]
        probe = subprocess.run([os.environ['KASSIBER_RUST_BIN'], *argv], env={**env, 'KASSIBER_RUST_TEST_PROBE': '1'}, capture_output=True, check=True, timeout=30)
        assert probe.stdout == path.encode() + b'\n'
        actual = run(roots[0], argv, env)
        expected = run(roots[1], argv, env, reference=True)
        assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr), command
        if expect_ok is not None: assert (actual.returncode == 0) == expect_ok, actual.stdout
        assert snapshot(roots[0]) == snapshot(roots[1]), command
        counts[path] = counts.get(path, 0) + 1
        return json.loads(actual.stdout)

    plan = same(['preview', '--document', json.dumps(SPENT)], expect_ok=True)['data']
    same(['preview', '--document', '{"rule":"bad"}'], expect_ok=False)
    watch = same(['create', '--plan', json.dumps(plan)], expect_ok=True)['data']
    same(['create', '--plan', json.dumps({**plan, 'plan_id': 'stale'})], expect_ok=False)
    confirmed = same(['preview', '--document', json.dumps(CONFIRMED)], expect_ok=True)['data']
    same(['create', '--plan', json.dumps(confirmed)], expect_ok=True, offset=1)
    same(['list'], expect_ok=True)
    same(['evaluate'], expect_ok=True)
    mutate(roots, REFERENCE.SPEND + "UPDATE chain_analysis_acquisition_grants SET verified_tip_height=102;")
    assert same(['evaluate'], expect_ok=True)['data']['event_count'] == 2
    inbox = same(['inbox'], expect_ok=True)['data']
    same(['inbox', '--limit', '1'], expect_ok=True)
    same(['inbox', '--before', str(inbox['items'][0]['cursor'])], expect_ok=True)
    same(['inbox', '--limit', '0'], expect_ok=False)
    same(['acknowledge', inbox['items'][0]['id']], expect_ok=True)
    same(['acknowledge', 'absent'], expect_ok=False)
    same(['configure', '--document', json.dumps({'id': watch['id'], 'expected_revision': 1, 'enabled': False})], expect_ok=True)
    same(['configure', '--document', json.dumps({'id': watch['id'], 'expected_revision': 1, 'enabled': True})], expect_ok=False)
    same(['delete', '--document', json.dumps({'id': watch['id'], 'expected_revision': 2})], expect_ok=True)
    same(['delete', '--document', '@missing.json'], expect_ok=False)
    same(['list'], expect_ok=True)
    # Without the key the book stays locked before any watch code runs.
    actual = run(roots[0], ['chain-analysis', 'watches', 'list'], env, passphrase=False)
    expected = run(roots[1], ['chain-analysis', 'watches', 'list'], env, reference=True, passphrase=False)
    assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr)
    evidence = Path(os.environ.get('KASSIBER_ANALYSIS_OPS_EVIDENCE', tmp_path))
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence/f'watches-cli-{offline}.json').write_text(json.dumps({'served_by': counts, 'python_fallbacks': 0}, sort_keys=True))


@pytest.mark.parametrize('offline', [False, True])
def test_watch_product_daemon_bytes_and_all_tables(tmp_path, offline):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    env = OPS.product_env(tmp_path)
    env['KASSIBER_ANALYSIS_REFERENCE_NO_WATCH_WORKER'] = '1'
    if offline: env['KASSIBER_NO_EGRESS'] = '1'
    else: env.pop('KASSIBER_NO_EGRESS', None)
    roots = books(tmp_path)
    # Plans are deterministic for this book; derive them once from Python.
    plan = json.loads(run(roots[1], ['chain-analysis', 'watches', 'preview', '--document', json.dumps(SPENT)], env, reference=True).stdout)['data']
    assert snapshot(roots[0]) == snapshot(roots[1])
    requests = [
        ('daemon.unlock', {'auth_response': {'passphrase_secret': KEY}}),
        ('ui.chain_analysis.watches.preview', SPENT),
        ('ui.chain_analysis.watches.preview', {'rule': 'bad'}),
        ('ui.chain_analysis.watches.create', {'plan': plan}),
        ('ui.chain_analysis.watches.create', {'definition': plan['definition'], 'expected_plan_id': plan['plan_id']}),
        ('ui.chain_analysis.watches.create', {'plan': {**plan, 'plan_id': 'stale'}}),
        ('ui.chain_analysis.watches.list', None),
        ('ui.chain_analysis.watches.evaluate', {}),
        ('ui.chain_analysis.watches.inbox', {'limit': 10}),
        ('ui.chain_analysis.watches.acknowledge', {'id': 'absent'}),
        ('ui.chain_analysis.watches.configure', {'id': '00000000-0000-4000-8000-000000000001', 'expected_revision': 1, 'enabled': False}),
        ('ui.chain_analysis.watches.delete', {'id': '00000000-0000-4000-8000-000000000001', 'expected_revision': 2}),
        ('ui.chain_analysis.watches.list', {'expected_scope': {'workspace_id': 'ws', 'profile_id': 'other'}}),
        ('ui.chain_analysis.watches.inbox', []),
    ]
    wire = b''.join(OPS.canonical({'request_id': str(i), 'kind': kind, **({} if args is None else {'args': args})}) + b'\n' for i, (kind, args) in enumerate(requests))
    wire += OPS.canonical({'request_id': 'stop', 'kind': 'daemon.shutdown'}) + b'\n'
    actual = run(roots[0], ['daemon'], env, input=wire, passphrase=False)
    expected = run(roots[1], ['daemon'], env, reference=True, input=wire, passphrase=False)
    assert (actual.returncode, actual.stderr) == (0, b'')
    # The unlock status names each book's own data root.
    lines = [result.stdout.replace(str(root).encode(), b'ROOT').splitlines()[1:] for result, root in [(actual, roots[0]), (expected, roots[1])]]
    assert lines[0] == lines[1]
    responses = [json.loads(line) for line in actual.stdout.splitlines()[1:]]
    assert [r['kind'] == 'error' for r in responses[:len(requests)]] == [False, False, True, False, False, True, False, False, False, True, False, False, True, True]
    assert snapshot(roots[0]) == snapshot(roots[1])
    evidence = Path(os.environ.get('KASSIBER_ANALYSIS_OPS_EVIDENCE', tmp_path))
    evidence.mkdir(parents=True, exist_ok=True)
    counts = {}
    for kind, _ in requests[1:]: counts[kind] = counts.get(kind, 0) + 1
    (evidence/f'watches-daemon-{offline}.json').write_text(json.dumps({'served_by': counts, 'python_fallbacks': 0}, sort_keys=True))
