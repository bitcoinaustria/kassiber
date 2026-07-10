"""Sealed courier/mailbox bundle serializer and strict parser."""

from __future__ import annotations

from dataclasses import dataclass
from io import BytesIO
import json
import os
from pathlib import Path
import tarfile
import tempfile
from typing import Any, Mapping

from ... import __version__
from ...backup.age_cli import AgeBackend, decrypt_age_stream, encrypt_age_stream
from ...backup.safe_tar import inspect_tar_members
from ...errors import AppError
from ...time_utils import now_iso
from .capture import capture_full_snapshot, capture_local_changes
from .crypto import canonical_json_bytes, sha256_hex, sign_canonical
from .events import version_vector


BUNDLE_SCHEMA_VERSION = 1
BUNDLE_MANIFEST_NAME = "manifest.json"
BUNDLE_EVENTS_NAME = "events.jsonl"
BUNDLE_BLOBS_DIR = "blobs"
BUNDLE_ALLOWED_TOP_LEVEL = (BUNDLE_MANIFEST_NAME, BUNDLE_EVENTS_NAME, BUNDLE_BLOBS_DIR)
MAX_BUNDLE_BYTES = 512 * 1024 * 1024
_PYRAGE_BACKEND = AgeBackend(flavor="pyrage")


@dataclass(frozen=True)
class BundleExportResult:
    ciphertext: bytes
    bundle_hash: str
    manifest: Mapping[str, Any]
    event_count: int
    first_seq: int
    last_seq: int
    captured_event_count: int


@dataclass(frozen=True)
class ParsedBundle:
    bundle_hash: str
    manifest: Mapping[str, Any]
    events: tuple[Mapping[str, Any], ...]
    blobs: Mapping[str, bytes]


def _event_from_row(row) -> dict[str, Any]:
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "replica_id": row["replica_id"],
        "replica_seq": int(row["replica_seq"]),
        "hlc": row["hlc"],
        "author_member_id": row["author_member_id"],
        "event_type": row["event_type"],
        "entity_table": row["entity_table"],
        "entity_key": row["entity_key"],
        "payload": json.loads(row["payload_json"]),
        "context": json.loads(row["context_json"]),
        "previous_hash": row["previous_hash"],
        "event_hash": row["event_hash"],
        "signature": row["signature"],
        "created_at": row["created_at"],
    }


def _membership_catalog(conn, profile_id: str) -> dict[str, list[dict[str, Any]]]:
    members = [
        {
            key: row[key]
            for key in (
                "id",
                "workspace_id",
                "profile_id",
                "display_name",
                "signing_public_key_b64",
                "role",
                "added_hlc",
                "added_at",
                "revoked_hlc",
                "revoked_at",
                "inviter_member_id",
                "record_signature",
            )
        }
        for row in conn.execute(
            "SELECT * FROM sync_members WHERE profile_id = ? ORDER BY added_hlc, id",
            (profile_id,),
        ).fetchall()
    ]
    devices = [
        {
            key: row[key]
            for key in (
                "id",
                "workspace_id",
                "profile_id",
                "member_id",
                "recipient_public_key",
                "label",
                "paired_hlc",
                "paired_at",
                "last_seen_at",
                "revoked_hlc",
                "revoked_at",
                "record_signature",
            )
        }
        for row in conn.execute(
            "SELECT * FROM sync_devices WHERE profile_id = ? ORDER BY paired_hlc, id",
            (profile_id,),
        ).fetchall()
    ]
    replicas = [
        {
            key: row[key]
            for key in ("id", "workspace_id", "profile_id", "member_id", "device_id", "created_at")
        }
        for row in conn.execute(
            "SELECT * FROM sync_replicas WHERE profile_id = ? ORDER BY id",
            (profile_id,),
        ).fetchall()
    ]
    return {"members": members, "devices": devices, "replicas": replicas}


