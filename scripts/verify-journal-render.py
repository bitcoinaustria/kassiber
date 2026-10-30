#!/usr/bin/env python3
"""Compare captured build() inputs with the local Rust rendering probe."""
import argparse
from collections import Counter
import json
from pathlib import Path
import subprocess


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(',', ':'), allow_nan=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directories', type=Path, nargs='+')
    parser.add_argument('--probe', type=Path, default=Path('target/release/examples/journal-render'))
    parser.add_argument('--summary', type=Path, required=True)
    args = parser.parse_args()
    summary = {}
    with subprocess.Popen([str(args.probe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          text=True, bufsize=1) as child:
        for directory in args.directories:
            counts = Counter()
            for path in sorted(directory.glob('*.json')):
                record = json.loads(path.read_text())
                if 'expected' not in record:
                    continue
                expected = record['expected']
                if record.get('response') is None and 'error' in expected:
                    counts['pre_render_refusals'] += 1
                    continue
                child.stdin.write(str(path) + '\n')
                child.stdin.flush()
                actual = json.loads(child.stdout.readline())
                counts['books'] += 1
                counts['refusals'] += int('error' in expected)
                if 'state' in expected:
                    matches = actual.get('state') == expected['state']
                    state = json.loads(expected['state'])
                    for field in ('entries', 'quarantines', 'ownership_review_candidates', 'quantity_differences'):
                        counts[field] += len(state[field])
                else:
                    matches = canonical(actual.get('error')) == canonical(expected['error'])
                if not matches:
                    counts['differences'] += 1
                    print(f'difference: {path}', flush=True)
                else:
                    counts['matched'] += 1
            summary[str(directory)] = dict(counts)
            print(f'{directory}: {dict(counts)}', flush=True)
        child.stdin.close()
        assert child.wait() == 0
    args.summary.write_text(json.dumps(summary, indent=2) + '\n')
    return int(any(c.get('differences') for c in summary.values()))


if __name__ == '__main__':
    raise SystemExit(main())
