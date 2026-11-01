"""Native local investigations: exact envelopes, all-table state and routing."""
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

import pytest

from tests.rust_sync_env import child_env

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('chain_analysis_reads_vectors', ROOT / 'crates/wallets/scripts/generate_chain_analysis_reads.py')
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def test_local_read_envelopes_and_all_112_tables_match_unchanged_python():
    cases = REFERENCE.vectors()
    assert cases == json.loads(REFERENCE.DEST.read_text())
    assert len(cases) >= 74
    assert all(s['state']['table_count'] == 112 for c in cases for s in c['expected'])


def test_local_reads_record_hashseed_variants_without_changing_python():
    from tests.rust_port.repro_chain_analysis_reads_hashseed import variants
    recorded=json.loads((ROOT/'crates/wallets/tests/data/chain_analysis_reads_hashseeds.json').read_text())
    assert variants()==recorded
    assert len({json.dumps(v['full_build_finding_node_ids']) for v in recorded.values()}) > 1
    assert len({v['wire_sha256'] for v in recorded.values()}) == 1


@pytest.mark.parametrize('offline', [False, True])
def test_native_local_read_envelopes_all_tables_without_network_consent(tmp_path, offline):
    executable = os.environ.get('KASSIBER_CHAIN_ANALYSIS_READS_BIN')
    if not executable: pytest.skip('requires chain_analysis_reads_fixture test-harness build')
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    if offline: env['KASSIBER_NO_EGRESS'] = '1'
    cases = json.loads(REFERENCE.DEST.read_text())
    result = subprocess.run([executable], input=json.dumps(cases).encode(), env=env, capture_output=True, check=True, timeout=120)
    assert result.stderr == b''
    assert result.stdout == REFERENCE.canonical([c['expected'] for c in cases]).encode() + b'\n'


def test_local_read_mutation_catalog_stays_applicable(tmp_path):
    path = ROOT / 'crates/wallets/scripts/chain_analysis_reads_mutations.json'
    entries = json.loads(path.read_text())
    result = subprocess.run([sys.executable, str(ROOT / 'crates/wallets/scripts/record_mutation_check.py'), '--mutations', str(path), '--check'], env=child_env(tmp_path), check=True, capture_output=True, timeout=30)
    assert result.stdout == f'{len(entries)} entries apply\n'.encode()
    source = (ROOT / 'crates/wallets/src/analysis_index/reads/tests.rs').read_text()
    assert entries and all(f'fn {test}()' in source for entry in entries for test in entry['expect_failing'])


