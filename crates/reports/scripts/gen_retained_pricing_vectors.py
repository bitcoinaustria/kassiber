#!/usr/bin/env python3
"""Freeze round 8's retained lot pricing check (7c7fcc0a8) for Rust.

A stored tax calculation whose fragments are not mappings, or whose lot is
present but not a mapping, or whose lot pricing is present but not a mapping,
is an `engine_detail_mismatch`, never an internal error. The check runs on the
explained result's own calculation and on every inherited source calculation.

This is a separate file from `gain_reads.json`: regenerating that one from the
merged Python would also pull in journal calculation semantics 7 and other
main-sync drift that later slices own. Books stay under target/.

    mise exec rust@1.99.1 uv@latest -- uv run --locked python \\
        crates/reports/scripts/gen_retained_pricing_vectors.py [--check]
"""
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.db import open_db  # noqa: E402
from crates.reports.scripts.gen_gain_vectors import BASE, REF, calculation, outcome, stored  # noqa: E402

OUT = ROOT / 'crates/reports/tests/data/retained_pricing.json'
LOTS = [
    ('null-pricing', lambda lot: lot.__setitem__('pricing', None)),
    ('number-pricing', lambda lot: lot.__setitem__('pricing', 1)),
    ('string-pricing', lambda lot: lot.__setitem__('pricing', 'manual')),
    ('list-pricing', lambda lot: lot.__setitem__('pricing', [])),
    ('missing-pricing', lambda lot: lot.pop('pricing')),
    ('empty-pricing', lambda lot: lot.__setitem__('pricing', {})),
]
FRAGMENTS = [
    ('number-lot', lambda f: f.__setitem__('lot', 1)),
    ('zero-lot', lambda f: f.__setitem__('lot', 0)),
    ('false-lot', lambda f: f.__setitem__('lot', False)),
    ('empty-string-lot', lambda f: f.__setitem__('lot', '')),
    ('string-lot', lambda f: f.__setitem__('lot', 'in')),
    ('list-lot', lambda f: f.__setitem__('lot', [])),
    ('empty-dict-lot', lambda f: f.__setitem__('lot', {})),
    ('null-lot', lambda f: f.__setitem__('lot', None)),
    ('missing-lot', lambda f: f.pop('lot')),
]


def carrying(calc, target='earn'):
    lot = calc['fragments'][0]['lot']
    lot['pricing']['pricing_method'] = 'carrying_value'
    lot['transaction_id'] = target
    lot['asset'] = 'LBTC'
    return calc


def cases():
    for fragment in (0, 1):
        for label, change in LOTS:
            calc = calculation()
            change(calc['fragments'][fragment]['lot'])
            yield f'{label}-{fragment}', stored(calc, compact=False)
        for label, change in FRAGMENTS:
            calc = calculation()
            change(calc['fragments'][fragment])
            yield f'{label}-{fragment}', stored(calc, compact=False)
    for label, value in [('null-fragment', None), ('number-fragment', 1), ('list-fragment', [])]:
        calc = calculation()
        calc['fragments'][1] = value
        yield label, stored(calc, compact=False)
    calc = calculation()
    calc['fragments'] = {}
    yield 'mapping-fragments', stored(calc, compact=False)
    # An inherited source calculation with a malformed lot reports mismatch.
    source_link = "UPDATE journal_entries SET transaction_id='out',asset='BTC',quantity=-100000001 WHERE id='a';"
    for label, change in LOTS[:4] + [(name, None) for name, _ in FRAGMENTS[:6]]:
        inherited = calculation()
        if change is None:
            dict(FRAGMENTS)[label](inherited['fragments'][0])
        else:
            change(inherited['fragments'][0]['lot'])
        yield 'inherited-' + label, stored(carrying(calculation())) + stored(inherited, compact=False, entry='a') + source_link
    yield 'inherited-valid', stored(carrying(calculation())) + stored(entry='a') + source_link


def build(scratch):
    result = []
    for name, sql in cases():
        args = dict(reference=json.dumps(REF))
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            conn = open_db(Path(directory) / 'data')
            conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + BASE + sql)
            conn.commit()
            expected = outcome(conn, 'reports.explain-capital-gain', args)
            conn.close()
        result.append(dict(name=name, base=True, command='reports.explain-capital-gain', sql=BASE + sql,
                           args_json=json.dumps(args), expected=expected,
                           expected_json=json.dumps(expected, sort_keys=True, separators=(',', ':'))))
    return result


if __name__ == '__main__':
    scratch = ROOT / 'target/r5-reports15/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    content = '[\n' + ',\n'.join(json.dumps(case, ensure_ascii=True, separators=(',', ':')) for case in build(scratch)) + '\n]\n'
    if '--check' in sys.argv:
        assert OUT.read_text() == content, 'retained pricing vectors drifted'
    else:
        OUT.write_text(content)
        print(f'{content.count(chr(10)) - 2} retained pricing cases')
