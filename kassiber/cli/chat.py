from __future__ import annotations

import json
import subprocess
import sys
import threading
import uuid
from dataclasses import dataclass, field
from typing import Any, Iterable, TextIO

from ..ai.client import CLI_DEFAULT_MODEL, is_cli_provider_locator
from ..ai.tools import TOOL_CATALOG
from ..core.runtime import resolve_db_passphrase_for_bypass
from ..errors import AppError
from .termrender import MarkdownStreamRenderer, render_envelope_table


_CONSENT_DECISIONS = {"allow_once", "allow_session", "deny"}
_ANSI_DIM = "\x1b[2m"
_ANSI_RESET = "\x1b[0m"
_DAEMON_STDERR_TAIL_CHARS = 2000
_FINISH_NOTICES = {
    "cancelled": "Cancelled.",
    "tool_loop_max_iterations": (
        "Stopped at the tool-loop iteration limit; raise "
        "--tool-loop-max-iterations to let the assistant keep working."
    ),
    "length": "Stopped at the provider token limit; raise --max-tokens for longer answers.",
}
_REPL_HELP = (
    "Commands:\n"
    "  /help             show this help\n"
    "  /tools            list daemon AI tools and their consent class\n"
    "  /model [id]       show or switch the model for following turns\n"
    "  /provider [name]  show or switch the provider (model re-resolves)\n"
    "  /allow <tool>     allow a mutating tool for this session\n"
    "  /allowed          show which mutating tools are pre-allowed\n"
    "  /new              start a fresh conversation (history cleared)\n"
    "  /exit             leave the chat (also /quit or Ctrl-D)\n"
    "Press Ctrl-C while the assistant is replying to cancel that turn.\n"
)


@dataclass
class ChatTurnResult:
    content: str
    terminal: dict[str, Any]
    tool_calls: list[dict[str, Any]] = field(default_factory=list)


@dataclass
class ChatSessionResult:
    provider: str | None = None
    model: str | None = None
    turns: list[ChatTurnResult] = field(default_factory=list)

    def to_payload(self) -> dict[str, Any]:
        last = self.turns[-1] if self.turns else None
        terminal_data = last.terminal.get("data", {}) if last else {}
        return {
            "provider": terminal_data.get("provider", self.provider),
            "model": terminal_data.get("model", self.model),
            "message": {
                "role": "assistant",
                "content": last.content if last else "",
            },
            "finish_reason": terminal_data.get("finish_reason") if last else None,
            "provenance": terminal_data.get("provenance") if last else None,
            "session_id": terminal_data.get("session_id") if last else None,
            "tool_calls": last.tool_calls if last else [],
        }


