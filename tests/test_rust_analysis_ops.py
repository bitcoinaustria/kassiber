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
from tests.rust_port.repro_analysis_label_address import reproduce

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('analysis_ops_vectors', ROOT/'crates/wallets/scripts/generate_analysis_ops.py')
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)
SEED = ROOT/'crates/reports/tests/data/privacy_hygiene_seed.sql'
BOOK = ROOT/'crates/reports/tests/data/book.sql'
REFERENCE_MAIN = ROOT/'tests/rust_port/analysis_ops_reference.py'


def canonical(value):
    return REFERENCE.canonical(value).encode()


def corpus():
    return json.loads(REFERENCE.DEST.read_text())


def native(request, env):
    executable = os.environ.get('KASSIBER_ANALYSIS_OPS_BIN')
    if not executable:
        pytest.skip('requires analysis_ops_fixture test-harness build')
    result = subprocess.run([executable],input=canonical(request),capture_output=True,env=env,check=True,timeout=120)
    assert result.stderr == b''
    value = json.loads(result.stdout)
    assert result.stdout == canonical(value)+b'\n'
    return value


def connect(path, encrypted=False):
    codec = sqlcipher if encrypted else sqlite3
    conn = codec.connect(str(path))
    if encrypted:
        conn.execute("PRAGMA key='analysis-ops-fixture'")
    conn.row_factory = codec.Row
    conn.execute('PRAGMA foreign_keys=ON')
    return conn


def test_analysis_operations_frozen_python_bytes_and_all_112_tables():
    assert canonical(REFERENCE.vectors()) == canonical(corpus())


@pytest.mark.parametrize('offline',[False,True])
def test_analysis_operations_native_bytes_without_network_consent(tmp_path, offline):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS']='strict'
    if offline: env['KASSIBER_NO_EGRESS']='1'
    else: env.pop('KASSIBER_NO_EGRESS',None)
    cases = corpus()
    assert native(cases,env) == [c['expected'] for c in cases]


@pytest.mark.parametrize('encrypted',[False,True],ids=['sqlite','sqlcipher'])
@pytest.mark.parametrize('journal',['DELETE','WAL'])
def test_analysis_operations_disk_codecs_preserve_atomic_writes_and_rollback(tmp_path,encrypted,journal):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS']='strict'
    names = {'revisioned-lifecycle','atomic-import-3','atomic-import-7','save-delete-owner','save-delete-public','corrupt-case-delete',
             'revoke-active-1','discard-failed','caller-failed-import','caller-import-rollback','dataset-caller-rollback','dataset-busy','case-caller-rollback-owner'}
    selected = [c for c in corpus() if c['name'] in names]
    assert len(selected)==len(names)
    for i,case in enumerate(selected):
        path=tmp_path/f'book-{i}.db'
        conn=connect(path,encrypted)
        conn.execute(f'PRAGMA journal_mode={journal}')
        assert conn.execute('PRAGMA journal_mode').fetchone()[0]==journal.lower()
        conn.executescript(SEED.read_text())
        conn.executescript(BOOK.read_text())
        conn.executescript(case.get('sql',''))
        conn.close()
        assert native({'database':str(path),'encrypted':encrypted,'case':case},env)==case['expected'],case['name']
        conn=connect(path,encrypted)
        assert conn.execute('PRAGMA integrity_check').fetchone()[0]=='ok'
        assert REFERENCE.capture(conn)==case['expected'][-1]['state']
        conn.close()


@pytest.mark.parametrize('encrypted',[False,True],ids=['sqlite','sqlcipher'])
@pytest.mark.parametrize('journal',['DELETE','WAL'])
def test_analysis_history_sql_failure_rolls_back_both_disk_codecs(tmp_path,encrypted,journal):
    env=child_env(tmp_path); env['KASSIBER_TEST_NO_EGRESS']='strict'
    path=tmp_path/'fault.db'
    conn=connect(path,encrypted)
    conn.execute(f'PRAGMA journal_mode={journal}')
    conn.executescript(SEED.read_text()); conn.executescript(BOOK.read_text())
    conn.executescript("CREATE TRIGGER reject_analysis_history BEFORE INSERT ON chain_analysis_label_history BEGIN SELECT RAISE(ABORT,'injected history failure'); END;")
    before=REFERENCE.capture(conn)
    for operation,args in [('labels.upsert',REFERENCE.label()),('labels.import',{'items':[REFERENCE.label()]})]:
        with pytest.raises((sqlite3.DatabaseError,sqlcipher.DatabaseError)):
            if operation.endswith('upsert'): REFERENCE.cases.upsert_label(conn,'p',args)
            else: REFERENCE.cases.import_labels(conn,'p',args)
        assert REFERENCE.capture(conn)==before
    conn.close()
    case={'actions':[{'operation':'labels.upsert','args':REFERENCE.label()}, {'operation':'labels.import','args':{'items':[REFERENCE.label()]}}]}
    actual=native({'database':str(path),'encrypted':encrypted,'case':case},env)
    assert all(state['result']['error']['code']=='internal_error'
               and state['result']['error']['message']=='database operation failed'
               and state['result']['error']['details']=={'sqlite_error_code':1811} for state in actual)
    assert all(state['state']==before and not state['in_transaction'] for state in actual)


