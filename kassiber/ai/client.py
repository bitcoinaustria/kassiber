"""OpenAI-compatible chat client over stdlib `urllib.request`.

Speaks one wire format — `/v1/chat/completions`, `/v1/models` — so the same
code path works against local Ollama (which exposes an OpenAI-compatible
endpoint), LM Studio, llama.cpp's server, vLLM, OpenAI itself, Maple AI,
OpenRouter, and similar.

Streaming uses Server-Sent Events. The SSE parser is split out from the
HTTP code so it's testable with synthetic inputs.

Errors from the network layer are mapped onto `AppError` codes the CLI/UI
already render through the standard envelope:

    ai_unavailable          — connection refused, DNS failure, 5xx, timeout
    ai_auth_failed          — 401
    ai_rate_limited         — 429 (retryable)
    ai_request_invalid      — 400 / other 4xx
    ai_provider_not_configured — caller did not supply a usable provider
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable, Iterator

import urllib.error
import urllib.request

from ..errors import AppError


DEFAULT_TIMEOUT_SECONDS = 120
DEFAULT_INACTIVITY_TIMEOUT_SECONDS = 90
SSE_DONE_SENTINEL = "[DONE]"
CLI_DEFAULT_MODEL = "default"
REASONING_EFFORTS = {"low", "medium", "high", "max"}
CLI_MODEL_CHECK_KIND = "binary_presence"
MODEL_SUPPORT_LIST_LIMIT = 32
MODEL_SUPPORT_STRING_LIMIT = 96
CLI_FALLBACK_DIRS = (
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "~/.local/bin",
    "/opt/local/bin",
)
CLI_MODEL_LIST_TIMEOUT_SECONDS = 10
CLAUDE_CLI_MODEL_ROWS = (
    {"id": CLI_DEFAULT_MODEL, "check_kind": CLI_MODEL_CHECK_KIND},
    {"id": "sonnet", "check_kind": "claude_cli_alias"},
    {"id": "opus", "check_kind": "claude_cli_alias"},
)


@dataclass(frozen=True)
class ChatDelta:
    """One streaming chunk from an OpenAI-compatible chat stream.

    `delta` mirrors the upstream OpenAI shape: `{role?, content?}`. `role` is
    typically only set on the first chunk. `content` is the additive token
    string. `finish_reason` is non-null only on the terminal chunk.
    """

    delta: dict[str, Any]
    finish_reason: str | None
    raw: dict[str, Any]


class ToolCallAccumulator:
    """Accumulate OpenAI-compatible streaming tool_call deltas.

    Providers may split a single function-call argument string across many
    chunks. This helper keeps the latest complete-ish snapshot per index so
    callers can inspect accumulated arguments without reimplementing the SSE
    merge rules.
    """

    def __init__(self) -> None:
        self._calls: dict[int, dict[str, Any]] = {}

    def add_delta(self, tool_call_deltas: object) -> list[dict[str, Any]]:
        if not isinstance(tool_call_deltas, list):
            return self.snapshot()
        for position, raw in enumerate(tool_call_deltas):
            if not isinstance(raw, dict):
                continue
            raw_index = raw.get("index", position)
            try:
                index = int(raw_index)
            except (TypeError, ValueError):
                index = position
            current = self._calls.setdefault(
                index,
                {
                    "id": None,
                    "type": "function",
                    "function": {"name": "", "arguments": ""},
                },
            )
            call_id = raw.get("id")
            if isinstance(call_id, str) and call_id:
                current["id"] = call_id
            call_type = raw.get("type")
            if isinstance(call_type, str) and call_type:
                current["type"] = call_type
            function = raw.get("function")
            if isinstance(function, dict):
                current_function = current.setdefault("function", {"name": "", "arguments": ""})
                name = function.get("name")
                if isinstance(name, str) and name:
                    current_function["name"] = name
                arguments = function.get("arguments")
                if isinstance(arguments, str):
                    current_function["arguments"] = (
                        str(current_function.get("arguments") or "") + arguments
                    )
        return self.snapshot()

    def snapshot(self) -> list[dict[str, Any]]:
        calls: list[dict[str, Any]] = []
        for index in sorted(self._calls):
            raw = self._calls[index]
            function = raw.get("function") if isinstance(raw.get("function"), dict) else {}
            calls.append(
                {
                    "id": raw.get("id") or f"call_{index}",
                    "type": raw.get("type") or "function",
                    "function": {
                        "name": function.get("name") or "",
                        "arguments": function.get("arguments") or "",
                    },
                }
            )
        return calls


def parse_sse_chunks(lines: Iterable[str]) -> Iterator[dict]:
    """Yield JSON objects from an iterator over SSE text lines.

    SSE grammar handled (per HTML living standard, narrowed to what
    OpenAI-compat servers actually emit):

    * Comments — lines starting with ``:`` are skipped.
    * Field lines — ``data:``, ``event:``, ``id:``, ``retry:``. Only ``data``
      is used; others are ignored.
    * Multi-line ``data:`` — joined with ``\\n`` per spec.
    * Empty line — event boundary; emits the accumulated ``data`` payload
      as a JSON object (or stops the iterator on the ``[DONE]`` sentinel).
    * Stray malformed JSON in a ``data:`` payload is skipped, not raised.
    """
    buffer: list[str] = []

    def take_event() -> tuple[bool, dict | None]:
        """Return ``(done, parsed)`` for the buffered event.

        ``done`` is True iff the buffered payload is the ``[DONE]`` sentinel
        and the caller should stop iterating. ``parsed`` is the decoded JSON
        object on success, or ``None`` if the buffer was empty / malformed.
        """
        if not buffer:
            return False, None
        payload = "\n".join(buffer)
        buffer.clear()
        if payload == SSE_DONE_SENTINEL:
            return True, None
        try:
            obj = json.loads(payload)
        except json.JSONDecodeError:
            return False, None
        if isinstance(obj, dict):
            return False, obj
        return False, None

    for raw_line in lines:
        line = raw_line.rstrip("\n").rstrip("\r")
        if not line:
            done, parsed = take_event()
            if parsed is not None:
                yield parsed
            if done:
                return
            continue
        if line.startswith(":"):
            continue
        if line.startswith("data:"):
            buffer.append(line[5:].lstrip(" "))
            continue
        # event:, id:, retry:, or unknown — ignore for our purposes.
    done, parsed = take_event()
    if parsed is not None:
        yield parsed


def _http_error_app_error(exc: urllib.error.HTTPError) -> AppError:
    status = exc.code
    body = ""
    try:
        body = (exc.read() or b"").decode("utf-8", errors="replace")
    except Exception:
        body = ""
    details = {"status": status}
    if body:
        details["body"] = body[:1024]
    if status == 401 or status == 403:
        return AppError(
            "AI provider rejected the request as unauthorized",
            code="ai_auth_failed",
            details=details,
            hint="Check the provider's API key in Settings → AI providers.",
            retryable=False,
        )
    if status == 429:
        return AppError(
            "AI provider is rate-limiting requests",
            code="ai_rate_limited",
            details=details,
            hint="Wait a few seconds and try again, or pick a different provider.",
            retryable=True,
        )
    if 500 <= status < 600:
        return AppError(
            f"AI provider returned a server error ({status})",
            code="ai_unavailable",
            details=details,
            hint="The provider may be temporarily unavailable. Try again or switch providers.",
            retryable=True,
        )
    return AppError(
        f"AI provider rejected the request ({status})",
        code="ai_request_invalid",
        details=details,
        hint="Check the model name and request shape.",
        retryable=False,
    )


def _network_error_app_error(exc: Exception) -> AppError:
    return AppError(
        "AI provider is unreachable",
        code="ai_unavailable",
        details={"reason": str(exc)},
        hint="Is the provider running? For local Ollama, try `ollama serve`. For remote providers, check connectivity.",
        retryable=True,
    )


def is_cli_provider_locator(base_url: str) -> bool:
    return base_url in {"claude-cli://default", "codex-cli://default"}


def _messages_to_prompt(messages: list[dict]) -> str:
    lines: list[str] = []
    for message in messages:
        role = str(message.get("role") or "user").strip() or "user"
        content = message.get("content")
        if not isinstance(content, str) or not content:
            continue
        lines.append(f"{role.upper()}:\n{content}")
    return "\n\n".join(lines).strip()


def _cli_unavailable(command: str) -> AppError:
    return AppError(
        f"AI CLI provider '{command}' is not installed or not on PATH",
        code="ai_unavailable",
        hint=f"Install and authenticate `{command}` before using this provider.",
        retryable=True,
    )


def _resolve_cli_executable(command: str) -> str | None:
    resolved = shutil.which(command)
    if resolved:
        return resolved

    for directory in CLI_FALLBACK_DIRS:
        candidate = Path(directory).expanduser() / command
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    return None


def _cli_default_model_row() -> dict[str, Any]:
    return {
        "id": CLI_DEFAULT_MODEL,
        "check_kind": CLI_MODEL_CHECK_KIND,
        "supports_reasoning_effort": True,
        "reasoning_efforts": ["low", "medium", "high"],
    }


def _codex_catalog_models(executable: str) -> list[dict[str, Any]]:
    try:
        completed = subprocess.run(
            [executable, "debug", "models"],
            text=True,
            capture_output=True,
            timeout=CLI_MODEL_LIST_TIMEOUT_SECONDS,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return []
    if completed.returncode != 0:
        return []

    try:
        payload = json.loads(completed.stdout or "{}")
    except json.JSONDecodeError:
        return []

    raw_models = payload.get("models")
    if not isinstance(raw_models, list):
        return []

    rows: list[dict[str, Any]] = []
    for item in raw_models:
        if not isinstance(item, dict):
            continue
        if item.get("visibility") != "list":
            continue
        slug = item.get("slug")
        if not isinstance(slug, str) or not slug.strip():
            continue
        raw_levels = item.get("supported_reasoning_levels")
        if not isinstance(raw_levels, list):
            raw_levels = []
        supported_efforts = _safe_string_list(
            [
                level.get("effort")
                for level in raw_levels
                if isinstance(level, dict)
            ]
        )
        row: dict[str, Any] = {
            "id": slug.strip(),
            "check_kind": "codex_model_catalog",
        }
        display_name = item.get("display_name")
        if isinstance(display_name, str) and display_name.strip():
            row["display_name"] = display_name.strip()[:MODEL_SUPPORT_STRING_LIMIT]
        if supported_efforts:
            row["supports_reasoning_effort"] = True
            row["reasoning_efforts"] = supported_efforts
        rows.append(row)
    return rows


def _cli_failure(command: str, completed: subprocess.CompletedProcess[str]) -> AppError:
    stderr = (completed.stderr or "").strip()
    stdout = (completed.stdout or "").strip()
    details: dict[str, Any] = {"exit_code": completed.returncode}
    if stderr:
        details["stderr"] = stderr[-2048:]
    if stdout:
        details["stdout"] = stdout[-2048:]
    return AppError(
        f"AI CLI provider '{command}' failed",
        code="ai_request_invalid",
        hint=(
            f"Run `{command} --help` or check that the CLI is authenticated. "
            "Prompts sent through Claude/Codex CLI may leave this device."
        ),
        details=details,
        retryable=False,
    )


def _reasoning_effort(options: dict[str, Any] | None) -> str | None:
    if not isinstance(options, dict):
        return None
    raw = options.get("reasoning_effort")
    if not isinstance(raw, str):
        return None
    effort = raw.strip().lower()
    if effort in REASONING_EFFORTS:
        return effort
    return None


def _safe_string_list(value: Any) -> list[str] | None:
    if not isinstance(value, list):
        return None
    out: list[str] = []
    for item in value:
        if not isinstance(item, str):
            continue
        text = item.strip()
        if not text:
            continue
        out.append(text[:MODEL_SUPPORT_STRING_LIMIT])
        if len(out) >= MODEL_SUPPORT_LIST_LIMIT:
            break
    return out or None


def _safe_capability_value(value: Any) -> bool | str | list[str] | None:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        text = value.strip()
        return text[:MODEL_SUPPORT_STRING_LIMIT] if text else None
    return _safe_string_list(value)


def _safe_model_capabilities(item: dict[str, Any]) -> dict[str, Any]:
    metadata: dict[str, Any] = {}
    supports_reasoning_effort = item.get("supports_reasoning_effort")
    if isinstance(supports_reasoning_effort, bool):
        metadata["supports_reasoning_effort"] = supports_reasoning_effort

    supported_parameters = _safe_string_list(item.get("supported_parameters"))
    if supported_parameters is not None:
        metadata["supported_parameters"] = supported_parameters

    reasoning_efforts = _safe_string_list(item.get("reasoning_efforts"))
    if reasoning_efforts is not None:
        metadata["reasoning_efforts"] = reasoning_efforts

    capabilities = item.get("capabilities")
    if isinstance(capabilities, dict):
        safe_capabilities: dict[str, Any] = {}
        for key in ("reasoning_effort", "reasoning_efforts", "supported_parameters"):
            raw = capabilities.get(key)
            safe = _safe_capability_value(raw)
            if safe is not None:
                safe_capabilities[key] = safe
        if safe_capabilities:
            metadata["capabilities"] = safe_capabilities
    return metadata


@dataclass
class OpenAICompatClient:
    """Minimal OpenAI-compatible HTTP client.

    Construct one per chat call (cheap). `base_url` should be the
    OpenAI-compat root including ``/v1`` (e.g. ``http://localhost:11434/v1``);
    a trailing slash is stripped.
    """

    base_url: str
    api_key: str | None = None
    timeout: float = DEFAULT_TIMEOUT_SECONDS
    user_agent: str = "kassiber/ai"

    def _headers(self, *, json_body: bool = False, accept_sse: bool = False) -> dict[str, str]:
        headers = {"Accept": "application/json", "User-Agent": self.user_agent}
        if json_body:
            headers["Content-Type"] = "application/json"
        if accept_sse:
            headers["Accept"] = "text/event-stream"
        if self.api_key:
            headers["Authorization"] = f"Bearer {self.api_key}"
        return headers

    def _open(
        self,
        path: str,
        *,
        method: str,
        body: bytes | None,
        accept_sse: bool,
        timeout: float | None = None,
    ):
        url = f"{self.base_url.rstrip('/')}/{path.lstrip('/')}"
        request = urllib.request.Request(
            url,
            data=body,
            method=method,
            headers=self._headers(json_body=body is not None, accept_sse=accept_sse),
        )
        try:
            return urllib.request.urlopen(
                request,
                timeout=timeout if timeout is not None else self.timeout,
            )
        except urllib.error.HTTPError as exc:
            raise _http_error_app_error(exc) from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise _network_error_app_error(exc) from exc

    def list_models(self, *, strict: bool = False) -> list[dict]:
        """`GET /v1/models`.

        Default mode (``strict=False``) is forgiving: a 4xx response is
        treated as "this provider doesn't expose `/v1/models`" and returns
        ``[]`` so the picker can fall back to the configured default
        model. Some OpenAI-compatible servers (small llama.cpp builds,
        custom proxies) skip the `/models` endpoint entirely.
        """
        try:
            response = self._open("models", method="GET", body=None, accept_sse=False)
        except AppError as exc:
            if exc.code == "ai_request_invalid" and not strict:
                return []
            raise
        with response:
            try:
                payload = json.loads(response.read().decode("utf-8", errors="replace"))
            except json.JSONDecodeError:
                if strict:
                    raise AppError(
                        "AI provider returned a non-JSON models response",
                        code="ai_request_invalid",
                        hint="Check that the base URL points at an OpenAI-compatible /v1 endpoint.",
                        retryable=False,
                    )
                return []
        data = payload.get("data") if isinstance(payload, dict) else None
        if not isinstance(data, list):
            if strict:
                raise AppError(
                    "AI provider models response did not contain a data list",
                    code="ai_request_invalid",
                    details={"shape": type(payload).__name__},
                    hint="Check that the base URL points at an OpenAI-compatible /v1 endpoint.",
                    retryable=False,
                )
            return []
        models: list[dict] = []
        for item in data:
            if not isinstance(item, dict):
                continue
            model_id = item.get("id")
            if not isinstance(model_id, str):
                continue
            row: dict[str, Any] = {"id": model_id}
            owned_by = item.get("owned_by")
            if isinstance(owned_by, str):
                row["owned_by"] = owned_by
            row.update(_safe_model_capabilities(item))
            models.append(row)
        return models

    def chat(
        self,
        *,
        messages: list[dict],
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Non-streaming `POST /v1/chat/completions`. Returns the assistant message."""
        body = dict(options or {})
        if tools is not None:
            body["tools"] = tools
        if tool_choice is not None:
            body["tool_choice"] = tool_choice
        body.update({"model": model, "messages": messages, "stream": False})
        response = self._open(
            "chat/completions",
            method="POST",
            body=json.dumps(body).encode("utf-8"),
            accept_sse=False,
        )
        with response:
            try:
                payload = json.loads(response.read().decode("utf-8", errors="replace"))
            except json.JSONDecodeError as exc:
                raise AppError(
                    "AI provider returned a non-JSON response",
                    code="ai_request_invalid",
                    details={"reason": str(exc)},
                ) from exc
        choice = ((payload.get("choices") or [{}])[0]) if isinstance(payload, dict) else {}
        message = (choice.get("message") or {}) if isinstance(choice, dict) else {}
        reasoning = message.get("reasoning")
        result: dict[str, Any] = {
            "role": message.get("role") or "assistant",
            "content": message.get("content") or "",
            "finish_reason": choice.get("finish_reason"),
            "usage": payload.get("usage") if isinstance(payload, dict) else None,
        }
        if isinstance(reasoning, str) and reasoning:
            result["reasoning"] = reasoning
        tool_calls = message.get("tool_calls")
        if isinstance(tool_calls, list):
            result["tool_calls"] = [
                tool_call for tool_call in tool_calls if isinstance(tool_call, dict)
            ]
        return result

    def stream_chat(
        self,
        *,
        messages: list[dict],
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
    ) -> Iterator[ChatDelta]:
        """Streaming `POST /v1/chat/completions`. Yields one ChatDelta per SSE chunk."""
        body = dict(options or {})
        if tools is not None:
            body["tools"] = tools
        if tool_choice is not None:
            body["tool_choice"] = tool_choice
        body.update({"model": model, "messages": messages, "stream": True})
        response = self._open(
            "chat/completions",
            method="POST",
            body=json.dumps(body).encode("utf-8"),
            accept_sse=True,
            timeout=DEFAULT_INACTIVITY_TIMEOUT_SECONDS,
        )
        try:
            with response:
                tool_call_accumulator = ToolCallAccumulator()
                line_iter = (raw.decode("utf-8", errors="replace") for raw in response)
                for chunk in parse_sse_chunks(line_iter):
                    choices = chunk.get("choices") if isinstance(chunk, dict) else None
                    if not isinstance(choices, list) or not choices:
                        continue
                    choice = choices[0] if isinstance(choices[0], dict) else {}
                    delta = choice.get("delta")
                    if not isinstance(delta, dict):
                        delta = {}
                    elif isinstance(delta.get("tool_calls"), list):
                        delta = dict(delta)
                        delta["tool_calls"] = tool_call_accumulator.add_delta(
                            delta.get("tool_calls")
                        )
                    yield ChatDelta(
                        delta=delta,
                        finish_reason=choice.get("finish_reason"),
                        raw=chunk,
                    )
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise _network_error_app_error(exc) from exc


@dataclass
class CliAIClient:
    """Fixed adapter for Claude Code and Codex CLI providers.

    This is intentionally narrow: Kassiber sends a single non-interactive
    prompt over stdin, uses an isolated temporary cwd, and asks the CLIs not
    to persist sessions. These CLIs may still call their vendor or configured
    model provider, so callers must keep the normal off-device acknowledgement
    gate.
    """

    locator: str
    timeout: float = DEFAULT_TIMEOUT_SECONDS

    @property
    def command(self) -> str:
        if self.locator == "claude-cli://default":
            return "claude"
        if self.locator == "codex-cli://default":
            return "codex"
        raise AppError(
            f"Unsupported AI CLI provider locator '{self.locator}'",
            code="validation",
            hint="Use claude-cli://default or codex-cli://default.",
        )

    def list_models(self, *, strict: bool = False) -> list[dict]:
        del strict
        executable = _resolve_cli_executable(self.command)
        if not executable:
            raise _cli_unavailable(self.command)
        if self.command == "codex":
            return _codex_catalog_models(executable) or [_cli_default_model_row()]
        return [dict(row) for row in CLAUDE_CLI_MODEL_ROWS]

    def _claude_args(
        self,
        *,
        command: str | None = None,
        model: str,
        effort: str | None,
    ) -> list[str]:
        args = [
            command or self.command,
            "--print",
            "--no-session-persistence",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "",
            "--output-format",
            "json",
        ]
        if model and model != CLI_DEFAULT_MODEL:
            args.extend(["--model", model])
        if effort:
            args.extend(["--effort", effort])
        return args

    def _codex_args(
        self,
        *,
        command: str | None = None,
        cwd: str,
        output_path: str,
        model: str,
        effort: str | None,
    ) -> list[str]:
        args = [
            command or self.command,
            "exec",
            "--sandbox",
            "read-only",
            "--cd",
            cwd,
            "--skip-git-repo-check",
            "--ephemeral",
            "--ignore-rules",
            "--color",
            "never",
            "--output-last-message",
            output_path,
        ]
        if model and model != CLI_DEFAULT_MODEL:
            args.extend(["--model", model])
        if effort:
            args.extend(["-c", f'model_reasoning_effort="{effort}"'])
        args.append("-")
        return args

    def _run(
        self,
        *,
        prompt: str,
        model: str,
        options: dict[str, Any] | None = None,
    ) -> str:
        command = self.command
        executable = _resolve_cli_executable(command)
        if not executable:
            raise _cli_unavailable(command)
        env = dict(os.environ)
        env.setdefault("NO_COLOR", "1")
        effort = _reasoning_effort(options)
        with tempfile.TemporaryDirectory(prefix="kassiber-ai-cli-") as cwd:
            if command == "claude":
                args = self._claude_args(command=executable, model=model, effort=effort)
                completed = subprocess.run(
                    args,
                    input=prompt,
                    text=True,
                    capture_output=True,
                    cwd=cwd,
                    env=env,
                    timeout=self.timeout,
                    check=False,
                )
                if completed.returncode != 0:
                    raise _cli_failure(command, completed)
                try:
                    payload = json.loads(completed.stdout or "{}")
                except json.JSONDecodeError:
                    return (completed.stdout or "").strip()
                result = payload.get("result") if isinstance(payload, dict) else None
                if isinstance(result, str):
                    return result.strip()
                return (completed.stdout or "").strip()

            with tempfile.NamedTemporaryFile("r", encoding="utf-8", delete=False) as output:
                output_path = output.name
            try:
                args = self._codex_args(
                    command=executable,
                    cwd=cwd,
                    output_path=output_path,
                    model=model,
                    effort=effort,
                )
                completed = subprocess.run(
                    args,
                    input=prompt,
                    text=True,
                    capture_output=True,
                    cwd=cwd,
                    env=env,
                    timeout=self.timeout,
                    check=False,
                )
                if completed.returncode != 0:
                    raise _cli_failure(command, completed)
                try:
                    with open(output_path, "r", encoding="utf-8") as handle:
                        content = handle.read().strip()
                except OSError:
                    content = ""
                return content or (completed.stdout or "").strip()
            finally:
                try:
                    os.unlink(output_path)
                except OSError:
                    pass

    def chat(
        self,
        *,
        messages: list[dict],
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        del tools, tool_choice
        content = self._run(prompt=_messages_to_prompt(messages), model=model, options=options)
        return {
            "role": "assistant",
            "content": content,
            "finish_reason": "stop",
            "usage": None,
        }

    def stream_chat(
        self,
        *,
        messages: list[dict],
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
    ) -> Iterator[ChatDelta]:
        response = self.chat(
            messages=messages,
            model=model,
            options=options,
            tools=tools,
            tool_choice=tool_choice,
        )
        yield ChatDelta(
            delta={"role": "assistant", "content": response["content"]},
            finish_reason=response.get("finish_reason"),
            raw={"provider": self.command},
        )


def ai_client_for_locator(
    base_url: str,
    *,
    api_key: str | None = None,
    timeout: float = DEFAULT_TIMEOUT_SECONDS,
):
    if is_cli_provider_locator(base_url):
        return CliAIClient(locator=base_url, timeout=timeout)
    return OpenAICompatClient(base_url=base_url, api_key=api_key, timeout=timeout)
