"""Creation and verification of per-replica signed authored events."""

from __future__ import annotations

from dataclasses import dataclass
import json
import sqlite3
import uuid
from typing import Any, Mapping

from ...errors import AppError
from ...time_utils import now_iso
from .clock import HybridLogicalClock, tick_clock
from .crypto import event_hash, sign_domain_bytes, verify_domain_bytes, verify_bytes


EVENT_SIGNATURE_DOMAIN = "event-v1"


@dataclass(frozen=True)
class AuthoredEvent:
    id: str
    workspace_id: str
    profile_id: str
    replica_id: str
    replica_seq: int
    hlc: str
    author_member_id: str
    event_type: str
    entity_table: str
    entity_key: str
    payload: Mapping[str, Any]
    context: Mapping[str, int]
    previous_hash: str | None
    event_hash: str
    signature: str
    created_at: str

    def to_wire_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "workspace_id": self.workspace_id,
            "profile_id": self.profile_id,
            "replica_id": self.replica_id,
            "replica_seq": self.replica_seq,
            "hlc": self.hlc,
            "author_member_id": self.author_member_id,
            "event_type": self.event_type,
            "entity_table": self.entity_table,
            "entity_key": self.entity_key,
            "payload": dict(self.payload),
            "context": dict(self.context),
            "previous_hash": self.previous_hash,
            "event_hash": self.event_hash,
            "signature": self.signature,
            "created_at": self.created_at,
        }


def _event_core(
    *,
    event_id: str,
    workspace_id: str,
    profile_id: str,
    replica_id: str,
    replica_seq: int,
    hlc: str,
    author_member_id: str,
    event_type: str,
    entity_table: str,
    entity_key: str,
    payload: Mapping[str, Any],
    context: Mapping[str, int],
    previous_hash: str | None,
    created_at: str,
) -> dict[str, Any]:
    return {
        "id": event_id,
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "replica_id": replica_id,
        "replica_seq": replica_seq,
        "hlc": hlc,
        "author_member_id": author_member_id,
        "event_type": event_type,
        "entity_table": entity_table,
        "entity_key": entity_key,
        "payload": dict(payload),
        "context": {key: int(value) for key, value in sorted(context.items())},
        "previous_hash": previous_hash,
        "created_at": created_at,
    }


def sync_enabled(conn: sqlite3.Connection, profile_id: str) -> bool:
    row = conn.execute(
        "SELECT enabled FROM sync_books WHERE profile_id = ?",
        (profile_id,),
    ).fetchone()
    return bool(row and row["enabled"])


def version_vector(conn: sqlite3.Connection, profile_id: str) -> dict[str, int]:
    rows = conn.execute(
        "SELECT id, last_seq FROM sync_replicas WHERE profile_id = ? ORDER BY id",
        (profile_id,),
    ).fetchall()
    return {row["id"]: int(row["last_seq"] or 0) for row in rows}