def test_analysis_label_python_reference_address_bug_is_retained():
    assert reproduce()['p2pkh_error']=='Invalid address'


def test_analysis_operations_mutation_catalog_stays_applicable(tmp_path):
    catalog=ROOT/'crates/wallets/scripts/analysis_ops_mutations.json'
    entries=json.loads(catalog.read_text())
    result=subprocess.run([sys.executable,str(ROOT/'crates/wallets/scripts/record_mutation_check.py'),'--mutations',str(catalog),'--check'],env=child_env(tmp_path),capture_output=True,check=True,timeout=30)
    assert result.stdout==f'{len(entries)} entries apply\n'.encode()
    source=(ROOT/'crates/wallets/src/analysis_ops/tests.rs').read_text()
    assert len(entries)>=7
    assert all(f'fn {name}()' in source for entry in entries for name in entry['expect_failing'])


def product_env(tmp_path):
    env=child_env(tmp_path)
    owners=tmp_path/'owners'; owners.mkdir(mode=0o700)
    env.update({'KASSIBER_TEST_NO_EGRESS':'strict','KASSIBER_RUST_TEST_OWNER_ROOT':str(owners),
                'KASSIBER_RUST_TEST_ANALYSIS_IDS':'sequential','KASSIBER_RUST_TEST_ANALYSIS_TIME':REFERENCE.NOW,
                'KASSIBER_RUST_TEST_ANALYSIS_ID_OFFSET':'0'})
    return env


def product_books(tmp_path):
    roots=[tmp_path/'native',tmp_path/'reference']
    from kassiber.db import open_db
    conn=open_db(str(roots[0]))
    conn.executescript(BOOK.read_text())
    conn.executescript(REFERENCE.dataset_sql('active',3))
    conn.executescript(REFERENCE.dataset_sql('failed',2001).replace("'pack'","'cleanup'"))
    from types import SimpleNamespace
    from unittest.mock import patch
    with patch.object(REFERENCE.projection_store,'now_iso',return_value=REFERENCE.NOW), patch.object(REFERENCE.projection_store,'uuid',SimpleNamespace(uuid4=lambda:SimpleNamespace(hex='0123456789abcdef0123456789abcdef'))):
        REFERENCE.run_analysis(conn,'p',{})
    conn.commit()
    conn.close()
    shutil.copytree(roots[0],roots[1])
    return roots


def product_snapshot(root):
    from kassiber.db import resolve_database_path
    conn=connect(resolve_database_path(str(root)))
    try: return REFERENCE.capture(conn)
    finally: conn.close()


def product_run(root, argv, env, *, reference=False, input=None):
    executable=os.environ.get('KASSIBER_RUST_BIN')
    if not executable: pytest.skip('requires kassiber test-harness build')
    prefix=[sys.executable,str(REFERENCE_MAIN)] if reference else [executable]
    return subprocess.run([*prefix,'--machine','--data-root',str(root),*argv],input=input,capture_output=True,env=env,cwd=ROOT,timeout=120)


def current_snapshot():
    from kassiber.core.chain_analysis.index import digest
    return digest(['0123456789abcdef0123456789abcdef','p',2,2])


@pytest.mark.parametrize('offline',[False,True])
def test_analysis_operations_product_cli_bytes_all_tables_and_zero_fallback(tmp_path,offline):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    env=product_env(tmp_path)
    if offline: env['KASSIBER_NO_EGRESS']='1'
    else: env.pop('KASSIBER_NO_EGRESS',None)
    roots=product_books(tmp_path)
    label=REFERENCE.label()
    commands=[
        ['labels','upsert','--document',json.dumps(label)],
        ['labels','upsert','--document',json.dumps(REFERENCE.label(id=REFERENCE.FIRST,expected_revision=1,label='Revised'))],
        ['labels','delete',REFERENCE.FIRST,'--expected-revision','1'],
        ['labels','delete',REFERENCE.FIRST,'--expected-revision','2'],
        ['labels','import','--document',json.dumps({'items':[REFERENCE.label(),REFERENCE.label(chain='unsupported')]})],
        ['labels','import','--document',json.dumps({'items':[REFERENCE.label(),REFERENCE.label(subject='b'*64)]})],
        ['labels','upsert','--document','[]'],
        ['labels','upsert','--document','@missing.json'],
        ['cases','save','--title','Stale','--query','{}','--expected-snapshot-id','stale'],
        ['cases','delete','absent'],
        ['datasets','revoke','absent','--expected-revision','1'],
        ['datasets','discard','absent'],
        ['cases','save','--title','Saved','--query','{}','--expected-snapshot-id',current_snapshot()],
        ['cases','delete','00000000-0000-4000-8000-000000000005'],
        ['datasets','revoke','pack','--expected-revision','1'],
        ['datasets','discard','cleanup'],
    ]
    counts={}
    for command in commands:
        env['KASSIBER_RUST_TEST_ANALYSIS_ID_OFFSET'] = str(2 if command[:2] == ['labels','import'] else 4 if command[:2] == ['cases','save'] else 0)
        argv=['chain-analysis',*command]
        probe=subprocess.run([os.environ['KASSIBER_RUST_BIN'],*argv],env={**env,'KASSIBER_RUST_TEST_PROBE':'1'},capture_output=True,check=True,timeout=30)
        path='.'.join(['chain-analysis',*command[:2]])
        assert probe.stdout==path.encode()+b'\n'
        actual=product_run(roots[0],argv,env)
        expected=product_run(roots[1],argv,env,reference=True)
        assert (actual.returncode,actual.stdout,actual.stderr)==(expected.returncode,expected.stdout,expected.stderr),command
        if command[0:2] in [['cases','save'],['cases','delete'],['datasets','revoke'],['datasets','discard']] and command in commands[-4:]:
            assert actual.returncode==0,actual.stdout
        assert product_snapshot(roots[0])==product_snapshot(roots[1]),command
        counts[path]=counts.get(path,0)+1
    evidence=Path(os.environ.get('KASSIBER_ANALYSIS_OPS_EVIDENCE',tmp_path))
    evidence.mkdir(parents=True,exist_ok=True)
    (evidence/f'cli-{offline}.json').write_text(json.dumps({'served_by':counts,'python_fallbacks':0},sort_keys=True))


