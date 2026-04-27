"""Orchestrate Kassiber `.kassiber` backup export and import.

Layout inside the encrypted age envelope (decrypted plaintext is a tar
archive):

    manifest.json
    kassiber.sqlite3            # SQLCipher copy of the live database
    attachments/...             # mirror of <state_root>/attachments
    config/backends.env         # mirror of <state_root>/config/backends.env

The export uses `Connection.backup()` to take an in-place SQLCipher copy
to a temp file, so writers can continue against the live DB while the
archive is being built. The result is then tarred and piped through age.

The import reverses the process: decrypt to a temp tarball, pass through
the strict tar member validator, extract into a staging directory,
validate the manifest against the staged files, and atomically move the
staging tree into place.
"""

from __future__ import annotations

import json
import shutil
import tarfile
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from io import BytesIO
from pathlib import Path
from typing import Iterable, Optional

from .. import __version__
from ..db import (
    resolve_attachments_root,
    resolve_config_root,
    resolve_database_path,
    resolve_effective_data_root,
    resolve_effective_state_root,
)
from ..errors import AppError
from ..secrets.sqlcipher import (
    looks_like_plaintext_sqlite,
    open_encrypted,
    require_sqlcipher,
)
from .age_cli import (
    AgeBackend,
    decrypt_age_stream,
    encrypt_age_stream,
    select_age_backend,
)
from .safe_tar import extract_tar_safely


BACKUP_DB_NAME = "kassiber.sqlite3"
BACKUP_MANIFEST_NAME = "manifest.json"
BACKUP_ATTACHMENTS_DIR = "attachments"
BACKUP_CONFIG_DIR = "config"
BACKUP_BACKENDS_ENV = f"{BACKUP_CONFIG_DIR}/backends.env"

MANIFEST_SCHEMA_VERSION = 1


@dataclass
class BackupExportResult:
    output_path: Path
    manifest: dict
    age_backend: str


@dataclass
class BackupImportResult:
    staging_path: Path
    installed_data_root: Optional[Path]
    manifest: dict
    pre_restore_backup: Optional[Path] = None


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _backup_sqlcipher_database(
    src_path: Path,
    src_passphrase: str,
    dst_path: Path,
) -> None:
    """Take a hot copy of `src_path` into `dst_path` via Connection.backup()."""

    if looks_like_plaintext_sqlite(src_path):
        raise AppError(
            f"refusing to back up plaintext database at {src_path}",
            code="plaintext_database",
            hint="Run `kassiber secrets init` first.",
            retryable=False,
        )
    require_sqlcipher()
    src = open_encrypted(src_path, src_passphrase)
    try:
        # Mirror the source key onto the destination so the backup
        # produces a SQLCipher-encrypted copy under the same passphrase.
        dst = open_encrypted(dst_path, src_passphrase)
        try:
            src.backup(dst)
        finally:
            dst.close()
    finally:
        src.close()


def _build_manifest(
    *,
    workspace_paths: dict,
    db_relpath: str,
    attachments_count: int,
    backends_env_present: bool,
) -> dict:
    return {
        "schema_version": MANIFEST_SCHEMA_VERSION,
        "kassiber_version": __version__,
        "created_at": _now_iso(),
        "paths": workspace_paths,
        "entries": {
            "database": db_relpath,
            "attachments_files": attachments_count,
            "backends_env": backends_env_present,
        },
        "notes": {
            "inner_db_encrypted": True,
            "inner_db_passphrase_required": True,
        },
    }


def _add_directory_to_tar(
    tar: tarfile.TarFile,
    source_root: Path,
    archive_prefix: str,
) -> int:
    """Recursively add `source_root` to `tar` under `archive_prefix/`.

    Returns the number of regular files added (used for manifest stats).
    """

    if not source_root.exists():
        return 0
    file_count = 0
    for entry in sorted(source_root.rglob("*")):
        rel = entry.relative_to(source_root).as_posix()
        arc_name = f"{archive_prefix}/{rel}" if rel else archive_prefix
        info = tar.gettarinfo(str(entry), arcname=arc_name)
        if info is None:
            continue
        # Strip ownership / link metadata so the archive is reproducible.
        info.uid = 0
        info.gid = 0
        info.uname = ""
        info.gname = ""
        if info.isfile():
            with open(entry, "rb") as handle:
                tar.addfile(info, fileobj=handle)
            file_count += 1
        elif info.isdir():
            tar.addfile(info)
    return file_count


