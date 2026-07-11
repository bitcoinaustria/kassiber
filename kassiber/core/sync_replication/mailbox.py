"""Signed append-only mailbox protocol over injected dumb object storage."""

from __future__ import annotations

from dataclasses import asdict, dataclass
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import re
import sqlite3
from typing import Any, Mapping
import uuid

from ...errors import AppError
from ...time_utils import now_iso
from .bundle import build_bundle
from .crypto import (
    canonical_json_bytes,
    decode_secret,
    hmac_identifier,
    sha256_hex,
    sign_domain_canonical,
    verify_canonical,
    verify_domain_canonical,
)
from .merge import import_bundle
from .events import version_vector
from .gc import record_ack_vector
from .transports import ObjectTransport, load_transport


MAILBOX_SCHEMA_VERSION = 1
DEFAULT_STALE_AFTER_SECONDS = 7 * 24 * 60 * 60
MAILBOX_HEAD_DOMAIN = "mailbox-head-v1"
MAILBOX_ACK_DOMAIN = "mailbox-ack-v1"
MAX_HEAD_FUTURE_DRIFT_SECONDS = 5 * 60


@dataclass(frozen=True)
class MailboxPushResult:
    transport_id: str
    transport_label: str
    up_to_date: bool
    bundle_hash: str | None
    event_count: int
    first_seq: int | None
    last_seq: int | None
    object_key: str | None
    head_key: str | None


@dataclass(frozen=True)
class MailboxPullResult:
    transport_id: str
    transport_label: str
    heads_seen: int
    bundles_seen: int
    bundles_imported: int
    applied_events: int
    duplicate_events: int
    pending_events: int
    rejected_events: int
    conflicts_created: int
    peers: tuple[Mapping[str, Any], ...]


def _book(conn: sqlite3.Connection, profile_id: str, *, require_enabled: bool = True):
    row = conn.execute(
        "SELECT * FROM sync_books WHERE profile_id = ?",
        (profile_id,),
    ).fetchone()
    if not row or (require_enabled and not row["enabled"]):
        raise AppError("sync is disabled", code="sync_disabled")
    return row


def _scope_hmac(book, namespace: str, value: str) -> str:
    return hmac_identifier(decode_secret(book["hmac_key_b64"]), namespace, value)


def mailbox_book_prefix(book) -> str:
    return f"kassiber-sync/v1/books/{_scope_hmac(book, 'mailbox-book', book['book_id'])}"


def mailbox_replica_prefix(book, replica_id: str) -> str:
    return f"{mailbox_book_prefix(book)}/replicas/{_scope_hmac(book, 'mailbox-replica', replica_id)}"


def mailbox_head_key(book, replica_id: str) -> str:
    return f"{mailbox_replica_prefix(book, replica_id)}/head.json"


def mailbox_ack_key(book, replica_id: str) -> str:
    return f"{mailbox_replica_prefix(book, replica_id)}/ack.json"


def mailbox_bundle_key(
    book,
    replica_id: str,
    first_seq: int,
    last_seq: int,
    bundle_hash: str,
    *,
    snapshot: bool = False,
) -> str:
    kind_prefix = "snapshot-" if snapshot else ""
    return (
        f"{mailbox_replica_prefix(book, replica_id)}/bundles/"
        f"{kind_prefix}{first_seq:020d}-{last_seq:020d}-{bundle_hash}.age"
    )


_BUNDLE_NAME_RE = re.compile(
    r"^(?P<snapshot>snapshot-)?(?P<first>[0-9]{20})-(?P<last>[0-9]{20})-(?P<hash>[0-9a-f]{64})\.age$"
)


def _bundle_key_parts(key: str) -> tuple[bool, int, int, str]:
    match = _BUNDLE_NAME_RE.fullmatch(key.rsplit("/", 1)[-1])
    if not match:
        raise AppError("mailbox bundle name is invalid", code="sync_mailbox_bundle_tampered")
    first_seq = int(match.group("first"))
    last_seq = int(match.group("last"))
    if first_seq < 1 or last_seq < first_seq:
        raise AppError("mailbox bundle range is invalid", code="sync_mailbox_bundle_tampered")
    return bool(match.group("snapshot")), first_seq, last_seq, match.group("hash")


