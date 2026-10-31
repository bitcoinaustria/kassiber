#!/usr/bin/env python3
"""Compare the unregistered R4-13 prepared pipeline on disposable book copies.

This measures the prepared-book probe, not registered CLI/daemon acceptance.
Each pair compares every persisted scalar, including REAL bits, outside timing.
"""
import argparse
from contextlib import ExitStack
import hashlib
import itertools
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

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
CLOCK = '2026-10-31T12:00:00Z'


def worker(spec):
    from kassiber.cli import handlers
    from kassiber.core import custody_journal, custody_filed_reports, filed_report_chain
    from kassiber.db import open_db
    started = time.perf_counter()
    started_cpu = time.process_time()
    conn = open_db(Path(spec['root']), require_existing_schema=True)
    try:
        profile = dict(conn.execute('SELECT * FROM profiles').fetchone())
        original_store = custody_journal.store_ledger_state
        original_build = custody_journal.build_ledger_state
        initial_changes = conn.total_changes
        identities = []
        counter = itertools.count(1)

        def identity():
            value = uuid.UUID(int=next(counter), version=4)
            identities.append(str(value))
            return value

        def store(*args, **kwargs):
            with patch.object(uuid, 'uuid4', identity):
                return original_store(*args, **kwargs)

        def build(*args, **kwargs):
            if conn.total_changes != initial_changes:
                raise RuntimeError('benchmark does not qualify for the no-op R4-10 seam')
            return original_build(*args, **kwargs)

        phases = []
        def progress(value):
            phases.append(dict(**value, wall=time.perf_counter()-started, cpu=time.process_time()-started_cpu))

        with ExitStack() as stack:
            for module in (custody_journal, custody_filed_reports, filed_report_chain):
                stack.enter_context(patch.object(module, 'now_iso', lambda: CLOCK))
            stack.enter_context(patch.object(custody_journal, 'store_ledger_state', store))
            stack.enter_context(patch.object(custody_journal, 'build_ledger_state', build))
            payload = handlers.process_journals(conn, profile['workspace_id'], profile['id'], progress_observer=progress)
        assert payload['auto_priced'] == 0 and not payload.get('source_overlap_repair')
        assert not payload.get('custody_pair_evidence_repairs')
        return dict(ok=True, payload=payload, ids=identities, profile=profile, phases=phases,
                    wall_seconds=time.perf_counter()-started, cpu_seconds=time.process_time()-started_cpu)
    finally:
        conn.close()


