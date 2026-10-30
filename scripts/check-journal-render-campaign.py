#!/usr/bin/env python3
"""Compare R4 render bytes on all generated books/permutations or local recordings.

Only unmatched operands are retained. The summary counts upstream refusals
separately; an upstream refusal is never a successful rendering comparison.
Run the custody replay gate separately to compare recorded custody stages.
"""
from __future__ import annotations

import argparse
from collections import Counter
import json
import hashlib
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from tests.custody_engine_compare import permuted_snapshot, compare_rust, rust_results, STAGES
from kassiber.core.custody_compute import compute_custody_projection
from tests.custody_gap_stage import suites
from tests.custody_snapshot import load, SCHEMA
from tests.rust_port.journal_render_oracle import capture_projection, canonical


def books(recordings):
    if recordings:
        for path in sorted(recordings.rglob('*.json')):
            document = load(path)
            if document.get('schema') != SCHEMA:
                continue
            book = document.get('input') or {}
            if book.get('schema_version') != 1:
                continue
            yield 'recorded', path.stem, False, book
            yield 'recorded', path.stem, True, permuted_snapshot(book)
    else:
        for suite, scenarios, generate in suites():
            for seed in range(2000):
                scenario = scenarios[seed % len(scenarios)]
                book = generate(seed)
                yield suite, scenario, False, book
                if seed < len(scenarios) * 2:
                    yield suite, scenario, True, permuted_snapshot(book)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe', type=Path, default=Path('target/release/examples/journal-render'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--recordings', type=Path)
    args = parser.parse_args()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=True)
    args.output.chmod(0o700)
    path = args.output / 'current.json'
    summary = {}
    failures = []
    sequence = 0
    active_inputs = {}
    with subprocess.Popen([str(args.probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          text=True, bufsize=1) as child:
        for index, (suite, scenario, permuted, book) in enumerate(books(args.recordings)):
            totals = summary.setdefault(suite, dict(totals=Counter(), scenarios={}))
            counts = totals['scenarios'].setdefault(scenario, Counter())
            activity = Counter(permutations=1) if permuted else Counter(books=1)
            try:
                stages = {}
                custody = compute_custody_projection(book, stage_observer=lambda name, value: stages.setdefault(name, value))
            except Exception as error:
                rust_results(book, python_error=error)
                activity['upstream_refusals'] += 1
                activity[f'upstream_{type(error).__name__}'] += 1
                activity['custody_refusals_matched'] += 1
            else:
                compared, skipped = compare_rust(book, None, python_stages=stages)
                assert set(compared) == set(STAGES) and not skipped, (suite, scenario, compared, skipped)
                activity['custody_compared'] += 1
                record = capture_projection(book['profile'], custody)
                if record['response'] is not None:
                    activity['active_requests'] += 1
                    if not permuted:
                        digest = hashlib.sha256(json.dumps(book, sort_keys=True, ensure_ascii=True).encode()).hexdigest()
                        active_inputs.setdefault(suite, set()).add(digest)
                        totals['distinct_active_seed_inputs'] = len(active_inputs[suite])
                if record['response'] is None and 'error' in record['expected']:
                    activity['pre_render_refusals'] += 1
                else:
                    path.touch(mode=0o600, exist_ok=True)
                    path.chmod(0o600)
                    path.write_text(json.dumps(record, ensure_ascii=True, allow_nan=True))
                    child.stdin.write(str(path) + '\n')
                    child.stdin.flush()
                    # A different path forces the probe to reload this book.
                    # Its last input stays immutable until the reply arrives.
                    actual = json.loads(child.stdout.readline())
                    expected = record['expected']
                    matched = (actual.get('state') == expected['state'] if 'state' in expected
                               else canonical(actual.get('error')) == canonical(expected['error']))
                    activity['compared'] += 1
                    activity['render_refusals'] += int('error' in expected)
                    activity['matched' if matched else 'byte_mismatches'] += 1
                    activity.update({{'differences': 'quantity_differences', 'ownership_candidates': 'ownership_review_candidates'}.get(key, key): value
                                     for key, value in record['activity'].items()})
                    if not matched:
                        failure = args.output / f'difference-{index}.json'
                        path.rename(failure)
                        actual_path = failure.with_suffix('.actual.json')
                        actual_path.touch(mode=0o600)
                        actual_path.write_text(json.dumps(actual, ensure_ascii=True))
                        failures.append(str(failure))
                    # The probe caches by path. Alternate two disposable names.
                    path.unlink(missing_ok=True)
                    sequence += 1
                    path = args.output / f'current-{sequence % 2}.json'
            counts.update(activity)
            totals['totals'].update(activity)
            if index % 500 == 0:
                (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
                print(suite, index, dict(totals['totals']), flush=True)
        child.stdin.close()
        assert child.wait() == 0
    path.unlink(missing_ok=True)
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps({suite: data['totals'] for suite, data in summary.items()}, indent=2), flush=True)
    if failures:
        print('\n'.join(failures), file=sys.stderr)
    if not args.recordings:
        assert sum(data['totals']['books'] for data in summary.values()) == 24000
        assert sum(data['totals']['permutations'] for data in summary.values()) == 1030
    vacuous = any(not data['totals']['compared'] or not data['totals']['active_requests']
                   for data in summary.values())
    return int(bool(failures) or not summary or vacuous)


if __name__ == '__main__':
    raise SystemExit(main())
