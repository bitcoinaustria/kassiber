"""Typed desktop daemon surface for opt-in authored-event replication."""

from __future__ import annotations

import base64
from dataclasses import asdict
from pathlib import Path
import sqlite3
from typing import Any, Callable, Mapping

from .core.repo import resolve_scope
from .core.sync_replication.conflicts import list_conflicts, resolve_conflict
from .core.sync_replication.identity import disable_sync, enable_sync, sync_status
from .core.sync_replication.mailbox import mailbox_status, pull_mailbox, push_mailbox
from .core.sync_replication.membership import (
    create_invitation,
    create_join_request,
    join_invitation,
    list_devices,
    list_members,
    revoke_device,
    revoke_member,
)
from .core.sync_replication.transports import (
    configure_transport,
    delete_transport,
)
from .db import resolve_attachments_root
from .errors import AppError


Progress = Callable[[str, Mapping[str, Any]], None]


def _required_text(args: Mapping[str, Any], key: str) -> str:
    value = args.get(key)
    if not isinstance(value, str) or not value.strip():
        raise AppError(f"{key} is required", code="validation")
    return value.strip()


def _scope(conn: sqlite3.Connection) -> tuple[Any, Any]:
    return resolve_scope(conn, None, None)


def _status(conn: sqlite3.Connection, profile_id: str) -> dict[str, Any]:
    identity = sync_status(conn, profile_id=profile_id)
    mailbox = mailbox_status(conn, profile_id=profile_id) if identity["configured"] else {
        "transports": [],
        "notices": [],
    }
    return {
        **identity,
        **mailbox,
        "members_list": list_members(conn, profile_id=profile_id) if identity["configured"] else [],
        "devices_list": list_devices(conn, profile_id=profile_id) if identity["configured"] else [],
        "conflicts": list_conflicts(conn, profile_id=profile_id) if identity["configured"] else [],
    }


