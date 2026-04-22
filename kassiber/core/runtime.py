from __future__ import annotations

import json
import sqlite3
import sys
from dataclasses import dataclass
from pathlib import Path

from .. import __version__
from ..backends import (
    load_runtime_config,
    merge_db_backends,
    resolve_effective_env_file,
    seed_db_backends,
)
from ..db import (
    DEFAULT_DATA_ROOT,
    ensure_data_root,
    ensure_settings_file,
    open_db,
    resolve_attachments_root,
    resolve_config_root,
    resolve_database_path,
    resolve_effective_data_root,
    resolve_effective_state_root,
    resolve_exports_root,
    resolve_settings_path,
)
from ..envelope import SCHEMA_VERSION, _write_text, build_error_envelope
from ..errors import AppError
from .repo import current_context_snapshot


@dataclass(frozen=True)
class RuntimePaths:
    data_root: str
    env_file: str
    state_root: str
    config_root: str
    settings_file: str
    exports_root: str
    attachments_root: str
    database: str


@dataclass
class RuntimeState:
    paths: RuntimePaths
    runtime_config: dict[str, object]
    conn: sqlite3.Connection | None


def resolve_output_format(args):
    if args.machine:
        if args.format is not None and args.format != "json":
            raise AppError(
                f"--machine requires --format json, got --format {args.format}",
                code="invalid_flag_combination",
            )
        return "json"
    return args.format or "table"


def resolve_runtime_paths(data_root=None, env_file=None):
    effective_data_root = str(resolve_effective_data_root(data_root or DEFAULT_DATA_ROOT))
    effective_env_file = str(resolve_effective_env_file(env_file, effective_data_root))
    return RuntimePaths(
        data_root=effective_data_root,
        env_file=effective_env_file,
        state_root=str(resolve_effective_state_root(effective_data_root)),
        config_root=str(resolve_config_root(effective_data_root)),
        settings_file=str(resolve_settings_path(effective_data_root)),
        exports_root=str(resolve_exports_root(effective_data_root)),
        attachments_root=str(resolve_attachments_root(effective_data_root)),
        database=str(resolve_database_path(effective_data_root)),
    )


def ensure_runtime_layout(paths):
    ensure_data_root(paths.data_root)
    ensure_data_root(paths.config_root)
    ensure_data_root(Path(paths.env_file).expanduser().parent)
    ensure_data_root(paths.exports_root)
    ensure_data_root(paths.attachments_root)
    ensure_settings_file(paths.data_root, paths.env_file)
    return paths


def bootstrap_runtime(args, needs_db=True):
    paths = ensure_runtime_layout(
        resolve_runtime_paths(
            getattr(args, "data_root", None),
            getattr(args, "env_file", None),
        )
    )
    args.data_root = paths.data_root
    args.env_file = paths.env_file
    args.runtime_config = load_runtime_config(paths.env_file)

    conn = None
    try:
        if needs_db:
            conn = open_db(paths.data_root)
            seed_db_backends(conn, args.runtime_config)
            merge_db_backends(conn, args.runtime_config)
        return RuntimeState(paths=paths, runtime_config=args.runtime_config, conn=conn)
    except Exception:
        if conn is not None:
            conn.close()
        raise


def close_runtime(runtime):
    if runtime.conn is not None:
        runtime.conn.close()


def emit_error(args, exc, debug_text=None):
    code = getattr(exc, "code", "app_error") or "app_error"
    message = str(exc)
    details = getattr(exc, "details", None)
    hint = getattr(exc, "hint", None)
    retryable = getattr(exc, "retryable", False)
    fmt = getattr(args, "format", None) or "table"
    if fmt == "json":
        envelope = build_error_envelope(
            code,
            message,
            details=details,
            hint=hint,
            retryable=retryable,
            debug=debug_text,
        )
        try:
            _write_text(args, json.dumps(envelope, indent=2, sort_keys=False))
        except Exception:
            print(json.dumps(envelope, indent=2, sort_keys=False), file=sys.stderr)
        return
    print(f"error: {message}", file=sys.stderr)
    if hint:
        print(f"hint: {hint}", file=sys.stderr)


def build_status_payload(conn, data_root):
    context = current_context_snapshot(conn)
    counts = {
        "workspaces": conn.execute("SELECT COUNT(*) AS count FROM workspaces").fetchone()["count"],
        "profiles": conn.execute("SELECT COUNT(*) AS count FROM profiles").fetchone()["count"],
        "accounts": conn.execute("SELECT COUNT(*) AS count FROM accounts").fetchone()["count"],
        "wallets": conn.execute("SELECT COUNT(*) AS count FROM wallets").fetchone()["count"],
        "transactions": conn.execute("SELECT COUNT(*) AS count FROM transactions").fetchone()["count"],
        "journal_entries": conn.execute("SELECT COUNT(*) AS count FROM journal_entries").fetchone()["count"],
        "quarantines": conn.execute("SELECT COUNT(*) AS count FROM journal_quarantines").fetchone()["count"],
    }
    paths = resolve_runtime_paths(data_root=data_root)
    return {
        "version": __version__,
        "schema_version": SCHEMA_VERSION,
        "auth": {"mode": "local", "authenticated": True},
        "state_root": paths.state_root,
        "data_root": paths.data_root,
        "database": paths.database,
        "config_root": paths.config_root,
        "settings_file": paths.settings_file,
        "exports_root": paths.exports_root,
        "attachments_root": paths.attachments_root,
        "current_workspace": context["workspace_label"],
        "current_profile": context["profile_label"],
        **counts,
    }
