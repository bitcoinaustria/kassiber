#![cfg(feature = "test-harness")]

use std::path::Path;
use std::process::Command;

#[test]
fn privacy_hygiene_cli_daemon_bytes_all_tables_and_zero_fallback() {
    reference(REFERENCE);
}

#[test]
fn privacy_hygiene_frozen_oracle_matches_unchanged_python() {
    reference(ORACLE);
}

#[test]
fn privacy_hygiene_corrupt_daemon_markers_preserve_native_debug_contract() {
    reference(NATIVE_DEBUG);
}

fn reference(script: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let python = std::env::var_os("KASSIBER_REFERENCE_PYTHON")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join(".venv/bin/python"));
    let scratch = tempfile::tempdir().unwrap();
    let output = Command::new(python)
        .current_dir(&root)
        .args(["-c", script])
        .env("KASSIBER_RUST_BIN", env!("CARGO_BIN_EXE_kassiber"))
        .env("KASSIBER_PRIVACY_SCRATCH", scratch.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}

const REFERENCE: &str = r#"
import json, os, sqlite3
from pathlib import Path
import pytest
from tests.test_rust_report_exports import _book, _dump
from tests.rust_port.book_schema import TABLES
from kassiber.db import open_db

root = Path(os.environ['KASSIBER_PRIVACY_SCRATCH'])
cases = json.loads(Path('crates/reports/tests/data/privacy_hygiene.json').read_text())
selected = ['baseline', 'raw-boundary-quarantine', 'disabled-provider', 'wallet-descriptor', 'no-active-transactions', 'malformed-journal_input_version']
totals = {'reports.privacy-hygiene': 0, 'daemon': 0}
for name in selected:
    base = root / name
    base.mkdir()
    with pytest.MonkeyPatch.context() as monkey:
        data, database, env, call = _book(base, monkey, 'generic', 'FIFO')
        assert call('--machine', 'status', reference=True).returncode == 0
        conn = open_db(data)
        case = next(c for c in cases if c['name'] == name)
        conn.executescript(case['sql'])
        conn.commit()
        conn.close()
        conn = open_db(data)
        conn.close()
        before = json.dumps(_dump(data, {}), sort_keys=True, separators=(',', ':')).encode()
        assert tuple(sorted(_dump(data, {}))) == TABLES
        for flags in [('--format', 'json'), ('--format', 'table'), ('--format', 'plain'), ('--format', 'csv'), ('--machine',)]:
            args = (*flags, 'reports', 'privacy-hygiene')
            expected = call(*args, reference=True)
            actual = call(*args)
            assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr), (name, flags, actual.stdout, expected.stdout, actual.stderr, expected.stderr)
            observed = _dump(data, {})
            assert json.dumps(observed, sort_keys=True, separators=(',', ':')).encode() == before, (name, flags, {k:(json.loads(before)[k],v) for k,v in observed.items() if json.loads(before)[k] != v})
            assert b'sentinel-' not in actual.stdout
            totals['reports.privacy-hygiene'] += 1
        for scope in [('other',), ('missing',)]:
            args = ('--machine', 'reports', 'privacy-hygiene', '--profile', *scope)
            expected = call(*args, reference=True)
            actual = call(*args)
            assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr), (name, scope, actual.stdout, expected.stdout)
            assert json.dumps(_dump(data, {}), sort_keys=True, separators=(',', ':')).encode() == before
            totals['reports.privacy-hygiene'] += 1
        output = base / 'report.json'
        args = ('--machine', '--output', str(output), 'reports', 'privacy-hygiene')
        expected = call(*args, reference=True)
        saved = output.read_bytes()
        actual = call(*args)
        assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr)
        assert output.read_bytes() == saved
        assert json.dumps(_dump(data, {}), sort_keys=True, separators=(',', ':')).encode() == before
        totals['reports.privacy-hygiene'] += 1
        if name.startswith('malformed-'):
            log = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
            assert len(log) == 8 and all(r['served_by'] == 'rust' for r in log), log
            continue
        requests = [dict(request_id=i, kind='ui.reports.privacy_hygiene', args=value) for i, value in enumerate([None, {}, {'wallet':'cold'}, {'z':1,'a':2}, [], '', True], 1)]
        requests.append(dict(request_id=8, kind='ui.reports.privacy_hygiene', expected_scope={'profile_id':'wrong'}))
        wire = b''.join((json.dumps(r)+'\n').encode() for r in requests)
        expected = call('daemon', reference=True, wire=wire)
        actual = call('daemon', wire=wire)
        assert actual.returncode == expected.returncode == 0, (actual.stderr, expected.stderr)
        assert actual.stderr == expected.stderr, (name, actual.stderr, expected.stderr)
        assert actual.stdout.splitlines(keepends=True)[1:] == expected.stdout.splitlines(keepends=True)[1:], (name, actual.stdout, expected.stdout)
        assert json.dumps(_dump(data, {}), sort_keys=True, separators=(',', ':')).encode() == before
        totals['daemon'] += 1
        log = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
        assert log and all(r['served_by'] == 'rust' for r in log), log
        assert sum(r['command'] == 'reports.privacy-hygiene' for r in log) == 8
        assert sum(r['command'] == 'daemon' for r in log) == 1
