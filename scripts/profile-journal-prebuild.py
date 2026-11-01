"""Measure whole-stage process CPU on disposable, already-built 20k books."""
import argparse
from contextlib import ExitStack
from dataclasses import asdict, replace
import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import statistics
import sys
import time
import uuid
from unittest.mock import patch

root=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(root))
for name in tuple(os.environ):
    if name.startswith(('KASSIBER_TEST_CUSTODY_','KASSIBER_TEST_JOURNAL_PREBUILD_')):
        os.environ.pop(name)
os.environ['KASSIBER_NO_EGRESS']='1'
os.environ['KASSIBER_OFFLINE_PREFERENCE_FILE']=str(root/'target/r4-10/scratch/cpu-offline-preference.json')
from kassiber.db import open_db
from kassiber.cli import handlers
from kassiber.core import custody_authored_migration as migration
from tests.rust_port.journal_pipeline_oracle import capture
from tests.rust_port.journal_persistence_oracle import dump
spec=importlib.util.spec_from_file_location('prebuild_check',root/'scripts/check-journal-prebuild.py')
check=importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

parser=argparse.ArgumentParser()
parser.add_argument('--binary',type=Path,required=True)
parser.add_argument('--out',type=Path,required=True)
parser.add_argument('--books-root',type=Path,default=root/'target/r4-10/books')
parser.add_argument('--repeats',type=int,default=7)
args=parser.parse_args()
if args.repeats < 3:
    parser.error('--repeats must be at least three')
if not args.books_root.resolve().is_relative_to((root/'target').resolve()):
    parser.error('--books-root must be disposable data below this checkout target/')
if not args.out.resolve().is_relative_to((root/'target').resolve()):
    parser.error('--out must be below this checkout target/')
args.out.parent.mkdir(parents=True,exist_ok=True)

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()

def python_stage(c,case,cache):
    ticks=iter(case['ticks'])
    ids=(uuid.UUID(int=n) for n in itertools.count(1))
    clock=lambda: next(ticks)
    with ExitStack() as stack:
        stack.enter_context(patch('uuid.uuid4',side_effect=lambda: next(ids)))
        stack.enter_context(patch.object(handlers,'_metadata_hooks',return_value=replace(handlers._metadata_hooks(),now_iso=clock)))
        stack.enter_context(patch.object(migration,'now_iso',side_effect=clock))
        stack.enter_context(patch('kassiber.core.custody_components._now_iso',side_effect=clock))
        c.execute('BEGIN IMMEDIATE')
        previous=c.execute('PRAGMA cache_size').fetchone()[0]
        start=time.process_time()
        if cache: c.execute('PRAGMA cache_size=-131072')
        profile=case['profile']
        repair=handlers._repair_journal_source_overlaps(c,profile)
        warning=handlers._journal_source_overlap_warning(c,profile,repair)
        legacy=migration.refresh_drifted_legacy_pair_components(c,profile_id=profile['id'])
        if legacy.changed:
            profile=dict(c.execute('SELECT * FROM profiles WHERE id=?',(profile['id'],)).fetchone())
        priced=handlers.auto_price_transactions_from_rates_cache(c,profile)
        result=dict(repair=repair,warning=warning,consolidation=asdict(legacy),profile=profile,auto_priced=priced)
        fields={}
        if repair is not None: fields['source_overlap_repair']=repair
        if warning is not None: fields['source_overlap_warning']=warning
        if legacy.activated: fields['custody_pair_evidence_repairs']=legacy.activated
        wire=json.dumps(fields)
        if cache: c.execute(f'PRAGMA cache_size={previous}')
        elapsed=time.process_time()-start
        assert next(ticks,None) is None
        assert result==case['result'] and wire==case['wire_fields']
        assert dump(c)==case['expected']
        c.rollback()
        return elapsed

records=[]
with ExitStack() as guard:
    for target in ('socket.getaddrinfo','socket.socket.connect','socket.socket.connect_ex','socket.socket.sendto'):
        guard.enter_context(patch(target,side_effect=AssertionError('benchmark attempted network I/O')))
    for kind in ('generic','multi-wallet'):
        book=args.books_root/f'{kind}-20000'
        database=book/'data/kassiber.sqlite3'
        c=open_db(book/'data')
        profile=dict(c.execute('SELECT * FROM profiles LIMIT 1').fetchone())
        case=capture(c,kind,profile['id'])
        c.close()
        assert case['error'] is None
        record=dict(kind=kind,transactions=20000,manifest=json.loads((book/'bench-book.json').read_text()),
            database_sha256=sha(database),activity=case['result'],samples=[],phase_samples=[])
        for repeat in range(args.repeats):
            for cache in (False,True):
                c=open_db(book/'data')
                load=os.getloadavg()
                pycpu=python_stage(c,case,cache)
                c.close()
                request=dict(case,before=None,stage='pipeline',database=str(database),dump=False,rebuild_cache=cache)
                native=check.native(args.binary.resolve(),[request])[0]
                assert native['verified'] and native['error'] is None and native['result']==case['result'] and native['projection_wire']==case['wire_fields']
                record['samples'].append(dict(repeat=repeat,rebuild_cache=cache,python_cpu_seconds=pycpu,
                    native_cpu_seconds=native['cpu_seconds'],native_wall_seconds=native['wall_seconds'],load_average=load))
        for cache in (False,True):
            for repeat in range(3):
                request=dict(case,before=None,stage='pipeline',database=str(database),dump=False,rebuild_cache=cache,phase_breakdown=True)
                native=check.native(args.binary.resolve(),[request])[0]
                assert native['verified'] and native['error'] is None and native['result']==case['result'] and native['projection_wire']==case['wire_fields']
                record['phase_samples'].append(dict(repeat=repeat,rebuild_cache=cache,native_cpu_seconds=native['cpu_seconds'],cpu_by_phase=native['cpu_by_phase'],load_average=os.getloadavg()))
        record['summary']={str(cache):dict(
            python_median=statistics.median(s['python_cpu_seconds'] for s in record['samples'] if s['rebuild_cache']==cache),
            native_median=statistics.median(s['native_cpu_seconds'] for s in record['samples'] if s['rebuild_cache']==cache),
            native_best=min(s['native_cpu_seconds'] for s in record['samples'] if s['rebuild_cache']==cache)) for cache in (False,True)}
        records.append(record)
        result=dict(schema='kassiber.r4-prebuild-cpu.v1',clock='user plus system process CPU; validation and fixture/open work outside timer',
            binary_sha256=sha(args.binary),script_sha256=sha(Path(__file__)),target_seconds=0.03,records=records)
        args.out.write_text(json.dumps(result,indent=2,sort_keys=True)+'\n')
        print(kind,json.dumps(record['summary']),flush=True)
