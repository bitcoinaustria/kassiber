from __future__ import annotations

import json
import os
import subprocess
import sys
import uuid
from dataclasses import dataclass, field
from typing import Any, Iterable, TextIO

from ..ai.client import CLI_DEFAULT_MODEL, is_cli_provider_locator
from ..ai.tools import TOOL_CATALOG
from ..errors import AppError


_CONSENT_DECISIONS = {"allow_once", "allow_session", "deny"}
_ANSI_DIM = "\x1b[2m"
_ANSI_RESET = "\x1b[0m"
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
    "  /help   show this help\n"
    "  /tools  list daemon AI tools and their consent class\n"
    "  /exit   leave the chat (also /quit or Ctrl-D)\n"
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
            "tool_calls": last.tool_calls if last else [],
        }


class _DaemonChatClient:
    def __init__(self, args: Any) -> None:
        self._pass_fds: tuple[int, ...] = ()
        self._duplicated_fd: int | None = None
        command = self._daemon_command(args)
        self._proc = subprocess.Popen(
            command,
            cwd=os.getcwd(),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            pass_fds=self._pass_fds,
        )
        if self._duplicated_fd is not None:
            os.close(self._duplicated_fd)
            self._duplicated_fd = None
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
        passphrase_fd = getattr(args, "db_passphrase_fd", None)
        if passphrase_fd is not None:
            self._duplicated_fd = os.dup(passphrase_fd)
            self._pass_fds = (self._duplicated_fd,)
            command.extend(["--db-passphrase-fd", str(self._duplicated_fd)])
        command.append("daemon")
        return command

    def send(self, payload: dict[str, Any]) -> None:
        if self._proc.stdin is None:
            raise AppError(
                "daemon input stream is closed",
                code="daemon_closed",
                retryable=False,
            )
        self._proc.stdin.write(json.dumps(payload, separators=(",", ":")) + "\n")
        self._proc.stdin.flush()

    def read(self) -> dict[str, Any]:
        if self._proc.stdout is None:
            raise AppError(
                "daemon output stream is closed",
                code="daemon_closed",
                retryable=False,
            )
        line = self._proc.stdout.readline()
        if line == "":
            stderr = self._proc.stderr.read() if self._proc.stderr is not None else ""
            raise AppError(
                "daemon exited before chat completed",
                code="daemon_closed",
                details={"stderr": stderr[-2000:]},
                retryable=False,
            )
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


def _build_chat_args(args: Any, messages: list[dict[str, str]]) -> dict[str, Any]:
    tools_enabled = not getattr(args, "no_tools", False)
    payload: dict[str, Any] = {
        "provider": getattr(args, "provider", None),
        "model": args.model,
        "messages": messages,
        "tools_enabled": tools_enabled,
        "tool_loop_max_iterations": getattr(args, "tool_loop_max_iterations", 8),
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
    if getattr(args, "format", None) == "json" or getattr(args, "stream_json", False):
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
) -> ChatTurnResult:
    request_id = f"chat-{uuid.uuid4().hex}"
    client.send(
        {
            "request_id": request_id,
            "kind": "ai.chat",
            "args": _build_chat_args(args, messages),
        }
    )
    content_parts: list[str] = []
    tool_calls: dict[str, dict[str, Any]] = {}
    control_requests: set[str] = set()
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
                    if render:
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
            elif kind == "ai.chat":
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
    if one_shot_prompt is None and not input_stream.isatty():
        raise AppError(
            "interactive chat requires a TTY; pass a prompt for one-shot mode",
            code="validation",
            hint="Use `kassiber chat \"...\"` or `kassiber chat --prompt \"...\"`.",
            retryable=False,
        )

    client = _DaemonChatClient(args)
    try:
        _resolve_default_model(client, args)
        session = ChatSessionResult(
            provider=getattr(args, "provider", None),
            model=args.model,
        )
        messages: list[dict[str, str]] = []
        render = not machine and not stream_json
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
            )
            session.turns.append(result)
            if render:
                _render_turn_footer(result.terminal, chrome)
            return session

        _write("Kassiber chat. /help for commands, /exit to quit.\n", output_stream)
        session_allowed: set[str] = set()
        while True:
            try:
                _write("> ", output_stream)
                prompt = input_stream.readline()
            except KeyboardInterrupt:
                _write("\n", output_stream)
                break
            if prompt == "":
                break
            prompt = prompt.strip()
            if not prompt:
                continue
            if prompt in {"/exit", "/quit"}:
                break
            if prompt == "/help":
                _write(_REPL_HELP, output_stream)
                continue
            if prompt == "/tools":
                _render_tool_listing(output_stream)
                continue
            if prompt.startswith("/"):
                _write(f"Unknown command {prompt}. /help lists commands.\n", output_stream)
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
            _render_turn_footer(result.terminal, output_stream)
            if result.content:
                messages.append({"role": "assistant", "content": result.content})
        return session
    finally:
        client.close()