class _DaemonChatClient:
    def __init__(self, args: Any, *, transcript: TextIO | None = None) -> None:
        self._transcript = transcript
        self._bootstrap_passphrase: str | None = None
        self._stderr_tail = ""
        self._stderr_lock = threading.Lock()
        self._stderr_thread: threading.Thread | None = None
        self._allow_passphrase_prompt = (
            sys.stdin.isatty() and not bool(getattr(args, "non_interactive", False))
        )
        command = self._daemon_command(args)
        self._proc = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        if self._proc.stderr is not None:
            self._stderr_thread = threading.Thread(
                target=self._drain_stderr,
                name="kassiber-daemon-stderr-drain",
                daemon=True,
            )
            self._stderr_thread.start()
        try:
            ready = self.read()
        except AppError:
            self.close()
            raise
        if ready.get("kind") != "daemon.ready":
            self.close()
            raise AppError(
                "daemon did not start cleanly",
                code="daemon_start_failed",
                details={"envelope": ready},
                retryable=False,
            )
        if self._bootstrap_passphrase is not None:
            passphrase = self._bootstrap_passphrase
            self._bootstrap_passphrase = None
            request_id = f"cli-bootstrap-{uuid.uuid4()}"
            self.send(
                {
                    "kind": "daemon.unlock",
                    "request_id": request_id,
                    "args": {
                        "require_existing_project": True,
                        "auth_response": {"passphrase_secret": passphrase},
                    },
                },
                record=False,
            )
            unlocked = self.read(record=False)
            if (
                unlocked.get("kind") != "daemon.unlock"
                or unlocked.get("request_id") != request_id
                or unlocked.get("data", {}).get("unlocked") is not True
            ):
                self.close()
                error = unlocked.get("error") if isinstance(unlocked, dict) else None
                if not isinstance(error, dict):
                    error = {}
                raise AppError(
                    error.get("message", "daemon database unlock failed"),
                    code=error.get("code", "daemon_unlock_failed"),
                    hint=error.get("hint"),
                    retryable=bool(error.get("retryable", False)),
                )

    def _daemon_command(self, args: Any) -> list[str]:
        command = [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            args.data_root,
        ]
        if getattr(args, "env_file", None):
            command.extend(["--env-file", args.env_file])
        self._bootstrap_passphrase = resolve_db_passphrase_for_bypass(
            args,
            allow_prompt=self._allow_passphrase_prompt,
            require_existing_schema=True,
        )
        command.append("daemon")
        return command

    def _append_stderr(self, chunk: str) -> None:
        if not chunk:
            return
        with self._stderr_lock:
            self._stderr_tail = (self._stderr_tail + chunk)[-_DAEMON_STDERR_TAIL_CHARS:]

    def _drain_stderr(self) -> None:
        stream = self._proc.stderr
        if stream is None:
            return
        while True:
            try:
                chunk = stream.read(4096)
            except ValueError:
                return
            if not chunk:
                return
            self._append_stderr(chunk)

    def _stderr_snapshot(self) -> str:
        with self._stderr_lock:
            return self._stderr_tail

    def send(self, payload: dict[str, Any], *, record: bool = True) -> None:
        if self._proc.stdin is None:
            raise AppError(
                "daemon input stream is closed",
                code="daemon_closed",
                retryable=False,
            )
        line = json.dumps(payload, separators=(",", ":")) + "\n"
        if record and self._transcript is not None:
            self._transcript.write(line)
        self._proc.stdin.write(line)
        self._proc.stdin.flush()

    def read(self, *, record: bool = True) -> dict[str, Any]:
        if self._proc.stdout is None:
            raise AppError(
                "daemon output stream is closed",
                code="daemon_closed",
                retryable=False,
            )
        line = self._proc.stdout.readline()
        if line == "":
            stderr = self._stderr_snapshot()
            raise AppError(
                "daemon exited before chat completed",
                code="daemon_closed",
                details={"stderr": stderr} if stderr else {},
                retryable=False,
            )
        if record and self._transcript is not None:
            self._transcript.write(line if line.endswith("\n") else line + "\n")
        try:
            payload = json.loads(line)
        except json.JSONDecodeError as exc:
            raise AppError(
                "daemon emitted invalid JSON",
                code="daemon_protocol_error",
                details={"line": line[:1000], "error": str(exc)},
                retryable=False,
            ) from None
        if not isinstance(payload, dict):
            raise AppError(
                "daemon emitted a non-object JSON record",
                code="daemon_protocol_error",
                retryable=False,
            )
        return payload

    def close(self) -> None:
        if self._proc.stdin is not None and not self._proc.stdin.closed:
            self._proc.stdin.close()
        try:
            self._proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self._proc.terminate()
            try:
                self._proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self._proc.kill()
                self._proc.wait(timeout=2)
        if self._proc.stdout is not None:
            self._proc.stdout.close()
        if self._proc.stderr is not None:
            self._proc.stderr.close()
        if self._stderr_thread is not None:
            self._stderr_thread.join(timeout=1)


def _split_tool_names(values: Iterable[str] | None) -> set[str]:
    names: set[str] = set()
    for value in values or ():
        for item in value.split(","):
            stripped = item.strip()
            if stripped:
                names.add(stripped)
    # Consent prompts carry the catalog display name; accept the OpenAI wire
    # name too by mapping it through the catalog instead of guessing with
    # string replacement (display names may themselves contain underscores).
    for entry in TOOL_CATALOG:
        if entry.wire_name and entry.wire_name in names:
            names.add(entry.name)
    return names


def _chat_options(args: Any) -> dict[str, Any] | None:
    options: dict[str, Any] = {}
    if getattr(args, "temperature", None) is not None:
        options["temperature"] = args.temperature
    if getattr(args, "max_tokens", None) is not None:
        options["max_tokens"] = args.max_tokens
    effort = getattr(args, "reasoning_effort", None)
    if effort and effort != "auto":
        options["reasoning_effort"] = effort
    return options or None


