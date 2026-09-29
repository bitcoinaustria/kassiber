"""Sync grants, bounded lookups, and local-only graph reads."""
import io
import json
import queue
import threading
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import daemon, daemon_freshness as df
from kassiber.backends import create_db_backend, merge_db_backends
from kassiber.core import freshness, transaction_graph as graph, wallet_graph_references as refs
from kassiber.db import set_setting
from kassiber.errors import AppError
from tests import test_transaction_graph as fixtures


@pytest.fixture
def book():
    book = fixtures.TransactionGraphTest()
    book.setUp()
    create_db_backend(book.conn, 'own', 'esplora', 'https://own.invalid/api', chain='bitcoin', network='main')
    book.conn.execute(
        "UPDATE wallets SET kind='address', config_json=? WHERE id='wallet-a'",
        (json.dumps({'backend': 'own', 'addresses': [fixtures.ADDR_A], 'chain': 'bitcoin', 'network': 'main'}),),
    )
    book.conn.commit()
    book.runtime = {'backends': {}, 'default_backend': 'own'}
    merge_db_backends(book.conn, book.runtime)
    with patch.object(df, "_prefetch_onchain_freshness_jobs", return_value={}):
        yield book
    book.tearDown()


def raw(txid, parent='b' * 64):
    return {'txid': txid, 'vin': [{'txid': parent, 'vout': 0, 'prevout': {'value': 100000, 'scriptpubkey': fixtures.SCRIPT_B}}],
            'vout': [{'value': 99000, 'scriptpubkey': fixtures.SCRIPT_A}]}


def add(book, index=1, *, wallet='wallet-a', payload=None, asset='BTC'):
    txid = f'{index:064x}'
    book._tx(f'row-{index}', wallet, 'inbound', 99000000, txid,
             payload if payload is not None else {'txid': txid}, asset=asset)
    book.conn.commit()
    return txid


def sync(book, *, automatic_trigger=None):
    grants = []
    token = df._GRAPH_SYNC_FOLLOWUPS.set(grants)
    try:
        with patch.object(df, 'sync_wallet_from_backend', return_value={'inserted': 0}):
            result = df._wallets_sync_payload(
                book.conn, book.runtime, {'wallet': 'Cold', 'process_journals': False},
                strict=True, automatic_trigger=automatic_trigger,
            )
        return grants, result
    finally:
        df._GRAPH_SYNC_FOLLOWUPS.reset(token)


class Out:
    def __init__(self):
        self.events = []

    def write(self, event):
        self.events.append(event)


def follow(book, grant, stop=None):
    out = Out()
    df._run_graph_followup(book.conn, book.runtime, out, grant, stop or threading.Event())
    return out.events


def test_sync_returns_before_followup_and_cache_makes_display_offline(book):
    txid = add(book)
    with patch.object(graph, '_fetch_graph_esplora_transaction', return_value=raw(txid)) as fetch:
        grants, result = sync(book)
        assert result['completed'][0]['status'] == 'done'
        fetch.assert_not_called()
        assert len(grants) == 1
        assert not any(j['job_type'] == freshness.JOB_GRAPH_REFERENCES for j in freshness.list_jobs(book.conn, 'profile-1'))
        events = follow(book, grants[0])
        fetch.assert_called_once()
        assert fetch.call_args.args[0]['url'] == 'https://own.invalid/api'
    with patch.object(graph, '_fetch_reference_graph_from_backend', side_effect=AssertionError('display egress')):
        assert book._graph('row-1')['supportLevel'] == 'full'
        second = follow(book, grants[0])
    job = events[0]['data']['completed'][0]
    assert job['job_type'] == freshness.JOB_GRAPH_REFERENCES
    assert job['result'] == {'scanned': 1, 'attempted': 1, 'cached': 1, 'skipped': 0, 'failed': 0, 'prevouts_requested': 0}
    assert second[0]['data']['completed'][0]['result']['attempted'] == 0
    assert events[0]['event'] is True and 'request_id' not in events[0]
    assert txid not in json.dumps(job)


