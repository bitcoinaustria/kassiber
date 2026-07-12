"""Verified replay importer, deterministic field merge, and conflict lane."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import uuid
from typing import Any, Mapping

from ...errors import AppError
from ...time_utils import now_iso
from ..repo import invalidate_journals
from .bundle import (
    BUNDLE_MANIFEST_DOMAIN,
    MAX_SYNC_SEQUENCE,
    ParsedBundle,
    parse_bundle,
)
from .clock import HybridLogicalClock, observe_clock
from .crypto import (
    canonical_json_bytes,
    decode_secret,
    hmac_identifier,
    sha256_hex,
    verify_canonical,
    verify_domain_canonical,
)
from .events import verify_event
from .gc import record_ack_vector
from .membership import (
    DEVICE_RECORD_DOMAIN,
    MEMBER_RECORD_DOMAIN,
    _device_record_core,
    _legacy_device_record_core,
    _legacy_member_record_core,
    _member_record_core,
)
from .schema_allowlist import (
    REFERENCE_TABLES,
    SYNC_TABLE_MAP,
    TableSpec,
    validate_wire_row,
)


_CUSTODY_COMPONENT_TABLES = frozenset(
    {
        "custody_components",
        "custody_component_legs",
        "custody_component_allocations",
    }
)


@dataclass(frozen=True)
class BundleImportResult:
    bundle_hash: str
    applied_events: int
    duplicate_events: int
    pending_events: int
    rejected_events: int
    row_mutations: int
    conflicts_created: int
    already_ingested: bool = False


def _json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True)


def _event_from_db(row) -> dict[str, Any]:
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


def _event_order(event: Mapping[str, Any]) -> tuple[str, str, int, str]:
    return (
        str(event["hlc"]),
        str(event["replica_id"]),
        int(event["replica_seq"]),
        str(event["id"]),
    )


def _validate_bundle_event_range(
    events: tuple[Mapping[str, Any], ...],
    *,
    sender: str,
    first_seq: int,
    last_seq: int,
) -> None:
    """Validate one contiguous replica stream without materializing its range."""

    if not events:
        return
    if any(event["replica_id"] != sender for event in events):
        raise AppError("bundle mixes replica event streams", code="sync_bundle_tampered")
    previous_seq = events[0]["replica_seq"]
    for event in events[1:]:
        current_seq = event["replica_seq"]
        if current_seq != previous_seq + 1:
            raise AppError("bundle event sequence is not contiguous", code="sync_bundle_tampered")
        previous_seq = current_seq
    if events[0]["replica_seq"] != first_seq or events[-1]["replica_seq"] != last_seq:
        raise AppError("bundle range does not match its events", code="sync_bundle_tampered")


def _normalize_snapshot_base(
    snapshot_base: Any,
    replicas: Mapping[str, Any],
) -> dict[str, Mapping[str, Any]]:
    if not isinstance(snapshot_base, Mapping) or set(snapshot_base) != set(replicas):
        raise AppError("snapshot checkpoint is incomplete", code="sync_bundle_invalid")
    normalized_base: dict[str, Mapping[str, Any]] = {}
    for replica_id, checkpoint in snapshot_base.items():
        if not isinstance(checkpoint, Mapping) or set(checkpoint) != {
            "last_seq",
            "last_hlc",
            "last_event_hash",
        }:
            raise AppError("snapshot checkpoint is invalid", code="sync_bundle_invalid")
        seq = checkpoint.get("last_seq")
        if type(seq) is not int or seq < 0 or seq > MAX_SYNC_SEQUENCE:
            raise AppError(
                "snapshot checkpoint sequence is invalid", code="sync_bundle_invalid"
            )
        last_hlc = checkpoint.get("last_hlc")
        last_event_hash = checkpoint.get("last_event_hash")
        if seq == 0 and (last_hlc is not None or last_event_hash is not None):
            raise AppError("empty snapshot checkpoint has a hash", code="sync_bundle_invalid")
        if seq > 0 and (
            not isinstance(last_hlc, str)
            or not last_hlc
            or not isinstance(last_event_hash, str)
            or len(last_event_hash) != 64
            or any(character not in "0123456789abcdef" for character in last_event_hash)
        ):
            raise AppError(
                "snapshot checkpoint is missing its hash chain tip",
                code="sync_bundle_invalid",
            )
        if last_hlc is not None:
            try:
                parsed_hlc = HybridLogicalClock.parse(last_hlc)
            except ValueError as exc:
                raise AppError(
                    "snapshot checkpoint clock is invalid",
                    code="sync_bundle_invalid",
                ) from exc
            if parsed_hlc.replica_id != str(replica_id):
                raise AppError(
                    "snapshot checkpoint clock names another replica",
                    code="sync_bundle_invalid",
                )
        normalized_base[str(replica_id)] = checkpoint
    return normalized_base


def _causal_relation(new: Mapping[str, Any], old: Mapping[str, Any]) -> str:
    if new["replica_id"] == old["replica_id"]:
        if int(new["replica_seq"]) > int(old["replica_seq"]):
            return "after"
        if int(new["replica_seq"]) < int(old["replica_seq"]):
            return "before"
        return "same"
    new_context = new.get("context") or {}
    old_context = old.get("context") or {}
    if int(new_context.get(old["replica_id"], 0)) >= int(old["replica_seq"]):
        return "after"
    if int(old_context.get(new["replica_id"], 0)) >= int(new["replica_seq"]):
        return "before"
    return "concurrent"


def _notice(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    code: str,
    severity: str,
    details: Mapping[str, Any],
    replica_id: str | None = None,
    member_id: str | None = None,
) -> None:
    stable = _json(
        {
            "profile_id": profile_id,
            "code": code,
            "replica_id": replica_id,
            "member_id": member_id,
            "details": details,
        }
    )
    notice_id = str(uuid.uuid5(uuid.NAMESPACE_URL, f"kassiber-sync-notice:{stable}"))
    conn.execute(
        """
        INSERT OR IGNORE INTO sync_notices(
            id, profile_id, code, severity, replica_id, member_id,
            details_json, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            notice_id,
            profile_id,
            code,
            severity,
            replica_id,
            member_id,
            _json(details),
            now_iso(),
        ),
    )


