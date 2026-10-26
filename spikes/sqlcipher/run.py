"""Synthetic, disposable Python↔Rust on-disk contract checks. No real books."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile

from kassiber.db import SCHEMA, _configure_connection_pragmas, _journal_settings_for_sqlite_version
from kassiber.secrets.migration import migrate_plaintext_to_encrypted
from kassiber.secrets.sqlcipher import open_encrypted, rekey_connection

HERE = Path(__file__).resolve().parent
RUST = Path(os.environ.get("R0_SQLCIPHER_BIN", HERE / "target/debug/r0-sqlcipher"))
KEY = "R0 only: quote ' newline\n🔑"
NEW = "R0 rotated '🔐"


def rust(*args):
    result = subprocess.run([str(RUST), *map(str, args)], text=True, capture_output=True)
    assert result.returncode == 0, result.stderr
    if result.stdout:
        print(result.stdout.strip())


def verify(path, key, expected=None):
    c = open_encrypted(path, key)
    _configure_connection_pragmas(c, encrypted=True)
    rows = c.execute("SELECT value FROM evidence ORDER BY id").fetchall()
    if expected:
        assert (expected,) in rows, rows
    assert c.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert c.execute("PRAGMA cipher_integrity_check").fetchall() == []
    assert c.execute("PRAGMA kdf_iter").fetchone()[0] == "256000"
    assert c.execute("PRAGMA cipher_page_size").fetchone()[0] == "4096"
    assert Path(path).read_bytes()[:16] != b"SQLite format 3\0"
    return c


with tempfile.TemporaryDirectory(prefix="kassiber-r0-sqlcipher-") as tmp:
    root = Path(tmp)
    py, rs = root / "python.sqlite3", root / "rust.sqlite3"
    schema = root / "schema.sql"
    schema.write_text(SCHEMA)
    rust("schema", root / "schema.sqlite3", KEY, schema)
    c = open_encrypted(root / "schema.sqlite3", KEY)
    assert c.execute("SELECT count(*) FROM sqlite_master WHERE type='table'").fetchone()[0] >= 91
    c.close()
    with open_encrypted(py, KEY) as c:
        c.executescript("CREATE TABLE evidence(id INTEGER PRIMARY KEY, value TEXT NOT NULL); INSERT INTO evidence VALUES(1,'python'); PRAGMA user_version=21;")
    c.close()
    rust("read-write", py, KEY)
    c = verify(py, KEY, "rust-write")
    sqlite = c.execute("SELECT sqlite_version()").fetchone()[0]
    assert c.execute("SELECT json_extract('{\"a\":42}', '$.a')").fetchone()[0] == 42
    assert c.execute("SELECT sum(value) FROM json_each('[1,2,3]')").fetchone()[0] == 6
    assert c.execute("SELECT row_number() OVER (ORDER BY id) FROM evidence").fetchone()[0] == 1
    assert c.execute("INSERT INTO evidence VALUES(1,'python-upsert') ON CONFLICT(id) DO UPDATE SET value=excluded.value RETURNING value").fetchone()[0] == "python-upsert"
    c.commit()
    print(json.dumps({"python_sqlcipher": c.execute("PRAGMA cipher_version").fetchone()[0], "python_sqlite": sqlite, "stdlib_sqlite": sqlite3.sqlite_version, "python_journal": _journal_settings_for_sqlite_version(sqlite), "python_features": "JSON1/window/UPSERT/RETURNING pass"}))
    rekey_connection(c, NEW)
    c.close()
    rust("read-write", py, NEW)
    rust("rekey", py, NEW, KEY)
    verify(py, KEY).close()
    rust("create", rs, KEY)
    c = verify(rs, KEY, "rust")
    c.execute("INSERT INTO evidence(value) VALUES('python-write')")
    c.commit()
    c.close()
    rust("read-write", rs, KEY)
    rust("rekey", rs, KEY, NEW)
    c = verify(rs, NEW, "python-write")
    rekey_connection(c, KEY)
    c.close()
    rust("read-write", rs, KEY)
    rust("backup", rs, KEY, root / "rust-backup.sqlite3")
    verify(root / "rust-backup.sqlite3", KEY, "before-hot-backup").close()
    src = open_encrypted(py, KEY)
    dst = open_encrypted(root / "python-backup.sqlite3", KEY)
    # DB-API backup API, with the source open and an independent writer alive.
    writer = open_encrypted(py, KEY)
    writer.execute("INSERT INTO evidence(value) VALUES('python-hot-backup')")
    writer.commit()
    src.backup(dst, pages=1)
    writer.close(); src.close(); dst.close()
    rust("read-write", root / "python-backup.sqlite3", KEY)
    verify(root / "python-backup.sqlite3", KEY, "python-hot-backup").close()
    for side in ("python", "rust"):
        plain = root / f"plain-{side}.sqlite3"
        c = sqlite3.connect(plain)
        c.executescript("PRAGMA auto_vacuum=FULL; CREATE TABLE evidence(id INTEGER PRIMARY KEY,value TEXT); INSERT INTO evidence VALUES(1,'python-plain'); PRAGMA user_version=21;")
        c.close()
        if side == "python":
            result = migrate_plaintext_to_encrypted(plain, KEY)
            encrypted = plain
            assert result.plaintext_user_version == 21
            assert result.plaintext_auto_vacuum == 1
            assert result.backup_path.exists()
        else:
            encrypted = root / "rust-export.sqlite3"
            rust("export", plain, KEY, encrypted)
            assert plain.read_bytes()[:16] == b"SQLite format 3\0"
        c = verify(encrypted, KEY, "python-plain")
        assert c.execute("PRAGMA user_version").fetchone()[0] == 21
        assert c.execute("PRAGMA auto_vacuum").fetchone()[0] == 1
        c.close()
        rust("read-write", encrypted, KEY)
    try:
        open_encrypted(rs, "wrong", quiet_unlock_errors=True)
    except Exception as e:
        assert getattr(e, "code", None) == "unlock_failed"
    else:
        raise AssertionError("wrong key accepted")
    bad = subprocess.run([str(RUST), "read-write", str(rs), "wrong"], capture_output=True)
    assert bad.returncode != 0
print("PASS: bidirectional read/write, both rekeys, both exports, both hot backups, features, wrong-key rejection")
