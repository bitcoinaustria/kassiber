"""Signed replica acknowledgements and quorum-safe tombstone compaction."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import json
import sqlite3
from typing import Any, Mapping
import uuid

from ...errors import AppError
from ...time_utils import now_iso


DEFAULT_TOMBSTONE_HORIZON_DAYS = 180
MIN_TOMBSTONE_HORIZON_DAYS = 30


def record_ack_vector(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    observer_replica_id: str,
    vector: Mapping[str, Any],
    observed_hlc: str | None = None,
) -> dict[str, int]:
    observer = conn.execute(
        "SELECT 1 FROM sync_replicas WHERE id = ? AND profile_id = ?",
        (observer_replica_id, profile_id),
    ).fetchone()
    if not observer:
        raise AppError("acknowledgement observer is unknown", code="sync_ack_invalid")
    known = {
        row["id"]: int(row["last_seq"] or 0)
        for row in conn.execute(
            "SELECT id, last_seq FROM sync_replicas WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
    }
    if set(vector) - set(known):
        raise AppError("acknowledgement names an unknown replica", code="sync_ack_invalid")
    accepted: dict[str, int] = {}
    timestamp = now_iso()
    for subject, raw_seq in vector.items():
        if not isinstance(raw_seq, int) or raw_seq < 0:
            raise AppError("acknowledgement sequence is invalid", code="sync_ack_invalid")
        seq = min(raw_seq, known[str(subject)])
        accepted[str(subject)] = seq
        conn.execute(
            """
            INSERT INTO sync_replica_acknowledgements(
                profile_id, observer_replica_id, subject_replica_id,
                acknowledged_seq, observed_hlc, observed_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            ON CONFLICT(profile_id, observer_replica_id, subject_replica_id) DO UPDATE SET
                acknowledged_seq = MAX(sync_replica_acknowledgements.acknowledged_seq, excluded.acknowledged_seq),
                observed_hlc = CASE
                    WHEN excluded.acknowledged_seq >= sync_replica_acknowledgements.acknowledged_seq
                    THEN excluded.observed_hlc ELSE sync_replica_acknowledgements.observed_hlc END,
                observed_at = excluded.observed_at
            """,
            (profile_id, observer_replica_id, subject, seq, observed_hlc, timestamp),
        )
    return accepted


def _parse_time(value: str | None) -> datetime | None:
    if not value:
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)
    except ValueError:
        return None


def tombstone_gc_plan(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    horizon_days: int = DEFAULT_TOMBSTONE_HORIZON_DAYS,
    now: datetime | None = None,
) -> dict[str, Any]:
    if horizon_days < MIN_TOMBSTONE_HORIZON_DAYS:
        raise AppError(
            f"tombstone horizon must be at least {MIN_TOMBSTONE_HORIZON_DAYS} days",
            code="validation",
        )
    book = conn.execute("SELECT * FROM sync_books WHERE profile_id = ?", (profile_id,)).fetchone()
    if not book:
        raise AppError("sync is not configured", code="sync_disabled")
    current = (now or datetime.now(timezone.utc)).astimezone(timezone.utc)
    cutoff = current - timedelta(days=horizon_days)
    active_replicas = conn.execute(
        """
        SELECT r.*, m.display_name, d.label AS device_label
        FROM sync_replicas AS r
        JOIN sync_members AS m ON m.id = r.member_id AND m.revoked_at IS NULL
        JOIN sync_devices AS d ON d.id = r.device_id AND d.revoked_at IS NULL
        WHERE r.profile_id = ?
        ORDER BY r.id
        """,
        (profile_id,),
    ).fetchall()
    items: list[dict[str, Any]] = []
    tombstones = conn.execute(
        """
        SELECT t.*, e.replica_id AS delete_replica_id, e.replica_seq AS delete_replica_seq
        FROM sync_tombstones AS t
        JOIN sync_events AS e ON e.id = t.event_id
        WHERE t.profile_id = ?
        ORDER BY t.deleted_at, t.entity_table, t.entity_key
        """,
        (profile_id,),
    ).fetchall()
    for tombstone in tombstones:
        deleted_at = _parse_time(tombstone["deleted_at"])
        old_enough = bool(deleted_at and deleted_at <= cutoff)
        acknowledgements: dict[str, int] = {}
        missing: list[dict[str, Any]] = []
        subject = tombstone["delete_replica_id"]
        required_seq = int(tombstone["delete_replica_seq"])
        for observer in active_replicas:
            if observer["id"] == book["local_replica_id"] or observer["id"] == subject:
                acknowledged = int(
                    conn.execute(
                        "SELECT last_seq FROM sync_replicas WHERE id = ?", (subject,)
                    ).fetchone()[0]
                )
            else:
                row = conn.execute(
                    """
                    SELECT acknowledged_seq FROM sync_replica_acknowledgements
                    WHERE profile_id = ? AND observer_replica_id = ? AND subject_replica_id = ?
                    """,
                    (profile_id, observer["id"], subject),
                ).fetchone()
                acknowledged = int(row["acknowledged_seq"] or 0) if row else 0
            acknowledgements[observer["id"]] = acknowledged
            if acknowledged < required_seq:
                last_seen = _parse_time(observer["last_seen_at"])
                missing.append(
                    {
                        "replica_id": observer["id"],
                        "member_name": observer["display_name"],
                        "device_label": observer["device_label"],
                        "acknowledged_seq": acknowledged,
                        "required_seq": required_seq,
                        "last_seen_at": observer["last_seen_at"],
                        "offline_past_horizon": bool(last_seen is None or last_seen <= cutoff),
                        "action": "reconnect, or revoke and re-invite this device via an owner snapshot",
                    }
                )
        items.append(
            {
                "entity_table": tombstone["entity_table"],
                "entity_key": tombstone["entity_key"],
                "event_id": tombstone["event_id"],
                "hlc": tombstone["hlc"],
                "deleted_at": tombstone["deleted_at"],
                "gc_after": (deleted_at + timedelta(days=horizon_days)).isoformat().replace("+00:00", "Z") if deleted_at else None,
                "old_enough": old_enough,
                "acknowledgements": acknowledgements,
                "missing_acknowledgements": missing,
                "eligible": old_enough and not missing,
            }
        )
    return {
        "horizon_days": horizon_days,
        "cutoff": cutoff.isoformat().replace("+00:00", "Z"),
        "active_replicas": len(active_replicas),
        "eligible": sum(bool(item["eligible"]) for item in items),
        "blocked": sum(not bool(item["eligible"]) for item in items),
        "items": items,
    }


def compact_tombstones(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    horizon_days: int = DEFAULT_TOMBSTONE_HORIZON_DAYS,
    dry_run: bool = True,
    now: datetime | None = None,
) -> dict[str, Any]:
    plan = tombstone_gc_plan(
        conn,
        profile_id=profile_id,
        horizon_days=horizon_days,
        now=now,
    )
    if dry_run:
        return plan | {"dry_run": True, "compacted": 0}
    book = conn.execute("SELECT * FROM sync_books WHERE profile_id = ?", (profile_id,)).fetchone()
    member = conn.execute(
        "SELECT role, revoked_at FROM sync_members WHERE id = ?", (book["local_member_id"],)
    ).fetchone()
    if not member or member["role"] != "owner" or member["revoked_at"]:
        raise AppError("only an active owner can compact tombstones", code="sync_role_denied")
    compacted = 0
    timestamp = now_iso()
    for item in plan["items"]:
        if not item["eligible"]:
            continue
        conn.execute(
            """
            INSERT OR IGNORE INTO sync_tombstone_gc_log(
                id, profile_id, entity_table, entity_key, delete_event_id,
                delete_hlc, quorum_json, horizon_days, compacted_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                str(uuid.uuid4()), profile_id, item["entity_table"], item["entity_key"],
                item["event_id"], item["hlc"], json.dumps(item["acknowledgements"], sort_keys=True),
                horizon_days, timestamp,
            ),
        )
        cursor = conn.execute(
            """
            DELETE FROM sync_tombstones
            WHERE profile_id = ? AND entity_table = ? AND entity_key = ? AND event_id = ?
            """,
            (profile_id, item["entity_table"], item["entity_key"], item["event_id"]),
        )
        compacted += int(cursor.rowcount)
    return plan | {"dry_run": False, "compacted": compacted, "compacted_at": timestamp}
