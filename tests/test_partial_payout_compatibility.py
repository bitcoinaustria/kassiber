"""Partial-payout semantic upgrade and exact historical journal regression."""
import json
from decimal import Decimal

import pytest

from kassiber.cli.handlers import create_direct_swap_payout, process_journals
from kassiber.core.custody_review_terms import delete_payout_review
from kassiber.db import open_db, PARTIAL_PAYOUT_SEMANTICS_MIGRATION
from tests import cli_runner
from tests.test_custody_authored_migration import _scope, _tx, NOW


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
    conn.commit()
    yield conn
    conn.close()


@pytest.mark.parametrize('partial,deleted', [(True, False), (False, False), (True, True)])
def test_partial_payout_migration_is_targeted_once(book, tmp_path, partial, deleted):
    payout = create_direct_swap_payout(book, 'Books', 'Book', 'out', payout_asset='EUR', payout_amount='1',
                                     payout_fiat_value='10', out_amount='0.0000004' if partial else None)
    if deleted:
        delete_payout_review(book, 'profile', payout['id'])
    book.execute("INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at, last_processed_at) VALUES('unrelated', 'ws', 'Other', 'EUR', ?, ?)", (NOW, NOW))
    book.execute("UPDATE profiles SET last_processed_at=? WHERE id='profile'", (NOW,))
    book.execute('DELETE FROM schema_migration_audits WHERE migration_name=?', (PARTIAL_PAYOUT_SEMANTICS_MIGRATION,))
    version = book.execute("SELECT journal_input_version FROM profiles WHERE id='profile'").fetchone()[0]
    book.commit()
    conn = open_db(tmp_path)
    try:
        affected = partial and not deleted
        assert conn.execute("SELECT last_processed_at FROM profiles WHERE id='profile'").fetchone()[0] == (None if affected else NOW)
        assert conn.execute("SELECT last_processed_at FROM profiles WHERE id='unrelated'").fetchone()[0] == NOW
        expected = version + int(affected)
        assert conn.execute("SELECT journal_input_version FROM profiles WHERE id='profile'").fetchone()[0] == expected
        audit = conn.execute('SELECT impact_json FROM schema_migration_audits WHERE migration_name=?', (PARTIAL_PAYOUT_SEMANTICS_MIGRATION,)).fetchone()[0]
        assert json.loads(audit)['affected_profile_count'] == int(affected)
    finally:
        conn.close()
    conn = open_db(tmp_path)
    assert conn.execute("SELECT journal_input_version FROM profiles WHERE id='profile'").fetchone()[0] == expected
    conn.close()


def test_pre_fix_historical_book_requires_rebuild(tmp_path):
    """Exact ca9a672f book: old process stored 20 rather than 11.2 proceeds."""
    import base64
    import gzip
    import sqlite3
    from pathlib import Path

    fixture = Path(__file__).parent / 'fixtures/historical/pre_partial_payout_ca9a672f.sqlite3.gz.b64'
    database = tmp_path / 'kassiber.sqlite3'
    database.write_bytes(gzip.decompress(base64.decodebytes(fixture.read_bytes())))
    with sqlite3.connect(database) as before:
        assert before.execute("SELECT last_processed_at FROM profiles WHERE id='profile'").fetchone()[0]
        assert sum(Decimal(row[0]) for row in before.execute("SELECT proceeds_exact FROM journal_entries WHERE entry_type='disposal'")) == 20
    conn = open_db(tmp_path)
    try:
        assert conn.execute("SELECT last_processed_at FROM profiles WHERE id='profile'").fetchone()[0] is None
        command = ['--machine', '--data-root', str(tmp_path),
                   'reports', 'capital-gains', '--workspace', 'Books', '--profile', 'Book']
        stale = json.loads(cli_runner.run(command, capture_output=True, text=True).stdout)
        assert stale['kind'] == 'error'
        assert 'fresh journals' in stale['error']['message']
        process_journals(conn, 'Books', 'Book')
        report = json.loads(cli_runner.run(command, capture_output=True, text=True).stdout)
        assert report['kind'] != 'error'
        assert sum(Decimal(str(row['proceeds'])) for row in report['data']) == Decimal('11.2')
        assert conn.execute("PRAGMA integrity_check").fetchone()[0] == 'ok'
    finally:
        conn.close()


