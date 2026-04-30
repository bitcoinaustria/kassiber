from __future__ import annotations

import csv
import json
import queue
import sqlite3
import sys
import threading
import time
import traceback
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, TextIO

from . import __version__
from .ai import (
    create_db_ai_provider,
    delete_db_ai_provider,
    get_db_ai_provider,
    redact_ai_provider_for_output,
    require_ai_provider_acknowledged,
    resolve_ai_provider,
    set_default_ai_provider,
    clear_default_ai_provider,
    update_db_ai_provider,
)
from .ai.client import OpenAICompatClient
from .ai.prompt import (
    build_chat_messages,
    build_openai_tools,
    normalize_system_prompt_kind,
)
from .ai.providers import (
    acknowledge_remote_use,
    get_default_ai_provider_name,
    list_with_default as list_ai_providers_with_default,
    normalize_base_url,
)
from .ai.tools import (
    get_tool,
    read_skill_reference,
    redact_tool_arguments,
    summarize_tool_call,
)
from .cli.handlers import _report_hooks, process_journals, sync_wallet
from .core import reports as core_reports
from .core import accounts as core_accounts
from .core import wallets as core_wallets
from .core.repo import current_context_snapshot
from .core.runtime import build_status_payload
from .core.ui_snapshot import (
    build_backends_list_snapshot,
    build_capital_gains_snapshot,
    build_journals_snapshot,
    build_journals_quarantine_snapshot,
    build_journals_transfers_list_snapshot,
    build_next_actions_snapshot,
    build_overview_snapshot,
    build_profiles_snapshot,
    build_rates_summary_snapshot,
    build_transactions_snapshot,
    build_wallets_list_snapshot,
    build_workspace_health_snapshot,
)
from .backends import load_runtime_config, merge_db_backends
from .db import (
    ensure_data_root,
    open_db,
    resolve_config_root,
    resolve_database_path,
    resolve_effective_data_root,
    set_setting,
    resolve_exports_root,
)
from .envelope import build_envelope, build_error_envelope, json_ready
from .errors import AppError
from .secrets.credentials import migrate_dotenv_credentials
from .secrets.migration import create_empty_encrypted_database, migrate_plaintext_to_encrypted
from .secrets.passphrase import change_database_passphrase
from .secrets.sqlcipher import looks_like_plaintext_sqlite, open_encrypted, sqlcipher_available


MAX_REQUEST_LINE_CHARS = 1_000_000
_REQUEST_ID_MISSING = object()
SUPPORTED_KINDS = (
    "status",
    "ui.overview.snapshot",
    "ui.transactions.list",
    "ui.wallets.list",
    "ui.backends.list",
    "ui.reports.capital_gains",
    "ui.reports.export_pdf",
    "ui.reports.export_capital_gains_csv",
    "ui.reports.export_austrian_e1kv_pdf",
    "ui.reports.export_austrian_e1kv_xlsx",
    "ui.journals.snapshot",
    "ui.journals.quarantine",
    "ui.journals.transfers.list",
    "ui.journals.process",
    "ui.profiles.snapshot",
    "ui.profiles.create",
    "ui.profiles.switch",
    "ui.rates.summary",
    "ui.workspace.health",
    "ui.workspace.create",
    "ui.workspace.delete",
    "ui.secrets.init",
    "ui.secrets.change_passphrase",
    "ui.next_actions",
    "ui.wallets.update",
    "ui.wallets.delete",
    "ui.wallets.sync",
    "daemon.lock",
    "daemon.unlock",
    "ai.providers.list",
    "ai.providers.get",
    "ai.providers.create",
    "ai.providers.update",
    "ai.providers.delete",
    "ai.providers.set_default",
    "ai.providers.clear_default",
    "ai.providers.acknowledge",
    "ai.list_models",
    "ai.test_connection",
    "ai.chat",
    "ai.chat.cancel",
    "ai.tool_call.consent",
    "wallets.reveal_descriptor",
    "backends.reveal_token",
    "daemon.shutdown",
)
PENDING_AI_CANCEL_TTL_SECONDS = 30.0
MAX_PENDING_AI_CANCELS = 128
AI_TOOL_CONSENT_TIMEOUT_SECONDS = 300.0
PLAINTEXT_DELETE_ACK = "DELETE LOCAL DATA"
PLAINTEXT_CHANGE_ACK = "CHANGE LOCAL DATA"
MIN_DATABASE_PASSPHRASE_CHARS = 12
AUTH_FAILURES_BEFORE_BACKOFF = 3
AUTH_BACKOFF_BASE_SECONDS = 5.0
AUTH_BACKOFF_MAX_SECONDS = 30.0
AUTH_BACKOFF_FILENAME = "auth_backoff.json"


class AiToolConsentState:
    """Per-chat consent queue for mutating AI tool calls."""

    def __init__(self) -> None:
        self._condition = threading.Condition()
        self._decisions: dict[str, str] = {}
        self._pending_call_ids: set[str] = set()
        self._allow_session: set[str] = set()

    def expect(self, call_id: str) -> None:
        with self._condition:
            self._pending_call_ids.add(call_id)

    def record(self, call_id: str, decision: str) -> bool:
        with self._condition:
            if call_id not in self._pending_call_ids:
                return False
            self._decisions[call_id] = decision
            self._condition.notify_all()
            return True

    def notify_cancelled(self) -> None:
        with self._condition:
            self._condition.notify_all()

    def has_session_allow(self, tool_name: str) -> bool:
        with self._condition:
            return tool_name in self._allow_session

    def wait(
        self,
        *,
        call_id: str,
        tool_name: str,
        cancel_event: threading.Event,
        timeout: float,
    ) -> str:
        deadline = time.monotonic() + timeout
        with self._condition:
            if tool_name in self._allow_session:
                return "allow_session"
            self._pending_call_ids.add(call_id)
            try:
                while True:
                    if cancel_event.is_set():
                        return "cancelled"
                    decision = self._decisions.pop(call_id, None)
                    if decision is not None:
                        if decision == "allow_session":
                            self._allow_session.add(tool_name)
                        return decision
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        return "consent_timeout"
                    self._condition.wait(min(0.25, remaining))
            finally:
                self._pending_call_ids.discard(call_id)
                self._decisions.pop(call_id, None)


@dataclass(frozen=True)
class ActiveAiChat:
    cancel_event: threading.Event
    consent: AiToolConsentState


