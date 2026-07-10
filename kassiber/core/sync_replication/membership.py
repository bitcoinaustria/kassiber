"""Sealed invitations and multi-user membership operations."""

from __future__ import annotations

from io import BytesIO
import json
import sqlite3
import uuid
import zlib
from typing import Any, Mapping

from ...backup.age_cli import AgeBackend, decrypt_age_stream, encrypt_age_stream
from ...db import set_setting
from ...errors import AppError
from ...time_utils import now_iso
from .clock import tick_clock
from .crypto import (
    canonical_json_bytes,
    generate_device_keypair,
    generate_signing_keypair,
    sign_canonical,
    verify_canonical,
)
from .events import author_event
from .identity import SYNC_ROLES, connection_is_encrypted


INVITATION_SCHEMA_VERSION = 1
_INVITATION_COMPRESSED_PREFIX = b"KSINV1Z\x00"
_MAX_INVITATION_PLAINTEXT_BYTES = 2 * 1024 * 1024
_PYRAGE_BACKEND = AgeBackend(flavor="pyrage")


def _member_record_core(record: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "id": record["id"],
        "profile_id": record["profile_id"],
        "display_name": record["display_name"],
        "signing_public_key_b64": record["signing_public_key_b64"],
        "role": record["role"],
        "added_hlc": record["added_hlc"],
        "inviter_member_id": record["inviter_member_id"],
    }


def _device_record_core(record: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "id": record["id"],
        "member_id": record["member_id"],
        "recipient_public_key": record["recipient_public_key"],
        "label": record["label"],
    }


