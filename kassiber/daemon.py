from __future__ import annotations

import json
import sqlite3
import sys
from dataclasses import dataclass
from typing import Any, TextIO

from . import __version__
from .core.runtime import build_status_payload
from .envelope import SCHEMA_VERSION, build_envelope, build_error_envelope, json_ready
from .errors import AppError


@dataclass(frozen=True)
class DaemonContext:
    conn: sqlite3.Connection
    data_root: str
    runtime_config: dict[str, object]


def _write_jsonl(stream: TextIO, payload: dict[str, Any]) -> None:
    stream.write(json.dumps(json_ready(payload), sort_keys=False, separators=(",", ":")))
    stream.write("\n")
    stream.flush()


def _with_request_id(envelope: dict[str, Any], request_id: object | None) -> dict[str, Any]:
    if request_id is not None:
        envelope["request_id"] = request_id
    return envelope


def _error_envelope(
    code: str,
    message: str,
    *,
    request_id: object | None = None,
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
    request_id = request.get("request_id")
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
                "supported_kinds": ["status", "daemon.shutdown"],
            },
        ),
    )

    for line in input_stream:
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
