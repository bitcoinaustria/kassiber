#!/usr/bin/env python3
"""Reproduce Python SQLCipher rollback's logical/physical identity distinction.

Uses a disposable book with synthetic data and the unchanged Python wallet
savepoint. No network, user book, or wallet material is read.
"""
import hashlib
from pathlib import Path
import tempfile

import sqlcipher3

from generate_publication_vectors import SEED, snapshot
from kassiber.cli.handlers import _run_wallet_refresh_savepoint
from kassiber.db import SCHEMA


def main():
    with tempfile.TemporaryDirectory(prefix="kassiber-ciphertext-rollback-") as tmp:
        path = Path(tmp) / "book.db"

        def open_book():
            conn = sqlcipher3.connect(path)
            conn.execute("PRAGMA key='synthetic-test-only'")
            conn.execute("PRAGMA journal_mode=DELETE")
            conn.execute("PRAGMA synchronous=FULL")
            conn.execute("PRAGMA foreign_keys=ON")
            return conn

        conn = open_book()
        conn.executescript(SCHEMA)
        conn.executescript(SEED)
        conn.commit()
        conn.close()
        before = path.read_bytes()
        conn = open_book()
        logical_before = snapshot(conn)
        conn.execute("CREATE TEMP TRIGGER fail BEFORE DELETE ON transaction_tags "
                     "BEGIN SELECT RAISE(ABORT,'injected'); END")
        try:
            _run_wallet_refresh_savepoint(
                conn, lambda: conn.execute("DELETE FROM transactions WHERE id='one'"),
                suppress_progress=True,
            )
        except sqlcipher3.DatabaseError as exc:
            print("injected failure:", str(exc))
        else:
            raise AssertionError("failure injection was not reached")
        assert snapshot(conn) == logical_before, "logical rollback failed"
        conn.close()
        after = path.read_bytes()
        print("logical dump restored: True")
        print("ciphertext identical:", before == after)
        print("before SHA256:", hashlib.sha256(before).hexdigest())
        print("after SHA256:", hashlib.sha256(after).hexdigest())


if __name__ == "__main__":
    main()
