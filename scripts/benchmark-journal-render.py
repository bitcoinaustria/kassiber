#!/usr/bin/env python3
"""R4-11 render + joins parity and back-to-back timing on copied v1 snapshots.

Build the probe with cargo build --offline --release -p kassiber-journals
--example journal-render. Supply a disposable --output directory under target/.
Custody, tax computation, JSON parsing/encoding and persistence are outside the
measured span on both sides. Every book first compares the complete state bytes.
"""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from kassiber.core.custody_compute import compute_custody_projection
from kassiber.core.custody_journal import CustodyJournalBuilder
from kassiber.core.engines import ledger
from tests.rust_port.journal_render_oracle import capture_build, canonical
from tests.custody_snapshot import to_jsonable


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--snapshots', type=Path, required=True)
    parser.add_argument('--probe', type=Path, default=Path('target/release/examples/journal-render'))
    parser.add_argument('--output', type=Path, default=Path('target/r4-render/benchmark'))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    timings = []
    for shape in ('generic', 'rich'):
        for size in (2000, 20000):
            name = f'{shape}-{size}'
            snapshot = json.loads((args.snapshots / f'snap-{name}.json').read_text())
            custody = compute_custody_projection(snapshot)
            builder = CustodyJournalBuilder(None, snapshot['profile'], as_of=snapshot['as_of'])
            builder.build_custody_projection = lambda: custody
            with capture_build(builder) as (record, _elapsed):
                state = builder.build()
            record.pop('_custody', None)
            record['expected'] = {'state': canonical(to_jsonable(state, redact_config=False))}
            path = args.output / f'{name}.json'
            path.write_text(json.dumps(record, ensure_ascii=True, allow_nan=True))
            result = subprocess.run([str(args.probe)], input=str(path) + '\n', capture_output=True,
                                    text=True, check=True)
            actual = json.loads(result.stdout)
            assert actual.get('state') == record['expected']['state'], f'{name}: render parity failed'
            response = record['response']
            original_reader = ledger.read_ledger
            with subprocess.Popen([str(args.probe), '--timing-only'], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, text=True, bufsize=1) as child:
                # Keep five trials even if the shared-host load rises only
                # after this book starts. Record the actual load on every run.
                trials = 5
                for trial in range(trials):
                    load = os.getloadavg()[0]
                    with patch.object(ledger.KassiberTaxEngine, 'build_ledger_state',
                                      lambda self, inputs: original_reader(self.profile, inputs, response)):
                        start = time.perf_counter()
                        cpu_start = time.process_time()
                        python_state = builder.build()
                        cpu_elapsed = time.process_time() - cpu_start
                        elapsed = time.perf_counter() - start
                    # Match Rust's span: release the completed state outside the
                    # build timer, rather than freeing the previous trial while
                    # assigning this trial's result.
                    del python_state
                    row = dict(book=name, trial=trial + 1, implementation='python',
                               seconds=elapsed, cpu_seconds=cpu_elapsed, load_1m=load)
                    timings.append(row)
                    print(json.dumps(row), flush=True)
                    load = os.getloadavg()[0]
                    child.stdin.write(str(path) + '\n')
                    child.stdin.flush()
                    actual = json.loads(child.stdout.readline())
                    assert 'error' not in actual, actual.get('error')
                    row = dict(book=name, trial=trial + 1, implementation='rust',
                               seconds=actual['seconds'], cpu_seconds=actual['cpu_seconds'], load_1m=load)
                    timings.append(row)
                    print(json.dumps(row), flush=True)
                child.stdin.close()
                assert child.wait() == 0
            del state, custody, snapshot, record, actual, result
            (args.output / 'timings.json').write_text(json.dumps(timings, indent=2) + '\n')


if __name__ == '__main__':
    main()
