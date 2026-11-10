"""Recurring acquisition grants: unchanged-Python bytes, rollback and CLI/daemon parity without egress."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess

import pytest

from tests.rust_sync_env import child_env
from tests import test_rust_analysis_watches as WATCHES

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('analysis_grant_vectors', ROOT/'crates/wallets/scripts/generate_analysis_grants.py')
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)
OPS = WATCHES.OPS


def corpus():
    return json.loads(REFERENCE.DEST.read_text())


def test_grant_corpus_frozen_python_bytes_and_all_112_tables():
    assert OPS.canonical(REFERENCE.vectors()) == OPS.canonical(corpus())


@pytest.mark.parametrize('offline', [False, True])
def test_grant_native_bytes_without_network_consent(tmp_path, offline):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    if offline: env['KASSIBER_NO_EGRESS'] = '1'
    cases = corpus()
    assert WATCHES.native(cases, env) == [c['expected'] for c in cases]


@pytest.mark.parametrize('journal', ['DELETE', 'WAL'])
def test_grant_sqlcipher_disk_writes_and_caller_rollback(tmp_path, journal):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    names = {'grant-lifecycle', 'caller-rollback', 'removed-source', 'plaintext-authorize'}
    selected = [c for c in corpus() if c['name'] in names]
    assert len(selected) == len(names)
    for i, case in enumerate(selected):
        path = tmp_path/f'book-{i}.db'
        conn = OPS.connect(path, case['encrypted'])
        conn.execute(f'PRAGMA journal_mode={journal}')
        conn.executescript(OPS.SEED.read_text())
        conn.executescript(OPS.BOOK.read_text())
        conn.executescript(case['sql'])
        conn.close()
        assert WATCHES.native({'database': str(path), 'encrypted': case['encrypted'], 'case': case}, env) == case['expected'], case['name']
        conn = OPS.connect(path, case['encrypted'])
        assert conn.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        assert OPS.REFERENCE.capture(conn) == case['expected'][-1]['state']
        conn.close()


SUBJECT = {'backend': 'core', 'network': 'main', 'mode': 'subject', 'subject': 'a1' * 32, 'max_requests': 10, 'max_bytes': 8 * 1024 * 1024}
BLOCKS = {'backend': 'core', 'network': 'main', 'mode': 'blocks', 'start_height': 100, 'max_requests': 100, 'max_bytes': 10 ** 9}


def grant_env(tmp_path, offline):
    env = OPS.product_env(tmp_path)
    env['KASSIBER_RUST_TEST_ANALYSIS_EPOCH'] = str(REFERENCE.EPOCH)
    env['KASSIBER_ANALYSIS_REFERENCE_NO_WATCH_WORKER'] = env['KASSIBER_RUST_TEST_NO_WATCH_WORKER'] = '1'
    if offline: (tmp_path/'offline.json').write_text(json.dumps({'schema_version': 1, 'enabled': True}))
    return env


@pytest.mark.parametrize('offline', [False, True])
def test_grant_product_cli_bytes_all_tables_and_zero_fallback(tmp_path, offline):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    env = grant_env(tmp_path, offline)
    roots = WATCHES.books(tmp_path, REFERENCE.backend())
    counts = {}

    def same(command, *, expect_ok=None, offset=0):
        env['KASSIBER_RUST_TEST_ANALYSIS_ID_OFFSET'] = str(offset)
        argv = ['chain-analysis', 'sources', *command]
        path = 'chain-analysis.sources.' + command[0]
        probe = subprocess.run([os.environ['KASSIBER_RUST_BIN'], *argv], env={**env, 'KASSIBER_RUST_TEST_PROBE': '1'}, capture_output=True, check=True, timeout=30)
        assert probe.stdout == path.encode() + b'\n'
        actual = WATCHES.run(roots[0], argv, env)
        expected = WATCHES.run(roots[1], argv, env, reference=True)
        assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr), command
        if expect_ok is not None: assert (actual.returncode == 0) == expect_ok, actual.stdout
        assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1]), command
        counts[path] = counts.get(path, 0) + 1
        return json.loads(actual.stdout)

    plan = same(['plan', '--document', json.dumps(SUBJECT)], expect_ok=True)['data']
    same(['plan', '--document', json.dumps({**SUBJECT, 'max_requests': 3})], expect_ok=False)
    grant = same(['authorize', '--plan', json.dumps(plan)], expect_ok=True)['data']
    same(['authorize', '--plan', json.dumps({**plan, 'plan_id': 'stale'})], expect_ok=False)
    blocks = same(['plan', '--document', json.dumps(BLOCKS)], expect_ok=True)['data']
    same(['authorize', '--plan', json.dumps(blocks)], expect_ok=True, offset=1)
    same(['list'], expect_ok=True)
    same(['list', '--limit', '1'], expect_ok=True)
    same(['list', '--after-id', grant['id']], expect_ok=True)
    same(['run', grant['id']], expect_ok=not offline)
    same(['run', 'absent'], expect_ok=False)
    same(['revoke', grant['id'], '--expected-revision', '2'], expect_ok=False)
    same(['revoke', grant['id'], '--expected-revision', '1'], expect_ok=True)
    same(['run', grant['id']], expect_ok=False)
    same(['list'], expect_ok=True)
    evidence = Path(os.environ.get('KASSIBER_ANALYSIS_OPS_EVIDENCE', tmp_path))
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence/f'sources-cli-{offline}.json').write_text(json.dumps({'served_by': counts, 'python_fallbacks': 0}, sort_keys=True))


@pytest.mark.parametrize('offline', [False, True])
def test_grant_product_daemon_bytes_and_all_tables(tmp_path, offline):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    env = grant_env(tmp_path, offline)
    roots = WATCHES.books(tmp_path, REFERENCE.backend())
    plan = json.loads(WATCHES.run(roots[1], ['chain-analysis', 'sources', 'plan', '--document', json.dumps(SUBJECT)], env, reference=True).stdout)['data']
    grant = '00000000000040008000000000000001'
    requests = [
        ('daemon.unlock', {'auth_response': {'passphrase_secret': WATCHES.KEY}}),
        ('ui.chain_analysis.sources.plan', SUBJECT),
        ('ui.chain_analysis.sources.plan', {**SUBJECT, 'mode': 'bad'}),
        ('ui.chain_analysis.sources.authorize', {'plan': plan}),
        ('ui.chain_analysis.sources.authorize', {'plan': {**plan, 'plan_id': 'stale'}}),
        ('ui.chain_analysis.sources.list', None),
        ('ui.chain_analysis.sources.run', {'id': grant}),
        ('ui.chain_analysis.sources.revoke', {'id': grant, 'expected_revision': 1}),
        ('ui.chain_analysis.sources.run', {'id': grant}),
        ('ui.chain_analysis.sources.list', {'expected_scope': {'workspace_id': 'ws', 'profile_id': 'other'}}),
    ]
    wire = b''.join(OPS.canonical({'request_id': str(i), 'kind': kind, **({} if args is None else {'args': args})}) + b'\n' for i, (kind, args) in enumerate(requests))
    wire += OPS.canonical({'request_id': 'stop', 'kind': 'daemon.shutdown'}) + b'\n'
    actual = WATCHES.run(roots[0], ['daemon'], env, input=wire, passphrase=False)
    expected = WATCHES.run(roots[1], ['daemon'], env, reference=True, input=wire, passphrase=False)
    assert (actual.returncode, actual.stderr) == (0, b'')
    lines = [result.stdout.replace(str(root).encode(), b'ROOT').splitlines()[1:] for result, root in [(actual, roots[0]), (expected, roots[1])]]
    assert lines[0] == lines[1]
    responses = [json.loads(line) for line in actual.stdout.splitlines()[1:]]
    assert [r['kind'] == 'error' for r in responses[:len(requests)]] == [False, False, True, False, True, False, offline, False, True, True]
    assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1])
    evidence = Path(os.environ.get('KASSIBER_ANALYSIS_OPS_EVIDENCE', tmp_path))
    evidence.mkdir(parents=True, exist_ok=True)
    counts = {}
    for kind, _ in requests[1:]: counts[kind] = counts.get(kind, 0) + 1
    (evidence/f'sources-daemon-{offline}.json').write_text(json.dumps({'served_by': counts, 'python_fallbacks': 0}, sort_keys=True))
