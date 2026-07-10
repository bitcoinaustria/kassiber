"""CLI parser and dispatch for transport-independent replication commands."""

from __future__ import annotations

import argparse
from dataclasses import asdict
import json
import os
from pathlib import Path
import sys
import tempfile

from ...db import resolve_attachments_root
from ...errors import AppError
from ...secrets.cli_input import (
    add_secret_stdin_options,
    enforce_single_stdin_consumer,
    read_secret_from_args,
)
from ..repo import resolve_scope
from .bundle import build_bundle, write_bundle_atomic
from .conflicts import list_conflicts, resolve_conflict
from .identity import disable_sync, enable_sync, sync_status
from .membership import (
    create_invitation,
    create_join_request,
    join_invitation,
    list_devices,
    list_members,
    revoke_device,
    revoke_member,
)
from .merge import import_bundle
from .lan import LanSyncServer, connect_lan, discover_lan_services
from .tor import TorOnionSyncServer, connect_onion
from .gc import compact_tombstones
from .mailbox import mailbox_status, pull_mailbox, pull_result_dict, push_mailbox, push_result_dict
from .transports import configure_transport, delete_transport


def _scope_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--workspace")
    parser.add_argument("--profile")


def add_sync_parser(subparsers) -> None:
    sync = subparsers.add_parser(
        "sync",
        help="Secure authored-event replication; never syncs the live database file",
    )
    commands = sync.add_subparsers(dest="sync_command", required=True)
    status = commands.add_parser("status")
    _scope_args(status)

    enable = commands.add_parser("enable")
    _scope_args(enable)
    enable.add_argument("--member-name", required=True)
    enable.add_argument("--device-label", required=True)

    disable = commands.add_parser("disable")
    _scope_args(disable)

    transport = commands.add_parser("transport")
    transport_commands = transport.add_subparsers(dest="sync_transport_command", required=True)
    transport_list = transport_commands.add_parser("list")
    _scope_args(transport_list)
    transport_add = transport_commands.add_parser("add")
    _scope_args(transport_add)
    transport_add.add_argument("--kind", required=True, choices=("folder", "webdav", "s3"))
    transport_add.add_argument("--label", required=True)
    transport_add.add_argument("--path")
    transport_add.add_argument("--url")
    transport_add.add_argument("--endpoint")
    transport_add.add_argument("--bucket")
    transport_add.add_argument("--region")
    transport_add.add_argument("--prefix")
    transport_add.add_argument("--username")
    transport_add.add_argument("--access-key")
    add_secret_stdin_options(transport_add, "password", label="WebDAV password")
    add_secret_stdin_options(transport_add, "secret-key", label="S3 secret key")
    add_secret_stdin_options(transport_add, "session-token", label="S3 session token")
    transport_remove = transport_commands.add_parser("remove")
    _scope_args(transport_remove)
    transport_remove.add_argument("transport_id")

    lan = commands.add_parser("lan")
    lan_commands = lan.add_subparsers(dest="sync_lan_command", required=True)
    lan_listen = lan_commands.add_parser("listen")
    _scope_args(lan_listen)
    lan_listen.add_argument("--offer", required=True, help="Write the short-lived pairing offer here")
    lan_listen.add_argument("--bind-host")
    lan_listen.add_argument("--advertise-host")
    lan_listen.add_argument("--no-mdns", action="store_true")
    lan_listen.add_argument("--timeout", type=float, default=120.0)
    lan_connect = lan_commands.add_parser("connect")
    _scope_args(lan_connect)
    lan_connect.add_argument("--offer", required=True, help="Pairing offer file from the listening peer")
    lan_connect.add_argument("--timeout", type=float, default=30.0)
    lan_discover = lan_commands.add_parser("discover")
    _scope_args(lan_discover)
    lan_discover.add_argument("--timeout", type=float, default=1.5)

    tor = commands.add_parser("tor")
    tor_commands = tor.add_subparsers(dest="sync_tor_command", required=True)
    tor_listen = tor_commands.add_parser("listen")
    _scope_args(tor_listen)
    tor_listen.add_argument("--onion-host", required=True)
    tor_listen.add_argument("--onion-port", type=int, required=True)
    tor_listen.add_argument("--local-port", type=int, required=True)
    tor_listen.add_argument("--offer", required=True)
    tor_listen.add_argument("--timeout", type=float, default=300.0)
    tor_connect = tor_commands.add_parser("connect")
    _scope_args(tor_connect)
    tor_connect.add_argument("--offer", required=True)
    tor_connect.add_argument("--proxy", help="Deprecated argv proxy shim; prefer --tor-proxy-stdin/fd")
    add_secret_stdin_options(tor_connect, "tor-proxy", label="Tor SOCKS proxy URL")
    tor_connect.add_argument("--timeout", type=float, default=60.0)

    gc = commands.add_parser("gc")
    gc_commands = gc.add_subparsers(dest="sync_gc_command", required=True)
    gc_status = gc_commands.add_parser("status")
    _scope_args(gc_status)
    gc_status.add_argument("--horizon-days", type=int, default=180)
    gc_run = gc_commands.add_parser("run")
    _scope_args(gc_run)
    gc_run.add_argument("--horizon-days", type=int, default=180)
    gc_run.add_argument("--apply", action="store_true", help="Compact eligible tombstones; default is dry-run")

    join_request = commands.add_parser("join-request")
    join_request.add_argument("--member-name", required=True)
    join_request.add_argument("--device-label", required=True)

    invite = commands.add_parser("invite")
    _scope_args(invite)
    invite.add_argument("--request", required=True, help="Signed join-request JSON file")
    invite.add_argument("--role", required=True, choices=("owner", "editor", "auditor"))
    invite.add_argument("--invitation", required=True, help="Output path for the sealed invitation")

    join = commands.add_parser("join")
    join.add_argument("--request-id", required=True)
    join.add_argument("--invitation", required=True)

    push = commands.add_parser("push")
    _scope_args(push)
    push_target = push.add_mutually_exclusive_group(required=True)
    push_target.add_argument("--bundle", help="Output path for a sealed courier bundle")
    push_target.add_argument("--transport", help="Configured mailbox transport label")
    push.add_argument(
        "--snapshot",
        action="store_true",
        help="Owner-attested full checkpoint for newly invited devices",
    )

    pull = commands.add_parser("pull")
    _scope_args(pull)
    pull_source = pull.add_mutually_exclusive_group(required=True)
    pull_source.add_argument("--bundle", help="Sealed courier bundle file to replay")
    pull_source.add_argument("--transport", help="Configured mailbox transport label")

    members = commands.add_parser("members")
    members_commands = members.add_subparsers(dest="sync_members_command", required=True)
    members_list = members_commands.add_parser("list")
    _scope_args(members_list)
    members_revoke = members_commands.add_parser("revoke")
    _scope_args(members_revoke)
    members_revoke.add_argument("member_id")

    devices = commands.add_parser("devices")
    devices_commands = devices.add_subparsers(dest="sync_devices_command", required=True)
    devices_list = devices_commands.add_parser("list")
    _scope_args(devices_list)
    devices_revoke = devices_commands.add_parser("revoke")
    _scope_args(devices_revoke)
    devices_revoke.add_argument("device_id")

    conflicts = commands.add_parser("conflicts")
    conflict_commands = conflicts.add_subparsers(dest="sync_conflicts_command", required=True)
    conflicts_list = conflict_commands.add_parser("list")
    _scope_args(conflicts_list)
    conflicts_list.add_argument("--include-resolved", action="store_true")
    conflicts_resolve = conflict_commands.add_parser("resolve")
    _scope_args(conflicts_resolve)
    conflicts_resolve.add_argument("conflict_id")
    choice = conflicts_resolve.add_mutually_exclusive_group(required=True)
    choice.add_argument("--source-event-id")
    choice.add_argument("--value-json")