@pytest.mark.parametrize('offline', [False, True])
def test_local_reads_cli_daemon_bytes_all_tables_and_zero_fallback(tmp_path, offline, monkeypatch):
    binary = os.environ.get('KASSIBER_RUST_BIN')
    if not binary: pytest.skip('requires native product test-harness build')
    from kassiber.db import open_db
    from tests import cli_runner
    from tests.test_rust_analysis_index import tables
    root = tmp_path / 'data'
    conn = open_db(root)
    cases = json.loads(REFERENCE.DEST.read_text())
    seed = next(c for c in cases if c['name'] == 'cases-read-0')['sql']
    conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + seed)
    conn.close()
    conn = open_db(root)
    REFERENCE.evaluate({'actions':[{'operation':'query'}]}, conn)
    conn.commit()
    db = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    conn.close()
    snapshot = tmp_path / 'snapshot.sqlite3'
    shutil.copyfile(db, snapshot)
    owners = tmp_path / 'owners'; owners.mkdir(mode=0o700)
    env = child_env(tmp_path)
    env.update({'KASSIBER_RUST_TEST_OWNER_ROOT': str(owners),
                'KASSIBER_TEST_NO_EGRESS': 'strict',
                'KASSIBER_HYBRID_RUST_BIN': str(Path(binary).resolve()),
                'KASSIBER_HYBRID_LOG': str(tmp_path / 'served.jsonl'),
                'KASSIBER_HYBRID_RUST_DAEMON': '1'})
    if offline: env['KASSIBER_NO_EGRESS'] = '1'
    monkeypatch.setenv('KASSIBER_TEST_CLI', json.dumps([sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py')]))
    original = cli_runner.prefix
    code = 'import os,sys; from pathlib import Path; from kassiber.operator import project; project._owner_lock_root=lambda:Path(os.environ["KASSIBER_RUST_TEST_OWNER_ROOT"]); from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))'
    monkeypatch.setattr(cli_runner, 'prefix', lambda *, reference=False, python_options=(): (sys.executable, '-c', code) if reference else original(python_options=python_options))
    def run(args, reference, wire=None):
        shutil.copyfile(snapshot, db)
        result = cli_runner.run(['--machine','--data-root',root,*args],reference=reference,env=env,input=wire,capture_output=True,timeout=120)
        c=sqlite3.connect(db); c.row_factory=sqlite3.Row
        try: state=tables(c)
        finally:c.close()
        return result, state
    tx=REFERENCE.TX
    commands = [
        ['overview'], ['trace',tx,'--observer','public','--include-hypotheses'], ['path',tx,tx+':0'],
        ['entropy',tx], ['datasets','list','--limit','1'], ['datasets','get','pack'],
        ['datasets','query','--label','Declared entity','--dataset-id','pack'],
        ['cases','list','--limit','1'], ['cases','get','first'], ['cases','compare','first','--other-id','second'], ['labels','list'],
        ['trace','absent'], ['cases','get','missing'], ['entropy',tx,'--scenario','[]'],
    ]
    for command in commands:
        ref, before=run(['chain-analysis',*command],True)
        native, after=run(['chain-analysis',*command],False)
        (tmp_path/'reference.stdout').write_bytes(ref.stdout)
        (tmp_path/'native.stdout').write_bytes(native.stdout)
        assert (native.returncode,native.stdout,native.stderr)==(ref.returncode,ref.stdout,ref.stderr),command
        assert after==before,command
    operations = [
        ('query',{}),('query',{'mode':'trace','subject':tx,'observer':'public'}),
        ('entropy',{'subject':tx}),('datasets.list',{}),('datasets.get',{'id':'pack'}),
        ('datasets.query',{'label':'Declared entity','dataset_id':'pack'}),('cases.list',{}),
        ('cases.get',{'id':'first'}),('cases.compare',{'id':'first','other_id':'second'}),('labels.list',{}),
        ('query',{'mode':'trace','subject':'absent'}),('cases.get',{'id':'missing'}),
        ('query',{'expected_scope':{'workspace_id':'ws','profile_id':'other'}}),('labels.list',[]),
    ]
    requests=[{'request_id':i,'kind':'ui.chain_analysis.'+op,'args':args} for i,(op,args) in enumerate(operations)]
    requests.append({'request_id':'done','kind':'daemon.shutdown'})
    wire=''.join(json.dumps(r)+'\n' for r in requests).encode()
    ref,before=run(['daemon'],True,wire);native,after=run(['daemon'],False,wire)
    assert (native.returncode,native.stderr)==(ref.returncode,ref.stderr)==(0,b'')
    assert native.stdout.splitlines()[1:]==ref.stdout.splitlines()[1:]
    assert after==before
    ready=json.loads(native.stdout.splitlines()[0])['data']['supported_kinds']
    assert all('ui.chain_analysis.'+op in ready for op,_ in operations)
    calls=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    counts=Counter((c['command'],c['served_by']) for c in calls)
    assert all(c['served_by']=='rust' for c in calls)
    assert sum(counts.values()) == len(commands)+1
    (tmp_path/'served-summary.json').write_text(json.dumps({command: {'rust':n,'python':0} for (command,_),n in sorted(counts.items())},indent=2)+'\n')