def _merge_membership_catalog(
    conn: sqlite3.Connection,
    *,
    book,
    catalog: Mapping[str, Any],
) -> None:
    if not isinstance(catalog, Mapping):
        raise AppError("bundle membership catalog is invalid", code="sync_bundle_invalid")
    incoming_members = [row for row in catalog.get("members") or [] if isinstance(row, Mapping)]
    incoming_devices = [row for row in catalog.get("devices") or [] if isinstance(row, Mapping)]
    incoming_replicas = [row for row in catalog.get("replicas") or [] if isinstance(row, Mapping)]
    combined_members = {
        row["id"]: dict(row)
        for row in conn.execute(
            "SELECT * FROM sync_members WHERE profile_id = ?",
            (book["profile_id"],),
        ).fetchall()
    }
    combined_members.update({str(row.get("id")): dict(row) for row in incoming_members})

    for member in incoming_members:
        required = {
            "id", "workspace_id", "profile_id", "display_name", "signing_public_key_b64",
            "role", "added_hlc", "added_at", "revoked_hlc", "revoked_at",
            "revoked_context_json",
            "inviter_member_id", "record_signature",
        }
        if set(member) != required:
            raise AppError("membership catalog row shape is invalid", code="sync_bundle_invalid")
        if member["profile_id"] != book["profile_id"] or member["workspace_id"] != book["workspace_id"]:
            raise AppError("membership row targets another book", code="sync_bundle_tampered")
        inviter_row = conn.execute(
            "SELECT * FROM sync_members WHERE id = ? AND profile_id = ?",
            (member["inviter_member_id"], book["profile_id"]),
        ).fetchone()
        inviter = dict(inviter_row) if inviter_row else None
        if not inviter or inviter.get("role") != "owner" or inviter.get("revoked_at"):
            raise AppError("membership signer is not an owner", code="sync_signature_invalid")
        member_signature = str(member["record_signature"])
        if not (
            verify_domain_canonical(
                str(inviter["signing_public_key_b64"]),
                MEMBER_RECORD_DOMAIN,
                _member_record_core(member),
                member_signature,
            )
            or verify_canonical(
                str(inviter["signing_public_key_b64"]),
                _legacy_member_record_core(member),
                member_signature,
            )
        ):
            raise AppError("membership record signature is invalid", code="sync_signature_invalid")
        existing = conn.execute("SELECT * FROM sync_members WHERE id = ?", (member["id"],)).fetchone()
        if existing:
            for immutable in ("profile_id", "workspace_id", "signing_public_key_b64", "added_hlc"):
                if existing[immutable] != member[immutable]:
                    raise AppError("membership identity changed", code="sync_replica_fork")
            continue
        conn.execute(
            """
            INSERT INTO sync_members(
                id, workspace_id, profile_id, display_name, signing_public_key_b64,
                role, added_hlc, added_at, revoked_hlc, revoked_at,
                revoked_context_json, inviter_member_id, record_signature
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            tuple(
                member[key]
                for key in (
                    "id", "workspace_id", "profile_id", "display_name",
                    "signing_public_key_b64", "role", "added_hlc", "added_at",
                    "revoked_hlc", "revoked_at", "revoked_context_json",
                    "inviter_member_id", "record_signature",
                )
            ),
        )

    for device in incoming_devices:
        required = {
            "id", "workspace_id", "profile_id", "member_id", "recipient_public_key",
            "label", "paired_hlc", "paired_at", "last_seen_at", "revoked_hlc",
            "revoked_at", "revoked_context_json", "record_signer_member_id",
            "record_signature",
        }
        if set(device) != required:
            raise AppError("device catalog row shape is invalid", code="sync_bundle_invalid")
        if device["profile_id"] != book["profile_id"] or device["workspace_id"] != book["workspace_id"]:
            raise AppError("device row targets another book", code="sync_bundle_tampered")
        member = combined_members.get(str(device["member_id"]))
        signer_id = str(device.get("record_signer_member_id") or device["member_id"])
        signer = combined_members.get(signer_id)
        signature = str(device["record_signature"])
        scoped_signature_valid = bool(
            signer
            and (signer_id == str(device["member_id"]) or signer.get("role") == "owner")
            and verify_domain_canonical(
                str(signer["signing_public_key_b64"]),
                DEVICE_RECORD_DOMAIN,
                _device_record_core(device),
                signature,
            )
        )
        legacy_signature_valid = bool(
            member
            and signer_id == str(device["member_id"])
            and verify_canonical(
                str(member["signing_public_key_b64"]),
                _legacy_device_record_core(device),
                signature,
            )
        )
        if not member or not (scoped_signature_valid or legacy_signature_valid):
            raise AppError("device record signature is invalid", code="sync_signature_invalid")
        existing = conn.execute("SELECT * FROM sync_devices WHERE id = ?", (device["id"],)).fetchone()
        if existing:
            for immutable in ("profile_id", "workspace_id", "member_id", "recipient_public_key"):
                if existing[immutable] != device[immutable]:
                    raise AppError("device identity changed", code="sync_replica_fork")
            continue
        conn.execute(
            """
            INSERT INTO sync_devices(
                id, workspace_id, profile_id, member_id, recipient_public_key,
                label, paired_hlc, paired_at, last_seen_at, revoked_hlc,
                revoked_at, revoked_context_json, record_signer_member_id,
                record_signature
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            tuple(
                device[key]
                for key in (
                    "id", "workspace_id", "profile_id", "member_id",
                    "recipient_public_key", "label", "paired_hlc", "paired_at",
                    "last_seen_at", "revoked_hlc", "revoked_at", "revoked_context_json",
                    "record_signer_member_id", "record_signature",
                )
            ),
        )

    for replica in incoming_replicas:
        required = {"id", "workspace_id", "profile_id", "member_id", "device_id", "created_at"}
        if set(replica) != required:
            raise AppError("replica catalog row shape is invalid", code="sync_bundle_invalid")
        if replica["profile_id"] != book["profile_id"] or replica["workspace_id"] != book["workspace_id"]:
            raise AppError("replica row targets another book", code="sync_bundle_tampered")
        if not conn.execute(
            "SELECT 1 FROM sync_devices WHERE id = ? AND member_id = ? AND profile_id = ?",
            (replica["device_id"], replica["member_id"], book["profile_id"]),
        ).fetchone():
            raise AppError("replica is not bound to a signed device", code="sync_signature_invalid")
        existing = conn.execute("SELECT * FROM sync_replicas WHERE id = ?", (replica["id"],)).fetchone()
        if existing:
            for immutable in ("profile_id", "workspace_id", "member_id", "device_id"):
                if existing[immutable] != replica[immutable]:
                    raise AppError("replica identity changed", code="sync_replica_fork")
            continue
        conn.execute(
            """
            INSERT INTO sync_replicas(
                id, workspace_id, profile_id, member_id, device_id,
                last_seq, last_hlc, last_event_hash, last_seen_at, created_at
            ) VALUES(?, ?, ?, ?, ?, 0, NULL, NULL, NULL, ?)
            """,
            (
                replica["id"], replica["workspace_id"], replica["profile_id"],
                replica["member_id"], replica["device_id"], replica["created_at"],
            ),
        )


_REQUIRED_EVENT_FIELDS = frozenset(
    {
        "id", "workspace_id", "profile_id", "replica_id", "replica_seq", "hlc",
        "author_member_id", "event_type", "entity_table", "entity_key", "payload",
        "context", "previous_hash", "event_hash", "signature", "created_at",
    }
)


def _validate_event_shape(event: Mapping[str, Any]) -> None:
    if set(event) != _REQUIRED_EVENT_FIELDS:
        raise AppError("sync event shape is invalid", code="sync_bundle_invalid")
    if (
        type(event["replica_seq"]) is not int
        or event["replica_seq"] <= 0
        or event["replica_seq"] > MAX_SYNC_SEQUENCE
    ):
        raise AppError("sync event sequence is invalid", code="sync_bundle_invalid")
    for field in ("replica_id", "author_member_id", "hlc"):
        if not isinstance(event[field], str) or not event[field]:
            raise AppError(f"sync event {field} is invalid", code="sync_bundle_invalid")
    try:
        parsed_hlc = HybridLogicalClock.parse(event["hlc"])
    except ValueError as exc:
        raise AppError("sync event HLC is invalid", code="sync_bundle_invalid") from exc
    if parsed_hlc.encode() != event["hlc"]:
        raise AppError("sync event HLC is invalid", code="sync_bundle_invalid")
    if not isinstance(event["context"], Mapping) or any(
        not isinstance(key, str)
        or type(value) is not int
        or value < 0
        or value > MAX_SYNC_SEQUENCE
        for key, value in event["context"].items()
    ):
        raise AppError("sync event version vector is invalid", code="sync_bundle_invalid")


def _validate_event_for_book(
    conn: sqlite3.Connection,
    *,
    book,
    event: Mapping[str, Any],
) -> None:
    _validate_event_shape(event)
    if event["workspace_id"] != book["workspace_id"] or event["profile_id"] != book["profile_id"]:
        raise AppError("sync event targets another book", code="sync_bundle_tampered")
    if int(event["context"].get(str(event["replica_id"]), -1)) != int(
        event["replica_seq"]
    ) - 1:
        raise AppError(
            "sync event version vector does not match its replica sequence",
            code="sync_bundle_invalid",
        )
    # A valid signer can reference a replica whose membership catalog has not
    # arrived on this device yet. Signature validation below still authenticates
    # the event author; the causal gate stores the event as pending until every
    # referenced replica and signed prefix becomes known.
    replica = conn.execute(
        "SELECT * FROM sync_replicas WHERE id = ? AND profile_id = ?",
        (event["replica_id"], book["profile_id"]),
    ).fetchone()
    member = conn.execute(
        "SELECT * FROM sync_members WHERE id = ? AND profile_id = ?",
        (event["author_member_id"], book["profile_id"]),
    ).fetchone()
    if not replica or not member or replica["member_id"] != member["id"]:
        raise AppError("event author is not bound to its replica", code="sync_signature_invalid")
    if not verify_event(event, member["signing_public_key_b64"]):
        raise AppError("sync event signature is invalid", code="sync_signature_invalid")


def _causal_dependencies_satisfied(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    event: Mapping[str, Any],
) -> bool:
    """Return whether every signed version-vector prerequisite was observed.

    Per-replica sequence contiguity is necessary but not sufficient: an event
    may reference a transaction or custody header authored by another replica.
    Deferring it until that replica's signed prefix has been replayed makes
    import independent of mailbox/bundle ordering without guessing from FK
    failures.
    """

    observed = {
        str(row["id"]): int(row["last_seq"] or 0)
        for row in conn.execute(
            "SELECT id, last_seq FROM sync_replicas WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
    }
    return all(
        replica_id in observed and observed[replica_id] >= int(required_seq)
        for replica_id, required_seq in event["context"].items()
    )


def _store_event(conn: sqlite3.Connection, event: Mapping[str, Any]) -> None:
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
            event["id"], event["workspace_id"], event["profile_id"], event["replica_id"],
            event["replica_seq"], event["hlc"], event["author_member_id"],
            event["event_type"], event["entity_table"], event["entity_key"],
            _json(event["payload"]), _json(event["context"]), event["previous_hash"],
            event["event_hash"], event["signature"], event["created_at"], now_iso(),
        ),
    )


def _field_state(conn, *, profile_id: str, table: str, key: str, field: str):
    state = conn.execute(
        """
        SELECT * FROM sync_field_state
        WHERE profile_id = ? AND entity_table = ? AND entity_key = ? AND field = ?
        """,
        (profile_id, table, key, field),
    ).fetchone()
    if not state:
        return None, None
    event_row = conn.execute("SELECT * FROM sync_events WHERE id = ?", (state["event_id"],)).fetchone()
    if not event_row:
        return None, None
    return state, _event_from_db(event_row)


