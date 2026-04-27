from __future__ import annotations

import json
import sqlite3
import sys
import threading
import traceback
from dataclasses import dataclass
from typing import Any, TextIO

from . import __version__
from .ai import (
    create_db_ai_provider,
    delete_db_ai_provider,
    get_db_ai_provider,
    redact_ai_provider_for_output,
    resolve_ai_provider,
    set_default_ai_provider,
    clear_default_ai_provider,
    update_db_ai_provider,
)
from .ai.client import OpenAICompatClient
from .ai.providers import (
    acknowledge_remote_use,
    get_default_ai_provider_name,
    list_with_default as list_ai_providers_with_default,
)
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


class _OutputChannel:
    """Thread-safe writer for daemon JSONL output.

    The main loop and any in-flight AI thread share this writer; the lock
    serializes whole JSON lines so concurrent producers don't interleave
    bytes mid-line.
    """

    def __init__(self, stream: TextIO) -> None:
        self._stream = stream
        self._lock = threading.Lock()

    def write(self, payload: dict[str, Any]) -> None:
        line = json.dumps(json_ready(payload), sort_keys=False, separators=(",", ":"))
        with self._lock:
            self._stream.write(line)
            self._stream.write("\n")
            self._stream.flush()


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


def _coerce_args_dict(request_id: object, args: object) -> dict[str, Any]:
    if args is None:
        return {}
    if isinstance(args, dict):
        return args
    raise AppError(
        "daemon args must be an object",
        code="validation",
        details={"type": type(args).__name__},
        retryable=False,
    )


def _ai_chat_args(args: dict) -> dict[str, Any]:
    model = args.get("model")
    if not isinstance(model, str) or not model.strip():
        raise AppError(
            "ai.chat requires a non-empty model",
            code="validation",
            hint="Pass {model: '<id>', messages: [...]}.",
        )
    messages = args.get("messages")
    if not isinstance(messages, list) or not messages:
        raise AppError(
            "ai.chat requires a non-empty messages array",
            code="validation",
        )
    cleaned: list[dict] = []
    for index, raw in enumerate(messages):
        if not isinstance(raw, dict):
            raise AppError(
                f"ai.chat messages[{index}] must be an object",
                code="validation",
            )
        role = raw.get("role")
        content = raw.get("content")
        if role not in ("system", "user", "assistant", "tool"):
            raise AppError(
                f"ai.chat messages[{index}].role must be system | user | assistant | tool",
                code="validation",
            )
        if not isinstance(content, str):
            raise AppError(
                f"ai.chat messages[{index}].content must be a string",
                code="validation",
            )
        cleaned.append({"role": role, "content": content})
    options = args.get("options")
    if options is not None and not isinstance(options, dict):
        raise AppError(
            "ai.chat options must be an object",
            code="validation",
        )
    provider = args.get("provider")
    if provider is not None and not isinstance(provider, str):
        raise AppError(
            "ai.chat provider must be a string",
            code="validation",
        )
    return {
        "provider": provider,
        "model": model.strip(),
        "messages": cleaned,
        "options": options or {},
    }


