"""Fingerprint-bound provenance handoffs compare bytes without changing HOME."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess

import pytest

from kassiber.db import open_db
from crates.reports.scripts.gen_source_funds_review import outcome
from tests.test_rust_source_funds_reads import ROOT, SERVICE, RUST_BIN, book, state

CASES=json.loads((ROOT/'crates/reports/tests/data/source_funds_review.json').read_text())


@pytest.mark.skipif(not RUST_BIN or os.name != 'posix',reason='requires native binary and Unix account paths')
def test_source_funds_review_expands_selected_recipe_paths_without_changing_home(tmp_path,monkeypatch):
    import pwd
    root,_,env,call=book(tmp_path,monkeypatch,'ready')
    before=state(root)
    recipe=tmp_path/'selected-recipe.json'
    recipe.write_text('{"planned_note":"Selected recipe"}')
    account=pwd.getpwuid(os.getuid())
    selected=['~/'+os.path.relpath(recipe,Path.home()),'~'+account.pw_name+'/'+os.path.relpath(recipe,account.pw_dir)]
    for path in selected:
        args=['--machine','source-funds','review-context','--target-transaction','in','--recipe-file',path]
        native=call(*args);reference=call(*args,reference=True)
        assert (native.returncode,native.stdout,native.stderr)==(reference.returncode,reference.stdout,reference.stderr)
        assert native.returncode==0
        assert state(root)==before
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served==[dict(command='source-funds.review-context',served_by='rust')]*2


@pytest.mark.skipif(not SERVICE,reason='requires native source-funds service probe')
def test_source_funds_review_service_bytes_fingerprints_and_all_tables(tmp_path):
    for index,case in enumerate(CASES):
        root=tmp_path/str(index)
        conn=open_db(root);conn.executescript(case['sql']);conn.commit()
        database=Path(conn.execute('PRAGMA database_list').fetchone()[2]);conn.close()
        before=state(root)
        result=subprocess.run([SERVICE],input=json.dumps(dict(database=str(database),command=case['command'],args_json=case['args_json']))+'\n',capture_output=True,text=True,check=True,timeout=180)
        assert not result.stderr,case['name']
        assert result.stdout in [json.dumps(json.loads(v))+'\n' for v in case['expected_variants']],(case['name'],result.stdout[:1000])
        assert state(root)==before,case['name']


def inspection(database):
    conn=sqlite3.connect(database);conn.row_factory=sqlite3.Row
    result=outcome(conn,'ui.source_funds.review_context',dict(target_transaction='in'))['ok']
    conn.close()
    return result


@pytest.mark.skipif(not RUST_BIN,reason='requires Rust test-harness binary')
@pytest.mark.parametrize('variant',['ready','stale','source-blocked','no-context'])
def test_source_funds_review_cli_bytes_files_all_tables_and_zero_fallbacks(tmp_path,monkeypatch,variant):
    root,database,env,call=book(tmp_path,monkeypatch,variant)
    before=state(root);count=0
    def compare(args,output=None):
        nonlocal count
        native=call(*args)
        payload=output.read_bytes() if output and output.exists() else None
        if output:output.unlink(missing_ok=True)
        reference=call(*args,reference=True)
        assert (native.returncode,native.stdout,native.stderr)==(reference.returncode,reference.stdout,reference.stderr),args
        if output:assert (output.read_bytes() if output.exists() else None)==payload
        assert state(root)==before
        count+=1
    for flags in [[],['--machine'],['--format','json'],['--format','plain'],['--format','csv']]:
        for target in ['in','missing','duplicate','excluded']:
            compare([*flags,'source-funds','review-context','--target-transaction',target])
    for encoding in ['utf-8','utf-8-sig','utf-16','utf-16-le','utf-16-be','utf-32','utf-32-le','utf-32-be']:
        recipe=tmp_path/('recipe-'+encoding+'.json')
        recipe.write_bytes(json.dumps(dict(data=dict(recipe=dict(target_amount='0.1',planned_note='Note Ω'))),ensure_ascii=False).encode(encoding))
        compare(['--machine','source-funds','review-context','--target-transaction','in','--recipe-file',str(recipe)])
    for name,payload in [('invalid',b'broken'),('unknown',b'{"unknown":true}'),('list',b'[]'),('oversize',b' '*1_000_001),('bad-encoding',b'\xff'),('surrogate',b'\xff\xfe{\x00"\x00p\x00l\x00a\x00n\x00n\x00e\x00d\x00_\x00n\x00o\x00t\x00e\x00"\x00:\x00"\x00\x00\xd8"\x00}\x00')]:
        recipe=tmp_path/(name+'.json');recipe.write_bytes(payload)
        compare(['--machine','source-funds','review-context','--target-transaction','in','--recipe-file',str(recipe)])
    compare(['--machine','source-funds','review-context','--target-transaction','in','--recipe-file',str(tmp_path/'missing.json')])
    fingerprint=inspection(database)['review_fingerprint'] if variant!='no-context' else '0'*64
    for action in ['connect_wallet','import_history','attach_evidence']:
        compare(['--machine','source-funds','request-input','--target-transaction','in','--action',action,'--expected-review-fingerprint',fingerprint,'--explanation','  Missing evidence Ω  '])
    compare(['--machine','source-funds','request-input','--target-transaction','in','--action','attach_evidence','--expected-review-fingerprint','0'*64])
    for command in ['review-context','request-input']:
        output=tmp_path/(command+'.json')
        args=['--machine','--output',str(output),'source-funds',command,'--target-transaction','in']
        if command=='request-input':args+=['--action','attach_evidence','--expected-review-fingerprint',fingerprint]
        compare(args,output)
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert len(served)==count==41
    assert all(item['served_by']=='rust' and item['command'] in ['source-funds.review-context','source-funds.request-input'] for item in served)


@pytest.mark.skipif(not RUST_BIN,reason='requires Rust test-harness binary')
@pytest.mark.parametrize('variant',['ready','stale','source-blocked','no-context'])
def test_source_funds_review_daemon_bytes_all_tables_and_zero_fallbacks(tmp_path,monkeypatch,variant):
    root,database,env,call=book(tmp_path,monkeypatch,variant);before=state(root)
    recipes=[None,[],False,{},dict(target_transaction='in'),dict(target_transaction='excluded'),dict(target_transaction='missing'),dict(target_transaction='duplicate'),dict(target_transaction='in',max_depth=True),dict(target_transaction='in',unknown=True),dict(target_transaction='in',target_amount='0.1',report_purpose='planned_exchange_sale',planned_destination='Bank Ω',report_options=dict(mask_recipient=True))]
    requests=[dict(kind='ui.source_funds.review_context',args=args) for args in recipes]
    context=inspection(database) if variant!='no-context' else dict(review_fingerprint='0'*64,recipe={})
    request=dict(target_transaction='in',recipe=context['recipe'],expected_review_fingerprint=context['review_fingerprint'],action='attach_evidence')
    for action in ['attach_evidence','connect_wallet','import_history']:
        requests.append(dict(kind='ui.source_funds.request_input',args=dict(request,action=action,explanation='  Evidence Ω  ')))
    for extra in [dict(expected_review_fingerprint='0'*64),dict(expected_review_fingerprint='bad'),dict(explanation=''),dict(explanation='bad\x00'),dict(unknown=True),dict(recipe=[]),dict(action='bad')]:
        requests.append(dict(kind='ui.source_funds.request_input',args=dict(request,**extra)))
    wire=''.join(json.dumps(dict(request_id=i,**r))+'\n' for i,r in enumerate(requests))+'{"request_id":"done","kind":"daemon.shutdown"}\n'
    native=call('daemon',wire=wire.encode());assert state(root)==before
    reference=call('daemon',reference=True,wire=wire.encode());assert state(root)==before
    assert native.returncode==reference.returncode==0
    assert native.stderr==reference.stderr==b''
    for i,(actual,expected) in enumerate(zip(native.stdout.splitlines()[1:],reference.stdout.splitlines()[1:])):
        assert actual==expected,(i,actual[:1000],expected[:1000])
    assert len(native.stdout.splitlines())==len(reference.stdout.splitlines())==len(requests)+2
    served=[json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served==[dict(command='daemon',served_by='rust')]
