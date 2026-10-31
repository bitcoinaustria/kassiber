"""First-upgrade adoption must work independently of other semantic migrations."""
import json
import sqlite3

import pytest

from kassiber.cli.handlers import process_journals
from kassiber.core.custody_journal import projection_freshness
from kassiber.db import open_db
from tests.test_custody_authored_migration import NOW, _scope, _tx


MIGRATION = 'journal-calculation-semantics-adoption-v1'


@pytest.mark.parametrize('drop_columns', [True, False])
@pytest.mark.parametrize('retained_only', [True, False])
def test_adoption_requires_stamped_rebuild_and_records_one_audit(tmp_path, drop_columns, retained_only):
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, 'buy', 'btc', 'inbound', 'BTC', 100_000_000_000, NOW)
    conn.execute("UPDATE transactions SET kind='buy', fiat_rate=100, fiat_value=100 WHERE id='buy'")
    conn.execute("""INSERT INTO profiles(id, workspace_id, label, fiat_currency, gains_algorithm, created_at)
        VALUES('empty', 'ws', 'Unprocessed', 'EUR', 'FIFO', ?)""", (NOW,))
    conn.commit()
    process_journals(conn, 'Books', 'Book')
    assert projection_freshness(conn, 'profile')['is_current']
    if retained_only:
        conn.execute("UPDATE profiles SET last_processed_at=NULL WHERE id='profile'")
    retained = [tuple(row) for row in conn.execute('SELECT * FROM journal_entries')]
    assert retained
    raw = [tuple(row) for row in conn.execute('SELECT * FROM transactions')]
    markers = tuple(conn.execute("""SELECT last_processed_at, last_processed_tx_count,
        journal_input_version, last_processed_input_version FROM profiles WHERE id='profile'""").fetchone())
    conn.commit()
    conn.close()

    # All earlier migration audits remain present: none can mask disabled adoption.
    with sqlite3.connect(tmp_path / 'kassiber.sqlite3') as legacy:
        legacy.execute('DELETE FROM schema_migration_audits WHERE migration_name=?', (MIGRATION,))
        if drop_columns:
            legacy.execute('DROP TRIGGER trg_journal_semantics_unstamped')
            legacy.execute('ALTER TABLE profiles DROP COLUMN required_journal_semantics_version')
            legacy.execute('ALTER TABLE profiles DROP COLUMN journal_semantics_version')
        else:
            # Also adopt databases opened by the earlier unaudited implementation.
            legacy.execute('UPDATE profiles SET required_journal_semantics_version=0, journal_semantics_version=0')

    for _ in range(2):
        migrated = open_db(tmp_path)
        try:
            assert not projection_freshness(migrated, 'profile')['is_current']
            stamps = tuple(migrated.execute("""SELECT required_journal_semantics_version,
                journal_semantics_version FROM profiles WHERE id='profile'""").fetchone())
            assert stamps == (1, 0)
            assert migrated.execute("SELECT required_journal_semantics_version FROM profiles WHERE id='empty'").fetchone()[0] == 0
            assert tuple(migrated.execute("""SELECT last_processed_at, last_processed_tx_count,
                journal_input_version, last_processed_input_version FROM profiles WHERE id='profile'""").fetchone()) == markers
            assert [tuple(row) for row in migrated.execute('SELECT * FROM journal_entries')] == retained
            assert [tuple(row) for row in migrated.execute('SELECT * FROM transactions')] == raw
            audits = migrated.execute('SELECT impact_json FROM schema_migration_audits WHERE migration_name=?', (MIGRATION,)).fetchall()
            assert len(audits) == 1
            impact = json.loads(audits[0][0])
            assert impact['affected_profile_count'] == 1
            assert impact['required_journal_semantics_version'] == 1
            assert 'buy' not in audits[0][0]
        finally:
            migrated.close()

    rebuilt = open_db(tmp_path)
    try:
        process_journals(rebuilt, 'Books', 'Book')
        assert projection_freshness(rebuilt, 'profile')['is_current']
    finally:
        rebuilt.close()
    reopened = open_db(tmp_path)
    try:
        assert projection_freshness(reopened, 'profile')['is_current']
        assert reopened.execute('SELECT COUNT(*) FROM schema_migration_audits WHERE migration_name=?', (MIGRATION,)).fetchone()[0] == 1
    finally:
        reopened.close()
