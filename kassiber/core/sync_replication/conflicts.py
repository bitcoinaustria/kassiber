"""Blocking conflict review and signed human resolutions."""

from __future__ import annotations

import json
from pathlib import Path
import sqlite3
from typing import Any, Mapping

from ...errors import AppError
from ...time_utils import now_iso
from ..repo import invalidate_journals
from .bundle import ParsedBundle
from .events import author_event
from .schema_allowlist import SYNC_TABLE_MAP


def _is_immutable_conflict(conflict, spec) -> bool:
    if not spec.immutable_fields:
        return False
    field = str(conflict["field"])
    return field == "__exists__" or field in spec.immutable_fields


def _materialized_conflict_value(conn, *, profile_id: str, conflict, spec) -> Any:
    from .merge import _mapped_id

    try:
        wire_pk = json.loads(conflict["entity_key"])
    except (TypeError, ValueError, json.JSONDecodeError) as exc:
        raise AppError("conflict entity key is invalid", code="sync_conflict_invalid") from exc
    if not isinstance(wire_pk, list) or len(wire_pk) != len(spec.primary_key):
        raise AppError("conflict entity key is invalid", code="sync_conflict_invalid")
    local_pk = tuple(
        _mapped_id(
            conn,
            profile_id=profile_id,
            table=spec.table,
            wire_id=item,
        )
        for item in wire_pk
    )
    where = " AND ".join(f"{column} = ?" for column in spec.primary_key)
    row = conn.execute(f"SELECT * FROM {spec.table} WHERE {where}", local_pk).fetchone()
    if conflict["field"] == "__exists__":
        return row is not None
    if row is None:
        raise AppError("conflicted row no longer exists", code="sync_conflict_invalid")
    value = row[conflict["field"]]
    if conflict["field"] in spec.json_columns and value is not None:
        value = json.loads(value)
    return value


def _require_immutable_resolution_matches_materialized(
    conn, *, profile_id: str, conflict, spec, value: Any
) -> None:
    if not _is_immutable_conflict(conflict, spec):
        return
    if _materialized_conflict_value(
        conn, profile_id=profile_id, conflict=conflict, spec=spec
    ) == value:
        return
    raise AppError(
        "custody revision conflicts cannot rewrite authored economic facts in place",
        code="sync_conflict_requires_revision",
        hint=(
            "Acknowledge the currently materialized signed fact to close this "
            "conflict, then create a new custody component revision if it is wrong."
        ),
        details={
            "entity_table": conflict["entity_table"],
            "entity_key": conflict["entity_key"],
            "field": conflict["field"],
        },
        retryable=False,
    )


def list_conflicts(conn: sqlite3.Connection, *, profile_id: str, include_resolved: bool = False) -> list[dict[str, Any]]:
    where = "profile_id = ?" if include_resolved else "profile_id = ? AND status = 'open'"
    rows = conn.execute(
        f"""
        SELECT * FROM sync_conflicts
        WHERE {where}
        ORDER BY CASE status WHEN 'open' THEN 0 ELSE 1 END, created_at, id
        """,
        (profile_id,),
    ).fetchall()
    output = []
    for row in rows:
        item = dict(row)
        item["first_value"] = json.loads(item.pop("local_value_json"))
        item["second_value"] = json.loads(item.pop("remote_value_json"))
        item["first_event_id"] = item.pop("local_event_id")
        item["second_event_id"] = item.pop("remote_event_id")
        authors = conn.execute(
            """
            SELECT e.id, e.replica_id, e.replica_seq, e.hlc, e.author_member_id,
                   m.display_name, m.role
            FROM sync_events e
            JOIN sync_members m ON m.id = e.author_member_id
            WHERE e.id IN (?, ?)
            ORDER BY e.id
            """,
            (item["first_event_id"], item["second_event_id"]),
        ).fetchall()
        item["events"] = [dict(author) for author in authors]
        output.append(item)
    return output