def _write_field_state(
    conn,
    *,
    profile_id: str,
    table: str,
    key: str,
    field: str,
    event: Mapping[str, Any],
    value: Any,
) -> None:
    conn.execute(
        """
        INSERT INTO sync_field_state(
            profile_id, entity_table, entity_key, field, event_id, hlc, value_json
        ) VALUES(?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, entity_table, entity_key, field) DO UPDATE SET
            event_id = excluded.event_id,
            hlc = excluded.hlc,
            value_json = excluded.value_json
        """,
        (profile_id, table, key, field, event["id"], event["hlc"], _json(value)),
    )


def _create_conflict(
    conn,
    *,
    profile_id: str,
    workspace_id: str,
    table: str,
    key: str,
    field: str,
    old_event: Mapping[str, Any],
    new_event: Mapping[str, Any],
    old_value: Any,
    new_value: Any,
) -> bool:
    event_ids = sorted((str(old_event["id"]), str(new_event["id"])))
    if str(old_event["id"]) == event_ids[0]:
        first_value, second_value = old_value, new_value
    else:
        first_value, second_value = new_value, old_value
    conflict_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"kassiber-sync-conflict:{profile_id}:{table}:{key}:{field}:{event_ids[0]}:{event_ids[1]}",
        )
    )
    cursor = conn.execute(
        """
        INSERT OR IGNORE INTO sync_conflicts(
            id, workspace_id, profile_id, entity_table, entity_key, field,
            local_event_id, remote_event_id, local_value_json,
            remote_value_json, status, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'open', ?)
        """,
        (
            conflict_id, workspace_id, profile_id, table, key, field,
            event_ids[0], event_ids[1], _json(first_value), _json(second_value), now_iso(),
        ),
    )
    return bool(cursor.rowcount)


def _mapped_id(conn, *, profile_id: str, table: str, wire_id: Any) -> Any:
    if wire_id is None:
        return None
    row = conn.execute(
        "SELECT local_id FROM sync_id_map WHERE profile_id = ? AND entity_table = ? AND wire_id = ?",
        (profile_id, table, str(wire_id)),
    ).fetchone()
    return row["local_id"] if row else wire_id


def _record_id_map(
    conn,
    *,
    profile_id: str,
    table: str,
    wire_id: str,
    local_id: str,
) -> None:
    conn.execute(
        """
        INSERT INTO sync_id_map(profile_id, entity_table, wire_id, local_id, created_at)
        VALUES(?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, entity_table, wire_id) DO UPDATE SET local_id = excluded.local_id
        """,
        (profile_id, table, wire_id, local_id, now_iso()),
    )


def _transaction_local_id(conn, *, profile_id: str, wire_id: str, fingerprint_hmac: str, book_key: bytes) -> str:
    mapped = _mapped_id(conn, profile_id=profile_id, table="transactions", wire_id=wire_id)
    if mapped != wire_id:
        return str(mapped)
    direct = conn.execute("SELECT id FROM transactions WHERE id = ?", (wire_id,)).fetchone()
    if direct:
        return wire_id
    synthetic = conn.execute(
        "SELECT id FROM transactions WHERE profile_id = ? AND fingerprint = ?",
        (profile_id, f"sync:{fingerprint_hmac}"),
    ).fetchone()
    if synthetic:
        return str(synthetic["id"])
    for row in conn.execute(
        "SELECT id, fingerprint FROM transactions WHERE profile_id = ?",
        (profile_id,),
    ).fetchall():
        if hmac_identifier(book_key, "transaction-fingerprint", str(row["fingerprint"])) == fingerprint_hmac:
            return str(row["id"])
    return wire_id


def _prepare_actual_row(
    conn,
    *,
    book,
    spec: TableSpec,
    wire_row: Mapping[str, Any],
    blobs: Mapping[str, bytes],
    attachments_root: Path | None,
    created_files: list[Path],
) -> tuple[dict[str, Any], tuple[Any, ...]]:
    actual = {column: wire_row.get(column) for column in spec.columns}
    profile_id = book["profile_id"]
    book_key = decode_secret(book["hmac_key_b64"])
    wire_pk = tuple(actual[column] for column in spec.primary_key)
    if spec.table == "transactions":
        fingerprint_hmac = str(wire_row["fingerprint_hmac"])
        local_id = _transaction_local_id(
            conn,
            profile_id=profile_id,
            wire_id=str(actual["id"]),
            fingerprint_hmac=fingerprint_hmac,
            book_key=book_key,
        )
        _record_id_map(
            conn,
            profile_id=profile_id,
            table="transactions",
            wire_id=str(actual["id"]),
            local_id=local_id,
        )
        actual["id"] = local_id
        existing = conn.execute("SELECT fingerprint FROM transactions WHERE id = ?", (local_id,)).fetchone()
        actual["fingerprint"] = existing["fingerprint"] if existing else f"sync:{fingerprint_hmac}"
        actual["raw_json"] = "{}"
    if spec.table == "wallets":
        existing = conn.execute("SELECT config_json FROM wallets WHERE id = ?", (actual["id"],)).fetchone()
        local_config: dict[str, Any] = {}
        if existing:
            try:
                parsed = json.loads(existing["config_json"] or "{}")
                if isinstance(parsed, dict):
                    local_config = parsed
            except json.JSONDecodeError:
                # A malformed local wallet config must not abort a remote
                # merge; treat it as empty and retain only safe incoming keys.
                pass
        incoming_config = wire_row.get("config_json") if isinstance(wire_row.get("config_json"), dict) else {}
        actual["config_json"] = _json(local_config | incoming_config)
    else:
        for column in spec.json_columns:
            actual[column] = _json(actual[column]) if actual[column] is not None else None
    if spec.table == "attachments" and wire_row.get("content_hmac"):
        content_hmac = str(wire_row["content_hmac"])
        blob = blobs.get(content_hmac)
        if blob is None:
            raise AppError("attachment blob is missing", code="sync_bundle_tampered")
        raw_sha = hashlib.sha256(blob).hexdigest()
        expected_hmac = hmac_identifier(book_key, "attachment-sha256", raw_sha)
        if expected_hmac != content_hmac:
            raise AppError("attachment blob integrity check failed", code="sync_bundle_tampered")
        if attachments_root is None:
            raise AppError("attachments root is required to import file evidence", code="sync_attachment_root_required")
        root = Path(attachments_root).expanduser().resolve()
        destination = root / "sync" / content_hmac[:2] / content_hmac
        destination.parent.mkdir(parents=True, exist_ok=True)
        if not destination.exists():
            temporary = destination.with_name(f".{destination.name}.{uuid.uuid4().hex}.tmp")
            temporary.write_bytes(blob)
            os.chmod(temporary, 0o600)
            os.replace(temporary, destination)
            created_files.append(destination)
        actual["stored_relpath"] = destination.relative_to(root).as_posix()
        actual["sha256"] = raw_sha

    for column, referenced_table in REFERENCE_TABLES.items():
        if column in actual and actual[column] is not None:
            actual[column] = _mapped_id(
                conn,
                profile_id=profile_id,
                table=referenced_table,
                wire_id=actual[column],
            )
    if spec.table == "custody_component_legs":
        # Live FKs are intentionally retractable while the immutable anchor
        # survives. A peer may receive an older/live authored leg after its
        # importer has already removed that transaction or wallet. Materialize
        # the same retracted shape SQLite's ON DELETE SET NULL would have
        # produced locally instead of wedging the replica stream on the FK or
        # scope trigger forever.
        transaction_id = actual.get("transaction_id")
        if transaction_id is not None and not conn.execute(
            "SELECT 1 FROM transactions "
            "WHERE id = ? AND workspace_id = ? AND profile_id = ?",
            (transaction_id, book["workspace_id"], profile_id),
        ).fetchone():
            actual["transaction_id"] = None
        wallet_id = actual.get("wallet_id")
        if wallet_id is not None and not conn.execute(
            "SELECT 1 FROM wallets "
            "WHERE id = ? AND workspace_id = ? AND profile_id = ?",
            (wallet_id, book["workspace_id"], profile_id),
        ).fetchone():
            actual["wallet_id"] = None
    missing_optional_columns = [
        column for column in spec.optional_columns if column not in wire_row
    ]
    if missing_optional_columns:
        where = " AND ".join(f"{column} = ?" for column in spec.primary_key)
        local_pk = tuple(actual[column] for column in spec.primary_key)
        existing_optional = conn.execute(
            f"SELECT {', '.join(sorted(missing_optional_columns))} "
            f"FROM {spec.table} WHERE {where}",
            local_pk,
        ).fetchone()
        if existing_optional:
            for column in missing_optional_columns:
                actual[column] = existing_optional[column]
    if (
        spec.table == "custody_component_legs"
        and "anchor_transaction_id" not in wire_row
    ):
        # Bundles signed before the durable anchor column existed omit it. On
        # insert derive the anchor from the then-live transaction; on update
        # retain the already materialized anchor instead of interpreting the
        # absent additive field as an in-place rewrite to NULL.
        existing_anchor = conn.execute(
            "SELECT anchor_transaction_id FROM custody_component_legs WHERE id = ?",
            (actual["id"],),
        ).fetchone()
        actual["anchor_transaction_id"] = (
            existing_anchor["anchor_transaction_id"]
            if existing_anchor
            else actual.get("transaction_id")
        )
    if spec.table == "custody_components" and (
        "expected_leg_count" not in wire_row
        or "expected_allocation_count" not in wire_row
    ):
        existing_commitment = conn.execute(
            "SELECT expected_leg_count, expected_allocation_count "
            "FROM custody_components WHERE id = ?",
            (actual["id"],),
        ).fetchone()
        if existing_commitment:
            for field in ("expected_leg_count", "expected_allocation_count"):
                if field not in wire_row:
                    actual[field] = existing_commitment[field]
    if spec.table == "source_funds_link_attachments" and actual.get("link_id") is not None:
        actual["link_id"] = _mapped_id(
            conn, profile_id=profile_id, table="source_funds_links", wire_id=actual["link_id"]
        )
    if spec.table == "source_funds_source_attachments" and actual.get("source_id") is not None:
        actual["source_id"] = _mapped_id(
            conn, profile_id=profile_id, table="source_funds_sources", wire_id=actual["source_id"]
        )
    if spec.table == "commercial_links":
        existing_link = conn.execute(
            "SELECT btcpay_record_id FROM commercial_links WHERE id = ? AND profile_id = ?",
            (actual.get("id"), profile_id),
        ).fetchone()
        if existing_link and existing_link["btcpay_record_id"]:
            # Fetched BTCPay provenance is device-local. Preserve a local FK
            # when the signed wire row intentionally carries only its HMAC.
            actual["btcpay_record_id"] = existing_link["btcpay_record_id"]
        elif actual.get("btcpay_record_id") is not None:
            exists = conn.execute(
                "SELECT 1 FROM btcpay_provenance_records WHERE id = ?",
                (actual["btcpay_record_id"],),
            ).fetchone()
            if not exists:
                actual["btcpay_record_id"] = None
        if actual.get("btcpay_record_id") is None and actual.get("document_id") is None:
            btcpay_record_hmac = wire_row.get("btcpay_record_hmac")
            reviewed_snapshot = actual.get("reviewed_record_snapshot_json")
            if not btcpay_record_hmac or not reviewed_snapshot:
                raise AppError(
                    "commercial link has no portable reviewed evidence",
                    code="sync_dependency_missing",
                )
            document_id = str(
                uuid.uuid5(
                    uuid.NAMESPACE_URL,
                    f"kassiber-sync-commercial-record:{book['book_id']}:{btcpay_record_hmac}",
                )
            )
            snapshot = json.loads(reviewed_snapshot)
            if not isinstance(snapshot, dict):
                snapshot = {}
            timestamp = str(actual.get("reviewed_at") or actual.get("updated_at") or now_iso())
            conn.execute(
                """
                INSERT OR IGNORE INTO external_documents(
                    id, workspace_id, profile_id, document_type, label,
                    external_ref, issuer, counterparty, issued_at, due_at,
                    fiat_currency, fiat_value_exact, review_state, notes,
                    raw_json, created_at, updated_at
                ) VALUES(?, ?, ?, 'commercial_record_snapshot', ?, ?, NULL, NULL,
                         ?, NULL, ?, ?, 'reviewed', ?, '{}', ?, ?)
                """,
                (
                    document_id,
                    book["workspace_id"],
                    profile_id,
                    str(
                        snapshot.get("label")
                        or snapshot.get("origin_label")
                        or "Synced commercial record"
                    ),
                    f"sync:{btcpay_record_hmac}",
                    snapshot.get("occurred_at"),
                    snapshot.get("fiat_currency"),
                    snapshot.get("fiat_value_exact"),
                    "Materialized from a signed reviewed BTCPay snapshot; live BTCPay provenance remains device-local.",
                    timestamp,
                    timestamp,
                ),
            )
            actual["document_id"] = document_id
    local_pk = tuple(actual[column] for column in spec.primary_key)
    if len(spec.primary_key) == 1 and local_pk[0] is not None:
        _record_id_map(
            conn,
            profile_id=profile_id,
            table=spec.table,
            wire_id=str(wire_pk[0]),
            local_id=str(local_pk[0]),
        )
    return actual, local_pk