def execute(argv, request):
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.perf_counter()
    result = subprocess.run(argv, input=json.dumps(request)+'\n', text=True,
                            capture_output=True, check=True, timeout=300, cwd=ROOT)
    wall = time.perf_counter()-start
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    if result.stderr:
        raise RuntimeError(result.stderr[-2000:])
    value = json.loads(result.stdout)
    assert value['ok'], value
    value['subprocess_wall'] = wall
    value['subprocess_cpu'] = after.ru_utime+after.ru_stime-before.ru_utime-before.ru_stime
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker', action='store_true', help=argparse.SUPPRESS)
    parser.add_argument('--books', type=Path)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--baseline-binary', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--pairs', type=int, default=5)
    args = parser.parse_args()
    if args.worker:
        print(json.dumps(worker(json.load(sys.stdin))))
        return
    if not args.books or not args.binary or not args.output:
        parser.error('--books, --binary and --output are required')
    if args.pairs < 1:
        parser.error('--pairs must be positive')
    scratch = ROOT/'target/r4-13/bench'
    scratch.mkdir(parents=True, exist_ok=True)
    from kassiber.db import open_db
    from tests.rust_port.journal_persistence_oracle import dump
    from tests.test_rust_journal_process import scalar_exact
    records = []
    measurement = dict(schema='kassiber.journal-process-prepared-bench.v1',
                       registered_command=False, acceptance='open', date=CLOCK[:10],
                       python_version=sys.version, trials_per_book=args.pairs,
                       native_binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                       baseline_binary_sha256=(hashlib.sha256(args.baseline_binary.read_bytes()).hexdigest()
                                               if args.baseline_binary else None),
                       records=records)
    for shape in ['generic-2000', 'multi-wallet-2000', 'generic-20000', 'multi-wallet-20000']:
        source = args.books/shape
        metadata = json.loads((source/'bench-book.json').read_text())
        assert metadata['schema'] == 'kassiber.journals-bench-book.v1'
        base = scratch/shape/'base'
        base.mkdir(parents=True, exist_ok=True)
        database = source/'data/kassiber.sqlite3'
        if Path(str(database)+'-wal').exists():
            raise RuntimeError('source benchmark must be checkpointed before a read-only snapshot')
        with sqlite3.connect(database.resolve().as_uri()+'?mode=ro&immutable=1', uri=True) as src:
            with sqlite3.connect(base/'kassiber.sqlite3') as dst:
                src.backup(dst)
        conn = open_db(base, require_existing_schema=True)
        conn.close()

        def clone(name):
            root = scratch/shape/name
            if root.exists():
                shutil.rmtree(root)
            root.mkdir()
            shutil.copy2(base/'kassiber.sqlite3', root/'kassiber.sqlite3')
            return root

        py_command = [sys.executable, str(Path(__file__).resolve()), '--worker']
        warm = execute(py_command, {'root': str(clone('warm'))})
        pairs = []
        for number in range(args.pairs):
            names = ['python', 'rust'] + (['baseline'] if args.baseline_binary else [])
            roots = {name: clone(f'{number}-{name}') for name in names}
            requests = {'python': {'root': str(roots['python'])},
                        'rust': dict(database=str(roots['rust']/'kassiber.sqlite3'),
                                     prepared_noop=True, clock=CLOCK, profile=warm['profile'], ids=warm['ids'])}
            if args.baseline_binary:
                requests['baseline'] = dict(requests['rust'], database=str(roots['baseline']/'kassiber.sqlite3'))
            outputs = {}
            load = os.getloadavg()
            order = (['python', 'baseline', 'rust'] if number % 2 == 0 else ['rust', 'baseline', 'python']) if args.baseline_binary else (['python', 'rust'] if number % 2 == 0 else ['rust', 'python'])
            for name in order:
                command = py_command if name == 'python' else [str((args.baseline_binary if name == 'baseline' else args.binary).resolve())]
                starting_load = os.getloadavg()
                outputs[name] = execute(command, requests[name])
                outputs[name]['load_before'] = starting_load
                outputs[name]['load_after'] = os.getloadavg()
            for name in names[1:]:
                assert outputs['python']['payload'] == outputs[name]['payload']
                assert [p['phase'] for p in outputs['python']['phases']] == [p['phase'] for p in outputs[name]['phases']]
            dumps = {}
            for name, root in roots.items():
                with sqlite3.connect(root/'kassiber.sqlite3') as c:
                    dumps[name] = dump(c)
            for name in names[1:]:
                for table in dumps['python']:
                    assert scalar_exact(dumps['python'][table], dumps[name][table]), (shape, name, table)
                assert dumps['python'].keys() == dumps[name].keys()
            for name in names:
                outputs[name]['dump_sha256'] = hashlib.sha256(
                    json.dumps(dumps[name], sort_keys=True, separators=(',', ':')).encode()
                ).hexdigest()
            for output in outputs.values():
                output.pop('ids', None)
                output.pop('profile', None)
            pairs.append(dict(load=load, complete_dump_equal=True, **outputs))
            print(shape, number, {k: round(v['subprocess_cpu'], 3) for k, v in outputs.items()}, flush=True)
            for root in roots.values():
                shutil.rmtree(root)
        records.append(dict(book=metadata, pairs=pairs))
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(measurement, indent=2)+'\n')
        shutil.rmtree(scratch/shape)


if __name__ == '__main__':
    main()
