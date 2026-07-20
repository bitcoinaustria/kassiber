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
import sys

from ..backends import resolve_effective_env_file
from ..db import (
    ensure_data_root,
    resolve_database_path,
    resolve_effective_data_root,
)
from ..errors import AppError
from ..operator.modes import set_unlock_mode, unlock_mode_status
from ..operator.native_auth import invalidate_operator_native_auth
from .credentials import (
    migrate_dotenv_credentials,
    scan_dotenv_for_secrets,
)
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
from .unlock_store import (
    cli_legacy_unlock_quarantined,
    cli_remembered_unlock_enabled,
    delete_legacy_shared_passphrase,
    delete_remembered_passphrase,
    mark_desktop_biometric_passphrase_stale,
    refresh_remembered_passphrase_after_rotation,
    remembered_unlock_status,
    set_cli_remembered_unlock_enabled,
    set_cli_unlock_state,
    store_remembered_passphrase,
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
    if fd_attr == "db_passphrase_fd":
        cached = getattr(args, "_db_passphrase_cached", None)
        if isinstance(cached, str) and cached:
            return cached
    fd = getattr(args, fd_attr, None)
    if fd is not None:
        return read_passphrase_from_fd(int(fd))
    if getattr(args, "non_interactive", False):
        flag = "--" + fd_attr.replace("_", "-")
        raise AppError(
            "passphrase input is required in non-interactive mode",
            code="interaction_required",
            hint=f"Pass the secret through {flag} from a controlling process.",
            retryable=False,
        )
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
    env_file = Path(
        resolve_effective_env_file(
            getattr(args, "env_file", None), args.data_root
        )
    )
    plaintext_secrets = scan_dotenv_for_secrets(env_file)
    classification["dotenv_path"] = str(env_file)
    classification["dotenv_plaintext_secrets"] = plaintext_secrets
    classification["remembered_unlock"] = remembered_unlock_status(args.data_root)
    classification["operator_unlock_mode"] = unlock_mode_status(args.data_root)
    if classification["encrypted"] and plaintext_secrets:
        classification["dotenv_warning"] = (
            "Encrypted database is in use but the bootstrap dotenv still "
            "contains plaintext secrets. Run "
            "`kassiber secrets migrate-credentials` to lift them into the "
            "encrypted backends table and sanitize the file."
        )
    return classification


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
        return {
            "mode": "migrated",
            "database": str(result.encrypted_path),
            "backup": str(result.backup_path),
            "user_version": result.plaintext_user_version,
            "auto_vacuum": result.plaintext_auto_vacuum,
            "integrity_check": result.integrity_check,
            "cipher_integrity_check": result.cipher_integrity_check,
            "credential_marker_clean": result.credential_marker_clean,
        }

    create_empty_encrypted_database(db_path, new_passphrase)
    return {"mode": "created", "database": str(db_path)}


def cmd_secrets_init_resume(args: argparse.Namespace) -> dict:
    db_path = _resolve_db_path(args)
    state = find_resumable_state(db_path)
    return {
        "database": str(db_path),
        "state": state,
        "hint": (
            "If `encrypted_temp` is present and trustworthy, you can rename it "
            "to the database path manually after verifying with `kassiber secrets verify`."
        ),
    }


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

    desktop_stale_generation = None
    operator_stale_generation = None

    def invalidate_native_credentials() -> None:
        nonlocal desktop_stale_generation, operator_stale_generation
        desktop_stale_generation = mark_desktop_biometric_passphrase_stale(
            args.data_root
        )
        operator_stale_generation = invalidate_operator_native_auth(args.data_root)

    result = change_database_passphrase(
        db_path,
        current,
        new_passphrase,
        before_rekey=invalidate_native_credentials,
    )
    result["desktop_biometric_invalidated"] = desktop_stale_generation is not None
    result["desktop_biometric_stale_generation"] = desktop_stale_generation
    result["operator_native_auth_invalidated"] = True
    result["operator_native_auth_stale_generation"] = operator_stale_generation
    remembered_warning = refresh_remembered_passphrase_after_rotation(
        args.data_root,
        new_passphrase,
    )
    if remembered_warning is not None:
        sys.stderr.write(
            "warning: remembered_unlock_update_failed: the database "
            "passphrase changed, but the CLI credential-store copy could not "
            "be updated safely; inspect `kassiber secrets status`, remove any "
            "retained legacy credential, then re-enroll.\n"
        )
    result["remembered_unlock"] = remembered_unlock_status(args.data_root)
    if remembered_warning is not None:
        result["remembered_unlock_warning"] = remembered_warning
    return result


def cmd_secrets_remember_unlock(args: argparse.Namespace) -> dict:
    require_sqlcipher()
    db_path = _resolve_db_path(args)
    classification = _classify(db_path)
    if not classification["exists"]:
        raise AppError(
            "database does not exist; run `kassiber secrets init` first",
            code="missing_database",
            details={"database": str(db_path)},
            retryable=False,
        )
    if classification["plaintext"]:
        raise AppError(
            "database is plaintext; encrypt it with `kassiber secrets init` first",
            code="plaintext_database",
            details={"database": str(db_path)},
            retryable=False,
        )

    passphrase = _resolve_passphrase(
        args,
        "passphrase_fd",
        label="Database passphrase: ",
        confirm=False,
    )
    conn = open_encrypted(db_path, passphrase)
    conn.close()

    cli_enabled_before = cli_remembered_unlock_enabled(args.data_root)
    legacy_quarantined_before = cli_legacy_unlock_quarantined(args.data_root)
    if not store_remembered_passphrase(args.data_root, passphrase):
        raise AppError(
            "the OS credential store is unavailable or rejected the passphrase",
            code="remembered_unlock_unavailable",
            hint=(
                "Unlock the platform credential store and retry, or keep using "
                "--db-passphrase-fd. Kassiber will not use a plaintext fallback."
            ),
            details=remembered_unlock_status(args.data_root),
            retryable=True,
        )
    try:
        set_cli_remembered_unlock_enabled(args.data_root, True)
    except OSError as exc:
        credential_deleted = delete_remembered_passphrase(args.data_root)
        raise AppError(
            "the passphrase was stored, but the CLI opt-in marker could not be written",
            code="remembered_unlock_settings_failed",
            hint=(
                "Fix permissions on the managed config directory and retry enrollment."
                if credential_deleted
                else "Fix config permissions, remove the OS credential manually, and retry."
            ),
            details={
                "settings_error": str(exc),
                "credential_deleted": credential_deleted,
            },
            retryable=True,
        ) from None

    if not delete_legacy_shared_passphrase(args.data_root):
        marker_restored = cli_enabled_before
        marker_restore_error = None
        credential_deleted = False
        if not cli_enabled_before:
            try:
                set_cli_unlock_state(
                    args.data_root,
                    enabled=False,
                    legacy_quarantined=legacy_quarantined_before,
                )
                marker_restored = True
            except OSError as exc:
                marker_restore_error = str(exc)
            if marker_restored:
                credential_deleted = delete_remembered_passphrase(args.data_root)
        raise AppError(
            "the legacy shared unlock credential could not be removed",
            code="remembered_unlock_legacy_cleanup_failed",
            hint=(
                "Remove `Kassiber Database Passphrase` in the OS credential "
                "manager and retry enrollment."
            ),
            details={
                "cli_enabled_before": cli_enabled_before,
                "marker_restored": marker_restored,
                "marker_restore_error": marker_restore_error,
                "cli_credential_deleted": credential_deleted,
            },
            retryable=True,
        )

    try:
        set_cli_unlock_state(
            args.data_root,
            enabled=True,
            legacy_quarantined=False,
        )
        set_unlock_mode(args.data_root, "unattended")
    except OSError as exc:
        raise AppError(
            "CLI enrollment succeeded, but legacy quarantine state could not be cleared",
            code="remembered_unlock_settings_failed",
            hint="Fix permissions on the managed config directory and retry enrollment.",
            details={"settings_error": str(exc)},
            retryable=True,
        ) from None

    return {
        "database": str(db_path),
        "remembered_unlock": remembered_unlock_status(args.data_root),
    }


def cmd_secrets_forget_unlock(args: argparse.Namespace) -> dict:
    cli_owned_legacy = cli_remembered_unlock_enabled(
        args.data_root
    ) or cli_legacy_unlock_quarantined(args.data_root)
    deleted = delete_remembered_passphrase(args.data_root)
    legacy_deleted = (
        delete_legacy_shared_passphrase(args.data_root)
        if cli_owned_legacy
        else True
    )
    if not legacy_deleted:
        quarantine_error = None
        try:
            set_cli_unlock_state(
                args.data_root,
                enabled=False,
                legacy_quarantined=True,
            )
        except OSError as exc:
            quarantine_error = str(exc)
        raise AppError(
            "the CLI-owned legacy unlock credential could not be deleted",
            code="remembered_unlock_legacy_cleanup_failed",
            hint=(
                "Remove `Kassiber Database Passphrase` in the OS credential "
                "manager, then retry. Kassiber quarantined the leftover from "
                "both CLI and desktop use when managed settings allowed it."
            ),
            details={
                "cli_marker_cleared": quarantine_error is None,
                "credential_deleted": deleted,
                "legacy_credential_deleted": False,
                "legacy_quarantined": quarantine_error is None,
                "quarantine_error": quarantine_error,
            },
            retryable=True,
        )

    marker_error = None
    try:
        set_cli_unlock_state(
            args.data_root,
            enabled=False,
            legacy_quarantined=False,
        )
        set_unlock_mode(args.data_root, "manual")
    except OSError as exc:
        marker_error = str(exc)

    if marker_error is not None:
        raise AppError(
            "the CLI remembered-unlock marker could not be cleared",
            code="remembered_unlock_settings_failed",
            hint=(
                "Fix permissions on the managed config directory and retry."
                if deleted
                else "Fix config permissions, remove the OS credential manually, and retry."
            ),
            details={
                "settings_error": marker_error,
                "cli_marker_cleared": False,
                "credential_deleted": deleted,
                "legacy_credential_deleted": legacy_deleted,
            },
            retryable=True,
        ) from None

    result = {
        "cli_marker_cleared": True,
        "credential_deleted": deleted,
        "legacy_credential_deleted": legacy_deleted,
        "remembered_unlock": remembered_unlock_status(args.data_root),
    }
    if not deleted:
        result["warning"] = (
            "The CLI opt-in marker was cleared, but the OS credential could not "
            "be deleted. Remove it in the platform credential manager."
        )
    return result


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

    return {
        "database": str(db_path),
        "integrity_check": integrity_check,
        "cipher_integrity_check": cipher_integrity,
        "sqlite_master_rows": master_count,
    }


def cmd_secrets_migrate_credentials(args: argparse.Namespace) -> dict:
    """Lift plaintext secrets from `backends.env` into the encrypted DB."""

    require_sqlcipher()
    db_path = _resolve_db_path(args)
    classification = _classify(db_path)
    if not classification["exists"]:
        raise AppError(
            "database does not exist; run `kassiber secrets init` first",
            code="missing_database",
            details={"database": str(db_path)},
            retryable=False,
        )
    if classification["plaintext"]:
        raise AppError(
            "database is plaintext; encrypt it with `kassiber secrets init` "
            "before migrating credentials into it",
            code="plaintext_database",
            details={"database": str(db_path)},
            retryable=False,
        )

    env_file = Path(
        resolve_effective_env_file(
            getattr(args, "env_file", None), args.data_root
        )
    )
    findings = scan_dotenv_for_secrets(env_file)
    if not findings:
        return {
            "dotenv_path": str(env_file),
            "migrated": [],
            "skipped": [],
            "backup_path": None,
            "rewritten": False,
            "note": "dotenv has no plaintext secret-shaped entries",
        }

    if getattr(args, "dry_run", False):
        return {
            "dotenv_path": str(env_file),
            "dry_run": True,
            "would_migrate": findings,
            "rewritten": False,
        }

    passphrase = _resolve_passphrase(
        args,
        "db_passphrase_fd",
        label="Database passphrase: ",
        confirm=False,
    )
    conn = open_encrypted(db_path, passphrase)
    try:
        result = migrate_dotenv_credentials(
            conn,
            env_file,
            create_missing_backends=False,
        )
    finally:
        conn.close()
    return result


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

    remember = secrets_sub.add_parser(
        "remember-unlock",
        help="Verify and store the database passphrase in the OS credential store",
    )
    remember.add_argument(
        "--passphrase-fd",
        type=int,
        default=None,
        metavar="FD",
        help="Read the passphrase from this open file descriptor",
    )

    forget = secrets_sub.add_parser(
        "forget-unlock",
        help="Disable CLI remembered unlock and delete the OS credential",
    )
    _ = forget

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

    migrate_creds = secrets_sub.add_parser(
        "migrate-credentials",
        help=(
            "Move plaintext backend secrets (token/password/auth-header/username) "
            "from the bootstrap dotenv into the encrypted backends table"
        ),
    )
    migrate_creds.add_argument(
        "--dry-run",
        action="store_true",
        help="List the dotenv entries that would migrate, without touching the file",
    )

    return secrets


def dispatch_secrets(args: argparse.Namespace) -> dict:
    sub = args.secrets_command
    if sub == "init":
        return cmd_secrets_init(args)
    if sub == "init-resume":
        return cmd_secrets_init_resume(args)
    if sub == "change-passphrase":
        return cmd_secrets_change_passphrase(args)
    if sub == "remember-unlock":
        return cmd_secrets_remember_unlock(args)
    if sub == "forget-unlock":
        return cmd_secrets_forget_unlock(args)
    if sub == "verify":
        return cmd_secrets_verify(args)
    if sub == "status":
        return cmd_secrets_status(args)
    if sub == "migrate-credentials":
        return cmd_secrets_migrate_credentials(args)
    raise AppError(
        f"unknown secrets command: {sub!r}",
        code="unknown_command",
        retryable=False,
    )