def _upsert_sql(spec: TableSpec, columns: list[str]) -> str:
    keys = ", ".join(spec.primary_key)
    placeholders = ", ".join("?" for _ in columns)
    update_columns = [column for column in columns if column not in spec.primary_key]
    if update_columns:
        update = ", ".join(f"{column} = excluded.{column}" for column in update_columns)
        conflict = f"DO UPDATE SET {update}"
    else:
        conflict = "DO NOTHING"
    return (
        f"INSERT INTO {spec.table}({', '.join(columns)}) VALUES({placeholders}) "
        f"ON CONFLICT({keys}) {conflict}"
    )


def _insert_or_update_with_collision_notice(
    conn,
    *,
    book,
    spec: TableSpec,
    actual: dict[str, Any],
    event: Mapping[str, Any],
) -> None:
    columns = list(actual)
    try:
        conn.execute(_upsert_sql(spec, columns), tuple(actual[column] for column in columns))
        return
    except sqlite3.IntegrityError as exc:
        collision_field = next(
            (field for field in ("label", "name", "code") if field in actual and actual[field]),
            None,
        )
        if not collision_field:
            raise AppError(
                "synced row violates a local integrity constraint",
                code="sync_row_constraint",
                details={"table": spec.table, "event_id": event["id"]},
            ) from exc
        original = str(actual[collision_field])
        actual[collision_field] = f"{original} (sync {str(event['id'])[:8]})"
        try:
            conn.execute(_upsert_sql(spec, columns), tuple(actual[column] for column in columns))
        except sqlite3.IntegrityError as retry_exc:
            raise AppError(
                "synced row could not be merged after deterministic rename",
                code="sync_row_constraint",
                details={"table": spec.table, "event_id": event["id"]},
            ) from retry_exc
        _notice(
            conn,
            profile_id=book["profile_id"],
            code="sync_name_collision",
            severity="warning",
            replica_id=str(event["replica_id"]),
            member_id=str(event["author_member_id"]),
            details={
                "table": spec.table,
                "field": collision_field,
                "original": original,
                "renamed": actual[collision_field],
            },
        )


def _apply_row_upsert(
    conn,
    *,
    book,
    event: Mapping[str, Any],
    parsed: ParsedBundle,
    attachments_root: Path | None,
    created_files: list[Path],
) -> tuple[bool, int]:
    payload = event.get("payload") or {}
    wire_row = payload.get("row") if isinstance(payload, Mapping) else None
    if not isinstance(wire_row, Mapping):
        raise AppError("row upsert event has no row", code="sync_bundle_invalid")
    spec = validate_wire_row(str(event["entity_table"]), wire_row)
    key = str(event["entity_key"])
    exists_state, exists_event = _field_state(
        conn,
        profile_id=book["profile_id"],
        table=spec.table,
        key=key,
        field="__exists__",
    )
    conflicts = 0
    if exists_state and exists_event and json.loads(exists_state["value_json"]) is False:
        relation = _causal_relation(event, exists_event)
        if relation in {"before", "concurrent"}:
            if relation == "concurrent" and spec.high_stakes_fields:
                conflicts += int(
                    _create_conflict(
                        conn,
                        profile_id=book["profile_id"],
                        workspace_id=book["workspace_id"],
                        table=spec.table,
                        key=key,
                        field="__exists__",
                        old_event=exists_event,
                        new_event=event,
                        old_value=False,
                        new_value=True,
                    )
                )
            return False, conflicts

    merged = dict(wire_row)
    winning_events: dict[str, Mapping[str, Any]] = {}
    for field, incoming_value in wire_row.items():
        state, old_event = _field_state(
            conn,
            profile_id=book["profile_id"],
            table=spec.table,
            key=key,
            field=field,
        )
        if not state or not old_event:
            winning_events[field] = event
            continue
        old_value = json.loads(state["value_json"]) if state["value_json"] is not None else None
        relation = _causal_relation(event, old_event)
        if relation in {"after", "same"}:
            winning_events[field] = event
            continue
        if relation == "before":
            merged[field] = old_value
            winning_events[field] = old_event
            continue
        if old_value != incoming_value and field in spec.high_stakes_fields:
            conflicts += int(
                _create_conflict(
                    conn,
                    profile_id=book["profile_id"],
                    workspace_id=book["workspace_id"],
                    table=spec.table,
                    key=key,
                    field=field,
                    old_event=old_event,
                    new_event=event,
                    old_value=old_value,
                    new_value=incoming_value,
                )
            )
        if _event_order(event) > _event_order(old_event):
            winning_events[field] = event
        else:
            merged[field] = old_value
            winning_events[field] = old_event

    actual, _ = _prepare_actual_row(
        conn,
        book=book,
        spec=spec,
        wire_row=merged,
        blobs=parsed.blobs,
        attachments_root=attachments_root,
        created_files=created_files,
    )
    _insert_or_update_with_collision_notice(conn, book=book, spec=spec, actual=actual, event=event)
    for field, value in merged.items():
        winner = winning_events.get(field, event)
        _write_field_state(
            conn,
            profile_id=book["profile_id"],
            table=spec.table,
            key=key,
            field=field,
            event=winner,
            value=value,
        )
    _write_field_state(
        conn,
        profile_id=book["profile_id"],
        table=spec.table,
        key=key,
        field="__exists__",
        event=event,
        value=True,
    )
    conn.execute(
        "DELETE FROM sync_tombstones WHERE profile_id = ? AND entity_table = ? AND entity_key = ?",
        (book["profile_id"], spec.table, key),
    )
    conn.execute(
        """
        INSERT INTO sync_row_state(
            profile_id, entity_table, entity_key, row_hash, last_event_id,
            last_hlc, tombstoned, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, 0, ?)
        ON CONFLICT(profile_id, entity_table, entity_key) DO UPDATE SET
            row_hash = excluded.row_hash,
            last_event_id = excluded.last_event_id,
            last_hlc = excluded.last_hlc,
            tombstoned = 0,
            updated_at = excluded.updated_at
        """,
        (
            book["profile_id"], spec.table, key,
            sha256_hex(canonical_json_bytes(merged)), event["id"], event["hlc"], now_iso(),
        ),
    )
    return True, conflicts