def _resolve_prompt(args: Any) -> str | None:
    prompt = getattr(args, "prompt", None)
    prompt_flag = getattr(args, "prompt_text", None)
    if prompt and prompt_flag:
        raise AppError(
            "pass the chat prompt either positionally or with --prompt, not both",
            code="validation",
            retryable=False,
        )
    return prompt_flag if prompt_flag is not None else prompt


def _build_chat_args(
    args: Any,
    messages: list[dict[str, str]],
    *,
    session_id: str | None = None,
) -> dict[str, Any]:
    tools_enabled = not getattr(args, "no_tools", False)
    payload: dict[str, Any] = {
        "provider": getattr(args, "provider", None),
        "model": args.model,
        "messages": messages,
        "tools_enabled": tools_enabled,
        "tool_loop_max_iterations": getattr(args, "tool_loop_max_iterations", 8),
        "persist": False if getattr(args, "incognito", False) else "auto",
        "session_id": session_id,
    }
    system = getattr(args, "system", None)
    if system:
        payload["system_prompt_kind"] = "raw"
        payload["system_prompt"] = system
    elif tools_enabled:
        # With tools disabled the daemon default (no system prompt) is right;
        # the Kassiber prompt instructs tool use the provider would not have.
        payload["system_prompt_kind"] = "kassiber"
    options = _chat_options(args)
    if options:
        payload["options"] = options
    return payload


def _provider_rows(payload: dict[str, Any]) -> list[dict[str, Any]]:
    data = payload.get("data")
    if isinstance(data, dict) and isinstance(data.get("providers"), list):
        return [row for row in data["providers"] if isinstance(row, dict)]
    if isinstance(data, list):
        return [row for row in data if isinstance(row, dict)]
    return []


def _error_from_envelope(record: dict[str, Any], *, message: str, code: str) -> AppError:
    error = record.get("error") if isinstance(record.get("error"), dict) else {}
    return AppError(
        error.get("message", message),
        code=error.get("code", code),
        details=error.get("details"),
        hint=error.get("hint"),
        retryable=bool(error.get("retryable")),
    )


def _read_control_response(client: _DaemonChatClient, request_id: str) -> dict[str, Any]:
    while True:
        record = client.read()
        if record.get("request_id") != request_id:
            continue
        if record.get("kind") == "error":
            raise _error_from_envelope(
                record, message="daemon request failed", code="daemon_request_failed"
            )
        return record


def _resolve_continuation(
    client: _DaemonChatClient, args: Any
) -> tuple[str | None, list[dict[str, str]], dict[str, Any] | None]:
    """Resolve --continue / --session into (session_id, prior messages, session)."""
    requested = getattr(args, "session", None)
    if not requested and not getattr(args, "continue_session", False):
        return None, [], None
    if requested is None:
        request_id = f"chat-sessions-list-{uuid.uuid4().hex}"
        client.send(
            {
                "request_id": request_id,
                "kind": "ui.chat.sessions.list",
                "args": {"limit": 1},
            }
        )
        response = _read_control_response(client, request_id)
        data = response.get("data") if isinstance(response.get("data"), dict) else {}
        sessions = data.get("sessions") or []
        if not sessions:
            raise AppError(
                "no persisted chat sessions to continue",
                code="not_found",
                hint="Run a chat with history enabled first, or pass --session <id>.",
                retryable=False,
            )
        requested = sessions[0]["id"]
    request_id = f"chat-sessions-get-{uuid.uuid4().hex}"
    client.send(
        {
            "request_id": request_id,
            "kind": "ui.chat.sessions.get",
            "args": {"session_id": requested},
        }
    )
    response = _read_control_response(client, request_id)
    session = response.get("data") if isinstance(response.get("data"), dict) else {}
    messages = [
        {"role": message["role"], "content": message["content"]}
        for message in session.get("messages") or []
        if message.get("role") in {"user", "assistant"} and message.get("content")
    ]
    return requested, messages, session


