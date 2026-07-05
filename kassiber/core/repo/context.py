from __future__ import annotations

from ...db import get_setting
from ...errors import AppError


def current_context_ids(conn):
    return get_setting(conn, "context_workspace"), get_setting(conn, "context_profile")


def current_context_snapshot(conn):
    workspace_id, profile_id = current_context_ids(conn)
    workspace = (
        conn.execute("SELECT id, label FROM workspaces WHERE id = ?", (workspace_id,)).fetchone()
        if workspace_id
        else None
    )
    profile = (
        conn.execute("SELECT id, workspace_id, label FROM profiles WHERE id = ?", (profile_id,)).fetchone()
        if profile_id
        else None
    )
    if workspace and profile and profile["workspace_id"] != workspace["id"]:
        profile = None
    return {
        "workspace_id": workspace["id"] if workspace else "",
        "workspace_label": workspace["label"] if workspace else "",
        "profile_id": profile["id"] if profile else "",
        "profile_label": profile["label"] if profile else "",
    }


def resolve_workspace(conn, ref=None):
    ref = ref or get_setting(conn, "context_workspace")
    if not ref:
        raise AppError("No workspace selected. Create one or run `kassiber context set --workspace ...`.")
    ref = str(ref).strip()
    if not ref:
        raise AppError("No workspace selected. Create one or run `kassiber context set --workspace ...`.")
    row = conn.execute(
        "SELECT * FROM workspaces WHERE id = ? LIMIT 1",
        (ref,),
    ).fetchone()
    if row:
        return row
    rows = conn.execute(
        "SELECT * FROM workspaces WHERE lower(label) = lower(?) ORDER BY label ASC, id ASC",
        (ref,),
    ).fetchall()
    if len(rows) == 1:
        return rows[0]
    if len(rows) > 1:
        raise AppError(
            f"Workspace label '{ref}' is ambiguous",
            code="validation",
            hint="Use the workspace id instead of the non-unique label.",
            details={
                "matches": [
                    {"id": row["id"], "label": row["label"]}
                    for row in rows
                ]
            },
        )
    raise AppError(f"Workspace '{ref}' not found", code="not_found")


def resolve_profile(conn, workspace_id, ref=None):
    ref = ref or get_setting(conn, "context_profile")
    if not ref:
        raise AppError("No profile selected. Create one or run `kassiber context set --profile ...`.")
    ref = str(ref).strip()
    if not ref:
        raise AppError("No profile selected. Create one or run `kassiber context set --profile ...`.")
    row = conn.execute(
        """
        SELECT * FROM profiles
        WHERE workspace_id = ? AND id = ?
        LIMIT 1
        """,
        (workspace_id, ref),
    ).fetchone()
    if row:
        return row
    rows = conn.execute(
        """
        SELECT * FROM profiles
        WHERE workspace_id = ? AND lower(label) = lower(?)
        ORDER BY label ASC, id ASC
        """,
        (workspace_id, ref),
    ).fetchall()
    if len(rows) == 1:
        return rows[0]
    if len(rows) > 1:
        raise AppError(
            f"Profile label '{ref}' is ambiguous in the selected workspace",
            code="validation",
            hint="Use the profile id instead of the non-unique label.",
            details={
                "matches": [
                    {"id": row["id"], "label": row["label"]}
                    for row in rows
                ]
            },
        )
    raise AppError(f"Profile '{ref}' not found in the selected workspace", code="not_found")


def resolve_scope(conn, workspace_ref=None, profile_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)
    return workspace, profile


def invalidate_journals(conn, profile_id):
    conn.execute(
        """
        UPDATE profiles
        SET last_processed_at = NULL,
            last_processed_tx_count = 0,
            journal_input_version = journal_input_version + 1
        WHERE id = ?
        """,
        (profile_id,),
    )