def test_transaction_cap_and_cursor_eventually_cover_leftovers(book, monkeypatch):
    monkeypatch.setattr(refs, 'MAX_TRANSACTIONS', 2)
    for n in range(1, 6):
        add(book, n)
    add(book, 20, wallet='wallet-b')
    seen = []
    def fetch(backend, txid):
        seen.append(txid)
        return raw(txid)
    with patch.object(graph, '_fetch_graph_esplora_transaction', side_effect=fetch):
        for expected in (2, 4, 5):
            grants, _ = sync(book)
            follow(book, grants[0])
            assert len(seen) == expected
    assert f'{20:064x}' not in seen


def test_failures_advance_cursor_without_falling_back(book, monkeypatch):
    monkeypatch.setattr(refs, 'MAX_TRANSACTIONS', 1)
    create_db_backend(book.conn, 'other', 'esplora', 'https://other.invalid/api', chain='bitcoin', network='main')
    add(book, 1)
    add(book, 2)
    seen = []
    def fail(backend, txid):
        assert backend['name'] == 'own'
        seen.append(txid)
        raise OSError('backend unavailable')
    with patch.object(graph, '_fetch_graph_esplora_transaction', side_effect=fail):
        for _ in range(2):
            grants, _ = sync(book)
            follow(book, grants[0])
    assert seen == [f'{1:064x}', f'{2:064x}']


def test_shared_prevout_budget_and_electrum_batching(book, monkeypatch):
    monkeypatch.setattr(refs, 'MAX_PREVOUT_FETCHES', 2)
    book.runtime['backends']['own']['kind'] = 'electrum'
    book.runtime['backends']['own']['url'] = 'ssl://own.invalid:50002'
    book.conn.execute("UPDATE backends SET kind='electrum',url='ssl://own.invalid:50002' WHERE name='own'")
    parents = ['a' * 64, 'b' * 64, 'c' * 64]
    ids = [add(book, n) for n in (1, 2)]
    decoded = {
        ids[0]: {'vin': [{'txid': p, 'vout': 0} for p in parents[:2]], 'vout': [{'value_sats': 99000, 'script_hex': fixtures.SCRIPT_A}]},
        ids[1]: {'vin': [{'txid': parents[2], 'vout': 0}], 'vout': [{'value_sats': 99000, 'script_hex': fixtures.SCRIPT_A}]},
        **{p: {'vin': [], 'vout': [{'value_sats': 100000, 'script_hex': fixtures.SCRIPT_B}]} for p in parents},
    }
    class Client(fixtures._FakeElectrumClient):
        responses = {txid: txid for txid in decoded}
        calls = []
        batches = []
        def batch_call(self, requests):
            self.batches.append(requests)
            return super().batch_call(requests)
    with patch.object(graph, 'ElectrumClient', Client), patch.object(graph, 'decode_raw_transaction', side_effect=lambda value: decoded[value]):
        grants, _ = sync(book)
        events = follow(book, grants[0])
    assert len(Client.batches) == 1 and len(Client.batches[0]) == 2
    assert len(Client.calls) == 4  # two current transactions plus two parents
    assert events[0]['data']['completed'][0]['result']['prevouts_requested'] == 2
    with patch.object(graph, 'ElectrumClient', side_effect=AssertionError('display egress')):
        assert book._graph('row-1')['supportLevel'] == 'full'


