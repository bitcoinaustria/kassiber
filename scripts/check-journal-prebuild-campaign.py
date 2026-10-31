"""Compare complete ordered pre-build dumps on synthetic database books."""
from __future__ import annotations

import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path
import subprocess
import sqlite3
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tests.rust_port.journal_pipeline_oracle import capture
from tests.rust_port.journal_prebuild_campaign import PATH, activity, digest, generated, materialize

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
    parser.add_argument('--recordings', type=Path)
    parser.add_argument('--files', action='store_true')
    parser.add_argument('--frozen', action='store_true')
    args = parser.parse_args()
    scratch = ROOT / 'target/r4-10/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    summary = dict(schema='kassiber.r4-prebuild-campaign.v1', suites={}, comparisons=0, complete=False,
                   database_mode='SQLite and SQLCipher files' if args.files else 'native SQLCipher driver in memory',
                   adapter='Stored snapshot columns only, synthetic cache rows; no inferred inventory, ownership, or legacy pair links')
    args.out.parent.mkdir(parents=True, exist_ok=True)
    def save():
        args.out.write_text(json.dumps(summary, indent=2, sort_keys=True) + '\n')
    save()
    with tempfile.TemporaryDirectory(dir=scratch, prefix='campaign-') as directory:
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
            else:
                operands = generated(args.seeds)
            for index, (suite, scenario, seed_id, permuted, snapshot) in enumerate(operands):
                if args.recordings or args.frozen:
                    case = snapshot
                    omitted = {}
                else:
                    try:
                        conn, omitted = materialize(snapshot)
                    except sqlite3.IntegrityError as error:
                        totals = summary['suites'].setdefault(suite, dict(totals=Counter(), scenarios={}, omitted_columns=Counter()))
                        counts = totals['scenarios'].setdefault(scenario, Counter())
                        counts['schema_refusals'] += 1
                        counts['schema_refusal'] = str(error)
                        totals['totals']['schema_refusals'] += 1
                        continue
                    try:
                        case = capture(conn, f'{suite}/{scenario}/{seed_id}/{permuted}', snapshot['profile']['id'])
                    finally:
                        conn.close()
                totals = summary['suites'].setdefault(suite, dict(totals=Counter(), scenarios={}, omitted_columns=Counter()))
                counts = totals['scenarios'].setdefault(scenario, Counter())
                counts.update(activity(case))
                counts['permutations'] += int(permuted)
                totals['totals'].update(activity(case))
                totals['omitted_columns'].update(omitted)
                for cipher in ((False, True) if args.files else (False,)):
                    compare(child, case, Path(directory), cipher, args.files)
                    summary['comparisons'] += 1
                counts['driver_comparisons'] += 2 if args.files else 1
                counts['last_vector_sha256'] = digest(case)
                if index % 250 == 0:
                    save()
                    print(suite, index, summary['comparisons'], flush=True)
            child.stdin.close()
            assert child.wait(timeout=30) == 0
    assert summary['comparisons']
    if not args.recordings and not args.frozen:
        assert len(summary['suites']) == 12
        assert all(data['totals']['books'] + data['totals']['schema_refusals'] >= args.seeds for data in summary['suites'].values())
    summary['complete'] = True
    save()
    print(summary['comparisons'], 'full-dump driver comparisons', flush=True)


if __name__ == '__main__':
    main()