def dispatch_sync_ui(
    conn: sqlite3.Connection,
    *,
    data_root: Path,
    kind: str,
    args: Mapping[str, Any],
    progress: Progress | None = None,
) -> dict[str, Any]:
    """Dispatch a denylisted ``ui.sync.*`` kind and commit local mutations."""

    progress = progress or (lambda _stage, _details: None)
    if kind == "ui.sync.join_request":
        result = create_join_request(
            conn,
            member_name=_required_text(args, "member_name"),
            device_label=_required_text(args, "device_label"),
        )
        conn.commit()
        return result
    if kind == "ui.sync.join":
        encoded = _required_text(args, "invitation")
        try:
            ciphertext = base64.b64decode(encoded, validate=True)
        except ValueError as exc:
            raise AppError("invitation code is invalid", code="sync_invitation_invalid") from exc
        result = join_invitation(
            conn,
            request_id=_required_text(args, "request_id"),
            ciphertext=ciphertext,
        )
        conn.commit()
        return result

    workspace, profile = _scope(conn)
    profile_id = profile["id"]
    if kind == "ui.sync.status":
        return _status(conn, profile_id)
    if kind == "ui.sync.enable":
        current = sync_status(conn, profile_id=profile_id)
        if current["configured"]:
            member = conn.execute(
                "SELECT display_name FROM sync_members WHERE id = ?",
                (current["local_member_id"],),
            ).fetchone()
            device = conn.execute(
                "SELECT label FROM sync_devices WHERE id = ?",
                (current["local_device_id"],),
            ).fetchone()
            member_name = member["display_name"]
            device_label = device["label"]
        else:
            member_name = _required_text(args, "member_name")
            device_label = _required_text(args, "device_label")
        result = enable_sync(
            conn,
            workspace_id=workspace["id"],
            profile_id=profile_id,
            member_name=member_name,
            device_label=device_label,
        )
        conn.commit()
        return _status(conn, profile_id) | {"identity": result}
    if kind == "ui.sync.disable":
        result = disable_sync(conn, profile_id=profile_id)
        conn.commit()
        return _status(conn, profile_id) | {"identity": result}
    if kind == "ui.sync.transports.list":
        return mailbox_status(conn, profile_id=profile_id)
    if kind == "ui.sync.transports.configure":
        config = args.get("config")
        credentials = args.get("credentials")
        if not isinstance(config, Mapping) or (
            credentials is not None and not isinstance(credentials, Mapping)
        ):
            raise AppError("transport config is invalid", code="validation")
        result = configure_transport(
            conn,
            profile_id=profile_id,
            kind=_required_text(args, "kind"),
            label=_required_text(args, "label"),
            config=config,
            credentials=credentials,
        )
        conn.commit()
        return result
    if kind == "ui.sync.transports.delete":
        result = delete_transport(
            conn,
            profile_id=profile_id,
            transport_id=_required_text(args, "transport_id"),
        )
        conn.commit()
        return result
    if kind == "ui.sync.push":
        progress("capture", {"direction": "push"})
        result = push_mailbox(
            conn,
            profile_id=profile_id,
            transport_id=args.get("transport_id") if isinstance(args.get("transport_id"), str) else None,
            transport_label=args.get("transport_label") if isinstance(args.get("transport_label"), str) else None,
            attachments_root=resolve_attachments_root(data_root),
            snapshot=bool(args.get("snapshot", False)),
        )
        conn.commit()
        progress("complete", {"direction": "push", "event_count": result.event_count})
        return asdict(result)
    if kind == "ui.sync.pull":
        progress("poll", {"direction": "pull"})
        result = pull_mailbox(
            conn,
            profile_id=profile_id,
            transport_id=args.get("transport_id") if isinstance(args.get("transport_id"), str) else None,
            transport_label=args.get("transport_label") if isinstance(args.get("transport_label"), str) else None,
            attachments_root=resolve_attachments_root(data_root),
        )
        conn.commit()
        progress("complete", {"direction": "pull", "applied_events": result.applied_events})
        payload = asdict(result)
        payload["peers"] = list(result.peers)
        return payload
    if kind == "ui.sync.invite":
        join_request = args.get("join_request")
        if not isinstance(join_request, Mapping):
            raise AppError("join_request must be an object", code="validation")
        invitation = create_invitation(
            conn,
            profile_id=profile_id,
            join_request=join_request,
            role=_required_text(args, "role"),
        )
        conn.commit()
        return {
            "invitation": base64.b64encode(invitation).decode("ascii"),
            "role": args["role"],
            "sealed": True,
        }
    if kind == "ui.sync.members.list":
        return {"members": list_members(conn, profile_id=profile_id)}
    if kind == "ui.sync.members.revoke":
        result = revoke_member(
            conn,
            profile_id=profile_id,
            member_id=_required_text(args, "member_id"),
        )
        conn.commit()
        return result
    if kind == "ui.sync.devices.list":
        return {"devices": list_devices(conn, profile_id=profile_id)}
    if kind == "ui.sync.devices.revoke":
        result = revoke_device(
            conn,
            profile_id=profile_id,
            device_id=_required_text(args, "device_id"),
        )
        conn.commit()
        return result
    if kind == "ui.sync.conflicts.list":
        return {
            "conflicts": list_conflicts(
                conn,
                profile_id=profile_id,
                include_resolved=bool(args.get("include_resolved", False)),
            )
        }
    if kind == "ui.sync.conflicts.resolve":
        result = resolve_conflict(
            conn,
            profile_id=profile_id,
            conflict_id=_required_text(args, "conflict_id"),
            source_event_id=(
                args.get("source_event_id")
                if isinstance(args.get("source_event_id"), str)
                else None
            ),
            custom_value=args.get("custom_value"),
            use_custom_value=bool(args.get("use_custom_value", False)),
        )
        conn.commit()
        return result
    raise AppError("sync UI kind is unsupported", code="unsupported_kind", details={"kind": kind})


SYNC_UI_KINDS = (
    "ui.sync.status",
    "ui.sync.enable",
    "ui.sync.disable",
    "ui.sync.transports.list",
    "ui.sync.transports.configure",
    "ui.sync.transports.delete",
    "ui.sync.push",
    "ui.sync.pull",
    "ui.sync.join_request",
    "ui.sync.invite",
    "ui.sync.join",
    "ui.sync.members.list",
    "ui.sync.members.revoke",
    "ui.sync.devices.list",
    "ui.sync.devices.revoke",
    "ui.sync.conflicts.list",
    "ui.sync.conflicts.resolve",
)
