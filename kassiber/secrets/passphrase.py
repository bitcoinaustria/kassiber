"""Change the SQLCipher passphrase of an already-encrypted database."""

from __future__ import annotations

from pathlib import Path

from ..errors import AppError
from .sqlcipher import (
    CIPHER_COMPATIBILITY,
    CIPHER_PAGE_SIZE_DEFAULT,
    KDF_ITER_DEFAULT,
    looks_like_plaintext_sqlite,
    open_encrypted,
    rekey_connection,
    require_sqlcipher,
)


def change_database_passphrase(
    db_path: str | Path,
    current_passphrase: str,
    new_passphrase: str,
    *,
    kdf_iter: int = KDF_ITER_DEFAULT,
    cipher_page_size: int = CIPHER_PAGE_SIZE_DEFAULT,
    compatibility: int = CIPHER_COMPATIBILITY,
) -> dict:
    """Rotate the database key. Verifies new key by reopening fresh."""

    require_sqlcipher()
    target = Path(db_path).expanduser()
    if not target.exists():
        raise AppError(
            f"database not found at {target}",
            code="missing_database",
            retryable=False,
        )
    if looks_like_plaintext_sqlite(target):
        raise AppError(
            f"database at {target} is plaintext; run `kassiber secrets init` first",
            code="plaintext_database",
            retryable=False,
        )

    conn = open_encrypted(
        target,
        current_passphrase,
        kdf_iter=kdf_iter,
        cipher_page_size=cipher_page_size,
        compatibility=compatibility,
    )
    try:
        rekey_connection(conn, new_passphrase)
    finally:
        conn.close()

    verify = open_encrypted(
        target,
        new_passphrase,
        kdf_iter=kdf_iter,
        cipher_page_size=cipher_page_size,
        compatibility=compatibility,
    )
    try:
        integrity_row = verify.execute("PRAGMA integrity_check").fetchone()
        integrity_check = integrity_row[0] if integrity_row else "missing"
        if integrity_check != "ok":
            raise AppError(
                "integrity_check failed on the rekeyed database",
                code="rekey_verification_failed",
                details={"integrity_check": integrity_check},
                retryable=False,
            )
        cipher_integrity = None
        try:
            rows = verify.execute("PRAGMA cipher_integrity_check").fetchall()
            cipher_integrity = "ok" if not rows else "; ".join(str(r[0]) for r in rows)
            if cipher_integrity != "ok":
                raise AppError(
                    "cipher_integrity_check reported issues after rekey",
                    code="rekey_verification_failed",
                    details={"cipher_integrity_check": cipher_integrity},
                    retryable=False,
                )
        except Exception as exc:
            if isinstance(exc, AppError):
                raise
            cipher_integrity = None
    finally:
        verify.close()

    return {
        "database": str(target),
        "integrity_check": integrity_check,
        "cipher_integrity_check": cipher_integrity,
    }