def _entity_pk(spec: TableSpec, event: Mapping[str, Any]) -> tuple[str, ...]:
    try:
        values = json.loads(str(event["entity_key"]))
    except json.JSONDecodeError as exc:
        raise AppError("row delete key is invalid", code="sync_bundle_invalid") from exc
    if not isinstance(values, list) or len(values) != len(spec.primary_key):
        raise AppError("row delete key does not match table primary key", code="sync_bundle_invalid")
    return tuple(str(value) for value in values)


def _has_other_active_alias(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    spec: TableSpec,
    entity_key: str,
    local_pk: tuple[Any, ...],
) -> bool:
    if len(spec.primary_key) != 1 or len(local_pk) != 1:
        return False
    rows = conn.execute(
        """
        SELECT entity_key FROM sync_row_state
        WHERE profile_id = ? AND entity_table = ?
          AND entity_key != ? AND tombstoned = 0
        """,
        (profile_id, spec.table, entity_key),
    ).fetchall()
    for row in rows:
        try:
            values = json.loads(row["entity_key"])
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        if not isinstance(values, list) or len(values) != 1:
            continue
        alias_local_id = _mapped_id(
            conn,
            profile_id=profile_id,
            table=spec.table,
            wire_id=values[0],
        )
        if str(alias_local_id) == str(local_pk[0]):
            return True
    return False


def _apply_row_delete(conn, *, book, event: Mapping[str, Any]) -> tuple[bool, int]:
    spec = SYNC_TABLE_MAP.get(str(event["entity_table"]))
    if not spec:
        raise AppError("delete targets table outside sync allowlist", code="sync_schema_forbidden")
    key = str(event["entity_key"])
    state, old_event = _field_state(
        conn,
        profile_id=book["profile_id"],
        table=spec.table,
        key=key,
        field="__exists__",
    )
    conflicts = 0
    if state and old_event:
        old_exists = json.loads(state["value_json"])
        relation = _causal_relation(event, old_event)
        if relation == "before":
            return False, 0
        if relation == "concurrent" and old_exists is True and spec.high_stakes_fields:
            conflicts += int(
                _create_conflict(
                    conn,
                    profile_id=book["profile_id"],
                    workspace_id=book["workspace_id"],
                    table=spec.table,
                    key=key,
                    field="__exists__",
                    old_event=old_event,
                    new_event=event,
                    old_value=True,
                    new_value=False,
                )
            )
    wire_pk = _entity_pk(spec, event)
    local_pk = tuple(
        _mapped_id(
            conn,
            profile_id=book["profile_id"],
            table=REFERENCE_TABLES.get(column, spec.table),
            wire_id=value,
        )
        for column, value in zip(spec.primary_key, wire_pk, strict=True)
    )
    where = " AND ".join(f"{column} = ?" for column in spec.primary_key)
    preserve_alias = _has_other_active_alias(
        conn,
        profile_id=book["profile_id"],
        spec=spec,
        entity_key=key,
        local_pk=local_pk,
    )
    if not preserve_alias:
        if spec.soft_delete_column:
            conn.execute(
                f"UPDATE {spec.table} SET {spec.soft_delete_column} = ? WHERE {where}",
                (event["created_at"], *local_pk),
            )
        else:
            conn.execute(f"DELETE FROM {spec.table} WHERE {where}", local_pk)
    _write_field_state(
        conn,
        profile_id=book["profile_id"],
        table=spec.table,
        key=key,
        field="__exists__",
        event=event,
        value=False,
    )
    conn.execute(
        """
        INSERT INTO sync_tombstones(
            profile_id, entity_table, entity_key, event_id, hlc,
            deleted_by_member_id, deleted_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, entity_table, entity_key) DO UPDATE SET
            event_id = excluded.event_id,
            hlc = excluded.hlc,
            deleted_by_member_id = excluded.deleted_by_member_id,
            deleted_at = excluded.deleted_at
        """,
        (
            book["profile_id"], spec.table, key, event["id"], event["hlc"],
            event["author_member_id"], event["created_at"],
        ),
    )
    conn.execute(
        """
        INSERT INTO sync_row_state(
            profile_id, entity_table, entity_key, row_hash, last_event_id,
            last_hlc, tombstoned, updated_at
        ) VALUES(?, ?, ?, NULL, ?, ?, 1, ?)
        ON CONFLICT(profile_id, entity_table, entity_key) DO UPDATE SET
            row_hash = NULL,
            last_event_id = excluded.last_event_id,
            last_hlc = excluded.last_hlc,
            tombstoned = 1,
            updated_at = excluded.updated_at
        """,
        (book["profile_id"], spec.table, key, event["id"], event["hlc"], now_iso()),
    )
    return True, conflicts


