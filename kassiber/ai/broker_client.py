"""Supervise the bundled chat-only Node provider broker."""

from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import threading
from pathlib import Path
from typing import Any, Iterator

from ..errors import AppError
from .contracts import (
    ChatDelta,
    DEFAULT_TIMEOUT_SECONDS,
    ResponsesRequestContext,
    cli_provider_for_locator,
)


_NODE_FALLBACKS = (
    "/opt/homebrew/bin/node",
    "/usr/local/bin/node",
    "~/.local/bin/node",
    "/opt/local/bin/node",
)


def _content_text(content: object) -> str:
    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return ""
    return "\n".join(
        str(part.get("text"))
        for part in content
        if isinstance(part, dict)
        and part.get("type") in {"input_text", "output_text", "text"}
        and isinstance(part.get("text"), str)
    )


def _broker_messages_for_context(
    context: ResponsesRequestContext,
) -> list[dict[str, str]]:
    messages: list[dict[str, str]] = []
    for item in context.input_items:
        if item.get("type") != "message":
            continue
        role = str(item.get("role") or "user")
        content = _content_text(item.get("content"))
        if content:
            messages.append({"role": role, "content": content})
    return messages


def _broker_tool_definitions(tools: list[dict[str, Any]]) -> list[dict[str, Any]]:
    from .tools import get_tool

    definitions: list[dict[str, Any]] = []
    for tool in tools:
        function = tool.get("function") if isinstance(tool, dict) else None
        if not isinstance(function, dict) and isinstance(tool, dict):
            function = tool
        if not isinstance(function, dict) or not isinstance(function.get("name"), str):
            continue
        entry = get_tool(function["name"])
        definitions.append(
            {
                "name": function["name"],
                "description": function.get("description") or "",
                "parameters": function.get("parameters") or {"type": "object"},
                "read_only": entry is not None and entry.kind_class == "read_only",
                "destructive": entry is not None and entry.kind_class == "mutating",
            }
        )
    return definitions


def _node_executable() -> str | None:
    configured = os.environ.get("KASSIBER_AI_BROKER_NODE")
    if configured:
        candidate = Path(configured).expanduser()
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    resolved = shutil.which("node")
    if resolved:
        return resolved
    for raw in _NODE_FALLBACKS:
        candidate = Path(raw).expanduser()
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    return None


def _broker_script() -> Path:
    configured = os.environ.get("KASSIBER_AI_PROVIDER_BROKER")
    if configured:
        return Path(configured).expanduser()
    return Path(__file__).with_name("provider_broker") / "index.mjs"


def _broker_unavailable() -> AppError:
    return AppError(
        "The local AI provider broker is unavailable",
        code="ai_unavailable",
        hint="Install Node.js 20 or newer, or use an HTTP/Ollama/oMLX provider.",
        retryable=True,
    )


def _broker_event_error(event: dict[str, Any]) -> AppError:
    code = event.get("code")
    if code == "authentication_required":
        return AppError(
            "The selected CLI provider requires authentication",
            code="ai_auth_failed",
            hint=str(event.get("hint") or "Run the provider's normal login command outside Kassiber."),
            retryable=False,
        )
    if code == "missing_executable":
        return AppError(
            "The selected CLI provider is not installed",
            code="ai_unavailable",
            hint="Install the provider CLI and authenticate it outside Kassiber.",
            retryable=True,
        )
    return AppError(
        str(event.get("message") or "The CLI provider request failed."),
        code="ai_request_invalid",
        hint="Check the provider status in the model picker.",
        retryable=False,
    )