def _run_ai_chat_stream(
    request_id: object,
    provider_snapshot: dict[str, Any],
    validated: dict[str, Any],
    out: _OutputChannel,
) -> None:
    """Thread target — streams `ai.chat.delta` records and a terminal `ai.chat`.

    Receives an already-resolved provider snapshot so SQLite is never
    touched from the worker thread (sqlite3 connections are tied to the
    thread that opened them).
    """
    try:
        client = OpenAICompatClient(
            base_url=provider_snapshot["base_url"],
            api_key=provider_snapshot.get("api_key"),
        )
        finish_reason = None
        for chunk in client.stream_chat(
            messages=validated["messages"],
            model=validated["model"],
            options=validated["options"],
        ):
            delta_payload = {"delta": chunk.delta}
            if chunk.finish_reason is not None:
                finish_reason = chunk.finish_reason
            out.write(
                _with_request_id(
                    build_envelope("ai.chat.delta", delta_payload),
                    request_id,
                )
            )
        out.write(
            _with_request_id(
                build_envelope(
                    "ai.chat",
                    {
                        "provider": provider_snapshot["name"],
                        "model": validated["model"],
                        "finish_reason": finish_reason,
                    },
                ),
                request_id,
            )
        )
    except AppError as exc:
        out.write(
            _error_envelope(
                exc.code or "app_error",
                str(exc),
                request_id=request_id,
                details=exc.details,
                hint=exc.hint,
                retryable=exc.retryable,
            )
        )
    except Exception as exc:
        traceback.print_exc(file=sys.stderr)
        sys.stderr.flush()
        out.write(
            _error_envelope(
                "internal_error",
                str(exc) or exc.__class__.__name__,
                request_id=request_id,
                retryable=False,
            )
        )


def _ai_provider_redacted(ctx: DaemonContext, provider: dict) -> dict:
    return redact_ai_provider_for_output(
        provider,
        default_name=get_default_ai_provider_name(ctx.conn),
    )