def _scope(conn, args):
    return resolve_scope(conn, getattr(args, "workspace", None), getattr(args, "profile", None))


def _write_text_atomic(path: Path, value: str) -> Path:
    output = path.expanduser().resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary_name = tempfile.mkstemp(prefix=f".{output.name}.", dir=output.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(value)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, output)
    except Exception:
        temporary.unlink(missing_ok=True)
        raise
    return output


def dispatch_sync(conn, args):
    command = args.sync_command
    if command == "join-request":
        result = create_join_request(
            conn,
            member_name=args.member_name,
            device_label=args.device_label,
        )
        conn.commit()
        return result
    if command == "join":
        ciphertext = Path(args.invitation).expanduser().read_bytes()
        result = join_invitation(
            conn,
            request_id=args.request_id,
            ciphertext=ciphertext,
        )
        conn.commit()
        return result

    workspace, profile = _scope(conn, args)
    profile_id = profile["id"]
    if command == "status":
        return sync_status(conn, profile_id=profile_id)
    if command == "enable":
        result = enable_sync(
            conn,
            workspace_id=workspace["id"],
            profile_id=profile_id,
            member_name=args.member_name,
            device_label=args.device_label,
        )
        conn.commit()
        return result
    if command == "disable":
        result = disable_sync(conn, profile_id=profile_id)
        conn.commit()
        return result
    if command == "lan":
        if args.sync_lan_command == "discover":
            return {"services": discover_lan_services(timeout_seconds=args.timeout)}
        if args.sync_lan_command == "listen":
            server = LanSyncServer(
                conn,
                profile_id=profile_id,
                bind_host=args.bind_host,
                advertise_host=args.advertise_host,
                advertise_mdns=not args.no_mdns,
            )
            offer_path = _write_text_atomic(Path(args.offer), server.offer.encode())
            sys.stderr.write(f"LAN pairing offer written to {offer_path}; waiting for one peer.\n")
            sys.stderr.flush()
            result = server.serve_once(
                conn,
                attachments_root=resolve_attachments_root(args.data_root),
                timeout_seconds=args.timeout,
            )
            conn.commit()
            return asdict(result) | {"offer": str(offer_path), "mdns": not args.no_mdns}
        offer_path = Path(args.offer).expanduser().resolve()
        if not offer_path.is_file():
            raise AppError("LAN pairing offer file was not found", code="not_found")
        result = connect_lan(
            conn,
            profile_id=profile_id,
            offer_code=offer_path.read_text(encoding="utf-8").strip(),
            attachments_root=resolve_attachments_root(args.data_root),
            timeout_seconds=args.timeout,
        )
        conn.commit()
        return asdict(result) | {"offer": str(offer_path)}
    if command == "tor":
        if args.sync_tor_command == "listen":
            server = TorOnionSyncServer(
                conn,
                profile_id=profile_id,
                onion_host=args.onion_host,
                onion_port=args.onion_port,
                local_port=args.local_port,
            )
            offer_path = _write_text_atomic(Path(args.offer), server.offer.encode())
            sys.stderr.write(
                f"Tor pairing offer written to {offer_path}; waiting on loopback port {args.local_port}.\n"
            )
            sys.stderr.flush()
            result = server.serve_once(
                conn,
                attachments_root=resolve_attachments_root(args.data_root),
                timeout_seconds=args.timeout,
            )
            conn.commit()
            return asdict(result) | {"offer": str(offer_path), "transport": "tor-onion"}
        offer_path = Path(args.offer).expanduser().resolve()
        if not offer_path.is_file():
            raise AppError("Tor pairing offer file was not found", code="not_found")
        proxy = read_secret_from_args(
            args,
            "tor-proxy",
            legacy_attr="proxy",
            label="Tor SOCKS proxy URL",
        )
        result = connect_onion(
            conn,
            profile_id=profile_id,
            offer_code=offer_path.read_text(encoding="utf-8").strip(),
            proxy_url=proxy or "",
            attachments_root=resolve_attachments_root(args.data_root),
            timeout_seconds=args.timeout,
        )
        conn.commit()
        return asdict(result) | {"offer": str(offer_path), "transport": "tor-onion"}
    if command == "gc":
        result = compact_tombstones(
            conn,
            profile_id=profile_id,
            horizon_days=args.horizon_days,
            dry_run=(args.sync_gc_command == "status" or not args.apply),
        )
        if not result["dry_run"]:
            conn.commit()
        return result
    if command == "transport":
        if args.sync_transport_command == "list":
            return mailbox_status(conn, profile_id=profile_id)
        if args.sync_transport_command == "remove":
            result = delete_transport(
                conn,
                profile_id=profile_id,
                transport_id=args.transport_id,
            )
            conn.commit()
            return result
        enforce_single_stdin_consumer(args, ("password", "secret_key", "session_token"))
        config = {
            key: value
            for key, value in {
                "path": args.path,
                "url": args.url,
                "endpoint": args.endpoint,
                "bucket": args.bucket,
                "region": args.region,
                "prefix": args.prefix,
            }.items()
            if value is not None
        }
        credentials = {
            key: value
            for key, value in {
                "username": args.username,
                "password": read_secret_from_args(args, "password"),
                "access_key": args.access_key,
                "secret_key": read_secret_from_args(args, "secret-key"),
                "session_token": read_secret_from_args(args, "session-token"),
            }.items()
            if value is not None
        }
        result = configure_transport(
            conn,
            profile_id=profile_id,
            kind=args.kind,
            label=args.label,
            config=config,
            credentials=credentials,
        )
        conn.commit()
        return result
    if command == "invite":
        try:
            request = json.loads(Path(args.request).expanduser().read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise AppError("join request file is invalid", code="sync_join_request_invalid") from exc
        if isinstance(request, dict) and request.get("kind") == "sync.join-request":
            request = request.get("data")
        if not isinstance(request, dict):
            raise AppError("join request file is invalid", code="sync_join_request_invalid")
        invitation = create_invitation(
            conn,
            profile_id=profile_id,
            join_request=request,
            role=args.role,
        )
        output = Path(args.invitation).expanduser().resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(invitation)
        output.chmod(0o600)
        conn.commit()
        return {"invitation": str(output), "role": args.role, "sealed": True}
    if command == "push":
        if args.transport:
            result = push_mailbox(
                conn,
                profile_id=profile_id,
                transport_label=args.transport,
                attachments_root=resolve_attachments_root(args.data_root),
                snapshot=args.snapshot,
            )
            conn.commit()
            return push_result_dict(result)
        result = build_bundle(
            conn,
            profile_id=profile_id,
            attachments_root=resolve_attachments_root(args.data_root),
            snapshot=args.snapshot,
        )
        if result is None:
            return {"bundle": None, "event_count": 0, "up_to_date": True}
        output = write_bundle_atomic(result, Path(args.bundle))
        conn.commit()
        payload = asdict(result)
        payload.pop("ciphertext")
        payload["bundle"] = str(output)
        payload["up_to_date"] = False
        return payload
    if command == "pull":
        if args.transport:
            result = pull_mailbox(
                conn,
                profile_id=profile_id,
                transport_label=args.transport,
                attachments_root=resolve_attachments_root(args.data_root),
            )
            conn.commit()
            return pull_result_dict(result)
        path = Path(args.bundle).expanduser().resolve()
        if not path.is_file():
            raise AppError("sync bundle was not found", code="not_found", details={"bundle": str(path)})
        result = import_bundle(
            conn,
            profile_id=profile_id,
            ciphertext=path.read_bytes(),
            attachments_root=resolve_attachments_root(args.data_root),
        )
        conn.commit()
        return asdict(result) | {"bundle": str(path)}
    if command == "members":
        if args.sync_members_command == "list":
            return list_members(conn, profile_id=profile_id)
        if args.sync_members_command == "revoke":
            result = revoke_member(conn, profile_id=profile_id, member_id=args.member_id)
            conn.commit()
            return result
    if command == "devices":
        if args.sync_devices_command == "list":
            return list_devices(conn, profile_id=profile_id)
        if args.sync_devices_command == "revoke":
            result = revoke_device(conn, profile_id=profile_id, device_id=args.device_id)
            conn.commit()
            return result
    if command == "conflicts":
        if args.sync_conflicts_command == "list":
            return list_conflicts(
                conn,
                profile_id=profile_id,
                include_resolved=args.include_resolved,
            )
        if args.sync_conflicts_command == "resolve":
            custom_value = None
            use_custom = args.value_json is not None
            if use_custom:
                try:
                    custom_value = json.loads(args.value_json)
                except json.JSONDecodeError as exc:
                    raise AppError("--value-json must be valid JSON", code="validation") from exc
            result = resolve_conflict(
                conn,
                profile_id=profile_id,
                conflict_id=args.conflict_id,
                source_event_id=args.source_event_id,
                custom_value=custom_value,
                use_custom_value=use_custom,
            )
            conn.commit()
            return result
    raise AppError("unknown sync command", code="unknown_command", details={"command": command})
