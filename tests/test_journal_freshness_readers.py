"""Every retained-projection reader honors the canonical semantics stamp."""
import pytest

from kassiber.core import custody_journal
from kassiber.core.chain_analysis import analyze_snapshot, build_index, run_analysis
from kassiber.core.chain_analysis_ai import project_ai_result
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, open_db
from kassiber.diagnostics import _profile_summary
from tests.test_chain_analysis import add_tx, connection
from tests.test_custody_authored_migration import NOW, _scope, _tx


@pytest.mark.parametrize('columns', [True, False])
@pytest.mark.parametrize('reader', ['snapshot', 'incremental', 'ai'])
def test_chain_readers_withhold_current_custody_without_a_stamp(columns, reader):
    conn = connection()
    try:
        if columns:
            conn.execute('UPDATE profiles SET required_journal_semantics_version=1')
        else:
            conn.execute('ALTER TABLE profiles DROP COLUMN required_journal_semantics_version')
            conn.execute('ALTER TABLE profiles DROP COLUMN journal_semantics_version')
        add_tx(conn, 1)
        add_tx(conn, 2, [(1, 0)])
        conn.execute("INSERT INTO journal_custody_decisions VALUES('p','move','1','2',0,1000,'BTC','BTC','internal_reviewed','carried',NULL)")
        conn.execute("UPDATE profiles SET last_processed_at='now', last_processed_tx_count=2")
        if columns:
            conn.execute('UPDATE profiles SET journal_semantics_version=?', (JOURNAL_CALCULATION_SEMANTICS_VERSION,))
            assert build_index(conn, 'p').coverage['custody_fresh']
            assert run_analysis(conn, 'p', {})['coverage']['custody_fresh']
            # An older writer publishes markers with a zero semantics stamp.
            conn.execute('UPDATE profiles SET journal_semantics_version=0')
        result = analyze_snapshot(build_index(conn, 'p'), {}) if reader == 'snapshot' else run_analysis(conn, 'p', {})
        if reader == 'ai':
            result = project_ai_result(conn, 'p', result)
        assert not result['coverage']['custody_fresh']
        edges = [edge for edge in result['edges'] if edge['kind'] == 'custody']
        assert edges and all(edge['status'] == 'stale' for edge in edges)
        assert any(finding['code'] == 'custody_projection_stale' for finding in result['findings'])
    finally:
        conn.close()


def test_diagnostics_counts_unstamped_book_as_stale(tmp_path):
    conn = open_db(tmp_path)
    try:
        _scope(conn)
        _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
        conn.execute("""UPDATE profiles SET last_processed_at=?, last_processed_tx_count=1,
            last_processed_input_version=journal_input_version WHERE id='profile'""", (NOW,))
        assert _profile_summary(conn)['processed'] == 0
        assert _profile_summary(conn)['stale_or_unprocessed'] == 1
        conn.execute('UPDATE profiles SET journal_semantics_version=?', (JOURNAL_CALCULATION_SEMANTICS_VERSION,))
        assert _profile_summary(conn)['processed'] == 1
    finally:
        conn.close()


@pytest.mark.parametrize('change', [{}, {'journal_semantics_version': 0},
    {'last_processed_tx_count': 0}, {'last_processed_input_version': -1},
    {'last_processed_at': None}, {'required_journal_semantics_version': 999}])
def test_sql_predicate_matches_shared_evaluator(tmp_path, change):
    conn = open_db(tmp_path)
    try:
        _scope(conn)
        _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
        conn.execute("""UPDATE profiles SET last_processed_at=?, last_processed_tx_count=1,
            last_processed_input_version=journal_input_version WHERE id='profile'""", (NOW,))
        conn.execute('UPDATE profiles SET journal_semantics_version=?', (JOURNAL_CALCULATION_SEMANTICS_VERSION,))
        for key, value in change.items():
            conn.execute(f'UPDATE profiles SET {key}=?', (value,))
        predicate = custody_journal.projection_freshness_sql('p')
        actual = conn.execute(f"SELECT {predicate} FROM profiles p WHERE id='profile'").fetchone()[0]
        assert bool(actual) == custody_journal.projection_freshness(conn, 'profile')['is_current']
    finally:
        conn.close()