def test_whole_principal_payout_with_included_fee_stays_current(book, tmp_path):
    book.execute("UPDATE transactions SET fee=1000, amount_includes_fee=1 WHERE id='out'")
    book.commit()
    create_direct_swap_payout(book, 'Books', 'Book', 'out', payout_asset='EUR', payout_amount='1', payout_fiat_value='10')
    book.execute("UPDATE profiles SET last_processed_at=? WHERE id='profile'", (NOW,))
    book.execute('DELETE FROM schema_migration_audits WHERE migration_name=?', (PARTIAL_PAYOUT_SEMANTICS_MIGRATION,))
    book.commit()
    conn = open_db(tmp_path)
    try:
        assert conn.execute("SELECT last_processed_at FROM profiles WHERE id='profile'").fetchone()[0] == NOW
    finally:
        conn.close()


@pytest.mark.parametrize('rebuild_current_first', [False, True])
def test_older_rebuild_cannot_reuse_current_semantics_stamp(tmp_path, rebuild_current_first):
    import base64
    import gzip
    from pathlib import Path
    from kassiber.core.custody_journal import projection_freshness
    from kassiber.core import transaction_graph
    from kassiber.cli import handlers

    fixture = Path(__file__).parent / 'fixtures/historical/pre_partial_payout_ca9a672f.sqlite3.gz.b64'
    (tmp_path / 'kassiber.sqlite3').write_bytes(gzip.decompress(base64.decodebytes(fixture.read_bytes())))
    conn = open_db(tmp_path)
    cache = {}
    projection_sql = handlers._transaction_custody_projection_exists_sql()
    if rebuild_current_first:
        process_journals(conn, 'Books', 'Book')
        assert projection_freshness(conn, 'profile')['is_current']
        assert transaction_graph._load_profile_semantics(conn, 'profile', cache=cache).semantics['projection_current']
        assert conn.execute(f"SELECT COUNT(*) FROM transactions t WHERE {projection_sql}").fetchone()[0] > 0
    # The exact historical builder's publication contract. Unknown additive
    # columns survive an older binary; they must not preserve a current stamp.
    for _ in range(2):
        conn.execute("""UPDATE profiles SET last_processed_at=?,
            last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='profile' AND excluded=0),
            last_processed_input_version=journal_input_version WHERE id='profile'""", (NOW,))
        conn.execute("UPDATE journal_entries SET proceeds_exact='20', proceeds=20 WHERE entry_type='disposal'")
        conn.commit()
        assert not projection_freshness(conn, 'profile')['is_current']
        profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
        assert not handlers._journal_processing_status(conn, profile)['processed_journals_current']
        assert conn.execute(f"SELECT COUNT(*) FROM transactions t WHERE {projection_sql}").fetchone()[0] == 0
        assert not transaction_graph._load_profile_semantics(conn, 'profile', cache=cache).semantics['projection_current']
    conn.close()
    conn = open_db(tmp_path)
    try:
        assert not projection_freshness(conn, 'profile')['is_current']
        command = ['--machine', '--data-root', str(tmp_path),
                   'reports', 'capital-gains', '--workspace', 'Books', '--profile', 'Book']
        stale = json.loads(cli_runner.run(command, capture_output=True, text=True).stdout)
        assert stale['kind'] == 'error'
        process_journals(conn, 'Books', 'Book')
        assert projection_freshness(conn, 'profile')['is_current']
        report = json.loads(cli_runner.run(command, capture_output=True, text=True).stdout)
        assert sum(Decimal(str(row['proceeds'])) for row in report['data']) == Decimal('11.2')
    finally:
        conn.close()