print(json.dumps({'rust': totals, 'python_fallbacks': 0, 'daemon_requests': totals['daemon']*8, 'tables_per_comparison': len(TABLES)}, sort_keys=True))
"#;

const ORACLE: &str = r#"
import hashlib, json, os, sqlite3
from pathlib import Path
from kassiber.core import reports
from kassiber.cli.handlers import _report_hooks
from kassiber.errors import AppError

root = Path(os.environ['KASSIBER_PRIVACY_SCRATCH'])
conn = sqlite3.connect(root / 'oracle.sqlite')
conn.row_factory = sqlite3.Row
conn.execute('PRAGMA foreign_keys=ON')
conn.execute('PRAGMA synchronous=OFF')
conn.execute('PRAGMA journal_mode=MEMORY')
conn.executescript(Path('crates/reports/tests/data/privacy_hygiene_seed.sql').read_text())
base = Path('crates/reports/tests/data/book.sql').read_text()
cases = json.loads(Path('crates/reports/tests/data/privacy_hygiene.json').read_text())
wire = lambda v: json.dumps(v, sort_keys=True, separators=(',', ':'))
def state():
    tables = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    assert len(tables) == 112
    return {t: hashlib.sha256(wire(sorted([dict(r) for r in conn.execute('SELECT * FROM "'+t+'"')], key=wire)).encode()).hexdigest() for t in tables}
for case in cases:
    conn.executescript('SAVEPOINT fixture;' + base + case['sql'])
    before = state()
    assert before == case['table_sha256'], case['name']
    args = json.loads(case['args_json'])
    try:
        result = {'ok': reports.report_privacy_hygiene(conn, args.get('workspace'), args.get('profile'), _report_hooks())}
    except AppError as e:
        result = {'error': dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable)}
    except (ValueError, TypeError, OverflowError) as e:
        result = {'error': dict(code='internal_error', message=str(e), hint=None, details=None, retryable=False)}
    assert wire(result) == case['expected_json'], case['name']
    assert state() == before, case['name']
    conn.execute('ROLLBACK TO fixture')
    conn.execute('RELEASE fixture')
conn.close()
print(json.dumps({'unchanged_python_cases': len(cases), 'tables_per_comparison': 112}, sort_keys=True))
"#;

const NATIVE_DEBUG: &str = r#"
import json, os
from pathlib import Path
import pytest
from tests.test_rust_report_exports import _book, _dump
from tests.rust_port.book_schema import TABLES
from kassiber.db import open_db

root = Path(os.environ['KASSIBER_PRIVACY_SCRATCH'])
sessions = 0
for field in ('journal_input_version', 'last_processed_input_version', 'last_processed_tx_count'):
    for flags in ((), ('--debug',)):
        base = root / (field + ('-debug' if flags else '-ordinary'))
        base.mkdir()
        with pytest.MonkeyPatch.context() as monkey:
            data, _, env, call = _book(base, monkey, 'generic', 'FIFO')
            assert call('--machine', 'status', reference=True).returncode == 0
            conn = open_db(data)
            conn.execute(f"UPDATE profiles SET {field}='bad' WHERE id='p'")
            conn.commit()
            conn.close()
            before = _dump(data, {})
            assert tuple(sorted(before)) == TABLES
            wire = b'{"request_id":1,"kind":"ui.reports.privacy_hygiene","args":{"unknown":true}}\n{"request_id":2,"kind":"ui.reports.privacy_hygiene"}\n{"request_id":3,"kind":"ui.reports.privacy_hygiene","args":false}\n'
            expected = call(*flags, 'daemon', reference=True, wire=wire)
            actual = call(*flags, 'daemon', wire=wire)
            assert actual.returncode == expected.returncode == 0
            expected_lines = expected.stdout.splitlines(keepends=True)
            actual_lines = actual.stdout.splitlines(keepends=True)
            assert len(expected_lines) == len(actual_lines) == 4
            for index in (1, 3):
                assert actual_lines[index] == expected_lines[index]
                assert json.loads(actual_lines[index])['error']['debug'] is None
            expected_error = json.loads(expected_lines[2])['error']
            actual_error = json.loads(actual_lines[2])['error']
            assert expected_error['code'] == actual_error['code'] == 'internal_error'
            assert expected_error['message'] == actual_error['message'] == "invalid literal for int() with base 10: 'bad'"
            assert isinstance(expected_error['debug'], str) and 'ValueError' in expected_error['debug']
            assert actual_error['debug'] == ''
            debug_bytes = json.dumps(expected_error['debug'], ensure_ascii=False).encode()
            assert expected_lines[2].count(debug_bytes) == 1
            assert actual_lines[2] == expected_lines[2].replace(debug_bytes, b'""', 1)
            assert expected.stderr.startswith(b'Traceback (most recent call last):\n')
            assert expected.stderr.endswith(b"ValueError: invalid literal for int() with base 10: 'bad'\n")
            assert actual.stderr == b''
            assert _dump(data, {}) == before
            log = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
            assert len(log) == 1 and log[0]['served_by'] == 'rust', log
            sessions += 1
print(json.dumps({'native_debug_sessions': sessions, 'hygiene_requests': sessions * 3, 'python_fallbacks': 0, 'tables_per_comparison': len(TABLES)}, sort_keys=True))
"#;