def test_budget_refused_row_is_first_in_the_next_pass(book, monkeypatch):
    # The first row takes the whole prevout budget and its parents keep failing;
    # the second row must still get its own pass instead of being skipped forever.
    monkeypatch.setattr(refs, 'MAX_PREVOUT_FETCHES', 2)
    book.runtime['backends']['own']['kind'] = 'electrum'
    book.runtime['backends']['own']['url'] = 'ssl://own.invalid:50002'
    book.conn.execute("UPDATE backends SET kind='electrum',url='ssl://own.invalid:50002' WHERE name='own'")
    parents = ['a' * 64, 'b' * 64, 'c' * 64]
    ids = [add(book, n) for n in (1, 2)]
    decoded = {
        ids[0]: {'vin': [{'txid': p, 'vout': 0} for p in parents[:2]], 'vout': [{'value_sats': 99000, 'script_hex': fixtures.SCRIPT_A}]},
        ids[1]: {'vin': [{'txid': parents[2], 'vout': 0}], 'vout': [{'value_sats': 99000, 'script_hex': fixtures.SCRIPT_A}]},
        parents[2]: {'vin': [], 'vout': [{'value_sats': 100000, 'script_hex': fixtures.SCRIPT_B}]},
    }
    class Client(fixtures._FakeElectrumClient):
        responses = {txid: txid for txid in (ids[0], ids[1], parents[2])}
        calls = []
        def batch_call(self, requests):
            if any(params and params[0] in parents[:2] for _method, params in requests):
                raise OSError('parents unavailable')
            return super().batch_call(requests)
    with patch.object(graph, 'ElectrumClient', Client), patch.object(graph, 'decode_raw_transaction', side_effect=lambda value: decoded[value]):
        for _ in range(2):
            grants, _ = sync(book)
            follow(book, grants[0])
    with patch.object(graph, 'ElectrumClient', side_effect=AssertionError('display egress')):
        assert book._graph('row-2')['supportLevel'] == 'full'


def test_liquid_local_graph_is_preserved(book):
    local = raw('1' * 64)
    add(book, payload=local, asset='LBTC')
    book.conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (json.dumps({'backend': 'own', 'chain': 'liquid', 'network': 'liquidv1', 'addresses': [fixtures.ADDR_A]}),))
    with patch.object(graph, '_fetch_reference_graph_from_backend', side_effect=AssertionError('Liquid egress')):
        grants, _ = sync(book)
        events = follow(book, grants[0])
    assert events[0]['data']['completed'][0]['result']['skipped'] == 1
    assert json.loads(book.conn.execute("SELECT raw_json FROM transactions").fetchone()[0]) == local


@pytest.mark.parametrize('action', ['display', 'status', 'idle', 'offline_sync', 'uncollected_sync'])
def test_no_grants_from_passive_or_offline_paths(book, monkeypatch, action):
    add(book)
    grants = []
    token = df._GRAPH_SYNC_FOLLOWUPS.set(grants)
    try:
        with patch.object(graph, '_fetch_reference_graph_from_backend', side_effect=AssertionError('egress')):
            if action == 'display':
                book._graph('row-1')
            elif action == 'status':
                df._freshness_status_payload(book.conn)
            elif action == 'idle':
                df._freshness_background_tick(book.conn, book.runtime, Out())
            elif action == 'offline_sync':
                monkeypatch.setenv('KASSIBER_NO_EGRESS', '1')
                captured, _ = sync(book)
                assert captured == []
            else:
                df._GRAPH_SYNC_FOLLOWUPS.set(None)
                with patch.object(df, 'sync_wallet_from_backend', return_value={}):
                    df._wallets_sync_payload(book.conn, book.runtime, {'wallet': 'Cold', 'process_journals': False}, strict=True)
        assert grants == []
        assert not any(j['job_type'] == freshness.JOB_GRAPH_REFERENCES for j in freshness.list_jobs(book.conn, 'profile-1'))
    finally:
        df._GRAPH_SYNC_FOLLOWUPS.reset(token)


@pytest.mark.parametrize('revoke', ['stop', 'book', 'backend', 'wallet', 'offline', 'cancel_sync'])
def test_grants_are_rechecked_before_enqueue(book, monkeypatch, revoke):
    add(book)
    grants, _ = sync(book)
    stop = threading.Event()
    if revoke == 'stop':
        stop.set()
    elif revoke == 'book':
        set_setting(book.conn, 'context_profile', 'other-book')
    elif revoke == 'backend':
        book.runtime['backends']['own']['url'] = 'https://changed.invalid'
    elif revoke == 'wallet':
        book.conn.execute("UPDATE wallets SET config_json='{}' WHERE id='wallet-a'")
    elif revoke == 'offline':
        monkeypatch.setenv('KASSIBER_NO_EGRESS', '1')
    else:
        book.conn.execute('UPDATE freshness_jobs SET cancel_requested=1 WHERE id=?', (grants[0]['sync_job_id'],))
    book.conn.commit()
    with patch.object(graph, '_fetch_reference_graph_from_backend', side_effect=AssertionError('egress')):
        assert follow(book, grants[0], stop) == []