def _head_core(
    conn: sqlite3.Connection,
    *,
    book,
    bundle_hash: str,
    bundle_key: str,
    first_seq: int,
    last_seq: int,
    prior_bundle_hash: str | None,
    previous_head_hash: str | None,
    created_at: str,
    bundle_kind: str,
) -> dict[str, Any]:
    return {
        "schema_version": MAILBOX_SCHEMA_VERSION,
        "book_hmac": _scope_hmac(book, "mailbox-book", book["book_id"]),
        "replica_hmac": _scope_hmac(book, "mailbox-replica", book["local_replica_id"]),
        "member_hmac": _scope_hmac(book, "mailbox-member", book["local_member_id"]),
        "device_hmac": _scope_hmac(book, "mailbox-device", book["local_device_id"]),
        "first_seq": first_seq,
        "last_seq": last_seq,
        "bundle_hash": bundle_hash,
        "bundle_key": bundle_key,
        "bundle_kind": bundle_kind,
        "prior_bundle_hash": prior_bundle_hash,
        "previous_head_hash": previous_head_hash,
        "created_at": created_at,
    }


def _sign_head(conn: sqlite3.Connection, *, book, core: Mapping[str, Any]) -> bytes:
    key = conn.execute(
        "SELECT signing_private_key_b64 FROM sync_member_private_keys WHERE member_id = ?",
        (book["local_member_id"],),
    ).fetchone()
    if not key:
        raise AppError("local signing key is missing", code="sync_identity_incomplete")
    document = dict(core)
    document["head_hash"] = sha256_hex(canonical_json_bytes(core))
    document["signature"] = sign_domain_canonical(
        key["signing_private_key_b64"], MAILBOX_HEAD_DOMAIN, document
    )
    return canonical_json_bytes(document)


def _sign_ack(conn: sqlite3.Connection, *, book) -> bytes:
    key = conn.execute(
        "SELECT signing_private_key_b64 FROM sync_member_private_keys WHERE member_id = ?",
        (book["local_member_id"],),
    ).fetchone()
    if not key:
        raise AppError("local signing key is missing", code="sync_identity_incomplete")
    vector = {
        _scope_hmac(book, "mailbox-replica", replica_id): seq
        for replica_id, seq in version_vector(conn, book["profile_id"]).items()
    }
    core = {
        "schema_version": MAILBOX_SCHEMA_VERSION,
        "book_hmac": _scope_hmac(book, "mailbox-book", book["book_id"]),
        "replica_hmac": _scope_hmac(book, "mailbox-replica", book["local_replica_id"]),
        "member_hmac": _scope_hmac(book, "mailbox-member", book["local_member_id"]),
        "device_hmac": _scope_hmac(book, "mailbox-device", book["local_device_id"]),
        "ack_vector": vector,
        "created_at": now_iso(),
    }
    document = dict(core)
    document["ack_hash"] = sha256_hex(canonical_json_bytes(core))
    document["signature"] = sign_domain_canonical(
        key["signing_private_key_b64"], MAILBOX_ACK_DOMAIN, document
    )
    return canonical_json_bytes(document)


def _replica_catalog(conn: sqlite3.Connection, *, book) -> dict[str, tuple[Any, Any, Any]]:
    output: dict[str, tuple[Any, Any, Any]] = {}
    rows = conn.execute(
        """
        SELECT r.*, m.signing_public_key_b64, m.revoked_at AS member_revoked_at,
               d.revoked_at AS device_revoked_at
        FROM sync_replicas AS r
        JOIN sync_members AS m ON m.id = r.member_id
        JOIN sync_devices AS d ON d.id = r.device_id
        WHERE r.profile_id = ?
        """,
        (book["profile_id"],),
    ).fetchall()
    for row in rows:
        output[_scope_hmac(book, "mailbox-replica", row["id"])] = (row, row, row)
    return output


