"""Opt-in book identity, membership, device, and local replica lifecycle."""

from __future__ import annotations

import sqlite3
import uuid

from ...errors import AppError
from ...time_utils import now_iso
from .clock import tick_clock
from .crypto import (
    encode_secret,
    generate_device_keypair,
    generate_signing_keypair,
    random_book_key,
    sign_canonical,
)
from .events import author_event, version_vector


SYNC_ROLES = frozenset({"owner", "editor", "auditor"})


def connection_is_encrypted(conn: sqlite3.Connection) -> bool:
    try:
        row = conn.execute("PRAGMA cipher_version").fetchone()
    except sqlite3.DatabaseError:
        return False
    return bool(row and row[0])


def _scope_rows(conn: sqlite3.Connection, workspace_id: str, profile_id: str):
    workspace = conn.execute(
        "SELECT * FROM workspaces WHERE id = ?",
        (workspace_id,),
    ).fetchone()
    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ? AND workspace_id = ?",
        (profile_id, workspace_id),
    ).fetchone()
    if not workspace or not profile:
        raise AppError(
            "sync book scope was not found",
            code="not_found",
            details={"workspace_id": workspace_id, "profile_id": profile_id},
            retryable=False,
        )
    return workspace, profile


def enable_sync(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    member_name: str,
    device_label: str,
) -> dict:
    """Create the first owner/member/device identity for one encrypted book."""

    _scope_rows(conn, workspace_id, profile_id)
    if not connection_is_encrypted(conn):
        raise AppError(
            "sync keys require an encrypted SQLCipher database",
            code="sync_requires_encrypted_database",
            hint="Run `kassiber secrets init`, verify the encrypted database, then enable sync.",
            retryable=False,
        )
    existing = conn.execute(
        "SELECT * FROM sync_books WHERE profile_id = ?",
        (profile_id,),
    ).fetchone()
    timestamp = now_iso()
    if existing:
        conn.execute(
            "UPDATE sync_books SET enabled = 1, updated_at = ? WHERE profile_id = ?",
            (timestamp, profile_id),
        )
        return sync_status(conn, profile_id=profile_id)

    cleaned_member_name = str(member_name or "").strip()
    cleaned_device_label = str(device_label or "").strip()
    if not cleaned_member_name or not cleaned_device_label:
        raise AppError(
            "member name and device label are required",
            code="validation",
            retryable=False,
        )

    member_id = str(uuid.uuid4())
    device_id = str(uuid.uuid4())
    replica_id = str(uuid.uuid4())
    book_id = str(uuid.uuid4())
    signing = generate_signing_keypair()
    device = generate_device_keypair()
    bootstrap_hlc = tick_clock(None, replica_id).encode()
    member_record = {
        "id": member_id,
        "profile_id": profile_id,
        "display_name": cleaned_member_name,
        "signing_public_key_b64": signing.public_key_b64,
        "role": "owner",
        "added_hlc": bootstrap_hlc,
        "inviter_member_id": member_id,
    }
    member_signature = sign_canonical(signing.private_key_b64, member_record)
    device_record = {
        "id": device_id,
        "profile_id": profile_id,
        "member_id": member_id,
        "recipient_public_key": device.recipient,
        "label": cleaned_device_label,
        "paired_hlc": bootstrap_hlc,
    }
    device_signature = sign_canonical(
        signing.private_key_b64,
        {
            "id": device_id,
            "member_id": member_id,
            "recipient_public_key": device.recipient,
            "label": cleaned_device_label,
        },
    )

    conn.execute(
        """
        INSERT INTO sync_books(
            profile_id, workspace_id, book_id, enabled, local_member_id,
            local_device_id, local_replica_id, hmac_key_b64, created_at, updated_at
        ) VALUES(?, ?, ?, 1, ?, ?, ?, ?, ?, ?)
        """,
        (
            profile_id,
            workspace_id,
            book_id,
            member_id,
            device_id,
            replica_id,
            encode_secret(random_book_key()),
            timestamp,
            timestamp,
        ),
    )
    conn.execute(
        """
        INSERT INTO sync_members(
            id, workspace_id, profile_id, display_name, signing_public_key_b64,
            role, added_hlc, added_at, inviter_member_id, record_signature
        ) VALUES(?, ?, ?, ?, ?, 'owner', ?, ?, ?, ?)
        """,
        (
            member_id,
            workspace_id,
            profile_id,
            cleaned_member_name,
            signing.public_key_b64,
            bootstrap_hlc,
            timestamp,
            member_id,
            member_signature,
        ),
    )
    conn.execute(
        "INSERT INTO sync_member_private_keys(member_id, signing_private_key_b64, created_at) VALUES(?, ?, ?)",
        (member_id, signing.private_key_b64, timestamp),
    )
    conn.execute(
        """
        INSERT INTO sync_devices(
            id, workspace_id, profile_id, member_id, recipient_public_key,
            label, paired_hlc, paired_at, last_seen_at, record_signature
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            device_id,
            workspace_id,
            profile_id,
            member_id,
            device.recipient,
            cleaned_device_label,
            bootstrap_hlc,
            timestamp,
            timestamp,
            device_signature,
        ),
    )
    conn.execute(
        "INSERT INTO sync_device_private_keys(device_id, age_identity, created_at) VALUES(?, ?, ?)",
        (device_id, device.age_identity, timestamp),
    )
    conn.execute(
        """
        INSERT INTO sync_replicas(
            id, workspace_id, profile_id, member_id, device_id,
            last_seq, last_hlc, last_event_hash, last_seen_at, created_at
        ) VALUES(?, ?, ?, ?, ?, 0, NULL, NULL, ?, ?)
        """,
        (replica_id, workspace_id, profile_id, member_id, device_id, timestamp, timestamp),
    )
    author_event(
        conn,
        profile_id=profile_id,
        event_type="membership.root",
        entity_table="sync_members",
        entity_key=member_id,
        payload=member_record | {"record_signature": member_signature},
        created_at=timestamp,
    )
    author_event(
        conn,
        profile_id=profile_id,
        event_type="device.add",
        entity_table="sync_devices",
        entity_key=device_id,
        payload=device_record | {"record_signature": device_signature},
        created_at=timestamp,
    )
    return sync_status(conn, profile_id=profile_id)


def disable_sync(conn: sqlite3.Connection, *, profile_id: str) -> dict:
    timestamp = now_iso()
    cursor = conn.execute(
        "UPDATE sync_books SET enabled = 0, updated_at = ? WHERE profile_id = ?",
        (timestamp, profile_id),
    )
    if not cursor.rowcount:
        raise AppError("sync is not configured for this book", code="not_found", retryable=False)
    return sync_status(conn, profile_id=profile_id)


def sync_status(conn: sqlite3.Connection, *, profile_id: str) -> dict:
    book = conn.execute(
        "SELECT * FROM sync_books WHERE profile_id = ?",
        (profile_id,),
    ).fetchone()
    if not book:
        return {
            "configured": False,
            "enabled": False,
            "members": 0,
            "devices": 0,
            "open_conflicts": 0,
            "version_vector": {},
        }
    counts = conn.execute(
        """
        SELECT
            (SELECT COUNT(*) FROM sync_members WHERE profile_id = ? AND revoked_at IS NULL) AS members,
            (SELECT COUNT(*) FROM sync_devices WHERE profile_id = ? AND revoked_at IS NULL) AS devices,
            (SELECT COUNT(*) FROM sync_conflicts WHERE profile_id = ? AND status = 'open') AS conflicts
        """,
        (profile_id, profile_id, profile_id),
    ).fetchone()
    return {
        "configured": True,
        "enabled": bool(book["enabled"]),
        "book_id": book["book_id"],
        "local_member_id": book["local_member_id"],
        "local_device_id": book["local_device_id"],
        "local_replica_id": book["local_replica_id"],
        "members": int(counts["members"]),
        "devices": int(counts["devices"]),
        "open_conflicts": int(counts["conflicts"]),
        "version_vector": version_vector(conn, profile_id),
    }