def author_event(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    event_type: str,
    entity_table: str,
    entity_key: str,
    payload: Mapping[str, Any],
    created_at: str | None = None,
    allow_disabled: bool = False,
) -> AuthoredEvent | None:
    """Sign and append one event, or return ``None`` when sync is disabled.

    The caller owns the surrounding SQL transaction. Event insertion and the
    authored mutation therefore commit or roll back together.
    """

    book = conn.execute(
        "SELECT * FROM sync_books WHERE profile_id = ?",
        (profile_id,),
    ).fetchone()
    if not book or (not book["enabled"] and not allow_disabled):
        return None
    replica = conn.execute(
        "SELECT * FROM sync_replicas WHERE id = ? AND profile_id = ?",
        (book["local_replica_id"], profile_id),
    ).fetchone()
    member = conn.execute(
        "SELECT * FROM sync_members WHERE id = ? AND profile_id = ?",
        (book["local_member_id"], profile_id),
    ).fetchone()
    private_key = conn.execute(
        "SELECT signing_private_key_b64 FROM sync_member_private_keys WHERE member_id = ?",
        (book["local_member_id"],),
    ).fetchone()
    if not replica or not member or not private_key:
        raise AppError(
            "sync identity is incomplete",
            code="sync_identity_incomplete",
            hint="Disable and re-enable sync from the owner device, or join with a fresh invitation.",
            retryable=False,
        )
    if member["role"] == "auditor" or member["revoked_at"]:
        raise AppError(
            "this sync member cannot author edits",
            code="sync_role_forbidden",
            details={"member_id": member["id"], "role": member["role"]},
            retryable=False,
        )

    context = version_vector(conn, profile_id)
    replica_seq = int(replica["last_seq"] or 0) + 1
    hlc = tick_clock(replica["last_hlc"], replica["id"]).encode()
    timestamp = created_at or now_iso()
    event_id = str(uuid.uuid4())
    core = _event_core(
        event_id=event_id,
        workspace_id=book["workspace_id"],
        profile_id=profile_id,
        replica_id=replica["id"],
        replica_seq=replica_seq,
        hlc=hlc,
        author_member_id=member["id"],
        event_type=event_type,
        entity_table=entity_table,
        entity_key=entity_key,
        payload=payload,
        context=context,
        previous_hash=replica["last_event_hash"],
        created_at=timestamp,
    )
    digest = event_hash(core)
    signature = sign_domain_bytes(
        private_key["signing_private_key_b64"],
        EVENT_SIGNATURE_DOMAIN,
        bytes.fromhex(digest),
    )
    conn.execute(
        """
        INSERT INTO sync_events(
            id, workspace_id, profile_id, replica_id, replica_seq, hlc,
            author_member_id, event_type, entity_table, entity_key,
            payload_json, context_json, previous_hash, event_hash, signature,
            created_at, applied_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            event_id,
            book["workspace_id"],
            profile_id,
            replica["id"],
            replica_seq,
            hlc,
            member["id"],
            event_type,
            entity_table,
            entity_key,
            json.dumps(payload, ensure_ascii=True, separators=(",", ":"), sort_keys=True),
            json.dumps(context, ensure_ascii=True, separators=(",", ":"), sort_keys=True),
            replica["last_event_hash"],
            digest,
            signature,
            timestamp,
            timestamp,
        ),
    )
    conn.execute(
        """
        UPDATE sync_replicas
        SET last_seq = ?, last_hlc = ?, last_event_hash = ?, last_seen_at = ?
        WHERE id = ?
        """,
        (replica_seq, hlc, digest, timestamp, replica["id"]),
    )
    return AuthoredEvent(
        id=event_id,
        workspace_id=book["workspace_id"],
        profile_id=profile_id,
        replica_id=replica["id"],
        replica_seq=replica_seq,
        hlc=hlc,
        author_member_id=member["id"],
        event_type=event_type,
        entity_table=entity_table,
        entity_key=entity_key,
        payload=dict(payload),
        context=context,
        previous_hash=replica["last_event_hash"],
        event_hash=digest,
        signature=signature,
        created_at=timestamp,
    )


def verify_event(event: Mapping[str, Any], signing_public_key_b64: str) -> bool:
    try:
        replica_seq = event["replica_seq"]
        raw_hlc = event["hlc"]
        if type(replica_seq) is not int or replica_seq <= 0:
            return False
        if not isinstance(raw_hlc, str):
            return False
        parsed_hlc = HybridLogicalClock.parse(raw_hlc)
        if parsed_hlc.encode() != raw_hlc:
            return False
        core = _event_core(
            event_id=str(event["id"]),
            workspace_id=str(event["workspace_id"]),
            profile_id=str(event["profile_id"]),
            replica_id=str(event["replica_id"]),
            replica_seq=replica_seq,
            hlc=raw_hlc,
            author_member_id=str(event["author_member_id"]),
            event_type=str(event["event_type"]),
            entity_table=str(event["entity_table"]),
            entity_key=str(event["entity_key"]),
            payload=event["payload"],
            context=event["context"],
            previous_hash=event.get("previous_hash"),
            created_at=str(event["created_at"]),
        )
        digest = event_hash(core)
        if digest != event["event_hash"]:
            return False
        signature = str(event["signature"])
        digest_bytes = bytes.fromhex(digest)
        return verify_domain_bytes(
            signing_public_key_b64,
            EVENT_SIGNATURE_DOMAIN,
            digest_bytes,
            signature,
        ) or verify_bytes(signing_public_key_b64, digest_bytes, signature)
    except (KeyError, TypeError, ValueError, json.JSONDecodeError):
        return False
