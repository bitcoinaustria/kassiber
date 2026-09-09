"""Desktop-only encrypted backup actions; no AI tool or generic CLI dispatch.

Restore previews retain one validated snapshot, never a password. Applying the
preview requires the same open container and unchanged database generation.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
from pathlib import Path
import secrets
import shutil
import time
from typing import Any, Callable

from .backup.install import install_staged_backup, restore_targets
from .backup.pack import BACKUP_DB_NAME, export_backup, import_backup
from .db import resolve_effective_state_root
from .errors import AppError
from .operator.project import exclusive_project_maintenance
from .secrets.sqlcipher import looks_like_plaintext_sqlite, open_encrypted

KINDS = frozenset({"ui.backup.export", "ui.backup.preview", "ui.backup.apply", "ui.backup.cancel"})


@dataclass
class RestorePreview:
    token: str
    staging: Path
    target: tuple
    digest: str
    expires_at: float
    unavailable_secrets: bool


class BackupSessions:
    def __init__(self) -> None:
        self.preview: RestorePreview | None = None

    def clear(self) -> None:
        preview, self.preview = self.preview, None
        if preview is not None:
            shutil.rmtree(preview.staging.parent, ignore_errors=True)

    def expire(self) -> None:
        if self.preview is not None and self.preview.expires_at <= time.monotonic():
            self.clear()


def _secret(args: dict, key: str) -> str:
    value = args.get(key)
    if not isinstance(value, str) or not value:
        raise AppError("Both backup and database passphrases are required", code="backup_passphrase_required")
    return value


def _fingerprint(ctx: Any) -> tuple:
    db = restore_targets(Path(ctx.data_root))[BACKUP_DB_NAME]
    stat = db.stat()
    sidecars = []
    for name, target in restore_targets(Path(ctx.data_root)).items():
        if name == BACKUP_DB_NAME:
            continue
        paths = [target, *sorted(target.rglob("*"))] if target.is_dir() else [target]
        for path in paths:
            if path.exists():
                info = path.lstat()
                sidecars.append((str(path), info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns))
    return (
        tuple(sidecars), str(Path(ctx.data_root).resolve()), ctx.project_id, id(ctx.conn),
        stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns,
        ctx.conn.total_changes, ctx.conn.execute("PRAGMA data_version").fetchone()[0],
    )


def _digest_tree(staging: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(staging.rglob("*")):
        if path.is_symlink():
            raise AppError("Restore staging changed", code="stale_backup_preview")
        if path.is_file():
            digest.update(path.relative_to(staging).as_posix().encode())
            with path.open("rb") as source:
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(chunk)
    return digest.hexdigest()


def _book_summary(conn: Any) -> dict:
    return {
        "workspaces": [str(row[0]) for row in conn.execute("SELECT label FROM workspaces ORDER BY label LIMIT 100")],
        "books": [str(row[0]) for row in conn.execute("SELECT label FROM profiles ORDER BY label LIMIT 100")],
        "book_count": conn.execute("SELECT count(*) FROM profiles").fetchone()[0],
    }


def handle_backup(
    ctx: Any, kind: str, args: dict, *,
    close_database: Callable[[Callable[[], None]], None],
    ensure_owner: Callable[[], None],
    release_owner: Callable[[], None],
    invalidate_credentials: Callable[[], None],
) -> dict:
    sessions = ctx.backup_sessions
    sessions.expire()
    if kind == "ui.backup.cancel":
        if sessions.preview is not None and args.get("token") == sessions.preview.token:
            sessions.clear()
        return {"cancelled": True}
    if ctx.conn is None:
        raise AppError("Unlock the target container before managing backups", code="backup_target_locked")
    if kind == "ui.backup.export":
        destination = args.get("path")
        if not isinstance(destination, str) or not Path(destination).is_absolute() or Path(destination).suffix != ".kassiber":
            raise AppError("Choose a .kassiber backup destination", code="invalid_backup_destination")
        target = Path(destination).resolve()
        if target.is_relative_to(Path(resolve_effective_state_root(ctx.data_root)).resolve()):
            raise AppError("Save the backup outside the active container", code="invalid_backup_destination")
        if not ctx.db_passphrase:
            raise AppError("Encrypt this container before exporting a backup", code="plaintext_database")
        try:
            result = export_backup(ctx.data_root, target, ctx.db_passphrase, backup_passphrase=_secret(args, "backup_passphrase_secret"))
        except OSError:
            raise AppError("The backup could not be saved", code="backup_export_failed", hint="Check destination permissions and available space, then choose the destination again.") from None
        return {"path": str(result.output_path), "size_bytes": result.output_path.stat().st_size}
    if kind == "ui.backup.preview":
        sessions.clear()
        archive = args.get("path")
        if not isinstance(archive, str) or not Path(archive).is_absolute():
            raise AppError("Choose a backup file", code="missing_backup")
        result = None
        try:
            result = import_backup(Path(archive), Path(ctx.data_root), backup_passphrase=_secret(args, "backup_passphrase_secret"))
            staging = result.staging_path
            assert staging is not None
            db = staging / BACKUP_DB_NAME
            if looks_like_plaintext_sqlite(db):
                raise AppError("Backup database is not encrypted", code="invalid_backup")
            restored = open_encrypted(db, _secret(args, "database_passphrase_secret"), quiet_unlock_errors=True, enforce_operator_identity=False)
            try:
                if restored.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
                    raise AppError("Backup database integrity check failed", code="invalid_backup")
                incoming = _book_summary(restored)
            finally:
                restored.close()
            preview = RestorePreview(secrets.token_urlsafe(32), staging, _fingerprint(ctx), _digest_tree(staging), time.monotonic() + 600, bool(result.secret_ref_unavailable))
            sessions.preview = preview
            return {
                "token": preview.token,
                "target_data_root": str(Path(ctx.data_root).resolve()),
                "target": _book_summary(ctx.conn), "incoming": incoming,
                "attachments_files": result.manifest["entries"]["attachments_files"],
                "replaces": [name for name, path in restore_targets(Path(ctx.data_root)).items() if path.exists()],
                "unavailable_secrets": preview.unavailable_secrets,
                "expires_in_seconds": 600,
            }
        except Exception as exc:
            if result is not None and result.staging_path is not None:
                shutil.rmtree(result.staging_path.parent, ignore_errors=True)
            if isinstance(exc, AppError) and exc.code in {"backup_passphrase_required", "missing_backup"}:
                raise
            raise AppError(
                "The backup could not be opened or validated",
                code="invalid_backup",
                hint="Check the backup passphrase and the database passphrase used when this backup was created, then choose a valid backup file.",
            ) from None
    if kind == "ui.backup.apply":
        preview = sessions.preview
        if args.get("confirm") != "RESTORE":
            raise AppError("Confirm replacement before restoring", code="backup_confirmation_required")
        if preview is None or args.get("token") != preview.token or preview.target != _fingerprint(ctx):
            sessions.clear()
            raise AppError("The restore preview is no longer current", code="stale_backup_preview", hint="Preview the backup again in the target container.")
        if preview.digest != _digest_tree(preview.staging):
            sessions.clear()
            raise AppError("The staged backup changed", code="stale_backup_preview")
        ensure_owner()
        # Keep the desktop lease while closing connections, so a second process
        # cannot open the old database during installation.
        with exclusive_project_maintenance(ctx.data_root, active_owner_kind="desktop"):
            sessions.preview = None
            try:
                def require_current() -> None:
                    if preview.target != _fingerprint(ctx):
                        raise AppError("The target changed while preparing restore", code="stale_backup_preview")

                close_database(require_current)
                recovery = install_staged_backup(preview.staging, Path(ctx.data_root))
                warning = None
                try:
                    invalidate_credentials()
                except Exception:
                    warning = "restore_unlock_settings_failed"
                return {"warning": warning, "restored": True, "locked": True, "pre_restore_backup": str(recovery) if recovery else None, "unavailable_secrets": preview.unavailable_secrets}
            finally:
                shutil.rmtree(preview.staging.parent, ignore_errors=True)
                if ctx.conn is None:
                    release_owner()
    raise AppError("Unsupported backup action", code="unsupported_kind")
