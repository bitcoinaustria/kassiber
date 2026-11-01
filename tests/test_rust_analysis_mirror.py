import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env
from tests.test_rust_analysis_index import canonical, tables

ROOT=Path(__file__).resolve().parents[1]
SPEC=importlib.util.spec_from_file_location('analysis_mirror_vectors',ROOT/'crates/wallets/scripts/generate_analysis_mirror.py')
REFERENCE=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def test_mirror_matches_unchanged_python_bytes_and_all_112_tables():
    corpus=REFERENCE.vectors()
    assert len(corpus['cases'])>=140
    assert canonical(corpus)==canonical(json.loads(REFERENCE.DEST.read_text()))
    for case in corpus['cases']:
        conn=REFERENCE.database(corpus['books'][case['book']])
        try:
            before=tables(conn)
            assert canonical(REFERENCE.evaluate(case,conn))==canonical(case['expected']),case['name']
            assert tables(conn)==before
        finally:conn.close()


@pytest.mark.parametrize('offline',[False,True])
def test_mirror_native_bytes_all_tables_without_network_consent(tmp_path,offline):
    executable=os.environ.get('KASSIBER_ANALYSIS_INDEX_BIN')
    if not executable:pytest.skip('requires analysis_index_fixture test-harness build')
    corpus=json.loads(REFERENCE.DEST.read_text())
    env=child_env(tmp_path);env['KASSIBER_TEST_NO_EGRESS']='strict'
    if offline:env['KASSIBER_NO_EGRESS']='1'
    else:env.pop('KASSIBER_NO_EGRESS',None)
    cases=corpus['cases']
    for offset in range(0,len(cases),2):
        part=cases[offset:offset+2]
        requests=[{**case,'sql':corpus['books'][case['book']]} for case in part]
        result=subprocess.run([str(Path(executable).resolve())],input=json.dumps(requests).encode(),env=env,capture_output=True,check=True,timeout=120)
        assert result.stderr==b''
        assert result.stdout==canonical([case['expected'] for case in part])+b'\n'


def test_mirror_mutation_catalog_stays_applicable(tmp_path):
    catalog=ROOT/'crates/wallets/scripts/analysis_mirror_mutations.json'
    entries=json.loads(catalog.read_text())
    result=subprocess.run([sys.executable,str(ROOT/'crates/wallets/scripts/record_mutation_check.py'),'--mutations',str(catalog),'--check'],env=child_env(tmp_path),check=True,capture_output=True,timeout=30)
    assert result.stdout==f'{len(entries)} entries apply\n'.encode()
    source=(ROOT/'crates/wallets/src/analysis_index/mirror/tests.rs').read_text()
    assert entries
    assert all(f'fn {name}()' in source for entry in entries for name in entry['expect_failing'])
