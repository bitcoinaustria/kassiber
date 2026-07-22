"""OpenAI Responses API client over stdlib `urllib.request`.

HTTP providers speak ``POST /v1/responses`` for inference and ``GET /v1/models``
for discovery. The client keeps Kassiber's daemon-facing chat-message contract
small by translating it to Responses input Items at the transport boundary.
Current Ollama, oMLX, OpenAI, and OpenRouter endpoints implement this surface.

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
import urllib.parse
import urllib.request

from ..egress_ledger import get_egress_ledger, http_request_bytes_out
from ..errors import AppError
from ..redaction import provider_error_body_preview


DEFAULT_TIMEOUT_SECONDS = 120
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
_CLI_BASE_ENV_NAMES = {
    "HOME",
    "LANG",
    "LC_ALL",
    "PATH",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "TERM",
    "TMPDIR",
    "USER",
    "USERNAME",
}
_CLI_PROXY_ENV_NAMES = {
    "ALL_PROXY",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "all_proxy",
    "https_proxy",
    "http_proxy",
    "no_proxy",
}


def _url_origin(url: str) -> tuple[str, str, int | None]:
    parsed = urllib.parse.urlsplit(url)
    port = parsed.port
    if port is None:
        port = 443 if parsed.scheme.lower() == "https" else 80
    return parsed.scheme.lower(), (parsed.hostname or "").lower(), port


class _SameOriginRedirectHandler(urllib.request.HTTPRedirectHandler):
    """Allow redirects only within the configured direct provider origin."""

    def __init__(self, origin_url: str):
        super().__init__()
        self._origin = _url_origin(origin_url)

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if _url_origin(newurl) != self._origin:
            raise AppError(
                "Direct local AI provider attempted an off-origin redirect",
                code="ai_request_invalid",
                retryable=False,
            )
        return super().redirect_request(req, fp, code, msg, headers, newurl)


_CLAUDE_ENV_NAMES = {
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_VERTEX_PROJECT_ID",
    "ANTHROPIC_VERTEX_REGION",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLOUD_ML_REGION",
    "CLOUDSDK_CORE_PROJECT",
    "GCLOUD_PROJECT",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_QUOTA_PROJECT",
}
_CLAUDE_ENV_PREFIXES = ("AWS_",)
_CODEX_ENV_NAMES = {
    "CODEX_ACCESS_TOKEN",
    "CODEX_API_KEY",
    "CODEX_HOME",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_ORGANIZATION",
    "OPENAI_PROJECT",
}
CLAUDE_CLI_MODEL_ROWS = (
    {"id": CLI_DEFAULT_MODEL, "check_kind": CLI_MODEL_CHECK_KIND},
    {"id": "sonnet", "check_kind": "claude_cli_alias"},
    {"id": "opus", "check_kind": "claude_cli_alias"},
)


@dataclass(frozen=True)
class ChatDelta:
    """One normalized chunk from a Responses API semantic event stream.

    ``delta`` keeps the existing daemon shape (``content``, ``reasoning``, and
    normalized ``tool_calls``). The terminal event also carries the complete
    typed ``response_output`` so the tool loop can replay reasoning and tool
    Items without flattening them back into chat messages.
    """

    delta: dict[str, Any]
    finish_reason: str | None
    raw: dict[str, Any]
    response_output: list[dict[str, Any]] | None = None


class ResponsesToolCallAccumulator:
    """Accumulate Responses ``function_call`` semantic stream events.

    The daemon still consumes the familiar nested function-call shape. Keeping
    that normalization here lets the rest of Kassiber remain provider-agnostic
    while the wire format uses typed Responses Items.
    """

    def __init__(self) -> None:
        self._calls: dict[int, dict[str, Any]] = {}

    @staticmethod
    def _index(event: dict[str, Any]) -> int:
        raw_index = event.get("output_index", 0)
        try:
            return int(raw_index)
        except (TypeError, ValueError):
            return 0

    def _merge_item(self, index: int, item: object, *, replace_arguments: bool) -> None:
        if not isinstance(item, dict) or item.get("type") != "function_call":
            return
        current = self._calls.setdefault(
            index,
            {
                "id": None,
                "type": "function",
                "function": {"name": "", "arguments": ""},
            },
        )
        call_id = item.get("call_id") or item.get("id")
        if isinstance(call_id, str) and call_id:
            current["id"] = call_id
        name = item.get("name")
        if isinstance(name, str) and name:
            current["function"]["name"] = name
        arguments = item.get("arguments")
        if isinstance(arguments, str):
            if replace_arguments:
                current["function"]["arguments"] = arguments
            elif arguments:
                current["function"]["arguments"] += arguments

    def add_event(self, event: object) -> list[dict[str, Any]]:
        if not isinstance(event, dict):
            return self.snapshot()
        event_type = event.get("type")
        index = self._index(event)
        if event_type in {"response.output_item.added", "response.output_item.done"}:
            self._merge_item(
                index,
                event.get("item"),
                replace_arguments=event_type == "response.output_item.done",
            )
        elif event_type == "response.function_call_arguments.delta":
            current = self._calls.setdefault(
                index,
                {
                    "id": None,
                    "type": "function",
                    "function": {"name": "", "arguments": ""},
                },
            )
            call_id = event.get("call_id")
            if isinstance(call_id, str) and call_id:
                current["id"] = call_id
            name = event.get("name")
            if isinstance(name, str) and name:
                current["function"]["name"] = name
            delta = event.get("delta")
            if isinstance(delta, str):
                current["function"]["arguments"] += delta
        elif event_type == "response.function_call_arguments.done":
            current = self._calls.setdefault(
                index,
                {
                    "id": None,
                    "type": "function",
                    "function": {"name": "", "arguments": ""},
                },
            )
            call_id = event.get("call_id")
            if isinstance(call_id, str) and call_id:
                current["id"] = call_id
            name = event.get("name")
            if isinstance(name, str) and name:
                current["function"]["name"] = name
            arguments = event.get("arguments")
            if isinstance(arguments, str):
                current["function"]["arguments"] = arguments
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


def responses_request_context(
    messages: list[dict[str, Any]],
) -> tuple[str | None, list[dict[str, Any]]]:
    """Translate Kassiber chat messages to Responses instructions and Items."""

    instruction_parts: list[str] = []
    input_items: list[dict[str, Any]] = []
    for message in messages:
        role = message.get("role")
        content = message.get("content")
        if role == "system":
            if isinstance(content, str) and content:
                instruction_parts.append(content)
            continue
        if role == "tool":
            call_id = message.get("tool_call_id")
            if isinstance(call_id, str) and call_id and isinstance(content, str):
                input_items.append(
                    {
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": content,
                    }
                )
            continue
        if role not in {"user", "assistant", "developer"}:
            continue
        response_content = _responses_message_content(content)
        if response_content is not None and (response_content or role != "assistant"):
            input_items.append(
                {"type": "message", "role": role, "content": response_content}
            )
        if role != "assistant":
            continue
        raw_tool_calls = message.get("tool_calls")
        if not isinstance(raw_tool_calls, list):
            continue
        for raw_call in raw_tool_calls:
            if not isinstance(raw_call, dict):
                continue
            function = raw_call.get("function")
            if not isinstance(function, dict):
                continue
            call_id = raw_call.get("id")
            name = function.get("name")
            arguments = function.get("arguments")
            if not (
                isinstance(call_id, str)
                and call_id
                and isinstance(name, str)
                and name
                and isinstance(arguments, str)
            ):
                continue
            input_items.append(
                {
                    "type": "function_call",
                    "call_id": call_id,
                    "name": name,
                    "arguments": arguments,
                }
            )
    instructions = "\n\n".join(instruction_parts) or None
    return instructions, input_items


def _responses_message_content(content: object) -> str | list[dict[str, Any]] | None:
    """Normalize Chat-style text/image parts to Responses input content."""

    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return None
    parts: list[dict[str, Any]] = []
    for raw_part in content:
        if not isinstance(raw_part, dict):
            continue
        part_type = raw_part.get("type")
        if part_type == "text" and isinstance(raw_part.get("text"), str):
            parts.append({"type": "input_text", "text": raw_part["text"]})
            continue
        if part_type == "image_url":
            image = raw_part.get("image_url")
            image_url = image.get("url") if isinstance(image, dict) else image
            if isinstance(image_url, str) and image_url:
                part: dict[str, Any] = {
                    "type": "input_image",
                    "image_url": image_url,
                }
                detail = image.get("detail") if isinstance(image, dict) else None
                if isinstance(detail, str) and detail:
                    part["detail"] = detail
                parts.append(part)
            continue
        if part_type in {"input_text", "input_image", "input_file"}:
            parts.append(dict(raw_part))
    return parts


def _responses_tools(tools: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Normalize legacy nested function definitions to Responses tools."""

    normalized: list[dict[str, Any]] = []
    for tool in tools:
        if not isinstance(tool, dict) or tool.get("type") != "function":
            continue
        function = tool.get("function")
        if isinstance(function, dict):
            item = {"type": "function", **function}
        else:
            item = dict(tool)
        if isinstance(item.get("name"), str) and item["name"]:
            normalized.append(item)
    return normalized