def _resolve_default_model(client: _DaemonChatClient, args: Any) -> None:
    if getattr(args, "model", None):
        return
    request_id = f"chat-provider-list-{uuid.uuid4().hex}"
    client.send({"request_id": request_id, "kind": "ai.providers.list", "args": {}})
    rows = _provider_rows(_read_control_response(client, request_id))
    selected: dict[str, Any] | None = None
    provider_name = getattr(args, "provider", None)
    if provider_name:
        selected = next((row for row in rows if row.get("name") == provider_name), None)
    else:
        selected = next((row for row in rows if row.get("is_default")), None)
    if selected is None:
        raise AppError(
            "AI chat requires a provider",
            code="validation",
            hint="Set a default provider or pass --provider.",
            retryable=False,
        )
    model = selected.get("default_model")
    if (not isinstance(model, str) or not model.strip()) and is_cli_provider_locator(
        str(selected.get("base_url") or "")
    ):
        model = CLI_DEFAULT_MODEL
    if not isinstance(model, str) or not model.strip():
        name = selected.get("name") or provider_name or "selected provider"
        raise AppError(
            "AI chat requires a model",
            code="validation",
            hint=f"Pass --model, or set --default-model on provider '{name}'.",
            retryable=False,
        )
    args.model = model


def _write(text: str, out: TextIO) -> None:
    out.write(text)
    out.flush()


def _write_dim(text: str, out: TextIO) -> None:
    if out.isatty():
        _write(f"{_ANSI_DIM}{text}{_ANSI_RESET}", out)
    else:
        _write(text, out)


def _interactive_consent(
    *,
    name: str,
    summary: str,
    arguments_preview: dict[str, Any],
    stdin: TextIO,
    out: TextIO,
) -> str:
    _write(f"\nConsent required: {summary or name}\n", out)
    _write(f"Tool: {name}\n", out)
    if arguments_preview:
        _write(
            "Arguments:\n"
            + json.dumps(arguments_preview, indent=2, sort_keys=True)
            + "\n",
            out,
        )
    while True:
        _write("Allow? [y] once, [s] session, [n] deny, [c] cancel: ", out)
        choice = stdin.readline()
        if choice == "":
            return "deny"
        normalized = choice.strip().lower()
        if normalized in {"y", "yes"}:
            return "allow_once"
        if normalized in {"s", "session"}:
            return "allow_session"
        if normalized in {"n", "no", "deny"}:
            return "deny"
        if normalized in {"c", "cancel"}:
            return "cancel"


def _policy_decision(args: Any, tool_name: str, stdin: TextIO) -> str | None:
    if getattr(args, "yes", False):
        return "allow_session"
    if tool_name in _split_tool_names(getattr(args, "allow_tool", None)):
        return "allow_session"
    # Machine and NDJSON outputs are scripted surfaces: never mix an
    # interactive prompt into them, even when stdin happens to be a TTY.
    if (
        getattr(args, "format", None) == "json"
        or getattr(args, "stream_json", False)
        or getattr(args, "non_interactive", False)
    ):
        return "deny"
    if not stdin.isatty():
        return "deny"
    return None


def _send_consent(
    client: _DaemonChatClient,
    *,
    target_request_id: str,
    call_id: str,
    decision: str,
) -> str:
    request_id = f"chat-consent-{uuid.uuid4().hex}"
    client.send(
        {
            "request_id": request_id,
            "kind": "ai.tool_call.consent",
            "args": {
                "target_request_id": target_request_id,
                "call_id": call_id,
                "decision": decision,
            },
        }
    )
    return request_id


def _send_cancel(client: _DaemonChatClient, target_request_id: str) -> str:
    request_id = f"chat-cancel-{uuid.uuid4().hex}"
    client.send(
        {
            "request_id": request_id,
            "kind": "ai.chat.cancel",
            "args": {"target_request_id": target_request_id},
        }
    )
    return request_id


def _drain_until_terminal(client: _DaemonChatClient, request_id: str) -> dict[str, Any]:
    while True:
        record = client.read()
        if record.get("request_id") != request_id:
            continue
        if record.get("kind") in {"ai.chat", "error"}:
            return record


