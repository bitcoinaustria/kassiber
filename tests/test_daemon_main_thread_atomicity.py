"""Queued AI operations cannot leak writes or locks into the next request."""
import queue
import sqlite3
from types import SimpleNamespace

import pytest

from kassiber import daemon
from kassiber.cli import handlers
from kassiber.db import open_db
from tests.test_custody_authoring_atomicity import _refuse_version, _snapshot
from tests.test_custody_authored_migration import NOW, _scope, _tx


@pytest.mark.parametrize('operation', ['payout', 'callback', 'cancelled_callback'])
def test_failed_main_thread_task_releases_writer_and_discards_writes(tmp_path, monkeypatch, operation):
    conn = open_db(tmp_path)
    try:
        _scope(conn)
        _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
        conn.commit()
        before = _snapshot(conn)
        _refuse_version(conn, 2)
        failure = KeyboardInterrupt('cancelled') if operation == 'cancelled_callback' else RuntimeError('late callback')

        def callback(shared):
            if operation == 'payout':
                return daemon._ui_swap_matching_payload_from_conn(shared, 'ui.transfers.payouts.create',
                    dict(workspace='Books', profile='Book', tx_out='out', payout_asset='EUR',
                         payout_amount='1', payout_fiat_value='10', policy='taxable'), authored_source='ai_tool')
            shared.execute("UPDATE profiles SET label='partial' WHERE id='profile'")
            daemon._GRAPH_SYNC_FOLLOWUPS.get().append(object())
            raise failure

        response = queue.Queue()
        tasks = queue.Queue()
        tasks.put(daemon._DaemonMainThreadTask(callback, response))
        followups = []
        monkeypatch.setattr(daemon, '_start_freshness_background_worker', lambda *a, **kw: followups.append(kw))
        daemon._drain_daemon_main_thread_tasks(SimpleNamespace(conn=conn, main_thread_tasks=tasks))
        ok, error = response.get_nowait()
        assert not ok
        assert isinstance(error, sqlite3.IntegrityError) if operation == 'payout' else error is failure
        assert not conn.in_transaction
        assert _snapshot(conn) == before
        assert not followups
        # A second writer must acquire the lock without waiting for a later request.
        with sqlite3.connect(tmp_path / 'kassiber.sqlite3', timeout=0) as other:
            other.execute('BEGIN IMMEDIATE')
        conn.execute('DROP TRIGGER temp.refuse_invalidation')
        handlers.create_transfer_rule(conn, 'ws', 'profile')
        assert conn.execute('SELECT COUNT(*) FROM custody_components').fetchone()[0] == 0
    finally:
        conn.close()


def test_failed_rollback_still_answers_waiting_worker(tmp_path, monkeypatch):
    """drain_edge.py: a callback closes the shared connection before failing."""
    conn = open_db(tmp_path)
    failure = RuntimeError('callback failed after closing the database')
    response = queue.Queue(maxsize=1)
    tasks = queue.Queue()
    def callback(shared):
        daemon._GRAPH_SYNC_FOLLOWUPS.get().append(object())
        shared.close()
        raise failure
    tasks.put(daemon._DaemonMainThreadTask(callback, response, request_id='queued'))
    followups = []
    monkeypatch.setattr(daemon, '_start_freshness_background_worker', lambda *a, **kw: followups.append(kw))
    previous_request = daemon.current_request_id.get()
    previous_graph_context = daemon._GRAPH_SYNC_FOLLOWUPS.get()
    with pytest.raises(sqlite3.ProgrammingError, match='closed database'):
        daemon._drain_daemon_main_thread_tasks(SimpleNamespace(conn=conn, main_thread_tasks=tasks))
    # A real worker uses response.get() with no timeout. Never strand it here.
    assert response.get_nowait() == (False, failure)
    assert daemon.current_request_id.get() == previous_request
    assert daemon._GRAPH_SYNC_FOLLOWUPS.get() is previous_graph_context
    assert not followups
