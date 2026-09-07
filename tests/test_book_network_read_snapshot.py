"""SQLCipher cursor and transaction ownership at the shared network read seam."""

import pytest

from kassiber.core.book_network import read_network_snapshot


@pytest.fixture
def conn(tmp_path):
    sqlcipher = pytest.importorskip("sqlcipher3.dbapi2")
    connection = sqlcipher.connect(str(tmp_path / "snapshot.db"))
    connection.execute("PRAGMA key='snapshot-test-key'")
    connection.execute("PRAGMA journal_mode=WAL")
    connection.execute("CREATE TABLE observations(id INTEGER PRIMARY KEY)")
    connection.executemany("INSERT INTO observations VALUES(?)", [(1,), (2,), (3,)])
    connection.commit()
    yield connection
    connection.close()


@pytest.mark.parametrize("failure", [False, True])
def test_owned_snapshot_preserves_live_cursor_and_discards_writes(conn, failure):
    cursor = conn.execute("SELECT id FROM observations ORDER BY id")
    assert cursor.fetchone() == (1,)
    assert not conn.in_transaction
    try:
        with read_network_snapshot(conn):
            conn.execute("INSERT INTO observations VALUES(4)")
            if failure:
                raise ValueError("synthetic read failure")
    except ValueError:
        assert failure
    assert not conn.in_transaction
    assert cursor.fetchall() == [(2,), (3,)]
    assert conn.execute("SELECT count(*) FROM observations").fetchone()[0] == 3


@pytest.mark.parametrize("failure", [False, True])
def test_borrowed_snapshot_preserves_caller_transaction(conn, failure):
    conn.execute("INSERT INTO observations VALUES(4)")
    cursor = conn.execute("SELECT id FROM observations ORDER BY id")
    assert cursor.fetchone() == (1,)
    try:
        with read_network_snapshot(conn):
            with read_network_snapshot(conn):
                assert conn.execute("SELECT count(*) FROM observations").fetchone()[0] == 4
                if failure:
                    raise ValueError("synthetic read failure")
    except ValueError:
        assert failure
    assert conn.in_transaction
    assert cursor.fetchall() == [(2,), (3,), (4,)]
    conn.rollback()
    assert conn.execute("SELECT count(*) FROM observations").fetchone()[0] == 3


def test_owned_snapshot_remains_consistent_across_concurrent_commit(conn, tmp_path):
    sqlcipher = pytest.importorskip("sqlcipher3.dbapi2")
    writer = sqlcipher.connect(str(tmp_path / "snapshot.db"))
    writer.execute("PRAGMA key='snapshot-test-key'")
    try:
        with read_network_snapshot(conn):
            assert conn.execute("SELECT count(*) FROM observations").fetchone()[0] == 3
            writer.execute("INSERT INTO observations VALUES(4)")
            writer.commit()
            with read_network_snapshot(conn):
                assert conn.execute("SELECT count(*) FROM observations").fetchone()[0] == 3
        assert not conn.in_transaction
        assert conn.execute("SELECT count(*) FROM observations").fetchone()[0] == 4
    finally:
        writer.close()
