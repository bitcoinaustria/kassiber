#!/usr/bin/env python3
import argparse
from collections import Counter
from copy import deepcopy
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
CORPORA = [ROOT / 'crates/journals/tests/data' / name for name in (
    'render.json', 'render-legacy-floats.json', 'render-response-refusals.json',
    'render-scalar-coercions.json', 'render-branches.json',
    'render-native-builder.json', 'render-native-graph.json',
    'render-nonfinite.json',
)]


def cases(document):
    if isinstance(document, list):
        return document
    result = []
    for case in document['cases']:
        inputs = deepcopy(document['bases'][case['base']])
        for path, value in case['patches']:
            parent = inputs
            for key in path[:-1]:
                parent = parent[key]
            parent[path[-1]] = value
        result.append(dict(inputs, **case))
    return result


def main():
    parser = argparse.ArgumentParser(description='Replay exact Python outcomes through the lossless native journal probe.')
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--corpus', type=Path, action='append')
    args = parser.parse_args()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=True)
    args.output.chmod(0o700)
    summary = {}
    failures = []
    count = 0
    with subprocess.Popen([str(args.probe.resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          text=True, bufsize=1) as child:
        for corpus in args.corpus or CORPORA:
            totals = Counter()
            for case in cases(json.loads(corpus.read_text())):
                path = args.output / f'current-{count % 2}.json'
                path.touch(mode=0o600, exist_ok=True)
                path.chmod(0o600)
                path.write_text(json.dumps(case, ensure_ascii=True, allow_nan=True))
                expected = case['expected']
                for cached in (False, True):
                    child.stdin.write(str(path.resolve()) + '\n')
                    child.stdin.flush()
                    line = child.stdout.readline()
                    if not line:
                        raise RuntimeError(f'probe exited before {corpus.name}:{case["name"]}')
                    actual = json.loads(line)
                    field = 'state' if 'state' in expected else 'error'
                    matched = (actual.get(field) == expected[field]
                               and actual.get('entry_ids_consumed') == len(case['entry_ids']))
                    totals['cached_replies' if cached else 'decoded_replies'] += 1
                    totals['matched' if matched else 'mismatched'] += 1
                    totals['refusals' if field == 'error' else 'states'] += 1
                    if not matched:
                        failures.append(dict(corpus=corpus.name, name=case['name'], cached=cached,
                                             expected=expected, actual=actual,
                                             entry_ids_consumed=len(case['entry_ids'])))
                count += 1
            summary[corpus.name] = dict(totals)
            print(corpus.name, dict(totals), flush=True)
        child.stdin.close()
        if child.wait() != 0:
            raise RuntimeError('probe did not exit successfully')
    for path in args.output.glob('current-*.json'):
        path.unlink()
    record = dict(cases=count, corpora=summary, failures=failures)
    (args.output / 'summary.json').write_text(json.dumps(record, ensure_ascii=True, indent=2) + '\n')
    return int(bool(failures) or not count)


if __name__ == '__main__':
    raise SystemExit(main())
