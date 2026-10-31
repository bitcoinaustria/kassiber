"""Contracts for disposable whole-stage parity operands."""
import json
import sqlite3

from tests.rust_port.journal_pipeline_oracle import capture
from tests.rust_port.journal_prebuild_campaign import PATH, frozen, packed
from tests.rust_port.journal_prebuild_oracle import seed, transaction, rate
from tests.rust_port.journal_persistence_oracle import dump


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
