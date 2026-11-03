#!/usr/bin/env python3
"""Time registered journal routes from process start to exit, Python against native.

Each pair runs unchanged Python (`tests.rust_port.journal_cli_reference`) and
the given binary on fresh copies of one bench book, alternating which goes
first. Both bind the registered tests' clock and identities, so after timing
the script requires identical exit status, output bytes (desktop: request rows,
with only the interpreter traceback allowance) and scalar-exact complete
books. CPU is the child's user+system time from `RUSAGE_CHILDREN`, including
interpreter or binary startup, book open, the response, deferred checkpoint
and exit. The one-minute load average is sampled before each run.

    uv run --locked python scripts/benchmark-journal-routes.py \\
        --books <dir with generic-2000/ ...> --binary target/release/kassiber \\
        --output target/c8/bench/routes.json
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import shutil
import sqlite3
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
CLOCK = '2026-10-31T12:00:00Z'
BOOKS = ('generic-2000', 'multi-wallet-2000', 'generic-20000', 'multi-wallet-20000')
KIND = 'ui.journals.process'


def environment(scratch):
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('KASSIBER_OPERATOR_', 'KASSIBER_RUST_TEST_', 'KASSIBER_HYBRID_'))}
    owners = scratch / 'owners'
    owners.mkdir(parents=True, exist_ok=True)
    env.update(KASSIBER_RUST_TEST_OWNER_ROOT=str(owners), KASSIBER_RUST_TEST_JOURNAL_TIME=CLOCK,
               KASSIBER_RUST_TEST_JOURNAL_IDS='sequential', KASSIBER_TEST_NO_EGRESS='1',
               KASSIBER_DISABLE_UPDATE_CHECK='1', XDG_DATA_HOME=str(scratch / 'xdg-data'),
               XDG_CONFIG_HOME=str(scratch / 'xdg-config'), XDG_STATE_HOME=str(scratch / 'xdg-state'))
    return env


def command(runtime, binary, root, route):
    prefix = [str(binary)] if runtime == 'rust' else [sys.executable, '-m', 'tests.rust_port.journal_cli_reference']
    if route == 'desktop':
        return [*prefix, '--data-root', str(root), 'daemon']
    return [*prefix, '--machine', '--data-root', str(root), 'journals', 'process',
            *(('--auto-pair',) if route == 'auto' else ())]


def daemon_input():
    rows = [dict(kind='status', request_id='open'), dict(kind=KIND, request_id='process'),
            dict(kind='daemon.shutdown', request_id='stop')]
    return ''.join(json.dumps(row) + '\n' for row in rows)


def run(runtime, binary, root, route, env):
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    load = os.getloadavg()
    started = time.perf_counter()
    if route == 'desktop' and runtime == 'rust':
        # The native worker answers asynchronously; send shutdown only after
        # the terminal row, exactly as the desktop would.
        process = subprocess.Popen(command(runtime, binary, root, route), cwd=ROOT, env=env, text=True,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        lines = [process.stdout.readline()]
        for row in daemon_input().splitlines(keepends=True):
            process.stdin.write(row)
            process.stdin.flush()
            request = json.loads(row)['request_id']
            while True:
                line = process.stdout.readline()
                if not line:
                    raise RuntimeError('native daemon closed output')
                lines.append(line)
                value = json.loads(line)
                if value.get('request_id') == request and value['kind'] != KIND + '.progress':
                    break
        process.stdin.close()
        stdout = ''.join(lines) + process.stdout.read()
        stderr = process.stderr.read()
        process.wait(timeout=600)
        returncode = process.returncode
    else:
        completed = subprocess.run(command(runtime, binary, root, route), cwd=ROOT, env=env, text=True,
                                   input=daemon_input() if route == 'desktop' else '', capture_output=True,
                                   timeout=1800, check=False)
        stdout, stderr, returncode = completed.stdout, completed.stderr, completed.returncode
    wall = time.perf_counter() - started
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime
    return dict(returncode=returncode, stdout=stdout, stderr=stderr, wall=wall, cpu=cpu, load=load)


def comparable(route, output):
    if route != 'desktop':
        return [output['returncode'], output['stdout'], output['stderr']]
    rows = [json.loads(line) for line in output['stdout'].splitlines() if line.strip()]
    rows = [row for row in rows if row.get('request_id') == 'process']
    for row in rows:
        error = row.get('error') if row['kind'] == 'error' else None
        if error and error.get('code') == 'internal_error' and isinstance(error.get('debug'), str):
            error['debug'] = None
    return [output['returncode'], rows]


def median(values):
    ordered = sorted(values)
    middle = len(ordered) // 2
    return ordered[middle] if len(ordered) % 2 else (ordered[middle - 1] + ordered[middle]) / 2


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--books', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--routes', nargs='+', default=['plain', 'auto', 'desktop'], choices=['plain', 'auto', 'desktop'])
    parser.add_argument('--pairs', type=int, default=5)
    parser.add_argument('--shapes', nargs='+', default=list(BOOKS), choices=BOOKS)
    args = parser.parse_args()
    output = args.output.resolve()
    if not output.is_relative_to((ROOT / 'target').resolve()):
        parser.error('output must stay in the worktree target directory')
    from tests.rust_port.journal_persistence_oracle import dump
    from tests.test_rust_journal_process import scalar_exact
    binary = args.binary.resolve()
    scratch = output.parent / 'bench-scratch'
    measurement = dict(schema='kassiber.journal-route-bench.v1', date=time.strftime('%Y-%m-%d', time.gmtime()),
                       registered_command=True, clock=CLOCK, python_version=sys.version, pairs_per_cell=args.pairs,
                       cpus=os.cpu_count(), native_binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                       cells=[])
    for shape in args.shapes:
        source = args.books / shape
        metadata = json.loads((source / 'bench-book.json').read_text())
        database = source / 'data/kassiber.sqlite3'
        if Path(str(database) + '-wal').exists() and Path(str(database) + '-wal').stat().st_size:
            raise RuntimeError(f'{database} must be checkpointed before a read-only snapshot')
        base = scratch / shape / 'base'
        shutil.rmtree(base, ignore_errors=True)
        base.mkdir(parents=True)
        with sqlite3.connect(database.resolve().as_uri() + '?mode=ro&immutable=1', uri=True) as src:
            with sqlite3.connect(base / 'kassiber.sqlite3') as dst:
                src.backup(dst)
        # Apply any pending compatibility migration once, outside timing, so
        # both runtimes open the same current book.
        from kassiber.db import open_db
        open_db(base, require_existing_schema=True).close()
        for route in args.routes:
            pairs = []
            for number in range(args.pairs):
                roots = {}
                for runtime in ('python', 'rust'):
                    roots[runtime] = scratch / shape / f'{route}-{number}-{runtime}'
                    shutil.rmtree(roots[runtime], ignore_errors=True)
                    roots[runtime].mkdir(parents=True)
                    shutil.copy2(base / 'kassiber.sqlite3', roots[runtime] / 'kassiber.sqlite3')
                outputs = {}
                for runtime in (('python', 'rust') if number % 2 == 0 else ('rust', 'python')):
                    env = environment(scratch / shape / f'{route}-{number}-{runtime}-env')
                    outputs[runtime] = run(runtime, binary, roots[runtime], route, env)
                if comparable(route, outputs['python']) != comparable(route, outputs['rust']):
                    raise AssertionError((shape, route, number, outputs['python']['stdout'][-2000:],
                                          outputs['rust']['stdout'][-2000:], outputs['rust']['stderr'][-2000:]))
                dumps = {}
                for runtime, root in roots.items():
                    with sqlite3.connect(root / 'kassiber.sqlite3') as conn:
                        dumps[runtime] = dump(conn)
                if not scalar_exact(dumps['python'], dumps['rust']):
                    raise AssertionError(f'{shape} {route} pair {number}: books differ')
                pairs.append({runtime: dict(cpu=round(value['cpu'], 4), wall=round(value['wall'], 4),
                                            load=[round(v, 2) for v in value['load']], returncode=value['returncode'])
                              for runtime, value in outputs.items()})
                print(shape, route, number, {k: v['cpu'] for k, v in pairs[-1].items()}, flush=True)
                for root in roots.values():
                    shutil.rmtree(root)
                shutil.rmtree(scratch / shape / f'{route}-{number}-python-env', ignore_errors=True)
                shutil.rmtree(scratch / shape / f'{route}-{number}-rust-env', ignore_errors=True)
            summary = {runtime: dict(cpu_best=min(p[runtime]['cpu'] for p in pairs),
                                     cpu_median=median([p[runtime]['cpu'] for p in pairs]),
                                     wall_median=median([p[runtime]['wall'] for p in pairs]))
                       for runtime in ('python', 'rust')}
            summary['cpu_speedup'] = summary['python']['cpu_median'] / summary['rust']['cpu_median']
            loads = [p[r]['load'][0] for p in pairs for r in ('python', 'rust')]
            summary['load_1m_range'] = [min(loads), max(loads)]
            measurement['cells'].append(dict(book=shape, transactions=metadata.get('transactions'), route=route,
                                             summary=summary, pairs=pairs, outputs_and_books_equal=True))
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps(measurement, indent=2) + '\n')
        shutil.rmtree(scratch / shape, ignore_errors=True)
    shutil.rmtree(scratch, ignore_errors=True)


if __name__ == '__main__':
    main()
