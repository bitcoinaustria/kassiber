"""Refresh sessions isolate copied books and dispatch only current selected jobs."""
import json
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import daemon, daemon_freshness
from kassiber.core import freshness
from kassiber.core.wallets import update_wallet
from kassiber.db import open_db, set_setting


@pytest.fixture
def make_book(tmp_path):
    connections = []

    def make(name, count=1):
        conn = open_db(tmp_path / name)
        connections.append(conn)
        conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2026-01-01')")
        conn.execute(
            "INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','ws',?,'EUR','2026-01-01')",
            (name,),
        )
        for index in range(count):
            conn.execute(
                "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'ws','p',?,'address',?,'2026-01-01')",
                (f'w{index}', f'{name}-wallet-{index}', json.dumps({'addresses': ['bc1qfixture']})),
            )
        set_setting(conn, 'context_workspace', 'ws')
        set_setting(conn, 'context_profile', 'p')
        freshness.set_policy(
            conn, 'p', background_enabled=True, report_read_sync=True,
            source_classes={freshness.SOURCE_ONCHAIN: True, freshness.SOURCE_RATES: False, freshness.SOURCE_JOURNALS: False},
        )
        conn.commit()
        return conn

    yield make
    for conn in connections:
        conn.close()


@pytest.fixture(autouse=True)
def reset_cache():
    daemon_freshness._clear_unlocked_passphrase(SimpleNamespace(db_passphrase=None))
    yield
    daemon_freshness._clear_unlocked_passphrase(SimpleNamespace(db_passphrase=None))


def _automatic_refresh(conn):
    return daemon_freshness._auto_sync_wallets_if_enabled(
        conn, {'backends': {'fixture': {}}}, state={},
    )


def test_report_refresh_cache_must_not_cross_project_database(make_book):
    first_book, second_book = make_book('book-a'), make_book('book-b')
    visited = []

    def sync(conn, config, workspace, profile, wallet, **kwargs):
        visited.append(wallet['label'])
        return {'wallet': wallet['label'], 'inserted': 1}

    with patch.object(daemon_freshness, 'sync_wallet_from_backend', side_effect=sync):
        _automatic_refresh(first_book)
        cached = _automatic_refresh(first_book)
        second = _automatic_refresh(second_book)
    assert cached['status'] == 'cached'
    assert visited == ['book-a-wallet-0', 'book-b-wallet-0']
    assert 'book-a-wallet-0' not in json.dumps(second)
    assert second['ok'] is True


def test_report_refresh_reopen_of_same_book_starts_new_session(make_book):
    conn = make_book('book')
    database = conn.execute('PRAGMA database_list').fetchone()[2]
    with patch.object(daemon_freshness, 'sync_wallet_from_backend', return_value={}) as sync:
        _automatic_refresh(conn)
        conn.close()
        reopened = open_db(Path(database).parent)
        try:
            _automatic_refresh(reopened)
        finally:
            reopened.close()
    assert sync.call_count == 2


def test_lock_releases_private_refresh_and_graph_cache(make_book):
    conn = make_book('book')
    context = SimpleNamespace(db_passphrase='fixture-passphrase')
    with patch.dict(daemon._GRAPH_SEMANTICS_CACHE, {'p': object()}, clear=True):
        with patch.object(daemon_freshness, 'sync_wallet_from_backend', return_value={}):
            _automatic_refresh(conn)
        assert daemon_freshness._AUTO_SYNC_CONNECTION is conn
        assert daemon_freshness._AUTO_SYNC_PROFILE_LAST_RESULT
        daemon._clear_unlocked_passphrase(context)
        assert context.db_passphrase is None
        assert daemon_freshness._AUTO_SYNC_CONNECTION is None
        assert daemon_freshness._AUTO_SYNC_PROFILE_LAST_ATTEMPT == {}
        assert daemon_freshness._AUTO_SYNC_PROFILE_LAST_RESULT == {}
        assert daemon._GRAPH_SEMANTICS_CACHE == {}


@pytest.mark.parametrize('count', [10, 100, 300])
def test_foreground_requested_job_reads_scale_linearly(make_book, count):
    conn = make_book(f'book-{count}', count)
    reads = []

    def trace(sql):
        flat = ' '.join(sql.split())
        if flat.startswith('SELECT * FROM freshness_jobs WHERE id ='):
            reads.append(flat)

    conn.set_trace_callback(trace)
    with (
        patch.object(daemon_freshness, '_prefetch_onchain_freshness_jobs', return_value={}),
        patch.object(daemon_freshness, 'sync_wallet_from_backend', return_value={'inserted': 0}),
    ):
        result = daemon_freshness._freshness_run_payload(conn, {}, {'all': True, 'rates': False, 'journals': False})
    assert len(result['completed']) == count
    # Allow bounded per-job checks without prescribing their exact count.
    assert len(reads) <= 8 * count