def _parse_head(conn: sqlite3.Connection, *, book, payload: bytes) -> tuple[dict[str, Any], Any]:
    try:
        document = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AppError("mailbox head is invalid", code="sync_mailbox_head_invalid") from exc
    if not isinstance(document, dict) or document.get("schema_version") != MAILBOX_SCHEMA_VERSION:
        raise AppError("mailbox head version is invalid", code="sync_mailbox_head_invalid")
    signature = document.pop("signature", None)
    head_hash = document.get("head_hash")
    core = {key: value for key, value in document.items() if key != "head_hash"}
    if head_hash != sha256_hex(canonical_json_bytes(core)):
        raise AppError("mailbox head hash is invalid", code="sync_mailbox_head_tampered")
    expected_book = _scope_hmac(book, "mailbox-book", book["book_id"])
    if document.get("book_hmac") != expected_book:
        raise AppError("mailbox head belongs to another book", code="sync_wrong_book")
    catalog = _replica_catalog(conn, book=book)
    replica_tuple = catalog.get(str(document.get("replica_hmac") or ""))
    if not replica_tuple:
        raise AppError("mailbox head names an unknown replica", code="sync_mailbox_head_unknown")
    replica = replica_tuple[0]
    if document.get("member_hmac") != _scope_hmac(book, "mailbox-member", replica["member_id"]):
        raise AppError("mailbox head member binding is invalid", code="sync_mailbox_head_tampered")
    if document.get("device_hmac") != _scope_hmac(book, "mailbox-device", replica["device_id"]):
        raise AppError("mailbox head device binding is invalid", code="sync_mailbox_head_tampered")
    if not isinstance(signature, str) or not (
        verify_domain_canonical(
            replica["signing_public_key_b64"], MAILBOX_HEAD_DOMAIN, document, signature
        )
        or verify_canonical(replica["signing_public_key_b64"], document, signature)
    ):
        raise AppError("mailbox head signature is invalid", code="sync_mailbox_head_tampered")
    created_at = _parse_timestamp(document.get("created_at"))
    if created_at is None or created_at > datetime.now(timezone.utc) + timedelta(
        seconds=MAX_HEAD_FUTURE_DRIFT_SECONDS
    ):
        raise AppError("mailbox head timestamp is invalid", code="sync_mailbox_head_invalid")
    first_seq = document.get("first_seq")
    last_seq = document.get("last_seq")
    if type(first_seq) is not int or type(last_seq) is not int or first_seq < 1 or last_seq < first_seq:
        raise AppError("mailbox head range is invalid", code="sync_mailbox_head_invalid")
    expected_prefix = mailbox_replica_prefix(book, replica["id"]) + "/bundles/"
    bundle_key = str(document.get("bundle_key") or "")
    if not bundle_key.startswith(expected_prefix) or not bundle_key.endswith(f"-{document.get('bundle_hash')}.age"):
        raise AppError("mailbox head object binding is invalid", code="sync_mailbox_head_tampered")
    is_snapshot, first_seq, last_seq, bundle_hash = _bundle_key_parts(bundle_key)
    if (
        first_seq != document["first_seq"]
        or last_seq != document["last_seq"]
        or bundle_hash != document["bundle_hash"]
        or document.get("bundle_kind") != ("snapshot" if is_snapshot else "incremental")
    ):
        raise AppError("mailbox head range binding is invalid", code="sync_mailbox_head_tampered")
    document["signature"] = signature
    return document, replica


def _parse_ack(conn: sqlite3.Connection, *, book, payload: bytes) -> tuple[Any, dict[str, int]]:
    try:
        document = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise AppError("mailbox acknowledgement is invalid", code="sync_ack_invalid") from exc
    if not isinstance(document, dict) or document.get("schema_version") != MAILBOX_SCHEMA_VERSION:
        raise AppError("mailbox acknowledgement is invalid", code="sync_ack_invalid")
    signature = document.pop("signature", None)
    ack_hash = document.get("ack_hash")
    core = {key: value for key, value in document.items() if key != "ack_hash"}
    if ack_hash != sha256_hex(canonical_json_bytes(core)):
        raise AppError("mailbox acknowledgement hash is invalid", code="sync_ack_invalid")
    if document.get("book_hmac") != _scope_hmac(book, "mailbox-book", book["book_id"]):
        raise AppError("mailbox acknowledgement belongs to another book", code="sync_wrong_book")
    catalog = _replica_catalog(conn, book=book)
    replica_tuple = catalog.get(str(document.get("replica_hmac") or ""))
    if not replica_tuple:
        raise AppError("mailbox acknowledgement replica is unknown", code="sync_ack_invalid")
    replica = replica_tuple[0]
    if (
        document.get("member_hmac") != _scope_hmac(book, "mailbox-member", replica["member_id"])
        or document.get("device_hmac") != _scope_hmac(book, "mailbox-device", replica["device_id"])
        or not isinstance(signature, str)
        or not (
            verify_domain_canonical(
                replica["signing_public_key_b64"], MAILBOX_ACK_DOMAIN, document, signature
            )
            or verify_canonical(replica["signing_public_key_b64"], document, signature)
        )
    ):
        raise AppError("mailbox acknowledgement signature is invalid", code="sync_ack_invalid")
    raw_vector = document.get("ack_vector")
    if not isinstance(raw_vector, dict):
        raise AppError("mailbox acknowledgement vector is invalid", code="sync_ack_invalid")
    by_hmac = {opaque: row_tuple[0]["id"] for opaque, row_tuple in catalog.items()}
    vector: dict[str, int] = {}
    for opaque, seq in raw_vector.items():
        replica_id = by_hmac.get(str(opaque))
        if not replica_id or type(seq) is not int or seq < 0:
            raise AppError("mailbox acknowledgement vector is invalid", code="sync_ack_invalid")
        vector[replica_id] = seq
    return replica, vector


