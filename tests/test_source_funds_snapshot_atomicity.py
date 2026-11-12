"""A failed frozen-case save must not leave a partial case behind."""

import sqlite3

import pytest

from kassiber.core.source_funds import save_case_snapshot
from kassiber.db import open_db
from tests.test_source_overlap import _seed_book, _tx, _wallet


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path / "data")
    _seed_book(conn)
    _wallet(conn, "wallet", "Wallet", "csv", {})
    _tx(conn, "target", "wallet", "target")
    conn.commit()
    yield conn
    conn.close()


def save(conn):
    return save_case_snapshot(conn, "ws", "pf", "target", 1000, "BTC", "minimal", "blocked", {"synthetic": True})


@pytest.mark.parametrize("pending", [False, True])
def test_snapshot_insert_failure_removes_case_and_preserves_callers_work(book, pending):
    book.execute("""CREATE TEMP TRIGGER refuse_snapshot BEFORE INSERT ON source_funds_snapshots
        BEGIN SELECT RAISE(ABORT, 'synthetic late refusal'); END""")
    if pending:
        book.execute("UPDATE profiles SET label='Pending' WHERE id='pf'")
    with pytest.raises(sqlite3.IntegrityError, match="synthetic late refusal"):
        save(book)
    assert book.execute("SELECT count(*) FROM source_funds_cases").fetchone()[0] == 0
    assert book.execute("SELECT count(*) FROM source_funds_snapshots").fetchone()[0] == 0
    assert book.in_transaction == pending
    if pending:
        assert book.execute("SELECT label FROM profiles WHERE id='pf'").fetchone()[0] == "Pending"


def test_case_save_does_not_commit_callers_pending_transaction(book):
    book.execute("UPDATE profiles SET label='Pending' WHERE id='pf'")
    save(book)
    assert book.in_transaction
    book.rollback()
    assert book.execute("SELECT count(*) FROM source_funds_cases").fetchone()[0] == 0


def test_deferred_commit_failure_removes_both_case_and_snapshot(book):
    book.executescript("""
        CREATE TABLE synthetic_deferred(id TEXT REFERENCES profiles(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TEMP TRIGGER refuse_commit AFTER INSERT ON source_funds_snapshots
        BEGIN INSERT INTO synthetic_deferred VALUES('missing'); END;
    """)
    with pytest.raises(sqlite3.IntegrityError, match="FOREIGN KEY"):
        save(book)
    assert book.execute("SELECT count(*) FROM source_funds_cases").fetchone()[0] == 0
    assert book.execute("SELECT count(*) FROM source_funds_snapshots").fetchone()[0] == 0
    assert not book.in_transaction