def handle_request(
    ctx: DaemonContext,
    request: dict[str, Any],
    out: _OutputChannel,
) -> tuple[dict[str, Any] | None, bool]:
    """Handle a single daemon request.

    Returns ``(envelope, should_shutdown)``. ``envelope = None`` means the
    handler took responsibility for writing its own response (e.g. a
    streaming AI chat that runs in a thread and emits its own envelopes).
    """
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

    if kind == "ai.providers.list":
        return (
            _with_request_id(
                build_envelope("ai.providers.list", list_ai_providers_with_default(ctx.conn)),
                request_id,
            ),
            False,
        )

    if kind == "ai.providers.create":
        args = _coerce_args_dict(request_id, request.get("args"))
        name = args.get("name")
        base_url = args.get("base_url")
        if not isinstance(name, str) or not isinstance(base_url, str):
            raise AppError(
                "ai.providers.create requires name and base_url strings",
                code="validation",
            )
        created = create_db_ai_provider(
            ctx.conn,
            name,
            base_url,
            api_key=args.get("api_key"),
            default_model=args.get("default_model"),
            kind=str(args.get("kind") or "local"),
            notes=args.get("notes"),
            acknowledged=bool(args.get("acknowledged")),
        )
        return (
            _with_request_id(
                build_envelope("ai.providers.create", _ai_provider_redacted(ctx, created)),
                request_id,
            ),
            False,
        )

    if kind == "ai.providers.update":
        args = _coerce_args_dict(request_id, request.get("args"))
        name = args.get("name")
        if not isinstance(name, str):
            raise AppError("ai.providers.update requires a name string", code="validation")
        clear_raw = args.get("clear")
        if clear_raw is None:
            clear_list: list[str] = []
        elif isinstance(clear_raw, list):
            clear_list = [str(item) for item in clear_raw]
        else:
            raise AppError("ai.providers.update clear must be a list", code="validation")
        updated = update_db_ai_provider(
            ctx.conn,
            name,
            {
                "base_url": args.get("base_url"),
                "api_key": args.get("api_key"),
                "default_model": args.get("default_model"),
                "kind": args.get("kind"),
                "notes": args.get("notes"),
                "clear": clear_list,
                "acknowledged": bool(args.get("acknowledged")),
                "acknowledge_clear": bool(args.get("acknowledge_clear")),
            },
        )
        return (
            _with_request_id(
                build_envelope("ai.providers.update", _ai_provider_redacted(ctx, updated)),
                request_id,
            ),
            False,
        )

    if kind == "ai.providers.delete":
        args = _coerce_args_dict(request_id, request.get("args"))
        name = args.get("name")
        if not isinstance(name, str):
            raise AppError("ai.providers.delete requires a name string", code="validation")
        return (
            _with_request_id(
                build_envelope("ai.providers.delete", delete_db_ai_provider(ctx.conn, name)),
                request_id,
            ),
            False,
        )

    if kind == "ai.providers.set_default":
        args = _coerce_args_dict(request_id, request.get("args"))
        name = args.get("name")
        if not isinstance(name, str):
            raise AppError("ai.providers.set_default requires a name string", code="validation")
        return (
            _with_request_id(
                build_envelope("ai.providers.set_default", set_default_ai_provider(ctx.conn, name)),
                request_id,
            ),
            False,
        )

    if kind == "ai.providers.clear_default":
        return (
            _with_request_id(
                build_envelope("ai.providers.clear_default", clear_default_ai_provider(ctx.conn)),
                request_id,
            ),
            False,
        )

    if kind == "ai.providers.get":
        args = _coerce_args_dict(request_id, request.get("args"))
        name = args.get("name")
        if not isinstance(name, str):
            raise AppError("ai.providers.get requires a name string", code="validation")
        return (
            _with_request_id(
                build_envelope("ai.providers.get", _ai_provider_redacted(ctx, get_db_ai_provider(ctx.conn, name))),
                request_id,
            ),
            False,
        )

    if kind == "ai.providers.acknowledge":
        args = _coerce_args_dict(request_id, request.get("args"))
        name = args.get("name")
        if not isinstance(name, str):
            raise AppError("ai.providers.acknowledge requires a name string", code="validation")
        return (
            _with_request_id(
                build_envelope("ai.providers.acknowledge", acknowledge_remote_use(ctx.conn, name)),
                request_id,
            ),
            False,
        )

    if kind == "ai.list_models":
        args = _coerce_args_dict(request_id, request.get("args"))
        provider_name = args.get("provider")
        if provider_name is not None and not isinstance(provider_name, str):
            raise AppError("ai.list_models provider must be a string", code="validation")
        provider = resolve_ai_provider(ctx.conn, provider_name)
        client = OpenAICompatClient(
            base_url=provider["base_url"],
            api_key=provider.get("api_key"),
        )
        return (
            _with_request_id(
                build_envelope(
                    "ai.list_models",
                    {
                        "provider": provider["name"],
                        "models": client.list_models(),
                    },
                ),
                request_id,
            ),
            False,
        )

    if kind == "ai.chat":
        # Validate eagerly so syntax errors surface synchronously.
        validated = _ai_chat_args(_coerce_args_dict(request_id, request.get("args")))
        # Resolve the provider + record acknowledgement on the main thread —
        # the worker thread never touches SQLite (sqlite3 connections are
        # bound to the thread that opened them).
        provider = resolve_ai_provider(ctx.conn, validated["provider"])
        if provider["kind"] != "local" and not provider.get("acknowledged_at"):
            acknowledge_remote_use(ctx.conn, provider["name"])
        provider_snapshot = {
            "name": provider["name"],
            "base_url": provider["base_url"],
            "api_key": provider.get("api_key"),
            "kind": provider["kind"],
        }
        thread = threading.Thread(
            target=_run_ai_chat_stream,
            args=(request_id, provider_snapshot, validated, out),
            daemon=True,
            name="kassiber-ai-chat",
        )
        thread.start()
        return (None, False)

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
    out = _OutputChannel(output_stream)
    ctx = DaemonContext(
        conn=conn,
        data_root=args.data_root,
        runtime_config=args.runtime_config,
    )

    out.write(
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
                    "ai.providers.list",
                    "ai.providers.get",
                    "ai.providers.create",
                    "ai.providers.update",
                    "ai.providers.delete",
                    "ai.providers.set_default",
                    "ai.providers.clear_default",
                    "ai.providers.acknowledge",
                    "ai.list_models",
                    "ai.chat",
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
            out.write(
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
            out.write(
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
            out.write(
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
            response, should_shutdown = handle_request(ctx, request, out)
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

        if response is not None:
            out.write(response)
        if should_shutdown:
            return 0

    return 0
