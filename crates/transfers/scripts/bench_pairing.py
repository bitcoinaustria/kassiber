#!/usr/bin/env python3
"""Back-to-back Python/Rust pairing, ordered suggestion and complete DB parity.

Sources are copied with SQLite backup before use. Never opens a live owner book.
"""
from contextlib import ExitStack
from datetime import datetime, timezone
import argparse
import json
import os
from pathlib import Path
import resource
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

TEST_PASSPHRASE = 'synthetic-pairing-benchmark-only'


def connect(path, passphrase=None):
    if passphrase is None:
        driver = sqlite3
    else:
        from sqlcipher3 import dbapi2 as driver
    c = driver.connect(str(path))
    if passphrase is not None:
        c.execute("PRAGMA key='" + passphrase.replace("'", "''") + "'")
        c.execute('PRAGMA cipher_compatibility=4')
        c.execute('PRAGMA kdf_iter=256000')
        c.execute('PRAGMA cipher_page_size=4096')
    c.row_factory = driver.Row
    return c


def encrypt(path):
    from sqlcipher3 import dbapi2 as driver
    encrypted = path.with_suffix('.encrypted')
    encrypted.unlink(missing_ok=True)
    c = driver.connect(str(path))
    version = c.execute('PRAGMA user_version').fetchone()[0]
    c.execute('ATTACH DATABASE ? AS encrypted KEY ?', (str(encrypted), TEST_PASSPHRASE))
    c.execute("SELECT sqlcipher_export('encrypted')")
    c.execute(f'PRAGMA encrypted.user_version={int(version)}')
    c.execute('DETACH DATABASE encrypted')
    c.close()
    path.unlink()
    encrypted.replace(path)


def copy(source,dest,passphrase=None):
    if passphrase is not None:
        shutil.copyfile(source,dest)
        return
    source=sqlite3.connect(f'file:{source}?mode=ro',uri=True)
    target=sqlite3.connect(dest)
    source.backup(target);target.close();source.close()


def python_run(path,operation,workspace,profile,passphrase=None):
    c=connect(path,passphrase);c.execute('PRAGMA foreign_keys=ON');c.execute('BEGIN IMMEDIATE')
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
        load=os.getloadavg()[0];start=time.perf_counter();cpu_start=time.process_time()
        if operation=='auto':result=_auto_pair_before_journals(c,dict(workspace_id=workspace,profile_id=profile))
        else:result=handlers.suggest_transfer_candidates(c,workspace,profile)
        cpu_seconds=time.process_time()-cpu_start;seconds=time.perf_counter()-start
    c.commit();c.close()
    return json.dumps(result,ensure_ascii=True),dict(seconds=seconds,cpu_seconds=cpu_seconds,load_1m=load)


def rust_run(binary,path,operation,workspace,profile,passphrase=None):
    command=[str(binary),str(path),workspace,profile,operation]
    if passphrase is not None:command.append('--passphrase-stdin')
    before=resource.getrusage(resource.RUSAGE_CHILDREN)
    r=subprocess.run(command,input=None if passphrase is None else passphrase+'\n',capture_output=True,text=True,check=True)
    after=resource.getrusage(resource.RUSAGE_CHILDREN)
    timing=json.loads(r.stderr)
    timing['cpu_seconds']=(after.ru_utime-before.ru_utime)+(after.ru_stime-before.ru_stime)
    return r.stdout.strip(),timing


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source',type=Path,nargs='?')
    parser.add_argument('--book',type=Path,action='append',default=[])
    parser.add_argument('--shapes',nargs='+',choices=['generic-2000','rich-2000','generic-20000','rich-20000'],default=['generic-2000','rich-2000','generic-20000','rich-20000'])
    parser.add_argument('--cipher',action='store_true')
    parser.add_argument('--runs',type=int,default=5)
    parser.add_argument('--out',type=Path,default=ROOT/'target/r4-pairing/bench')
    args=parser.parse_args()
    if args.runs<1:parser.error('--runs must be positive')
    target=args.out.resolve()
    if not target.is_relative_to(ROOT/'target'):parser.error('--out must be under this worktree target/')
    target.mkdir(parents=True,exist_ok=True)
    books=[(f'recorded-{i}',path) for i,path in enumerate(args.book)]
    if args.source:
        books += [(shape,args.source/shape/'data/kassiber.sqlite3') for shape in args.shapes]
    if not books:parser.error('provide a generator-book directory or --book DB')
    passphrase=TEST_PASSPHRASE if args.cipher else None
    binary=ROOT/'target/release/examples/pairing-bench'
    scoped=[]
    for shape,original in books:
        c=sqlite3.connect(f'file:{original.resolve()}?mode=ro',uri=True)
        profiles=c.execute('SELECT id,workspace_id FROM profiles ORDER BY id').fetchall();c.close()
        for index,(profile,workspace) in enumerate(profiles):
            scoped.append((shape if len(profiles)==1 else f'{shape}-profile-{index}',original,profile,workspace))
    if not scoped:parser.error('books contain no profiles')
    all_results=[]
    for shape,original,profile,workspace in scoped:
        baseline=target/(shape+'-base.db');copy(original,baseline)
        if args.cipher:encrypt(baseline)
        py,_=python_run(baseline,'suggest',workspace,profile,passphrase)
        rs,_=rust_run(binary,baseline,'suggest',workspace,profile,passphrase)
        assert py==rs,(shape,'suggestions before pairing')
        before_count=len(json.loads(py)['candidates'])
        runs=[]
        # Five alternating runs also cover load crossing eight mid-book.
        for trial in range(args.runs):
            paths={kind:target/(shape+'-'+kind+'.db') for kind in ['python','rust']}
            for p in paths.values():copy(baseline,p,passphrase)
            measurements={};outputs={}
            order=['python','rust'] if trial%2==0 else ['rust','python']
            for kind in order:
                result,timing=python_run(paths[kind],'auto',workspace,profile,passphrase) if kind=='python' else rust_run(binary,paths[kind],'auto',workspace,profile,passphrase)
                outputs[kind]=result;measurements[kind]=timing
            assert outputs['python']==outputs['rust'],(shape,trial,'summary',outputs)
            connections={k:connect(v,passphrase) for k,v in paths.items()}
            dumps={k:dump(c) for k,c in connections.items()}
            for c in connections.values():c.close()
            assert json.dumps(dumps['python'],sort_keys=True)==json.dumps(dumps['rust'],sort_keys=True),(shape,trial,'database bytes')
            # Suggestions from the post-write state have to preserve wire order.
            py,_=python_run(paths['python'],'suggest',workspace,profile,passphrase)
            rs,_=rust_run(binary,paths['rust'],'suggest',workspace,profile,passphrase)
            assert py==rs,(shape,trial,'suggestions')
            runs.append(measurements)
            for p in paths.values():p.unlink()
        best={k:min(r[k]['seconds'] for r in runs) for k in ['python','rust']}
        best_cpu={k:min(r[k]['cpu_seconds'] for r in runs) for k in ['python','rust']}
        result=dict(book=shape,storage='sqlcipher' if args.cipher else 'sqlite',runs=runs,best=best,best_cpu=best_cpu,cpu_speedup=best_cpu['python']/best_cpu['rust'],native_cpu_scope='complete child, including open, serialization and commit',speedup=best['python']/best['rust'],database_parity=True,suggestion_byte_parity=True,suggestions_before=before_count,suggestions_after=len(json.loads(py)['candidates']))
        all_results.append(result);print(json.dumps(result),flush=True)
        baseline.unlink()
    (target/'results.json').write_text(json.dumps(all_results,indent=2)+'\n')


if __name__=='__main__':main()