def export_backup(
    data_root: str,
    output_path: Path,
    db_passphrase: str,
    *,
    backup_passphrase: Optional[str] = None,
    recipients: Optional[Iterable[str]] = None,
    age_backend: Optional[AgeBackend] = None,
) -> BackupExportResult:
    """Build a `.kassiber` backup file for the active data root."""

    if (backup_passphrase is None) == (recipients is None):
        raise AppError(
            "export_backup requires exactly one of `backup_passphrase` or `recipients`",
            code="invalid_backup_call",
            retryable=False,
        )

    backend = age_backend or select_age_backend()
    state_root = Path(resolve_effective_state_root(data_root)).expanduser()
    effective_data_root = Path(resolve_effective_data_root(data_root)).expanduser()
    db_path = Path(resolve_database_path(effective_data_root)).expanduser()
    attachments_root = Path(resolve_attachments_root(data_root)).expanduser()
    backends_env = Path(resolve_config_root(data_root)).expanduser() / "backends.env"

    if not db_path.exists():
        raise AppError(
            f"database not found at {db_path}",
            code="missing_database",
            retryable=False,
        )

    output_path = Path(output_path).expanduser()
    output_path.parent.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="kassiber-backup-") as tmpdir:
        staging = Path(tmpdir)
        db_copy = staging / BACKUP_DB_NAME
        _backup_sqlcipher_database(db_path, db_passphrase, db_copy)

        tarball_path = staging / "bundle.tar"
        attachments_count = 0
        backends_env_present = backends_env.exists()
        with tarfile.open(tarball_path, "w") as tar:
            db_info = tar.gettarinfo(str(db_copy), arcname=BACKUP_DB_NAME)
            db_info.uid = db_info.gid = 0
            db_info.uname = db_info.gname = ""
            with open(db_copy, "rb") as handle:
                tar.addfile(db_info, fileobj=handle)

            attachments_count = _add_directory_to_tar(
                tar, attachments_root, BACKUP_ATTACHMENTS_DIR
            )
            if backends_env_present:
                env_info = tar.gettarinfo(
                    str(backends_env), arcname=BACKUP_BACKENDS_ENV
                )
                env_info.uid = env_info.gid = 0
                env_info.uname = env_info.gname = ""
                with open(backends_env, "rb") as handle:
                    tar.addfile(env_info, fileobj=handle)

            manifest = _build_manifest(
                workspace_paths={
                    "state_root": str(state_root),
                    "data_root": str(effective_data_root),
                    "attachments_root": str(attachments_root),
                    "backends_env": str(backends_env),
                },
                db_relpath=BACKUP_DB_NAME,
                attachments_count=attachments_count,
                backends_env_present=backends_env_present,
            )
            manifest_bytes = (
                json.dumps(manifest, indent=2, sort_keys=True).encode("utf-8") + b"\n"
            )
            manifest_info = tarfile.TarInfo(name=BACKUP_MANIFEST_NAME)
            manifest_info.size = len(manifest_bytes)
            manifest_info.mtime = int(datetime.now(timezone.utc).timestamp())
            manifest_info.mode = 0o600
            tar.addfile(manifest_info, fileobj=BytesIO(manifest_bytes))

        with open(tarball_path, "rb") as src, open(output_path, "wb") as dst:
            encrypt_age_stream(
                src,
                dst,
                passphrase=backup_passphrase,
                recipients=list(recipients) if recipients else None,
                backend=backend,
            )

    return BackupExportResult(
        output_path=output_path,
        manifest=manifest,
        age_backend=backend.flavor,
    )


