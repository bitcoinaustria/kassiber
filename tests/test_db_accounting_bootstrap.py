"""Accounting storage bootstrap batches durable writes without owning caller work."""
from unittest.mock import patch

import pytest

from kassiber import db


@pytest.fixture(params=[False, True], ids=["sqlite", "sqlcipher"])
def passphrase(request):
    if request.param and not db.secrets_sqlcipher.sqlcipher_available():
        pytest.skip("SQLCipher is unavailable")
    return "accounting-bootstrap-fixture" if request.param else None


def test_open_batches_accounting_schema_and_returns_ready(tmp_path, passphrase):
    states = []
    target = db.secrets_sqlcipher if passphrase else db.sqlite3
    method = "open_encrypted" if passphrase else "connect"
    original = getattr(target, method)

    def traced_connect(*args, **kwargs):
        conn = original(*args, **kwargs)

        def trace(statement):
            if statement.lstrip().upper().startswith(("CREATE ", "ALTER ", "DROP ")) and "gl_" in statement:
                states.append(conn.in_transaction)

        conn.set_trace_callback(trace)
        return conn

    with patch.object(target, method, traced_connect):
        conn = db.open_db(tmp_path, passphrase=passphrase)
    try:
        assert len(states) > 200
        assert all(states), "accounting DDL must share a transaction"
        assert not conn.in_transaction
        assert conn.execute("SELECT COUNT(*) FROM gl_books").fetchone()[0] == 0
        assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert conn.execute("PRAGMA foreign_key_check").fetchall() == []
    finally:
        conn.close()


def _schema(conn):
    return [tuple(row) for row in conn.execute(
        "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name"
    )]


@pytest.fixture
def before_accounting(tmp_path, passphrase):
    with patch.object(db, "_ensure_accounting_schema"):
        conn = db.open_db(tmp_path, passphrase=passphrase)
    try:
        conn.execute("CREATE TABLE caller_work(value TEXT)")
        conn.commit()
        yield conn
    finally:
        conn.close()


def test_bootstrap_matches_independent_accounting_installers(before_accounting):
    from kassiber.core.accounting import (
        schema, evidence, bank, schedules, document_text, tax_workpapers,
        sources, artifacts, projection, ai_proposals, posting_batch, valuation,
        cashbook, task_schema,
    )

    conn = before_accounting
    conn.execute("BEGIN")
    for module in (
        schema, evidence, bank, schedules, document_text, tax_workpapers,
        sources, artifacts, projection, ai_proposals, posting_batch, valuation,
        cashbook, task_schema,
    ):
        module.ensure_schema(conn)
    expected = _schema(conn)
    conn.rollback()

    db._ensure_accounting_schema(conn)
    assert _schema(conn) == expected
    assert not conn.in_transaction
    assert conn.execute("SELECT COUNT(*) FROM gl_books").fetchone()[0] == 0
    db._ensure_accounting_schema(conn)
    assert _schema(conn) == expected
    assert not conn.in_transaction


def test_success_preserves_caller_transaction_and_rollback(before_accounting):
    conn = before_accounting
    initial_schema = _schema(conn)
    conn.execute("INSERT INTO caller_work VALUES ('pending')")
    db._ensure_accounting_schema(conn)
    assert conn.in_transaction
    assert conn.execute("SELECT value FROM caller_work").fetchone()[0] == "pending"
    assert conn.execute("SELECT COUNT(*) FROM gl_books").fetchone()[0] == 0
    conn.rollback()
    assert _schema(conn) == initial_schema
    assert conn.execute("SELECT * FROM caller_work").fetchall() == []


@pytest.mark.parametrize("caller_transaction", [False, True])
@pytest.mark.parametrize("failure_type", [RuntimeError, KeyboardInterrupt])
def test_failure_rolls_back_whole_bootstrap_only(before_accounting, caller_transaction, failure_type):
    from kassiber.core.accounting import task_schema

    conn = before_accounting
    initial_schema = _schema(conn)
    if caller_transaction:
        conn.execute("INSERT INTO caller_work VALUES ('pending')")

    def fail_late(connection):
        assert connection.execute("SELECT COUNT(*) FROM gl_books").fetchone()[0] == 0
        connection.execute("ALTER TABLE caller_work ADD COLUMN partial_column TEXT")
        connection.execute("INSERT INTO caller_work(value) VALUES ('bootstrap write')")
        raise failure_type("synthetic accounting migration failure")

    with patch.object(task_schema, "ensure_schema", fail_late), pytest.raises(failure_type):
        db._ensure_accounting_schema(conn)
    assert conn.in_transaction == caller_transaction
    assert _schema(conn) == initial_schema
    assert [row[0] for row in conn.execute("SELECT value FROM caller_work")] == (
        ["pending"] if caller_transaction else []
    )
    conn.rollback()
    assert conn.execute("SELECT * FROM caller_work").fetchall() == []
    db._ensure_accounting_schema(conn)
    assert not conn.in_transaction
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
