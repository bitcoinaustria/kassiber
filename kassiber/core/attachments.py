from __future__ import annotations

import hashlib
import mimetypes
import sqlite3
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Mapping
from urllib.parse import urlparse

from ..db import ensure_data_root, resolve_attachments_root
from ..errors import AppError

ScopeResolver = Callable[[sqlite3.Connection, str | None, str | None], tuple[Mapping[str, Any], Mapping[str, Any]]]
TransactionResolver = Callable[..., Mapping[str, Any]]
NowIso = Callable[[], str]


@dataclass(frozen=True)
class AttachmentHooks:
    resolve_scope: ScopeResolver
    resolve_transaction: TransactionResolver
    now_iso: NowIso


def _attachments_root(data_root: str) -> Path:
    return ensure_data_root(resolve_attachments_root(data_root))


def _sanitize_filename(name: str | None) -> str:
    raw = Path(name or "").name.strip()
    if not raw:
        return "attachment.bin"
    sanitized = []
    for char in raw:
        if char.isalnum() or char in {".", "_", "-"}:
            sanitized.append(char)
        else:
            sanitized.append("_")
    collapsed = "".join(sanitized).strip("._")
    return collapsed or "attachment.bin"


def _attachment_storage_path(root: Path, profile_id: str, attachment_id: str, original_name: str | None) -> tuple[Path, str]:
    safe_name = _sanitize_filename(original_name)
    profile_dir = ensure_data_root(root / profile_id)
    path = profile_dir / f"{attachment_id}-{safe_name}"
    return path, path.relative_to(root).as_posix()


def _hash_and_copy_file(source: Path, destination: Path) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    with source.open("rb") as src, destination.open("wb") as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
            digest.update(chunk)
            size += len(chunk)
    return size, digest.hexdigest()


def _hash_file(path: Path) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
            size += len(chunk)
    return size, digest.hexdigest()


def _resolve_stored_path(attachments_root: Path, stored_relpath: str | None) -> tuple[Path | None, bool]:
    raw = (stored_relpath or "").strip()
    if not raw:
        return None, True
    relpath = Path(raw)
    if relpath.is_absolute():
        return None, False
    root = attachments_root.resolve()
    candidate = (attachments_root / relpath).resolve(strict=False)
    try:
        candidate.relative_to(root)
    except ValueError:
        return None, False
    return candidate, True


def _attachment_row_to_dict(row: Mapping[str, Any], attachments_root: Path) -> dict[str, Any]:
    stored_relpath = row["stored_relpath"] or None
    stored_path, path_valid = _resolve_stored_path(attachments_root, stored_relpath)
    exists = stored_path.exists() if stored_path else (False if stored_relpath and not path_valid else None)
    return {
        "id": row["id"],
        "transaction_id": row["transaction_id"],
        "external_id": row["external_id"] or "",
        "wallet": row["wallet"] or "",
        "occurred_at": row["occurred_at"] or "",
        "asset": row["asset"] or "",
        "attachment_type": row["attachment_type"],
        "label": row["label"],
        "original_filename": row["original_filename"] or "",
        "url": row["source_url"] or "",
        "media_type": row["media_type"] or "",
        "size_bytes": int(row["size_bytes"]) if row["size_bytes"] is not None else None,
        "sha256": row["sha256"] or "",
        "stored_relpath": stored_relpath or "",
        "copied_from_attachment_id": row["copied_from_attachment_id"] or "",
        "copied_from_transaction_id": row["copied_from_transaction_id"] or "",
        "exists": exists,
        "created_at": row["created_at"],
    }


def _select_attachment_row(conn, profile_id: str, attachment_id: str):
    return conn.execute(
        """
        SELECT
            a.*,
            t.external_id,
            t.occurred_at,
            t.asset,
            w.label AS wallet
        FROM attachments a
        LEFT JOIN transactions t ON t.id = a.transaction_id
        LEFT JOIN wallets w ON w.id = t.wallet_id
        WHERE a.profile_id = ? AND a.id = ?
        """,
        (profile_id, attachment_id),
    ).fetchone()