def create_join_request(
    conn: sqlite3.Connection,
    *,
    member_name: str,
    device_label: str,
) -> dict[str, Any]:
    """Create an out-of-band public join request while secrets stay in SQLCipher."""

    if not connection_is_encrypted(conn):
        raise AppError(
            "sync join keys require an encrypted SQLCipher database",
            code="sync_requires_encrypted_database",
            retryable=False,
        )
    member_name = str(member_name or "").strip()
    device_label = str(device_label or "").strip()
    if not member_name or not device_label:
        raise AppError("member name and device label are required", code="validation")
    request_id = str(uuid.uuid4())
    member_id = str(uuid.uuid4())
    device_id = str(uuid.uuid4())
    replica_id = str(uuid.uuid4())
    signing = generate_signing_keypair()
    device = generate_device_keypair()
    created_at = now_iso()
    request_core = {
        "schema_version": INVITATION_SCHEMA_VERSION,
        "request_id": request_id,
        "member_id": member_id,
        "device_id": device_id,
        "replica_id": replica_id,
        "member_name": member_name,
        "device_label": device_label,
        "signing_public_key_b64": signing.public_key_b64,
        "recipient_public_key": device.recipient,
        "created_at": created_at,
    }
    request_signature = sign_canonical(signing.private_key_b64, request_core)
    device_signature = sign_canonical(
        signing.private_key_b64,
        {
            "id": device_id,
            "member_id": member_id,
            "recipient_public_key": device.recipient,
            "label": device_label,
        },
    )
    conn.execute(
        """
        INSERT INTO sync_join_requests(
            id, member_id, device_id, replica_id, member_name, device_label,
            signing_public_key_b64, signing_private_key_b64,
            recipient_public_key, age_identity, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            request_id,
            member_id,
            device_id,
            replica_id,
            member_name,
            device_label,
            signing.public_key_b64,
            signing.private_key_b64,
            device.recipient,
            device.age_identity,
            created_at,
        ),
    )
    return request_core | {
        "request_signature": request_signature,
        "device_signature": device_signature,
    }


def _require_owner(conn: sqlite3.Connection, profile_id: str):
    row = conn.execute(
        """
        SELECT b.*, m.role, m.revoked_at, k.signing_private_key_b64
        FROM sync_books b
        JOIN sync_members m ON m.id = b.local_member_id
        JOIN sync_member_private_keys k ON k.member_id = m.id
        WHERE b.profile_id = ? AND b.enabled = 1
        """,
        (profile_id,),
    ).fetchone()
    if not row:
        raise AppError("sync is disabled or identity is incomplete", code="sync_disabled")
    if row["role"] != "owner" or row["revoked_at"]:
        raise AppError(
            "only an active owner can invite or revoke members",
            code="sync_role_forbidden",
            retryable=False,
        )
    return row


def _validate_join_request(request: Mapping[str, Any]) -> None:
    required = {
        "schema_version",
        "request_id",
        "member_id",
        "device_id",
        "replica_id",
        "member_name",
        "device_label",
        "signing_public_key_b64",
        "recipient_public_key",
        "created_at",
        "request_signature",
        "device_signature",
    }
    if set(request) != required or request.get("schema_version") != INVITATION_SCHEMA_VERSION:
        raise AppError("join request shape is invalid", code="sync_join_request_invalid")
    core = {
        key: request[key]
        for key in required
        if key not in {"request_signature", "device_signature"}
    }
    if not verify_canonical(
        str(request["signing_public_key_b64"]),
        core,
        str(request["request_signature"]),
    ):
        raise AppError("join request signature is invalid", code="sync_signature_invalid")
    if not verify_canonical(
        str(request["signing_public_key_b64"]),
        {
            "id": request["device_id"],
            "member_id": request["member_id"],
            "recipient_public_key": request["recipient_public_key"],
            "label": request["device_label"],
        },
        str(request["device_signature"]),
    ):
        raise AppError("join device signature is invalid", code="sync_signature_invalid")


def _public_catalog(conn: sqlite3.Connection, profile_id: str) -> dict[str, list[dict[str, Any]]]:
    from .bundle import _membership_catalog

    return _membership_catalog(conn, profile_id)


def create_invitation(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    join_request: Mapping[str, Any],
    role: str,
) -> bytes:
    """Accept a signed join request and return an age-sealed invitation."""

    owner = _require_owner(conn, profile_id)
    role = str(role or "").strip().lower()
    if role not in SYNC_ROLES:
        raise AppError("sync role is invalid", code="validation", details={"role": role})
    _validate_join_request(join_request)
    for table, column, value in (
        ("sync_members", "id", join_request["member_id"]),
        ("sync_devices", "id", join_request["device_id"]),
        ("sync_replicas", "id", join_request["replica_id"]),
    ):
        if conn.execute(f"SELECT 1 FROM {table} WHERE {column} = ?", (value,)).fetchone():
            raise AppError("join request identity already exists", code="conflict")

    replica = conn.execute(
        "SELECT * FROM sync_replicas WHERE id = ?",
        (owner["local_replica_id"],),
    ).fetchone()
    timestamp = now_iso()
    added_hlc = tick_clock(replica["last_hlc"], replica["id"]).encode()
    member_record = {
        "id": join_request["member_id"],
        "profile_id": profile_id,
        "display_name": join_request["member_name"],
        "signing_public_key_b64": join_request["signing_public_key_b64"],
        "role": role,
        "added_hlc": added_hlc,
        "inviter_member_id": owner["local_member_id"],
    }
    member_signature = sign_canonical(owner["signing_private_key_b64"], member_record)
    device_record = {
        "id": join_request["device_id"],
        "profile_id": profile_id,
        "member_id": join_request["member_id"],
        "recipient_public_key": join_request["recipient_public_key"],
        "label": join_request["device_label"],
        "paired_hlc": added_hlc,
    }
    # The join request proves control of the new member signing key. Bind the
    # device record explicitly with that key rather than trusting UI input.
    device_signature = str(join_request["device_signature"])
    conn.execute(
        """
        INSERT INTO sync_members(
            id, workspace_id, profile_id, display_name, signing_public_key_b64,
            role, added_hlc, added_at, inviter_member_id, record_signature
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            member_record["id"],
            owner["workspace_id"],
            profile_id,
            member_record["display_name"],
            member_record["signing_public_key_b64"],
            role,
            added_hlc,
            timestamp,
            owner["local_member_id"],
            member_signature,
        ),
    )
    conn.execute(
        """
        INSERT INTO sync_devices(
            id, workspace_id, profile_id, member_id, recipient_public_key,
            label, paired_hlc, paired_at, record_signature
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            device_record["id"],
            owner["workspace_id"],
            profile_id,
            device_record["member_id"],
            device_record["recipient_public_key"],
            device_record["label"],
            added_hlc,
            timestamp,
            device_signature,
        ),
    )
    conn.execute(
        """
        INSERT INTO sync_replicas(
            id, workspace_id, profile_id, member_id, device_id,
            last_seq, last_hlc, last_event_hash, last_seen_at, created_at
        ) VALUES(?, ?, ?, ?, ?, 0, NULL, NULL, NULL, ?)
        """,
        (
            join_request["replica_id"],
            owner["workspace_id"],
            profile_id,
            join_request["member_id"],
            join_request["device_id"],
            timestamp,
        ),
    )
    author_event(
        conn,
        profile_id=profile_id,
        event_type="membership.add",
        entity_table="sync_members",
        entity_key=str(join_request["member_id"]),
        payload=member_record | {"record_signature": member_signature},
    )
    author_event(
        conn,
        profile_id=profile_id,
        event_type="device.add",
        entity_table="sync_devices",
        entity_key=str(join_request["device_id"]),
        payload=device_record
        | {
            "request_core": {key: join_request[key] for key in join_request if key != "request_signature"},
            "record_signature": device_signature,
        },
    )

    workspace = conn.execute(
        "SELECT id, label, created_at FROM workspaces WHERE id = ?",
        (owner["workspace_id"],),
    ).fetchone()
    profile = conn.execute(
        """
        SELECT id, workspace_id, label, fiat_currency, tax_country,
               tax_long_term_days, gains_algorithm, require_coarse_review,
               bitcoin_rail_carrying_value, created_at
        FROM profiles WHERE id = ?
        """,
        (profile_id,),
    ).fetchone()
    invitation_core = {
        "schema_version": INVITATION_SCHEMA_VERSION,
        "request_id": join_request["request_id"],
        "book_id": owner["book_id"],
        "hmac_key_b64": owner["hmac_key_b64"],
        "workspace": dict(workspace),
        "profile": dict(profile),
        "local_member_id": join_request["member_id"],
        "local_device_id": join_request["device_id"],
        "local_replica_id": join_request["replica_id"],
        "membership": _public_catalog(conn, profile_id),
        "inviter_member_id": owner["local_member_id"],
        "created_at": timestamp,
    }
    invitation_signature = sign_canonical(owner["signing_private_key_b64"], invitation_core)
    output = BytesIO()
    invitation_bytes = canonical_json_bytes(
        invitation_core | {"invitation_signature": invitation_signature}
    )
    encrypt_age_stream(
        BytesIO(_INVITATION_COMPRESSED_PREFIX + zlib.compress(invitation_bytes, level=9)),
        output,
        recipients=[str(join_request["recipient_public_key"])],
        backend=_PYRAGE_BACKEND,
    )
    return output.getvalue()


def _verify_invitation_catalog(invitation: Mapping[str, Any]) -> None:
    catalog = invitation.get("membership")
    if not isinstance(catalog, dict):
        raise AppError("invitation membership catalog is missing", code="sync_invitation_invalid")
    members = {row.get("id"): row for row in catalog.get("members") or [] if isinstance(row, dict)}
    devices = [row for row in catalog.get("devices") or [] if isinstance(row, dict)]
    inviter = members.get(invitation.get("inviter_member_id"))
    if not inviter or inviter.get("role") != "owner" or inviter.get("revoked_at"):
        raise AppError("invitation owner identity is invalid", code="sync_invitation_invalid")
    core = {key: value for key, value in invitation.items() if key != "invitation_signature"}
    if not verify_canonical(
        str(inviter["signing_public_key_b64"]),
        core,
        str(invitation.get("invitation_signature") or ""),
    ):
        raise AppError("invitation signature is invalid", code="sync_signature_invalid")
    for member in members.values():
        signer = members.get(member.get("inviter_member_id"))
        if not signer or signer.get("role") != "owner":
            raise AppError("membership signature chain is incomplete", code="sync_invitation_invalid")
        if not verify_canonical(
            str(signer["signing_public_key_b64"]),
            _member_record_core(member),
            str(member.get("record_signature") or ""),
        ):
            raise AppError("membership record signature is invalid", code="sync_signature_invalid")
    for device in devices:
        member = members.get(device.get("member_id"))
        if not member:
            raise AppError("device has no member identity", code="sync_invitation_invalid")
        if not verify_canonical(
            str(member["signing_public_key_b64"]),
            _device_record_core(device),
            str(device.get("record_signature") or ""),
        ):
            raise AppError("device record signature is invalid", code="sync_signature_invalid")


def join_invitation(
    conn: sqlite3.Connection,
    *,
    request_id: str,
    ciphertext: bytes,
) -> dict[str, Any]:
    """Decrypt and install one invitation into an encrypted local database."""

    if not connection_is_encrypted(conn):
        raise AppError(
            "sync join keys require an encrypted SQLCipher database",
            code="sync_requires_encrypted_database",
        )
    pending = conn.execute(
        "SELECT * FROM sync_join_requests WHERE id = ? AND consumed_at IS NULL",
        (request_id,),
    ).fetchone()
    if not pending:
        raise AppError("join request was not found or was already used", code="not_found")
    plaintext = BytesIO()
    try:
        decrypt_age_stream(
            BytesIO(ciphertext),
            plaintext,
            identity=pending["age_identity"],
            backend=_PYRAGE_BACKEND,
        )
        raw_invitation = plaintext.getvalue()
        if raw_invitation.startswith(_INVITATION_COMPRESSED_PREFIX):
            decompressor = zlib.decompressobj()
            decoded = decompressor.decompress(
                raw_invitation[len(_INVITATION_COMPRESSED_PREFIX) :],
                _MAX_INVITATION_PLAINTEXT_BYTES + 1,
            )
            if (
                len(decoded) > _MAX_INVITATION_PLAINTEXT_BYTES
                or decompressor.unconsumed_tail
                or not decompressor.eof
            ):
                raise AppError("invitation payload is too large", code="sync_invitation_invalid")
        else:
            decoded = raw_invitation
        invitation = json.loads(decoded.decode("utf-8"))
    except AppError:
        raise
    except Exception as exc:
        raise AppError("invitation could not be decrypted", code="sync_invitation_invalid") from exc
    if invitation.get("schema_version") != INVITATION_SCHEMA_VERSION:
        raise AppError("invitation version is unsupported", code="sync_invitation_invalid")
    for invitation_key, pending_key in (
        ("request_id", "id"),
        ("local_member_id", "member_id"),
        ("local_device_id", "device_id"),
        ("local_replica_id", "replica_id"),
    ):
        if invitation.get(invitation_key) != pending[pending_key]:
            raise AppError("invitation targets a different join request", code="sync_invitation_invalid")
    _verify_invitation_catalog(invitation)
    target_members = {
        member["id"]: member for member in invitation["membership"].get("members", [])
    }
    target_devices = {
        device["id"]: device for device in invitation["membership"].get("devices", [])
    }
    target_member = target_members.get(pending["member_id"])
    target_device = target_devices.get(pending["device_id"])
    if (
        not target_member
        or not target_device
        or target_member.get("signing_public_key_b64") != pending["signing_public_key_b64"]
        or target_device.get("recipient_public_key") != pending["recipient_public_key"]
        or target_device.get("member_id") != pending["member_id"]
    ):
        raise AppError(
            "invitation does not preserve the requested member/device keys",
            code="sync_invitation_invalid",
        )
    workspace = invitation.get("workspace")
    profile = invitation.get("profile")
    if not isinstance(workspace, dict) or not isinstance(profile, dict):
        raise AppError("invitation book scope is invalid", code="sync_invitation_invalid")
    if conn.execute("SELECT 1 FROM sync_books WHERE profile_id = ?", (profile["id"],)).fetchone():
        raise AppError("this book is already joined", code="conflict")
    conn.execute(
        "INSERT OR IGNORE INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        (workspace["id"], workspace["label"], workspace["created_at"]),
    )
    conn.execute(
        """
        INSERT OR IGNORE INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country,
            tax_long_term_days, gains_algorithm, require_coarse_review,
            bitcoin_rail_carrying_value, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            profile["id"],
            profile["workspace_id"],
            profile["label"],
            profile["fiat_currency"],
            profile["tax_country"],
            profile["tax_long_term_days"],
            profile["gains_algorithm"],
            profile["require_coarse_review"],
            profile["bitcoin_rail_carrying_value"],
            profile["created_at"],
        ),
    )
    timestamp = now_iso()
    conn.execute(
        """
        INSERT INTO sync_books(
            profile_id, workspace_id, book_id, enabled, local_member_id,
            local_device_id, local_replica_id, hmac_key_b64, created_at, updated_at
        ) VALUES(?, ?, ?, 1, ?, ?, ?, ?, ?, ?)
        """,
        (
            profile["id"],
            workspace["id"],
            invitation["book_id"],
            pending["member_id"],
            pending["device_id"],
            pending["replica_id"],
            invitation["hmac_key_b64"],
            timestamp,
            timestamp,
        ),
    )
    catalog = invitation["membership"]
    for member in catalog["members"]:
        conn.execute(
            """
            INSERT INTO sync_members(
                id, workspace_id, profile_id, display_name, signing_public_key_b64,
                role, added_hlc, added_at, revoked_hlc, revoked_at,
                inviter_member_id, record_signature
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            tuple(
                member[key]
                for key in (
                    "id", "workspace_id", "profile_id", "display_name",
                    "signing_public_key_b64", "role", "added_hlc", "added_at",
                    "revoked_hlc", "revoked_at", "inviter_member_id", "record_signature",
                )
            ),
        )
    for device in catalog["devices"]:
        conn.execute(
            """
            INSERT INTO sync_devices(
                id, workspace_id, profile_id, member_id, recipient_public_key,
                label, paired_hlc, paired_at, last_seen_at, revoked_hlc,
                revoked_at, record_signature
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            tuple(
                device[key]
                for key in (
                    "id", "workspace_id", "profile_id", "member_id",
                    "recipient_public_key", "label", "paired_hlc", "paired_at",
                    "last_seen_at", "revoked_hlc", "revoked_at", "record_signature",
                )
            ),
        )
    for replica in catalog["replicas"]:
        conn.execute(
            """
            INSERT INTO sync_replicas(
                id, workspace_id, profile_id, member_id, device_id,
                last_seq, last_hlc, last_event_hash, last_seen_at, created_at
            ) VALUES(?, ?, ?, ?, ?, 0, NULL, NULL, NULL, ?)
            """,
            (
                replica["id"],
                replica["workspace_id"],
                replica["profile_id"],
                replica["member_id"],
                replica["device_id"],
                replica["created_at"],
            ),
        )
    conn.execute(
        "INSERT INTO sync_member_private_keys(member_id, signing_private_key_b64, created_at) VALUES(?, ?, ?)",
        (pending["member_id"], pending["signing_private_key_b64"], timestamp),
    )
    conn.execute(
        "INSERT INTO sync_device_private_keys(device_id, age_identity, created_at) VALUES(?, ?, ?)",
        (pending["device_id"], pending["age_identity"], timestamp),
    )
    conn.execute(
        "UPDATE sync_join_requests SET consumed_at = ? WHERE id = ?",
        (timestamp, request_id),
    )
    set_setting(conn, "context_workspace", workspace["id"])
    set_setting(conn, "context_profile", profile["id"])
    return {
        "book_id": invitation["book_id"],
        "workspace_id": workspace["id"],
        "profile_id": profile["id"],
        "member_id": pending["member_id"],
        "device_id": pending["device_id"],
        "replica_id": pending["replica_id"],
        "role": next(
            member["role"] for member in catalog["members"] if member["id"] == pending["member_id"]
        ),
    }


def list_members(conn: sqlite3.Connection, *, profile_id: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT m.id, m.display_name, m.role, m.added_at, m.revoked_at,
               COUNT(d.id) AS device_count,
               SUM(CASE WHEN d.revoked_at IS NULL THEN 1 ELSE 0 END) AS active_devices
        FROM sync_members m
        LEFT JOIN sync_devices d ON d.member_id = m.id
        WHERE m.profile_id = ?
        GROUP BY m.id
        ORDER BY m.added_hlc, m.id
        """,
        (profile_id,),
    ).fetchall()
    return [dict(row) for row in rows]


def list_devices(conn: sqlite3.Connection, *, profile_id: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT d.id, d.member_id, m.display_name AS member_name, d.label,
               d.paired_at, d.last_seen_at, d.revoked_at,
               CASE WHEN p.device_id IS NULL THEN 0 ELSE 1 END AS local_device
        FROM sync_devices d
        JOIN sync_members m ON m.id = d.member_id
        LEFT JOIN sync_books b ON b.profile_id = d.profile_id
        LEFT JOIN sync_devices p ON p.id = b.local_device_id AND p.id = d.id
        WHERE d.profile_id = ?
        ORDER BY d.paired_hlc, d.id
        """,
        (profile_id,),
    ).fetchall()
    return [dict(row) for row in rows]


def revoke_member(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    member_id: str,
) -> dict[str, Any]:
    owner = _require_owner(conn, profile_id)
    if member_id == owner["local_member_id"]:
        raise AppError(
            "the active owner cannot revoke itself",
            code="validation",
            hint="Transfer ownership or rotate the book before removing this owner.",
        )
    member = conn.execute(
        "SELECT * FROM sync_members WHERE id = ? AND profile_id = ?",
        (member_id, profile_id),
    ).fetchone()
    if not member:
        raise AppError("sync member was not found", code="not_found")
    if member["revoked_at"]:
        return {"member_id": member_id, "revoked_at": member["revoked_at"], "already_revoked": True}
    event = author_event(
        conn,
        profile_id=profile_id,
        event_type="membership.revoke",
        entity_table="sync_members",
        entity_key=member_id,
        payload={"member_id": member_id},
    )
    timestamp = event.created_at
    conn.execute(
        "UPDATE sync_members SET revoked_hlc = ?, revoked_at = ? WHERE id = ?",
        (event.hlc, timestamp, member_id),
    )
    conn.execute(
        """
        UPDATE sync_devices
        SET revoked_hlc = COALESCE(revoked_hlc, ?),
            revoked_at = COALESCE(revoked_at, ?)
        WHERE member_id = ?
        """,
        (event.hlc, timestamp, member_id),
    )
    return {"member_id": member_id, "revoked_at": timestamp, "event_id": event.id, "already_revoked": False}


def revoke_device(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    device_id: str,
) -> dict[str, Any]:
    owner = _require_owner(conn, profile_id)
    if device_id == owner["local_device_id"]:
        raise AppError("the active device cannot revoke itself", code="validation")
    device = conn.execute(
        "SELECT * FROM sync_devices WHERE id = ? AND profile_id = ?",
        (device_id, profile_id),
    ).fetchone()
    if not device:
        raise AppError("sync device was not found", code="not_found")
    if device["revoked_at"]:
        return {"device_id": device_id, "revoked_at": device["revoked_at"], "already_revoked": True}
    event = author_event(
        conn,
        profile_id=profile_id,
        event_type="device.revoke",
        entity_table="sync_devices",
        entity_key=device_id,
        payload={"device_id": device_id},
    )
    conn.execute(
        "UPDATE sync_devices SET revoked_hlc = ?, revoked_at = ? WHERE id = ?",
        (event.hlc, event.created_at, device_id),
    )
    return {
        "device_id": device_id,
        "revoked_at": event.created_at,
        "event_id": event.id,
        "already_revoked": False,
    }
