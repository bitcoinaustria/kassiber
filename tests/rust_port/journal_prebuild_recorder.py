"""Explicit pytest plugin for local full-book pre-build recording and comparison."""
from __future__ import annotations

import importlib.util
import itertools
import json
import os
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]

_ORIGINAL = None
_FAILURES = []
_SEQUENCE = itertools.count()
_PREVIOUS_ENV = {}
BINARY_ENV = 'KASSIBER_TEST_JOURNAL_PREBUILD_BINARY'
DIRECTORY_ENV = 'KASSIBER_TEST_JOURNAL_PREBUILD_DIR'


def pytest_addoption(parser):
    parser.addoption('--prebuild-binary', type=Path)
    parser.addoption('--prebuild-recordings', type=Path)


def pytest_configure(config):
    binary = config.getoption('--prebuild-binary')
    if binary is None:
        return
    _FAILURES.clear()
    destination = config.getoption('--prebuild-recordings')
    if destination is None:
        raise ValueError('pre-build recordings require a disposable worktree target directory')
    if destination.exists() and any(destination.iterdir()):
        raise ValueError('pre-build recordings require an empty destination')
    for key, value in ((BINARY_ENV, str(binary.resolve())), (DIRECTORY_ENV, str(destination.resolve()))):
        _PREVIOUS_ENV[key] = os.environ.get(key)
        os.environ[key] = value
    uninstall()
    install_from_environment()


def install_from_environment():
    global _ORIGINAL
    if _ORIGINAL is not None or not os.environ.get(BINARY_ENV):
        return
    binary = Path(os.environ[BINARY_ENV])
    destination = Path(os.environ[DIRECTORY_ENV])
    if not destination.resolve().is_relative_to((ROOT / 'target').resolve()):
        raise ValueError('pre-build recordings require a disposable worktree target directory')
    destination.mkdir(mode=0o700, parents=True, exist_ok=True)
    from kassiber.cli import handlers
    if _ORIGINAL is not None:
        return
    original = handlers._repair_journal_source_overlaps
    _ORIGINAL = original
    check = None

    def compare(conn, profile, stem):
        nonlocal check
        from tests.rust_port.journal_pipeline_oracle import capture
        if check is None:
            spec = importlib.util.spec_from_file_location('prebuild_check', ROOT / 'scripts/check-journal-prebuild.py')
            check = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(check)
        schema = [list(row) for row in conn.execute("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")]
        with patch.object(handlers, '_repair_journal_source_overlaps', original):
            case = capture(conn, f'fresh-{stem}', profile['id'], pending=True)
        case['schema'] = schema
        case['recorded_by'] = os.environ.get('PYTEST_CURRENT_TEST')
        request = dict(case, stage='pipeline', dump=False)
        row = check.native(binary.resolve(), [request])[0]
        assert row['verified'], 'pre-build full-book database mismatch'
        for actual, expected in (('result', 'result'), ('error', 'error'), ('projection_wire', 'wire_fields')):
            assert row[actual] == case[expected], f'pre-build full-book {actual} mismatch'
        path = destination / f'{stem}.prebuild.json'
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, 'w') as output:
            output.write(json.dumps(case, ensure_ascii=True, allow_nan=False))

    def compared(conn, profile):
        stem = f'{os.getpid()}-{next(_SEQUENCE):06}'
        try:
            compare(conn, profile, stem)
        except Exception:
            _FAILURES.append('full-book pre-build comparison failed')
            descriptor = os.open(destination / f'{stem}.prebuild.failure', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, 'w') as output:
                output.write('full-book pre-build comparison failed\n')
            raise
        return original(conn, profile)

    handlers._repair_journal_source_overlaps = compared


def uninstall():
    global _ORIGINAL
    if _ORIGINAL is not None:
        from kassiber.cli import handlers
        handlers._repair_journal_source_overlaps = _ORIGINAL
        _ORIGINAL = None


def pytest_unconfigure(config):
    uninstall()
    for key, previous in _PREVIOUS_ENV.items():
        if previous is None:
            os.environ.pop(key, None)
        else:
            os.environ[key] = previous
    _PREVIOUS_ENV.clear()


def recordings(config):
    destination = config.getoption('--prebuild-recordings')
    if destination is None:
        return 0, 0
    return len(list(destination.glob('*.prebuild.json'))), len(list(destination.glob('*.prebuild.failure')))


def pytest_terminal_summary(terminalreporter):
    if terminalreporter.config.getoption('--prebuild-binary'):
        count, failures = recordings(terminalreporter.config)
        terminalreporter.write_line(f'whole pre-build: {count} complete full-book comparisons')
        if failures or _FAILURES:
            terminalreporter.write_line(f'pre-build comparison failures: {max(failures, len(_FAILURES))}', red=True)


def pytest_sessionfinish(session):
    if session.config.getoption('--prebuild-binary'):
        count, failures = recordings(session.config)
        if _FAILURES or failures or not count:
            session.exitstatus = 1