def _apply_transaction_edit_history(conn, *, book, event: Mapping[str, Any]) -> None:
    payload = event.get("payload") or {}
    transaction_id = _mapped_id(
        conn,
        profile_id=book["profile_id"],
        table="transactions",
        wire_id=payload.get("transaction_id"),
    )
    transaction = conn.execute(
        "SELECT * FROM transactions WHERE id = ? AND profile_id = ?",
        (transaction_id, book["profile_id"]),
    ).fetchone()
    if not transaction:
        raise AppError(
            "transaction edit arrived before its transaction anchor",
            code="sync_dependency_missing",
            details={"transaction_id": payload.get("transaction_id"), "event_id": event["id"]},
        )
    history_id = str(event["entity_key"])
    if conn.execute("SELECT 1 FROM transaction_edit_events WHERE id = ?", (history_id,)).fetchone():
        return
    profile = conn.execute("SELECT * FROM profiles WHERE id = ?", (book["profile_id"],)).fetchone()
    conn.execute(
        """
        INSERT INTO transaction_edit_events(
            id, workspace_id, profile_id, transaction_id, wallet_id,
            transaction_external_id, transaction_occurred_at, source, reason,
            changed_at, journal_input_version, journal_input_version_after,
            last_processed_input_version, last_processed_at, last_processed_tx_count,
            sync_event_id, sync_replica_id, sync_replica_seq, sync_hlc,
            sync_author_member_id, sync_signature, sync_context_json
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            history_id,
            book["workspace_id"],
            book["profile_id"],
            transaction_id,
            _mapped_id(
                conn,
                profile_id=book["profile_id"],
                table="wallets",
                wire_id=payload.get("wallet_id"),
            ),
            payload.get("transaction_external_id"),
            payload.get("transaction_occurred_at"),
            payload.get("source") or "gui",
            payload.get("reason"),
            payload.get("changed_at") or event["created_at"],
            int(profile["journal_input_version"] or 0),
            int(profile["journal_input_version"] or 0),
            int(profile["last_processed_input_version"] or 0),
            profile["last_processed_at"],
            int(profile["last_processed_tx_count"] or 0),
            event["id"],
            event["replica_id"],
            event["replica_seq"],
            event["hlc"],
            event["author_member_id"],
            event["signature"],
            _json(event["context"]),
        ),
    )
    fields = payload.get("fields") or []
    if not isinstance(fields, list):
        raise AppError("transaction edit fields are invalid", code="sync_bundle_invalid")
    conn.executemany(
        """
        INSERT INTO transaction_edit_fields(
            id, event_id, field, before_value, after_value, diff_json
        ) VALUES(?, ?, ?, ?, ?, ?)
        """,
        [
            (
                str(field["id"]),
                history_id,
                str(field["field"]),
                _json(field.get("before_value")),
                _json(field.get("after_value")),
                _json(field.get("diff") or {}),
            )
            for field in fields
            if isinstance(field, Mapping)
        ],
    )


def _revocation_rejects(row, *, replica_id: str, replica_seq: int) -> bool:
    if not row["revoked_at"]:
        return False
    raw_context = row["revoked_context_json"]
    if not raw_context:
        return True
    try:
        context = json.loads(raw_context)
    except (TypeError, ValueError, json.JSONDecodeError):
        return True
    return replica_seq > int(context.get(replica_id, 0))


def _event_role_rejection(conn, member, replica, event: Mapping[str, Any]) -> str | None:
    if member["role"] == "auditor":
        return "auditor_authored_event"
    if _revocation_rejects(
        member,
        replica_id=str(event["replica_id"]),
        replica_seq=int(event["replica_seq"]),
    ):
        return "revoked_member_event"
    device = conn.execute(
        "SELECT * FROM sync_devices WHERE id = ? AND profile_id = ?",
        (replica["device_id"], replica["profile_id"]),
    ).fetchone()
    if not device:
        return "replica_device_missing"
    if _revocation_rejects(
        device,
        replica_id=str(event["replica_id"]),
        replica_seq=int(event["replica_seq"]),
    ):
        return "revoked_device_event"
    if str(event["event_type"]).startswith(("membership.", "device.")) and member["role"] != "owner":
        return "owner_role_required"
    return None


def _immutable_revision_rejection(
    conn: sqlite3.Connection,
    *,
    book,
    event: Mapping[str, Any],
) -> dict[str, Any] | None:
    """Detect an attempted in-place rewrite of an authored custody revision.

    Component lifecycle fields may evolve, but economic/header evidence and
    every leg/allocation value are append-only. Live transaction/wallet FKs may
    become NULL when an anchored row is retracted; a transaction may reconnect
    only to the durable anchor identity if that exact row is restored.
    """

    spec = SYNC_TABLE_MAP.get(str(event["entity_table"]))
    if not spec or not spec.immutable_fields:
        return None
    if event["event_type"] == "row.delete":
        wire_pk = _entity_pk(spec, event)
        local_pk = tuple(
            _mapped_id(
                conn,
                profile_id=book["profile_id"],
                table=spec.table,
                wire_id=value,
            )
            for value in wire_pk
        )
        where = " AND ".join(f"{column} = ?" for column in spec.primary_key)
        existing = conn.execute(
            f"SELECT * FROM {spec.table} WHERE {where}",
            local_pk,
        ).fetchone()
        if not existing:
            # A parent profile/component tombstone may already have cascaded
            # this authored row. Its explicit child tombstone is then a safe,
            # idempotent audit fence rather than an erasure.
            return None
        parent_alive = (
            conn.execute(
                "SELECT 1 FROM profiles WHERE id = ?",
                (existing["profile_id"],),
            ).fetchone()
            if spec.table == "custody_components"
            else conn.execute(
                "SELECT 1 FROM custody_components WHERE id = ?",
                (existing["component_id"],),
            ).fetchone()
        )
        if parent_alive:
            return {
                "event_id": event["id"],
                "replica_seq": event["replica_seq"],
                "entity_table": spec.table,
                "entity_key": event["entity_key"],
                "operation": "delete",
                "required_action": "supersede_custody_revision",
            }
        return None
    if event["event_type"] != "row.upsert":
        return None
    payload = event.get("payload") or {}
    wire_row = payload.get("row") if isinstance(payload, Mapping) else None
    if not isinstance(wire_row, Mapping):
        return None
    spec = validate_wire_row(str(event["entity_table"]), wire_row)

    wire_pk = tuple(wire_row[column] for column in spec.primary_key)
    local_pk = tuple(
        _mapped_id(
            conn,
            profile_id=book["profile_id"],
            table=spec.table,
            wire_id=value,
        )
        for value in wire_pk
    )
    where = " AND ".join(f"{column} = ?" for column in spec.primary_key)
    existing = conn.execute(
        f"SELECT * FROM {spec.table} WHERE {where}",
        local_pk,
    ).fetchone()
    if not existing:
        if spec.table in {
            "custody_component_legs",
            "custody_component_allocations",
        }:
            component_id = _mapped_id(
                conn,
                profile_id=book["profile_id"],
                table="custody_components",
                wire_id=wire_row.get("component_id"),
            )
            count_field, child_table = (
                ("expected_leg_count", "custody_component_legs")
                if spec.table == "custody_component_legs"
                else ("expected_allocation_count", "custody_component_allocations")
            )
            component = conn.execute(
                f"SELECT {count_field} AS expected FROM custody_components WHERE id = ?",
                (component_id,),
            ).fetchone()
            if component and component["expected"] is not None:
                actual_count = int(
                    conn.execute(
                        f"SELECT COUNT(*) FROM {child_table} WHERE component_id = ?",
                        (component_id,),
                    ).fetchone()[0]
                )
                if actual_count >= int(component["expected"]):
                    return {
                        "event_id": event["id"],
                        "replica_seq": event["replica_seq"],
                        "entity_table": spec.table,
                        "entity_key": event["entity_key"],
                        "operation": "append",
                        "expected_count": int(component["expected"]),
                        "actual_count": actual_count,
                        "required_action": "create_new_custody_revision",
                    }
        return None

    changed_fields: list[str] = []
    for field in sorted(spec.immutable_fields):
        # Additive fields omitted by an older signed bundle do not rewrite the
        # locally materialized value.
        if field not in wire_row:
            continue
        incoming = wire_row[field]
        referenced_table = REFERENCE_TABLES.get(field)
        if referenced_table and incoming is not None:
            incoming = _mapped_id(
                conn,
                profile_id=book["profile_id"],
                table=referenced_table,
                wire_id=incoming,
            )
        previous = existing[field]
        if previous == incoming:
            continue
        if spec.table == "custody_component_legs" and field == "transaction_id":
            if incoming is None or (
                previous is None
                and incoming == existing["anchor_transaction_id"]
            ):
                continue
        if spec.table == "custody_component_legs" and field == "wallet_id":
            if incoming is None:
                continue
        changed_fields.append(field)

    if not changed_fields:
        return None
    return {
        "event_id": event["id"],
        "replica_seq": event["replica_seq"],
        "entity_table": spec.table,
        "entity_key": event["entity_key"],
        "immutable_fields": changed_fields,
        "required_action": "create_new_custody_revision",
    }


def _reject_contiguous_event(
    conn: sqlite3.Connection,
    *,
    book,
    replica,
    event: Mapping[str, Any],
    reason: str,
    details: Mapping[str, Any] | None = None,
) -> tuple[str, int, int, bool]:
    conn.execute(
        """
        INSERT INTO sync_rejected_events(
            profile_id, replica_id, replica_seq, event_hash, reason, received_at
        ) VALUES(?, ?, ?, ?, ?, ?)
        """,
        (
            book["profile_id"], event["replica_id"], event["replica_seq"],
            event["event_hash"], reason, now_iso(),
        ),
    )
    notice_details = {
        "event_id": event["id"],
        "replica_seq": event["replica_seq"],
    }
    if details:
        notice_details.update(details)
    _notice(
        conn,
        profile_id=book["profile_id"],
        code=reason,
        severity="blocking",
        replica_id=str(event["replica_id"]),
        member_id=str(event["author_member_id"]),
        details=notice_details,
    )
    _advance_replica(conn, replica=replica, event=event)
    return "rejected", 0, 0, False


def _advance_replica(conn, *, replica, event: Mapping[str, Any]) -> None:
    conn.execute(
        """
        UPDATE sync_replicas
        SET last_seq = ?, last_hlc = ?, last_event_hash = ?, last_seen_at = ?
        WHERE id = ?
        """,
        (
            event["replica_seq"], event["hlc"], event["event_hash"], now_iso(), replica["id"],
        ),
    )


def _apply_contiguous_event(
    conn,
    *,
    book,
    event: Mapping[str, Any],
    parsed: ParsedBundle,
    attachments_root: Path | None,
    created_files: list[Path],
) -> tuple[str, int, int, bool]:
    replica = conn.execute("SELECT * FROM sync_replicas WHERE id = ?", (event["replica_id"],)).fetchone()
    expected_seq = int(replica["last_seq"] or 0) + 1
    if int(event["replica_seq"]) != expected_seq:
        raise AppError("event is not contiguous", code="sync_event_gap")
    if event["previous_hash"] != replica["last_event_hash"]:
        raise AppError(
            "replica hash chain does not match",
            code="sync_replica_fork",
            details={"replica_id": replica["id"], "replica_seq": event["replica_seq"]},
        )
    member = conn.execute("SELECT * FROM sync_members WHERE id = ?", (event["author_member_id"],)).fetchone()
    rejection = _event_role_rejection(conn, member, replica, event)
    if rejection:
        return _reject_contiguous_event(
            conn,
            book=book,
            replica=replica,
            event=event,
            reason=rejection,
        )
    immutable_rejection = _immutable_revision_rejection(
        conn,
        book=book,
        event=event,
    )
    if immutable_rejection:
        return _reject_contiguous_event(
            conn,
            book=book,
            replica=replica,
            event=event,
            reason="custody_revision_immutable",
            details=immutable_rejection,
        )
    if event["event_type"] == "transaction.edit":
        payload = event.get("payload") or {}
        transaction_id = _mapped_id(
            conn,
            profile_id=book["profile_id"],
            table="transactions",
            wire_id=payload.get("transaction_id"),
        )
        transaction_exists = transaction_id and conn.execute(
            "SELECT 1 FROM transactions WHERE id = ? AND profile_id = ?",
            (transaction_id, book["profile_id"]),
        ).fetchone()
        if not transaction_exists:
            return _reject_contiguous_event(
                conn,
                book=book,
                replica=replica,
                event=event,
                reason="sync_dependency_missing",
                details={"transaction_id": payload.get("transaction_id")},
            )
    _store_event(conn, event)
    mutated = False
    conflicts = 0
    custody_touched = False
    if event["event_type"] == "row.upsert":
        mutated, conflicts = _apply_row_upsert(
            conn,
            book=book,
            event=event,
            parsed=parsed,
            attachments_root=attachments_root,
            created_files=created_files,
        )
        custody_touched = bool(
            mutated and event["entity_table"] in _CUSTODY_COMPONENT_TABLES
        )
    elif event["event_type"] == "row.delete":
        mutated, conflicts = _apply_row_delete(conn, book=book, event=event)
        custody_touched = bool(
            mutated and event["entity_table"] in _CUSTODY_COMPONENT_TABLES
        )
    elif event["event_type"] == "conflict.resolve":
        from .conflicts import apply_resolution_event

        mutated = apply_resolution_event(
            conn,
            book=book,
            event=event,
            parsed=parsed,
            attachments_root=attachments_root,
            created_files=created_files,
        )
        custody_touched = bool(
            mutated and event["entity_table"] in _CUSTODY_COMPONENT_TABLES
        )
    elif event["event_type"] == "membership.revoke":
        member_id = str((event.get("payload") or {}).get("member_id") or "")
        target = conn.execute(
            "SELECT 1 FROM sync_members WHERE id = ? AND profile_id = ?",
            (member_id, book["profile_id"]),
        ).fetchone()
        if not target:
            raise AppError("revoked member was not found", code="sync_event_invalid")
        revoked_context_json = _json(dict(event.get("context") or {}))
        conn.execute(
            "UPDATE sync_members SET revoked_hlc = ?, revoked_at = ?, revoked_context_json = ? WHERE id = ?",
            (event["hlc"], event["created_at"], revoked_context_json, member_id),
        )
        conn.execute(
            """
            UPDATE sync_devices SET revoked_hlc = COALESCE(revoked_hlc, ?),
                                    revoked_at = COALESCE(revoked_at, ?),
                                    revoked_context_json = COALESCE(revoked_context_json, ?)
            WHERE member_id = ?
            """,
            (event["hlc"], event["created_at"], revoked_context_json, member_id),
        )
    elif event["event_type"] == "device.revoke":
        device_id = str((event.get("payload") or {}).get("device_id") or "")
        revoked_context_json = _json(dict(event.get("context") or {}))
        cursor = conn.execute(
            """
            UPDATE sync_devices SET revoked_hlc = ?, revoked_at = ?, revoked_context_json = ?
            WHERE id = ? AND profile_id = ?
            """,
            (
                event["hlc"], event["created_at"], revoked_context_json,
                device_id, book["profile_id"],
            ),
        )
        if not cursor.rowcount:
            raise AppError("revoked device was not found", code="sync_event_invalid")
    elif event["event_type"] == "transaction.edit":
        _apply_transaction_edit_history(conn, book=book, event=event)
    elif event["event_type"] in {
        "membership.root", "membership.add", "device.add"
    }:
        pass
    else:
        raise AppError(
            "sync event type is not supported",
            code="sync_event_type_unsupported",
            details={"event_type": event["event_type"]},
        )
    _advance_replica(conn, replica=replica, event=event)
    return "applied", int(mutated), conflicts, custody_touched


def _known_event_hash(conn, *, profile_id: str, replica_id: str, replica_seq: int):
    row = conn.execute(
        "SELECT event_hash FROM sync_events WHERE profile_id = ? AND replica_id = ? AND replica_seq = ?",
        (profile_id, replica_id, replica_seq),
    ).fetchone()
    if row:
        return row["event_hash"]
    row = conn.execute(
        "SELECT event_hash FROM sync_rejected_events WHERE profile_id = ? AND replica_id = ? AND replica_seq = ?",
        (profile_id, replica_id, replica_seq),
    ).fetchone()
    return row["event_hash"] if row else None


def _queue_pending(conn, *, profile_id: str, event: Mapping[str, Any], bundle_hash: str) -> bool:
    existing = conn.execute(
        "SELECT event_json FROM sync_pending_events WHERE profile_id = ? AND replica_id = ? AND replica_seq = ?",
        (profile_id, event["replica_id"], event["replica_seq"]),
    ).fetchone()
    serialized = _json(event)
    if existing:
        if existing["event_json"] != serialized:
            raise AppError("pending replica sequence forked", code="sync_replica_fork")
        return False
    conn.execute(
        """
        INSERT INTO sync_pending_events(
            profile_id, replica_id, replica_seq, event_json, bundle_hash, received_at
        ) VALUES(?, ?, ?, ?, ?, ?)
        """,
        (
            profile_id, event["replica_id"], event["replica_seq"], serialized,
            bundle_hash, now_iso(),
        ),
    )
    return True


def _store_pending_blobs(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    parsed: ParsedBundle,
) -> None:
    for content_hmac, blob in parsed.blobs.items():
        conn.execute(
            """
            INSERT OR IGNORE INTO sync_pending_blobs(
                profile_id, bundle_hash, content_hmac, payload
            ) VALUES(?, ?, ?, ?)
            """,
            (profile_id, parsed.bundle_hash, content_hmac, blob),
        )


def _drain_pending(
    conn,
    *,
    book,
    parsed: ParsedBundle,
    attachments_root: Path | None,
    created_files: list[Path],
) -> tuple[int, int, int, int, bool]:
    stored_blobs = {
        row["content_hmac"]: bytes(row["payload"])
        for row in conn.execute(
            "SELECT content_hmac, payload FROM sync_pending_blobs WHERE profile_id = ?",
            (book["profile_id"],),
        ).fetchall()
    }
    available_bundle = ParsedBundle(
        bundle_hash=parsed.bundle_hash,
        manifest=parsed.manifest,
        events=parsed.events,
        blobs=stored_blobs | dict(parsed.blobs),
    )
    applied = rejected = mutations = conflicts = 0
    custody_touched = False
    while True:
        progressed = False
        replicas = conn.execute(
            "SELECT * FROM sync_replicas WHERE profile_id = ? ORDER BY id",
            (book["profile_id"],),
        ).fetchall()
        for replica in replicas:
            next_seq = int(replica["last_seq"] or 0) + 1
            pending = conn.execute(
                """
                SELECT * FROM sync_pending_events
                WHERE profile_id = ? AND replica_id = ? AND replica_seq = ?
                """,
                (book["profile_id"], replica["id"], next_seq),
            ).fetchone()
            if not pending:
                continue
            event = json.loads(pending["event_json"])
            if not _causal_dependencies_satisfied(
                conn,
                profile_id=book["profile_id"],
                event=event,
            ):
                # Another replica's contiguous prefix is still missing. Keep
                # scanning: a later replica in this pass may satisfy it, after
                # which the outer loop retries deterministically.
                continue
            (
                status,
                row_mutations,
                made_conflicts,
                touched_custody,
            ) = _apply_contiguous_event(
                conn,
                book=book,
                event=event,
                parsed=available_bundle,
                attachments_root=attachments_root,
                created_files=created_files,
            )
            conn.execute(
                "DELETE FROM sync_pending_events WHERE profile_id = ? AND replica_id = ? AND replica_seq = ?",
                (book["profile_id"], replica["id"], next_seq),
            )
            applied += status == "applied"
            rejected += status == "rejected"
            mutations += row_mutations
            conflicts += made_conflicts
            custody_touched = custody_touched or touched_custody
            progressed = True
        if not progressed:
            break
    conn.execute(
        """
        DELETE FROM sync_pending_blobs
        WHERE profile_id = ?
          AND bundle_hash NOT IN (
              SELECT DISTINCT bundle_hash FROM sync_pending_events WHERE profile_id = ?
          )
        """,
        (book["profile_id"], book["profile_id"]),
    )
    return applied, rejected, mutations, conflicts, custody_touched


def import_bundle(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    ciphertext: bytes,
    attachments_root: Path | None = None,
) -> BundleImportResult:
    book = conn.execute(
        "SELECT * FROM sync_books WHERE profile_id = ? AND enabled = 1",
        (profile_id,),
    ).fetchone()
    if not book:
        raise AppError("sync is disabled", code="sync_disabled")
    private_device = conn.execute(
        "SELECT age_identity FROM sync_device_private_keys WHERE device_id = ?",
        (book["local_device_id"],),
    ).fetchone()
    if not private_device:
        raise AppError("local device key is missing", code="sync_identity_incomplete")
    parsed = parse_bundle(ciphertext, age_identity=private_device["age_identity"])
    manifest = parsed.manifest
    for event in parsed.events:
        _validate_event_shape(event)
    first_seq = manifest.get("first_seq")
    last_seq = manifest.get("last_seq")
    if (
        type(first_seq) is not int
        or type(last_seq) is not int
        or first_seq < 0
        or last_seq < first_seq
        or last_seq > MAX_SYNC_SEQUENCE
        or (parsed.events and first_seq < 1)
    ):
        raise AppError("bundle manifest range is invalid", code="sync_bundle_invalid")
    if (
        manifest.get("book_id") != book["book_id"]
        or manifest.get("workspace_id") != book["workspace_id"]
        or manifest.get("profile_id") != profile_id
    ):
        raise AppError("bundle belongs to another book", code="sync_wrong_book")
    if parsed.events:
        sender = str(manifest.get("sender_replica_id") or "")
        _validate_bundle_event_range(
            parsed.events,
            sender=sender,
            first_seq=first_seq,
            last_seq=last_seq,
        )

    created_files: list[Path] = []
    applied = duplicates = pending_count = rejected = mutations = conflicts = 0
    custody_touched = False
    try:
        manifest_signature = manifest.get("manifest_signature")
        sender_member_id = str(manifest.get("sender_member_id") or "")
        sender_member = conn.execute(
            "SELECT * FROM sync_members WHERE id = ? AND profile_id = ?",
            (sender_member_id, profile_id),
        ).fetchone()
        manifest_core = {
            key: value for key, value in manifest.items() if key != "manifest_signature"
        }
        if not sender_member or not isinstance(manifest_signature, str) or not (
            verify_domain_canonical(
                sender_member["signing_public_key_b64"],
                BUNDLE_MANIFEST_DOMAIN,
                manifest_core,
                manifest_signature,
            )
            or verify_canonical(
                sender_member["signing_public_key_b64"], manifest_core, manifest_signature
            )
        ):
            raise AppError("bundle manifest signature is invalid", code="sync_bundle_tampered")
        sender_replica = conn.execute(
            "SELECT * FROM sync_replicas WHERE id = ? AND profile_id = ?",
            (manifest.get("sender_replica_id"), profile_id),
        ).fetchone()
        if not sender_replica or sender_replica["member_id"] != sender_member_id:
            raise AppError("bundle sender binding is invalid", code="sync_bundle_tampered")
        sender_device = conn.execute(
            "SELECT * FROM sync_devices WHERE id = ? AND profile_id = ?",
            (sender_replica["device_id"], profile_id),
        ).fetchone()
        if sender_member["revoked_at"] or not sender_device or sender_device["revoked_at"]:
            raise AppError("bundle sender is revoked", code="sync_role_denied")
        observed_remote_hlc: str | None = None
        if parsed.events and manifest.get("sender_replica_id") != book["local_replica_id"]:
            local = conn.execute(
                "SELECT * FROM sync_replicas WHERE id = ?",
                (book["local_replica_id"],),
            ).fetchone()
            observed_remote_hlc = local["last_hlc"]
            try:
                for event in parsed.events:
                    observed_remote_hlc = observe_clock(
                        observed_remote_hlc,
                        str(event["hlc"]),
                        local["id"],
                    ).encode()
            except ValueError as exc:
                raise AppError(
                    "bundle event clock exceeds the allowed future-drift window",
                    code="sync_clock_invalid",
                ) from exc
        _merge_membership_catalog(
            conn,
            book=book,
            catalog=manifest.get("membership") or {},
        )
        bundle_kind = manifest.get("bundle_kind")
        if bundle_kind not in {"incremental", "snapshot"}:
            raise AppError("bundle kind is invalid", code="sync_bundle_invalid")
        if bundle_kind == "snapshot":
            if sender_member["role"] != "owner" or sender_member["revoked_at"]:
                raise AppError(
                    "snapshot bundle must be attested by an active owner",
                    code="sync_role_denied",
                )
            snapshot_base = manifest.get("snapshot_base")
            replicas = {
                row["id"]: row
                for row in conn.execute(
                    "SELECT * FROM sync_replicas WHERE profile_id = ?",
                    (profile_id,),
                ).fetchall()
            }
            normalized_base = _normalize_snapshot_base(snapshot_base, replicas)
            pristine = (
                conn.execute(
                    "SELECT COUNT(*) FROM sync_events WHERE profile_id = ?", (profile_id,)
                ).fetchone()[0]
                == 0
                and conn.execute(
                    "SELECT COUNT(*) FROM sync_row_state WHERE profile_id = ?", (profile_id,)
                ).fetchone()[0]
                == 0
                and all(int(row["last_seq"] or 0) == 0 for row in replicas.values())
            )
            if pristine:
                for replica_id, checkpoint in normalized_base.items():
                    conn.execute(
                        """
                        UPDATE sync_replicas
                        SET last_seq = ?, last_hlc = ?, last_event_hash = ?, last_seen_at = ?
                        WHERE id = ? AND profile_id = ?
                        """,
                        (
                            checkpoint["last_seq"],
                            checkpoint["last_hlc"],
                            checkpoint["last_event_hash"],
                            now_iso(),
                            replica_id,
                            profile_id,
                        ),
                    )
                _notice(
                    conn,
                    profile_id=profile_id,
                    code="sync_snapshot_bootstrap",
                    severity="info",
                    replica_id=str(manifest["sender_replica_id"]),
                    member_id=sender_member_id,
                    details={
                        "checkpoint": {
                            key: int(value["last_seq"])
                            for key, value in normalized_base.items()
                        }
                    },
                )
        elif manifest.get("snapshot_base") is not None:
            raise AppError(
                "incremental bundle contains a snapshot checkpoint", code="sync_bundle_invalid"
            )
        already = conn.execute(
            "SELECT 1 FROM sync_ingests WHERE profile_id = ? AND bundle_hash = ?",
            (profile_id, parsed.bundle_hash),
        ).fetchone()
        for event in parsed.events:
            _validate_event_for_book(conn, book=book, event=event)
            known_hash = _known_event_hash(
                conn,
                profile_id=profile_id,
                replica_id=str(event["replica_id"]),
                replica_seq=int(event["replica_seq"]),
            )
            if known_hash:
                if known_hash != event["event_hash"]:
                    raise AppError("replica sequence forked", code="sync_replica_fork")
                duplicates += 1
                continue
            replica = conn.execute(
                "SELECT * FROM sync_replicas WHERE id = ?",
                (event["replica_id"],),
            ).fetchone()
            expected = int(replica["last_seq"] or 0) + 1
            if int(event["replica_seq"]) > expected or (
                int(event["replica_seq"]) == expected
                and not _causal_dependencies_satisfied(
                    conn,
                    profile_id=profile_id,
                    event=event,
                )
            ):
                _store_pending_blobs(
                    conn,
                    profile_id=profile_id,
                    parsed=parsed,
                )
                pending_count += int(
                    _queue_pending(
                        conn,
                        profile_id=profile_id,
                        event=event,
                        bundle_hash=parsed.bundle_hash,
                    )
                )
                continue
            if int(event["replica_seq"]) < expected:
                raise AppError("replica history is missing a known sequence", code="sync_replica_fork")
            (
                status,
                row_mutations,
                made_conflicts,
                touched_custody,
            ) = _apply_contiguous_event(
                conn,
                book=book,
                event=event,
                parsed=parsed,
                attachments_root=attachments_root,
                created_files=created_files,
            )
            applied += status == "applied"
            rejected += status == "rejected"
            mutations += row_mutations
            conflicts += made_conflicts
            custody_touched = custody_touched or touched_custody

        drained = _drain_pending(
            conn,
            book=book,
            parsed=parsed,
            attachments_root=attachments_root,
            created_files=created_files,
        )
        applied += drained[0]
        rejected += drained[1]
        mutations += drained[2]
        conflicts += drained[3]
        custody_touched = custody_touched or drained[4]
        if not already:
            conn.execute(
                """
                INSERT INTO sync_ingests(
                    id, profile_id, replica_id, first_seq, last_seq,
                    bundle_hash, prior_bundle_hash, ingested_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    str(uuid.uuid4()), profile_id, manifest["sender_replica_id"],
                    manifest["first_seq"], manifest["last_seq"], parsed.bundle_hash,
                    manifest.get("prior_bundle_hash"), now_iso(),
                ),
            )
        if custody_touched:
            # Memberships are a local derived uniqueness guard and are never
            # replicated.  Rebuild only after the whole replay batch (including
            # drained events) so reordered headers/legs and concurrent active
            # revisions are evaluated atomically.  Invalid authored rows stay
            # visible but receive no effective membership.
            from ..custody_components import reconcile_active_memberships

            reconcile_active_memberships(conn, profile_id=profile_id)
        if mutations or conflicts:
            invalidate_journals(conn, profile_id)
        remote_hlcs = [str(event["hlc"]) for event in parsed.events]
        if observed_remote_hlc is not None:
            conn.execute(
                "UPDATE sync_replicas SET last_hlc = ? WHERE id = ?",
                (observed_remote_hlc, book["local_replica_id"]),
            )
        manifest_vector = manifest.get("version_vector")
        if not isinstance(manifest_vector, Mapping):
            raise AppError("bundle version vector is invalid", code="sync_bundle_invalid")
        record_ack_vector(
            conn,
            profile_id=profile_id,
            observer_replica_id=str(manifest["sender_replica_id"]),
            vector=manifest_vector,
            observed_hlc=remote_hlcs[-1] if remote_hlcs else None,
        )
        return BundleImportResult(
            bundle_hash=parsed.bundle_hash,
            applied_events=applied,
            duplicate_events=duplicates,
            pending_events=pending_count,
            rejected_events=rejected,
            row_mutations=mutations,
            conflicts_created=conflicts,
            already_ingested=bool(already),
        )
    except Exception:
        for path in created_files:
            path.unlink(missing_ok=True)
        raise
