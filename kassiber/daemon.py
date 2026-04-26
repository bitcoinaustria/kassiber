from __future__ import annotations

import json
import sqlite3
import sys
import traceback
from dataclasses import dataclass
from typing import Any, TextIO

from . import __version__
from .cli.handlers import sync_wallet
from .core.repo import current_context_snapshot
from .core.runtime import build_status_payload
from .core.ui_snapshot import (
    build_capital_gains_snapshot,
    build_journals_snapshot,
    build_overview_snapshot,
    build_profiles_snapshot,
    build_transactions_snapshot,
)
from .envelope import SCHEMA_VERSION, build_envelope, build_error_envelope, json_ready
from .errors import AppError


MAX_REQUEST_LINE_CHARS = 1_000_000
_REQUEST_ID_MISSING = object()


@dataclass(frozen=True)
class DaemonContext:
    conn: sqlite3.Connection
    data_root: str
    runtime_config: dict[str, object]


def _write_jsonl(stream: TextIO, payload: dict[str, Any]) -> None:
    stream.write(json.dumps(json_ready(payload), sort_keys=False, separators=(",", ":")))
    stream.write("\n")
    stream.flush()


def _with_request_id(
    envelope: dict[str, Any],
    request_id: object = _REQUEST_ID_MISSING,
) -> dict[str, Any]:
    if request_id is not _REQUEST_ID_MISSING:
        envelope["request_id"] = request_id
    return envelope


def _error_envelope(
    code: str,
    message: str,
    *,
    request_id: object = _REQUEST_ID_MISSING,
    details: Any = None,
    hint: str | None = None,
    retryable: bool = False,
) -> dict[str, Any]:
    return _with_request_id(
        build_error_envelope(
            code,
            message,
            details=details,
            hint=hint,
            retryable=retryable,
        ),
        request_id,
    )


def _status_payload(ctx: DaemonContext) -> dict[str, Any]:
    payload = build_status_payload(ctx.conn, ctx.data_root)
    payload["default_backend"] = ctx.runtime_config["default_backend"]
    payload["env_file"] = ctx.runtime_config["env_file"]
    return payload