def _responses_tool_choice(
    tool_choice: str | dict[str, Any],
) -> str | dict[str, Any]:
    if not isinstance(tool_choice, dict):
        return tool_choice
    function = tool_choice.get("function")
    if isinstance(function, dict) and isinstance(function.get("name"), str):
        return {"type": "function", "name": function["name"]}
    return dict(tool_choice)


def _responses_options(options: dict[str, Any] | None) -> dict[str, Any]:
    """Map exposed Chat-era generation options onto Responses fields."""

    body = dict(options or {})
    max_tokens = body.pop("max_tokens", None)
    if max_tokens is not None and "max_output_tokens" not in body:
        body["max_output_tokens"] = max_tokens

    response_format = body.pop("response_format", None)
    if isinstance(response_format, dict):
        text_options = body.get("text")
        if not isinstance(text_options, dict):
            text_options = {}
        else:
            text_options = dict(text_options)
        text_options.setdefault("format", response_format)
        body["text"] = text_options

    reasoning_effort = body.pop("reasoning_effort", None)
    if isinstance(reasoning_effort, str) and reasoning_effort:
        reasoning = body.get("reasoning")
        if not isinstance(reasoning, dict):
            reasoning = {}
        else:
            reasoning = dict(reasoning)
        reasoning["effort"] = reasoning_effort
        # Responses exposes summaries rather than raw chain of thought.
        reasoning.setdefault("summary", "auto")
        body["reasoning"] = reasoning

    for reserved in (
        "conversation",
        "input",
        "instructions",
        "messages",
        "model",
        "previous_response_id",
        "store",
        "stream",
        "tool_choice",
        "tools",
    ):
        body.pop(reserved, None)
    return body