class BrokerAIClient:
    """Provider-neutral adapter over the bundled JSONL broker."""

    def __init__(
        self,
        *,
        locator: str,
        timeout: float = DEFAULT_TIMEOUT_SECONDS,
    ) -> None:
        normalized = locator.strip().lower()
        provider = cli_provider_for_locator(normalized)
        if provider is None:
            raise AppError(
                f"Unsupported CLI AI provider locator '{locator}'",
                code="validation",
                retryable=False,
            )
        self.locator = normalized
        self.provider = provider
        self.timeout = timeout
        self.last_provider_session_id: str | None = None
        self._lock = threading.Lock()
        self._process: subprocess.Popen[str] | None = None
        self._pending_call_ids: set[str] = set()

    @staticmethod
    def runtime_status() -> list[dict[str, Any]]:
        event = BrokerAIClient._single_result({"command": "status"}, timeout=35.0)
        return event if isinstance(event, list) else []

    @staticmethod
    def _single_result(request: dict[str, Any], *, timeout: float) -> Any:
        node = _node_executable()
        script = _broker_script()
        if not node or not script.is_file():
            raise _broker_unavailable()
        try:
            completed = subprocess.run(
                [node, str(script)],
                input=json.dumps(request) + "\n",
                text=True,
                capture_output=True,
                timeout=timeout,
                check=False,
                start_new_session=os.name != "nt",
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise _broker_unavailable() from exc
        for line in completed.stdout.splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not isinstance(event, dict):
                continue
            if event.get("type") == "error":
                raise _broker_event_error(event)
            if event.get("type") == "result":
                return event.get("data")
        raise AppError(
            "The local AI provider broker returned no result",
            code="ai_unavailable",
            retryable=True,
        )

    def list_models(self, *, strict: bool = False) -> list[dict[str, Any]]:
        del strict
        result = self._single_result(
            {"command": "models", "provider": self.provider},
            timeout=min(self.timeout, 35.0),
        )
        return result if isinstance(result, list) else []

    @staticmethod
    def _signal_group(process: subprocess.Popen[str], sig: int) -> None:
        """Signal the whole broker process group.

        The broker spawns provider CLIs, which spawn their own children. Node is
        started with `start_new_session`, so signalling only its pid leaves those
        grandchildren running — a provider CLI can keep a model request alive
        after Kassiber believes the turn is over.
        """

        try:
            if os.name != "nt":
                os.killpg(os.getpgid(process.pid), sig)
            else:
                process.terminate()
        except (OSError, ProcessLookupError):
            return

    def cancel(self) -> None:
        with self._lock:
            process = self._process
        if process is None or process.poll() is not None:
            return
        self._signal_group(process, signal.SIGTERM)

    def stream_chat(
        self,
        *,
        messages: list[dict] | None = None,
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
        context: ResponsesRequestContext | None = None,
    ) -> Iterator[ChatDelta]:
        del tool_choice
        node = _node_executable()
        script = _broker_script()
        if not node or not script.is_file():
            raise _broker_unavailable()
        with self._lock:
            process = self._process
        continuing = (
            process is not None
            and process.poll() is None
            and bool(self._pending_call_ids)
        )
        if not continuing:
            self._pending_call_ids.clear()
            if process is not None and process.poll() is None:
                self._signal_group(process, signal.SIGTERM)
            try:
                process = subprocess.Popen(
                    [node, str(script)],
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                    text=True,
                    start_new_session=os.name != "nt",
                )
            except OSError as exc:
                raise _broker_unavailable() from exc
            with self._lock:
                self._process = process
            safe_options = {
                key: value
                for key, value in (options or {}).items()
                if key in {"reasoning_effort", "provider_session_id"}
            }
            broker_messages = (
                _broker_messages_for_context(context)
                if context is not None
                else [
                    {
                        "role": str(message.get("role") or "user"),
                        "content": str(message.get("content") or ""),
                    }
                    for message in (messages or [])
                    if isinstance(message, dict)
                ]
            )
            request = {
                "command": "chat",
                "request_id": "daemon-chat",
                "provider": self.provider,
                "model": model,
                "messages": broker_messages,
                "instructions": context.instructions if context is not None else None,
                "tools": _broker_tool_definitions(tools or []),
                "options": safe_options,
            }
            outbound = request
        else:
            assert context is not None
            results = [
                {
                    "call_id": str(item.get("call_id")),
                    "output": str(item.get("output") or ""),
                }
                for item in context.input_items
                if item.get("type") == "function_call_output"
                and item.get("call_id") in self._pending_call_ids
            ]
            if {result["call_id"] for result in results} != self._pending_call_ids:
                raise AppError(
                    "Kassiber did not provide every pending CLI tool result",
                    code="ai_request_invalid",
                    retryable=False,
                )
            self._pending_call_ids.clear()
            outbound = {"command": "tool_results", "results": results}

        assert process is not None
        # `for line in process.stdout` blocks with no deadline of its own, so a
        # provider that stops producing output would pin a daemon worker forever.
        # A timer kills the whole group instead, which closes stdout and ends the
        # read; `timed_out` distinguishes that from an ordinary early exit.
        timed_out = threading.Event()

        def _on_timeout() -> None:
            timed_out.set()
            self._signal_group(process, signal.SIGKILL)

        watchdog = threading.Timer(max(self.timeout, 1.0), _on_timeout)
        watchdog.daemon = True
        watchdog.start()
        saw_terminal = False
        keep_alive = False
        buffered_content: list[str] = []
        try:
            assert process.stdin is not None
            assert process.stdout is not None
            process.stdin.write(json.dumps(outbound) + "\n")
            process.stdin.flush()
            for line in process.stdout:
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if not isinstance(event, dict):
                    continue
                event_type = event.get("type")
                if event_type == "error":
                    raise _broker_event_error(event)
                if event_type == "delta":
                    delta = {
                        key: event[key]
                        for key in ("content", "reasoning")
                        if isinstance(event.get(key), str) and event[key]
                    }
                    content = delta.get("content")
                    if isinstance(content, str):
                        buffered_content.append(content)
                    if delta:
                        yield ChatDelta(
                            delta=delta,
                            finish_reason=None,
                            raw={"provider": self.provider},
                        )
                elif event_type == "tool_call":
                    call_id = event.get("call_id")
                    name = event.get("name")
                    arguments = event.get("arguments")
                    if (
                        not isinstance(call_id, str)
                        or not call_id
                        or not isinstance(name, str)
                        or not name
                        or not isinstance(arguments, dict)
                    ):
                        raise AppError(
                            "The CLI provider returned an invalid tool call",
                            code="ai_request_invalid",
                            retryable=False,
                        )
                    self._pending_call_ids.add(call_id)
                    encoded_arguments = json.dumps(
                        arguments,
                        sort_keys=True,
                        separators=(",", ":"),
                    )
                    response_output: list[dict[str, Any]] = []
                    content = "".join(buffered_content)
                    if content:
                        response_output.append(
                            {
                                "type": "message",
                                "role": "assistant",
                                "content": [{"type": "output_text", "text": content}],
                            }
                        )
                    response_output.append(
                        {
                            "type": "function_call",
                            "call_id": call_id,
                            "name": name,
                            "arguments": encoded_arguments,
                        }
                    )
                    keep_alive = True
                    saw_terminal = True
                    yield ChatDelta(
                        delta={
                            "tool_calls": [
                                {
                                    "id": call_id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": encoded_arguments,
                                    },
                                }
                            ]
                        },
                        finish_reason="tool_calls",
                        raw={"provider": self.provider},
                        response_output=response_output,
                    )
                    return
                elif event_type == "done":
                    saw_terminal = True
                    session_id = event.get("provider_session_id")
                    self.last_provider_session_id = (
                        session_id if isinstance(session_id, str) and session_id else None
                    )
                    finish_reason = str(event.get("finish_reason") or "stop")
                    yield ChatDelta(
                        delta={},
                        finish_reason=finish_reason,
                        raw={"provider": self.provider},
                    )
                    return
            # EOF without a terminal `done` event is a failure, whatever the
            # exit status. `returncode` is still None here until the process is
            # reaped, so testing it alone treated a crashed or silent broker as a
            # complete answer and emitted finish_reason=null downstream.
            if not saw_terminal:
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    pass
                if timed_out.is_set():
                    raise AppError(
                        "The CLI provider stopped responding",
                        code="ai_unavailable",
                        retryable=True,
                    )
                raise AppError(
                    "The CLI provider stopped before completing the response",
                    code="ai_unavailable",
                    retryable=True,
                )
        finally:
            watchdog.cancel()
            if not keep_alive:
                self._pending_call_ids.clear()
                with self._lock:
                    if self._process is process:
                        self._process = None
                if process.poll() is None:
                    self._signal_group(process, signal.SIGTERM)
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    self._signal_group(process, signal.SIGKILL)
                    process.wait()
                if process.stdin is not None:
                    process.stdin.close()
                if process.stdout is not None:
                    process.stdout.close()
