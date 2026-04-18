from __future__ import annotations

import sqlite3
from typing import Any

from ..backends import load_runtime_config, merge_db_backends
from ..core import accounts as core_accounts
from ..core import wallets as core_wallets
from ..core.repo import current_context_snapshot, resolve_scope
from ..core.runtime import build_status_payload, ensure_runtime_layout, resolve_runtime_paths
from ..db import open_db
from ..errors import AppError


def _empty_shell(notices: list[str] | None = None) -> dict[str, Any]:
    return {
        "phase": 1,
        "window_title": "Kassiber",
        "project_label": "No project selected",
        "current_workspace_label": "",
        "current_profile_label": "",
        "connection_count": 0,
        "is_empty": True,
        "empty_state_title": "Create a profile in the CLI first",
        "empty_state_body": (
            "Kassiber's Phase 1 desktop shell is ready, but it still depends on the existing "
            "workspace/profile setup from the CLI."
        ),
        "placeholder_title": "Dashboard coming next",
        "placeholder_body": (
            "The PySide6 shell is in place. Read-only dashboard tiles land in Phase 2."
        ),
        "notices": notices or [],
    }


def collect_ui_snapshot(
    conn: sqlite3.Connection,
    data_root: str,
    runtime_config: dict[str, Any],
    workspace_ref: str | None = None,
    profile_ref: str | None = None,
) -> dict[str, Any]:
    status = build_status_payload(conn, data_root)
    status["default_backend"] = runtime_config.get("default_backend", "")
    status["env_file"] = runtime_config.get("env_file", "")

    snapshot: dict[str, Any] = {
        "status": status,
        "context": current_context_snapshot(conn),
        "scope": None,
        "profiles": [],
        "shell": _empty_shell(),
    }
    explicit_scope = bool(workspace_ref or profile_ref)

    try:
        workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    except AppError as exc:
        if explicit_scope:
            raise
        snapshot["shell"] = _empty_shell([str(exc)])
        return snapshot

    profiles = core_accounts.list_profiles(conn, workspace["id"])
    wallets = core_wallets.list_wallets(conn, workspace["id"], profile["id"])
    connection_count = len(wallets)
    notices = [
        "The Add Connection modal is a Phase 1 placeholder. Use the CLI for wallet creation and sync today.",
    ]
    if connection_count:
        notices.append("Connections already exist. The live dashboard tiles arrive in Phase 2.")

    snapshot["scope"] = {
        "workspace_id": workspace["id"],
        "workspace_label": workspace["label"],
        "profile_id": profile["id"],
        "profile_label": profile["label"],
    }
    snapshot["profiles"] = profiles
    snapshot["shell"] = {
        "phase": 1,
        "window_title": "Kassiber",
        "project_label": f"{workspace['label']} / {profile['label']}",
        "current_workspace_label": workspace["label"],
        "current_profile_label": profile["label"],
        "connection_count": connection_count,
        "is_empty": connection_count == 0,
        "empty_state_title": "Add a connection",
        "empty_state_body": "Add a connection and automatically sync your transaction data to get started.",
        "placeholder_title": "Connections detected",
        "placeholder_body": (
            "Your current project already has connection data, but the read-only dashboard tiles "
            "still land in Phase 2."
        ),
        "notices": notices,
    }
    return snapshot


def load_ui_snapshot(
    data_root: str | None = None,
    env_file: str | None = None,
    workspace_ref: str | None = None,
    profile_ref: str | None = None,
) -> dict[str, Any]:
    paths = ensure_runtime_layout(resolve_runtime_paths(data_root, env_file))
    runtime_config = load_runtime_config(paths.env_file)
    conn = open_db(paths.data_root)
    try:
        merge_db_backends(conn, runtime_config)
        return collect_ui_snapshot(
            conn,
            paths.data_root,
            runtime_config,
            workspace_ref=workspace_ref,
            profile_ref=profile_ref,
        )
    finally:
        conn.close()


__all__ = ["collect_ui_snapshot", "load_ui_snapshot"]
