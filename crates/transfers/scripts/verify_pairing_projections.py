#!/usr/bin/env python3
"""Read Rust-authored reviews through unchanged Python desktop/AI projections."""
import json
from pathlib import Path
import sqlite3
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from bench_pairing import copy, python_run, rust_run
from kassiber.db import open_db
from kassiber.daemon import _ui_swap_matching_payload_from_conn
from export_pairing_services import dump


def main():
    target = ROOT / 'target/r4-pairing/projections'
    target.mkdir(parents=True, exist_ok=True)
    fixture = json.loads((ROOT / 'crates/transfers/tests/pairing_services.json').read_text())
    binary = ROOT / 'target/release/examples/pairing-bench'
    results = []
    with tempfile.TemporaryDirectory(prefix='pairing-projections-') as root:
        template = open_db(root)
        for scenario in fixture['scenarios']:
            if not scenario['name'].startswith('1-'):
                continue
            baseline = target / 'baseline.db'
            c = sqlite3.connect(baseline)
            template.backup(c)
            c.execute('PRAGMA foreign_keys=OFF')
            tables = [r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")]
            for table in tables:
                c.execute('DELETE FROM "' + table.replace('"', '""') + '"')
            c.commit()
            for statement in scenario['sql']:
                c.executescript(statement + ';')
            c.commit()
            c.close()
            paths = {kind: target / f'{kind}.db' for kind in ('python', 'rust')}
            for path in paths.values():
                copy(baseline, path)
            py, _ = python_run(paths['python'], 'auto', 'w', 'p')
            rs, _ = rust_run(binary, paths['rust'], 'auto', 'w', 'p')
            assert py == rs, scenario['name']
            projections = {}
            databases = {}
            for implementation, path in paths.items():
                c = sqlite3.connect(path)
                c.row_factory = sqlite3.Row
                databases[implementation] = dump(c)
                projections[implementation] = {
                    kind: _ui_swap_matching_payload_from_conn(c, kind, dict(workspace='w', profile='p'))
                    for kind in ('ui.transfers.list', 'ui.transfers.suggest', 'ui.transfers.review_context', 'ui.transfers.rules.list')
                }
                c.close()
            assert json.dumps(databases['python'],sort_keys=True) == json.dumps(databases['rust'],sort_keys=True), scenario['name']
            assert json.dumps(projections['python']) == json.dumps(projections['rust']), scenario['name']
            results.append(dict(book=scenario['name'], applied=json.loads(rs)['applied'], projections=4, byte_parity=True))
            for path in (*paths.values(), baseline):
                path.unlink()
        template.close()
    assert len(results) == 11
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
