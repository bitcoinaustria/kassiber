#!/usr/bin/env python3
"""Compare renderer checkpoints on identical captured operands and alternating CPU pairs."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess


def digest(path):
    with path.open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--operands', type=Path, required=True)
    parser.add_argument('--probe', nargs=2, action='append', required=True,
                        metavar=('LABEL', 'PATH'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cpu', type=int)
    parser.add_argument('--pairs', type=int, default=7)
    args = parser.parse_args()
    probes = [(label, Path(path).resolve()) for label, path in args.probe]
    if len(probes) < 2 or len({label for label, _ in probes}) != len(probes):
        parser.error('at least two probes with distinct labels are required')
    if args.cpu is not None and args.cpu not in os.sched_getaffinity(0):
        parser.error('--cpu must be in the current process affinity')
    if args.pairs < 7:
        parser.error('--pairs must be at least seven')
    args.output.mkdir(mode=0o700, parents=True, exist_ok=True)
    args.output.chmod(0o700)
    metadata = dict(cpu=args.cpu, pairs=args.pairs, probes={label: dict(path=str(path), sha256=digest(path))
                                        for label, path in probes}, operands={})
    summary = dict(metadata=metadata, books={})
    with (args.output / 'samples.jsonl').open('x', buffering=1) as log:
        for book in ('generic-20000', 'rich-20000'):
            path = (args.operands / f'{book}.json').resolve()
            metadata['operands'][book] = dict(path=str(path), sha256=digest(path))
            record = json.loads(path.read_text())
            expected = record['expected']['state']
            del record
            for label, probe in probes:
                result = subprocess.run([str(probe)], input=str(path) + '\n', text=True,
                                        capture_output=True, check=True)
                if json.loads(result.stdout).get('state') != expected:
                    raise RuntimeError(f'{book}: {label} state bytes differ')
            del expected, result
            samples = {label: [] for label, _ in probes}
            with ExitStack() as stack:
                children = []
                for label, probe in probes:
                    child = stack.enter_context(subprocess.Popen(
                        [str(probe), '--timing-only'], stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE, text=True, bufsize=1))
                    if args.cpu is not None:
                        os.sched_setaffinity(child.pid, {args.cpu})
                    children.append((label, child))
                try:
                    for trial in range(1, args.pairs + 1):
                        order = children if trial % 2 else list(reversed(children))
                        for label, child in order:
                            load = os.getloadavg()
                            child.stdin.write(str(path) + '\n')
                            child.stdin.flush()
                            result = json.loads(child.stdout.readline())
                            if 'error' in result or not result.get('cpu_seconds', 0) > 0:
                                raise RuntimeError(f'{book}: {label}: {result}')
                            row = dict(book=book, trial=trial, probe=label, load=load, **result)
                            log.write(json.dumps(row) + '\n')
                            print(json.dumps(row), flush=True)
                            samples[label].append(result['cpu_seconds'])
                finally:
                    for _, child in children:
                        child.stdin.close()
                    for label, child in children:
                        if child.wait() != 0:
                            raise RuntimeError(f'{label} probe failed')
            summary['books'][book] = {
                label: dict(best_cpu_seconds=min(values), median_cpu_seconds=statistics.median(values),
                            samples=values) for label, values in samples.items()}
            (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')


if __name__ == '__main__':
    main()