def import_backup(
    archive_path: Path,
    target_data_root: Path,
    *,
    backup_passphrase: Optional[str] = None,
    identity_file: Optional[Path] = None,
    age_backend: Optional[AgeBackend] = None,
    move_into_place: bool = False,
) -> BackupImportResult:
    """Decrypt and stage a `.kassiber` backup.

    By default the staging tree is left in place under a temp directory
    and the caller is responsible for moving it. With
    `move_into_place=True` we install the staged content under
    `target_data_root` after a manifest sanity check.
    """

    if (backup_passphrase is None) == (identity_file is None):
        raise AppError(
            "import_backup requires exactly one of `backup_passphrase` or `identity_file`",
            code="invalid_backup_call",
            retryable=False,
        )

    backend = age_backend or select_age_backend()
    archive_path = Path(archive_path).expanduser()
    if not archive_path.exists():
        raise AppError(
            f"backup file not found at {archive_path}",
            code="missing_backup",
            retryable=False,
        )

    staging_parent = Path(tempfile.mkdtemp(prefix="kassiber-restore-"))
    decrypted_tar = staging_parent / "bundle.tar"
    staging_dir = staging_parent / "stage"
    staging_dir.mkdir()

    try:
        with open(archive_path, "rb") as src, open(decrypted_tar, "wb") as dst:
            decrypt_age_stream(
                src,
                dst,
                passphrase=backup_passphrase,
                identity_file=identity_file,
                backend=backend,
            )

        with tarfile.open(decrypted_tar, "r") as tar:
            extract_tar_safely(tar, staging_dir)

        manifest_path = staging_dir / BACKUP_MANIFEST_NAME
        if not manifest_path.exists():
            raise AppError(
                "backup is missing manifest.json",
                code="invalid_backup",
                retryable=False,
            )
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if manifest.get("schema_version") != MANIFEST_SCHEMA_VERSION:
            raise AppError(
                f"unsupported backup manifest schema_version {manifest.get('schema_version')!r}",
                code="invalid_backup",
                retryable=False,
            )
        db_relpath = manifest.get("entries", {}).get("database")
        if db_relpath != BACKUP_DB_NAME:
            raise AppError(
                f"manifest declares unexpected database path {db_relpath!r}",
                code="invalid_backup",
                retryable=False,
            )
        if not (staging_dir / BACKUP_DB_NAME).exists():
            raise AppError(
                "decrypted backup is missing the SQLCipher database",
                code="invalid_backup",
                retryable=False,
            )

        # `entries.backends_env=True` in the manifest implies the bundle
        # actually carries the file. Cross-check so we fail loudly if a
        # tampered manifest claims content the archive does not contain.
        manifest_entries = manifest.get("entries", {}) if isinstance(manifest.get("entries"), dict) else {}
        if manifest_entries.get("backends_env") and not (staging_dir / BACKUP_BACKENDS_ENV).exists():
            raise AppError(
                "manifest declares backends_env but the file is missing from the archive",
                code="invalid_backup",
                retryable=False,
            )

        installed_root: Optional[Path] = None
        backup_dir: Optional[Path] = None
        if move_into_place:
            target_data_root = Path(target_data_root).expanduser()
            target_data_root.mkdir(parents=True, exist_ok=True)
            installed_root = target_data_root
            target_db = target_data_root / BACKUP_DB_NAME
            target_attachments = target_data_root.parent / BACKUP_ATTACHMENTS_DIR
            target_env = target_data_root.parent / BACKUP_CONFIG_DIR / "backends.env"

            # Move any pre-existing live data into a sibling
            # `pre-restore-<timestamp>/` directory so an accidental
            # restore over a populated data root is recoverable.
            needs_backup = (
                target_db.exists()
                or target_attachments.exists()
                or target_env.exists()
            )
            if needs_backup:
                backup_dir = target_data_root.parent / (
                    "pre-restore-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
                )
                backup_dir.mkdir(parents=True, exist_ok=False)
                if target_db.exists():
                    shutil.move(str(target_db), str(backup_dir / BACKUP_DB_NAME))
                if target_attachments.exists():
                    shutil.move(
                        str(target_attachments),
                        str(backup_dir / BACKUP_ATTACHMENTS_DIR),
                    )
                if target_env.exists():
                    backup_env_dir = backup_dir / BACKUP_CONFIG_DIR
                    backup_env_dir.mkdir(parents=True, exist_ok=True)
                    shutil.move(str(target_env), str(backup_env_dir / "backends.env"))

            shutil.copy2(staging_dir / BACKUP_DB_NAME, target_db)
            staged_attachments = staging_dir / BACKUP_ATTACHMENTS_DIR
            if staged_attachments.exists():
                shutil.copytree(staged_attachments, target_attachments)
            staged_env = staging_dir / BACKUP_BACKENDS_ENV
            if staged_env.exists():
                target_env.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(staged_env, target_env)

        return BackupImportResult(
            staging_path=staging_dir,
            installed_data_root=installed_root,
            manifest=manifest,
            pre_restore_backup=backup_dir if move_into_place else None,
        )
    except Exception:
        shutil.rmtree(staging_parent, ignore_errors=True)
        raise
