"""Contracts for disposable whole-stage parity operands."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import textwrap

import pytest

from tests.rust_port.journal_pipeline_oracle import capture
from tests.rust_port.journal_prebuild_campaign import PATH, frozen, packed
from tests.rust_port.journal_prebuild_oracle import ROOT, seed, transaction, rate
from tests.rust_port.journal_persistence_oracle import decode_scalar, dump


def test_prebuild_recorder_preserves_pending_caller_writes():
    conn = sqlite3.connect(':memory:')
    conn.row_factory = sqlite3.Row
    try:
        seed(conn)
        conn.execute('BEGIN IMMEDIATE')
        transaction(conn)
        rate(conn)
        before = dump(conn)
        case = capture(conn, 'pending-caller', 'p', pending=True)
        assert case['error'] is None
        assert case['result']['auto_priced'] == 1
        assert case['expected'] != before
        assert dump(conn) == before
        assert conn.in_transaction
        conn.rollback()
        assert conn.execute('SELECT COUNT(*) FROM transactions').fetchone()[0] == 0
    finally:
        conn.close()


def test_prebuild_campaign_scenario_vectors_reproduce():
    rejections = []
    cases = frozen(rejections)
    expected = dict(packed(cases), unrepresentable=rejections)
    assert json.loads(PATH.read_text()) == expected
    assert len({case['name'].split('/')[0] for case in cases}) == 12


def test_prebuild_numeric_oracles_reproduce(monkeypatch):
    from tests.rust_port import journal_pipeline_oracle, journal_prebuild_oracle

    monkeypatch.setattr(sys, 'argv', ['prebuild-oracle'])
    journal_prebuild_oracle.main()
    journal_pipeline_oracle.main()


def test_prebuild_dumps_round_trip_nonfinite_sqlite_values():
    source = sqlite3.connect(':memory:')
    restored = sqlite3.connect(':memory:')
    try:
        source.execute('CREATE TABLE samples(amount REAL)')
        source.executemany('INSERT INTO samples(amount) VALUES(?)',
                           [(float('inf'),), (float('-inf'),), (None,)])
        captured = json.loads(json.dumps(dump(source), allow_nan=False))
        restored.execute('CREATE TABLE samples(amount REAL)')
        restored.executemany('INSERT INTO samples(rowid,amount) VALUES(?,?)',
                             [tuple(decode_scalar(v) for v in row) for row in captured['samples']['rows']])
        assert restored.execute('SELECT typeof(amount),amount FROM samples ORDER BY rowid').fetchall() == [
            ('real', float('inf')), ('real', float('-inf')), ('null', None),
        ]
    finally:
        source.close()
        restored.close()


@pytest.mark.skipif(sys.platform != 'linux' or os.uname().machine != 'x86_64', reason='native syscall guard requires Linux x86-64')
@pytest.mark.parametrize('first_import', [
    'from kassiber.cli import handlers',
    'from tests.rust_port.journal_pipeline_oracle import capture',
    'from tests.rust_port.journal_prebuild_oracle import seed',
])
def test_prebuild_child_difference_cannot_be_swallowed(first_import):
    (ROOT / 'target').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=ROOT / 'target', prefix='prebuild-child-gate-') as directory:
        scratch = Path(directory)
        binary = scratch / 'fake-native'
        binary.write_text(f'#!{sys.executable}\n' + textwrap.dedent('''
            import sys
            for line in sys.stdin:
                print('{"verified": false}', flush=True)
        '''))
        binary.chmod(0o700)
        child = scratch / 'child.py'
        child.write_text(first_import + '\n' + textwrap.dedent('''
            import sqlite3
            from kassiber.cli import handlers
            from tests.rust_port.journal_prebuild_oracle import seed
            conn = sqlite3.connect(':memory:')
            conn.row_factory = sqlite3.Row
            seed(conn)
            conn.execute('BEGIN IMMEDIATE')
            try:
                handlers._repair_journal_source_overlaps(conn, conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone())
            except AssertionError:
                pass
            finally:
                conn.close()
        '''))
        parent = scratch / 'test_parent.py'
        parent.write_text('import subprocess, sys\ndef test_child():\n'
                          f'    assert subprocess.run([sys.executable, {str(child)!r}], check=False).returncode == 0\n')
        env = {key: value for key, value in os.environ.items() if not key.startswith(
            ('KASSIBER_TEST_TAX_', 'KASSIBER_TEST_CUSTODY_', 'KASSIBER_TEST_JOURNAL_PREBUILD_'))}
        result = subprocess.run([sys.executable, '-m', 'pytest', '-p', 'tests.conftest',
                                 '-p', 'tests.rust_port.journal_prebuild_recorder', '-p', 'no:cacheprovider',
                                 '--prebuild-binary', str(binary), '--prebuild-recordings', str(scratch / 'records'),
                                 '-q', str(parent)], cwd=ROOT, env=env, text=True, capture_output=True, timeout=120)
        assert result.returncode == 1, result.stdout + result.stderr
        assert '1 passed' in result.stdout, result.stdout + result.stderr
        assert 'pre-build comparison failures: 1' in result.stdout, result.stdout + result.stderr
        assert len(list((scratch / 'records').glob('*.prebuild.failure'))) == 1
        retained = subprocess.run([sys.executable, '-m', 'pytest', '-p', 'tests.conftest',
                                  '-p', 'tests.rust_port.journal_prebuild_recorder', '-p', 'no:cacheprovider',
                                  '--prebuild-binary', str(binary), '--prebuild-recordings', str(scratch / 'records'),
                                  '-q', str(parent)], cwd=ROOT, env=env, text=True, capture_output=True, timeout=120)
        assert retained.returncode != 0
        assert 'require an empty destination' in retained.stdout + retained.stderr
        assert len(list((scratch / 'records').glob('*.prebuild.failure'))) == 1
        parent.write_text('def test_empty():\n    pass\n')
        empty = subprocess.run([sys.executable, '-m', 'pytest', '-p', 'tests.conftest',
                               '-p', 'tests.rust_port.journal_prebuild_recorder', '-p', 'no:cacheprovider',
                               '--prebuild-binary', str(binary), '--prebuild-recordings', str(scratch / 'empty'),
                               '-q', str(parent)], cwd=ROOT, env=env, text=True, capture_output=True, timeout=120)
        assert empty.returncode == 1, empty.stdout + empty.stderr
        assert '1 passed' in empty.stdout
        assert 'whole pre-build: 0 complete full-book comparisons' in empty.stdout
        assert not list((scratch / 'empty').iterdir())


@pytest.mark.skipif(sys.platform != 'linux' or os.uname().machine != 'x86_64', reason='native syscall guard requires Linux x86-64')
def test_prebuild_native_guard_preserves_import_time_diagnostics():
    (ROOT / 'target').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=ROOT / 'target', prefix='prebuild-import-time-') as directory:
        binary = Path(directory) / 'native'
        binary.write_text(f'#!{sys.executable}\n' + textwrap.dedent('''
            import sys
            for line in sys.stdin:
                print('{"ok": true}', flush=True)
        '''))
        binary.chmod(0o700)
        probe = textwrap.dedent('''
            import importlib.util, sys
            from pathlib import Path
            spec = importlib.util.spec_from_file_location('prebuild_check', 'scripts/check-journal-prebuild.py')
            check = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(check)
            assert check.native(Path(sys.argv[1]), [{}]) == [{'ok': True}]
        ''')
        env = {key: value for key, value in os.environ.items() if not key.startswith(
            ('KASSIBER_TEST_TAX_', 'KASSIBER_TEST_CUSTODY_', 'KASSIBER_TEST_JOURNAL_PREBUILD_'))}
        result = subprocess.run([sys.executable, '-X', 'importtime', '-c', probe, str(binary)],
                                cwd=ROOT, env=env, text=True, capture_output=True, timeout=120)
        assert result.returncode == 0, result.stdout + result.stderr
        assert 'import time:' in result.stderr and 'resource' in result.stderr


@pytest.mark.skipif(sys.platform != 'linux' or os.uname().machine != 'x86_64', reason='native syscall guard requires Linux x86-64')
def test_prebuild_recorder_install_is_reentrant():
    (ROOT / 'target').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=ROOT / 'target', prefix='prebuild-reentrant-') as directory:
        scratch = Path(directory)
        binary = scratch / 'native'
        binary.write_text(f'#!{sys.executable}\n' + textwrap.dedent('''
            import json, sys
            for line in sys.stdin:
                request = json.loads(line)
                print(json.dumps(dict(verified=True, result=request['result'], error=request['error'],
                                      projection_wire=request['wire_fields'])), flush=True)
        '''))
        binary.chmod(0o700)
        child = scratch / 'child.py'
        child.write_text(textwrap.dedent(f'''
            import builtins, os, sqlite3
            from unittest.mock import patch
            from kassiber.cli import handlers
            from tests.rust_port import journal_prebuild_recorder as recorder
            from tests.rust_port.journal_prebuild_oracle import seed
            os.environ[recorder.BINARY_ENV] = {str(binary)!r}
            os.environ[recorder.DIRECTORY_ENV] = {str(scratch / 'records')!r}
            original_import = builtins.__import__
            entered = False
            def importing(name, globals=None, locals=None, fromlist=(), level=0):
                global entered
                module = original_import(name, globals, locals, fromlist, level)
                if name == 'kassiber.cli' and 'handlers' in fromlist and not entered:
                    entered = True
                    recorder.install_from_environment()
                return module
            with patch('builtins.__import__', side_effect=importing):
                recorder.install_from_environment()
            assert entered
            conn = sqlite3.connect(':memory:')
            conn.row_factory = sqlite3.Row
            seed(conn)
            conn.execute('BEGIN IMMEDIATE')
            handlers._repair_journal_source_overlaps(conn, conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone())
            conn.close()
            recorder.uninstall()
        '''))
        env = {key: value for key, value in os.environ.items() if not key.startswith(
            ('KASSIBER_TEST_TAX_', 'KASSIBER_TEST_CUSTODY_', 'KASSIBER_TEST_JOURNAL_PREBUILD_'))}
        result = subprocess.run([sys.executable, str(child)], cwd=ROOT, env=env,
                                text=True, capture_output=True, timeout=120)
        assert result.returncode == 0, result.stdout + result.stderr
        assert len(list((scratch / 'records').glob('*.prebuild.json'))) == 1
        assert not list((scratch / 'records').glob('*.prebuild.failure'))


@pytest.mark.skipif(sys.platform != 'linux' or os.uname().machine != 'x86_64', reason='native syscall guard requires Linux x86-64')
def test_prebuild_campaign_resume_checks_prefix_and_runs_only_missing_operands():
    from tests.rust_port.journal_prebuild_campaign import digest
    (ROOT / 'target').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=ROOT / 'target', prefix='prebuild-resume-') as directory:
        scratch = Path(directory)
        records = scratch / 'records'
        records.mkdir()
        conn = sqlite3.connect(':memory:')
        conn.row_factory = sqlite3.Row
        seed(conn)
        case = capture(conn, 'resume', 'p')
        conn.close()
        for name in ('a', 'b'):
            (records / f'{name}.prebuild.json').write_text(json.dumps(case))
        binary = scratch / 'native'
        calls = scratch / 'calls'
        binary.write_text(f'#!{sys.executable}\n' + textwrap.dedent(f'''
            import json, sys
            for line in sys.stdin:
                request = json.loads(line)
                with open({str(calls)!r}, 'a') as output:
                    output.write('called\\n')
                print(json.dumps(dict(verified=True, result=request['result'], error=request['error'],
                                      projection_wire=request['wire_fields'])), flush=True)
        '''))
        binary.chmod(0o700)
        out = scratch / 'summary.json'
        command = [sys.executable, str(ROOT / 'scripts/check-journal-prebuild-campaign.py'),
                   '--binary', str(binary), '--out', str(out), '--recordings', str(records), '--files']
        subprocess.run(command, check=True, capture_output=True, text=True, timeout=60)
        audit = out.with_suffix('.activity.jsonl')
        first = audit.read_text().splitlines()[0]
        audit.write_text(first + '\n')
        calls.write_text('')
        subprocess.run([*command, '--resume'], check=True, capture_output=True, text=True, timeout=60)
        summary = json.loads(out.read_text())
        assert summary['complete'] and summary['resumed_operands'] == 1
        assert summary['comparisons'] == 4
        assert len(calls.read_text().splitlines()) == 2
        assert audit.read_text().splitlines()[0] == first
        assert summary['suites']['recorded']['totals']['books'] == 2
        changed = dict(case, name='different')
        (records / 'a.prebuild.json').write_text(json.dumps(changed))
        refused = subprocess.run([*command, '--resume'], capture_output=True, text=True, timeout=60)
        assert refused.returncode != 0 and 'resume operand 0 differs' in refused.stderr
        assert json.loads(first)['input_sha256'] == digest(case)