def _render_turn_footer(terminal: dict[str, Any], chrome: TextIO) -> None:
    data = terminal.get("data") if isinstance(terminal.get("data"), dict) else {}
    notice = _FINISH_NOTICES.get(data.get("finish_reason") or "")
    if notice:
        _write(notice + "\n", sys.stderr)
    provenance = (
        data.get("provenance") if isinstance(data.get("provenance"), dict) else {}
    )
    parts: list[str] = []
    provider = data.get("provider")
    model = data.get("model")
    if provider and model:
        parts.append(f"{provider}/{model}")
    tools_used = provenance.get("tools_used")
    if isinstance(tools_used, list) and tools_used:
        parts.append("tools: " + ", ".join(str(name) for name in tools_used))
    if provenance.get("auto_journal_processed"):
        parts.append("journals auto-refreshed")
    if provenance.get("auto_sync_attempted"):
        parts.append("synced" if provenance.get("auto_sync_ok") else "sync attempted")
    if parts:
        _write_dim("— " + " · ".join(parts) + "\n", chrome)


def _render_tool_listing(out: TextIO) -> None:
    width = max(len(entry.name) for entry in TOOL_CATALOG)
    for entry in sorted(TOOL_CATALOG, key=lambda e: (e.kind_class, e.name)):
        consent = (
            "mutating (asks consent)"
            if entry.kind_class == "mutating"
            else "read-only"
        )
        _write(f"  {entry.name.ljust(width)}  {consent}\n", out)


def _read_repl_line(input_stream: TextIO, out: TextIO) -> str:
    if input_stream is sys.stdin and input_stream.isatty():
        try:
            import readline  # noqa: F401  # line editing + in-session history
        except ImportError:
            # readline is optional; fall back to plain input without line editing.
            pass
        try:
            return input("> ") + "\n"
        except EOFError:
            return ""
    _write("> ", out)
    return input_stream.readline()


def _mutating_tool_names() -> set[str]:
    return {entry.name for entry in TOOL_CATALOG if entry.kind_class == "mutating"}


def _handle_model_command(args: Any, arg: str, out: TextIO) -> None:
    if not arg:
        _write(f"model: {args.model}\n", out)
        return
    args.model = arg
    _write(f"Switched to model {arg}.\n", out)


def _handle_provider_command(
    client: _DaemonChatClient, args: Any, arg: str, out: TextIO
) -> None:
    if not arg:
        current = getattr(args, "provider", None) or "(stored default)"
        _write(f"provider: {current}\n", out)
        return
    previous = (getattr(args, "provider", None), args.model)
    args.provider = arg
    args.model = None
    try:
        _resolve_default_model(client, args)
    except AppError as exc:
        args.provider, args.model = previous
        _write(f"Error: {exc}\n", out)
        if exc.hint:
            _write(f"Hint: {exc.hint}\n", out)
        return
    _write(f"Switched to provider {arg}, model {args.model}.\n", out)


def _handle_allow_command(arg: str, session_allowed: set[str], out: TextIO) -> None:
    if not arg:
        _write("Usage: /allow <tool-name>\n", out)
        return
    matched = sorted(_split_tool_names([arg]) & _mutating_tool_names())
    if not matched:
        _write(f"{arg} is not a known mutating tool; /tools lists them.\n", out)
        return
    session_allowed.update(matched)
    _write("Allowed for this session: " + ", ".join(matched) + "\n", out)


def _render_allowed(args: Any, session_allowed: set[str], out: TextIO) -> None:
    if getattr(args, "yes", False):
        _write("All mutating tools are allowed for this session (--yes).\n", out)
        return
    flag_allowed = sorted(
        _split_tool_names(getattr(args, "allow_tool", None)) & _mutating_tool_names()
    )
    lines = [f"  {name}  (--allow-tool)" for name in flag_allowed]
    lines.extend(
        f"  {name}  (this session)"
        for name in sorted(session_allowed - set(flag_allowed))
    )
    if not lines:
        _write("No mutating tools are pre-allowed; each will ask for consent.\n", out)
        return
    _write("\n".join(lines) + "\n", out)


def _run_turn(
    client: _DaemonChatClient,
    args: Any,
    messages: list[dict[str, str]],
    *,
    stdin: TextIO,
    out: TextIO,
    chrome: TextIO,
    render: bool,
    stream_out: TextIO | None = None,
    session_allowed: set[str] | None = None,
    session_id: str | None = None,
) -> ChatTurnResult:
    request_id = f"chat-{uuid.uuid4().hex}"
    client.send(
        {
            "request_id": request_id,
            "kind": "ai.chat",
            "args": _build_chat_args(args, messages, session_id=session_id),
        }
    )
    content_parts: list[str] = []
    tool_calls: dict[str, dict[str, Any]] = {}
    control_requests: set[str] = set()
    pretty = render and out.isatty() and not getattr(args, "plain", False)
    markdown = MarkdownStreamRenderer() if pretty else None
    try:
        return _stream_turn_records(
            client,
            args,
            request_id,
            content_parts,
            tool_calls,
            control_requests,
            stdin=stdin,
            out=out,
            chrome=chrome,
            render=render,
            stream_out=stream_out,
            session_allowed=session_allowed,
            markdown=markdown,
        )
    except BaseException:
        # An abnormal exit mid-stream must not leak open ANSI styles (or a
        # buffered table) into the next prompt or the user's shell.
        if markdown is not None:
            _write(markdown.flush(), out)
        raise


