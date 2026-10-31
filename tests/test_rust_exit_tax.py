from dataclasses import replace
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
from unittest.mock import patch

import pytest

from kassiber.cli.handlers import _report_hooks
from kassiber.core import exit_tax, reports
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.rust_port.xlsx import RecordingWorkbook, canonical, first_difference, STAMP
from tests.test_rust_report_exports import _dump
from tests.test_rust_xlsx_exports import _wire
from tests.test_rust_austrian_xlsx import _clocked_book
from crates.reports.scripts.gen_exit_tax_vectors import FixedDateTime, BASE

ROOT = Path(__file__).resolve().parents[1]
SERVICE = os.environ.get('KASSIBER_XLSX_SERVICE_BIN')
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')
CASES = json.loads((ROOT / 'crates/reports/tests/data/exit_tax.json').read_text())


def _state(root):
    return json.dumps(_dump(root, {}), sort_keys=True, separators=(',', ':')).encode()


@pytest.mark.skipif(not SERVICE, reason='requires native XLSX service probe')
@pytest.mark.parametrize('case', CASES, ids=lambda c: c['name'])
@pytest.mark.parametrize('workbook', [False, True], ids=['read', 'xlsx'])
def test_exit_service_content_errors_and_all_database_tables(tmp_path, case, workbook):
    root = tmp_path / 'data'
    conn = open_db(root)
    conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + case['sql'])
    conn.commit()
    database = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    conn.close()
    before = database.read_bytes()
    conn = sqlite3.connect(database)
    conn.row_factory = sqlite3.Row
    target = tmp_path / 'exit.xlsx'
    args = json.loads(case['args_json'])
    try:
        with patch('xlsxwriter.Workbook', RecordingWorkbook), patch.object(exit_tax, 'datetime', FixedDateTime):
            params = dict(departure_date=args.get('departure_date'), destination=args.get('destination'))
            hooks = replace(_report_hooks(), now_iso=lambda: STAMP)
            expected = dict(ok=reports.export_exit_tax_xlsx_report(conn, args.get('workspace'), args.get('profile'), target, hooks, **params)
                            if workbook else reports.report_exit_tax(conn, args.get('workspace'), args.get('profile'), hooks, **params))
    except AppError as e:
        expected = dict(error=dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable))
    except (ValueError, TypeError, OverflowError, AttributeError) as e:
        expected = dict(error=dict(code='internal_error', message=str(e), hint=None, details=None, retryable=False))
    conn.close()
    if workbook and 'ok' in expected:
        assert 'report_snapshot' not in expected['ok']
        assert expected['ok']['bytes'] == target.stat().st_size
        content = canonical(target)
    expected_state = _state(root)
    expected_wire = _wire(json.dumps(expected).encode()) if workbook else json.dumps(expected, sort_keys=True).encode()
    database.write_bytes(before)
    target.unlink(missing_ok=True)
    request = dict(database=str(database), command='reports.export-exit-tax-xlsx' if workbook else 'reports.exit-tax',
                   args_json=case['args_json'], path=str(target))
    result = subprocess.run([SERVICE], input=json.dumps(request) + '\n', capture_output=True, text=True, timeout=180, check=True)
    assert not result.stderr
    actual = json.loads(result.stdout)
    if workbook and 'ok' in expected:
        assert list(actual['ok']) == list(expected['ok'])
        assert 'report_snapshot' not in actual['ok']
        assert actual['ok']['bytes'] == target.stat().st_size
        assert not (difference := first_difference(content, canonical(target))), difference
    else:
        assert not target.exists()
    actual_wire = _wire(json.dumps(actual).encode()) if workbook else json.dumps(actual, sort_keys=True).encode()
    assert actual_wire == expected_wire
    assert _state(root) == expected_state


def _book(tmp_path, monkeypatch, country='at', variant='ready'):
    root, database, env, call = _clocked_book(tmp_path, monkeypatch, country, variant)
    from tests import cli_runner
    original = cli_runner.prefix
    def prefix(*, reference=False, python_options=()):
        if reference:
            command = list(original(reference=True, python_options=python_options))
            command[2] = command[2].replace('reports.datetime = FixedDateTime',
                'from kassiber.core import exit_tax\nexit_tax.datetime = FixedDateTime\nreports.datetime = FixedDateTime')
            return command
        return original(python_options=python_options)
    monkeypatch.setattr(cli_runner, 'prefix', prefix)
    conn = sqlite3.connect(database)
    conn.executescript(BASE if country == 'at' else BASE.replace("tax_country='at',gains_algorithm='MOVING_AVERAGE_AT'", "tax_country='generic',gains_algorithm='HIFO'"))
    conn.close()
    env['PYTHONWARNINGS'] = "ignore:Can't merge single cell:UserWarning:xlsxwriter.worksheet"
    return root, database, env, call