def add_attachment(
    conn,
    data_root: str,
    workspace_ref: str | None,
    profile_ref: str | None,
    tx_ref: str,
    hooks: AttachmentHooks,
    *,
    file_path: str | None = None,
    url: str | None = None,
    label: str | None = None,
    media_type: str | None = None,
):
    if bool(file_path) == bool(url):
        raise AppError("Provide exactly one of --file or --url", code="validation")
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
    attachment_id = str(uuid.uuid4())
    created_at = hooks.now_iso()
    attachments_root = _attachments_root(data_root)
    attachment_type = "url" if url else "file"
    original_filename = None
    stored_relpath = None
    source_url = None
    size_bytes = None
    sha256 = None
    destination = None

    if file_path:
        source = Path(file_path).expanduser()
        if not source.exists():
            raise AppError(f"Attachment file '{file_path}' not found", code="not_found")
        if not source.is_file():
            raise AppError(f"Attachment path '{file_path}' is not a file", code="validation")
        original_filename = source.name
        destination, stored_relpath = _attachment_storage_path(
            attachments_root,
            profile["id"],
            attachment_id,
            original_filename,
        )
        size_bytes, sha256 = _hash_and_copy_file(source, destination)
        inferred_media_type = mimetypes.guess_type(source.name)[0]
        media_type = media_type or inferred_media_type or "application/octet-stream"
        label = label or original_filename
    else:
        parsed = urlparse(url or "")
        if not parsed.scheme:
            raise AppError("--url must include a scheme such as https://", code="validation")
        source_url = str(url)
        label = label or source_url
        media_type = media_type or "text/uri-list"

    try:
        conn.execute(
            """
            INSERT INTO attachments(
                id, workspace_id, profile_id, transaction_id, attachment_type, label,
                original_filename, stored_relpath, source_url, media_type,
                size_bytes, sha256, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                attachment_id,
                workspace["id"],
                profile["id"],
                tx["id"],
                attachment_type,
                label,
                original_filename,
                stored_relpath,
                source_url,
                media_type,
                size_bytes,
                sha256,
                created_at,
            ),
        )
        conn.commit()
    except Exception:
        if destination is not None:
            try:
                destination.unlink()
            except OSError:
                pass
        raise

    row = _select_attachment_row(conn, profile["id"], attachment_id)
    return _attachment_row_to_dict(row, attachments_root)


def copy_attachments(
    conn,
    data_root: str,
    workspace_ref: str | None,
    profile_ref: str | None,
    target_tx_ref: str,
    attachment_ids: list[str],
    hooks: AttachmentHooks,
    *,
    source_tx_ref: str | None = None,
):
    if not attachment_ids:
        raise AppError("Select at least one attachment to copy", code="validation")
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    target_tx = hooks.resolve_transaction(conn, profile["id"], target_tx_ref)
    source_tx = (
        hooks.resolve_transaction(conn, profile["id"], source_tx_ref)
        if source_tx_ref
        else None
    )
    if source_tx and source_tx["id"] == target_tx["id"]:
        raise AppError(
            "Choose a different source transaction for evidence reuse",
            code="validation",
        )

    cleaned_ids = []
    seen = set()
    for attachment_id in attachment_ids:
        if not isinstance(attachment_id, str) or not attachment_id.strip():
            raise AppError("attachment_ids must be non-empty strings", code="validation")
        normalized = attachment_id.strip()
        if normalized not in seen:
            cleaned_ids.append(normalized)
            seen.add(normalized)

    placeholders = ",".join("?" for _ in cleaned_ids)
    where = [
        "a.profile_id = ?",
        f"a.id IN ({placeholders})",
        "a.transaction_id IS NOT NULL",
    ]
    params: list[Any] = [profile["id"], *cleaned_ids]
    if source_tx:
        where.append("a.transaction_id = ?")
        params.append(source_tx["id"])
    rows = conn.execute(
        f"""
        SELECT a.*
        FROM attachments a
        WHERE {' AND '.join(where)}
        """,
        params,
    ).fetchall()
    rows_by_id = {row["id"]: row for row in rows}
    missing_ids = [
        attachment_id
        for attachment_id in cleaned_ids
        if attachment_id not in rows_by_id
    ]
    if missing_ids:
        raise AppError(
            "One or more source attachments were not found",
            code="not_found",
            details={"attachment_ids": missing_ids},
        )

    attachments_root = _attachments_root(data_root)
    created_paths: list[Path] = []
    new_ids: list[str] = []
    created_at = hooks.now_iso()
    conn.execute("SAVEPOINT attachment_copy")
    try:
        for source in (rows_by_id[attachment_id] for attachment_id in cleaned_ids):
            new_id = str(uuid.uuid4())
            new_ids.append(new_id)
            stored_relpath = None
            size_bytes = source["size_bytes"]
            sha256 = source["sha256"]
            if source["attachment_type"] == "file":
                source_path, path_valid = _resolve_stored_path(
                    attachments_root,
                    source["stored_relpath"],
                )
                if not path_valid or source_path is None:
                    raise AppError(
                        "Source attachment has an invalid managed storage path",
                        code="validation",
                        details={"attachment_id": source["id"]},
                    )
                if not source_path.exists():
                    raise AppError(
                        "Source attachment file is missing",
                        code="not_found",
                        details={"attachment_id": source["id"]},
                    )
                destination, stored_relpath = _attachment_storage_path(
                    attachments_root,
                    profile["id"],
                    new_id,
                    source["original_filename"] or source["label"],
                )
                size_bytes, sha256 = _hash_and_copy_file(source_path, destination)
                created_paths.append(destination)
                if (
                    source["size_bytes"] is not None
                    and int(source["size_bytes"]) != size_bytes
                ):
                    raise AppError(
                        "Source attachment size does not match its stored metadata",
                        code="validation",
                        details={"attachment_id": source["id"]},
                    )
                if source["sha256"] and source["sha256"] != sha256:
                    raise AppError(
                        "Source attachment hash does not match its stored metadata",
                        code="validation",
                        details={"attachment_id": source["id"]},
                    )
            conn.execute(
                """
                INSERT INTO attachments(
                    id, workspace_id, profile_id, transaction_id, attachment_type, label,
                    original_filename, stored_relpath, source_url, media_type,
                    size_bytes, sha256, copied_from_attachment_id,
                    copied_from_transaction_id, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    new_id,
                    workspace["id"],
                    profile["id"],
                    target_tx["id"],
                    source["attachment_type"],
                    source["label"],
                    source["original_filename"],
                    stored_relpath,
                    source["source_url"],
                    source["media_type"],
                    size_bytes,
                    sha256,
                    source["id"],
                    source["transaction_id"],
                    created_at,
                ),
            )
        conn.execute("RELEASE SAVEPOINT attachment_copy")
        conn.commit()
    except Exception:
        conn.execute("ROLLBACK TO SAVEPOINT attachment_copy")
        conn.execute("RELEASE SAVEPOINT attachment_copy")
        for path in created_paths:
            try:
                path.unlink()
                _prune_empty_dirs(attachments_root, path)
            except OSError:
                # Best-effort rollback cleanup must not mask the original copy error.
                pass
        raise

    copied = [
        _attachment_row_to_dict(
            _select_attachment_row(conn, profile["id"], new_id),
            attachments_root,
        )
        for new_id in new_ids
    ]
    return {
        "source_transaction_id": source_tx["id"] if source_tx else "",
        "target_transaction_id": target_tx["id"],
        "attachments": copied,
        "copied": len(copied),
    }