@pytest.mark.parametrize('offline',[False,True])
def test_analysis_operations_product_daemon_bytes_events_and_all_tables(tmp_path,offline):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    env=product_env(tmp_path)
    if offline: env['KASSIBER_NO_EGRESS']='1'
    else: env.pop('KASSIBER_NO_EGRESS',None)
    roots=product_books(tmp_path)
    requests=[
        ('labels.upsert',REFERENCE.label()),
        ('labels.upsert',REFERENCE.label(id=REFERENCE.FIRST,expected_revision=1,label='Revised')),
        ('labels.delete',{'id':REFERENCE.FIRST,'expected_revision':True}),
        ('labels.delete',{'id':REFERENCE.FIRST,'expected_revision':2}),
        ('labels.import',{'items':[REFERENCE.label(),REFERENCE.label(subject='b'*64)]}),
        ('labels.import',{'items':[REFERENCE.label(),REFERENCE.label(chain='unsupported')]}),
        ('labels.upsert',{'expected_scope':{'workspace_id':'ws','profile_id':'other'}}),
        ('labels.upsert',[]),
        ('cases.save',{'title':'Stale','query':{},'expected_snapshot_id':'stale'}),
        ('cases.delete',{'id':'absent'}),
        ('datasets.revoke',{'id':'absent','expected_revision':1}),
        ('datasets.discard',{'id':'absent'}),
        ('cases.save',{'title':'Saved','query':{},'expected_snapshot_id':current_snapshot()}),
        ('cases.delete',{'id':'00000000-0000-4000-8000-000000000005'}),
        ('datasets.revoke',{'id':'pack','expected_revision':1}),
        ('datasets.discard',{'id':'cleanup'}),
    ]
    wire=b''.join(canonical({'request_id':str(i),'kind':'ui.chain_analysis.'+op,'args':args})+b'\n' for i,(op,args) in enumerate(requests))
    wire+=canonical({'request_id':'stop','kind':'daemon.shutdown'})+b'\n'
    actual=product_run(roots[0],['daemon'],env,input=wire)
    expected=product_run(roots[1],['daemon'],env,reference=True,input=wire)
    assert (actual.returncode,actual.stderr)==(0,b'')
    assert actual.stdout.splitlines()[1:]==expected.stdout.splitlines()[1:]
    for response in actual.stdout.splitlines()[-5:-1]: assert json.loads(response)['kind']!='error',response
    assert product_snapshot(roots[0])==product_snapshot(roots[1])
    ready=json.loads(actual.stdout.splitlines()[0])
    assert ready['kind']=='daemon.ready'
    expected_ready=json.loads(expected.stdout.splitlines()[0])
    kinds=ready['data']['supported_kinds']
    scoped={'ui.chain_analysis.'+op for op,_ in requests}
    assert kinds[:len(scoped)]==sorted(scoped)
    assert kinds[len(scoped)]=='status'
    assert len(kinds)==len(set(kinds))
    assert set(kinds)<=set(expected_ready['data']['supported_kinds'])|{'test.stream','test.emit_event','test.late','test.secret_store'}
    for record in [ready,expected_ready]:
        record['data']['supported_kinds']=[kind for kind in record['data']['supported_kinds'] if kind in scoped|{'status'}]
    assert json.dumps(ready,separators=(',',':'))==json.dumps(expected_ready,separators=(',',':'))
    evidence=Path(os.environ.get('KASSIBER_ANALYSIS_OPS_EVIDENCE',tmp_path))
    evidence.mkdir(parents=True,exist_ok=True)
    counts={}
    for op,_ in requests: counts['ui.chain_analysis.'+op]=counts.get('ui.chain_analysis.'+op,0)+1
    (evidence/f'daemon-{offline}.json').write_text(json.dumps({'served_by':counts,'python_fallbacks':0},sort_keys=True))
