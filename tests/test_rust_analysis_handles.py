import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env
from tests.test_rust_analysis_index import canonical, tables

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('analysis_handles_vectors', ROOT / 'crates/wallets/scripts/generate_analysis_handles.py')
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def test_handles_match_unchanged_python_bytes_and_all_112_tables():
    cases = REFERENCE.vectors()
    assert len(cases) >= 48
    assert canonical(cases) == canonical(json.loads(REFERENCE.DEST.read_text()))
    for case in cases:
        conn = REFERENCE.database(case['sql'])
        try:
            before = tables(conn)
            assert canonical(REFERENCE.evaluate(case, conn)) == canonical(case['expected']), case['name']
            assert tables(conn) == before
        finally:
            conn.close()


@pytest.mark.parametrize('offline', [False, True])
def test_handles_native_bytes_all_tables_without_network_consent(tmp_path, offline):
    executable = os.environ.get('KASSIBER_ANALYSIS_INDEX_BIN')
    if not executable:
        pytest.skip('requires analysis_index_fixture test-harness build')
    cases = json.loads(REFERENCE.DEST.read_text())
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    if offline:
        env['KASSIBER_NO_EGRESS'] = '1'
    else:
        env.pop('KASSIBER_NO_EGRESS', None)
    result = subprocess.run([str(Path(executable).resolve())], input=json.dumps(cases).encode(), env=env, capture_output=True, check=True, timeout=120)
    assert result.stderr == b''
    assert result.stdout == canonical([case['expected'] for case in cases]) + b'\n'


def test_handles_mutation_catalog_stays_applicable(tmp_path):
    catalog = ROOT / 'crates/wallets/scripts/analysis_handles_mutations.json'
    entries = json.loads(catalog.read_text())
    result = subprocess.run([sys.executable, str(ROOT / 'crates/wallets/scripts/record_mutation_check.py'), '--mutations', str(catalog), '--check'], env=child_env(tmp_path), check=True, capture_output=True, timeout=30)
    assert result.stdout == f'{len(entries)} entries apply\n'.encode()
    source = (ROOT / 'crates/wallets/src/analysis_index/handles/tests.rs').read_text()
    assert entries
    assert all(f'fn {name}()' in source for entry in entries for name in entry['expect_failing'])


@pytest.mark.parametrize('offline', [False, True])
def test_handles_expire_across_real_native_processes(tmp_path, offline):
    executable = os.environ.get('KASSIBER_ANALYSIS_INDEX_BIN')
    if not executable:
        pytest.skip('requires analysis_index_fixture test-harness build')
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    if offline:
        env['KASSIBER_NO_EGRESS'] = '1'
    else:
        env.pop('KASSIBER_NO_EGRESS', None)
    base = json.loads(REFERENCE.DEST.read_text())[0]
    def invoke(actions):
        case = {'operation':'handles', 'production':True, 'sql':base['sql'], 'actions':actions}
        result = subprocess.run([str(Path(executable).resolve())], input=json.dumps([case]).encode(), env=env, capture_output=True, check=True, timeout=30)
        assert result.stderr == b''
        return result.stdout
    token = json.loads(invoke([{'operation':'issue','value':REFERENCE.TX}]))[0][0]
    second = json.loads(invoke([{'operation':'issue','value':REFERENCE.TX}]))[0][0]
    assert token.startswith('ca-ref:') and len(token) == 71
    assert token != second
    actions = [{'operation':'decode','args':{'subject':token}}]
    expected_case = {'actions':actions}
    conn = REFERENCE.database(base['sql'])
    try:
        expected = REFERENCE.evaluate(expected_case, conn)
    finally:
        conn.close()
    assert invoke(actions) == canonical([expected]) + b'\n'