def _safe_attachment_path(root: Path, stored_relpath: str) -> Path:
    root = root.expanduser().resolve()
    candidate = (root / stored_relpath).resolve()
    try:
        candidate.relative_to(root)
    except ValueError as exc:
        raise AppError(
            "attachment path escapes the managed attachment root",
            code="sync_attachment_unsafe",
            retryable=False,
        ) from exc
    if candidate.is_symlink() or not candidate.is_file():
        raise AppError(
            "a synced attachment file is missing or unsafe",
            code="sync_attachment_missing",
            details={"stored_relpath": stored_relpath},
            retryable=False,
        )
    return candidate


def _referenced_blobs(events: list[dict[str, Any]], attachments_root: Path | None) -> dict[str, bytes]:
    blobs: dict[str, bytes] = {}
    for event in events:
        if event["event_type"] != "row.upsert" or event["entity_table"] != "attachments":
            continue
        row = event.get("payload", {}).get("row")
        if not isinstance(row, dict) or row.get("attachment_type") != "file":
            continue
        content_hmac = row.get("content_hmac")
        stored_relpath = row.get("stored_relpath")
        if not content_hmac or not stored_relpath:
            raise AppError(
                "file attachment event is missing its sealed blob reference",
                code="sync_attachment_invalid",
                retryable=False,
            )
        if attachments_root is None:
            raise AppError(
                "attachments root is required to export file attachments",
                code="sync_attachment_root_required",
                retryable=False,
            )
        path = _safe_attachment_path(Path(attachments_root), str(stored_relpath))
        payload = path.read_bytes()
        if len(payload) > MAX_BUNDLE_BYTES:
            raise AppError(
                "attachment is too large for a sync bundle",
                code="sync_bundle_too_large",
                retryable=False,
            )
        blobs[str(content_hmac)] = payload
    return blobs


def _tar_add_bytes(tar: tarfile.TarFile, name: str, payload: bytes) -> None:
    info = tarfile.TarInfo(name=name)
    info.size = len(payload)
    info.mode = 0o600
    info.mtime = 0
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    tar.addfile(info, BytesIO(payload))


def _build_plaintext_tar(
    *,
    manifest: Mapping[str, Any],
    events_bytes: bytes,
    blobs: Mapping[str, bytes],
) -> bytes:
    output = BytesIO()
    with tarfile.open(fileobj=output, mode="w") as tar:
        _tar_add_bytes(tar, BUNDLE_MANIFEST_NAME, canonical_json_bytes(manifest))
        _tar_add_bytes(tar, BUNDLE_EVENTS_NAME, events_bytes)
        for content_hmac, payload in sorted(blobs.items()):
            _tar_add_bytes(tar, f"{BUNDLE_BLOBS_DIR}/{content_hmac}", payload)
    plaintext = output.getvalue()
    if len(plaintext) > MAX_BUNDLE_BYTES:
        raise AppError(
            "sync bundle exceeds the maximum size",
            code="sync_bundle_too_large",
            details={"max_bytes": MAX_BUNDLE_BYTES},
            retryable=False,
        )
    return plaintext


