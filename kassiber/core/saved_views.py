"""Generic saved-view storage, ``surface``-discriminated.

A *saved view* is a named filter snapshot a user can re-open from the
review queue header chip. The first consumer is the swap-candidate
queue (``surface = "swap_candidates"``); future surfaces — quarantine
review, source-funds suggestions, transaction filters — pick the same
schema for free by passing a different surface label.

The table this writes through (`saved_views`) is defined in commit 1
and scoped per profile:

    saved_views(id, workspace_id, profile_id, surface, name,
                filter_json, created_at, updated_at)

Filter contents are opaque JSON. The matcher / queue UI defines the
schema for its surface and round-trips it through this layer
unmodified. Empty filters are valid (``{}``).

This module is the thin CRUD seam — no business logic, no daemon
plumbing. Callers manage the connection and commit boundary; helpers
return rows as plain dicts so machine envelopes stay deterministic.
"""

from __future__ import annotations

import json
import sqlite3
import uuid
from typing import Any, Mapping, Optional

from ..errors import AppError
from ..time_utils import now_iso


SURFACE_SWAP_CANDIDATES = "swap_candidates"


def create_view(
    conn: sqlite3.Connection,
    workspace_id: str,
    profile_id: str,
    *,
    surface: str,
    name: str,
    filter_payload: Optional[Mapping[str, Any]] = None,
    commit: bool = True,
) -> dict:
    """Insert one saved view. Raises ``AppError`` on duplicate ``(profile,
    surface, name)`` or empty inputs.
    """
    _require_non_empty("surface", surface)
    _require_non_empty("name", name)
    payload = dict(filter_payload or {})
    view_id = str(uuid.uuid4())
    timestamp = now_iso()
    filter_json = json.dumps(payload, sort_keys=True)
    try:
        conn.execute(
            """
            INSERT INTO saved_views(id, workspace_id, profile_id, surface, name,
                                    filter_json, created_at, updated_at)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (view_id, workspace_id, profile_id, surface, name, filter_json, timestamp, timestamp),
        )
    except sqlite3.IntegrityError as exc:
        raise AppError(
            f"Saved view '{name}' already exists on surface '{surface}' for this profile",
            code="conflict",
            details={"surface": surface, "name": name},
        ) from exc
    if commit:
        conn.commit()
    return _row_to_dict(
        conn.execute("SELECT * FROM saved_views WHERE id = ?", (view_id,)).fetchone()
    )


def list_views(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    surface: Optional[str] = None,
) -> list[dict]:
    """Return saved views for ``profile_id``, optionally filtered by
    ``surface``. Sorted by ``created_at DESC`` so the most recently saved
    chips appear first in the UI.
    """
    if surface:
        rows = conn.execute(
            """
            SELECT * FROM saved_views
            WHERE profile_id = ? AND surface = ?
            ORDER BY created_at DESC, id ASC
            """,
            (profile_id, surface),
        ).fetchall()
    else:
        rows = conn.execute(
            """
            SELECT * FROM saved_views
            WHERE profile_id = ?
            ORDER BY surface ASC, created_at DESC, id ASC
            """,
            (profile_id,),
        ).fetchall()
    return [_row_to_dict(row) for row in rows]


def delete_view(
    conn: sqlite3.Connection,
    profile_id: str,
    view_id: str,
    *,
    commit: bool = True,
) -> dict:
    """Delete one saved view. Raises ``AppError(code="not_found")`` when
    the id does not belong to ``profile_id``.
    """
    row = conn.execute(
        "SELECT * FROM saved_views WHERE id = ? AND profile_id = ?",
        (view_id, profile_id),
    ).fetchone()
    if not row:
        raise AppError(
            f"Saved view '{view_id}' not found for this profile",
            code="not_found",
            details={"id": view_id},
        )
    conn.execute("DELETE FROM saved_views WHERE id = ?", (view_id,))
    if commit:
        conn.commit()
    return {"deleted": view_id}


def update_view(
    conn: sqlite3.Connection,
    profile_id: str,
    view_id: str,
    *,
    name: Optional[str] = None,
    filter_payload: Optional[Mapping[str, Any]] = None,
    commit: bool = True,
) -> dict:
    """Update name / filter payload on a saved view.

    Either argument is optional — a no-op call still touches
    ``updated_at`` so chip ordering reflects user activity. Raises
    ``AppError`` on missing rows or duplicate name within the same
    surface.
    """
    row = conn.execute(
        "SELECT * FROM saved_views WHERE id = ? AND profile_id = ?",
        (view_id, profile_id),
    ).fetchone()
    if not row:
        raise AppError(
            f"Saved view '{view_id}' not found for this profile",
            code="not_found",
            details={"id": view_id},
        )
    new_name = row["name"] if name is None else name
    new_filter_payload = (
        json.loads(row["filter_json"] or "{}")
        if filter_payload is None
        else dict(filter_payload)
    )
    _require_non_empty("name", new_name)
    timestamp = now_iso()
    try:
        conn.execute(
            """
            UPDATE saved_views
            SET name = ?, filter_json = ?, updated_at = ?
            WHERE id = ?
            """,
            (new_name, json.dumps(new_filter_payload, sort_keys=True), timestamp, view_id),
        )
    except sqlite3.IntegrityError as exc:
        raise AppError(
            f"Saved view name '{new_name}' already exists on surface '{row['surface']}'",
            code="conflict",
            details={"surface": row["surface"], "name": new_name},
        ) from exc
    if commit:
        conn.commit()
    return _row_to_dict(
        conn.execute("SELECT * FROM saved_views WHERE id = ?", (view_id,)).fetchone()
    )


def _row_to_dict(row) -> dict:
    try:
        filter_payload = json.loads(row["filter_json"] or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        filter_payload = {}
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "surface": row["surface"],
        "name": row["name"],
        "filter": filter_payload,
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


def _require_non_empty(field: str, value: Optional[str]) -> None:
    if not value or not str(value).strip():
        raise AppError(f"Missing required field '{field}'", code="validation")