def list_attachments(
    conn,
    data_root: str,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: AttachmentHooks,
    *,
    tx_ref: str | None = None,
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    where = ["a.profile_id = ?"]
    params: list[Any] = [profile["id"]]
    if tx_ref:
        tx = hooks.resolve_transaction(conn, profile["id"], tx_ref)
        where.append("a.transaction_id = ?")
        params.append(tx["id"])
    rows = conn.execute(
        f"""
        SELECT
            a.*,
            t.external_id,
            t.occurred_at,
            t.asset,
            w.label AS wallet
        FROM attachments a
        LEFT JOIN transactions t ON t.id = a.transaction_id
        LEFT JOIN wallets w ON w.id = t.wallet_id
        WHERE {' AND '.join(where)}
        ORDER BY a.created_at DESC, a.id DESC
        """,
        params,
    ).fetchall()
    attachments_root = _attachments_root(data_root)
    return [_attachment_row_to_dict(row, attachments_root) for row in rows]


def _prune_empty_dirs(root: Path, starting_path: Path):
    current = starting_path.parent
    while current != root and current.is_dir():
        try:
            current.rmdir()
        except OSError:
            break
        current = current.parent


def remove_attachment(
    conn,
    data_root: str,
    workspace_ref: str | None,
    profile_ref: str | None,
    attachment_id: str,
    hooks: AttachmentHooks,
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        """
        SELECT
            a.*,
            t.external_id,
            t.occurred_at,
            t.asset,
            w.label AS wallet
        FROM attachments a
        LEFT JOIN transactions t ON t.id = a.transaction_id
        LEFT JOIN wallets w ON w.id = t.wallet_id
        WHERE a.profile_id = ? AND a.id = ?
        """,
        (profile["id"], attachment_id),
    ).fetchone()
    if not row:
        raise AppError(f"Attachment '{attachment_id}' not found", code="not_found")
    attachments_root = _attachments_root(data_root)
    attachment = _attachment_row_to_dict(row, attachments_root)
    stored_path, _ = _resolve_stored_path(attachments_root, attachment["stored_relpath"])
    deleted_file = False
    conn.execute("SAVEPOINT attachment_remove")
    try:
        conn.execute("DELETE FROM attachments WHERE id = ?", (attachment_id,))
        if stored_path and stored_path.exists():
            stored_path.unlink()
            deleted_file = True
            _prune_empty_dirs(attachments_root, stored_path)
        conn.execute("RELEASE SAVEPOINT attachment_remove")
    except OSError as exc:
        conn.execute("ROLLBACK TO SAVEPOINT attachment_remove")
        conn.execute("RELEASE SAVEPOINT attachment_remove")
        raise AppError(
            f"Could not delete attachment file '{attachment['stored_relpath']}': {exc}",
            code="filesystem_error",
            hint="Fix the local file permissions and retry `attachments remove`.",
            retryable=True,
        ) from exc
    except Exception:
        conn.execute("ROLLBACK TO SAVEPOINT attachment_remove")
        conn.execute("RELEASE SAVEPOINT attachment_remove")
        raise
    conn.commit()
    attachment["removed"] = True
    attachment["deleted_file"] = deleted_file
    return attachment


def rename_attachment(
    conn,
    data_root: str,
    workspace_ref: str | None,
    profile_ref: str | None,
    attachment_id: str,
    label: str,
    hooks: AttachmentHooks,
):
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    clean_label = (label or "").strip()
    if not clean_label:
        raise AppError("Attachment label cannot be empty", code="validation")
    row = _select_attachment_row(conn, profile["id"], attachment_id)
    if not row:
        raise AppError(f"Attachment '{attachment_id}' not found", code="not_found")
    conn.execute(
        "UPDATE attachments SET label = ? WHERE profile_id = ? AND id = ?",
        (clean_label, profile["id"], attachment_id),
    )
    conn.commit()
    updated = _select_attachment_row(conn, profile["id"], attachment_id)
    return _attachment_row_to_dict(updated, _attachments_root(data_root))


def verify_attachments(
    conn,
    data_root: str,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: AttachmentHooks,
    *,
    tx_ref: str | None = None,
):
    attachments_root = _attachments_root(data_root)
    rows = list_attachments(conn, data_root, workspace_ref, profile_ref, hooks, tx_ref=tx_ref)
    results = []
    broken = 0
    for row in rows:
        issues: list[str] = []
        if row["attachment_type"] == "url":
            status = "ok"
        else:
            stored_path, path_valid = _resolve_stored_path(attachments_root, row["stored_relpath"])
            if not path_valid:
                issues.append("invalid_storage_path")
            elif stored_path is None or not stored_path.exists():
                issues.append("missing_file")
            else:
                size_bytes, sha256 = _hash_file(stored_path)
                if row["size_bytes"] is not None and size_bytes != row["size_bytes"]:
                    issues.append("size_mismatch")
                if row["sha256"] and sha256 != row["sha256"]:
                    issues.append("sha256_mismatch")
            status = "ok" if not issues else "broken"
        if issues:
            broken += 1
        result = dict(row)
        result["status"] = status
        result["issues"] = issues
        results.append(result)
    return {
        "checked": len(results),
        "broken": broken,
        "ok": len(results) - broken,
        "results": results,
    }


def gc_attachments(conn, data_root: str, *, dry_run: bool = False):
    attachments_root = _attachments_root(data_root)
    live_paths = {
        row["stored_relpath"]
        for row in conn.execute(
            "SELECT stored_relpath FROM attachments WHERE stored_relpath IS NOT NULL AND stored_relpath != ''"
        ).fetchall()
    }
    orphan_files = []
    orphan_paths: list[Path] = []
    total_bytes = 0
    for path in attachments_root.rglob("*"):
        if not path.is_file():
            continue
        relpath = path.relative_to(attachments_root).as_posix()
        if relpath in live_paths:
            continue
        size_bytes = path.stat().st_size
        orphan_files.append({"stored_relpath": relpath, "size_bytes": size_bytes})
        orphan_paths.append(path)
        total_bytes += size_bytes
    removed_files = 0
    removed_bytes = 0
    if not dry_run:
        for path, row in zip(orphan_paths, orphan_files):
            size_bytes = row["size_bytes"]
            path.unlink()
            removed_files += 1
            removed_bytes += size_bytes
            _prune_empty_dirs(attachments_root, path)
    return {
        "attachments_root": str(attachments_root),
        "dry_run": bool(dry_run),
        "orphaned_files": len(orphan_files),
        "orphaned_bytes": total_bytes,
        "removed_files": removed_files,
        "removed_bytes": removed_bytes,
        "files": orphan_files,
    }


__all__ = [
    "AttachmentHooks",
    "add_attachment",
    "copy_attachments",
    "gc_attachments",
    "list_attachments",
    "rename_attachment",
    "remove_attachment",
    "verify_attachments",
]
