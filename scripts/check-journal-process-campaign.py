"""Resumable complete-process comparisons on disposable stored book operands."""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import sqlite3
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from tests.custody_engine_compare import permuted_snapshot
from tests.custody_gap_stage import suites
from tests.custody_snapshot import SCHEMA, load
from tests.rust_port.journal_native_process_oracle import capture
from tests.rust_port.journal_prebuild_campaign import materialize
from tests.rust_port.journal_prebuild_oracle import CLOCK


def digest(data):
    return hashlib.sha256(data).hexdigest()


def exact(a, b):
    if type(a) is not type(b):
        return False
    if isinstance(a, float):
        return struct.pack('!d', a) == struct.pack('!d', b)
    if isinstance(a, list):
        return len(a) == len(b) and all(exact(x, y) for x, y in zip(a, b))
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(exact(a[k], b[k]) for k in a)
    return a == b


def operands(recordings=None, seeds=2000):
    if recordings:
        for path in sorted(recordings.rglob('*.json')):
            document = load(path)
            snapshot = document.get('input') or {}
            if document.get('schema') != SCHEMA or snapshot.get('schema_version') != 1:
                continue
            for permuted in (False, True):
                yield dict(suite='recorded', scenario=path.stem, seed=0, permuted=permuted), lambda s=snapshot, p=permuted: permuted_snapshot(s) if p else s
    else:
        for suite, scenarios, generate in suites():
            for seed in range(seeds):
                for permuted in ((False, True) if seed < len(scenarios) * 2 else (False,)):
                    yield dict(suite=suite, scenario=scenarios[seed % len(scenarios)], seed=seed, permuted=permuted), lambda g=generate, s=seed, p=permuted: permuted_snapshot(g(s)) if p else g(s)


def fingerprint(binary, recordings):
    paths = {Path(__file__), *ROOT.glob('crates/**/*.rs'), *ROOT.glob('crates/**/Cargo.toml'),
             *ROOT.glob('kassiber/**/*.py'), *ROOT.glob('tests/custody*.py'),
             *ROOT.glob('tests/rust_port/journal*.py'), ROOT / 'Cargo.lock', ROOT / 'uv.lock'}
    if recordings:
        paths.update(recordings.rglob('*.json'))
    return dict(binary=digest(binary.read_bytes()), sources=digest(''.join(
        str(p.relative_to(ROOT) if p.is_relative_to(ROOT) else p) + ':' + digest(p.read_bytes()) + '\n'
        for p in sorted(paths)).encode()))


def compare(child, case):
    request = dict(before=case['before'], native_prebuild=True, profile=case['profile'],
                   clock=CLOCK, ticks=case['ticks'], ids=case['ids'], dump=True)
    if 'schema' in case:
        request['schema'] = case['schema']
    child.stdin.write(json.dumps(request, ensure_ascii=True, allow_nan=False) + '\n')
    child.stdin.flush()
    line = child.stdout.readline()
    if not line:
        raise RuntimeError('native process probe exited before replying')
    actual = json.loads(line)
    checks = dict(outcome=actual.get('ok') == (case['error'] is None),
                  result=exact(actual.get('payload') if case['error'] is None else actual.get('refusal'),
                               case['result'] if case['error'] is None else case['error']),
                  ordered_result=case['error'] is not None or actual.get('payload_json') == json.dumps(case['result']),
                  dump=exact(actual.get('dump'), case['expected']),
                  phases=[dict(phase=p['phase']) for p in actual.get('phases', [])] == case['phases'],
                  clocks=actual.get('clock_reads') == len(case['ticks']),
                  identities=actual.get('uuid_reads') == len(case['ids']),
                  build_clock=actual.get('build_clock_reads') == int({'phase': 'building'} in case['phases']))
    return checks, request, actual


