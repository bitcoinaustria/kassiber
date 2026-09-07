"""Fresh books create their initial schema in one durable transaction."""
from unittest.mock import patch

import pytest

from kassiber import db
from kassiber.secrets.migration import create_empty_encrypted_database


@pytest.fixture(params=[False, True], ids=["sqlite", "sqlcipher"])
def database_driver(request):
    encrypted = request.param
    if encrypted and not db.secrets_sqlcipher.sqlcipher_available():
        pytest.skip("SQLCipher is unavailable")
    return "fixture-passphrase" if encrypted else None


def _precreate_empty_database(root, passphrase):
    path = db.resolve_database_path(root)
    if passphrase:
        create_empty_encrypted_database(path, passphrase)
    else:
        conn = db.sqlite3.connect(path)
        conn.execute("CREATE TABLE marker(value)")
        conn.execute("DROP TABLE marker")
        conn.close()
    assert path.stat().st_size > 0


@pytest.mark.parametrize("precreated", [False, True])
def test_initial_schema_is_transactional_and_open_returns_ready(tmp_path, database_driver, precreated):
    if precreated:
        _precreate_empty_database(tmp_path, database_driver)
    states = []
    target = db.secrets_sqlcipher if database_driver else db.sqlite3
    method = "open_encrypted" if database_driver else "connect"
    original = getattr(target, method)

    def traced_connect(*args, **kwargs):
        conn = original(*args, **kwargs)

        def trace(statement):
            if "CREATE TABLE IF NOT EXISTS settings (" in statement:
                states.append(conn.in_transaction)

        conn.set_trace_callback(trace)
        return conn

    with patch.object(target, method, traced_connect), patch.object(
        db, "_journal_settings_for_sqlite_version", return_value=("delete", "FULL")
    ):
        conn = db.open_db(tmp_path, passphrase=database_driver)
    try:
        assert states == [True]
        assert not conn.in_transaction
        assert conn.execute("PRAGMA foreign_keys").fetchone()[0] == 1
        assert conn.execute("PRAGMA journal_mode").fetchone()[0] == "delete"
        assert conn.execute("PRAGMA synchronous").fetchone()[0] == 2
        assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        identity = db.database_instance_id(conn)
    finally:
        conn.close()
    reopened = db.open_db(tmp_path, passphrase=database_driver)
    try:
        assert db.database_instance_id(reopened) == identity
        assert not reopened.in_transaction
    finally:
        reopened.close()


@pytest.mark.parametrize("precreated", [False, True])
def test_failed_fresh_schema_rolls_back_all_schema_objects(tmp_path, database_driver, precreated):
    if precreated:
        _precreate_empty_database(tmp_path, database_driver)
    with patch.object(db, "SCHEMA", db.SCHEMA + "\nINVALID SCHEMA STATEMENT;"), pytest.raises(Exception, match="syntax error"):
        db.open_db(tmp_path, passphrase=database_driver)
    path = db.resolve_database_path(tmp_path)
    if database_driver:
        assert not db.secrets_sqlcipher.looks_like_plaintext_sqlite(path)
        conn = db.secrets_sqlcipher.open_encrypted(path, database_driver)
    else:
        conn = db.sqlite3.connect(path)
    try:
        assert conn.execute("SELECT name FROM sqlite_master WHERE name='settings'").fetchone() is None
    finally:
        conn.close()


def test_existing_book_keeps_schema_migration_transaction_behavior(tmp_path, database_driver):
    conn = db.open_db(tmp_path, passphrase=database_driver)
    identity = db.database_instance_id(conn)
    conn.close()
    with patch.object(
        db, "SCHEMA", "CREATE TABLE existing_schema_probe(value); INVALID SCHEMA STATEMENT;"
    ), pytest.raises(Exception, match="syntax error"):
        db.open_db(tmp_path, passphrase=database_driver)
    conn = db.open_db(tmp_path, passphrase=database_driver)
    try:
        assert db.database_instance_id(conn) == identity
        assert conn.execute("SELECT name FROM sqlite_master WHERE name='existing_schema_probe'").fetchone()
    finally:
        conn.close()
