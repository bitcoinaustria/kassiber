"""Acquisition plans: unchanged-Python bytes and CLI/daemon parity; planning never contacts a backend."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess

import pytest

from tests.rust_sync_env import child_env
from tests import test_rust_analysis_watches as WATCHES
from tests import test_rust_analysis_grants as GRANTS

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('analysis_acquisition_vectors', ROOT/'crates/wallets/scripts/generate_analysis_acquisition.py')
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)
OPS = WATCHES.OPS


def corpus():
    return json.loads(REFERENCE.DEST.read_text())


def test_acquisition_plan_corpus_frozen_python_bytes_and_all_112_tables():
    assert OPS.canonical(REFERENCE.vectors()) == OPS.canonical(corpus())


@pytest.mark.parametrize('offline', [False, True])
def test_acquisition_plan_native_bytes_without_network_consent(tmp_path, offline):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    if offline: env['KASSIBER_NO_EGRESS'] = '1'
    cases = corpus()
    assert WATCHES.native(cases, env) == [c['expected'] for c in cases]


PLAN = {'backend': 'core', 'subject': REFERENCE.A, 'chain': 'bitcoin', 'network': 'main', 'direction': 'both'}


@pytest.mark.parametrize('offline', [False, True])
def test_acquisition_plan_product_cli_and_daemon_bytes(tmp_path, offline):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    env = GRANTS.grant_env(tmp_path, offline)
    roots = WATCHES.books(tmp_path, REFERENCE.BACKENDS)
    counts = {}
    for command in [['acquire', 'plan', REFERENCE.A, '--backend', 'Core', '--chain', 'bitcoin', '--network', 'main'],
                    ['acquire', 'plan', REFERENCE.A, '--backend', 'esplora', '--chain', 'bitcoin', '--network', 'main', '--direction', 'backward', '--depth', '2', '--max-transactions', '9'],
                    ['acquire', 'plan', REFERENCE.A, '--backend', 'core', '--chain', 'bitcoin', '--network', 'main', '--genesis-hash', 'ab' * 32],
                    ['acquire', 'plan', 'unknown', '--backend', 'core', '--chain', 'bitcoin', '--network', 'main'],
                    ['acquire', 'plan', REFERENCE.A, '--backend', 'absent', '--chain', 'bitcoin', '--network', 'main']]:
        argv = ['chain-analysis', *command]
        probe = subprocess.run([os.environ['KASSIBER_RUST_BIN'], *argv], env={**env, 'KASSIBER_RUST_TEST_PROBE': '1'}, capture_output=True, check=True, timeout=30)
        assert probe.stdout == b'chain-analysis.acquire.plan\n'
        actual = WATCHES.run(roots[0], argv, env)
        expected = WATCHES.run(roots[1], argv, env, reference=True)
        assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr), command
        assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1]), command
        counts['chain-analysis.acquire.plan'] = counts.get('chain-analysis.acquire.plan', 0) + 1
    requests = [('daemon.unlock', {'auth_response': {'passphrase_secret': WATCHES.KEY}}), ('ui.chain_analysis.acquire.plan', PLAN),
                ('ui.chain_analysis.acquire.plan', {**PLAN, 'depth': 0}), ('ui.chain_analysis.acquire.plan', {**PLAN, 'backend': 'electrum'})]
    wire = b''.join(OPS.canonical({'request_id': str(i), 'kind': kind, 'args': args}) + b'\n' for i, (kind, args) in enumerate(requests))
    wire += OPS.canonical({'request_id': 'stop', 'kind': 'daemon.shutdown'}) + b'\n'
    actual = WATCHES.run(roots[0], ['daemon'], env, input=wire, passphrase=False)
    expected = WATCHES.run(roots[1], ['daemon'], env, reference=True, input=wire, passphrase=False)
    assert (actual.returncode, actual.stderr) == (0, b'')
    lines = [result.stdout.replace(str(root).encode(), b'ROOT').splitlines()[1:] for result, root in [(actual, roots[0]), (expected, roots[1])]]
    assert lines[0] == lines[1]
    assert [json.loads(line)['kind'] == 'error' for line in lines[0][:4]] == [False, False, True, False]
    assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1])
    evidence = Path(os.environ.get('KASSIBER_ANALYSIS_OPS_EVIDENCE', tmp_path))
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence/f'acquire-plan-{offline}.json').write_text(json.dumps({'served_by': {**counts, 'ui.chain_analysis.acquire.plan': 3}, 'python_fallbacks': 0}, sort_keys=True))