def test_moving_the_inherited_default_revokes_the_grant(book):
    # The wallet syncs through the chosen default rather than naming a backend.
    create_db_backend(book.conn, 'other', 'esplora', 'https://other.invalid/api', chain='bitcoin', network='main')
    book.conn.execute(
        "UPDATE wallets SET config_json=? WHERE id='wallet-a'",
        (json.dumps({'addresses': [fixtures.ADDR_A], 'chain': 'bitcoin', 'network': 'main'}),),
    )
    book.conn.commit()
    merge_db_backends(book.conn, book.runtime)
    add(book)
    grants, _ = sync(book)
    assert grants and grants[0]['backend_name'] == 'own'
    # Choose another default; the old backend stays configured and unchanged.
    book.runtime['default_backend'] = 'other'
    set_setting(book.conn, 'default_backend', 'other')
    book.conn.commit()
    with patch.object(graph, '_fetch_reference_graph_from_backend', side_effect=AssertionError('egress')):
        assert follow(book, grants[0]) == []


@pytest.mark.parametrize('revoke', ['cancel', 'book', 'stop'])
def test_mid_request_revocation_prevents_cache_and_later_requests(book, revoke):
    txid = add(book)
    add(book, 2)
    grants, _ = sync(book)
    stop = threading.Event()
    def fetch(backend, requested):
        if revoke == 'cancel':
            job = next(j for j in freshness.list_jobs(book.conn, 'profile-1') if j['job_type'] == freshness.JOB_GRAPH_REFERENCES)
            freshness.cancel_job(book.conn, job['id'])
        elif revoke == 'book':
            set_setting(book.conn, 'context_profile', 'other-book')
        else:
            stop.set()
        book.conn.commit()
        return raw(txid)
    with patch.object(graph, '_fetch_graph_esplora_transaction', side_effect=fetch) as fetcher:
        events = follow(book, grants[0], stop)
    fetcher.assert_called_once()
    assert events[0]['data']['completed'][0]['status'] == 'cancelled'
    assert freshness.report_blocking_source_summary(book.conn, 'profile-1')['count'] == 0
    assert book.conn.execute('SELECT count(*) FROM transaction_graph_cache').fetchone()[0] == 0


def test_recovery_does_not_steal_running_graph_job(book):
    job = freshness.enqueue_job(book.conn, profile_id='profile-1', job_type=freshness.JOB_GRAPH_REFERENCES,
                                source_key='wallet_graph:wallet-a', source_type=freshness.SOURCE_ONCHAIN, source_label='Graph')
    book.conn.execute("UPDATE freshness_jobs SET status='running' WHERE id=?", (job['id'],))
    assert freshness.recover_interrupted_jobs(book.conn, profile_id='profile-1') == []
    assert freshness.get_job(book.conn, job['id'])['status'] == 'running'
    # A persisted row has no authority when picked up through an idle handler.
    result = freshness.run_job(book.conn, job['id'], df._freshness_handlers(book.runtime))
    assert result['status'] == 'cancelled'


def test_worker_does_not_block_sync_or_a_subsequent_sync(book):
    txid = add(book)
    grants, _ = sync(book)
    entered, release = threading.Event(), threading.Event()
    ctx = SimpleNamespace(conn=book.conn, data_root=str(book.conn.execute('PRAGMA database_list').fetchone()[2]).rsplit('/', 1)[0],
                          runtime_config=book.runtime, out=Out(), freshness_stop_event=threading.Event(),
                          freshness_worker=None, db_passphrase=None, graph_followups=queue.Queue())
    def fetch(*args):
        entered.set()
        assert release.wait(3)
        return raw(txid)
    try:
        with patch.object(df, 'FRESHNESS_BACKGROUND_POLL_SECONDS', 0.01), patch.object(graph, '_fetch_graph_esplora_transaction', side_effect=fetch):
            df._start_freshness_background_worker(ctx, graph_followups=grants)
            assert entered.wait(3)
            more, result = sync(book)
            assert result['completed'][0]['status'] == 'done'
            assert not release.is_set()
            release.set()
    finally:
        release.set()
        df._stop_freshness_background_worker(ctx)


