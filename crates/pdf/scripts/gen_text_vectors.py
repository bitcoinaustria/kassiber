#!/usr/bin/env python3
"""Freeze Python's text-PDF wrapping, table and document vectors."""
import json
from pathlib import Path
import random
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.pdf_report import format_table, wrap_report_lines, write_text_pdf

OUT = ROOT / 'crates/pdf/tests/data/text_vectors.json'


def lines_cases():
    rng = random.Random(21)
    alphabet = ['a', 'b', ' ', '  ', '\t', '\n', '\r', '\x0b', '\x0c', '-', '€', '日', '😀', '→', 'ä', '\xa0', '\x85', '(', ')', '\\']
    cases = [
        [''], [' '], ['   '], ['\t'], ['x' * 138], ['x' * 139], ['x' * 300], [' ' * 140 + 'y'],
        ['word ' * 40], [('w' * 137) + ' ' + ('z' * 10)], [('w' * 136) + '  ' + 'z'], ['a\tb\tc' * 30],
        ['€' * 50, '—–‑⚠…‘’“”‚„≈•'], ['line\nbreak ' * 30],
    ]
    for _ in range(200):
        cases.append([''.join(rng.choice(alphabet) * rng.randint(1, 60 if rng.random() < 0.1 else 3)
                              for _ in range(rng.randint(0, 50))) for _ in range(rng.randint(1, 2))])
    return cases


def table_cases():
    return [
        dict(headers=['A', 'Bee', 'C'], rows=[['1', 'long value that is truncated', '€3'], ['', '', '']],
             widths=[3, 6, 4], right=[2]),
        dict(headers=['X', 'Y'], rows=[['abc', 'de', 'extra']], widths=[2, 3], right=[0, 1]),
        dict(headers=['Narrow'], rows=[['abcdef'], ['ab']], widths=[3], right=[]),
        dict(headers=['W'], rows=[['abcdef']], widths=[2], right=[0]),
        dict(headers=['Trailing', 'Blank'], rows=[['x\xa0', ' \x85'], ['y', '\t']], widths=[8, 5], right=[]),
    ]


def documents():
    return [('Empty', []), ('Exact (page) \\', ['l%d' % i for i in range(46)]),
            ('Next page €', ['l%d' % i for i in range(47)]), ('Many', ['x' * 200] * 60)]


def build():
    result = dict(
        wrap=[dict(lines=lines, wrapped=wrap_report_lines(lines)) for lines in lines_cases()],
        tables=[dict(case, lines=format_table(case['headers'], case['rows'], case['widths'], set(case['right'])))
                for case in table_cases()],
        documents=[],
    )
    with tempfile.TemporaryDirectory() as root:
        for title, lines in documents():
            path = Path(root) / 'out.pdf'
            written = write_text_pdf(path, title, lines)
            result['documents'].append(dict(title=title, lines=lines, pages=written['pages'],
                                            pdf=path.read_bytes().decode('latin-1')))
    return json.dumps(result, ensure_ascii=True, separators=(',', ':')) + '\n'


if __name__ == '__main__':
    content = build()
    if '--check' in sys.argv:
        assert OUT.read_text() == content, 'text PDF vectors drifted'
    else:
        OUT.write_text(content)