@pytest.mark.parametrize('stage', ['prefetch', 'first_apply', 'current_apply', 'dispatch'])
def test_deprecating_selected_wallet_does_not_abort_other_wallets(make_book, stage):
    conn = make_book('deprecation', 3)
    daemon_freshness._freshness_run_payload(
        conn, {}, {'all': True, 'rates': False, 'journals': False, 'run': False},
    )
    for index in range(3):
        conn.execute(
            'UPDATE freshness_jobs SET priority=? WHERE source_key=?',
            (index, freshness.source_key(freshness.SOURCE_ONCHAIN, f'w{index}')),
        )
    conn.commit()
    visited = []
    removed_wallet = 'w0' if stage in {'current_apply', 'dispatch'} else 'w1'

    def deprecate():
        database = conn.execute('PRAGMA database_list').fetchone()[2]
        other = open_db(Path(database).parent)
        try:
            update_wallet(other, 'ws', 'p', removed_wallet, {'config': {'deprecated': True}})
        finally:
            other.close()

    def prefetch(*args, **kwargs):
        if stage == 'prefetch':
            deprecate()
        return {}

    def sync(conn, config, workspace, profile, wallet, **kwargs):
        visited.append(wallet['id'])
        if stage in {'first_apply', 'current_apply'} and wallet['id'] == 'w0':
            deprecate()
        return {'wallet': wallet['label'], 'inserted': 0}

    original_dispatch = freshness.get_dispatchable_job

    def dispatch(*args):
        job = original_dispatch(*args)
        if stage == 'dispatch' and job and job['payload']['wallet_id'] == 'w0':
            deprecate()
        return job

    with (
        patch.object(daemon_freshness, '_prefetch_onchain_freshness_jobs', side_effect=prefetch),
        patch.object(daemon_freshness, 'sync_wallet_from_backend', side_effect=sync),
        patch.object(freshness, 'get_dispatchable_job', side_effect=dispatch),
    ):
        result = daemon_freshness._freshness_run_payload(conn, {}, {'all': True, 'rates': False, 'journals': False})
    assert visited == ({'current_apply': ['w0', 'w1', 'w2'], 'dispatch': ['w1', 'w2']}.get(stage, ['w0', 'w2']))
    assert len(result['completed']) == 2
    assert len(result['enqueued']) - len(result['completed']) == 1
    assert not conn.in_transaction
    assert freshness.get_source_state(conn, 'p', freshness.source_key(freshness.SOURCE_ONCHAIN, removed_wallet)) is None


def test_completed_selected_outcome_precedes_due_work_and_foreign_jobs(make_book):
    conn = make_book('book', 3)
    selected = daemon_freshness._freshness_run_payload(
        conn, {}, {'all': True, 'rates': False, 'journals': False, 'run': False},
    )['enqueued']
    conn.execute(
        "INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('foreign','ws','Other','EUR','2026-01-01')"
    )
    foreign = freshness.enqueue_job(
        conn, profile_id='foreign', job_type=freshness.JOB_ONCHAIN_WALLET,
        source_type=freshness.SOURCE_ONCHAIN, source_key='onchain_wallet:foreign', source_label='Foreign',
    )
    handlers = {freshness.JOB_ONCHAIN_WALLET: lambda *args: {'status': 'synced'}}
    done = freshness.run_job(conn, selected[-1]['id'], handlers)
    with patch.object(freshness, 'run_job') as run:
        result = daemon_freshness._run_requested_freshness_jobs(
            conn, 'p', [foreign, *selected], handlers, limit=1,
        )
    assert [job['id'] for job in result] == [done['id']]
    run.assert_not_called()


def test_selected_handler_not_found_is_still_a_failed_job(make_book):
    from kassiber.errors import AppError

    conn = make_book('book')
    with (
        patch.object(daemon_freshness, '_prefetch_onchain_freshness_jobs', return_value={}),
        patch.object(daemon_freshness, 'sync_wallet_from_backend', side_effect=AppError('Missing import detail', code='not_found')),
    ):
        result = daemon_freshness._freshness_run_payload(conn, {}, {'all': True, 'rates': False, 'journals': False})
    assert result['ok'] is False
    assert result['completed'][0]['status'] == freshness.JOB_ERROR
    assert result['completed'][0]['error']['code'] == 'not_found'
    assert not conn.in_transaction


def test_removed_job_rolls_back_uncommitted_handler_changes(make_book):
    conn = make_book('book')
    job = daemon_freshness._freshness_run_payload(
        conn, {}, {'all': True, 'rates': False, 'journals': False, 'run': False},
    )['enqueued'][0]

    def handler(connection, *_args):
        database = connection.execute('PRAGMA database_list').fetchone()[2]
        other = open_db(Path(database).parent)
        try:
            update_wallet(other, 'ws', 'p', 'w0', {'config': {'deprecated': True}})
        finally:
            other.close()
        connection.execute("UPDATE profiles SET label='partial work' WHERE id='p'")
        return {}

    result = freshness.run_job(conn, job['id'], {freshness.JOB_ONCHAIN_WALLET: handler}, missing_ok=True)
    assert result is None
    assert not conn.in_transaction
    assert conn.execute("SELECT label FROM profiles WHERE id='p'").fetchone()[0] == 'book'
    assert freshness.get_source_state(conn, 'p', job['source_key']) is None