class ActiveAiChats:
    """In-memory registry for cooperative AI chat controls."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._chats: dict[str, ActiveAiChat] = {}
        self._pending_cancel_deadlines: dict[str, float] = {}

    def register(self, request_id: object) -> tuple[str | None, ActiveAiChat]:
        chat = ActiveAiChat(
            cancel_event=threading.Event(),
            consent=AiToolConsentState(),
        )
        key = _request_id_registry_key(request_id)
        if key is None:
            return None, chat
        now = time.monotonic()
        with self._lock:
            self._prune_pending_locked(now)
            deadline = self._pending_cancel_deadlines.pop(key, None)
            if deadline is not None and deadline >= now:
                chat.cancel_event.set()
            self._chats[key] = chat
        return key, chat

    def unregister(self, key: str | None, chat: ActiveAiChat) -> None:
        if key is None:
            return
        with self._lock:
            if self._chats.get(key) is chat:
                self._chats.pop(key, None)

    def cancel(self, target_request_id: str) -> tuple[bool, bool]:
        now = time.monotonic()
        with self._lock:
            self._prune_pending_locked(now)
            chat = self._chats.get(target_request_id)
            if chat is not None:
                chat.cancel_event.set()
                chat.consent.notify_cancelled()
                return True, False
            self._pending_cancel_deadlines[target_request_id] = (
                now + PENDING_AI_CANCEL_TTL_SECONDS
            )
            self._trim_pending_locked()
            return False, True

    def _prune_pending_locked(self, now: float) -> None:
        expired = [
            key
            for key, deadline in self._pending_cancel_deadlines.items()
            if deadline <= now
        ]
        for key in expired:
            self._pending_cancel_deadlines.pop(key, None)

    def _trim_pending_locked(self) -> None:
        while len(self._pending_cancel_deadlines) > MAX_PENDING_AI_CANCELS:
            oldest = min(
                self._pending_cancel_deadlines.items(),
                key=lambda item: item[1],
            )[0]
            self._pending_cancel_deadlines.pop(oldest, None)

    def record_consent(self, target_request_id: str, call_id: str, decision: str) -> bool:
        with self._lock:
            chat = self._chats.get(target_request_id)
        if chat is None:
            return False
        return chat.consent.record(call_id, decision)


class AuthAttemptBackoff:
    """Database-level throttling for passphrase verification attempts."""

    def __init__(self, state_path: str | None = None) -> None:
        self._lock = threading.Lock()
        self._failures = 0
        self._locked_until = 0.0
        self._state_path = state_path

    def check(self, scope: str) -> None:
        now = time.time()
        with self._lock:
            self._load_locked()
            retry_after = self._locked_until - now
            if retry_after <= 0:
                if self._locked_until:
                    self._locked_until = 0.0
                    self._persist_locked()
                return
        raise AppError(
            "too many failed passphrase attempts",
            code="local_auth_rate_limited",
            details={
                "scope": scope,
                "throttle": "database",
                "retry_after_seconds": max(1, int(retry_after + 0.999)),
            },
            hint="Wait before trying the passphrase again.",
            retryable=True,
        )

    def record_success(self) -> None:
        with self._lock:
            self._failures = 0
            self._locked_until = 0.0
            self._persist_locked()

    def record_failure(self) -> None:
        now = time.time()
        with self._lock:
            self._load_locked()
            self._failures += 1
            if self._failures < AUTH_FAILURES_BEFORE_BACKOFF:
                self._persist_locked()
                return
            delay = min(
                AUTH_BACKOFF_MAX_SECONDS,
                AUTH_BACKOFF_BASE_SECONDS
                * 2 ** (self._failures - AUTH_FAILURES_BEFORE_BACKOFF),
            )
            self._locked_until = now + delay
            self._persist_locked()

    def _load_locked(self) -> None:
        if not self._state_path:
            return
        try:
            with open(self._state_path, encoding="utf-8") as handle:
                payload = json.load(handle)
        except FileNotFoundError:
            return
        except (OSError, json.JSONDecodeError, TypeError, ValueError):
            return
        self._failures = max(0, int(payload.get("failures", 0)))
        self._locked_until = max(0.0, float(payload.get("locked_until", 0.0)))

    def _persist_locked(self) -> None:
        if not self._state_path:
            return
        try:
            if self._failures <= 0 and self._locked_until <= 0:
                try:
                    Path(self._state_path).unlink()
                except FileNotFoundError:
                    pass
                return
            state_path = Path(self._state_path)
            state_path.parent.mkdir(parents=True, exist_ok=True)
            tmp_path = state_path.with_suffix(state_path.suffix + ".tmp")
            tmp_path.write_text(
                json.dumps(
                    {
                        "failures": self._failures,
                        "locked_until": self._locked_until,
                    },
                    sort_keys=True,
                )
                + "\n",
                encoding="utf-8",
            )
            tmp_path.replace(state_path)
        except OSError:
            return


@dataclass(frozen=True)
class _DaemonMainThreadTask:
    callback: Callable[[sqlite3.Connection], Any]
    response: queue.Queue[tuple[bool, Any]]


@dataclass
class DaemonContext:
    conn: sqlite3.Connection | None
    data_root: str
    runtime_config: dict[str, object]
    active_ai_chats: ActiveAiChats
    main_thread_tasks: queue.Queue[_DaemonMainThreadTask]
    auth_backoff: AuthAttemptBackoff


@dataclass(frozen=True)
class AiToolRuntime:
    data_root: str
    runtime_config: dict[str, object]
    main_thread_tasks: queue.Queue[_DaemonMainThreadTask]


@dataclass(frozen=True)
class ParsedAiToolCall:
    call_id: str
    name: str
    arguments: dict[str, Any]
    argument_error: str | None = None


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


def _run_on_daemon_main_thread(
    runtime: AiToolRuntime,
    callback: Callable[[sqlite3.Connection], Any],
) -> Any:
    response: queue.Queue[tuple[bool, Any]] = queue.Queue(maxsize=1)
    runtime.main_thread_tasks.put(
        _DaemonMainThreadTask(callback=callback, response=response)
    )
    ok, payload = response.get()
    if ok:
        return payload
    if isinstance(payload, BaseException):
        raise payload
    raise RuntimeError(str(payload))


def _drain_daemon_main_thread_tasks(ctx: DaemonContext) -> None:
    while True:
        try:
            task = ctx.main_thread_tasks.get_nowait()
        except queue.Empty:
            return
        try:
            if ctx.conn is None:
                raise AppError(
                    "database is locked; unlock the daemon before running AI tools",
                    code="passphrase_required",
                    retryable=False,
                )
            payload = task.callback(ctx.conn)
        except BaseException as exc:
            task.response.put((False, exc))
        else:
            task.response.put((True, payload))


def _start_stdin_reader(input_stream: TextIO) -> queue.Queue[str]:
    lines: queue.Queue[str] = queue.Queue()

    def _reader() -> None:
        while True:
            line = input_stream.readline(MAX_REQUEST_LINE_CHARS + 1)
            lines.put(line)
            if line == "":
                return

    threading.Thread(
        target=_reader,
        daemon=True,
        name="kassiber-daemon-stdin",
    ).start()
    return lines


def _with_request_id(
    envelope: dict[str, Any],
    request_id: object = _REQUEST_ID_MISSING,
) -> dict[str, Any]:
    if request_id is not _REQUEST_ID_MISSING:
        envelope["request_id"] = request_id
    return envelope


def _request_id_registry_key(request_id: object) -> str | None:
    if request_id is _REQUEST_ID_MISSING or request_id is None:
        return None
    if isinstance(request_id, str):
        return request_id or None
    return str(request_id)


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


def _status_payload_from_parts(
    conn: sqlite3.Connection,
    data_root: str,
    runtime_config: dict[str, object],
) -> dict[str, Any]:
    payload = build_status_payload(conn, data_root)
    payload["default_backend"] = runtime_config["default_backend"]
    payload["env_file"] = runtime_config["env_file"]
    return payload


def _status_payload(ctx: DaemonContext) -> dict[str, Any]:
    conn = _require_conn(ctx)
    return _status_payload_from_parts(conn, ctx.data_root, ctx.runtime_config)


def _require_conn(ctx: DaemonContext) -> sqlite3.Connection:
    if ctx.conn is None:
        raise AppError(
            "database is locked; unlock the daemon before accessing workspace data",
            code="passphrase_required",
            hint="Enter the SQLCipher database passphrase to unlock the local daemon session.",
            retryable=False,
        )
    return ctx.conn


def _managed_report_export_path(data_root: str, stem: str, suffix: str) -> Path:
    root = ensure_data_root(resolve_exports_root(data_root) / "reports")
    timestamp = time.strftime("%Y%m%d-%H%M%S", time.gmtime())
    base = f"{stem}-{timestamp}"
    candidate = root / f"{base}{suffix}"
    counter = 2
    while candidate.exists():
        candidate = root / f"{base}-{counter}{suffix}"
        counter += 1
    return candidate


def _write_records_csv(
    file_path: Path,
    rows: list[dict[str, Any]],
    headers: list[str],
) -> dict[str, Any]:
    file_path.parent.mkdir(parents=True, exist_ok=True)
    with file_path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=headers, extrasaction="ignore")
        writer.writeheader()
        for row in rows:
            writer.writerow(
                {header: json_ready(row.get(header)) for header in headers}
            )
    return {
        "file": str(file_path.resolve()),
        "bytes": file_path.stat().st_size,
        "rows": len(rows),
    }


def _ui_report_export_payload(
    ctx: DaemonContext,
    kind: str,
    args: dict[str, Any],
) -> dict[str, Any]:
    conn = _require_conn(ctx)
    hooks = _report_hooks()
    if kind == "ui.reports.export_pdf":
        path = _managed_report_export_path(ctx.data_root, "kassiber-report", ".pdf")
        wallet = args.get("wallet")
        if wallet is not None and not isinstance(wallet, str):
            raise AppError(
                "ui.reports.export_pdf wallet must be a string",
                code="validation",
            )
        payload = dict(
            core_reports.export_pdf_report(
                conn,
                None,
                None,
                path,
                hooks,
                wallet_ref=wallet,
                history_limit=args.get("history_limit", 0),
            )
        )
        payload.update(
            {
                "format": "pdf",
                "scope": "report",
                "filename": Path(payload["file"]).name,
            }
        )
        return payload

    if kind == "ui.reports.export_capital_gains_csv":
        path = _managed_report_export_path(
            ctx.data_root,
            "kassiber-capital-gains",
            ".csv",
        )
        rows = core_reports.report_capital_gains(conn, None, None, hooks)
        payload = _write_records_csv(
            path,
            rows,
            [
                "occurred_at",
                "wallet",
                "transaction_id",
                "entry_type",
                "asset",
                "quantity",
                "quantity_msat",
                "proceeds",
                "cost_basis",
                "gain_loss",
                "description",
                "at_category",
                "at_kennzahl",
            ],
        )
        payload.update(
            {
                "format": "csv",
                "scope": "capital_gains",
                "filename": path.name,
            }
        )
        return payload

    if kind == "ui.reports.export_austrian_e1kv_pdf":
        year = args.get("year")
        path = _managed_report_export_path(
            ctx.data_root,
            f"kassiber-austrian-e1kv-{year}",
            ".pdf",
        )
        payload = dict(
            core_reports.export_austrian_e1kv_pdf_report(
                conn,
                None,
                None,
                path,
                hooks,
                tax_year=year,
            )
        )
        payload.update(
            {
                "format": "pdf",
                "scope": "austrian_e1kv",
                "filename": Path(payload["file"]).name,
            }
        )
        return payload

    if kind == "ui.reports.export_austrian_e1kv_xlsx":
        year = args.get("year")
        path = _managed_report_export_path(
            ctx.data_root,
            f"kassiber-austrian-e1kv-{year}",
            ".xlsx",
        )
        payload = dict(
            core_reports.export_austrian_e1kv_xlsx_report(
                conn,
                None,
                None,
                path,
                hooks,
                tax_year=year,
            )
        )
        payload.update(
            {
                "format": "xlsx",
                "scope": "austrian_e1kv",
                "filename": Path(payload["file"]).name,
            }
        )
        return payload

    raise AppError(
        f"unsupported report export kind {kind}",
        code="unsupported_kind",
    )


def _open_daemon_connection(
    ctx: DaemonContext,
    *,
    passphrase: str | None = None,
) -> sqlite3.Connection:
    if ctx.conn is not None:
        return ctx.conn
    conn = open_db(ctx.data_root, passphrase=passphrase)
    merge_db_backends(conn, ctx.runtime_config)
    ctx.conn = conn
    return conn


def _locked_envelope(scope: str, label: str, request_id: object) -> dict[str, Any]:
    return _with_request_id(
        build_envelope(
            "auth_required",
            {
                "scope": scope,
                "label": label,
            },
        ),
        request_id,
    )


def _passphrase_from_auth(args: dict[str, Any]) -> str | None:
    auth = args.get("auth_response")
    if not isinstance(auth, dict):
        return None
    passphrase = auth.get("passphrase_secret")
    return passphrase if isinstance(passphrase, str) and passphrase else None


def _validate_new_database_passphrase(passphrase: str) -> None:
    if len(passphrase) < MIN_DATABASE_PASSPHRASE_CHARS:
        raise AppError(
            f"database passphrase must be at least {MIN_DATABASE_PASSPHRASE_CHARS} characters long",
            code="invalid_passphrase",
            hint="Pick a long passphrase from a password manager.",
            retryable=False,
        )


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
        cleaned_message: dict[str, Any] = {"role": role, "content": content}
        tool_call_id = raw.get("tool_call_id")
        if role == "tool" and tool_call_id is not None:
            if not isinstance(tool_call_id, str) or not tool_call_id:
                raise AppError(
                    f"ai.chat messages[{index}].tool_call_id must be a string",
                    code="validation",
                )
            cleaned_message["tool_call_id"] = tool_call_id
        tool_calls = raw.get("tool_calls")
        if role == "assistant" and tool_calls is not None:
            if not isinstance(tool_calls, list):
                raise AppError(
                    f"ai.chat messages[{index}].tool_calls must be an array",
                    code="validation",
                )
            cleaned_message["tool_calls"] = [
                tool_call for tool_call in tool_calls if isinstance(tool_call, dict)
            ]
        cleaned.append(cleaned_message)
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
    tools_enabled = args.get("tools_enabled", False)
    if not isinstance(tools_enabled, bool):
        raise AppError(
            "ai.chat tools_enabled must be a boolean",
            code="validation",
        )
    raw_loop_limit = args.get("tool_loop_max_iterations", 8)
    try:
        tool_loop_max_iterations = int(raw_loop_limit)
    except (TypeError, ValueError):
        raise AppError(
            "ai.chat tool_loop_max_iterations must be an integer",
            code="validation",
        ) from None
    if tool_loop_max_iterations < 1 or tool_loop_max_iterations > 32:
        raise AppError(
            "ai.chat tool_loop_max_iterations must be between 1 and 32",
            code="validation",
        )
    system_prompt_kind = normalize_system_prompt_kind(
        args.get("system_prompt_kind"),
        tools_enabled=tools_enabled,
    )
    system_prompt = args.get("system_prompt")
    if system_prompt is not None and not isinstance(system_prompt, str):
        raise AppError(
            "ai.chat system_prompt must be a string",
            code="validation",
        )
    if system_prompt is not None and system_prompt_kind != "raw":
        raise AppError(
            "ai.chat system_prompt is only accepted when system_prompt_kind is raw",
            code="validation",
        )
    return {
        "provider": provider,
        "model": model.strip(),
        "messages": cleaned,
        "options": options or {},
        "tools_enabled": tools_enabled,
        "tool_loop_max_iterations": tool_loop_max_iterations,
        "system_prompt_kind": system_prompt_kind,
        "system_prompt": system_prompt,
    }


def _coerce_wallets_sync_args(raw_args: dict[str, Any], *, strict: bool) -> dict[str, Any]:
    if strict:
        unknown = sorted(set(raw_args) - {"wallet", "all"})
        if unknown:
            raise AppError(
                "ui.wallets.sync received unsupported arguments",
                code="validation",
                details={"unknown": unknown},
                retryable=False,
            )
    wallet = raw_args.get("wallet")
    if wallet is not None:
        if not isinstance(wallet, str) or not wallet.strip():
            raise AppError(
                "ui.wallets.sync wallet must be a non-empty string",
                code="validation",
                details={"type": type(wallet).__name__},
                retryable=False,
            )
        wallet = wallet.strip()
    sync_all_raw = raw_args.get("all")
    if sync_all_raw is not None and not isinstance(sync_all_raw, bool):
        raise AppError(
            "ui.wallets.sync all must be a boolean",
            code="validation",
            details={"type": type(sync_all_raw).__name__},
            retryable=False,
        )
    sync_all = bool(sync_all_raw if sync_all_raw is not None else wallet is None)
    if sync_all and wallet:
        raise AppError(
            "ui.wallets.sync wallet and all are mutually exclusive",
            code="validation",
            retryable=False,
        )
    return {"wallet": wallet, "all": sync_all}


def _wallets_sync_payload(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object],
    raw_args: dict[str, Any],
    *,
    strict: bool,
) -> dict[str, Any]:
    args = _coerce_wallets_sync_args(raw_args, strict=strict)
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return {"results": []}
    return {
        "results": sync_wallet(
            conn,
            runtime_config,
            None,
            None,
            wallet_ref=args["wallet"],
            sync_all=args["all"],
        )
    }


def _journals_process_payload(conn: sqlite3.Connection) -> dict[str, Any]:
    return process_journals(conn, None, None)


def _parse_ai_tool_call(raw: dict[str, Any], index: int) -> ParsedAiToolCall:
    call_id = raw.get("id")
    if not isinstance(call_id, str) or not call_id:
        call_id = f"call_{index}"
    function = raw.get("function")
    if not isinstance(function, dict):
        return ParsedAiToolCall(
            call_id=call_id,
            name="",
            arguments={},
            argument_error="invalid_tool_call",
        )
    name = function.get("name")
    if not isinstance(name, str):
        name = ""
    raw_arguments = function.get("arguments")
    if raw_arguments in (None, ""):
        return ParsedAiToolCall(call_id=call_id, name=name, arguments={})
    if isinstance(raw_arguments, dict):
        return ParsedAiToolCall(call_id=call_id, name=name, arguments=raw_arguments)
    if not isinstance(raw_arguments, str):
        return ParsedAiToolCall(
            call_id=call_id,
            name=name,
            arguments={},
            argument_error="invalid_arguments",
        )
    try:
        parsed = json.loads(raw_arguments)
    except json.JSONDecodeError:
        return ParsedAiToolCall(
            call_id=call_id,
            name=name,
            arguments={},
            argument_error="invalid_arguments",
        )
    if not isinstance(parsed, dict):
        return ParsedAiToolCall(
            call_id=call_id,
            name=name,
            arguments={},
            argument_error="invalid_arguments",
        )
    return ParsedAiToolCall(call_id=call_id, name=name, arguments=parsed)


def _tool_result_denied(reason: str, *, message: str | None = None) -> dict[str, Any]:
    result: dict[str, Any] = {"ok": False, "reason": reason}
    if message:
        result["message"] = message
    return result


def _execute_read_only_ai_tool(call: ParsedAiToolCall, runtime: AiToolRuntime) -> dict[str, Any]:
    if call.argument_error:
        return _tool_result_denied(call.argument_error)
    entry = get_tool(call.name)
    if entry is None or entry.kind_class != "read_only":
        return _tool_result_denied("tool_not_allowed")
    try:
        if call.name == "read_skill_reference":
            reference_name = call.arguments.get("name")
            if not isinstance(reference_name, str):
                raise AppError(
                    "read_skill_reference requires a name string",
                    code="validation",
                    retryable=False,
                )
            return {
                "ok": True,
                "envelope": build_envelope(
                    "read_skill_reference",
                    read_skill_reference(reference_name),
                ),
            }
        if entry.daemon_kind is None:
            return _tool_result_denied("tool_not_allowed")

        def _read(conn: sqlite3.Connection) -> dict[str, Any]:
            if entry.daemon_kind == "status":
                payload = _status_payload_from_parts(
                    conn,
                    runtime.data_root,
                    runtime.runtime_config,
                )
            elif entry.daemon_kind == "ui.overview.snapshot":
                payload = build_overview_snapshot(conn)
            elif entry.daemon_kind == "ui.transactions.list":
                payload = build_transactions_snapshot(conn, call.arguments)
            elif entry.daemon_kind == "ui.wallets.list":
                payload = build_wallets_list_snapshot(conn, runtime.runtime_config)
            elif entry.daemon_kind == "ui.backends.list":
                payload = build_backends_list_snapshot(conn, runtime.runtime_config)
            elif entry.daemon_kind == "ui.profiles.snapshot":
                payload = build_profiles_snapshot(conn)
            elif entry.daemon_kind == "ui.reports.capital_gains":
                payload = build_capital_gains_snapshot(conn)
            elif entry.daemon_kind == "ui.journals.snapshot":
                payload = build_journals_snapshot(conn)
            elif entry.daemon_kind == "ui.journals.quarantine":
                payload = build_journals_quarantine_snapshot(conn, call.arguments)
            elif entry.daemon_kind == "ui.journals.transfers.list":
                payload = build_journals_transfers_list_snapshot(conn, call.arguments)
            elif entry.daemon_kind == "ui.rates.summary":
                payload = build_rates_summary_snapshot(conn)
            elif entry.daemon_kind == "ui.workspace.health":
                payload = build_workspace_health_snapshot(conn)
            elif entry.daemon_kind == "ui.next_actions":
                payload = build_next_actions_snapshot(conn)
            else:
                return _tool_result_denied("tool_not_allowed")
            return {"ok": True, "envelope": build_envelope(entry.daemon_kind, payload)}

        return _run_on_daemon_main_thread(runtime, _read)
    except AppError as exc:
        return _tool_result_denied(
            exc.code or "tool_error",
            message=str(exc),
        )
    except Exception as exc:
        traceback.print_exc(file=sys.stderr)
        sys.stderr.flush()
        return _tool_result_denied(
            "tool_error",
            message=str(exc) or exc.__class__.__name__,
        )


def _execute_mutating_ai_tool(call: ParsedAiToolCall, runtime: AiToolRuntime) -> dict[str, Any]:
    if call.argument_error:
        return _tool_result_denied(call.argument_error)
    entry = get_tool(call.name)
    if entry is None or entry.kind_class != "mutating":
        return _tool_result_denied("tool_not_allowed")
    try:
        if entry.daemon_kind == "ui.wallets.sync":
            args = _coerce_wallets_sync_args(call.arguments, strict=True)

            def _execute(conn: sqlite3.Connection) -> dict[str, Any]:
                payload = _wallets_sync_payload(
                    conn,
                    runtime.runtime_config,
                    args,
                    strict=True,
                )
                return {"ok": True, "envelope": build_envelope(entry.daemon_kind, payload)}

            return _run_on_daemon_main_thread(runtime, _execute)
        if entry.daemon_kind == "ui.journals.process":
            if call.arguments:
                unknown = sorted(call.arguments)
                raise AppError(
                    "ui.journals.process received unsupported arguments",
                    code="validation",
                    details={"unknown": unknown},
                    retryable=False,
                )

            def _execute(conn: sqlite3.Connection) -> dict[str, Any]:
                payload = _journals_process_payload(conn)
                return {"ok": True, "envelope": build_envelope(entry.daemon_kind, payload)}

            return _run_on_daemon_main_thread(runtime, _execute)
        else:
            return _tool_result_denied("tool_not_allowed")
    except AppError as exc:
        return _tool_result_denied(
            exc.code or "tool_error",
            message=str(exc),
        )
    except Exception as exc:
        traceback.print_exc(file=sys.stderr)
        sys.stderr.flush()
        return _tool_result_denied(
            "tool_error",
            message=str(exc) or exc.__class__.__name__,
        )


def _tool_result_content_for_model(result: dict[str, Any]) -> str:
    return json.dumps(json_ready(result), sort_keys=True, separators=(",", ":"))


def _write_ai_chat_status(
    out: _OutputChannel,
    request_id: object,
    *,
    phase: str,
    label: str,
) -> None:
    out.write(
        _with_request_id(
            build_envelope(
                "ai.chat.status",
                {
                    "phase": phase,
                    "label": label,
                },
            ),
            request_id,
        )
    )


def _stream_ai_chat_tool_turn(
    request_id: object,
    client: OpenAICompatClient,
    validated: dict[str, Any],
    messages: list[dict[str, Any]],
    tools: list[dict[str, Any]],
    out: _OutputChannel,
    cancel_event: threading.Event,
) -> tuple[list[dict[str, Any]], str, str, str | None]:
    tool_calls: list[dict[str, Any]] = []
    content_parts: list[str] = []
    reasoning_parts: list[str] = []
    finish_reason = None
    _write_ai_chat_status(
        out,
        request_id,
        phase="waiting_for_model",
        label="Thinking",
    )
    for chunk in client.stream_chat(
        messages=messages,
        model=validated["model"],
        options=validated["options"],
        tools=tools,
        tool_choice="auto",
    ):
        if cancel_event.is_set():
            finish_reason = "cancelled"
            break
        delta = chunk.delta
        delta_tool_calls = delta.get("tool_calls")
        if isinstance(delta_tool_calls, list):
            tool_calls = delta_tool_calls
        delta_payload: dict[str, Any] = {}
        content = delta.get("content")
        reasoning = delta.get("reasoning")
        if isinstance(content, str) and content:
            content_parts.append(content)
            delta_payload["content"] = content
        if isinstance(reasoning, str) and reasoning:
            reasoning_parts.append(reasoning)
            delta_payload["reasoning"] = reasoning
        if delta_payload:
            out.write(
                _with_request_id(
                    build_envelope("ai.chat.delta", {"delta": delta_payload}),
                    request_id,
                )
            )
        if chunk.finish_reason is not None:
            finish_reason = chunk.finish_reason
        if cancel_event.is_set():
            finish_reason = "cancelled"
            break
    return tool_calls, "".join(content_parts), "".join(reasoning_parts), finish_reason


def _write_ai_chat_terminal(
    out: _OutputChannel,
    request_id: object,
    provider_snapshot: dict[str, Any],
    validated: dict[str, Any],
    finish_reason: str | None,
) -> None:
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


def _run_ai_chat_tool_loop(
    request_id: object,
    client: OpenAICompatClient,
    provider_snapshot: dict[str, Any],
    validated: dict[str, Any],
    out: _OutputChannel,
    active_chat: ActiveAiChat,
    runtime: AiToolRuntime,
) -> None:
    cancel_event = active_chat.cancel_event
    messages = build_chat_messages(
        validated["messages"],
        system_prompt_kind=validated["system_prompt_kind"],
        system_prompt=validated["system_prompt"],
    )
    tools = build_openai_tools()
    finish_reason = None
    for _iteration in range(validated["tool_loop_max_iterations"]):
        if cancel_event.is_set():
            finish_reason = "cancelled"
            break
        tool_calls, content, _reasoning, finish_reason = _stream_ai_chat_tool_turn(
            request_id,
            client,
            validated,
            messages,
            tools,
            out,
            cancel_event,
        )
        if cancel_event.is_set():
            finish_reason = "cancelled"
            break
        if not tool_calls:
            break

        messages.append(
            {
                "role": "assistant",
                "content": content,
                "tool_calls": tool_calls,
            }
        )
        for index, raw_tool_call in enumerate(tool_calls):
            if not isinstance(raw_tool_call, dict):
                continue
            call = _parse_ai_tool_call(raw_tool_call, index)
            entry = get_tool(call.name)
            kind_class = entry.kind_class if entry is not None else "unknown"
            display_name = entry.name if entry is not None else call.name
            tool_session_name = entry.name if entry is not None else call.name
            preview_arguments = (
                redact_tool_arguments(call.arguments)
                if kind_class != "read_only"
                else call.arguments
            )
            needs_consent = (
                entry is not None
                and entry.kind_class == "mutating"
                and not call.argument_error
                and not active_chat.consent.has_session_allow(tool_session_name)
            )
            out.write(
                _with_request_id(
                    build_envelope(
                        "ai.chat.tool_call",
                        {
                            "call_id": call.call_id,
                            "name": display_name,
                            "arguments": preview_arguments,
                            "kind_class": kind_class,
                            "needs_consent": needs_consent,
                        },
                    ),
                    request_id,
                )
            )
            if cancel_event.is_set():
                finish_reason = "cancelled"
                break
            if entry is not None and entry.kind_class == "mutating" and not call.argument_error:
                if needs_consent:
                    active_chat.consent.expect(call.call_id)
                    out.write(
                        _with_request_id(
                            build_envelope(
                                "ai.chat.tool_consent_required",
                                {
                                    "call_id": call.call_id,
                                    "name": display_name,
                                    "summary": summarize_tool_call(entry, call.arguments),
                                    "arguments_preview": preview_arguments,
                                },
                            ),
                            request_id,
                        )
                    )
                decision = active_chat.consent.wait(
                    call_id=call.call_id,
                    tool_name=tool_session_name,
                    cancel_event=cancel_event,
                    timeout=AI_TOOL_CONSENT_TIMEOUT_SECONDS,
                )
                if decision == "cancelled" or cancel_event.is_set():
                    finish_reason = "cancelled"
                    break
                if decision == "deny":
                    result = _tool_result_denied("user_denied")
                elif decision == "consent_timeout":
                    result = _tool_result_denied("consent_timeout")
                else:
                    out.write(
                        _with_request_id(
                            build_envelope(
                                "ai.chat.tool_call",
                                {
                                    "call_id": call.call_id,
                                    "name": display_name,
                                    "arguments": preview_arguments,
                                    "kind_class": kind_class,
                                    "needs_consent": False,
                                },
                            ),
                            request_id,
                        )
                    )
                    result = _execute_mutating_ai_tool(call, runtime)
            else:
                result = _execute_read_only_ai_tool(call, runtime)
            out.write(
                _with_request_id(
                    build_envelope(
                        "ai.chat.tool_result",
                        {"call_id": call.call_id, **result},
                    ),
                    request_id,
                )
            )
            messages.append(
                {
                    "role": "tool",
                    "tool_call_id": call.call_id,
                    "content": _tool_result_content_for_model(result),
                }
            )
            if cancel_event.is_set():
                finish_reason = "cancelled"
                break
        if finish_reason == "cancelled":
            break
    else:
        finish_reason = "tool_loop_max_iterations"

    if cancel_event.is_set():
        finish_reason = "cancelled"
    _write_ai_chat_terminal(out, request_id, provider_snapshot, validated, finish_reason)


def _run_ai_chat_stream(
    request_id: object,
    provider_snapshot: dict[str, Any],
    validated: dict[str, Any],
    out: _OutputChannel,
    active_chat: ActiveAiChat,
    active_ai_chats: ActiveAiChats,
    registry_key: str | None,
    runtime: AiToolRuntime,
) -> None:
    """Thread target — streams AI records and a terminal `ai.chat`."""
    cancel_event = active_chat.cancel_event
    try:
        finish_reason = None
        if not cancel_event.is_set():
            _write_ai_chat_status(
                out,
                request_id,
                phase="preparing",
                label="Preparing chat",
            )
            client = OpenAICompatClient(
                base_url=provider_snapshot["base_url"],
                api_key=provider_snapshot.get("api_key"),
            )
            _write_ai_chat_status(
                out,
                request_id,
                phase="connecting",
                label="Connecting",
            )
            if validated["tools_enabled"]:
                _run_ai_chat_tool_loop(
                    request_id,
                    client,
                    provider_snapshot,
                    validated,
                    out,
                    active_chat,
                    runtime,
                )
                return
            stream_messages = build_chat_messages(
                validated["messages"],
                system_prompt_kind=validated["system_prompt_kind"],
                system_prompt=validated["system_prompt"],
            )
            _write_ai_chat_status(
                out,
                request_id,
                phase="waiting_for_model",
                label="Loading model",
            )
            for chunk in client.stream_chat(
                messages=stream_messages,
                model=validated["model"],
                options=validated["options"],
            ):
                if cancel_event.is_set():
                    finish_reason = "cancelled"
                    break
                delta_payload = {"delta": chunk.delta}
                if chunk.finish_reason is not None:
                    finish_reason = chunk.finish_reason
                out.write(
                    _with_request_id(
                        build_envelope("ai.chat.delta", delta_payload),
                        request_id,
                    )
                )
                if cancel_event.is_set():
                    finish_reason = "cancelled"
                    break
        if cancel_event.is_set():
            finish_reason = "cancelled"
        _write_ai_chat_terminal(out, request_id, provider_snapshot, validated, finish_reason)
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
    finally:
        active_ai_chats.unregister(registry_key, active_chat)


def _ai_provider_redacted(ctx: DaemonContext, provider: dict) -> dict:
    return redact_ai_provider_for_output(
        provider,
        default_name=get_default_ai_provider_name(ctx.conn),
    )


def _verify_passphrase_for_reveal(ctx: "DaemonContext", passphrase: str) -> bool:
    """Confirm that `passphrase` would unlock the active database.

    Opens a throw-away SQLCipher connection so a wrong passphrase fails
    cleanly without affecting the live `ctx.conn` handle.
    """

    if not sqlcipher_available():
        return False
    db_path = resolve_database_path(resolve_effective_data_root(ctx.data_root))
    try:
        probe = open_encrypted(db_path, passphrase, quiet_unlock_errors=True)
    except AppError as exc:
        if exc.code == "unlock_failed":
            return False
        raise
    probe.close()
    return True


def _verify_passphrase_with_backoff(
    ctx: "DaemonContext",
    scope: str,
    passphrase: str,
) -> bool:
    ctx.auth_backoff.check(scope)
    verified = _verify_passphrase_for_reveal(ctx, passphrase)
    if verified:
        ctx.auth_backoff.record_success()
    else:
        ctx.auth_backoff.record_failure()
    return verified


def _require_sensitive_local_auth(
    ctx: "DaemonContext",
    *,
    args: dict[str, Any],
    request_id: object,
    scope: str,
    label: str,
    plaintext_ack_key: str,
    plaintext_ack_value: str,
) -> tuple[dict[str, Any], bool] | None:
    auth = args.get("auth_response")
    if _database_file_is_encrypted(ctx):
        passphrase = auth.get("passphrase_secret") if isinstance(auth, dict) else None
        if not isinstance(passphrase, str) or not passphrase:
            return (
                _with_request_id(
                    build_envelope(
                        "auth_required",
                        {
                            "scope": scope,
                            "label": label,
                        },
                    ),
                    request_id,
                ),
                False,
            )
        verified = _verify_passphrase_with_backoff(ctx, scope, passphrase)
        if not verified:
            return (
                _error_envelope(
                    "local_auth_denied",
                    "passphrase verification failed",
                    request_id=request_id,
                    retryable=True,
                ),
                False,
            )
        return None

    if not isinstance(auth, dict) or auth.get(plaintext_ack_key) != plaintext_ack_value:
        raise AppError(
            f"{scope} requires plaintext acknowledgement",
            code="validation",
            hint=f"Ask the user to type {plaintext_ack_value!r} before changing plaintext local data.",
        )
    return None


def _delete_current_workspace(ctx: "DaemonContext") -> dict[str, Any]:
    context = current_context_snapshot(ctx.conn)
    workspace_id = context.get("workspace_id")
    workspace_label = context.get("workspace_label")
    if not workspace_id:
        raise AppError(
            "No current workspace is selected.",
            code="state_not_ready",
            hint="Reset the local UI identity if you only need to return to the Welcome flow.",
        )

    counts = {
        "profiles": ctx.conn.execute(
            "SELECT COUNT(*) AS count FROM profiles WHERE workspace_id = ?",
            (workspace_id,),
        ).fetchone()["count"],
        "wallets": ctx.conn.execute(
            "SELECT COUNT(*) AS count FROM wallets WHERE workspace_id = ?",
            (workspace_id,),
        ).fetchone()["count"],
        "transactions": ctx.conn.execute(
            "SELECT COUNT(*) AS count FROM transactions WHERE workspace_id = ?",
            (workspace_id,),
        ).fetchone()["count"],
    }
    with ctx.conn:
        ctx.conn.execute("DELETE FROM workspaces WHERE id = ?", (workspace_id,))
        ctx.conn.execute(
            "DELETE FROM settings WHERE key IN ('context_workspace', 'context_profile')"
        )

    return {
        "deleted": True,
        "workspace": {"id": workspace_id, "label": workspace_label},
        "removed": counts,
    }


def _switch_profile_payload(
    conn: sqlite3.Connection,
    args: dict[str, Any],
) -> dict[str, Any]:
    profile_id = args.get("profile_id")
    if not isinstance(profile_id, str) or not profile_id.strip():
        raise AppError(
            "ui.profiles.switch requires profile_id",
            code="validation",
            hint="Select a profile from the current profiles snapshot.",
            retryable=False,
        )
    profile_id = profile_id.strip()
    row = conn.execute(
        """
        SELECT
            p.id,
            p.label,
            p.workspace_id,
            w.label AS workspace_label
        FROM profiles p
        JOIN workspaces w ON w.id = p.workspace_id
        WHERE p.id = ?
        LIMIT 1
        """,
        (profile_id,),
    ).fetchone()
    if not row:
        raise AppError(
            "profile not found",
            code="validation",
            hint="Refresh profiles and choose an existing profile.",
            details={"profile_id": profile_id},
            retryable=False,
        )

    with conn:
        set_setting(conn, "context_workspace", row["workspace_id"])
        set_setting(conn, "context_profile", row["id"])

    return {
        "activeProfileId": row["id"],
        "activeWorkspaceId": row["workspace_id"],
        "profile": {"id": row["id"], "name": row["label"]},
        "workspace": {"id": row["workspace_id"], "name": row["workspace_label"]},
    }


def _profile_defaults_for_workspace(
    conn: sqlite3.Connection,
    workspace_id: str,
) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    rows = conn.execute(
        """
        SELECT
            id,
            fiat_currency,
            tax_country,
            tax_long_term_days,
            gains_algorithm
        FROM profiles
        WHERE workspace_id = ?
        ORDER BY created_at ASC, label ASC
        """,
        (workspace_id,),
    ).fetchall()
    row = next(
        (candidate for candidate in rows if candidate["id"] == context["profile_id"]),
        rows[0] if rows else None,
    )
    if row:
        return {
            "fiat_currency": row["fiat_currency"],
            "tax_country": row["tax_country"],
            "tax_long_term_days": row["tax_long_term_days"],
            "gains_algorithm": row["gains_algorithm"],
        }
    return {
        "fiat_currency": "EUR",
        "tax_country": "generic",
        "tax_long_term_days": 365,
        "gains_algorithm": "FIFO",
    }


def _create_profile_payload(
    conn: sqlite3.Connection,
    args: dict[str, Any],
) -> dict[str, Any]:
    workspace_id = args.get("workspace_id")
    if not isinstance(workspace_id, str) or not workspace_id.strip():
        raise AppError(
            "ui.profiles.create requires workspace_id",
            code="validation",
            hint="Choose the workspace that should own the new profile.",
            retryable=False,
        )
    label = args.get("label")
    if not isinstance(label, str) or not label.strip():
        raise AppError(
            "ui.profiles.create requires label",
            code="validation",
            hint="Enter a profile name.",
            retryable=False,
        )
    workspace_id = workspace_id.strip()
    defaults = _profile_defaults_for_workspace(conn, workspace_id)
    profile = core_accounts.create_profile(
        conn,
        workspace_id,
        label.strip(),
        defaults["fiat_currency"],
        defaults["gains_algorithm"],
        defaults["tax_country"],
        int(defaults["tax_long_term_days"]),
    )
    workspace = conn.execute(
        "SELECT id, label FROM workspaces WHERE id = ?",
        (profile["workspace_id"],),
    ).fetchone()
    return {
        "activeProfileId": profile["id"],
        "activeWorkspaceId": profile["workspace_id"],
        "profile": {"id": profile["id"], "name": profile["label"]},
        "workspace": {
            "id": workspace["id"] if workspace else profile["workspace_id"],
            "name": workspace["label"] if workspace else "",
        },
        "defaults": {
            "fiat_currency": profile["fiat_currency"],
            "tax_country": profile["tax_country"],
            "tax_long_term_days": profile["tax_long_term_days"],
            "gains_algorithm": profile["gains_algorithm"],
        },
    }


def _create_workspace_payload(
    conn: sqlite3.Connection,
    args: dict[str, Any],
) -> dict[str, Any]:
    label = args.get("label")
    if not isinstance(label, str) or not label.strip():
        raise AppError(
            "ui.workspace.create requires label",
            code="validation",
            hint="Enter a workspace name.",
            retryable=False,
        )
    workspace = core_accounts.create_workspace(conn, label.strip())
    return {
        "workspace": {
            "id": workspace["id"],
            "name": workspace["label"],
            "created": (workspace["created_at"] or "")[:10],
        },
        "activeWorkspaceId": workspace["id"],
        "activeProfileId": "",
    }


def _wallet_ref_from_args(args: dict[str, Any], kind: str) -> str:
    wallet_ref = args.get("wallet")
    if not isinstance(wallet_ref, str) or not wallet_ref.strip():
        raise AppError(
            f"{kind} requires wallet",
            code="validation",
            hint="Pass the wallet id or label for the active profile.",
            retryable=False,
        )
    return wallet_ref.strip()


def _update_wallet_payload(
    ctx: "DaemonContext",
    args: dict[str, Any],
    request_id: object,
) -> tuple[dict[str, Any], bool]:
    wallet_ref = _wallet_ref_from_args(args, "ui.wallets.update")
    label = args.get("label")
    if not isinstance(label, str) or not label.strip():
        raise AppError(
            "ui.wallets.update requires label",
            code="validation",
            hint="Enter a new connection label.",
            retryable=False,
        )
    wallet = core_wallets.get_wallet_details(ctx.conn, None, None, wallet_ref)
    auth_result = _require_sensitive_local_auth(
        ctx,
        args=args,
        request_id=request_id,
        scope="update_wallet",
        label=f"Re-enter database passphrase to change wallet source {wallet['label']!r}",
        plaintext_ack_key="plaintext_change_ack",
        plaintext_ack_value=PLAINTEXT_CHANGE_ACK,
    )
    if auth_result is not None:
        return auth_result
    updated = core_wallets.update_wallet(
        ctx.conn,
        None,
        None,
        wallet_ref,
        {"label": label.strip()},
    )
    return (
        _with_request_id(
            build_envelope("ui.wallets.update", {"wallet": updated}),
            request_id,
        ),
        False,
    )


def _delete_wallet_payload(
    ctx: "DaemonContext",
    args: dict[str, Any],
    request_id: object,
) -> tuple[dict[str, Any], bool]:
    if args.get("confirm") != "DELETE":
        raise AppError(
            "ui.wallets.delete requires confirm='DELETE'",
            code="validation",
            hint="Ask the user to confirm the destructive wallet-source deletion.",
            retryable=False,
        )
    cascade = args.get("cascade", False)
    if not isinstance(cascade, bool):
        raise AppError(
            "ui.wallets.delete cascade must be a boolean",
            code="validation",
            hint="Pass cascade=true only when the user confirmed local row deletion.",
            retryable=False,
        )
    wallet_ref = _wallet_ref_from_args(args, "ui.wallets.delete")
    wallet = core_wallets.get_wallet_details(ctx.conn, None, None, wallet_ref)
    confirm_wallet = args.get("confirm_wallet")
    if not isinstance(confirm_wallet, str) or confirm_wallet != wallet["label"]:
        raise AppError(
            "ui.wallets.delete requires the current wallet label",
            code="validation",
            hint="Ask the user to type the exact wallet label before deleting it.",
            details={"expected_wallet": wallet["label"]},
            retryable=False,
        )
    auth_result = _require_sensitive_local_auth(
        ctx,
        args=args,
        request_id=request_id,
        scope="delete_wallet",
        label=f"Re-enter database passphrase to delete wallet source {wallet['label']!r}",
        plaintext_ack_key="plaintext_delete_ack",
        plaintext_ack_value=PLAINTEXT_DELETE_ACK,
    )
    if auth_result is not None:
        return auth_result
    deleted = core_wallets.delete_wallet(
        ctx.conn,
        None,
        None,
        wallet_ref,
        cascade=cascade,
    )
    return (
        _with_request_id(
            build_envelope("ui.wallets.delete", {"wallet": deleted}),
            request_id,
        ),
        False,
    )


def _database_file_is_encrypted(ctx: "DaemonContext") -> bool:
    db_path = resolve_database_path(resolve_effective_data_root(ctx.data_root))
    return (
        db_path.exists()
        and db_path.stat().st_size > 0
        and not looks_like_plaintext_sqlite(db_path)
    )


def _handle_ai_chat_cancel(
    ctx: "DaemonContext", request_id: object, args: dict[str, Any]
) -> dict[str, Any]:
    target_request_id = args.get("target_request_id")
    if not isinstance(target_request_id, str) or not target_request_id:
        raise AppError(
            "ai.chat.cancel requires target_request_id",
            code="validation",
            hint="Pass {target_request_id: '<active ai.chat request_id>'}.",
        )
    cancelled, queued = ctx.active_ai_chats.cancel(target_request_id)
    payload: dict[str, Any] = {"cancelled": cancelled or queued}
    if queued:
        payload["queued"] = True
    return _with_request_id(
        build_envelope("ai.chat.cancel", payload),
        request_id,
    )


def _handle_ai_tool_call_consent(
    ctx: "DaemonContext", request_id: object, args: dict[str, Any]
) -> dict[str, Any]:
    target_request_id = args.get("target_request_id")
    call_id = args.get("call_id")
    decision = args.get("decision")
    if not isinstance(target_request_id, str) or not target_request_id:
        raise AppError(
            "ai.tool_call.consent requires target_request_id",
            code="validation",
            hint="Pass {target_request_id: '<active ai.chat request_id>'}.",
        )
    if not isinstance(call_id, str) or not call_id:
        raise AppError(
            "ai.tool_call.consent requires call_id",
            code="validation",
        )
    if decision not in ("allow_once", "allow_session", "deny"):
        raise AppError(
            "ai.tool_call.consent decision must be allow_once, allow_session, or deny",
            code="validation",
            details={"decision": decision},
        )
    recorded = ctx.active_ai_chats.record_consent(
        target_request_id,
        call_id,
        decision,
    )
    payload: dict[str, Any] = {"recorded": recorded}
    if not recorded:
        payload["reason"] = "not_found"
    return _with_request_id(
        build_envelope("ai.tool_call.consent", payload),
        request_id,
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

    if kind == "daemon.lock":
        if ctx.conn is not None:
            ctx.conn.close()
            ctx.conn = None
        return (
            _with_request_id(
                build_envelope("daemon.lock", {"locked": True}),
                request_id,
            ),
            False,
        )

    if kind == "daemon.unlock":
        args = _coerce_args_dict(request_id, request.get("args"))
        passphrase = _passphrase_from_auth(args)
        if _database_file_is_encrypted(ctx):
            if not passphrase:
                return (
                    _locked_envelope(
                        "unlock_database",
                        "Enter the SQLCipher database passphrase to unlock Kassiber.",
                        request_id,
                    ),
                    False,
                )
            if not _verify_passphrase_with_backoff(
                ctx, "unlock_database", passphrase
            ):
                return (
                    _error_envelope(
                        "local_auth_denied",
                        "passphrase verification failed",
                        request_id=request_id,
                        retryable=True,
                    ),
                    False,
                )
            if ctx.conn is None:
                try:
                    _open_daemon_connection(ctx, passphrase=passphrase)
                except AppError as exc:
                    if exc.code == "unlock_failed":
                        return (
                            _error_envelope(
                                "local_auth_denied",
                                "passphrase verification failed",
                                request_id=request_id,
                                retryable=True,
                            ),
                            False,
                        )
                    raise
        else:
            _open_daemon_connection(ctx)
        return (
            _with_request_id(
                build_envelope(
                    "daemon.unlock",
                    {"unlocked": True, "status": _status_payload(ctx)},
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.secrets.init":
        args = _coerce_args_dict(request_id, request.get("args"))
        passphrase = _passphrase_from_auth(args)
        if not passphrase:
            return (
                _locked_envelope(
                    "init_database_encryption",
                    "Choose a SQLCipher database passphrase.",
                    request_id,
                ),
                False,
            )
        _validate_new_database_passphrase(passphrase)
        db_path = resolve_database_path(resolve_effective_data_root(ctx.data_root))
        if _database_file_is_encrypted(ctx):
            if not _verify_passphrase_with_backoff(
                ctx, "init_database_encryption", passphrase
            ):
                return (
                    _error_envelope(
                        "local_auth_denied",
                        "passphrase verification failed",
                        request_id=request_id,
                        retryable=True,
                    ),
                    False,
                )
            _open_daemon_connection(ctx, passphrase=passphrase)
            result: dict[str, Any] = {
                "encrypted": True,
                "already_encrypted": True,
                "database": str(db_path),
            }
        else:
            if ctx.conn is not None:
                ctx.conn.close()
                ctx.conn = None
            if db_path.exists() and db_path.stat().st_size > 0:
                migration = migrate_plaintext_to_encrypted(db_path, passphrase)
                result = {
                    "encrypted": True,
                    "already_encrypted": False,
                    "database": str(migration.encrypted_path),
                    "backup_path": str(migration.backup_path),
                    "integrity_check": migration.integrity_check,
                    "cipher_integrity_check": migration.cipher_integrity_check,
                    "credential_marker_clean": migration.credential_marker_clean,
                }
            else:
                created = create_empty_encrypted_database(db_path, passphrase)
                result = {
                    "encrypted": True,
                    "already_encrypted": False,
                    "database": str(created),
                    "backup_path": None,
                    "integrity_check": "ok",
                    "cipher_integrity_check": None,
                    "credential_marker_clean": True,
                }
            conn = _open_daemon_connection(ctx, passphrase=passphrase)
            if args.get("migrate_credentials") is not False:
                result["credentials"] = migrate_dotenv_credentials(
                    conn,
                    ctx.runtime_config["env_file"],
                    create_missing_backends=False,
                )
                ctx.runtime_config = load_runtime_config(ctx.runtime_config["env_file"])
                merge_db_backends(conn, ctx.runtime_config)
        return (
            _with_request_id(build_envelope("ui.secrets.init", result), request_id),
            False,
        )

    if kind == "ui.secrets.change_passphrase":
        args = _coerce_args_dict(request_id, request.get("args"))
        auth = args.get("auth_response")
        current = auth.get("passphrase_secret") if isinstance(auth, dict) else None
        new_passphrase = args.get("new_passphrase_secret")
        if not isinstance(current, str) or not current:
            return (
                _locked_envelope(
                    "change_database_passphrase",
                    "Enter the current SQLCipher database passphrase.",
                    request_id,
                ),
                False,
            )
        if not isinstance(new_passphrase, str) or not new_passphrase:
            raise AppError(
                "ui.secrets.change_passphrase requires a new passphrase",
                code="validation",
                hint="Ask the user to enter and confirm a new database passphrase.",
            )
        _validate_new_database_passphrase(new_passphrase)
        if not _database_file_is_encrypted(ctx):
            raise AppError(
                "database is plaintext; initialize SQLCipher before changing passphrase",
                code="plaintext_database",
                retryable=False,
            )
        if not _verify_passphrase_with_backoff(
            ctx, "change_database_passphrase", current
        ):
            return (
                _error_envelope(
                    "local_auth_denied",
                    "passphrase verification failed",
                    request_id=request_id,
                    retryable=True,
                ),
                False,
            )
        if ctx.conn is not None:
            ctx.conn.close()
            ctx.conn = None
        db_path = resolve_database_path(resolve_effective_data_root(ctx.data_root))
        result = change_database_passphrase(db_path, current, new_passphrase)
        _open_daemon_connection(ctx, passphrase=new_passphrase)
        return (
            _with_request_id(
                build_envelope("ui.secrets.change_passphrase", result),
                request_id,
            ),
            False,
        )

    if kind == "ai.chat.cancel":
        return (
            _handle_ai_chat_cancel(
                ctx, request_id, _coerce_args_dict(request_id, request.get("args"))
            ),
            False,
        )

    if kind == "ai.tool_call.consent":
        return (
            _handle_ai_tool_call_consent(
                ctx, request_id, _coerce_args_dict(request_id, request.get("args"))
            ),
            False,
        )

    if ctx.conn is None:
        try:
            _open_daemon_connection(ctx)
        except AppError as exc:
            if exc.code == "passphrase_required":
                return (
                    _locked_envelope(
                        "unlock_database",
                        "Enter the SQLCipher database passphrase to unlock Kassiber.",
                        request_id,
                    ),
                    False,
                )
            raise

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

    if kind == "ui.wallets.list":
        return (
            _with_request_id(
                build_envelope(
                    "ui.wallets.list",
                    build_wallets_list_snapshot(ctx.conn, ctx.runtime_config),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.backends.list":
        return (
            _with_request_id(
                build_envelope(
                    "ui.backends.list",
                    build_backends_list_snapshot(ctx.conn, ctx.runtime_config),
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

    if kind in {
        "ui.reports.export_pdf",
        "ui.reports.export_capital_gains_csv",
        "ui.reports.export_austrian_e1kv_pdf",
        "ui.reports.export_austrian_e1kv_xlsx",
    }:
        return (
            _with_request_id(
                build_envelope(
                    kind,
                    _ui_report_export_payload(
                        ctx,
                        kind,
                        _coerce_args_dict(request_id, request.get("args")),
                    ),
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

    if kind == "ui.journals.quarantine":
        return (
            _with_request_id(
                build_envelope(
                    "ui.journals.quarantine",
                    build_journals_quarantine_snapshot(ctx.conn, request.get("args")),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.journals.transfers.list":
        return (
            _with_request_id(
                build_envelope(
                    "ui.journals.transfers.list",
                    build_journals_transfers_list_snapshot(ctx.conn, request.get("args")),
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

    if kind == "ui.profiles.create":
        return (
            _with_request_id(
                build_envelope(
                    "ui.profiles.create",
                    _create_profile_payload(
                        ctx.conn,
                        _coerce_args_dict(request_id, request.get("args")),
                    ),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.profiles.switch":
        return (
            _with_request_id(
                build_envelope(
                    "ui.profiles.switch",
                    _switch_profile_payload(
                        ctx.conn,
                        _coerce_args_dict(request_id, request.get("args")),
                    ),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.rates.summary":
        return (
            _with_request_id(
                build_envelope("ui.rates.summary", build_rates_summary_snapshot(ctx.conn)),
                request_id,
            ),
            False,
        )

    if kind == "ui.workspace.health":
        return (
            _with_request_id(
                build_envelope(
                    "ui.workspace.health",
                    build_workspace_health_snapshot(ctx.conn),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.workspace.create":
        return (
            _with_request_id(
                build_envelope(
                    "ui.workspace.create",
                    _create_workspace_payload(
                        ctx.conn,
                        _coerce_args_dict(request_id, request.get("args")),
                    ),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.workspace.delete":
        args = _coerce_args_dict(request_id, request.get("args"))
        if args.get("confirm") != "DELETE":
            raise AppError(
                "ui.workspace.delete requires confirm='DELETE'",
                code="validation",
                hint="Ask the user to confirm the destructive workspace deletion.",
            )
        context = current_context_snapshot(ctx.conn)
        workspace_label = context.get("workspace_label")
        if not workspace_label:
            raise AppError(
                "No current workspace is selected.",
                code="validation",
                hint="Select a workspace before deleting it.",
            )
        confirm_workspace = args.get("confirm_workspace")
        if not isinstance(confirm_workspace, str) or confirm_workspace != workspace_label:
            raise AppError(
                "ui.workspace.delete requires the current workspace name",
                code="validation",
                hint="Ask the user to type the exact current workspace name before deleting it.",
                details={"expected_workspace": workspace_label},
            )
        auth_result = _require_sensitive_local_auth(
            ctx,
            args=args,
            request_id=request_id,
            scope="delete_workspace",
            label=f"Re-enter database passphrase to delete workspace {workspace_label!r}",
            plaintext_ack_key="plaintext_delete_ack",
            plaintext_ack_value=PLAINTEXT_DELETE_ACK,
        )
        if auth_result is not None:
            return auth_result
        return (
            _with_request_id(
                build_envelope(
                    "ui.workspace.delete",
                    _delete_current_workspace(ctx),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.next_actions":
        return (
            _with_request_id(
                build_envelope(
                    "ui.next_actions",
                    build_next_actions_snapshot(ctx.conn),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.wallets.update":
        return _update_wallet_payload(
            ctx,
            _coerce_args_dict(request_id, request.get("args")),
            request_id,
        )

    if kind == "ui.wallets.delete":
        return _delete_wallet_payload(
            ctx,
            _coerce_args_dict(request_id, request.get("args")),
            request_id,
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
        return (
            _with_request_id(
                build_envelope(
                    "ui.wallets.sync",
                    _wallets_sync_payload(
                        ctx.conn,
                        ctx.runtime_config,
                        args or {},
                        strict=False,
                    ),
                ),
                request_id,
            ),
            False,
        )

    if kind == "ui.journals.process":
        args = request.get("args")
        if args is not None and args != {}:
            if not isinstance(args, dict):
                details: dict[str, Any] = {"type": type(args).__name__}
            else:
                details = {"unknown": sorted(args)}
            return (
                _error_envelope(
                    "validation",
                    "ui.journals.process does not accept arguments",
                    request_id=request_id,
                    details=details,
                    retryable=False,
                ),
                False,
            )
        return (
            _with_request_id(
                build_envelope("ui.journals.process", _journals_process_payload(ctx.conn)),
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

    if kind == "ai.test_connection":
        # Transient connection test against caller-supplied credentials —
        # nothing is persisted. The Settings form uses this to validate the
        # *entered* base_url + api_key before saving. If `provider` names a
        # stored row and `api_key` is blank, the saved key is reused so the
        # form's "leave blank to keep current key" affordance still tests
        # with credentials.
        args = _coerce_args_dict(request_id, request.get("args"))
        base_url_raw = args.get("base_url")
        if not isinstance(base_url_raw, str) or not base_url_raw.strip():
            raise AppError(
                "ai.test_connection requires a non-empty base_url string",
                code="validation",
            )
        canonical_url = normalize_base_url(base_url_raw)
        api_key_raw = args.get("api_key")
        if api_key_raw is not None and not isinstance(api_key_raw, str):
            raise AppError(
                "ai.test_connection api_key must be a string",
                code="validation",
            )
        api_key_text = api_key_raw.strip() if isinstance(api_key_raw, str) else ""
        if not api_key_text:
            stored_provider = args.get("provider")
            if isinstance(stored_provider, str) and stored_provider.strip():
                try:
                    stored = get_db_ai_provider(ctx.conn, stored_provider)
                except AppError:
                    stored = None
                if stored and stored.get("api_key"):
                    api_key_text = stored["api_key"]
        # Use a tight timeout so a dead URL surfaces a clean error before
        # the Tauri supervisor's `DAEMON_INVOKE_TIMEOUT` (15s) kills the
        # daemon process. Test connection is interactive — a 10s ceiling
        # matches what the user expects from a "does this work?" probe.
        client = OpenAICompatClient(
            base_url=canonical_url,
            api_key=api_key_text or None,
            timeout=10.0,
        )
        # Strict mode: surface 4xx as `ai_request_invalid` so a missing
        # `/v1` suffix or a typoed host fails the test instead of
        # silently reporting "0 models reachable".
        models = client.list_models(strict=True)
        return (
            _with_request_id(
                build_envelope(
                    "ai.test_connection",
                    {
                        "base_url": canonical_url,
                        "model_count": len(models),
                        "models": models,
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
        require_ai_provider_acknowledged(provider)
        provider_snapshot = {
            "name": provider["name"],
            "base_url": provider["base_url"],
            "api_key": provider.get("api_key"),
            "kind": provider["kind"],
        }
        runtime = AiToolRuntime(
            data_root=ctx.data_root,
            runtime_config=dict(ctx.runtime_config),
            main_thread_tasks=ctx.main_thread_tasks,
        )
        registry_key, active_chat = ctx.active_ai_chats.register(request_id)
        thread = threading.Thread(
            target=_run_ai_chat_stream,
            args=(
                request_id,
                provider_snapshot,
                validated,
                out,
                active_chat,
                ctx.active_ai_chats,
                registry_key,
                runtime,
            ),
            daemon=True,
            name="kassiber-ai-chat",
        )
        thread.start()
        return (None, False)

    if kind == "wallets.reveal_descriptor":
        return _handle_reveal_request(
            ctx,
            request,
            request_id,
            kind=kind,
            scope="reveal_descriptor",
            target_kind="wallet",
        )

    if kind == "backends.reveal_token":
        return _handle_reveal_request(
            ctx,
            request,
            request_id,
            kind=kind,
            scope="reveal_token",
            target_kind="backend",
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


def _handle_reveal_request(
    ctx: DaemonContext,
    request: dict[str, Any],
    request_id: object,
    *,
    kind: str,
    scope: str,
    target_kind: str,
) -> tuple[dict[str, Any], bool]:
    """Reveal a sensitive field after a passphrase round-trip.

    Per the V4.1 plan, the daemon does not return secrets without an
    explicit `auth_response` from the client carrying the SQLCipher
    passphrase. We verify by opening a throw-away SQLCipher connection
    against the on-disk database; a wrong passphrase produces
    `local_auth_denied`.
    """

    args = request.get("args") or {}
    target = args.get("name") or args.get("wallet") or args.get("backend")
    if not isinstance(target, str) or not target:
        return (
            _error_envelope(
                "validation",
                f"{target_kind} reveal request requires a string `name` (or `wallet`/`backend`)",
                request_id=request_id,
                retryable=False,
            ),
            False,
        )

    auth = args.get("auth_response")
    if not isinstance(auth, dict) or "passphrase_secret" not in auth:
        return (
            _with_request_id(
                build_envelope(
                    "auth_required",
                    {
                        "scope": scope,
                        "label": f"Re-enter database passphrase to reveal {target_kind} {target!r}",
                    },
                ),
                request_id,
            ),
            False,
        )

    passphrase = auth.get("passphrase_secret")
    if not isinstance(passphrase, str) or not passphrase:
        return (
            _error_envelope(
                "local_auth_denied",
                "auth_response did not include a passphrase",
                request_id=request_id,
                retryable=True,
            ),
            False,
        )

    try:
        verified = _verify_passphrase_with_backoff(ctx, scope, passphrase)
    except AppError as exc:
        return (
            _error_envelope(
                exc.code or "auth_error",
                str(exc),
                request_id=request_id,
                details=exc.details,
                hint=exc.hint,
                retryable=exc.retryable,
            ),
            False,
        )
    if not verified:
        return (
            _error_envelope(
                "local_auth_denied",
                "passphrase verification failed",
                request_id=request_id,
                retryable=True,
            ),
            False,
        )

    if scope == "reveal_token":
        payload = core_accounts.reveal_backend_secrets(ctx.conn, ctx.runtime_config, target)
    else:
        workspace = args.get("workspace")
        profile = args.get("profile")
        payload = core_wallets.reveal_wallet_secrets(ctx.conn, workspace, profile, target)

    return (
        _with_request_id(
            build_envelope(kind, payload),
            request_id,
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
        active_ai_chats=ActiveAiChats(),
        main_thread_tasks=queue.Queue(),
        auth_backoff=AuthAttemptBackoff(
            str(resolve_config_root(args.data_root) / AUTH_BACKOFF_FILENAME)
        ),
    )
    input_lines = _start_stdin_reader(input_stream)

    out.write(
        build_envelope(
            "daemon.ready",
            {
                "version": __version__,
                "supported_kinds": list(SUPPORTED_KINDS),
            },
        ),
    )

    while True:
        _drain_daemon_main_thread_tasks(ctx)
        try:
            line = input_lines.get(timeout=0.05)
        except queue.Empty:
            continue
        if line == "":
            break
        if len(line) > MAX_REQUEST_LINE_CHARS:
            while line and not line.endswith("\n"):
                line = input_lines.get()
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
        _drain_daemon_main_thread_tasks(ctx)
        if should_shutdown:
            return 0

    return 0
