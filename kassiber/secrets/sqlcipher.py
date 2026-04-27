"""Open SQLCipher databases with deterministic, recovery-friendly keying.

Every encrypted handle goes through `open_encrypted`, which:

1. Connects with the SQLCipher driver.
2. Issues `PRAGMA key` first, with the passphrase rendered as a SQL
   string literal — `sqlcipher` does not bind PRAGMA arguments.
3. Issues `cipher_compatibility`, `kdf_iter`, and `cipher_page_size` next,
   while the page-zero salt is still in play. Setting these later is a
   silent no-op.
4. Verifies by reading `sqlite_master`. SQLCipher's `PRAGMA key` itself
   does not fail on a wrong passphrase; the first real read does.

The keying defaults match SQLCipher 4's stock values so that a stranded
user can still recover with the upstream `sqlcipher` binary.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import Any

try:
    import sqlcipher3 as _sqlcipher_module
except ModuleNotFoundError:  # pragma: no cover - exercised in fallback tests
    _sqlcipher_module = None

from ..errors import AppError


KDF_ITER_DEFAULT = 256_000
CIPHER_PAGE_SIZE_DEFAULT = 4096
CIPHER_COMPATIBILITY = 4

_SQLITE_HEADER = b"SQLite format 3\x00"


def sqlcipher_available() -> bool:
    """Return True when the SQLCipher Python driver imported successfully."""

    return _sqlcipher_module is not None


def require_sqlcipher() -> Any:
    if _sqlcipher_module is None:
        raise AppError(
            "SQLCipher driver `sqlcipher3` is not installed",
            code="sqlcipher_unavailable",
            hint="Install with `pip install 'sqlcipher3>=0.6.2,<1'` or `uv add sqlcipher3`.",
            retryable=False,
        )
    return _sqlcipher_module


def escape_passphrase(passphrase: str) -> str:
    """Render a passphrase as a SQLite string literal.

    SQLite uses single-quote-doubling for embedded quotes. The driver does
    not bind PRAGMA arguments, so call sites must interpolate. We refuse
    NUL bytes (which terminate the SQL parser) and empty values up front;
    everything else, including emoji and embedded newlines, round-trips.
    """

    if passphrase is None:
        raise AppError(
            "passphrase must be a string",
            code="invalid_passphrase",
            retryable=False,
        )
    if not isinstance(passphrase, str):
        raise AppError(
            "passphrase must be a string",
            code="invalid_passphrase",
            details={"type": type(passphrase).__name__},
            retryable=False,
        )
    if passphrase == "":
        raise AppError(
            "passphrase must not be empty",
            code="invalid_passphrase",
            retryable=False,
        )
    if "\x00" in passphrase:
        raise AppError(
            "passphrase must not contain NUL bytes",
            code="invalid_passphrase",
            retryable=False,
        )
    return "'" + passphrase.replace("'", "''") + "'"


def looks_like_plaintext_sqlite(path: str | Path) -> bool:
    """Return True when the file at `path` starts with the standard SQLite header."""

    try:
        with open(path, "rb") as handle:
            header = handle.read(len(_SQLITE_HEADER))
    except FileNotFoundError:
        return False
    except OSError:
        return False
    return header == _SQLITE_HEADER


def apply_keying(
    conn: Any,
    passphrase: str,
    *,
    kdf_iter: int = KDF_ITER_DEFAULT,
    cipher_page_size: int = CIPHER_PAGE_SIZE_DEFAULT,
    compatibility: int = CIPHER_COMPATIBILITY,
) -> None:
    """Apply the standard keying PRAGMAs to a freshly opened connection.

    Order matters: `PRAGMA key` first, then `cipher_compatibility`, then
    `kdf_iter` and `cipher_page_size`. Setting these after the first real
    schema touch silently does nothing.
    """

    quoted = escape_passphrase(passphrase)
    conn.execute(f"PRAGMA key = {quoted}")
    conn.execute(f"PRAGMA cipher_compatibility = {int(compatibility)}")
    conn.execute(f"PRAGMA kdf_iter = {int(kdf_iter)}")
    conn.execute(f"PRAGMA cipher_page_size = {int(cipher_page_size)}")


def _database_error_classes() -> tuple[type, ...]:
    """Return every `DatabaseError` class we might see during verification.

    The SQLCipher driver raises `sqlcipher3.dbapi2.DatabaseError`, which
    does not inherit from the stdlib `sqlite3.DatabaseError`. Catch both
    so a wrong passphrase always surfaces as a structured `unlock_failed`.
    """

    classes: list[type] = [sqlite3.DatabaseError]
    if _sqlcipher_module is not None:
        sqlcipher_error = getattr(_sqlcipher_module, "DatabaseError", None)
        if sqlcipher_error is not None and sqlcipher_error not in classes:
            classes.append(sqlcipher_error)
    return tuple(classes)


def verify_unlock(conn: Any) -> None:
    """Read `sqlite_master` to confirm the supplied key was correct.

    SQLCipher does not validate `PRAGMA key` directly; the first real read
    against the encrypted pages is what proves the derived key matches.
    """

    try:
        conn.execute("SELECT count(*) FROM sqlite_master").fetchone()
    except _database_error_classes() as exc:
        raise AppError(
            "wrong passphrase or unsupported database format",
            code="unlock_failed",
            hint="Double-check the SQLCipher passphrase you entered.",
            retryable=True,
            details={"driver_error": str(exc)},
        ) from None


def get_row_class() -> Any:
    """Return the `Row` class belonging to the active SQLCipher driver."""

    sqlcipher = require_sqlcipher()
    return sqlcipher.Row


def open_encrypted(
    path: str | Path,
    passphrase: str,
    *,
    kdf_iter: int = KDF_ITER_DEFAULT,
    cipher_page_size: int = CIPHER_PAGE_SIZE_DEFAULT,
    compatibility: int = CIPHER_COMPATIBILITY,
    foreign_keys: bool = True,
    detect_types: int = 0,
    row_factory: Any | None = None,
) -> Any:
    """Open an encrypted SQLCipher database and return the keyed connection.

    `row_factory=None` keeps the driver default; pass `get_row_class()` to
    enable dict-like indexing matching the plaintext stdlib behavior.
    """

    sqlcipher = require_sqlcipher()
    conn = sqlcipher.connect(str(path), detect_types=detect_types)
    try:
        apply_keying(
            conn,
            passphrase,
            kdf_iter=kdf_iter,
            cipher_page_size=cipher_page_size,
            compatibility=compatibility,
        )
        verify_unlock(conn)
        if row_factory is not None:
            conn.row_factory = row_factory
        if foreign_keys:
            conn.execute("PRAGMA foreign_keys = ON")
        return conn
    except Exception:
        conn.close()
        raise


def rekey_connection(conn: Any, new_passphrase: str) -> None:
    """Change the passphrase of an already-keyed SQLCipher connection."""

    quoted = escape_passphrase(new_passphrase)
    conn.execute(f"PRAGMA rekey = {quoted}")
