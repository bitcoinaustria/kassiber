"""Local attribution imports: unchanged-Python bytes, progress, rollback and CLI parity."""
import base64
import importlib.util
import json
import os
from pathlib import Path
import subprocess

import pytest

from tests.rust_sync_env import child_env
from tests import test_rust_analysis_watches as WATCHES

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('analysis_dataset_import_vectors', ROOT/'crates/wallets/scripts/generate_analysis_dataset_import.py')
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)
OPS = WATCHES.OPS


def corpus():
    return json.loads(REFERENCE.DEST.read_text())


def test_dataset_import_corpus_frozen_python_bytes_and_all_112_tables():
    assert OPS.canonical(REFERENCE.vectors()) == OPS.canonical(corpus())


@pytest.mark.parametrize('offline', [False, True])
def test_dataset_import_native_bytes_without_network_consent(tmp_path, offline):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    if offline: env['KASSIBER_NO_EGRESS'] = '1'
    cases = corpus()
    assert WATCHES.native(cases, env) == [c['expected'] for c in cases]


@pytest.mark.parametrize('encrypted', [False, True], ids=['sqlite', 'sqlcipher'])
@pytest.mark.parametrize('journal', ['DELETE', 'WAL'])
def test_dataset_import_disk_batches_failures_and_cleanup(tmp_path, encrypted, journal):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    names = {'csv-generic', 'batched-job', 'cancelled-job', 'discard-job-cancel', 'replacement', 'hash-changed', 'caller-busy', 'malformed-0'}
    selected = [c for c in corpus() if c['name'] in names]
    assert len(selected) == len(names)
    for i, case in enumerate(selected):
        path = tmp_path/f'book-{i}.db'
        conn = OPS.connect(path, encrypted)
        conn.execute(f'PRAGMA journal_mode={journal}')
        conn.executescript(OPS.SEED.read_text())
        conn.executescript(OPS.BOOK.read_text())
        conn.close()
        assert WATCHES.native({'database': str(path), 'encrypted': encrypted, 'case': case}, env) == case['expected'], case['name']
        conn = OPS.connect(path, encrypted)
        assert conn.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        assert OPS.REFERENCE.capture(conn) == case['expected'][-1]['state']
        conn.close()


@pytest.mark.parametrize('offline', [False, True])
def test_dataset_import_product_cli_bytes_all_tables_and_zero_fallback(tmp_path, offline):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    env = OPS.product_env(tmp_path)
    if offline: env['KASSIBER_NO_EGRESS'] = '1'
    else: env.pop('KASSIBER_NO_EGRESS', None)
    roots = WATCHES.books(tmp_path)
    files = tmp_path/'files'; files.mkdir()
    csv = files/'pack.csv'
    csv.write_text(f'subject,label,category\n{REFERENCE.A},Exchange,exchange\n{REFERENCE.B}:0,Mixer,mixing\n')
    jsonl = files/'pack.jsonl'
    jsonl.write_text(json.dumps({'subject': REFERENCE.A, 'label': 'Pool', 'category': 'mining'}) + '\n')
    bad = files/'bad.csv'
    bad.write_text(f'subject,label\n{REFERENCE.A},"open\n')
    import io
    sha = REFERENCE.datasets.preview_dataset(REFERENCE.manifest(), io.BytesIO(csv.read_bytes()))['sha256']
    jsha = REFERENCE.datasets.preview_dataset(REFERENCE.manifest(version='2'), io.BytesIO(jsonl.read_bytes()), format='jsonl')['sha256']
    counts = {}

    def same(argv, *, offset=0, expect_ok=None):
        env['KASSIBER_RUST_TEST_ANALYSIS_ID_OFFSET'] = str(offset)
        argv = ['chain-analysis', 'datasets', 'import', *argv]
        probe = subprocess.run([os.environ['KASSIBER_RUST_BIN'], *argv], env={**env, 'KASSIBER_RUST_TEST_PROBE': '1'}, capture_output=True, check=True, timeout=30)
        assert probe.stdout == b'chain-analysis.datasets.import\n'
        actual = WATCHES.run(roots[0], argv, env)
        expected = WATCHES.run(roots[1], argv, env, reference=True)
        assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr), argv
        if expect_ok is not None: assert (actual.returncode == 0) == expect_ok, actual.stdout
        assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1]), argv
        counts['chain-analysis.datasets.import'] = counts.get('chain-analysis.datasets.import', 0) + 1
        return json.loads(actual.stdout)

    first = same(['--file', str(csv), '--manifest', json.dumps(REFERENCE.manifest()), '--expected-sha256', sha], expect_ok=True)['data']
    same(['--file', str(csv), '--manifest', json.dumps(REFERENCE.manifest()), '--expected-sha256', sha], offset=1, expect_ok=False)
    same(['--file', str(jsonl), '--manifest', json.dumps(REFERENCE.manifest(version='2', expected_active_id=first['id'])), '--expected-sha256', jsha,
          '--source-format', 'jsonl'], offset=2, expect_ok=True)
    same(['--file', str(bad), '--manifest', json.dumps(REFERENCE.manifest(version='3')), '--expected-sha256', 'f' * 64], offset=3, expect_ok=False)
    same(['--file', str(files/'missing.csv'), '--manifest', json.dumps(REFERENCE.manifest(version='4')), '--expected-sha256', sha], expect_ok=False)
    same(['--file', str(files), '--manifest', json.dumps(REFERENCE.manifest(version='4')), '--expected-sha256', sha], offset=4, expect_ok=False)
    same(['--file', str(csv), '--manifest', '[]', '--expected-sha256', sha], expect_ok=False)
    same(['--file', str(csv), '--manifest', json.dumps(REFERENCE.manifest(version='5')), '--expected-sha256', sha, '--adapter', 'maru92'], offset=5, expect_ok=False)
    evidence = Path(os.environ.get('KASSIBER_ANALYSIS_OPS_EVIDENCE', tmp_path))
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence/f'datasets-import-cli-{offline}.json').write_text(json.dumps({'served_by': counts, 'python_fallbacks': 0}, sort_keys=True))
