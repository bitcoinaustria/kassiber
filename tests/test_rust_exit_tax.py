from dataclasses import replace
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
from unittest.mock import patch

import pytest

from kassiber.cli.handlers import _report_hooks
from kassiber.core import exit_tax, reports
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.rust_port.xlsx import RecordingWorkbook, design_content, design_wire, STAMP
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
        content = design_content(target, False)
    expected_state = _state(root)
    expected_wire = (_wire(design_wire(json.dumps(expected).encode(), 'sheets')) if workbook
                     else json.dumps(expected, sort_keys=True).encode())
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
        assert design_content(target, True) == content
    else:
        assert not target.exists()
    actual_wire = (_wire(design_wire(json.dumps(actual).encode(), 'sheets')) if workbook
                   else json.dumps(actual, sort_keys=True).encode())
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
        for flags in [[], ['--departure-date', '2024-12-31'], ['--departure-date', '2025-01-01T00:30:00+02:00', '--destination', 'third_country'], ['--departure-date', 'bad'], ['--departure-date', '2025-01-01T12:00:00']]:
            for output in [[], ['--format', 'json'], ['--format', 'csv'], ['--format', 'plain'], ['--machine']]:
                result = call(*output, 'reports', 'exit-tax', *flags, reference=reference)
                results.append((result.returncode, result.stdout, result.stderr, _state(root)))
            result = call('--machine', 'reports', 'export-exit-tax-xlsx', '--file', str(target), *flags, reference=reference)
            payload = json.loads(result.stdout)
            content = design_content(target, not reference) if payload['kind'] != 'error' else None
            if payload['kind'] != 'error':
                assert 'report_snapshot' not in payload['data']
                assert payload['data']['bytes'] == target.stat().st_size
            results.append((result.returncode, _wire(design_wire(result.stdout, 'sheets')), result.stderr, content,
                            _state(root)))
        return results
    assert sequence(False) == sequence(True)
    served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served and all(c['served_by'] == 'rust' for c in served)
    assert {c['command'] for c in served} == {'reports.exit-tax', 'reports.export-exit-tax-xlsx'}


