"""Compare complete ordered pre-build dumps on synthetic database books."""
from __future__ import annotations

import argparse
from collections import Counter
import importlib.util
import json
import hashlib
import os
from pathlib import Path
import subprocess
import sqlite3
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tests.rust_port.journal_pipeline_oracle import capture
from tests.rust_port.journal_prebuild_campaign import PATH, activity, digest, generated, materialize, recorded_snapshots, historical_books

spec = importlib.util.spec_from_file_location('prebuild_check', ROOT / 'scripts/check-journal-prebuild.py')
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def compare(child, case, root, cipher, files):
    path = root / f'current-{cipher}.sqlite3'
    request = dict(case, stage='pipeline', database=str(path) if files else None, dump=False, reuse=True)
    if cipher:
        request['passphrase'] = 'synthetic-prebuild-test-only'
    child.stdin.write(json.dumps(request, ensure_ascii=True, allow_nan=False) + '\n')
    child.stdin.flush()
    line = child.stdout.readline()
    if not line:
        raise AssertionError(f'native probe exited: {case["name"]}')
    row = json.loads(line)
    assert row['verified'], f'{case["name"]}: complete dump mismatch'
    for actual, expected in (('result', 'result'), ('error', 'error'), ('projection_wire', 'wire_fields')):
        assert row[actual] == case[expected], f'{case["name"]}: {actual} mismatch'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--seeds', type=int, default=2000)
    inputs = parser.add_mutually_exclusive_group()
    inputs.add_argument('--recordings', type=Path)
    inputs.add_argument('--snapshots', type=Path)
    inputs.add_argument('--historical-books', action='store_true')
    parser.add_argument('--files', action='store_true')
    inputs.add_argument('--frozen', action='store_true')
    parser.add_argument('--activity-out', type=Path)
    parser.add_argument('--resume', action='store_true', help='verify and reuse the completed activity prefix')
    args = parser.parse_args()
    if args.seeds < 1:
        parser.error('--seeds must be positive')
    scratch = ROOT / 'target/r4-10/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    summary = dict(schema='kassiber.r4-prebuild-campaign.v1', suites={}, comparisons=0, complete=False,
                   database_mode='SQLite and SQLCipher files' if args.files else 'native SQLCipher driver in memory',
                   adapter='Stored snapshot columns only, synthetic cache rows; no inferred inventory, ownership, or legacy pair links')
    if args.recordings or args.historical_books or args.frozen:
        summary['adapter'] = 'Complete stored database dumps; no snapshot-to-database adaptation in this replay'
    args.out.parent.mkdir(parents=True, exist_ok=True)
    activity_path = args.activity_out or args.out.with_suffix('.activity.jsonl')
    activity_path.parent.mkdir(parents=True, exist_ok=True)
    summary['activity_path'] = str(activity_path)
    with args.binary.open('rb') as source:
        summary['binary_sha256'] = hashlib.file_digest(source, 'sha256').hexdigest()
    prefix = []
    if args.resume:
        previous = json.loads(args.out.read_text())
        for key in ('binary_sha256', 'database_mode', 'adapter'):
            if previous[key] != summary[key]:
                raise ValueError(f'resume {key} differs')
        prefix = [json.loads(line) for line in activity_path.read_text().splitlines()]
        summary['resumed_operands'] = len(prefix)
    start = time.monotonic()
    def save():
        args.out.write_text(json.dumps(summary, indent=2, sort_keys=True) + '\n')
    save()
    descriptor = os.open(activity_path, os.O_WRONLY | os.O_CREAT | (os.O_APPEND if args.resume else os.O_TRUNC), 0o600)
    with os.fdopen(descriptor, 'w') as audit, tempfile.TemporaryDirectory(dir=scratch, prefix='campaign-') as directory:
        with subprocess.Popen([str(args.binary.resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              text=True, bufsize=1, preexec_fn=check.deny_network) as child:
            if args.frozen:
                corpus = json.loads(PATH.read_text())
                def expand(case):
                    case = dict(case)
                    for side in ('before', 'expected'):
                        case[side] = {table: dict(data, rows=case[side].get(table, data['rows'])) for table, data in corpus['base'].items()}
                    return case
                operands = (("frozen", case['name'], 0, False, expand(case)) for case in corpus['cases'])
            elif args.recordings:
                operands = (("recorded", p.stem, 0, False, json.loads(p.read_text()))
                            for p in sorted(args.recordings.rglob('*.prebuild.json')))
            elif args.snapshots:
                operands = recorded_snapshots(args.snapshots)
            elif args.historical_books:
                operands = historical_books()
            else:
                operands = generated(args.seeds)
            for index, (suite, scenario, seed_id, permuted, snapshot) in enumerate(operands):
                totals = summary['suites'].setdefault(suite, dict(totals=Counter(), scenarios={}, omitted_columns=Counter(), schema_refusal_messages=Counter()))
                counts = totals['scenarios'].setdefault(scenario, Counter())
                attempted = Counter(attempts=1, seed_attempts=int(not permuted), permutation_attempts=int(permuted))
                counts.update(attempted)
                totals['totals'].update(attempted)
                entry = dict(suite=suite, scenario=scenario, seed=seed_id, permuted=permuted, input_sha256=digest(snapshot))
                if index < len(prefix):
                    retained = prefix[index]
                    if any(retained[key] != value for key, value in entry.items()):
                        raise ValueError(f'resume operand {index} differs')
                    if 'schema_refusal' in retained:
                        counts['schema_refusals'] += 1
                        totals['totals']['schema_refusals'] += 1
                        totals['schema_refusal_messages'][retained['schema_refusal']] += 1
                        continue
                    omitted = retained.get('omitted_columns')
                    if omitted is None and not (args.recordings or args.frozen or args.historical_books):
                        conn, omitted = materialize(snapshot)
                        conn.close()
                    totals['omitted_columns'].update(omitted or {})
                    counts.update(retained['activity'])
                    totals['totals'].update(retained['activity'])
                    counts['permutations'] += int(permuted)
                    counts['driver_comparisons'] += retained['driver_comparisons']
                    counts['last_vector_sha256'] = retained['vector_sha256']
                    summary['comparisons'] += retained['driver_comparisons']
                    continue
                if args.recordings or args.frozen or args.historical_books:
                    case = snapshot
                    omitted = {}
                else:
                    try:
                        conn, omitted = materialize(snapshot)
                    except sqlite3.IntegrityError as error:
                        counts['schema_refusals'] += 1
                        totals['totals']['schema_refusals'] += 1
                        totals['schema_refusal_messages'][str(error)] += 1
                        audit.write(json.dumps(dict(entry, schema_refusal=str(error)), sort_keys=True) + '\n')
                        continue
                    try:
                        case = capture(conn, f'{suite}/{scenario}/{seed_id}/{permuted}', snapshot['profile']['id'])
                    finally:
                        conn.close()
                counts.update(activity(case))
                counts['permutations'] += int(permuted)
                totals['totals'].update(activity(case))
                totals['omitted_columns'].update(omitted)
                for cipher in ((False, True) if args.files else (False,)):
                    compare(child, case, Path(directory), cipher, args.files)
                    summary['comparisons'] += 1
                counts['driver_comparisons'] += 2 if args.files else 1
                counts['last_vector_sha256'] = digest(case)
                audit.write(json.dumps(dict(entry, vector_sha256=counts['last_vector_sha256'], activity=dict(activity(case)), omitted_columns=dict(omitted), driver_comparisons=2 if args.files else 1), sort_keys=True) + '\n')
                if index % 250 == 0:
                    audit.flush()
                    save()
                    print(suite, index, summary['comparisons'], flush=True)
            child.stdin.close()
            assert child.wait(timeout=30) == 0
    assert summary['comparisons']
    assert index + 1 >= len(prefix), 'resume prefix exceeds selected corpus'
    if not args.recordings and not args.frozen and not args.snapshots and not args.historical_books:
        from tests.custody_gap_stage import suites
        assert len(summary['suites']) == 12
        for suite, scenarios, _ in suites():
            counts = summary['suites'][suite]['totals']
            assert counts['seed_attempts'] == args.seeds
            assert counts['permutation_attempts'] == min(args.seeds, len(scenarios) * 2)
            assert counts['books'] + counts['schema_refusals'] == counts['attempts']
    with activity_path.open('rb') as source:
        summary['activity_sha256'] = hashlib.file_digest(source, 'sha256').hexdigest()
    summary['elapsed_seconds'] = time.monotonic() - start
    summary['complete'] = True
    save()
    print(summary['comparisons'], 'full-dump driver comparisons', flush=True)


if __name__ == '__main__':
    main()
