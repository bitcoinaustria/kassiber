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
    row = conn.execute(
        "SELECT * FROM workspaces WHERE id = ? OR lower(label) = lower(?) LIMIT 1",
        (ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Workspace '{ref}' not found")
    return row


def resolve_profile(conn, workspace_id, ref=None):
    ref = ref or get_setting(conn, "context_profile")
    if not ref:
        raise AppError("No profile selected. Create one or run `kassiber context set --profile ...`.")
    row = conn.execute(
        """
        SELECT * FROM profiles
        WHERE workspace_id = ? AND (id = ? OR lower(label) = lower(?))
        LIMIT 1
        """,
        (workspace_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Profile '{ref}' not found in the selected workspace")
    return row


def resolve_scope(conn, workspace_ref=None, profile_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)
    return workspace, profile


def invalidate_journals(conn, profile_id):
    conn.execute(
        "UPDATE profiles SET last_processed_at = NULL, last_processed_tx_count = 0 WHERE id = ?",
        (profile_id,),
    )