def _response_output(payload: object) -> list[dict[str, Any]]:
    if not isinstance(payload, dict) or not isinstance(payload.get("output"), list):
        return []
    return [item for item in payload["output"] if isinstance(item, dict)]


def _response_tool_calls(payload: object) -> list[dict[str, Any]]:
    calls: list[dict[str, Any]] = []
    for index, item in enumerate(_response_output(payload)):
        if item.get("type") != "function_call":
            continue
        call_id = item.get("call_id") or item.get("id") or f"call_{index}"
        calls.append(
            {
                "id": call_id,
                "type": "function",
                "function": {
                    "name": item.get("name") or "",
                    "arguments": item.get("arguments") or "",
                },
            }
        )
    return calls


def _response_text(payload: object) -> str:
    parts: list[str] = []
    for item in _response_output(payload):
        if item.get("type") != "message" or not isinstance(item.get("content"), list):
            continue
        for part in item["content"]:
            if not isinstance(part, dict):
                continue
            text = part.get("text") if part.get("type") == "output_text" else None
            if not isinstance(text, str) and part.get("type") == "refusal":
                text = part.get("refusal")
            if isinstance(text, str):
                parts.append(text)
    return "".join(parts)


def _response_reasoning_summary(payload: object) -> str | None:
    parts: list[str] = []
    for item in _response_output(payload):
        if item.get("type") != "reasoning" or not isinstance(item.get("summary"), list):
            continue
        for summary in item["summary"]:
            if isinstance(summary, dict) and isinstance(summary.get("text"), str):
                parts.append(summary["text"])
    return "".join(parts) or None


