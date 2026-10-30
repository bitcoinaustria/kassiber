#!/usr/bin/env python3
"""Back-to-back Python/Rust pairing, ordered suggestion and complete DB parity.

Sources are copied with SQLite backup before use. Never opens a live owner book.
"""
from contextlib import ExitStack
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import time
import uuid
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
from kassiber.cli import handlers
from kassiber.core import custody_review_terms as terms,custody_authored_migration as migration,custody_components as components
from kassiber.daemon_freshness import _auto_pair_before_journals
from export_pairing_services import dump, STAMP


def copy(source,dest):
    source=sqlite3.connect(f'file:{source}?mode=ro',uri=True)
    target=sqlite3.connect(dest)
    source.backup(target);target.close();source.close()


def python_run(path,operation,workspace,profile):
    c=sqlite3.connect(path);c.row_factory=sqlite3.Row;c.execute('PRAGMA foreign_keys=ON');c.execute('BEGIN IMMEDIATE')
    sequence=0
    def uuid4():
        nonlocal sequence
        if sys._getframe(4).f_globals.get('__name__') != terms.__name__:
            return uuid.UUID(int=0xf000)
        sequence+=1
        return uuid.UUID(int=sequence)
    class Frozen(datetime):
        @classmethod
        def now(cls,tz=None):return cls(2026,10,30,12,tzinfo=timezone.utc)
    with ExitStack() as stack:
        stack.enter_context(patch.object(terms.uuid,'uuid4',side_effect=uuid4))
        for module,field in [(terms,'now_iso'),(migration,'now_iso'),(components,'_now_iso')]:stack.enter_context(patch.object(module,field,return_value=STAMP))
        stack.enter_context(patch.object(__import__('kassiber.core.transfer_matching',fromlist=['datetime']),'datetime',Frozen))
        load=os.getloadavg()[0];start=time.perf_counter()
        if operation=='auto':result=_auto_pair_before_journals(c,dict(workspace_id=workspace,profile_id=profile))
        else:result=handlers.suggest_transfer_candidates(c,workspace,profile)
        seconds=time.perf_counter()-start
    c.commit();c.close()
    return json.dumps(result,ensure_ascii=True),dict(seconds=seconds,load_1m=load)


def rust_run(binary,path,operation,workspace,profile):
    r=subprocess.run([str(binary),str(path),workspace,profile,operation],capture_output=True,text=True,check=True)
    return r.stdout.strip(),json.loads(r.stderr)


def main():
    source=Path(sys.argv[1]);target=ROOT/'target/r4-pairing/bench';target.mkdir(parents=True,exist_ok=True)
    binary=ROOT/'target/release/examples/pairing-bench'
    all_results=[]
    for shape in ['generic-2000','rich-2000','generic-20000','rich-20000']:
        original=source/shape/'data/kassiber.sqlite3'
        baseline=target/(shape+'-base.db');copy(original,baseline)
        c=sqlite3.connect(baseline);profile,workspace=c.execute('SELECT id,workspace_id FROM profiles LIMIT 1').fetchone();c.close()
        py,_=python_run(baseline,'suggest',workspace,profile)
        rs,_=rust_run(binary,baseline,'suggest',workspace,profile)
        assert py==rs,(shape,'suggestions before pairing')
        before_count=len(json.loads(py)['candidates'])
        runs=[]
        # Five alternating runs also cover load crossing eight mid-book.
        for trial in range(5):
            paths={kind:target/(shape+'-'+kind+'.db') for kind in ['python','rust']}
            for p in paths.values():copy(baseline,p)
            measurements={};outputs={}
            order=['python','rust'] if trial%2==0 else ['rust','python']
            for kind in order:
                result,timing=python_run(paths[kind],'auto',workspace,profile) if kind=='python' else rust_run(binary,paths[kind],'auto',workspace,profile)
                outputs[kind]=result;measurements[kind]=timing
            assert outputs['python']==outputs['rust'],(shape,trial,'summary',outputs)
            connections={k:sqlite3.connect(v) for k,v in paths.items()}
            for c in connections.values():c.row_factory=sqlite3.Row
            dumps={k:dump(c) for k,c in connections.items()}
            for c in connections.values():c.close()
            assert json.dumps(dumps['python'],sort_keys=True)==json.dumps(dumps['rust'],sort_keys=True),(shape,trial,'database bytes')
            # Suggestions from the post-write state have to preserve wire order.
            py,_=python_run(paths['python'],'suggest',workspace,profile)
            rs,_=rust_run(binary,paths['rust'],'suggest',workspace,profile)
            assert py==rs,(shape,trial,'suggestions')
            runs.append(measurements)
            for p in paths.values():p.unlink()
        best={k:min(r[k]['seconds'] for r in runs) for k in ['python','rust']}
        result=dict(book=shape,runs=runs,best=best,speedup=best['python']/best['rust'],database_parity=True,suggestion_byte_parity=True,suggestions_before=before_count,suggestions_after=len(json.loads(py)['candidates']))
        all_results.append(result);print(json.dumps(result),flush=True)
        baseline.unlink()
    (target/'results.json').write_text(json.dumps(all_results,indent=2)+'\n')


if __name__=='__main__':main()