def _event_row(conn: sqlite3.Connection, event_id: str) -> dict[str, Any]:
    row = conn.execute("SELECT * FROM sync_events WHERE id = ?", (event_id,)).fetchone()
    if not row:
        raise AppError("conflict source event was not found", code="sync_conflict_invalid")
    event = dict(row)
    event["payload"] = json.loads(event.pop("payload_json"))
    event["context"] = json.loads(event.pop("context_json"))
    event.pop("applied_at")
    return event


def _resolved_value(conflict, *, source_event_id: str | None, custom_value: Any) -> tuple[Any, str | None]:
    if source_event_id is not None:
        if source_event_id == conflict["local_event_id"]:
            return json.loads(conflict["local_value_json"]), source_event_id
        if source_event_id == conflict["remote_event_id"]:
            return json.loads(conflict["remote_value_json"]), source_event_id
        raise AppError(
            "chosen event is not one of the conflicting edits",
            code="sync_conflict_invalid",
            retryable=False,
        )
    return custom_value, None


def apply_resolution_event(
    conn: sqlite3.Connection,
    *,
    book,
    event: Mapping[str, Any],
    parsed: ParsedBundle,
    attachments_root: Path | None,
    created_files: list[Path],
) -> bool:
    from .merge import (
        _apply_row_delete,
        _apply_row_upsert,
        _entity_pk,
        _mapped_id,
        _write_field_state,
    )

    payload = event.get("payload") or {}
    conflict_id = str(payload.get("conflict_id") or "")
    conflict = conn.execute(
        "SELECT * FROM sync_conflicts WHERE id = ? AND profile_id = ?",
        (conflict_id, book["profile_id"]),
    ).fetchone()
    if not conflict:
        raise AppError("sync conflict was not found", code="sync_conflict_invalid")
    if conflict["status"] == "resolved":
        if conflict["resolution_event_id"] == event["id"]:
            return False
        raise AppError("sync conflict already has another resolution", code="sync_conflict_invalid")
    if (
        payload.get("entity_table") != conflict["entity_table"]
        or payload.get("entity_key") != conflict["entity_key"]
        or payload.get("field") != conflict["field"]
    ):
        raise AppError("resolution does not match its conflict", code="sync_conflict_invalid")
    value = payload.get("value")
    source_event_id = payload.get("source_event_id")
    if source_event_id not in {None, conflict["local_event_id"], conflict["remote_event_id"]}:
        raise AppError("resolution source event is invalid", code="sync_conflict_invalid")
    spec = SYNC_TABLE_MAP.get(conflict["entity_table"])
    if not spec:
        raise AppError("conflict table is outside sync allowlist", code="sync_schema_forbidden")
    _require_immutable_resolution_matches_materialized(
        conn,
        profile_id=book["profile_id"],
        conflict=conflict,
        spec=spec,
        value=value,
    )

    mutated = False
    if _is_immutable_conflict(conflict, spec):
        # Immutable authored facts are never rewritten. Matching the current
        # materialization is an audited acknowledgement that closes the row;
        # a different outcome requires a new signed custody revision.
        mutated = False
    elif conflict["field"] == "__exists__":
        if not isinstance(value, bool):
            raise AppError("row-existence resolution must be boolean", code="sync_conflict_invalid")
        if value:
            if not source_event_id:
                raise AppError("restoring a row requires an upsert source event", code="sync_conflict_invalid")
            source = _event_row(conn, str(source_event_id))
            if source["event_type"] != "row.upsert":
                raise AppError("chosen source event does not contain the row", code="sync_conflict_invalid")
            synthetic = dict(event)
            synthetic["event_type"] = "row.upsert"
            synthetic["payload"] = source["payload"]
            mutated, _ = _apply_row_upsert(
                conn,
                book=book,
                event=synthetic,
                parsed=parsed,
                attachments_root=attachments_root,
                created_files=created_files,
            )
        else:
            synthetic = dict(event)
            synthetic["event_type"] = "row.delete"
            mutated, _ = _apply_row_delete(conn, book=book, event=synthetic)
    else:
        if conflict["field"] not in spec.columns:
            raise AppError("conflict field is outside sync allowlist", code="sync_schema_forbidden")
        wire_pk = _entity_pk(spec, event)
        local_pk = tuple(
            _mapped_id(
                conn,
                profile_id=book["profile_id"],
                table=spec.table,
                wire_id=item,
            )
            for item in wire_pk
        )
        stored_value = value
        if conflict["field"] in spec.json_columns and value is not None:
            stored_value = json.dumps(value, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
        where = " AND ".join(f"{column} = ?" for column in spec.primary_key)
        cursor = conn.execute(
            f"UPDATE {spec.table} SET {conflict['field']} = ? WHERE {where}",
            (stored_value, *local_pk),
        )
        if not cursor.rowcount:
            raise AppError("conflicted row no longer exists", code="sync_conflict_invalid")
        _write_field_state(
            conn,
            profile_id=book["profile_id"],
            table=spec.table,
            key=conflict["entity_key"],
            field=conflict["field"],
            event=event,
            value=value,
        )
        mutated = True
    conn.execute(
        """
        UPDATE sync_conflicts
        SET status = 'resolved', resolution_event_id = ?,
            resolved_by_member_id = ?, resolved_at = ?
        WHERE id = ?
        """,
        (event["id"], event["author_member_id"], now_iso(), conflict["id"]),
    )
    return mutated


def resolve_conflict(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    conflict_id: str,
    source_event_id: str | None = None,
    custom_value: Any = None,
    use_custom_value: bool = False,
) -> dict[str, Any]:
    if (source_event_id is None) == (not use_custom_value):
        raise AppError(
            "choose exactly one source event or a custom value",
            code="validation",
            retryable=False,
        )
    conflict = conn.execute(
        "SELECT * FROM sync_conflicts WHERE id = ? AND profile_id = ? AND status = 'open'",
        (conflict_id, profile_id),
    ).fetchone()
    if not conflict:
        raise AppError("open sync conflict was not found", code="not_found")
    spec = SYNC_TABLE_MAP.get(conflict["entity_table"])
    if not spec:
        raise AppError("conflict table is outside sync allowlist", code="sync_schema_forbidden")
    value, chosen_event_id = _resolved_value(
        conflict,
        source_event_id=source_event_id,
        custom_value=custom_value,
    )
    _require_immutable_resolution_matches_materialized(
        conn,
        profile_id=profile_id,
        conflict=conflict,
        spec=spec,
        value=value,
    )
    event = author_event(
        conn,
        profile_id=profile_id,
        event_type="conflict.resolve",
        entity_table=conflict["entity_table"],
        entity_key=conflict["entity_key"],
        payload={
            "conflict_id": conflict["id"],
            "entity_table": conflict["entity_table"],
            "entity_key": conflict["entity_key"],
            "field": conflict["field"],
            "value": value,
            "source_event_id": chosen_event_id,
        },
        allow_disabled=True,
    )
    if event is None:
        raise AppError("sync is not configured", code="sync_disabled")
    book = conn.execute("SELECT * FROM sync_books WHERE profile_id = ?", (profile_id,)).fetchone()
    empty_bundle = ParsedBundle(bundle_hash="local", manifest={}, events=(), blobs={})
    mutated = apply_resolution_event(
        conn,
        book=book,
        event=event.to_wire_dict(),
        parsed=empty_bundle,
        attachments_root=None,
        created_files=[],
    )
    if mutated:
        if conflict["entity_table"] in {
            "custody_components",
            "custody_component_legs",
            "custody_component_allocations",
        }:
            from ..custody_components import reconcile_active_memberships

            reconcile_active_memberships(conn, profile_id=profile_id)
        invalidate_journals(conn, profile_id)
    return {
        "conflict_id": conflict["id"],
        "status": "resolved",
        "resolution_event_id": event.id,
        "resolved_value": value,
        "source_event_id": chosen_event_id,
    }
