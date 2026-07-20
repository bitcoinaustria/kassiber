"""Orchestrate Kassiber `.kassiber` backup export and import.

Layout inside the encrypted age envelope (decrypted plaintext is a tar
archive):

    manifest.json
    kassiber.sqlite3            # SQLCipher copy of the live database
    attachments/...             # mirror of the project-local attachments tree
    config/backends.env         # optional project-local bootstrap dotenv
    config/settings.json        # optional project-local plaintext settings

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
import os
import shutil
import tarfile
import tempfile
from dataclasses import dataclass, field
from datetime import datetime, timezone
from io import BytesIO
from pathlib import Path
from typing import Iterable, Optional

from .. import __version__
from ..db import (
    DEFAULT_ATTACHMENTS_DIRNAME,
    DEFAULT_CONFIG_DIRNAME,
    DEFAULT_DATA_DIRNAME,
    DEFAULT_EXPORTS_DIRNAME,
    resolve_attachments_root,
    resolve_config_root,
    resolve_database_path,
    resolve_effective_data_root,
    resolve_effective_state_root,
    resolve_exports_root,
    resolve_settings_path,
)
from ..errors import AppError
from ..projects import project_metadata_for_data_root
from ..secrets.sqlcipher import (
    get_row_class,
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
BACKUP_SETTINGS_JSON = f"{BACKUP_CONFIG_DIR}/settings.json"

MANIFEST_SCHEMA_VERSION = 1
SQLCIPHER_INLINE_SECRET_STORE = "sqlcipher_inline"
SECRET_REF_REPAIR_HINT = (
    "Open Settings -> AI providers and re-enter or repair the provider API key."
)


@dataclass
class BackupExportResult:
    output_path: Path
    manifest: dict
    age_backend: str


@dataclass
class BackupImportResult:
    staging_path: Optional[Path]
    installed_data_root: Optional[Path]
    manifest: dict
    pre_restore_backup: Optional[Path] = None
    temporary_artifacts_cleaned: bool = False
    secret_ref_unavailable: list[dict] = field(default_factory=list)


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
        dst = open_encrypted(
            dst_path,
            src_passphrase,
            enforce_operator_identity=False,
        )
        try:
            src.backup(dst)
        finally:
            dst.close()
    finally:
        src.close()


def _collect_ai_provider_secret_refs(db_path: Path, db_passphrase: str) -> list[dict]:
    """Return non-inline AI provider secret refs for restore warnings.

    The manifest intentionally records reference metadata only. Secret values
    stay either inside the SQLCipher database (`sqlcipher_inline`) or in a
    future OS store; OS-backed refs are never materialized into the backup.
    """

    conn = open_encrypted(db_path, db_passphrase, row_factory=get_row_class())
    try:
        exists = conn.execute(
            """
            SELECT 1
            FROM sqlite_master
            WHERE type = 'table' AND name = 'ai_provider_secret_refs'
            """
        ).fetchone()
        if not exists:
            return []
        rows = conn.execute(
            """
            SELECT
                r.provider_name,
                r.store_id,
                r.service,
                r.account,
                r.state,
                p.api_key IS NOT NULL AND p.api_key != '' AS has_inline_secret
            FROM ai_provider_secret_refs r
            LEFT JOIN ai_providers p ON p.name = r.provider_name
            WHERE r.store_id != ?
            ORDER BY r.provider_name
            """,
            (SQLCIPHER_INLINE_SECRET_STORE,),
        ).fetchall()
        for row in rows:
            if row["has_inline_secret"]:
                raise AppError(
                    (
                        "refusing to export an OS-backed AI provider ref that "
                        "still has an inline API key"
                    ),
                    code="secret_ref_inline_secret",
                    hint=(
                        "Repair the provider secret state before exporting the backup."
                    ),
                    details={
                        "provider_name": row["provider_name"],
                        "store_id": row["store_id"],
                    },
                    retryable=False,
                )
        return [
            {
                "provider_name": row["provider_name"],
                "store_id": row["store_id"],
                "service": row["service"],
                "account": row["account"],
                "state": row["state"],
            }
            for row in rows
        ]
    finally:
        conn.close()


def _restore_unavailable_secret_refs(manifest: dict) -> list[dict]:
    secret_refs = manifest.get("secret_refs")
    if not isinstance(secret_refs, dict):
        return []
    refs = secret_refs.get("ai_provider_refs")
    if not isinstance(refs, list):
        return []

    unavailable: list[dict] = []
    for ref in refs:
        if not isinstance(ref, dict):
            continue
        store_id = str(ref.get("store_id") or "")
        if not store_id or store_id == SQLCIPHER_INLINE_SECRET_STORE:
            continue
        unavailable.append(
            {
                "provider_name": str(ref.get("provider_name") or ""),
                "store_id": store_id,
                "service": str(ref.get("service") or ""),
                "account": str(ref.get("account") or ref.get("provider_name") or ""),
                "state": "unavailable",
            }
        )
    return unavailable


def _build_manifest(
    *,
    workspace_paths: dict,
    db_relpath: str,
    attachments_count: int,
    backends_env_present: bool,
    settings_json_present: bool,
    project: dict | None = None,
    ai_provider_secret_refs: Optional[list[dict]] = None,
) -> dict:
    return {
        "schema_version": MANIFEST_SCHEMA_VERSION,
        "kassiber_version": __version__,
        "created_at": _now_iso(),
        "project": project,
        "paths": workspace_paths,
        "entries": {
            "database": db_relpath,
            "attachments_files": attachments_count,
            "backends_env": backends_env_present,
            "settings_json": settings_json_present,
        },
        "notes": {
            "inner_db_encrypted": True,
            "inner_db_passphrase_required": True,
            "scope": "single_project_container",
            "plaintext_sidecars": [
                "attachments",
                "config/backends.env",
                "config/settings.json",
            ],
            "exports_included": False,
            "logs_included": False,
        },
        "secret_refs": {
            "ai_provider_refs": ai_provider_secret_refs or [],
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


def _count_regular_files(root: Path) -> int:
    if not root.exists():
        return 0
    return sum(1 for entry in root.rglob("*") if entry.is_file())


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

    backend = age_backend or select_age_backend(
        mode="passphrase" if backup_passphrase is not None else "recipient",
    )
    state_root = Path(resolve_effective_state_root(data_root)).expanduser()
    effective_data_root = Path(resolve_effective_data_root(data_root)).expanduser()
    db_path = Path(resolve_database_path(effective_data_root)).expanduser()
    attachments_root = Path(resolve_attachments_root(data_root)).expanduser()
    backends_env = Path(resolve_config_root(data_root)).expanduser() / "backends.env"
    settings_json = Path(resolve_settings_path(data_root)).expanduser()
    exports_root = Path(resolve_exports_root(data_root)).expanduser()

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
        ai_provider_secret_refs = _collect_ai_provider_secret_refs(
            db_copy, db_passphrase
        )

        tarball_path = staging / "bundle.tar"
        attachments_count = 0
        backends_env_present = backends_env.exists()
        settings_json_present = settings_json.exists()
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
            if settings_json_present:
                settings_info = tar.gettarinfo(
                    str(settings_json), arcname=BACKUP_SETTINGS_JSON
                )
                settings_info.uid = settings_info.gid = 0
                settings_info.uname = settings_info.gname = ""
                with open(settings_json, "rb") as handle:
                    tar.addfile(settings_info, fileobj=handle)

            manifest = _build_manifest(
                workspace_paths={
                    "state_root": str(state_root),
                    "data_root": str(effective_data_root),
                    "attachments_root": str(attachments_root),
                    "backends_env": str(backends_env),
                    "settings_json": str(settings_json),
                    "exports_root": str(exports_root),
                },
                db_relpath=BACKUP_DB_NAME,
                attachments_count=attachments_count,
                backends_env_present=backends_env_present,
                settings_json_present=settings_json_present,
                project=project_metadata_for_data_root(effective_data_root),
                ai_provider_secret_refs=ai_provider_secret_refs,
            )
            manifest_bytes = (
                json.dumps(manifest, indent=2, sort_keys=True).encode("utf-8") + b"\n"
            )
            manifest_info = tarfile.TarInfo(name=BACKUP_MANIFEST_NAME)
            manifest_info.size = len(manifest_bytes)
            manifest_info.mtime = int(datetime.now(timezone.utc).timestamp())
            manifest_info.mode = 0o600
            tar.addfile(manifest_info, fileobj=BytesIO(manifest_bytes))

        tmp_output_path: Path | None = None
        try:
            with open(tarball_path, "rb") as src, tempfile.NamedTemporaryFile(
                "wb",
                dir=output_path.parent,
                prefix=f".{output_path.name}.",
                suffix=".tmp",
                delete=False,
            ) as dst:
                tmp_output_path = Path(dst.name)
                encrypt_age_stream(
                    src,
                    dst,
                    passphrase=backup_passphrase,
                    recipients=list(recipients) if recipients else None,
                    backend=backend,
                )
                dst.flush()
                os.fsync(dst.fileno())
            os.replace(tmp_output_path, output_path)
            tmp_output_path = None
        finally:
            if tmp_output_path is not None:
                tmp_output_path.unlink(missing_ok=True)

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

    backend = age_backend or select_age_backend(
        mode="passphrase" if backup_passphrase is not None else "recipient",
    )
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
        decrypted_tar.unlink()

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
        manifest_entries = (
            manifest.get("entries", {})
            if isinstance(manifest.get("entries"), dict)
            else {}
        )
        if manifest_entries.get("backends_env") and not (
            staging_dir / BACKUP_BACKENDS_ENV
        ).exists():
            raise AppError(
                "manifest declares backends_env but the file is missing from the archive",
                code="invalid_backup",
                retryable=False,
            )
        if manifest_entries.get("settings_json") and not (
            staging_dir / BACKUP_SETTINGS_JSON
        ).exists():
            raise AppError(
                "manifest declares settings_json but the file is missing from the archive",
                code="invalid_backup",
                retryable=False,
            )
        attachments_declared = manifest_entries.get("attachments_files")
        if type(attachments_declared) is not int or attachments_declared < 0:
            raise AppError(
                "manifest declares an invalid attachments_files count",
                code="invalid_backup",
                retryable=False,
            )
        attachments_actual = _count_regular_files(staging_dir / BACKUP_ATTACHMENTS_DIR)
        if attachments_actual != attachments_declared:
            raise AppError(
                "manifest attachments_files count does not match the restored archive",
                code="invalid_backup",
                details={
                    "declared": attachments_declared,
                    "actual": attachments_actual,
                },
                retryable=False,
            )
        secret_ref_unavailable = _restore_unavailable_secret_refs(manifest)

        installed_root: Optional[Path] = None
        backup_dir: Optional[Path] = None
        if move_into_place:
            target_data_root = Path(target_data_root).expanduser()
            target_data_root.mkdir(parents=True, exist_ok=True)
            installed_root = target_data_root
            # Mirror resolve_effective_state_root() but skip the legacy
            # XDG fallback: install always writes to the literal target
            # the user asked for, never to a half-discovered legacy path.
            # Without this, --target-data-root=<state>/data would dump
            # attachments/ and config/ next to the data root only when
            # the path looks like `<state>/data`; for a flat custom root
            # like `--data-root /srv/kassiber`, sidecars would land in
            # `/srv/attachments` and `/srv/config/`, outside the tree.
            if target_data_root.name == DEFAULT_DATA_DIRNAME:
                target_state_root = target_data_root.parent
            else:
                target_state_root = target_data_root
            target_db = target_data_root / BACKUP_DB_NAME
            target_attachments = target_state_root / DEFAULT_ATTACHMENTS_DIRNAME
            target_env = (
                target_state_root / DEFAULT_CONFIG_DIRNAME / "backends.env"
            )
            target_settings = (
                target_state_root / DEFAULT_CONFIG_DIRNAME / "settings.json"
            )
            target_exports = target_state_root / DEFAULT_EXPORTS_DIRNAME

            # Move any pre-existing live data into a sibling
            # `pre-restore-<timestamp>/` directory so an accidental
            # restore over a populated data root is recoverable.
            needs_backup = (
                target_db.exists()
                or target_attachments.exists()
                or target_env.exists()
                or target_settings.exists()
                or target_exports.exists()
            )
            if needs_backup:
                backup_dir = target_state_root / (
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
                if target_settings.exists():
                    backup_settings_dir = backup_dir / BACKUP_CONFIG_DIR
                    backup_settings_dir.mkdir(parents=True, exist_ok=True)
                    shutil.move(str(target_settings), str(backup_settings_dir / "settings.json"))
                if target_exports.exists():
                    shutil.move(
                        str(target_exports),
                        str(backup_dir / DEFAULT_EXPORTS_DIRNAME),
                    )

            target_db.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(staging_dir / BACKUP_DB_NAME, target_db)
            staged_attachments = staging_dir / BACKUP_ATTACHMENTS_DIR
            if staged_attachments.exists():
                target_attachments.parent.mkdir(parents=True, exist_ok=True)
                shutil.copytree(
                    staged_attachments, target_attachments, dirs_exist_ok=True
                )
            staged_env = staging_dir / BACKUP_BACKENDS_ENV
            if staged_env.exists():
                target_env.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(staged_env, target_env)
            staged_settings = staging_dir / BACKUP_SETTINGS_JSON
            if staged_settings.exists():
                target_settings.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(staged_settings, target_settings)

            try:
                shutil.rmtree(staging_parent)
            except OSError as exc:
                raise AppError(
                    "backup was installed but the decrypted temporary restore workspace could not be removed",
                    code="restore_cleanup_failed",
                    hint=f"Remove the temporary restore workspace manually: {staging_parent}",
                    details={"temporary_path": str(staging_parent), "error": str(exc)},
                    retryable=False,
                ) from None
            return BackupImportResult(
                staging_path=None,
                installed_data_root=installed_root,
                manifest=manifest,
                secret_ref_unavailable=secret_ref_unavailable,
                pre_restore_backup=backup_dir,
                temporary_artifacts_cleaned=True,
            )

        return BackupImportResult(
            staging_path=staging_dir,
            installed_data_root=None,
            manifest=manifest,
            secret_ref_unavailable=secret_ref_unavailable,
        )
    except Exception:
        shutil.rmtree(staging_parent, ignore_errors=True)
        raise