def build_bundle(
    conn,
    *,
    profile_id: str,
    attachments_root: Path | None = None,
    snapshot: bool = False,
) -> BundleExportResult | None:
    """Capture and seal the next local per-replica bundle.

    Returns ``None`` when capture found no new local events.
    """

    book = conn.execute(
        "SELECT * FROM sync_books WHERE profile_id = ? AND enabled = 1",
        (profile_id,),
    ).fetchone()
    if not book:
        raise AppError("sync is disabled", code="sync_disabled", retryable=False)
    if snapshot:
        local_member = conn.execute(
            "SELECT role, revoked_at FROM sync_members WHERE id = ?",
            (book["local_member_id"],),
        ).fetchone()
        if not local_member or local_member["role"] != "owner" or local_member["revoked_at"]:
            raise AppError(
                "only an active owner can author a snapshot bundle",
                code="sync_role_denied",
            )
    captured = capture_local_changes(conn, profile_id=profile_id)
    snapshot_base = None
    if snapshot:
        snapshot_base = {
            row["id"]: {
                "last_seq": int(row["last_seq"] or 0),
                "last_hlc": row["last_hlc"],
                "last_event_hash": row["last_event_hash"],
            }
            for row in conn.execute(
                "SELECT * FROM sync_replicas WHERE profile_id = ? ORDER BY id",
                (profile_id,),
            ).fetchall()
        }
        captured.extend(capture_full_snapshot(conn, profile_id=profile_id))
    export_state = conn.execute(
        "SELECT * FROM sync_bundle_exports WHERE profile_id = ? AND replica_id = ?",
        (profile_id, book["local_replica_id"]),
    ).fetchone()
    after_seq = (
        int(snapshot_base[book["local_replica_id"]]["last_seq"])
        if snapshot_base is not None
        else (int(export_state["last_seq"] or 0) if export_state else 0)
    )
    rows = conn.execute(
        """
        SELECT * FROM sync_events
        WHERE profile_id = ? AND replica_id = ? AND replica_seq > ?
        ORDER BY replica_seq
        """,
        (profile_id, book["local_replica_id"], after_seq),
    ).fetchall()
    if not rows:
        return None
    events = [_event_from_row(row) for row in rows]
    events_bytes = b"".join(canonical_json_bytes(event) + b"\n" for event in events)
    blobs = _referenced_blobs(events, attachments_root)
    recipients = [
        row["recipient_public_key"]
        for row in conn.execute(
            "SELECT recipient_public_key FROM sync_devices WHERE profile_id = ? AND revoked_at IS NULL ORDER BY id",
            (profile_id,),
        ).fetchall()
    ]
    if not recipients:
        raise AppError(
            "sync book has no active device recipients",
            code="sync_identity_incomplete",
            retryable=False,
        )
    first_seq = int(rows[0]["replica_seq"])
    last_seq = int(rows[-1]["replica_seq"])
    timestamp = now_iso()
    manifest = {
        "schema_version": BUNDLE_SCHEMA_VERSION,
        "kassiber_version": __version__,
        "book_id": book["book_id"],
        "workspace_id": book["workspace_id"],
        "profile_id": profile_id,
        "sender_replica_id": book["local_replica_id"],
        "sender_member_id": book["local_member_id"],
        "bundle_kind": "snapshot" if snapshot else "incremental",
        "first_seq": first_seq,
        "last_seq": last_seq,
        "version_vector": version_vector(conn, profile_id),
        "prior_bundle_hash": export_state["last_bundle_hash"] if export_state else None,
        "events_sha256": sha256_hex(events_bytes),
        "event_count": len(events),
        "blob_hmacs": sorted(blobs),
        "membership": _membership_catalog(conn, profile_id),
        "snapshot_base": snapshot_base,
        "created_at": timestamp,
    }
    private_key = conn.execute(
        "SELECT signing_private_key_b64 FROM sync_member_private_keys WHERE member_id = ?",
        (book["local_member_id"],),
    ).fetchone()
    if not private_key:
        raise AppError("local signing key is missing", code="sync_identity_incomplete")
    manifest["manifest_signature"] = sign_canonical(
        private_key["signing_private_key_b64"], manifest
    )
    plaintext = _build_plaintext_tar(manifest=manifest, events_bytes=events_bytes, blobs=blobs)
    encrypted = BytesIO()
    encrypt_age_stream(
        BytesIO(plaintext),
        encrypted,
        recipients=recipients,
        backend=_PYRAGE_BACKEND,
    )
    ciphertext = encrypted.getvalue()
    bundle_hash = sha256_hex(ciphertext)
    conn.execute(
        """
        INSERT INTO sync_bundle_exports(profile_id, replica_id, last_seq, last_bundle_hash, exported_at)
        VALUES(?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, replica_id) DO UPDATE SET
            last_seq = excluded.last_seq,
            last_bundle_hash = excluded.last_bundle_hash,
            exported_at = excluded.exported_at
        """,
        (profile_id, book["local_replica_id"], last_seq, bundle_hash, timestamp),
    )
    return BundleExportResult(
        ciphertext=ciphertext,
        bundle_hash=bundle_hash,
        manifest=manifest,
        event_count=len(events),
        first_seq=first_seq,
        last_seq=last_seq,
        captured_event_count=len(captured),
    )


