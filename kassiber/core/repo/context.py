from __future__ import annotations

from ...db import get_setting


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
        conn.execute("SELECT id, label FROM profiles WHERE id = ?", (profile_id,)).fetchone()
        if profile_id
        else None
    )
    return {
        "workspace_id": workspace["id"] if workspace else "",
        "workspace_label": workspace["label"] if workspace else "",
        "profile_id": profile["id"] if profile else "",
        "profile_label": profile["label"] if profile else "",
    }