@pytest.mark.skipif(not RUST_BIN, reason='requires Rust test-harness binary')
@pytest.mark.parametrize('country', ['at', 'generic'])
@pytest.mark.parametrize('blocked', [False, True])
def test_exit_daemon_wire_workbooks_database_and_zero_fallbacks(tmp_path, monkeypatch, blocked, country):
    root, database, env, call = _book(tmp_path, monkeypatch, country)
    if blocked:
        conn = sqlite3.connect(database)
        conn.executescript("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','a','wallet','Cold','failed',1,'fixed');")
        conn.close()
    before = database.read_bytes()
    exports = tmp_path / 'exports'
    requests = [{}, {}, {'departure_date': '2025-01-01', 'destination': 'third_country'}, {'destination': 'EU'},
        {'departure_date': 'bad'}, {'departure_date': '2025-01-01T12:00:00'}, {'departure_date': '2025-01-01 12:00:00.123'}, {'destination': 'bad'}, {'departure_date': False}, {'destination': 1},
        {'departure_date': '2024-03-31T23:30:00Z'}, {'departure_date': '20240331'}, {'departure_date': '2024-03-31Z'},
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
                data = record['data']
                assert data['filename'].startswith(f"kassiber-exit-tax-{data['departure_date']}-")
                assert not re.search(r'[<>:"/\\|?*\x00-\x1f]', data['filename'])
        return ([_wire(design_wire(raw, 'sheets')) for raw in records], _state(root),
            {str(p.relative_to(exports)): design_content(p, not reference) for p in exports.rglob('*.xlsx')})
    assert session(False) == session(True)
    served = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert served == [{'command': 'daemon', 'served_by': 'rust'}]


# The reference fixture processes finalized custody inputs through the Rust tax
# engine; the probe then serves the stored report entirely in native Rust.
from tests.test_exit_tax_review import (
    engine_book, METHODS, DISPOSAL_OVERRIDES, REVIEW_TAX, REVIEW_GAIN,
    reviewer_rows,
)


@pytest.mark.skipif(not SERVICE, reason='requires native XLSX service probe')
@pytest.mark.parametrize('country,method', [(c,m) for c in ('at','generic') for m in METHODS
                                          if c == 'at' or m != 'moving_average_at'])
@pytest.mark.parametrize('override', DISPOSAL_OVERRIDES)
@pytest.mark.parametrize('original_times', [False, True], ids=['review-figures','original-vienna-times'])
def test_native_reviewer_figures_and_overrides(engine_book, tmp_path, country, method, override, original_times):
    rows = reviewer_rows()
    if country == 'at' and not original_times:
        rows = [(tid,wallet,{'offset':'2024-03-31T23:30:00+02:00',
                            'fraction':'2024-03-31T23:59:59.500000+02:00'}.get(tid,at),*rest)
                for tid,wallet,at,*rest in rows]
    quote = '2024-03-31T21:59:59.999999Z' if country == 'at' else '2024-03-31T23:59:59.999999Z'
    conn = engine_book(rows,method=method,country=country,pair=True,
                       overrides={'sell':override},quotes=[(quote,65000)])
    expected = reports.report_exit_tax(conn,'W','P',_report_hooks(),departure_date='2024-03-31')
    expected_figure = (REVIEW_TAX if country == 'at' else REVIEW_GAIN)[method]
    if country == 'at' and original_times:
        from decimal import Decimal
        expected_figure = float(Decimal(str(expected_figure)) - Decimal('69.85'))
    assert expected['totals']['estimatedTax' if country == 'at' else 'neuGain'] == expected_figure
    database = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    conn.close()
    request = dict(database=str(database),command='reports.exit-tax',args_json=json.dumps({'departure_date':'2024-03-31'}),path=str(tmp_path/'unused.xlsx'))
    before = _state(database.parent)
    result = subprocess.run([SERVICE],input=json.dumps(request)+'\n',capture_output=True,text=True,timeout=180,check=True)
    assert not result.stderr
    assert json.dumps(json.loads(result.stdout),sort_keys=True).encode() == json.dumps({'ok':expected},sort_keys=True).encode()
    assert _state(database.parent) == before


@pytest.mark.skipif(not SERVICE, reason='requires native XLSX service probe')
@pytest.mark.parametrize('country,method', [(c,m) for c in ('at','generic') for m in METHODS
                                          if c == 'at' or m != 'moving_average_at'])
@pytest.mark.parametrize('override', DISPOSAL_OVERRIDES)
def test_native_reviewer_big_disposal_matrix(engine_book, tmp_path, country, method, override):
    from tests.test_exit_tax_review import BTC
    rows = reviewer_rows() + [('bigsell','Cold','2023-10-01T00:00:00Z','outbound',11*BTC//10,0,26000,'withdrawal')]
    options = dict(method=method,country=country,pair=True,overrides={'bigsell':override},
                   quotes=[('2024-03-31T12:00:00Z',65000)],
                   expected_quarantines=('future-sell',))
    if country == 'at' and method == 'moving_average_at':
        # The review's base_bigma.py independently confirms this pre-existing
        # lot-engine refusal; the mirror must retain it rather than invent lots.
        with pytest.raises(AppError,match='Total in-transaction crypto value < total taxable crypto value'):
            engine_book(rows,**options)
        return
    conn = engine_book(rows,**options)
    expected = reports.report_exit_tax(conn,'W','P',_report_hooks(),departure_date='2024-03-31')
    figures = dict(zip(METHODS, (5228.44,74.25,74.25,3165.94,3114.47))) if country == 'at' else dict(zip(METHODS,(17223.70,24114.10,24114.10,9723.70,19592.12)))
    assert expected['totals']['estimatedTax' if country == 'at' else 'neuGain'] == figures[method]
    database = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    conn.close()
    before = _state(database.parent)
    request = dict(database=str(database),command='reports.exit-tax',args_json=json.dumps({'departure_date':'2024-03-31'}),path=str(tmp_path/'unused.xlsx'))
    result = subprocess.run([SERVICE],input=json.dumps(request)+'\n',capture_output=True,text=True,timeout=180,check=True)
    assert not result.stderr
    assert json.dumps(json.loads(result.stdout),sort_keys=True).encode() == json.dumps({'ok':expected},sort_keys=True).encode()
    assert _state(database.parent) == before



def _native_review_report(conn, tmp_path, departure):
    expected = reports.report_exit_tax(conn, 'W', 'P', _report_hooks(), departure_date=departure)
    database = Path(conn.execute('PRAGMA database_list').fetchone()[2])
    before = _state(database.parent)
    request = dict(database=str(database), command='reports.exit-tax',
                   args_json=json.dumps({'departure_date': departure}), path=str(tmp_path / 'unused.xlsx'))
    result = subprocess.run([SERVICE], input=json.dumps(request) + '\n', capture_output=True,
                            text=True, timeout=180, check=True)
    assert not result.stderr
    assert json.dumps(json.loads(result.stdout), sort_keys=True).encode() == json.dumps({'ok': expected}, sort_keys=True).encode()
    assert _state(database.parent) == before
    return json.loads(result.stdout)['ok']


def _review_quote_cases():
    from datetime import datetime, timedelta
    result = []
    for country, cutoff in [('at', '2024-01-15T23:00:00+00:00'), ('generic', '2024-01-16T00:00:00+00:00')]:
        instant = datetime.fromisoformat(cutoff)
        z = lambda at: at.isoformat().replace('+00:00', 'Z')
        last = z(instant - timedelta(microseconds=1))
        prev = z(instant - timedelta(hours=24))
        older = z(instant - timedelta(hours=24, microseconds=1))
        for label, tx, cache, expected in [
            ('cache-24h', [], [(prev, 100)], (100, 'cache', prev)),
            ('cache-stale', [], [(older, 100)], (None, 'missing', None)),
            ('tx-24h', [(prev, 200)], [], (200, 'transaction', prev)),
            ('tx-stale', [(older, 200)], [], (None, 'missing', None)),
            ('both-cutoff', [(z(instant), 200)], [(z(instant), 100)], (None, 'missing', None)),
            ('tx-last', [(last, 200)], [], (200, 'transaction', last)),
            ('cache-last', [], [(last, 100)], (100, 'cache', last)),
            ('cache-closer', [(prev, 200)], [(last, 100)], (100, 'cache', last)),
            ('tx-closer', [(last, 200)], [(prev, 100)], (200, 'transaction', last)),
            ('equal', [(last, 200)], [(last, 100)], (100, 'cache', last)),
            ('ancient-cache', [(last, 200)], [('2020-01-01T00:00:00Z', 1)], (200, 'transaction', last)),
            ('ancient-tx', [('2020-07-01T00:00:00Z', 200)], [(last, 100)], (100, 'cache', last)),
            ('both-stale', [(older, 200)], [(z(instant - timedelta(hours=30)), 100)], (None, 'missing', None)),
        ]:
            result.append(pytest.param(country, tx, cache, expected, id=country + '-' + label))
    for country, label, tx, cache, expected in [
        ('at', 'cache-offsets', [], [('2024-01-15T22:30:00+01:00', 111), ('2024-01-15T21:45:00Z', 222)], (222, 'cache', '2024-01-15T21:45:00Z')),
        ('at', 'tx-offset', [('2024-01-15T22:50:00+01:00', 333)], [('2024-01-15T21:45:00Z', 222)], (333, 'transaction', '2024-01-15T21:50:00Z')),
        ('at', 'tx-after-vienna', [('2024-01-16T00:10:00+01:00', 333)], [('2024-01-15T21:45:00Z', 222)], (222, 'cache', '2024-01-15T21:45:00Z')),
        ('generic', 'tx-before-utc', [('2024-01-16T00:10:00+01:00', 333)], [('2024-01-15T21:45:00Z', 222)], (333, 'transaction', '2024-01-15T23:10:00Z')),
    ]:
        result.append(pytest.param(country, tx, cache, expected, id=country + '-' + label))
    return result


@pytest.mark.skipif(not SERVICE, reason='requires native XLSX service probe')
@pytest.mark.parametrize('country,tx_quotes,cache,expected', _review_quote_cases())
def test_native_reviewer_quote_boundaries(engine_book, tmp_path, country, tx_quotes, cache, expected):
    from tests.test_exit_tax_review import BTC
    rows = [('alt', 'Cold', '2020-06-01T00:00:00Z', 'inbound', BTC, 0, 8000, 'deposit')]
    rows += [(f'q{i}', 'Cold', at, 'inbound', BTC // 1000, 0, rate, 'deposit')
             for i, (at, rate) in enumerate(tx_quotes)]
    conn = engine_book(rows, country=country, quotes=cache)
    report = _native_review_report(conn, tmp_path, '2024-01-15')
    quote = report['fmvSource'][0]
    assert (quote['rate'], quote['source'], quote['asOf']) == expected


@pytest.mark.skipif(not SERVICE, reason='requires native XLSX service probe')
@pytest.mark.parametrize('country,departure,lots,cache,day,neu,alt,quote', [
    ('at', '2024-03-31', [('2024-03-31T21:59:59.999999Z', 1000), ('2024-03-31T22:00:00Z', 2000)], [('2024-03-30T22:00:00Z', 60000)], '2024-03-31', 1000, 100000000, (50000, 'transaction', '2024-03-31T21:59:59.999999Z')),
    ('generic', '2024-03-31', [('2024-03-31T21:59:59.999999Z', 1000), ('2024-03-31T22:00:00Z', 2000)], [('2024-03-30T22:00:00Z', 60000)], '2024-03-31', 100003000, 0, (50000, 'transaction', '2024-03-31T22:00:00Z')),
    ('at', '2024-10-27', [('2024-10-27T22:59:59.999999Z', 1000), ('2024-10-27T23:00:00Z', 2000)], [('2024-10-26T23:00:00Z', 61000)], '2024-10-27', 1000, 100000000, (50000, 'transaction', '2024-10-27T22:59:59.999999Z')),
    ('at', '2024-10-27', [], [('2024-10-26T22:59:59Z', 61000)], '2024-10-27', 0, 100000000, (None, 'missing', None)),
    ('at', '2024-10-27', [], [('2024-10-26T22:00:00Z', 61000)], '2024-10-27', 0, 100000000, (None, 'missing', None)),
    ('at', '2024-03-31', [], [('2024-03-30T23:00:00Z', 62000)], '2024-03-31', 0, 100000000, (62000, 'cache', '2024-03-30T23:00:00Z')),
    ('at', '2024-03-31', [], [('2024-03-30T21:59:59Z', 62000)], '2024-03-31', 0, 100000000, (None, 'missing', None)),
    ('at', '2024-04-01T00:30:00+02:00', [('2024-04-01T21:59:59Z', 1000), ('2024-04-01T22:00:00Z', 2000)], [('2024-04-01T12:00:00Z', 63000)], '2024-04-01', 1000, 100000000, (50000, 'transaction', '2024-04-01T21:59:59Z')),
    ('generic', '2024-04-01T00:30:00+02:00', [('2024-03-31T23:59:59Z', 1000), ('2024-04-01T00:00:00Z', 2000)], [('2024-03-31T12:00:00Z', 63000)], '2024-03-31', 100001000, 0, (50000, 'transaction', '2024-03-31T23:59:59Z')),
])
def test_native_reviewer_vienna_days(engine_book, tmp_path, country, departure, lots, cache, day, neu, alt, quote):
    from tests.test_exit_tax_review import BTC
    rows = [('alt', 'Cold', '2020-06-01T00:00:00Z', 'inbound', BTC, 0, 8000, 'deposit')]
    rows += [(f'n{i}', 'Cold', at, 'inbound', sats * 1000, 0, 50000, 'deposit')
             for i, (at, sats) in enumerate(lots)]
    conn = engine_book(rows, country=country, quotes=cache)
    report = _native_review_report(conn, tmp_path, departure)
    assert report['departureDate'] == day
    assert report['departureTimezone'] == ('Europe/Vienna' if country == 'at' else 'UTC')
    assert (report['totals']['neuQuantitySats'], report['totals']['altQuantitySats']) == (neu, alt)
    source = report['fmvSource'][0]
    assert (source['rate'], source['source'], source['asOf']) == quote