def write_bundle_atomic(result: BundleExportResult, output_path: Path) -> Path:
    output = Path(output_path).expanduser().resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary_name = tempfile.mkstemp(prefix=f".{output.name}.", dir=output.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(result.ciphertext)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, output)
    except Exception:
        temporary.unlink(missing_ok=True)
        raise
    return output


def parse_bundle(ciphertext: bytes, *, age_identity: str) -> ParsedBundle:
    if not ciphertext or len(ciphertext) > MAX_BUNDLE_BYTES:
        raise AppError(
            "sync bundle is empty or too large",
            code="sync_bundle_invalid",
            retryable=False,
        )
    plaintext = BytesIO()
    try:
        decrypt_age_stream(
            BytesIO(ciphertext),
            plaintext,
            identity=age_identity,
            backend=_PYRAGE_BACKEND,
        )
    except AppError:
        raise
    except Exception as exc:
        raise AppError(
            "sync bundle could not be decrypted",
            code="sync_bundle_decrypt_failed",
            retryable=False,
        ) from exc

    entries: dict[str, bytes] = {}
    try:
        with tarfile.open(fileobj=BytesIO(plaintext.getvalue()), mode="r:*") as tar:
            members = tar.getmembers()
            inspect_tar_members(
                members,
                allowed_top_level=BUNDLE_ALLOWED_TOP_LEVEL,
                max_member_bytes=MAX_BUNDLE_BYTES,
            )
            for member in members:
                if not member.isfile():
                    continue
                extracted = tar.extractfile(member)
                if extracted is None:
                    raise AppError("sync bundle member could not be read", code="sync_bundle_invalid")
                entries[member.name.lstrip("./")] = extracted.read()
    except AppError:
        raise
    except Exception as exc:
        raise AppError(
            "sync bundle is not a valid safe tar archive",
            code="sync_bundle_invalid",
            retryable=False,
        ) from exc

    if BUNDLE_MANIFEST_NAME not in entries or BUNDLE_EVENTS_NAME not in entries:
        raise AppError(
            "sync bundle is missing its manifest or event stream",
            code="sync_bundle_invalid",
            retryable=False,
        )
    try:
        manifest = json.loads(entries[BUNDLE_MANIFEST_NAME].decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AppError("sync manifest is invalid", code="sync_bundle_invalid") from exc
    if manifest.get("schema_version") != BUNDLE_SCHEMA_VERSION:
        raise AppError(
            "sync bundle schema version is not supported",
            code="sync_bundle_version_unsupported",
            details={"schema_version": manifest.get("schema_version")},
            retryable=False,
        )
    events_bytes = entries[BUNDLE_EVENTS_NAME]
    if sha256_hex(events_bytes) != manifest.get("events_sha256"):
        raise AppError(
            "sync event stream hash does not match the manifest",
            code="sync_bundle_tampered",
            retryable=False,
        )
    events: list[Mapping[str, Any]] = []
    for line_number, line in enumerate(events_bytes.splitlines(), start=1):
        if not line.strip():
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError as exc:
            raise AppError(
                "sync event stream contains invalid JSON",
                code="sync_bundle_invalid",
                details={"line": line_number},
                retryable=False,
            ) from exc
        if not isinstance(event, dict):
            raise AppError("sync event must be a JSON object", code="sync_bundle_invalid")
        events.append(event)
    if len(events) != int(manifest.get("event_count", -1)):
        raise AppError(
            "sync event count does not match the manifest",
            code="sync_bundle_tampered",
            retryable=False,
        )
    blobs = {
        name.split("/", 1)[1]: payload
        for name, payload in entries.items()
        if name.startswith(f"{BUNDLE_BLOBS_DIR}/") and "/" in name
    }
    if sorted(blobs) != sorted(manifest.get("blob_hmacs") or []):
        raise AppError(
            "sync blob inventory does not match the manifest",
            code="sync_bundle_tampered",
            retryable=False,
        )
    return ParsedBundle(
        bundle_hash=sha256_hex(ciphertext),
        manifest=manifest,
        events=tuple(events),
        blobs=blobs,
    )