def test_daemon_publishes_grant_only_after_sync_response(book):
    add(book)
    published = []
    output = io.StringIO()
    def start(ctx, **kwargs):
        grants = kwargs.get('graph_followups') or []
        if grants:
            records = [json.loads(line) for line in output.getvalue().splitlines()]
            assert any(r.get('kind') == 'ui.wallets.sync' and r.get('request_id') == 'sync' for r in records)
            published.extend(grants)
    args = SimpleNamespace(data_root=book.tmp.name, runtime_config=book.runtime)
    request = {'kind': 'ui.wallets.sync', 'request_id': 'sync', 'args': {'wallet': 'Cold', 'process_journals': False}}
    with patch.object(df, 'sync_wallet_from_backend', return_value={}), patch.object(daemon, '_start_freshness_background_worker', side_effect=start), patch.object(daemon, '_ensure_daemon_project_owner'), patch.object(daemon, '_start_watch_worker'):
        assert daemon.run(book.conn, args, stdin=io.StringIO(json.dumps(request) + '\n'), stdout=output) == 0
    assert len(published) == 1


def test_new_sync_replaces_interrupted_graph_job(book):
    txid = add(book)
    grants, _ = sync(book)
    job = freshness.enqueue_job(book.conn, profile_id='profile-1', job_type=freshness.JOB_GRAPH_REFERENCES,
                                source_key='wallet_graph:wallet-a', source_type=freshness.SOURCE_ONCHAIN, source_label='Graph')
    book.conn.execute("UPDATE freshness_jobs SET status='running' WHERE id=?", (job['id'],))
    book.conn.commit()
    with patch.object(graph, '_fetch_graph_esplora_transaction', return_value=raw(txid)):
        events = follow(book, grants[0])
    assert freshness.get_job(book.conn, job['id'])['status'] == 'cancelled'
    assert events[0]['data']['completed'][0]['status'] == 'done'
    assert freshness.report_blocking_source_summary(book.conn, 'profile-1')['count'] == 0


@pytest.mark.parametrize('trigger', ['background', 'report_read'])
def test_automatic_sync_grant_honors_revocation(book, trigger):
    txid = add(book)
    freshness.set_policy(book.conn, 'profile-1', background_enabled=True, report_read_sync=True,
                         source_classes={freshness.SOURCE_ONCHAIN: True})
    grants, _ = sync(book, automatic_trigger=trigger)
    with patch.object(graph, '_fetch_graph_esplora_transaction', return_value=raw(txid)) as fetch:
        assert follow(book, grants[0])
        fetch.assert_called_once()
    grants, _ = sync(book, automatic_trigger=trigger)
    freshness.set_policy(book.conn, 'profile-1', background_enabled=False, report_read_sync=False)
    with patch.object(graph, '_fetch_reference_graph_from_backend', side_effect=AssertionError('revoked egress')):
        assert follow(book, grants[0]) == []


def test_seeded_default_does_not_grant_lookup(book):
    from kassiber.backends import BOOTSTRAP_DEFAULT_BACKEND_SETTING, DEFAULT_BACKEND_SETTING
    book.conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (json.dumps({'addresses': [fixtures.ADDR_A]}),))
    set_setting(book.conn, DEFAULT_BACKEND_SETTING, 'own')
    set_setting(book.conn, BOOTSTRAP_DEFAULT_BACKEND_SETTING, 'own')
    book.runtime['default_backend'] = 'own'
    book.runtime['default_backend_source'] = 'built-in default'
    book.runtime['bootstrap_default_backend'] = 'own'
    grants, _ = sync(book)
    assert grants == []


def test_book_network_mismatch_does_not_fetch(book):
    add(book, payload={'network': 'regtest'})
    grants, _ = sync(book)
    with patch.object(graph, '_fetch_reference_graph_from_backend', side_effect=AssertionError('wrong network')):
        events = follow(book, grants[0])
    assert events[0]['data']['completed'][0]['result']['skipped'] == 1
