"""Registered journal commands: wire bytes, scopes, handoff and crash recovery."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import sys

import pytest

from kassiber.db import open_db
from tests.rust_port.journal_persistence_oracle import dump
from tests.test_journal_storage_format import _book
from tests.test_rust_journal_process import scalar_exact
from tests.rust_port.journal_refresh_oracle import SCENARIOS

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get('KASSIBER_RUST_BIN', ROOT / 'target/rust/debug/kassiber'))
CLOCK = '2026-10-31T12:00:00Z'
KEY = 'synthetic-registration-only'
pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason='requires native harness binary')


@pytest.fixture
def environment(tmp_path):
    # Disposable roots: under target/, or the session recorder's namespace
    # (crates/session-replay/record-sessions.sh binds a scratch directory at /mnt).
    assert tmp_path.resolve().is_relative_to((ROOT / 'target').resolve()) or tmp_path.resolve().is_relative_to('/mnt/bt')
    owners = tmp_path / 'owners'
    owners.mkdir()
    env = {k: v for k, v in os.environ.items() if not k.startswith(('KASSIBER_OPERATOR_', 'KASSIBER_RUST_TEST_'))}
    env.update(
        KASSIBER_RUST_TEST_OWNER_ROOT=str(owners),
        KASSIBER_RUST_TEST_JOURNAL_TIME=CLOCK,
        KASSIBER_RUST_TEST_JOURNAL_IDS='sequential',
        KASSIBER_TEST_NO_EGRESS='1',
        KASSIBER_DISABLE_UPDATE_CHECK='1',
        KASSIBER_HYBRID_RUST_BIN=str(BINARY.resolve()),
        KASSIBER_HYBRID_LOG=str(tmp_path / 'served.jsonl'),
        XDG_DATA_HOME=str(tmp_path / 'xdg-data'),
        XDG_CONFIG_HOME=str(tmp_path / 'xdg-config'),
        XDG_STATE_HOME=str(tmp_path / 'xdg-state'),
    )
    return env


def seed(root, cipher=False, country='generic', shape='unchanged'):
    conn = _book(root, country=country, passphrase=KEY if cipher else None)
    conn.execute("INSERT OR REPLACE INTO settings VALUES('context_workspace','w'),('context_profile','p')")
    conn.execute("UPDATE profiles SET last_processed_at='2020-01-01T00:00:00Z'")
    if shape in ('changed', 'fault', 'kill'):
        conn.execute("UPDATE transactions SET fiat_rate=12345,fiat_rate_exact='12345' WHERE id='a'")
    elif shape == 'blocked':
        conn.execute("UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL WHERE id='a'")
    elif shape == 'empty':
        conn.execute('DELETE FROM transactions')
    elif shape == 'build-refusal':
        conn.execute("UPDATE profiles SET gains_algorithm='invalid-method'")
    elif shape == 'admission-refusal':
        conn.execute("UPDATE profiles SET tax_country='unsupported'")
    elif shape == 'label-refusal':
        # A multi-key refusal whose details keep Python's insertion order.
        conn.execute("UPDATE wallets SET label='Wallet:1'")
    conn.commit()
    conn.close()
    return root


def book_dump(root, cipher=False):
    conn = open_db(root, passphrase=KEY if cipher else None)
    try:
        assert conn.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        return dump(conn)
    finally:
        conn.close()


@contextmanager
def arguments(root, cipher=False, extra=()):
    args = ['--machine', '--data-root', str(root)]
    read_fd = None
    if cipher:
        read_fd, write_fd = os.pipe()
        os.write(write_fd, (KEY + '\n').encode())
        os.close(write_fd)
        args += ['--db-passphrase-fd', str(read_fd)]
    try:
        yield [*args, 'journals', 'process', *extra], (() if read_fd is None else (read_fd,))
    finally:
        if read_fd is not None:
            os.close(read_fd)


def invoke(root, env, *, cipher=False, native=False, extra=()):
    with arguments(root, cipher, extra) as (args, fds):
        prefix = [str(BINARY)] if native else [sys.executable, '-m', 'tests.rust_port.journal_cli_reference']
        return subprocess.run([*prefix, *args], env=env, pass_fds=fds, capture_output=True, cwd=ROOT, timeout=120)


def seed_auto(root, cipher=False, scenario='exact', current=False, country='generic'):
    from crates.transfers.scripts.export_pairing_services import generated, seed as seed_pairing
    from kassiber.cli import handlers
    conn = open_db(root, passphrase=KEY if cipher else None)
    try:
        rows, setup, actions, _ = generated(1, scenario)
        seed_pairing(conn, rows)
        for sql in setup:
            conn.execute(sql)
        pair = None
        for action, _ in actions:
            if action == 'pair':
                pair = handlers.create_transaction_pair(conn, 'w', 'p', 'a-o', 'a-i', kind='chain-swap')
            else:
                handlers.delete_transaction_pair(conn, 'w', 'p', pair['id'])
        conn.execute("INSERT OR REPLACE INTO settings VALUES('context_workspace','w'),('context_profile','p')")
        conn.execute('UPDATE profiles SET tax_country=?', (country,))
        if current:
            conn.execute("UPDATE profiles SET last_processed_at=?,last_processed_tx_count="
                         "(SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0),"
                         "last_processed_input_version=journal_input_version WHERE id='p'", (CLOCK,))
            # Python's trigger clears the semantics stamp when the markers move.
            conn.execute("UPDATE profiles SET journal_semantics_version=10 WHERE id='p'")
        else:
            conn.execute("UPDATE profiles SET last_processed_at=NULL WHERE id='p'")
        conn.commit()
    finally:
        conn.close()
    return root


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('current', [False, True])
@pytest.mark.parametrize('scenario,country', [(s, 'generic') for s in SCENARIOS] + [('exact', 'at')])
def test_registered_auto_pair_matches_exact_envelope_complete_book_and_native_route(tmp_path, environment, cipher, current, scenario, country):
    reference = seed_auto(tmp_path / 'python', cipher, scenario, current, country)
    native = tmp_path / 'rust'
    shutil.copytree(reference, native)
    expected = invoke(reference, environment, cipher=cipher, extra=('--auto-pair',))
    with arguments(native, cipher, ('--auto-pair',)) as (args, fds):
        actual = subprocess.run([sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py'), *args],
                                env=environment, pass_fds=fds, capture_output=True, cwd=ROOT, timeout=120)
    assert expected.returncode == actual.returncode == 0, (expected, actual)
    assert (actual.stdout, actual.stderr) == (expected.stdout, expected.stderr)
    assert scalar_exact(book_dump(native, cipher), book_dump(reference, cipher))
    record = json.loads(Path(environment['KASSIBER_HYBRID_LOG']).read_text())
    assert record == dict(command='journals.process', served_by='rust')
    if scenario == 'exact':
        assert json.loads(actual.stdout)['data']['auto_pair']['applied'] > 0


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('current', [False, True])
def test_registered_auto_pair_late_failure_retains_only_python_first_commit_and_never_replays_it(tmp_path, environment, cipher, current):
    reference = seed_auto(tmp_path / 'python', cipher, current=current)
    native = tmp_path / 'rust'
    shutil.copytree(reference, native)
    before = book_dump(native, cipher)
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_FAIL_PAIRED_STORE='1', KASSIBER_RUST_TEST_JOURNAL_FALLBACK='1')
    expected = invoke(reference, env, cipher=cipher, extra=('--auto-pair',))
    actual = invoke(native, env, cipher=cipher, native=True, extra=('--auto-pair',))
    assert expected.returncode == 1, expected
    if current:
        assert actual.returncode == 3, actual
        assert actual.stdout == b'storing:IntegrityError\n'
        assert not actual.stderr
    else:
        assert actual.returncode == 1, actual
        assert (actual.stdout, actual.stderr) == (expected.stdout, expected.stderr)
    assert scalar_exact(book_dump(native, cipher), book_dump(reference, cipher))
    assert scalar_exact(book_dump(native, cipher), before) == current


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('point', ['first', 'second-first', 'second-last'])
@pytest.mark.parametrize('mode', ['fault', 'kill'])
def test_registered_auto_pair_write_recovery_keeps_exact_reference_commit(tmp_path, environment, cipher, point, mode):
    root = seed_auto(tmp_path / 'native', cipher)
    before = book_dump(root, cipher)
    reference = tmp_path / 'python'
    shutil.copytree(root, reference)
    reference_env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_FAIL_PAIRED_STORE='1')
    failed = invoke(reference, reference_env, cipher=cipher, extra=('--auto-pair',))
    assert failed.returncode == 1, failed
    partial = book_dump(reference, cipher)
    assert not scalar_exact(partial, before)
    baseline_root = tmp_path / 'baseline'
    shutil.copytree(root, baseline_root)
    baseline = invoke(baseline_root, dict(environment, KASSIBER_RUST_TEST_JOURNAL_TRACE_WRITES='1'),
                      cipher=cipher, native=True, extra=('--auto-pair',))
    assert baseline.returncode == 0, baseline
    assert json.loads(baseline.stdout)['data']['auto_pair']['applied'] > 0
    boundaries = [json.loads(line)['writes'] for line in baseline.stderr.splitlines()]
    assert len(boundaries) == 2 and 0 < boundaries[0] < boundaries[1], boundaries
    write = {'first': 1, 'second-first': boundaries[0] + 1, 'second-last': boundaries[1]}[point]
    expected = before if point == 'first' else partial
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_FALLBACK='1')
    env['KASSIBER_RUST_TEST_JOURNAL_' + ('FAIL' if mode == 'fault' else 'PAUSE') + '_AFTER_WRITE'] = str(write)
    if mode == 'fault':
        actual = invoke(root, env, cipher=cipher, native=True, extra=('--auto-pair',))
        assert actual.returncode == (3 if point == 'first' else 1), actual
        if point == 'first':
            assert actual.stdout == b'storing:DatabaseError\n'
        else:
            assert json.loads(actual.stdout)['error']['code'] == 'internal_error'
        assert not actual.stderr
    else:
        with arguments(root, cipher, ('--auto-pair',)) as (args, fds):
            with subprocess.Popen([str(BINARY), *args], env=env, pass_fds=fds, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, cwd=ROOT) as child:
                try:
                    with selectors.DefaultSelector() as selector:
                        selector.register(child.stdout, selectors.EVENT_READ)
                        assert selector.select(timeout=120), 'registered pairing writer did not reach kill point'
                    record = json.loads(child.stdout.readline())
                    assert record['paused_at'] == write, record
                finally:
                    child.kill()
                    child.wait(timeout=30)
                assert not child.stderr.read()
    assert scalar_exact(book_dump(root, cipher), expected)


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('country', ['generic', 'at'])
@pytest.mark.parametrize('shape', ['unchanged', 'changed', 'blocked', 'empty'])
def test_registered_plain_cli_matches_exact_envelope_and_complete_book(tmp_path, environment, cipher, country, shape):
    reference = seed(tmp_path / 'python', cipher, country, shape)
    native = tmp_path / 'rust'
    shutil.copytree(reference, native)
    expected = invoke(reference, environment, cipher=cipher)
    actual = invoke(native, environment, cipher=cipher, native=True)
    assert expected.returncode == actual.returncode == 0, (expected, actual)
    assert (actual.stdout, actual.stderr) == (expected.stdout, expected.stderr)
    assert scalar_exact(book_dump(native, cipher), book_dump(reference, cipher))


@pytest.mark.parametrize('extra', [('--profile', 'missing'), ('--workspace', 'missing'), ('--profile', 'Foreign')])
@pytest.mark.parametrize('auto_pair', [False, True])
@pytest.mark.parametrize('cipher', [False, True])
def test_registered_plain_scope_refusals_preserve_envelope_and_every_table(tmp_path, environment, extra, auto_pair, cipher):
    reference = seed(tmp_path / 'python', cipher)
    conn = open_db(reference, passphrase=KEY if cipher else None)
    conn.execute("INSERT INTO workspaces VALUES('foreign','Foreign','2020')")
    conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('foreign','foreign','Foreign','2020')")
    conn.commit()
    conn.close()
    native = tmp_path / 'rust'
    shutil.copytree(reference, native)
    before = book_dump(native, cipher)
    if auto_pair:
        extra = ('--auto-pair', *extra)
    expected = invoke(reference, environment, cipher=cipher, extra=extra)
    actual = invoke(native, environment, cipher=cipher, native=True, extra=extra)
    assert expected.returncode == actual.returncode == 1
    assert (actual.stdout, actual.stderr) == (expected.stdout, expected.stderr)
    assert scalar_exact(book_dump(native, cipher), before)


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('selectors', [('w', 'p'), ('Main', 'Book')])
def test_registered_auto_pair_explicit_scope_overrides_foreign_active_context(tmp_path, environment, cipher, selectors):
    reference = seed_auto(tmp_path / 'python', cipher)
    conn = open_db(reference, passphrase=KEY if cipher else None)
    conn.execute("INSERT INTO workspaces VALUES('foreign','Foreign','2020')")
    conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('foreign','foreign','Foreign','2020')")
    conn.execute("UPDATE settings SET value='foreign' WHERE key IN ('context_workspace','context_profile')")
    conn.commit()
    conn.close()
    native = tmp_path / 'rust'
    shutil.copytree(reference, native)
    extra = ('--auto-pair', '--workspace', selectors[0], '--profile', selectors[1])
    expected = invoke(reference, environment, cipher=cipher, extra=extra)
    actual = invoke(native, environment, cipher=cipher, native=True, extra=extra)
    assert expected.returncode == actual.returncode == 0, (expected, actual)
    assert (actual.stdout, actual.stderr) == (expected.stdout, expected.stderr)
    assert json.loads(actual.stdout)['data']['auto_pair']['applied'] > 0
    assert scalar_exact(book_dump(native, cipher), book_dump(reference, cipher))


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('shape', ['changed', 'build-refusal', 'label-refusal', 'admission-refusal', 'fault'])
def test_hybrid_plain_counts_native_and_proven_rollback_fallback_shapes(tmp_path, environment, cipher, shape):
    reference = seed(tmp_path / 'python', cipher, shape=shape)
    native = tmp_path / 'rust'
    shutil.copytree(reference, native)
    expected = invoke(reference, environment, cipher=cipher)
    env = dict(environment)
    if shape == 'fault':
        env['KASSIBER_RUST_TEST_JOURNAL_FAIL_AFTER_WRITE'] = '8'
    with arguments(native, cipher) as (args, fds):
        actual = subprocess.run([sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py'), *args], env=env,
                                pass_fds=fds, capture_output=True, cwd=ROOT, timeout=120)
    assert actual.returncode == expected.returncode, (actual, expected)
    # A replay samples the real Python clock; compare only that declared source.
    if shape == 'fault':
        value = json.loads(actual.stdout)
        value['data']['processed_at'] = json.loads(expected.stdout)['data']['processed_at']
        assert value == json.loads(expected.stdout)
    else:
        assert (actual.stdout, actual.stderr) == (expected.stdout, expected.stderr)
    records = [json.loads(line) for line in Path(env['KASSIBER_HYBRID_LOG']).read_text().splitlines()]
    assert len(records) == 1
    assert records[0]['command'] == 'journals.process'
    # Typed AppError refusals are exact natively; exception families replay.
    native = shape in ('changed', 'build-refusal', 'label-refusal')
    assert records[0]['served_by'] == ('rust' if native else 'python'), records
    assert ('fallback_shape' in records[0]) == (not native)


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('write', [1, 8, 15])
def test_registered_plain_killed_writer_recovers_every_table(tmp_path, environment, cipher, write):
    root = seed(tmp_path / 'kill', cipher, shape='kill')
    before = book_dump(root, cipher)
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_PAUSE_AFTER_WRITE=str(write))
    with arguments(root, cipher) as (args, fds):
        with subprocess.Popen([str(BINARY), *args], env=env, pass_fds=fds, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, cwd=ROOT) as child:
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(child.stdout, selectors.EVENT_READ)
                    assert selector.select(timeout=120), 'registered writer did not reach kill point'
                record = json.loads(child.stdout.readline())
                assert record['paused_at'] == write
            finally:
                child.kill()
                child.wait(timeout=30)
            assert not child.stderr.read()
    assert scalar_exact(book_dump(root, cipher), before)


@pytest.mark.parametrize('cipher', [False, True])
@pytest.mark.parametrize('write', [1, 8, 15])
def test_registered_plain_fault_rolls_back_every_table_before_replay(tmp_path, environment, cipher, write):
    root = seed(tmp_path / 'fault', cipher, shape='fault')
    before = book_dump(root, cipher)
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_FAIL_AFTER_WRITE=str(write),
               KASSIBER_RUST_TEST_JOURNAL_FALLBACK='1')
    actual = invoke(root, env, cipher=cipher, native=True)
    assert actual.returncode == 3, (actual.returncode, actual.stdout, actual.stderr)
    assert actual.stdout == b'storing:DatabaseError\n'
    assert not actual.stderr
    assert scalar_exact(book_dump(root, cipher), before)


@pytest.mark.parametrize('cipher', [False, True])
def test_registered_plain_never_replays_after_a_bootstrap_schema_write(tmp_path, environment, cipher):
    # A replayable exception family, so only the schema write can block replay.
    root = seed(tmp_path / 'migration', cipher, shape='admission-refusal')
    conn = open_db(root, passphrase=KEY if cipher else None)
    conn.execute('DROP INDEX idx_transactions_profile_page')
    conn.commit()
    conn.close()
    env = dict(environment, KASSIBER_RUST_TEST_JOURNAL_FALLBACK='1')
    actual = invoke(root, env, cipher=cipher, native=True)
    assert actual.returncode == 1, (actual.returncode, actual.stdout, actual.stderr)
    assert json.loads(actual.stdout)['error']['code'] != 'not_implemented'
    # Inspect directly: open_db would itself recreate the missing index.
    from sqlcipher3 import dbapi2
    conn = dbapi2.connect(str(root / 'kassiber.sqlite3'))
    try:
        if cipher:
            conn.execute("PRAGMA key='synthetic-registration-only'")
        assert conn.execute("SELECT COUNT(*) FROM sqlite_master WHERE name='idx_transactions_profile_page'").fetchone()[0] == 1
        assert conn.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
    finally:
        conn.close()


@pytest.mark.parametrize('style', ['inline', 'abbreviated', 'repeated'])
def test_hybrid_refusal_replays_the_last_passphrase_descriptor(tmp_path, environment, style):
    # An exception-family refusal; typed AppError refusals no longer replay.
    reference = seed(tmp_path / 'python', True, shape='admission-refusal')
    native = tmp_path / 'rust'
    shutil.copytree(reference, native)
    expected = invoke(reference, environment, cipher=True)
    with arguments(native, True) as (args, fds):
        descriptor = str(fds[0])
        flags = {
            'inline': ['--db-passphrase-fd=' + descriptor],
            'abbreviated': ['--db-passphrase', descriptor],
            'repeated': ['--db-passphrase-fd', '999999', '--db-passphrase-fd=' + descriptor],
        }[style]
        args = [*args[:3], *flags, *args[5:]]
        actual = subprocess.run([sys.executable, str(ROOT / 'tests/rust_port/hybrid_cli.py'), *args],
                                env=environment, pass_fds=fds, capture_output=True, cwd=ROOT, timeout=120)
    assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr)
    record = json.loads(Path(environment['KASSIBER_HYBRID_LOG']).read_text())
    assert record == dict(command='journals.process', served_by='python', fallback_shape='preparing:ValueError')