def handle_request(ctx: DaemonContext, request: dict[str, Any]) -> tuple[dict[str, Any], bool]:
    request_id = request.get("request_id", _REQUEST_ID_MISSING)
    kind = request.get("kind")
    if not isinstance(kind, str) or not kind:
        return (
            _error_envelope(
                "validation",
                "daemon request requires a non-empty string kind",
                request_id=request_id,
                details={"keys": sorted(str(key) for key in request)},
                retryable=False,
            ),
            False,
        )

    if kind == "daemon.shutdown":
        return (
            _with_request_id(
                build_envelope("daemon.shutdown", {}),
                request_id,
            ),
            True,
        )

    if kind == "cancel":
        return (
            _error_envelope(
                "unsupported_kind",
                "daemon cancellation is not wired yet",
                request_id=request_id,
                hint="Cancellation lands with worker-pool execution.",
                retryable=False,
            ),
            False,
        )

    if kind == "status":
        return (
            _with_request_id(
                build_envelope("status", _status_payload(ctx)),
                request_id,
            ),
            False,
        )

    if kind == "ui.overview.snapshot":
        return (
            _with_request_id(
                build_envelope("ui.overview.snapshot", build_overview_snapshot(ctx.conn)),
                request_id,
            ),
            False,
        )

    if kind == "ui.transactions.list":
        return (
            _with_request_id(
                build_envelope(
                    "ui.transactions.list",
                    build_transactions_snapshot(ctx.conn, request.get("args")),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.reports.capital_gains":
        return (
            _with_request_id(
                build_envelope(
                    "ui.reports.capital_gains",
                    build_capital_gains_snapshot(ctx.conn),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.journals.snapshot":
        return (
            _with_request_id(
                build_envelope(
                    "ui.journals.snapshot",
                    build_journals_snapshot(ctx.conn),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.profiles.snapshot":
        return (
            _with_request_id(
                build_envelope(
                    "ui.profiles.snapshot",
                    build_profiles_snapshot(ctx.conn),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.wallets.sync":
        args = request.get("args")
        if args is not None and not isinstance(args, dict):
            return (
                _error_envelope(
                    "validation",
                    "ui.wallets.sync args must be an object",
                    request_id=request_id,
                    details={"type": type(args).__name__},
                    retryable=False,
                ),
                False,
            )
        raw_args = args or {}
        wallet = raw_args.get("wallet")
        sync_all = bool(raw_args.get("all", wallet is None))
        if wallet is not None and not isinstance(wallet, str):
            return (
                _error_envelope(
                    "validation",
                    "ui.wallets.sync wallet must be a string",
                    request_id=request_id,
                    details={"type": type(wallet).__name__},
                    retryable=False,
                ),
                False,
            )
        context = current_context_snapshot(ctx.conn)
        if not context["workspace_id"] or not context["profile_id"]:
            return (
                _with_request_id(
                    build_envelope("ui.wallets.sync", {"results": []}),
                    request_id,
                ),
                False,
            )
        return (
            _with_request_id(
                build_envelope(
                    "ui.wallets.sync",
                    {
                        "results": sync_wallet(
                            ctx.conn,
                            ctx.runtime_config,
                            None,
                            None,
                            wallet_ref=wallet,
                            sync_all=sync_all,
                        )
                    },
                ),
                request_id,
            ),
            False,
        )

    if kind.startswith("ui."):
        return (
            _error_envelope(
                "daemon_unavailable",
                f"daemon kind {kind!r} is not wired to real UI data yet",
                request_id=request_id,
                details={"kind": kind},
                hint="Use VITE_DAEMON=mock for dashboard fixture development until typed UI snapshot kinds land.",
                retryable=True,
            ),
            False,
        )

    return (
        _error_envelope(
            "unsupported_kind",
            f"daemon kind {kind!r} is not supported yet",
            request_id=request_id,
            details={"kind": kind},
            hint="Only status is exposed through the first daemon slice.",
            retryable=False,
        ),
        False,
    )


def run(
    conn: sqlite3.Connection,
    args: Any,
    *,
    stdin: TextIO | None = None,
    stdout: TextIO | None = None,
) -> int:
    input_stream = stdin or sys.stdin
    output_stream = stdout or sys.stdout
    ctx = DaemonContext(
        conn=conn,
        data_root=args.data_root,
        runtime_config=args.runtime_config,
    )

    _write_jsonl(
        output_stream,
        build_envelope(
            "daemon.ready",
            {
                "version": __version__,
                "supported_kinds": [
                    "status",
                    "ui.overview.snapshot",
                    "ui.transactions.list",
                    "ui.reports.capital_gains",
                    "ui.journals.snapshot",
                    "ui.profiles.snapshot",
                    "ui.wallets.sync",
                    "daemon.shutdown",
                ],
            },
        ),
    )

    while True:
        line = input_stream.readline(MAX_REQUEST_LINE_CHARS + 1)
        if line == "":
            break
        if len(line) > MAX_REQUEST_LINE_CHARS:
            while line and not line.endswith("\n"):
                line = input_stream.readline(MAX_REQUEST_LINE_CHARS + 1)
            _write_jsonl(
                output_stream,
                _error_envelope(
                    "request_too_large",
                    "daemon request line is too large",
                    request_id=None,
                    details={"max_chars": MAX_REQUEST_LINE_CHARS},
                    retryable=False,
                ),
            )
            continue
        raw = line.strip()
        if not raw:
            continue
        try:
            request = json.loads(raw)
        except json.JSONDecodeError as exc:
            _write_jsonl(
                output_stream,
                _error_envelope(
                    "invalid_json",
                    "daemon request line is not valid JSON",
                    request_id=None,
                    details={"error": str(exc)},
                    retryable=False,
                ),
            )
            continue
        if not isinstance(request, dict):
            _write_jsonl(
                output_stream,
                _error_envelope(
                    "validation",
                    "daemon request must be a JSON object",
                    request_id=None,
                    details={"type": type(request).__name__},
                    retryable=False,
                ),
            )
            continue

        try:
            response, should_shutdown = handle_request(ctx, request)
        except AppError as exc:
            response = _error_envelope(
                exc.code or "app_error",
                str(exc),
                request_id=request.get("request_id"),
                details=exc.details,
                hint=exc.hint,
                retryable=exc.retryable,
            )
            should_shutdown = False
        except Exception as exc:
            traceback.print_exc(file=sys.stderr)
            sys.stderr.flush()
            response = _error_envelope(
                "internal_error",
                str(exc) or exc.__class__.__name__,
                request_id=request.get("request_id"),
                retryable=False,
            )
            should_shutdown = False

        _write_jsonl(output_stream, response)
        if should_shutdown:
            return 0

    return 0