def _stream_turn_records(
    client: _DaemonChatClient,
    args: Any,
    request_id: str,
    content_parts: list[str],
    tool_calls: dict[str, dict[str, Any]],
    control_requests: set[str],
    *,
    stdin: TextIO,
    out: TextIO,
    chrome: TextIO,
    render: bool,
    stream_out: TextIO | None,
    session_allowed: set[str] | None,
    markdown: MarkdownStreamRenderer | None,
) -> ChatTurnResult:
    try:
        while True:
            record = client.read()
            kind = record.get("kind")
            record_request_id = record.get("request_id")
            if record_request_id in control_requests:
                if kind == "error":
                    raise _error_from_envelope(
                        record, message="chat control failed", code="chat_control_failed"
                    )
                continue
            if record_request_id != request_id:
                continue
            if stream_out is not None:
                _write(json.dumps(record, separators=(",", ":")) + "\n", stream_out)
            if kind == "error":
                raise _error_from_envelope(record, message="chat failed", code="chat_failed")
            data = record.get("data") if isinstance(record.get("data"), dict) else {}
            if kind == "ai.chat.status":
                if render and data.get("label"):
                    _write_dim(f"{data['label']}...\n", chrome)
            elif kind == "ai.chat.delta":
                delta = data.get("delta") if isinstance(data.get("delta"), dict) else {}
                reasoning = delta.get("reasoning")
                if render and isinstance(reasoning, str) and reasoning and out.isatty():
                    _write(f"{_ANSI_DIM}{reasoning}{_ANSI_RESET}", out)
                visible = delta.get("content")
                if isinstance(visible, str) and visible:
                    content_parts.append(visible)
                    if markdown is not None:
                        _write(markdown.feed(visible), out)
                    elif render:
                        _write(visible, out)
            elif kind == "ai.chat.tool_call":
                call_id = data.get("call_id")
                if isinstance(call_id, str):
                    tool_calls[call_id] = {
                        "call_id": call_id,
                        "name": data.get("name"),
                        "arguments": data.get("arguments") or {},
                        "kind_class": data.get("kind_class", "unknown"),
                        "needs_consent": bool(data.get("needs_consent")),
                        "status": "awaiting_consent"
                        if data.get("needs_consent")
                        else "running",
                    }
                    if render:
                        suffix = " (needs consent)" if data.get("needs_consent") else ""
                        _write(f"\nTool: {data.get('name', 'unknown')}{suffix}\n", chrome)
            elif kind == "ai.chat.tool_consent_required":
                call_id = data.get("call_id")
                name = data.get("name")
                if not isinstance(call_id, str) or not isinstance(name, str):
                    continue
                decision = _policy_decision(args, name, stdin)
                if decision is None and session_allowed is not None and name in session_allowed:
                    # Daemon-side allow_session only spans one ai.chat request;
                    # carry the user's "session" answer across REPL turns here.
                    decision = "allow_session"
                if decision is None:
                    decision = _interactive_consent(
                        name=name,
                        summary=str(data.get("summary") or name),
                        arguments_preview=data.get("arguments_preview") or {},
                        stdin=stdin,
                        out=chrome,
                    )
                    if decision == "allow_session" and session_allowed is not None:
                        session_allowed.add(name)
                if decision == "cancel":
                    control_requests.add(_send_cancel(client, request_id))
                elif decision in _CONSENT_DECISIONS:
                    control_requests.add(
                        _send_consent(
                            client,
                            target_request_id=request_id,
                            call_id=call_id,
                            decision=decision,
                        )
                    )
                else:
                    raise AppError(
                        "invalid consent decision",
                        code="validation",
                        details={"decision": decision},
                        retryable=False,
                    )
            elif kind == "ai.chat.tool_result":
                call_id = data.get("call_id")
                if isinstance(call_id, str):
                    existing = tool_calls.setdefault(
                        call_id,
                        {
                            "call_id": call_id,
                            "name": "Tool",
                            "arguments": {},
                            "kind_class": "unknown",
                            "needs_consent": False,
                        },
                    )
                    reason = data.get("reason")
                    if data.get("ok"):
                        existing["status"] = "done"
                    elif reason in {"user_denied", "consent_timeout"}:
                        existing["status"] = "denied"
                    else:
                        existing["status"] = "failed"
                    existing["reason"] = reason
                    existing["ok"] = bool(data.get("ok"))
                    if render and not data.get("ok") and data.get("reason"):
                        _write(f"\nTool result: {data['reason']}\n", chrome)
                    if (
                        markdown is not None
                        and chrome.isatty()
                        and data.get("ok")
                        and isinstance(data.get("envelope"), dict)
                        and not call_id.startswith("auto_read")
                    ):
                        # Deterministic data display: the numbers come from
                        # the daemon envelope, not from the model's retelling.
                        table = render_envelope_table(data["envelope"])
                        if table:
                            _write(table + "\n", chrome)
            elif kind == "ai.chat":
                if markdown is not None:
                    _write(markdown.flush(), out)
                if render and (content_parts or out.isatty()):
                    _write("\n", out)
                return ChatTurnResult(
                    content="".join(content_parts),
                    terminal=record,
                    tool_calls=list(tool_calls.values()),
                )
    except KeyboardInterrupt:
        # Cancel this turn cooperatively, then wait for the daemon's terminal
        # record so the transport stays usable for the next REPL turn.
        _send_cancel(client, request_id)
        try:
            terminal = _drain_until_terminal(client, request_id)
        except KeyboardInterrupt:
            raise AppError("chat cancelled", code="cancelled", retryable=False) from None
        if terminal.get("kind") == "error":
            raise _error_from_envelope(terminal, message="chat failed", code="chat_failed")
        if stream_out is not None:
            _write(json.dumps(terminal, separators=(",", ":")) + "\n", stream_out)
        if markdown is not None:
            _write(markdown.flush(), out)
        if render and (content_parts or out.isatty()):
            _write("\n", out)
        return ChatTurnResult(
            content="".join(content_parts),
            terminal=terminal,
            tool_calls=list(tool_calls.values()),
        )