def write_private(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'w') as output:
        json.dump(value, output, ensure_ascii=True, allow_nan=False)
        output.write('\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--recordings', type=Path)
    parser.add_argument('--seeds', type=int, default=2000)
    parser.add_argument('--shard-size', type=int, default=250)
    parser.add_argument('--start-shard', type=int, default=0)
    parser.add_argument('--max-shards', type=int)
    args = parser.parse_args()
    if args.seeds < 1 or args.shard_size < 1 or args.start_shard < 0 or (args.max_shards is not None and args.max_shards < 1):
        parser.error('positive sizes and nonnegative start shard required')
    destination = args.output.resolve()
    if not destination.is_relative_to((ROOT / 'target').resolve()):
        parser.error('campaign output must stay in the worktree target directory')
    destination.mkdir(mode=0o700, parents=True, exist_ok=True)
    destination.chmod(0o700)
    corpus = list(operands(args.recordings, args.seeds))
    if args.recordings and len(corpus) != 318:
        parser.error(f'expected 159 retained recordings and permutations; found {len(corpus)} operands')
    if not args.recordings and args.seeds == 2000:
        assert len(corpus) == 25030, len(corpus)
    manifest = dict(schema='kassiber.r4-process-campaign.v1', **fingerprint(args.binary.resolve(), args.recordings),
                    seeds=args.seeds, operands=len(corpus), shard_size=args.shard_size,
                    adapter='Stored columns only; omitted derived fields are counted. Synthetic rate cache; no inferred ownership or legacy pair links.')
    manifest_path = destination / 'manifest.json'
    if manifest_path.exists():
        if json.loads(manifest_path.read_text()) != manifest:
            parser.error('campaign fingerprint changed; choose a new output directory')
    else:
        write_private(manifest_path, manifest)
    spec = importlib.util.spec_from_file_location('prebuild_check', ROOT / 'scripts/check-journal-prebuild.py')
    guard = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(guard)
    rows = []
    errors = []
    stop = len(corpus) if args.max_shards is None else min(len(corpus), (args.start_shard + args.max_shards) * args.shard_size)
    with subprocess.Popen([str(args.binary.resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          text=True, bufsize=1, preexec_fn=guard.deny_network) as child:
        try:
            for index in range(args.start_shard * args.shard_size, stop):
                shard = index // args.shard_size
                path = destination / f'{shard:04}.jsonl'
                if index % args.shard_size == 0 or not rows:
                    rows = [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
                    expected = list(range(shard * args.shard_size, shard * args.shard_size + len(rows)))
                    if [row['index'] for row in rows] != expected:
                        raise ValueError('incomplete or out-of-order shard log')
                if any(row['index'] == index for row in rows):
                    continue
                metadata, generate = corpus[index]
                snapshot = generate()
                row = dict(index=index, **metadata, input_sha256=digest(json.dumps(snapshot, sort_keys=True, ensure_ascii=True).encode()))
                relaxed = False
                try:
                    conn, omitted = materialize(snapshot)
                except sqlite3.IntegrityError:
                    relaxed = True
                    conn, omitted = materialize(snapshot, relaxed=True)
                try:
                    schema = [list(r) for r in conn.execute("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")] if relaxed else None
                    profile = dict(conn.execute('SELECT * FROM profiles WHERE id=?', (snapshot['profile']['id'],)).fetchone())
                    case = capture(dict(name=str(index), profile=profile), connection=conn)
                    if schema is not None:
                        case['schema'] = schema
                finally:
                    conn.close()
                checks, request, actual = compare(child, case)
                row.update(checks=checks, matched=all(checks.values()), omitted_columns=dict(omitted), relaxed_schema=relaxed,
                           transactions=len(case['before']['transactions']['rows']),
                           entries=(case['result'] or {}).get('entries_created', 0),
                           quarantines=(case['result'] or {}).get('quarantined', 0),
                           refused=case['error'] is not None,
                           before_sha256=digest(json.dumps(case['before'], sort_keys=True).encode()))
                if not row['matched']:
                    write_private(destination / f'failure-{index:05}.json', dict(case=case, request=request, actual=actual))
                    errors.append(index)
                fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
                with os.fdopen(fd, 'a') as output:
                    output.write(json.dumps(row, sort_keys=True) + '\n')
                    output.flush()
                    os.fsync(output.fileno())
                rows.append(row)
                if index % 50 == 0 or not row['matched']:
                    print(index, metadata['suite'], 'matched' if row['matched'] else checks, flush=True)
                if errors:
                    break
        finally:
            child.stdin.close()
            try:
                child.wait(timeout=30)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
            if child.returncode:
                raise RuntimeError(f'native campaign probe exit {child.returncode}')
    all_rows = [json.loads(line) for path in sorted(destination.glob('*.jsonl')) for line in path.read_text().splitlines()]
    counts = Counter(operands=len(all_rows), books=sum(not r['permuted'] for r in all_rows),
                     permutations=sum(r['permuted'] for r in all_rows), matches=sum(r['matched'] for r in all_rows),
                     refusals=sum(r['refused'] for r in all_rows))
    suite_counts = {}
    for row in all_rows:
        suite = suite_counts.setdefault(row['suite'], dict(totals=Counter(), scenarios={}))
        activity = Counter(books=int(not row['permuted']), permutations=int(row['permuted']),
                           matches=int(row['matched']), refusals=int(row['refused']),
                           relaxed_schema=int(row['relaxed_schema']), transactions=row['transactions'],
                           entries=row['entries'], quarantines=row['quarantines'])
        suite['totals'].update(activity)
        suite['scenarios'].setdefault(row['scenario'], Counter()).update(activity)
    summary = dict(manifest=manifest, counts=counts, suites=suite_counts, complete=len(all_rows) == len(corpus),
                   failures=[r['index'] for r in all_rows if not r['matched']])
    write_private(destination / 'summary.json', summary)
    print(json.dumps(dict(counts, complete=summary['complete'])), flush=True)
    return int(bool(summary['failures']))


if __name__ == '__main__':
    raise SystemExit(main())
