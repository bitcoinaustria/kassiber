"""Capture current authored rows as signed upserts and tombstones."""

from __future__ import annotations

import json
import sqlite3
from typing import Any

from ...errors import AppError
from ...time_utils import now_iso
from .crypto import canonical_json_bytes, sha256_hex
from .events import AuthoredEvent, author_event
from .schema_allowlist import SYNC_TABLES, iter_rows, row_key, serialize_row


def _upsert_row_state(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    table: str,
    key: str,
    row_hash: str | None,
    event: AuthoredEvent,
    tombstoned: bool,
) -> None:
    conn.execute(
        """
        INSERT INTO sync_row_state(
            profile_id, entity_table, entity_key, row_hash, last_event_id,
            last_hlc, tombstoned, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, entity_table, entity_key) DO UPDATE SET
            row_hash = excluded.row_hash,
            last_event_id = excluded.last_event_id,
            last_hlc = excluded.last_hlc,
            tombstoned = excluded.tombstoned,
            updated_at = excluded.updated_at
        """,
        (
            profile_id,
            table,
            key,
            row_hash,
            event.id,
            event.hlc,
            1 if tombstoned else 0,
            event.created_at,
        ),
    )


def _record_tombstone(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    table: str,
    key: str,
    reason: str,
) -> AuthoredEvent:
    event = author_event(
        conn,
        profile_id=profile_id,
        event_type="row.delete",
        entity_table=table,
        entity_key=key,
        payload={"key": key, "reason": reason},
    )
    if event is None:
        raise AppError("sync is disabled", code="sync_disabled", retryable=False)
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
            deleted_at = excluded.deleted_at,
            gc_after = NULL
        """,
        (
            profile_id,
            table,
            key,
            event.id,
            event.hlc,
            event.author_member_id,
            event.created_at,
        ),
    )
    _upsert_row_state(
        conn,
        profile_id=profile_id,
        table=table,
        key=key,
        row_hash=None,
        event=event,
        tombstoned=True,
    )
    conn.execute(
        """
        INSERT INTO sync_field_state(
            profile_id, entity_table, entity_key, field, event_id, hlc, value_json
        ) VALUES(?, ?, ?, '__exists__', ?, ?, 'false')
        ON CONFLICT(profile_id, entity_table, entity_key, field) DO UPDATE SET
            event_id = excluded.event_id,
            hlc = excluded.hlc,
            value_json = excluded.value_json
        """,
        (profile_id, table, key, event.id, event.hlc),
    )
    return event


def capture_local_changes(conn: sqlite3.Connection, *, profile_id: str) -> list[AuthoredEvent]:
    """Capture changed authored state since the last successful capture.

    Snapshot comparison is persisted in ``sync_row_state``. It covers every
    mutation path (including raw SQL and cascades) without adding replication
    behavior to sync-disabled books. Missing rows become signed tombstones.
    """

    book = conn.execute(
        "SELECT * FROM sync_books WHERE profile_id = ? AND enabled = 1",
        (profile_id,),
    ).fetchone()
    if not book:
        raise AppError("sync is disabled", code="sync_disabled", retryable=False)

    emitted: list[AuthoredEvent] = []
    for spec in SYNC_TABLES:
        current_keys: set[str] = set()
        for row in iter_rows(conn, spec, profile_id=profile_id):
            key = row_key(spec, row)
            current_keys.add(key)
            is_soft_deleted = bool(spec.soft_delete_column and row[spec.soft_delete_column])
            existing = conn.execute(
                """
                SELECT * FROM sync_row_state
                WHERE profile_id = ? AND entity_table = ? AND entity_key = ?
                """,
                (profile_id, spec.table, key),
            ).fetchone()
            if is_soft_deleted:
                if not existing or not existing["tombstoned"]:
                    emitted.append(
                        _record_tombstone(
                            conn,
                            profile_id=profile_id,
                            table=spec.table,
                            key=key,
                            reason="soft-delete",
                        )
                    )
                continue

            payload = serialize_row(spec, row, hmac_key_b64=book["hmac_key_b64"])
            digest = sha256_hex(canonical_json_bytes(payload))
            if existing and not existing["tombstoned"] and existing["row_hash"] == digest:
                continue
            event = author_event(
                conn,
                profile_id=profile_id,
                event_type="row.upsert",
                entity_table=spec.table,
                entity_key=key,
                payload={"row": payload},
            )
            if event is None:
                raise AppError("sync is disabled", code="sync_disabled", retryable=False)
            conn.execute(
                "DELETE FROM sync_tombstones WHERE profile_id = ? AND entity_table = ? AND entity_key = ?",
                (profile_id, spec.table, key),
            )
            _upsert_row_state(
                conn,
                profile_id=profile_id,
                table=spec.table,
                key=key,
                row_hash=digest,
                event=event,
                tombstoned=False,
            )
            field_rows = [
                (
                    profile_id,
                    spec.table,
                    key,
                    field,
                    event.id,
                    event.hlc,
                    json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True),
                )
                for field, value in payload.items()
            ]
            field_rows.append(
                (profile_id, spec.table, key, "__exists__", event.id, event.hlc, "true")
            )
            conn.executemany(
                """
                INSERT INTO sync_field_state(
                    profile_id, entity_table, entity_key, field, event_id, hlc, value_json
                ) VALUES(?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(profile_id, entity_table, entity_key, field) DO UPDATE SET
                    event_id = excluded.event_id,
                    hlc = excluded.hlc,
                    value_json = excluded.value_json
                """,
                field_rows,
            )
            emitted.append(event)

        previous_rows = conn.execute(
            """
            SELECT entity_key FROM sync_row_state
            WHERE profile_id = ? AND entity_table = ? AND tombstoned = 0
            """,
            (profile_id, spec.table),
        ).fetchall()
        for previous in previous_rows:
            key = previous["entity_key"]
            if key in current_keys:
                continue
            emitted.append(
                _record_tombstone(
                    conn,
                    profile_id=profile_id,
                    table=spec.table,
                    key=key,
                    reason="row-missing",
                )
            )
    return emitted


def capture_full_snapshot(conn: sqlite3.Connection, *, profile_id: str) -> list[AuthoredEvent]:
    """Author a complete current-state checkpoint after normal capture.

    Snapshot events are ordinary signed row/tombstone events, so established
    peers can replay them without a second merge implementation. A joining
    peer uses the signed base checkpoint in the enclosing bundle to skip
    historical ciphertext that predates its device recipient key.
    """

    book = conn.execute(
        "SELECT * FROM sync_books WHERE profile_id = ? AND enabled = 1",
        (profile_id,),
    ).fetchone()
    if not book:
        raise AppError("sync is disabled", code="sync_disabled", retryable=False)
    emitted: list[AuthoredEvent] = []
    for spec in SYNC_TABLES:
        for row in iter_rows(conn, spec, profile_id=profile_id):
            if spec.soft_delete_column and row[spec.soft_delete_column]:
                continue
            key = row_key(spec, row)
            payload = serialize_row(spec, row, hmac_key_b64=book["hmac_key_b64"])
            digest = sha256_hex(canonical_json_bytes(payload))
            event = author_event(
                conn,
                profile_id=profile_id,
                event_type="row.upsert",
                entity_table=spec.table,
                entity_key=key,
                payload={"row": payload, "snapshot": True},
            )
            if event is None:
                raise AppError("sync is disabled", code="sync_disabled", retryable=False)
            _upsert_row_state(
                conn,
                profile_id=profile_id,
                table=spec.table,
                key=key,
                row_hash=digest,
                event=event,
                tombstoned=False,
            )
            field_rows = [
                (
                    profile_id,
                    spec.table,
                    key,
                    field,
                    event.id,
                    event.hlc,
                    json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True),
                )
                for field, value in payload.items()
            ]
            field_rows.append(
                (profile_id, spec.table, key, "__exists__", event.id, event.hlc, "true")
            )
            conn.executemany(
                """
                INSERT INTO sync_field_state(
                    profile_id, entity_table, entity_key, field, event_id, hlc, value_json
                ) VALUES(?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(profile_id, entity_table, entity_key, field) DO UPDATE SET
                    event_id = excluded.event_id,
                    hlc = excluded.hlc,
                    value_json = excluded.value_json
                """,
                field_rows,
            )
            emitted.append(event)

        tombstones = conn.execute(
            """
            SELECT entity_key FROM sync_tombstones
            WHERE profile_id = ? AND entity_table = ?
            ORDER BY entity_key
            """,
            (profile_id, spec.table),
        ).fetchall()
        for tombstone in tombstones:
            emitted.append(
                _record_tombstone(
                    conn,
                    profile_id=profile_id,
                    table=spec.table,
                    key=tombstone["entity_key"],
                    reason="snapshot-checkpoint",
                )
            )

    # The current row state is insufficient to reconstruct append-only edit
    # provenance on a brand-new peer. Re-attest each history entry as a new
    # signed snapshot event while preserving its stable history/field UUIDs.
    history_rows = conn.execute(
        """
        SELECT * FROM transaction_edit_events
        WHERE profile_id = ?
        ORDER BY changed_at, id
        """,
        (profile_id,),
    ).fetchall()
    for history in history_rows:
        fields = conn.execute(
            "SELECT * FROM transaction_edit_fields WHERE event_id = ? ORDER BY id",
            (history["id"],),
        ).fetchall()
        event = author_event(
            conn,
            profile_id=profile_id,
            event_type="transaction.edit",
            entity_table="transaction_edit_events",
            entity_key=history["id"],
            payload={
                "transaction_id": history["transaction_id"],
                "wallet_id": history["wallet_id"],
                "transaction_external_id": history["transaction_external_id"],
                "transaction_occurred_at": history["transaction_occurred_at"],
                "source": history["source"],
                "reason": history["reason"],
                "changed_at": history["changed_at"],
                "snapshot": True,
                "fields": [
                    {
                        "id": field["id"],
                        "field": field["field"],
                        "before_value": json.loads(field["before_value"]),
                        "after_value": json.loads(field["after_value"]),
                        "diff": json.loads(field["diff_json"]),
                    }
                    for field in fields
                ],
            },
        )
        if event is None:
            raise AppError("sync is disabled", code="sync_disabled", retryable=False)
        emitted.append(event)
    return emitted


def authored_state_digest(conn: sqlite3.Connection, *, profile_id: str) -> str:
    """Deterministic digest used by convergence tests and diagnostics."""

    rows: list[dict[str, Any]] = []
    for spec in SYNC_TABLES:
        for row in iter_rows(conn, spec, profile_id=profile_id):
            rows.append(
                {
                    "table": spec.table,
                    "key": row_key(spec, row),
                    "row": {column: row[column] for column in spec.columns},
                }
            )
    rows.sort(key=lambda item: (item["table"], item["key"]))
    return sha256_hex(canonical_json_bytes(rows))
