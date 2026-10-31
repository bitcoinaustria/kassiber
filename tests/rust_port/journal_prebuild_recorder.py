"""Explicit pytest plugin for local full-book pre-build recording and comparison."""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
from unittest.mock import patch

from tests.rust_port.journal_pipeline_oracle import capture
from tests.rust_port.journal_prebuild_oracle import ROOT

_ORIGINAL = None
_COUNT = 0
_FAILURES = []


def pytest_addoption(parser):
    parser.addoption('--prebuild-binary', type=Path)
    parser.addoption('--prebuild-recordings', type=Path)


def pytest_configure(config):
    global _ORIGINAL, _COUNT
    binary = config.getoption('--prebuild-binary')
    if binary is None:
        return
    _COUNT = 0
    _FAILURES.clear()
    destination = config.getoption('--prebuild-recordings')
    if destination is None or not destination.resolve().is_relative_to(ROOT / 'target'):
        raise ValueError('pre-build recordings require a disposable worktree target directory')
    destination.mkdir(mode=0o700, parents=True, exist_ok=True)
    spec = importlib.util.spec_from_file_location('prebuild_check', ROOT / 'scripts/check-journal-prebuild.py')
    check = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(check)
    from kassiber.cli import handlers
    _ORIGINAL = handlers._repair_journal_source_overlaps

    def compare(conn, profile):
        global _COUNT
        schema = [list(row) for row in conn.execute("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")]
        with patch.object(handlers, '_repair_journal_source_overlaps', _ORIGINAL):
            case = capture(conn, f'fresh-{_COUNT:04}', profile['id'], pending=True)
        case['schema'] = schema
        request = dict(case, stage='pipeline', dump=False)
        row = check.native(binary.resolve(), [request])[0]
        assert row['verified'], 'pre-build full-book database mismatch'
        for actual, expected in (('result', 'result'), ('error', 'error'), ('projection_wire', 'wire_fields')):
            assert row[actual] == case[expected], f'pre-build full-book {actual} mismatch'
        path = destination / f'{_COUNT:04}.prebuild.json'
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, 'w') as output:
            output.write(json.dumps(case, ensure_ascii=True, allow_nan=False))
        _COUNT += 1

    def compared(conn, profile):
        try:
            compare(conn, profile)
        except Exception:
            _FAILURES.append('full-book pre-build comparison failed')
            raise
        return _ORIGINAL(conn, profile)

    handlers._repair_journal_source_overlaps = compared


def pytest_unconfigure(config):
    global _ORIGINAL
    if _ORIGINAL is not None:
        from kassiber.cli import handlers
        handlers._repair_journal_source_overlaps = _ORIGINAL
        _ORIGINAL = None


def pytest_terminal_summary(terminalreporter):
    if terminalreporter.config.getoption('--prebuild-binary'):
        terminalreporter.write_line(f'whole pre-build: {_COUNT} complete full-book comparisons')
        for failure in _FAILURES:
            terminalreporter.write_line(failure, red=True)


def pytest_sessionfinish(session):
    if session.config.getoption('--prebuild-binary') and (_FAILURES or not _COUNT):
        session.exitstatus = 1