def run_chat_command(
    args: Any,
    *,
    stdin: TextIO | None = None,
    stdout: TextIO | None = None,
) -> ChatSessionResult:
    input_stream = stdin or sys.stdin
    output_stream = stdout or sys.stdout
    one_shot_prompt = _resolve_prompt(args)
    stream_json = bool(getattr(args, "stream_json", False))
    machine = getattr(args, "format", None) == "json"
    if stream_json and machine:
        raise AppError(
            "--stream-json already emits NDJSON; drop --machine / --format json",
            code="validation",
            retryable=False,
        )
    if (machine or stream_json) and one_shot_prompt is None:
        raise AppError(
            "machine chat output requires a one-shot prompt",
            code="validation",
            retryable=False,
        )
    if getattr(args, "non_interactive", False) and one_shot_prompt is None:
        raise AppError(
            "non-interactive chat requires a one-shot prompt",
            code="interaction_required",
            hint="Pass a prompt argument, --prompt, or `-` to read the prompt from stdin.",
            retryable=False,
        )
    if getattr(args, "incognito", False) and (
        getattr(args, "continue_session", False) or getattr(args, "session", None)
    ):
        raise AppError(
            "--incognito cannot continue a persisted session",
            code="validation",
            retryable=False,
        )
    if one_shot_prompt is None and not input_stream.isatty():
        raise AppError(
            "interactive chat requires a TTY; pass a prompt for one-shot mode",
            code="validation",
            hint="Use `kassiber chat \"...\"` or `kassiber chat --prompt \"...\"`.",
            retryable=False,
        )
    if one_shot_prompt == "-":
        one_shot_prompt = input_stream.read().strip()
        if not one_shot_prompt:
            raise AppError(
                "stdin prompt is empty",
                code="validation",
                hint="Pipe or type the prompt when using `kassiber chat -`.",
                retryable=False,
            )

    transcript_path = getattr(args, "transcript", None)
    transcript = (
        open(transcript_path, "a", buffering=1, encoding="utf-8")
        if transcript_path
        else None
    )
    try:
        client = _DaemonChatClient(args, transcript=transcript)
    except BaseException:
        if transcript is not None:
            transcript.close()
        raise
    try:
        _resolve_default_model(client, args)
        session = ChatSessionResult(
            provider=getattr(args, "provider", None),
            model=args.model,
        )
        render = not machine and not stream_json
        chat_session_id, messages, stored_session = _resolve_continuation(client, args)
        if render and stored_session is not None:
            _write(
                f"Continuing: {stored_session.get('title', chat_session_id)} "
                f"({len(messages)} messages)\n",
                output_stream,
            )
        if one_shot_prompt is not None:
            # Piped stdout gets only the answer text; progress, tool
            # announcements, consent UI, and provenance move to stderr.
            chrome = output_stream if output_stream.isatty() else sys.stderr
            messages.append({"role": "user", "content": one_shot_prompt})
            result = _run_turn(
                client,
                args,
                messages,
                stdin=input_stream,
                out=output_stream,
                chrome=chrome,
                render=render,
                stream_out=output_stream if stream_json else None,
                session_id=chat_session_id,
            )
            session.turns.append(result)
            if render:
                _render_turn_footer(result.terminal, chrome)
            return session

        _run_repl(
            client,
            args,
            session,
            messages,
            input_stream=input_stream,
            output_stream=output_stream,
            chat_session_id=chat_session_id,
        )
        return session
    finally:
        client.close()
        if transcript is not None:
            transcript.close()