def _notice(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    code: str,
    severity: str,
    replica_id: str | None,
    member_id: str | None,
    details: Mapping[str, Any],
) -> None:
    fingerprint = sha256_hex(canonical_json_bytes({"code": code, "replica_id": replica_id, "details": details}))
    if conn.execute(
        "SELECT 1 FROM sync_notices WHERE profile_id = ? AND code = ? AND details_json LIKE ?",
        (profile_id, code, f'%"fingerprint": "{fingerprint}"%'),
    ).fetchone():
        return
    conn.execute(
        """
        INSERT INTO sync_notices(
            id, profile_id, code, severity, replica_id, member_id, details_json, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            str(uuid.uuid4()), profile_id, code, severity, replica_id, member_id,
            json.dumps(dict(details) | {"fingerprint": fingerprint}, sort_keys=True), now_iso(),
        ),
    )


def _mark_transport_error(conn, *, transport_id: str, error: Exception) -> None:
    conn.execute(
        """
        UPDATE sync_transports
        SET last_error_at = ?, last_error_code = ?, updated_at = ?
        WHERE id = ?
        """,
        (now_iso(), getattr(error, "code", "sync_transport_error"), now_iso(), transport_id),
    )


def push_mailbox(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    transport_id: str | None = None,
    transport_label: str | None = None,
    attachments_root: Path | None = None,
    transport_override: ObjectTransport | None = None,
    snapshot: bool = False,
) -> MailboxPushResult:
    book = _book(conn, profile_id)
    row, transport = load_transport(
        conn, profile_id=profile_id, transport_id=transport_id, label=transport_label
    )
    if transport_override is not None:
        transport = transport_override
    conn.execute("SAVEPOINT sync_mailbox_push")
    try:
        result = build_bundle(
            conn,
            profile_id=profile_id,
            attachments_root=attachments_root,
            snapshot=snapshot,
        )
        if result is None:
            transport.put(
                mailbox_ack_key(book, book["local_replica_id"]),
                _sign_ack(conn, book=book),
            )
            conn.execute(
                "UPDATE sync_transports SET last_push_at = ?, last_error_code = NULL, updated_at = ? WHERE id = ?",
                (now_iso(), now_iso(), row["id"]),
            )
            conn.execute("RELEASE SAVEPOINT sync_mailbox_push")
            return MailboxPushResult(row["id"], row["label"], True, None, 0, None, None, None, None)
        key = mailbox_bundle_key(
            book,
            book["local_replica_id"],
            result.first_seq,
            result.last_seq,
            result.bundle_hash,
            snapshot=snapshot,
        )
        known_head = conn.execute(
            "SELECT * FROM sync_mailbox_heads WHERE profile_id = ? AND transport_id = ? AND replica_id = ?",
            (profile_id, row["id"], book["local_replica_id"]),
        ).fetchone()
        core = _head_core(
            conn,
            book=book,
            bundle_hash=result.bundle_hash,
            bundle_key=key,
            first_seq=result.first_seq,
            last_seq=result.last_seq,
            prior_bundle_hash=result.manifest.get("prior_bundle_hash"),
            previous_head_hash=known_head["head_hash"] if known_head else None,
            created_at=str(result.manifest["created_at"]),
            bundle_kind="snapshot" if snapshot else "incremental",
        )
        head_payload = _sign_head(conn, book=book, core=core)
        head_document = json.loads(head_payload)
        transport.put(key, result.ciphertext, if_absent=True)
        head_key = mailbox_head_key(book, book["local_replica_id"])
        transport.put(head_key, head_payload)
        transport.put(
            mailbox_ack_key(book, book["local_replica_id"]),
            _sign_ack(conn, book=book),
        )
        timestamp = now_iso()
        conn.execute(
            """
            INSERT INTO sync_mailbox_heads(
                profile_id, transport_id, replica_id, last_seq, bundle_hash, head_hash, observed_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(profile_id, transport_id, replica_id) DO UPDATE SET
                last_seq = excluded.last_seq,
                bundle_hash = excluded.bundle_hash,
                head_hash = excluded.head_hash,
                observed_at = excluded.observed_at
            """,
            (profile_id, row["id"], book["local_replica_id"], result.last_seq, result.bundle_hash, head_document["head_hash"], timestamp),
        )
        conn.execute(
            """
            UPDATE sync_transports
            SET last_push_at = ?, last_error_at = NULL, last_error_code = NULL, updated_at = ?
            WHERE id = ?
            """,
            (timestamp, timestamp, row["id"]),
        )
        conn.execute("RELEASE SAVEPOINT sync_mailbox_push")
        return MailboxPushResult(
            row["id"], row["label"], False, result.bundle_hash, result.event_count,
            result.first_seq, result.last_seq, key, head_key,
        )
    except Exception as exc:
        conn.execute("ROLLBACK TO SAVEPOINT sync_mailbox_push")
        conn.execute("RELEASE SAVEPOINT sync_mailbox_push")
        _mark_transport_error(conn, transport_id=row["id"], error=exc)
        raise


def _parse_timestamp(value: str | None) -> datetime | None:
    if not isinstance(value, str) or not value:
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)
    except ValueError:
        return None


def peer_staleness(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    transport_id: str,
    stale_after_seconds: int = DEFAULT_STALE_AFTER_SECONDS,
) -> list[dict[str, Any]]:
    book = _book(conn, profile_id, require_enabled=False)
    now = datetime.now(timezone.utc)
    rows = conn.execute(
        """
        SELECT p.*, m.display_name, d.label AS device_label
        FROM sync_peer_status AS p
        JOIN sync_members AS m ON m.id = p.member_id
        JOIN sync_devices AS d ON d.id = p.device_id
        WHERE p.profile_id = ? AND p.transport_id = ? AND p.replica_id != ?
        ORDER BY m.display_name, d.label, p.replica_id
        """,
        (profile_id, transport_id, book["local_replica_id"]),
    ).fetchall()
    output: list[dict[str, Any]] = []
    for row in rows:
        seen = _parse_timestamp(row["last_bundle_at"])
        age = max(0, int((now - seen).total_seconds())) if seen else None
        status = "never_seen" if seen is None else ("stale" if age > stale_after_seconds else "fresh")
        output.append(
            {
                "replica_id": row["replica_id"],
                "member_id": row["member_id"],
                "member_name": row["display_name"],
                "device_id": row["device_id"],
                "device_label": row["device_label"],
                "last_head_seq": int(row["last_head_seq"]),
                "last_seen_at": row["last_seen_at"],
                "last_bundle_at": row["last_bundle_at"],
                "age_seconds": age,
                "status": status,
            }
        )
        conn.execute(
            "UPDATE sync_peer_status SET status = ?, updated_at = ? WHERE profile_id = ? AND transport_id = ? AND replica_id = ?",
            (status, now_iso(), profile_id, transport_id, row["replica_id"]),
        )
    return output


def _seed_peer_rows(conn: sqlite3.Connection, *, profile_id: str, transport_id: str) -> None:
    timestamp = now_iso()
    conn.execute(
        """
        INSERT OR IGNORE INTO sync_peer_status(
            profile_id, transport_id, replica_id, member_id, device_id,
            status, updated_at
        )
        SELECT ?, ?, id, member_id, device_id, 'never_seen', ?
        FROM sync_replicas WHERE profile_id = ?
        """,
        (profile_id, transport_id, timestamp, profile_id),
    )


def pull_mailbox(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    transport_id: str | None = None,
    transport_label: str | None = None,
    attachments_root: Path | None = None,
    transport_override: ObjectTransport | None = None,
    stale_after_seconds: int = DEFAULT_STALE_AFTER_SECONDS,
) -> MailboxPullResult:
    book = _book(conn, profile_id)
    row, transport = load_transport(
        conn, profile_id=profile_id, transport_id=transport_id, label=transport_label
    )
    if transport_override is not None:
        transport = transport_override
    _seed_peer_rows(conn, profile_id=profile_id, transport_id=row["id"])
    heads_seen = bundles_seen = bundles_imported = 0
    applied = duplicates = pending = rejected = conflicts = 0
    conn.execute("SAVEPOINT sync_mailbox_pull")
    try:
        parsed_acknowledgements: list[tuple[Any, dict[str, int]]] = []
        ack_keys = [
            key for key in transport.list(mailbox_book_prefix(book) + "/replicas")
            if key.endswith("/ack.json")
        ]
        for ack_key in sorted(ack_keys):
            replica, vector = _parse_ack(conn, book=book, payload=transport.get(ack_key))
            if ack_key != mailbox_ack_key(book, replica["id"]):
                raise AppError("mailbox acknowledgement path is invalid", code="sync_ack_invalid")
            parsed_acknowledgements.append((replica, vector))
        head_keys = [
            key for key in transport.list(mailbox_book_prefix(book) + "/replicas")
            if key.endswith("/head.json")
        ]
        for head_key in sorted(head_keys):
            head, replica = _parse_head(conn, book=book, payload=transport.get(head_key))
            heads_seen += 1
            if head_key != mailbox_head_key(book, replica["id"]):
                raise AppError("mailbox head path binding is invalid", code="sync_mailbox_head_tampered")
            known = conn.execute(
                "SELECT * FROM sync_mailbox_heads WHERE profile_id = ? AND transport_id = ? AND replica_id = ?",
                (profile_id, row["id"], replica["id"]),
            ).fetchone()
            if known and int(head["last_seq"]) < int(known["last_seq"]):
                _notice(
                    conn, profile_id=profile_id, code="sync_mailbox_rollback", severity="blocking",
                    replica_id=replica["id"], member_id=replica["member_id"],
                    details={"known_seq": int(known["last_seq"]), "observed_seq": int(head["last_seq"])},
                )
                continue
            if known and int(head["last_seq"]) == int(known["last_seq"]) and head["head_hash"] != known["head_hash"]:
                _notice(
                    conn, profile_id=profile_id, code="sync_mailbox_equivocation", severity="blocking",
                    replica_id=replica["id"], member_id=replica["member_id"],
                    details={"seq": int(head["last_seq"]), "known_head_hash": known["head_hash"], "observed_head_hash": head["head_hash"]},
                )
                continue
            bundle_prefix = mailbox_replica_prefix(book, replica["id"]) + "/bundles"
            bundle_keys = [key for key in transport.list(bundle_prefix) if key.endswith(".age")]
            parsed_keys = [(key, *_bundle_key_parts(key)) for key in bundle_keys]
            pristine = (
                conn.execute(
                    "SELECT COUNT(*) FROM sync_events WHERE profile_id = ?", (profile_id,)
                ).fetchone()[0]
                == 0
                and conn.execute(
                    "SELECT COUNT(*) FROM sync_row_state WHERE profile_id = ?", (profile_id,)
                ).fetchone()[0]
                == 0
            )
            parsed_keys.sort(
                key=lambda item: (
                    0 if pristine and item[1] else 1,
                    -item[3] if pristine and item[1] else item[2],
                    item[0],
                )
            )
            for bundle_key, _is_snapshot, first_seq, last_seq, named_hash in parsed_keys:
                replica_progress = conn.execute(
                    "SELECT last_seq FROM sync_replicas WHERE id = ?", (replica["id"],)
                ).fetchone()
                if replica_progress and last_seq <= int(replica_progress["last_seq"] or 0):
                    continue
                ciphertext = transport.get(bundle_key)
                bundles_seen += 1
                bundle_hash = sha256_hex(ciphertext)
                if named_hash != bundle_hash:
                    raise AppError("mailbox bundle object hash is invalid", code="sync_mailbox_bundle_tampered")
                already = conn.execute(
                    "SELECT 1 FROM sync_ingests WHERE profile_id = ? AND bundle_hash = ?",
                    (profile_id, bundle_hash),
                ).fetchone()
                imported = import_bundle(
                    conn,
                    profile_id=profile_id,
                    ciphertext=ciphertext,
                    attachments_root=attachments_root,
                )
                if not already:
                    bundles_imported += 1
                applied += imported.applied_events
                duplicates += imported.duplicate_events
                pending += imported.pending_events
                rejected += imported.rejected_events
                conflicts += imported.conflicts_created
            observed_at = now_iso()
            conn.execute(
                """
                INSERT INTO sync_mailbox_heads(
                    profile_id, transport_id, replica_id, last_seq, bundle_hash, head_hash, observed_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(profile_id, transport_id, replica_id) DO UPDATE SET
                    last_seq = excluded.last_seq,
                    bundle_hash = excluded.bundle_hash,
                    head_hash = excluded.head_hash,
                    observed_at = excluded.observed_at
                """,
                (profile_id, row["id"], replica["id"], head["last_seq"], head["bundle_hash"], head["head_hash"], observed_at),
            )
            conn.execute(
                """
                INSERT INTO sync_peer_status(
                    profile_id, transport_id, replica_id, member_id, device_id,
                    last_head_seq, last_head_hash, last_seen_at, last_bundle_at,
                    status, updated_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, 'fresh', ?)
                ON CONFLICT(profile_id, transport_id, replica_id) DO UPDATE SET
                    member_id = excluded.member_id,
                    device_id = excluded.device_id,
                    last_head_seq = excluded.last_head_seq,
                    last_head_hash = excluded.last_head_hash,
                    last_seen_at = excluded.last_seen_at,
                    last_bundle_at = excluded.last_bundle_at,
                    status = 'fresh',
                    updated_at = excluded.updated_at
                """,
                (
                    profile_id, row["id"], replica["id"], replica["member_id"], replica["device_id"],
                    head["last_seq"], head["head_hash"], observed_at, head["created_at"], observed_at,
                ),
            )
        for replica, vector in parsed_acknowledgements:
            record_ack_vector(
                conn,
                profile_id=profile_id,
                observer_replica_id=replica["id"],
                vector=vector,
            )
        timestamp = now_iso()
        conn.execute(
            """
            UPDATE sync_transports
            SET last_pull_at = ?, last_error_at = NULL, last_error_code = NULL, updated_at = ?
            WHERE id = ?
            """,
            (timestamp, timestamp, row["id"]),
        )
        peers = tuple(
            peer_staleness(
                conn,
                profile_id=profile_id,
                transport_id=row["id"],
                stale_after_seconds=stale_after_seconds,
            )
        )
        conn.execute("RELEASE SAVEPOINT sync_mailbox_pull")
        return MailboxPullResult(
            row["id"], row["label"], heads_seen, bundles_seen, bundles_imported,
            applied, duplicates, pending, rejected, conflicts, peers,
        )
    except Exception as exc:
        conn.execute("ROLLBACK TO SAVEPOINT sync_mailbox_pull")
        conn.execute("RELEASE SAVEPOINT sync_mailbox_pull")
        _mark_transport_error(conn, transport_id=row["id"], error=exc)
        raise


def mailbox_status(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    stale_after_seconds: int = DEFAULT_STALE_AFTER_SECONDS,
) -> dict[str, Any]:
    from .transports import list_transports

    _book(conn, profile_id, require_enabled=False)
    transports = list_transports(conn, profile_id=profile_id)
    for transport in transports:
        _seed_peer_rows(conn, profile_id=profile_id, transport_id=transport["id"])
        transport["peers"] = peer_staleness(
            conn,
            profile_id=profile_id,
            transport_id=transport["id"],
            stale_after_seconds=stale_after_seconds,
        )
    notices = [
        {
            "id": item["id"],
            "code": item["code"],
            "severity": item["severity"],
            "replica_id": item["replica_id"],
            "member_id": item["member_id"],
            "details": json.loads(item["details_json"]),
            "created_at": item["created_at"],
        }
        for item in conn.execute(
            "SELECT * FROM sync_notices WHERE profile_id = ? AND acknowledged_at IS NULL ORDER BY created_at DESC",
            (profile_id,),
        ).fetchall()
    ]
    return {"transports": transports, "notices": notices}


def push_result_dict(result: MailboxPushResult) -> dict[str, Any]:
    return asdict(result)


def pull_result_dict(result: MailboxPullResult) -> dict[str, Any]:
    payload = asdict(result)
    payload["peers"] = list(result.peers)
    return payload