@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('country,variant', [('at', 'ready'), ('at', 'raw-hex'), ('at', 'stale'), ('generic', 'ready')])
def test_exit_cli_wire_workbooks_database_and_zero_fallbacks(tmp_path, monkeypatch, country, variant):
    root, database, env, call = _book(tmp_path, monkeypatch, country, variant)
    before = database.read_bytes()
    target = tmp_path / 'exit.xlsx'
    def sequence(reference):
        database.write_bytes(before)
        target.unlink(missing_ok=True)
        results = []
        for flags in [[], ['--departure-date', '2024-12-31'], ['--departure-date', '2025-01-01T00:30:00+02:00', '--destination', 'third_country'], ['--departure-date', 'bad']]:
            for output in [[], ['--format', 'json'], ['--format', 'csv'], ['--format', 'plain'], ['--machine']]:
                result = call(*output, 'reports', 'exit-tax', *flags, reference=reference)
                results.append((result.returncode, result.stdout, result.stderr, _state(root)))
            result = call('--machine', 'reports', 'export-exit-tax-xlsx', '--file', str(target), *flags, reference=reference)
            payload = json.loads(result.stdout)
            content = canonical(target) if payload['kind'] != 'error' else None
            if payload['kind'] != 'error':
                assert 'report_snapshot' not in payload['data']
                assert payload['data']['bytes'] == target.stat().st_size
            results.append((result.returncode, _wire(result.stdout), result.stderr, content, _state(root)))
        return results
    assert sequence(False) == sequence(True)
    served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served and all(c['served_by'] == 'rust' for c in served)
    assert {c['command'] for c in served} == {'reports.exit-tax', 'reports.export-exit-tax-xlsx'}


@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('blocked', [False, True])
def test_exit_daemon_wire_workbooks_database_and_zero_fallbacks(tmp_path, monkeypatch, blocked):
    root, database, env, call = _book(tmp_path, monkeypatch)
    if blocked:
        conn = sqlite3.connect(database)
        conn.executescript("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','a','wallet','Cold','failed',1,'fixed');")
        conn.close()
    before = database.read_bytes()
    exports = tmp_path / 'exports'
    requests = [{}, {}, {'departure_date': '2025-01-01', 'destination': 'third_country'}, {'destination': 'EU'},
        {'departure_date': 'bad'}, {'destination': 'bad'}, {'departure_date': False}, {'destination': 1},
        {'workspace': 'absent', 'profile': 'absent', 'year': 2024}, {'departure_date': []}, {'destination': {}},
        {'departure_date': '2024-12-31', 'destination': 'non-eu'}, None, [], 1]
    wire = b''.join((json.dumps(dict(request_id=f'{i}-{kind}', kind=kind, args=args)) + '\n').encode()
        for i, args in enumerate(requests) for kind in ['ui.reports.exit_tax_preview', 'ui.reports.export_exit_tax_xlsx'])
    wire += b'{"request_id":"done","kind":"daemon.shutdown"}\n'
    def session(reference):
        shutil.rmtree(exports, ignore_errors=True)
        database.write_bytes(before)
        result = call('daemon', reference=reference, wire=wire)
        assert (result.returncode, result.stderr) == (0, b''), result.stderr
        records = result.stdout.splitlines()[1:]
        for raw in records:
            record = json.loads(raw)
            if record['kind'] != 'error' and 'file' in record.get('data', {}):
                assert 'report_snapshot' not in record['data']
                assert record['data']['bytes'] == Path(record['data']['file']).stat().st_size
        return ([_wire(raw) for raw in records], _state(root),
            {str(p.relative_to(exports)): canonical(p) for p in exports.rglob('*.xlsx')})
    assert session(False) == session(True)
    served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served == [{'command': 'daemon', 'served_by': 'rust'}]