def _run_repl(
    client: _DaemonChatClient,
    args: Any,
    session: ChatSessionResult,
    messages: list[dict[str, str]],
    *,
    input_stream: TextIO,
    output_stream: TextIO,
    chat_session_id: str | None = None,
) -> None:
    _write("Kassiber chat. /help for commands, /exit to quit.\n", output_stream)
    session_allowed: set[str] = set()
    while True:
        try:
            line = _read_repl_line(input_stream, output_stream)
        except KeyboardInterrupt:
            _write("\n", output_stream)
            break
        if line == "":
            break
        prompt = line.strip()
        if not prompt:
            continue
        if prompt.startswith("/"):
            parts = prompt.split(None, 1)
            command = parts[0]
            arg = parts[1].strip() if len(parts) > 1 else ""
            if command in {"/exit", "/quit"}:
                break
            if command == "/help":
                _write(_REPL_HELP, output_stream)
            elif command == "/tools":
                _render_tool_listing(output_stream)
            elif command == "/model":
                _handle_model_command(args, arg, output_stream)
            elif command == "/provider":
                _handle_provider_command(client, args, arg, output_stream)
            elif command == "/allow":
                _handle_allow_command(arg, session_allowed, output_stream)
            elif command == "/allowed":
                _render_allowed(args, session_allowed, output_stream)
            elif command == "/new":
                messages.clear()
                chat_session_id = None
                _write("Started a new conversation.\n", output_stream)
            else:
                _write(
                    f"Unknown command {command}. /help lists commands.\n",
                    output_stream,
                )
            continue
        messages.append({"role": "user", "content": prompt})
        try:
            result = _run_turn(
                client,
                args,
                messages,
                stdin=input_stream,
                out=output_stream,
                chrome=output_stream,
                render=True,
                session_allowed=session_allowed,
                session_id=chat_session_id,
            )
        except AppError as exc:
            # Keep the REPL session (and its history) alive across
            # transient provider or daemon-request failures.
            if exc.code in {"daemon_closed", "daemon_protocol_error"}:
                raise
            messages.pop()
            _write(f"Error: {exc}\n", sys.stderr)
            if exc.hint:
                _write(f"Hint: {exc.hint}\n", sys.stderr)
            continue
        session.turns.append(result)
        terminal_data = (
            result.terminal.get("data")
            if isinstance(result.terminal.get("data"), dict)
            else {}
        )
        if isinstance(terminal_data.get("session_id"), str):
            chat_session_id = terminal_data["session_id"]
        _render_turn_footer(result.terminal, output_stream)
        if result.content:
            messages.append({"role": "assistant", "content": result.content})
