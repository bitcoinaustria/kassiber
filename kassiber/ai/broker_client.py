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
from .contracts import ChatDelta, DEFAULT_TIMEOUT_SECONDS, cli_provider_for_locator


_NODE_FALLBACKS = (
    "/opt/homebrew/bin/node",
    "/usr/local/bin/node",
    "~/.local/bin/node",
    "/opt/local/bin/node",
)


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
                input=json.dumps(request),
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

    def cancel(self) -> None:
        with self._lock:
            process = self._process
        if process is None or process.poll() is not None:
            return
        try:
            if os.name != "nt":
                os.killpg(process.pid, signal.SIGTERM)
            else:
                process.terminate()
        except (OSError, ProcessLookupError):
            return

    def stream_chat(
        self,
        *,
        messages: list[dict],
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
        **_: Any,
    ) -> Iterator[ChatDelta]:
        if tools or tool_choice not in (None, "none"):
            raise AppError(
                "Kassiber tools are unavailable for local CLI broker providers",
                code="ai_cli_tools_disabled",
                hint="Choose an HTTP provider when typed Kassiber tools are required.",
                retryable=False,
            )
        node = _node_executable()
        script = _broker_script()
        if not node or not script.is_file():
            raise _broker_unavailable()
        safe_options = {
            key: value
            for key, value in (options or {}).items()
            if key in {"reasoning_effort", "provider_session_id"}
        }
        request = {
            "command": "chat",
            "request_id": "daemon-chat",
            "provider": self.provider,
            "model": model,
            "messages": [
                {
                    "role": str(message.get("role") or "user"),
                    "content": str(message.get("content") or ""),
                }
                for message in messages
                if isinstance(message, dict)
            ],
            "options": safe_options,
        }
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
        try:
            assert process.stdin is not None
            assert process.stdout is not None
            process.stdin.write(json.dumps(request))
            process.stdin.close()
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
                    if delta:
                        yield ChatDelta(delta=delta, finish_reason=None, raw={"provider": self.provider})
                elif event_type == "done":
                    session_id = event.get("provider_session_id")
                    self.last_provider_session_id = (
                        session_id if isinstance(session_id, str) and session_id else None
                    )
                    yield ChatDelta(
                        delta={},
                        finish_reason=str(event.get("finish_reason") or "stop"),
                        raw={"provider": self.provider},
                    )
                    return
            if process.returncode not in (None, 0):
                raise AppError(
                    "The CLI provider stopped before completing the response",
                    code="ai_unavailable",
                    retryable=True,
                )
        finally:
            with self._lock:
                if self._process is process:
                    self._process = None
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            if process.stdout is not None:
                process.stdout.close()
