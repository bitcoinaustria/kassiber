#!/usr/bin/env python3
"""Measure Python, native persistence, and an identical prepared DML floor.

Only target/ scratch books are written. Rendering and dump comparison are
outside the timer; storage and COMMIT are inside, passive checkpoint outside.
Every run starts from the same physical database. At load >8 all five
Python/Rust pairs alternate order and the best of five is reported.
"""
from contextlib import ExitStack
from decimal import Decimal
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import uuid
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from kassiber.db import open_db
from kassiber.core import custody_journal,custody_filed_reports,filed_report_chain
from kassiber.core.journal_storage import journal_write_cache
from tests.rust_port.journal_persistence_oracle import capture,dump,CLOCK


class Recording:
    def __init__(self,conn): self.conn=conn;self.batches=[]
    def __getattr__(self,name): return getattr(self.conn,name)
    def execute(self,sql,args=()):
        if sql.lstrip().split()[0].upper() in ('INSERT','UPDATE','DELETE','REPLACE'):
            if self.batches and self.batches[-1][0]==sql: self.batches[-1][1].append(list(args))
            else: self.batches.append([sql,[list(args)]])
        return self.conn.execute(sql,args)
    def executemany(self,sql,rows):
        rows=list(rows);self.batches.append([sql,[list(r) for r in rows]])
        return self.conn.executemany(sql,rows)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--book',type=Path,required=True)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--modes',nargs='+',default=['fresh','unchanged','appended','changed'])
    parser.add_argument('--probe',type=Path,default=Path('target/release/examples/journal-persist'))
    parser.add_argument('--cipher',action='store_true',help='encrypt each disposable starting copy with a synthetic test key')
    args=parser.parse_args()
    target=Path('target').resolve();out=args.out.resolve()
    if not out.is_relative_to(target): parser.error('--out must be under target/')
    out.mkdir(parents=True,exist_ok=True)
    passphrase='synthetic-test-only' if args.cipher else None
    def connect(root): return open_db(root,passphrase=passphrase)
    results=[]
    for mode in args.modes:
        root=out/mode;root.mkdir(exist_ok=True)
        base=root/'base';base.mkdir(exist_ok=True)
        shutil.copyfile(args.book/'data/kassiber.sqlite3',base/'kassiber.sqlite3')
        if args.cipher:
            from sqlcipher3 import dbapi2 as sqlcipher
            plain=base/'kassiber.sqlite3';encrypted=base/'encrypted.sqlite3'
            c=sqlcipher.connect(plain)
            version=c.execute('PRAGMA user_version').fetchone()[0]
            c.execute('ATTACH DATABASE ? AS encrypted KEY ?',(str(encrypted),passphrase))
            c.execute("SELECT sqlcipher_export('encrypted')")
            c.execute(f'PRAGMA encrypted.user_version={int(version)}')
            c.execute('DETACH DATABASE encrypted');c.close()
            plain.unlink();encrypted.replace(plain)
        conn=connect(base)
        profile=conn.execute('SELECT * FROM profiles LIMIT 1').fetchone()
        count=conn.execute('SELECT COUNT(*) FROM transactions').fetchone()[0]
        if mode=='fresh':
            conn.execute('DELETE FROM journal_entries');conn.execute('DELETE FROM journal_quantity_postings')
        elif mode=='appended':
            rows=conn.execute("SELECT * FROM transactions WHERE direction='inbound' AND kind='buy' ORDER BY occurred_at LIMIT ?",(count//100,)).fetchall()
            for index,row in enumerate(rows):
                row=dict(row);row.update(id=f'append-{index}',fingerprint=f'append-{index}',external_id=f'append-{index}',occurred_at=f'2026-01-{index%28+1:02}T00:{index%60:02}:00Z')
                conn.execute(f'INSERT INTO transactions({",".join(row)}) VALUES({",".join("?" for _ in row)})',tuple(row.values()))
        elif mode=='changed':
            rows=conn.execute('SELECT id,fiat_rate_exact FROM transactions ORDER BY occurred_at').fetchall()
            for row in rows[::20]:
                rate=str(Decimal(row['fiat_rate_exact'] or '30000')*Decimal('1.01'))
                conn.execute('UPDATE transactions SET fiat_rate=?,fiat_rate_exact=? WHERE id=?',(float(rate),rate,row['id']))
        conn.execute('UPDATE profiles SET journal_input_version=journal_input_version+1');conn.commit()
        conn.execute('PRAGMA wal_checkpoint(TRUNCATE)')
        conn.close()
        reference=root/'reference';shutil.copytree(base,reference,dirs_exist_ok=True)
        conn=connect(reference);recorder=Recording(conn);states=[]
        original=custody_journal.store_ledger_state
        def observed(c,p,state,**kw):
            states.append(state);return original(c,p,state,**kw)
        with patch.object(custody_journal,'store_ledger_state',observed):
            vector=capture(recorder,mode,conn.execute('SELECT * FROM profiles LIMIT 1').fetchone())
        expected=vector.pop('expected');vector.pop('before');conn.close()
        native=root/'native';python=root/'python';floor=root/'floor'
        request=root/'request.json';floor_request=root/'floor.json'
        vector.update(database=str(native/'kassiber.sqlite3'))
        if passphrase: vector['passphrase']=passphrase
        request.write_text(json.dumps(dict(vector,expected=expected),separators=(',',':')))
        floor_request.write_text(json.dumps(dict(name=mode,database=str(floor/'kassiber.sqlite3'),passphrase=passphrase,floor=recorder.batches),separators=(',',':')))
        trials=[];probe=subprocess.Popen([str(args.probe.resolve())],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        def rust(path,dest):
            if dest.exists(): shutil.rmtree(dest)
            shutil.copytree(base,dest)
            load=os.getloadavg()[0];probe.stdin.write(str(path)+'\n');probe.stdin.flush()
            result=json.loads(probe.stdout.readline())
            if not result['ok']: raise RuntimeError(json.dumps(result)[:2000])
            return dict(seconds=result['seconds'],load1=load)
        def py():
            if python.exists(): shutil.rmtree(python)
            shutil.copytree(base,python);c=connect(python)
            c.execute('PRAGMA wal_autocheckpoint=0')
            identities=iter(vector['ids'])
            with ExitStack() as stack:
                stack.enter_context(patch.object(uuid,'uuid4',lambda:uuid.UUID(next(identities))))
                for module in (custody_journal,custody_filed_reports,filed_report_chain):
                    stack.enter_context(patch.object(module,'now_iso',lambda:CLOCK))
                stack.enter_context(journal_write_cache(c))
                c.execute('BEGIN IMMEDIATE');c.execute('SAVEPOINT journals_process')
                load=os.getloadavg()[0];start=time.perf_counter()
                original(c,vector['record']['profile'],states[0],created_at=CLOCK)
                c.execute('RELEASE journals_process');c.commit()
                seconds=time.perf_counter()-start
            assert dump(c)==expected
            c.execute('PRAGMA wal_checkpoint(PASSIVE)');c.close()
            return dict(seconds=seconds,load1=load)
        try:
            runs=5
            for run in range(runs):
                pair={}
                for engine in (['python','rust'] if run%2==0 else ['rust','python']):
                    pair[engine]=py() if engine=='python' else rust(request,native)
                pair['floor']=rust(floor_request,floor);trials.append(pair)
                print(mode,run,pair,flush=True)
        finally:
            probe.stdin.close();probe.wait(timeout=60)
        best={engine:min(t[engine]['seconds'] for t in trials) for engine in ('python','rust','floor')}
        results.append(dict(mode=mode,transactions=count,best=best,trials=trials,over_floor=best['rust']-best['floor']))
        (out/'summary.json').write_text(json.dumps(results,indent=2)+'\n')
        shutil.rmtree(root)


if __name__=='__main__': main()
