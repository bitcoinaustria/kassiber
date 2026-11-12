#!/usr/bin/env python3
"""Refreeze configuration privacy hygiene results from unchanged Python.

Case SQL and arguments are retained; table hashes and results are recomputed
on the frozen seed schema exactly as the native test and the Python oracle in
crates/kassiber/tests/privacy_reports.rs read them. Pass --write to update.
"""
import argparse
import hashlib
import json
import sqlite3
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports
from kassiber.errors import AppError

DATA = ROOT / 'crates/reports/tests/data'
DESTINATION = DATA / 'privacy_hygiene.json'


def wire(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'))


def tables(conn):
    return [r[0] for r in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]


def state(conn, seed):
    # The frozen seed keeps the layout it was cut from.
    tables_now = tables(conn)
    assert tables_now == seed
    return {t: hashlib.sha256(wire(sorted([dict(r) for r in conn.execute('SELECT * FROM "' + t + '"')],
                                          key=wire)).encode()).hexdigest() for t in tables_now}


def build():
    cases = json.loads(DESTINATION.read_text())
    scratch = ROOT / 'target/privacy-hygiene-vectors'
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch) as temporary:
        conn = sqlite3.connect(Path(temporary) / 'oracle.sqlite')
        conn.row_factory = sqlite3.Row
        conn.execute('PRAGMA foreign_keys=ON')
        conn.executescript((DATA / 'privacy_hygiene_seed.sql').read_text())
        seed = tables(conn)
        base = (DATA / 'book.sql').read_text()
        for case in cases:
            conn.executescript('SAVEPOINT fixture;' + base + case['sql'])
            before = state(conn, seed)
            args = json.loads(case['args_json'])
            try:
                result = {'ok': reports.report_privacy_hygiene(
                    conn, args.get('workspace'), args.get('profile'), _report_hooks())}
            except AppError as e:
                result = {'error': dict(code=e.code, message=str(e), hint=e.hint, details=e.details,
                                        retryable=e.retryable)}
            except (ValueError, TypeError, OverflowError) as e:
                result = {'error': dict(code='internal_error', message=str(e), hint=None, details=None,
                                        retryable=False)}
            assert state(conn, seed) == before, case['name']
            case['table_sha256'] = before
            case['expected_json'] = wire(result)
            conn.execute('ROLLBACK TO fixture')
            conn.execute('RELEASE fixture')
        conn.close()
    return json.dumps(cases, indent=2) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    rendered = build()
    if parser.parse_args().write:
        DESTINATION.write_text(rendered)
    else:
        assert DESTINATION.read_text() == rendered, 'privacy hygiene vectors drifted'


if __name__ == '__main__':
    main()