def test_first_older_build_in_new_book_requires_semantics_stamp(book):
    from kassiber.core.custody_journal import projection_freshness
    assert book.execute("SELECT required_journal_semantics_version FROM profiles WHERE id='profile'").fetchone()[0] == 0
    create_direct_swap_payout(book, 'Books', 'Book', 'out', payout_asset='EUR', payout_amount='1',
                             payout_fiat_value='10', out_amount='0.0000004')
    book.execute("""UPDATE profiles SET last_processed_at=?, last_processed_tx_count=1,
        last_processed_input_version=journal_input_version WHERE id='profile'""", (NOW,))
    book.commit()
    assert not projection_freshness(book, 'profile')['is_current']


@pytest.mark.parametrize('reader', ['desktop', 'ai', 'maintenance'])
def test_daemon_rebuilds_semantics_stale_historical_lots(tmp_path, monkeypatch, reader):
    """sem_autoproc.py: an old writer publishes matching input markers."""
    import base64
    import gzip
    import queue
    from pathlib import Path
    from types import SimpleNamespace
    from kassiber import daemon
    from kassiber.core.custody_journal import projection_freshness
    from kassiber.db import set_setting

    fixture = Path(__file__).parent / 'fixtures/historical/pre_partial_payout_ca9a672f.sqlite3.gz.b64'
    (tmp_path / 'kassiber.sqlite3').write_bytes(gzip.decompress(base64.decodebytes(fixture.read_bytes())))
    conn = open_db(tmp_path)
    try:
        workspace = conn.execute("SELECT workspace_id FROM profiles WHERE id='profile'").fetchone()[0]
        set_setting(conn, 'context_workspace', workspace)
        set_setting(conn, 'context_profile', 'profile')
        process_journals(conn, 'Books', 'Book')
        conn.execute("""UPDATE profiles SET last_processed_at=?,
            last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='profile' AND excluded=0),
            last_processed_input_version=journal_input_version WHERE id='profile'""", (NOW,))
        conn.execute("UPDATE journal_entries SET proceeds_exact='10', proceeds=10 WHERE entry_type='disposal'")
        conn.commit()
        assert not projection_freshness(conn, 'profile')['is_current']
        # A local rebuild must never use the network, including loopback.
        monkeypatch.setattr('socket.getaddrinfo', lambda *a, **kw: pytest.fail('unexpected DNS'))
        monkeypatch.setattr('socket.socket.connect', lambda *a, **kw: pytest.fail('unexpected socket'))
        if reader == 'ai':
            runtime = daemon.AiToolRuntime(str(tmp_path), {}, queue.Queue(), {
                'provider_kind': 'local', 'scope_workspace_id': workspace,
                'scope_profile_id': 'profile',
            })
            monkeypatch.setattr(daemon, '_run_on_daemon_main_thread', lambda runtime, callback: callback(conn))
            call = daemon._parse_ai_tool_call({
                'id': 'semantics', 'function': {'name': 'ui_reports_capital_gains', 'arguments': '{}'},
            }, 0)
            result = daemon._execute_read_only_ai_tool(call, runtime)
            assert result['ok'], result
            report = result['envelope']['data']
        else:
            kind = 'ui.maintenance.run' if reader == 'maintenance' else 'ui.reports.capital_gains'
            result, _ = daemon.handle_request(
                SimpleNamespace(conn=conn, data_root=str(tmp_path), runtime_config={},
                                backup_sessions=daemon.daemon_backup.BackupSessions()),
                {'request_id': 'semantics', 'kind': kind, 'args': {'sync': 'never'} if reader == 'maintenance' else {}},
                None,
            )
            assert result['kind'] == kind, result
            if reader == 'maintenance':
                assert 'journals' in result['data']['maintenance']
                report = daemon.build_capital_gains_snapshot(conn)
            else:
                report = result['data']
        assert projection_freshness(conn, 'profile')['is_current']
        assert not report['status']['needsJournals']
        # The AI projection names the (bounded) lot list `rows`.
        lots = report['rows'] if reader == 'ai' else report['lots']
        assert sum(Decimal(str(row['proceedsEur'])) for row in lots) == Decimal('11.2')
    finally:
        conn.close()
