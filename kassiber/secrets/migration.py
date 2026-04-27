"""Migrate a plaintext SQLite database into a SQLCipher-encrypted one.

The procedure follows the SQLCipher-recommended `sqlcipher_export()`
recipe, with a few safety extras:

- The new encrypted file is written to a sibling path (`*.encrypted.sqlite3`)
  so the original is untouched on failure.
- `user_version` and `auto_vacuum` are copied explicitly because
  `sqlcipher_export()` does not transfer them.
- After the export we open the new file fresh, verify the schema and
  basic invariants, and run `cipher_integrity_check` when the bundled
  SQLCipher build supports it.
- The original is renamed to `*.pre-encryption.sqlite3.bak` so a manual
  rollback is one `mv` away even if the migrator crashes mid-flight.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from ..errors import AppError
from .sqlcipher import (
    CIPHER_COMPATIBILITY,
    CIPHER_PAGE_SIZE_DEFAULT,
    KDF_ITER_DEFAULT,
    escape_passphrase,
    looks_like_plaintext_sqlite,
    open_encrypted,
    require_sqlcipher,
)


ENCRYPTED_SUFFIX = ".encrypted.sqlite3"
BACKUP_SUFFIX = ".pre-encryption.sqlite3.bak"

# Conservative grep list. Any of these strings appearing as raw bytes in
# the encrypted output would mean the export wrote unencrypted material
# (or that the disk file we just sanity-checked was not the encrypted
# one). The list is intentionally Bitcoin-shaped: descriptor prefixes,
# extended-key prefixes, and the env-style credential markers Kassiber
# may have stored before encryption.
DEFAULT_CREDENTIAL_MARKERS: tuple[bytes, ...] = (
    b"slip77(",
    b"xprv",
    b"tprv",
    b"_TOKEN=",
    b"_PASSWORD=",
    b"Authorization:",
)


@dataclass
class MigrationResult:
    encrypted_path: Path
    backup_path: Path
    plaintext_user_version: int
    plaintext_auto_vacuum: int
    integrity_check: str
    cipher_integrity_check: str | None
    credential_marker_clean: bool


def _stat_or_none(path: Path):
    try:
        return path.stat()
    except FileNotFoundError:
        return None


def find_resumable_state(plaintext_path: Path) -> dict:
    """Describe whatever artifacts exist around a half-finished migration."""

    encrypted = plaintext_path.with_name(plaintext_path.name.replace(".sqlite3", "") + ENCRYPTED_SUFFIX)
    backup = plaintext_path.with_name(plaintext_path.name.replace(".sqlite3", "") + BACKUP_SUFFIX)
    return {
        "plaintext": _stat_or_none(plaintext_path) is not None,
        "encrypted_temp": _stat_or_none(encrypted) is not None,
        "backup": _stat_or_none(backup) is not None,
        "plaintext_path": str(plaintext_path),
        "encrypted_temp_path": str(encrypted),
        "backup_path": str(backup),
    }


def _scan_for_markers(path: Path, markers: Iterable[bytes]) -> list[str]:
    """Return any marker strings that appear in the on-disk file as raw bytes."""

    haystack = path.read_bytes()
    return [m.decode("utf-8", "replace") for m in markers if m in haystack]


def migrate_plaintext_to_encrypted(
    plaintext_path: str | os.PathLike,
    new_passphrase: str,
    *,
    kdf_iter: int = KDF_ITER_DEFAULT,
    cipher_page_size: int = CIPHER_PAGE_SIZE_DEFAULT,
    compatibility: int = CIPHER_COMPATIBILITY,
    credential_markers: Iterable[bytes] = DEFAULT_CREDENTIAL_MARKERS,
) -> MigrationResult:
    """Encrypt the plaintext database at `plaintext_path` in place.

    Returns a `MigrationResult` describing the new file, the rollback
    backup, and the verification artifacts. Raises `AppError` on any
    failure; the original database is left untouched until the very last
    rename step.
    """

    require_sqlcipher()
    src = Path(plaintext_path).expanduser()
    if not src.exists():
        raise AppError(
            f"plaintext database not found at {src}",
            code="missing_database",
            retryable=False,
        )
    if not looks_like_plaintext_sqlite(src):
        raise AppError(
            f"file at {src} does not look like a plaintext SQLite database",
            code="not_plaintext_database",
            hint="Refusing to overwrite an unknown file. Inspect it manually.",
            retryable=False,
        )

    encrypted_path = src.with_name(src.name.replace(".sqlite3", "") + ENCRYPTED_SUFFIX)
    backup_path = src.with_name(src.name.replace(".sqlite3", "") + BACKUP_SUFFIX)

    if encrypted_path.exists():
        raise AppError(
            f"refusing to overwrite an existing encrypted temp at {encrypted_path}",
            code="resume_required",
            hint="Inspect the file or remove it before retrying.",
            details={"encrypted_temp": str(encrypted_path)},
            retryable=False,
        )

    quoted = escape_passphrase(new_passphrase)

    sqlcipher = require_sqlcipher()
    # Use the SQLCipher driver even for the plaintext side: when no
    # `PRAGMA key` is issued the driver behaves like vanilla SQLite, but
    # the `sqlcipher_export()` SQL function is only registered on
    # SQLCipher-built connections.
    src_conn = sqlcipher.connect(str(src))
    try:
        plaintext_user_version = src_conn.execute("PRAGMA user_version").fetchone()[0]
        plaintext_auto_vacuum = src_conn.execute("PRAGMA auto_vacuum").fetchone()[0]

        encrypted_quoted = "'" + str(encrypted_path).replace("'", "''") + "'"
        src_conn.execute(f"ATTACH DATABASE {encrypted_quoted} AS encrypted KEY {quoted}")
        src_conn.execute(f"PRAGMA encrypted.cipher_compatibility = {int(compatibility)}")
        src_conn.execute(f"PRAGMA encrypted.kdf_iter = {int(kdf_iter)}")
        src_conn.execute(f"PRAGMA encrypted.cipher_page_size = {int(cipher_page_size)}")
        src_conn.execute(f"PRAGMA encrypted.auto_vacuum = {int(plaintext_auto_vacuum)}")
        src_conn.execute("SELECT sqlcipher_export('encrypted')")
        src_conn.execute(f"PRAGMA encrypted.user_version = {int(plaintext_user_version)}")
        src_conn.execute("DETACH DATABASE encrypted")
    finally:
        src_conn.close()

    new_conn = open_encrypted(
        encrypted_path,
        new_passphrase,
        kdf_iter=kdf_iter,
        cipher_page_size=cipher_page_size,
        compatibility=compatibility,
    )
    try:
        post_user_version = new_conn.execute("PRAGMA user_version").fetchone()[0]
        if int(post_user_version) != int(plaintext_user_version):
            raise AppError(
                "user_version did not propagate into the encrypted database",
                code="migration_verification_failed",
                details={"expected": plaintext_user_version, "got": post_user_version},
                retryable=False,
            )
        integrity_row = new_conn.execute("PRAGMA integrity_check").fetchone()
        integrity_check = integrity_row[0] if integrity_row else "missing"
        if integrity_check != "ok":
            raise AppError(
                "integrity_check failed on the encrypted database",
                code="migration_verification_failed",
                details={"integrity_check": integrity_check},
                retryable=False,
            )
        cipher_integrity = None
        try:
            rows = new_conn.execute("PRAGMA cipher_integrity_check").fetchall()
            cipher_integrity = "ok" if not rows else "; ".join(str(r[0]) for r in rows)
            if cipher_integrity != "ok":
                raise AppError(
                    "cipher_integrity_check reported issues",
                    code="migration_verification_failed",
                    details={"cipher_integrity_check": cipher_integrity},
                    retryable=False,
                )
        except Exception as exc:
            if isinstance(exc, AppError):
                raise
            cipher_integrity = None
    finally:
        new_conn.close()

    leaks = _scan_for_markers(encrypted_path, credential_markers)
    credential_marker_clean = not leaks
    if leaks:
        raise AppError(
            "encrypted database file contains plaintext credential markers",
            code="migration_leaks_plaintext",
            details={"markers": leaks, "encrypted_path": str(encrypted_path)},
            retryable=False,
        )

    src.rename(backup_path)
    encrypted_path.rename(src)

    return MigrationResult(
        encrypted_path=src,
        backup_path=backup_path,
        plaintext_user_version=int(plaintext_user_version),
        plaintext_auto_vacuum=int(plaintext_auto_vacuum),
        integrity_check=integrity_check,
        cipher_integrity_check=cipher_integrity,
        credential_marker_clean=credential_marker_clean,
    )


def create_empty_encrypted_database(
    db_path: str | os.PathLike,
    new_passphrase: str,
    *,
    kdf_iter: int = KDF_ITER_DEFAULT,
    cipher_page_size: int = CIPHER_PAGE_SIZE_DEFAULT,
    compatibility: int = CIPHER_COMPATIBILITY,
) -> Path:
    """Create a brand-new encrypted database file at `db_path`.

    Used when `kassiber secrets init` runs in a data root that has no
    pre-existing plaintext database to migrate.
    """

    require_sqlcipher()
    target = Path(db_path).expanduser()
    if target.exists() and target.stat().st_size > 0:
        raise AppError(
            f"refusing to overwrite existing database at {target}",
            code="database_exists",
            retryable=False,
        )
    conn = open_encrypted(
        target,
        new_passphrase,
        kdf_iter=kdf_iter,
        cipher_page_size=cipher_page_size,
        compatibility=compatibility,
    )
    try:
        conn.execute("CREATE TABLE IF NOT EXISTS sqlite_init_marker(x INTEGER)")
        conn.execute("DROP TABLE sqlite_init_marker")
        conn.commit()
    finally:
        conn.close()
    return target