def _response_finish_reason(payload: object) -> str | None:
    if not isinstance(payload, dict):
        return None
    if _response_tool_calls(payload):
        return "tool_calls"
    status = payload.get("status")
    if status == "completed":
        return "stop"
    details = payload.get("incomplete_details")
    reason = details.get("reason") if isinstance(details, dict) else None
    if reason == "max_output_tokens":
        return "length"
    if isinstance(reason, str) and reason:
        return reason
    return status if isinstance(status, str) else None


def _response_stream_error(event: dict[str, Any]) -> AppError:
    response = event.get("response") if isinstance(event.get("response"), dict) else {}
    error = event.get("error") if isinstance(event.get("error"), dict) else None
    if error is None and isinstance(response, dict) and isinstance(response.get("error"), dict):
        error = response["error"]
    if error is None and event.get("type") == "error":
        error = {
            key: value
            for key, value in event.items()
            if key not in {"type", "sequence_number"}
        }
    error = error or {}
    provider_code = str(error.get("code") or "").lower()
    encoded = json.dumps(error, sort_keys=True, default=str)
    preview, truncated = provider_error_body_preview(encoded)
    details = {
        "provider_code": provider_code or None,
        "body": preview,
        "body_truncated": truncated,
    }
    if "rate" in provider_code:
        return AppError(
            "AI provider is rate-limiting requests",
            code="ai_rate_limited",
            details=details,
            retryable=True,
        )
    if any(
        marker in provider_code
        for marker in ("api_key", "auth", "forbidden", "permission")
    ):
        return AppError(
            "AI provider rejected the request as unauthorized",
            code="ai_auth_failed",
            details=details,
            retryable=False,
        )
    if provider_code in {"invalid_prompt", "invalid_request", "invalid_request_error"}:
        return AppError(
            "AI provider rejected the Responses request",
            code="ai_request_invalid",
            details=details,
            retryable=False,
        )
    return AppError(
        "AI provider failed while generating a response",
        code="ai_unavailable",
        details=details,
        retryable=True,
    )


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
        preview, truncated = provider_error_body_preview(body)
        details["body"] = preview
        details["body_truncated"] = truncated
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
    stderr = completed.stderr or ""
    stdout = completed.stdout or ""
    details: dict[str, Any] = {"exit_code": completed.returncode}
    if stderr:
        details["stderr_bytes"] = len(stderr.encode("utf-8", errors="replace"))
    if stdout:
        details["stdout_bytes"] = len(stdout.encode("utf-8", errors="replace"))
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
class OpenAIResponsesClient:
    """Minimal OpenAI Responses-compatible HTTP client.

    Construct one per chat call (cheap). `base_url` should be the
    provider root including ``/v1`` (e.g. ``http://localhost:11434/v1``); a
    trailing slash is stripped.
    """

    base_url: str
    api_key: str | None = None
    timeout: float = DEFAULT_TIMEOUT_SECONDS
    user_agent: str = "kassiber/ai"
    direct_connection: bool = False

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
        get_egress_ledger().record_url(
            url,
            subsystem="ai",
            operation="http.request",
            method=method,
            bytes_out=http_request_bytes_out(request, method),
        )
        try:
            request_timeout = timeout if timeout is not None else self.timeout
            if self.direct_connection:
                opener = urllib.request.build_opener(
                    urllib.request.ProxyHandler({}),
                    _SameOriginRedirectHandler(self.base_url),
                )
                return opener.open(request, timeout=request_timeout)
            return urllib.request.urlopen(request, timeout=request_timeout)
        except urllib.error.HTTPError as exc:
            raise _http_error_app_error(exc) from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise _network_error_app_error(exc) from exc

    def list_models(self, *, strict: bool = False) -> list[dict]:
        """`GET /v1/models`.

        Default mode (``strict=False``) is forgiving: a 4xx response is
        treated as "this provider doesn't expose `/v1/models`" and returns
        ``[]`` so the picker can fall back to the configured default
        model. Some Responses-compatible servers and custom proxies skip the
        `/models` endpoint entirely.
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
                        hint="Check that the base URL points at an OpenAI Responses-compatible /v1 endpoint.",
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
                    hint="Check that the base URL points at an OpenAI Responses-compatible /v1 endpoint.",
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
        input_items: list[dict[str, Any]] | None = None,
        instructions: str | None = None,
        timeout: float | None = None,
    ) -> dict[str, Any]:
        """Non-streaming ``POST /v1/responses`` normalized for the daemon."""

        derived_instructions, derived_input = responses_request_context(messages)
        body = _responses_options(options)
        if tools is not None:
            body["tools"] = _responses_tools(tools)
        if tool_choice is not None:
            body["tool_choice"] = _responses_tool_choice(tool_choice)
        body.update(
            {
                "model": model,
                "input": list(input_items) if input_items is not None else derived_input,
                "stream": False,
                # Kassiber is local-first. Never let a caller option silently
                # opt accounting context into provider-side response storage.
                "store": False,
            }
        )
        effective_instructions = instructions if instructions is not None else derived_instructions
        if effective_instructions:
            body["instructions"] = effective_instructions
        response = self._open(
            "responses",
            method="POST",
            body=json.dumps(body).encode("utf-8"),
            accept_sse=False,
            timeout=timeout,
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
        if not isinstance(payload, dict):
            raise AppError(
                "AI provider returned an invalid Responses payload",
                code="ai_request_invalid",
                retryable=False,
            )
        if payload.get("status") == "failed" or isinstance(payload.get("error"), dict):
            raise _response_stream_error({"type": "response.failed", "response": payload})
        reasoning = _response_reasoning_summary(payload)
        result: dict[str, Any] = {
            "role": "assistant",
            "content": _response_text(payload),
            "finish_reason": _response_finish_reason(payload),
            "usage": payload.get("usage"),
            "response_output": _response_output(payload),
        }
        if isinstance(reasoning, str) and reasoning:
            result["reasoning"] = reasoning
        tool_calls = _response_tool_calls(payload)
        if tool_calls:
            result["tool_calls"] = tool_calls
        return result

    def stream_chat(
        self,
        *,
        messages: list[dict],
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
        input_items: list[dict[str, Any]] | None = None,
        instructions: str | None = None,
    ) -> Iterator[ChatDelta]:
        """Stream semantic events from ``POST /v1/responses``."""

        derived_instructions, derived_input = responses_request_context(messages)
        body = _responses_options(options)
        if tools is not None:
            body["tools"] = _responses_tools(tools)
        if tool_choice is not None:
            body["tool_choice"] = _responses_tool_choice(tool_choice)
        body.update(
            {
                "model": model,
                "input": list(input_items) if input_items is not None else derived_input,
                "stream": True,
                "store": False,
            }
        )
        effective_instructions = instructions if instructions is not None else derived_instructions
        if effective_instructions:
            body["instructions"] = effective_instructions
        response = self._open(
            "responses",
            method="POST",
            body=json.dumps(body).encode("utf-8"),
            accept_sse=True,
            timeout=self.timeout,
        )
        try:
            with response:
                tool_call_accumulator = ResponsesToolCallAccumulator()
                emitted_content = False
                emitted_reasoning = False
                line_iter = (raw.decode("utf-8", errors="replace") for raw in response)
                for event in parse_sse_chunks(line_iter):
                    event_type = event.get("type")
                    if event_type in {"error", "response.failed"}:
                        raise _response_stream_error(event)
                    delta: dict[str, Any] = {}
                    if event_type == "response.output_text.delta":
                        text = event.get("delta")
                        if isinstance(text, str) and text:
                            delta["content"] = text
                            emitted_content = True
                    elif event_type == "response.refusal.delta":
                        refusal = event.get("delta")
                        if isinstance(refusal, str) and refusal:
                            delta["content"] = refusal
                            emitted_content = True
                    elif event_type in {
                        "response.reasoning_summary_text.delta",
                        "response.reasoning_text.delta",
                    }:
                        reasoning = event.get("delta")
                        if isinstance(reasoning, str) and reasoning:
                            delta["reasoning"] = reasoning
                            emitted_reasoning = True
                    if event_type in {
                        "response.output_item.added",
                        "response.output_item.done",
                        "response.function_call_arguments.delta",
                        "response.function_call_arguments.done",
                    }:
                        calls = tool_call_accumulator.add_event(event)
                        if calls:
                            delta["tool_calls"] = calls

                    completed = event.get("response") if event_type == "response.completed" else None
                    response_output = _response_output(completed) if completed is not None else None
                    finish_reason = _response_finish_reason(completed) if completed is not None else None
                    if completed is not None:
                        for index, item in enumerate(response_output or []):
                            tool_call_accumulator._merge_item(
                                index,
                                item,
                                replace_arguments=True,
                            )
                        calls = tool_call_accumulator.snapshot()
                        if calls:
                            delta["tool_calls"] = calls
                        if not emitted_content:
                            completed_text = _response_text(completed)
                            if completed_text:
                                delta["content"] = completed_text
                        if not emitted_reasoning:
                            completed_reasoning = _response_reasoning_summary(completed)
                            if completed_reasoning:
                                delta["reasoning"] = completed_reasoning
                    if not delta and completed is None:
                        continue
                    yield ChatDelta(
                        delta=delta,
                        finish_reason=finish_reason,
                        raw=event,
                        response_output=response_output,
                    )
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise _network_error_app_error(exc) from exc


def _cli_subprocess_env(command: str) -> dict[str, str]:
    """Return a minimal environment for external AI CLI subprocesses.

    Do not pass Kassiber's full process environment to agent CLIs: desktop and
    daemon processes may carry backend tokens, passphrase plumbing, or other
    app secrets that are unrelated to the selected AI provider.  Keep only
    basic process settings plus the provider auth variables those CLIs commonly
    use for non-interactive operation.
    """
    allowed = set(_CLI_BASE_ENV_NAMES)
    allowed.update(_CLI_PROXY_ENV_NAMES)
    allowed_prefixes: tuple[str, ...] = ()
    if command == "claude":
        allowed.update(_CLAUDE_ENV_NAMES)
        allowed_prefixes = _CLAUDE_ENV_PREFIXES
    elif command == "codex":
        allowed.update(_CODEX_ENV_NAMES)
    env = {
        key: value
        for key, value in os.environ.items()
        if key in allowed or any(key.startswith(prefix) for prefix in allowed_prefixes)
    }
    env.setdefault("NO_COLOR", "1")
    return env


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
        env = _cli_subprocess_env(command)
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
        if tools or tool_choice not in (None, "none"):
            raise AppError(
                "CLI AI providers cannot be used with Kassiber tools enabled",
                code="ai_cli_tools_disabled",
                hint=(
                    "Turn off assistant tools for Claude/Codex CLI providers, "
                    "or use an OpenAI Responses-compatible provider so Kassiber can enforce "
                    "the typed tool allowlist and consent gates."
                ),
                retryable=False,
            )
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
    direct_connection: bool = False,
):
    if is_cli_provider_locator(base_url):
        return CliAIClient(locator=base_url, timeout=timeout)
    return OpenAIResponsesClient(
        base_url=base_url,
        api_key=api_key,
        timeout=timeout,
        direct_connection=direct_connection,
    )
