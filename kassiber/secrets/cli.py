"""`kassiber secrets ...` command handlers.

These commands manage the SQLCipher database passphrase and the
plaintext-to-encrypted migration. They never open the database through
the normal runtime bootstrap because the bootstrap path expects the file
to already be in its target state (plaintext OR encrypted, with the
right passphrase). The secrets commands work on the file directly.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from ..db import (
    ensure_data_root,
    resolve_database_path,
    resolve_effective_data_root,
)
from ..envelope import build_envelope
from ..errors import AppError
from .migration import (
    create_empty_encrypted_database,
    find_resumable_state,
    migrate_plaintext_to_encrypted,
)
from .passphrase import change_database_passphrase
from .prompt import (
    PassphraseInputError,
    prompt_passphrase,
    prompt_passphrase_with_confirmation,
    read_passphrase_from_fd,
)
from .sqlcipher import (
    looks_like_plaintext_sqlite,
    open_encrypted,
    require_sqlcipher,
    sqlcipher_available,
)


_MIN_PASSPHRASE_CHARS = 8


def _resolve_db_path(args: argparse.Namespace) -> Path:
    data_root = ensure_data_root(resolve_effective_data_root(args.data_root))
    return resolve_database_path(data_root)


def _resolve_passphrase(
    args: argparse.Namespace,
    fd_attr: str,
    *,
    label: str,
    confirm: bool = False,
) -> str:
    fd = getattr(args, fd_attr, None)
    if fd is not None:
        return read_passphrase_from_fd(int(fd))
    if confirm:
        return prompt_passphrase_with_confirmation(label, "Confirm passphrase: ")
    return prompt_passphrase(label)


def _enforce_min_length(passphrase: str) -> None:
    if len(passphrase) < _MIN_PASSPHRASE_CHARS:
        raise PassphraseInputError(
            f"passphrase must be at least {_MIN_PASSPHRASE_CHARS} characters long",
            hint="Pick a long passphrase from a password manager.",
        )


def _classify(db_path: Path) -> dict:
    if not db_path.exists() or db_path.stat().st_size == 0:
        return {"path": str(db_path), "exists": False, "encrypted": False, "plaintext": False}
    plaintext = looks_like_plaintext_sqlite(db_path)
    return {
        "path": str(db_path),
        "exists": True,
        "encrypted": not plaintext,
        "plaintext": plaintext,
        "size_bytes": db_path.stat().st_size,
    }


def cmd_secrets_status(args: argparse.Namespace) -> dict:
    db_path = _resolve_db_path(args)
    classification = _classify(db_path)
    classification["resumable"] = find_resumable_state(db_path)
    classification["sqlcipher_available"] = sqlcipher_available()
    return build_envelope("secrets.status", classification)


def cmd_secrets_init(args: argparse.Namespace) -> dict:
    require_sqlcipher()
    db_path = _resolve_db_path(args)
    classification = _classify(db_path)

    if classification["exists"] and classification["encrypted"]:
        raise AppError(
            "database is already encrypted",
            code="already_encrypted",
            hint="Use `kassiber secrets change-passphrase` to rotate the passphrase.",
            details={"database": str(db_path)},
            retryable=False,
        )

    new_passphrase = _resolve_passphrase(
        args,
        "new_passphrase_fd",
        label="New database passphrase: ",
        confirm=getattr(args, "new_passphrase_fd", None) is None,
    )
    _enforce_min_length(new_passphrase)

    if classification["exists"] and classification["plaintext"]:
        result = migrate_plaintext_to_encrypted(db_path, new_passphrase)
        return build_envelope(
            "secrets.init",
            {
                "mode": "migrated",
                "database": str(result.encrypted_path),
                "backup": str(result.backup_path),
                "user_version": result.plaintext_user_version,
                "auto_vacuum": result.plaintext_auto_vacuum,
                "integrity_check": result.integrity_check,
                "cipher_integrity_check": result.cipher_integrity_check,
                "credential_marker_clean": result.credential_marker_clean,
            },
        )

    create_empty_encrypted_database(db_path, new_passphrase)
    return build_envelope(
        "secrets.init",
        {
            "mode": "created",
            "database": str(db_path),
        },
    )


def cmd_secrets_init_resume(args: argparse.Namespace) -> dict:
    db_path = _resolve_db_path(args)
    state = find_resumable_state(db_path)
    return build_envelope(
        "secrets.init.resume",
        {
            "database": str(db_path),
            "state": state,
            "hint": (
                "If `encrypted_temp` is present and trustworthy, you can rename it "
                "to the database path manually after verifying with `kassiber secrets verify`."
            ),
        },
    )


def cmd_secrets_change_passphrase(args: argparse.Namespace) -> dict:
    require_sqlcipher()
    db_path = _resolve_db_path(args)
    classification = _classify(db_path)
    if not classification["exists"]:
        raise AppError(
            "database does not exist",
            code="missing_database",
            details={"database": str(db_path)},
            retryable=False,
        )
    if classification["plaintext"]:
        raise AppError(
            "database is plaintext; run `kassiber secrets init` first",
            code="plaintext_database",
            details={"database": str(db_path)},
            retryable=False,
        )

    current = _resolve_passphrase(
        args,
        "db_passphrase_fd",
        label="Current passphrase: ",
        confirm=False,
    )
    new_passphrase = _resolve_passphrase(
        args,
        "new_passphrase_fd",
        label="New passphrase: ",
        confirm=getattr(args, "new_passphrase_fd", None) is None,
    )
    _enforce_min_length(new_passphrase)

    result = change_database_passphrase(db_path, current, new_passphrase)
    return build_envelope("secrets.change_passphrase", result)


def cmd_secrets_verify(args: argparse.Namespace) -> dict:
    require_sqlcipher()
    db_path = _resolve_db_path(args)
    classification = _classify(db_path)
    if not classification["exists"]:
        raise AppError(
            "database does not exist",
            code="missing_database",
            details={"database": str(db_path)},
            retryable=False,
        )
    if classification["plaintext"]:
        raise AppError(
            "database is plaintext; nothing to verify",
            code="plaintext_database",
            details={"database": str(db_path)},
            retryable=False,
        )

    passphrase = _resolve_passphrase(
        args,
        "db_passphrase_fd",
        label="Database passphrase: ",
        confirm=False,
    )
    conn = open_encrypted(db_path, passphrase)
    try:
        integrity = conn.execute("PRAGMA integrity_check").fetchone()
        integrity_check = integrity[0] if integrity else "missing"
        cipher_integrity = None
        try:
            rows = conn.execute("PRAGMA cipher_integrity_check").fetchall()
            cipher_integrity = "ok" if not rows else "; ".join(str(r[0]) for r in rows)
        except Exception:
            cipher_integrity = None
        master_count = conn.execute("SELECT count(*) FROM sqlite_master").fetchone()[0]
    finally:
        conn.close()

    return build_envelope(
        "secrets.verify",
        {
            "database": str(db_path),
            "integrity_check": integrity_check,
            "cipher_integrity_check": cipher_integrity,
            "sqlite_master_rows": master_count,
        },
    )


def add_secrets_parser(subparsers) -> argparse.ArgumentParser:
    """Attach the `secrets` subcommand tree to the top-level parser."""

    secrets = subparsers.add_parser(
        "secrets",
        help="Manage SQLCipher database encryption (init, change passphrase, verify)",
    )
    secrets_sub = secrets.add_subparsers(dest="secrets_command", required=True)

    init = secrets_sub.add_parser("init", help="Create or migrate to an encrypted database")
    init.add_argument(
        "--new-passphrase-fd",
        type=int,
        default=None,
        metavar="FD",
        help="Read the new passphrase from this open file descriptor (skips the interactive prompt)",
    )

    init_resume = secrets_sub.add_parser(
        "init-resume",
        help="Inspect leftover artifacts from a half-finished `secrets init`",
    )
    init_resume.set_defaults(secrets_command="init-resume")
    _ = init_resume

    change = secrets_sub.add_parser(
        "change-passphrase",
        help="Rotate the SQLCipher passphrase on an already-encrypted database",
    )
    change.add_argument(
        "--new-passphrase-fd",
        type=int,
        default=None,
        metavar="FD",
        help="Read the new passphrase from this open file descriptor",
    )

    verify = secrets_sub.add_parser(
        "verify",
        help="Open the encrypted database and run integrity checks",
    )
    _ = verify

    # `change-passphrase` and `verify` may also need the *current* passphrase
    # via fd. We accept the global `--db-passphrase-fd` rather than reusing
    # `args.db_passphrase_fd` because the secrets dispatch path skips the
    # normal runtime bootstrap.

    status = secrets_sub.add_parser(
        "status",
        help="Show whether the local database is plaintext, encrypted, or missing",
    )
    _ = status

    return secrets


def dispatch_secrets(args: argparse.Namespace) -> dict:
    sub = args.secrets_command
    if sub == "init":
        return cmd_secrets_init(args)
    if sub == "init-resume":
        return cmd_secrets_init_resume(args)
    if sub == "change-passphrase":
        return cmd_secrets_change_passphrase(args)
    if sub == "verify":
        return cmd_secrets_verify(args)
    if sub == "status":
        return cmd_secrets_status(args)
    raise AppError(
        f"unknown secrets command: {sub!r}",
        code="unknown_command",
        retryable=False,
    )
