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
from dataclasses import dataclass
from typing import Any, Iterable, Iterator

import urllib.error
import urllib.parse
import urllib.request

from ..egress_ledger import get_egress_ledger, http_request_bytes_out
from ..errors import AppError
from ..redaction import provider_error_body_preview
from .cli_client import CliAIClient
from .contracts import (
    ChatDelta,
    DEFAULT_TIMEOUT_SECONDS,
    ResponsesRequestContext,
    is_cli_provider_locator,
)
from .model_metadata import safe_model_capabilities


SSE_DONE_SENTINEL = "[DONE]"


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


class ResponsesToolCallAccumulator:
    """Accumulate Responses ``function_call`` semantic stream events.

    The daemon still consumes the familiar nested function-call shape. Keeping
    that normalization here lets the rest of Kassiber remain provider-agnostic
    while the wire format uses typed Responses Items.
    """

    def __init__(self) -> None:
        self._calls: dict[int, dict[str, Any]] = {}
        self._slot_by_output_index: dict[int, int] = {}
        self._slot_by_item_id: dict[str, int | None] = {}
        self._slot_by_call_id: dict[str, int | None] = {}
        self._next_slot = 0

    @staticmethod
    def _empty_call() -> dict[str, Any]:
        return {
            "id": None,
            "type": "function",
            "function": {"name": "", "arguments": ""},
        }

    def _call(self, slot: int) -> dict[str, Any]:
        return self._calls.setdefault(slot, self._empty_call())

    @staticmethod
    def _output_index(event: dict[str, Any]) -> int | None:
        raw_index = event.get("output_index")
        if isinstance(raw_index, bool):
            return None
        try:
            index = int(raw_index)
        except (TypeError, ValueError):
            return None
        return index if index >= 0 else None

    @staticmethod
    def _identifiers(
        event: dict[str, Any], item: object = None
    ) -> tuple[tuple[str, ...], tuple[str, ...]]:
        item_ids = [event.get("item_id")]
        call_ids = [event.get("call_id")]
        if isinstance(item, dict):
            item_ids.append(item.get("id"))
            call_ids.append(item.get("call_id"))
        return (
            tuple(
                dict.fromkeys(
                    value for value in item_ids if isinstance(value, str) and value
                )
            ),
            tuple(
                dict.fromkeys(
                    value for value in call_ids if isinstance(value, str) and value
                )
            ),
        )

    @staticmethod
    def _mapped_slot(
        slots: dict[str, int | None], identifiers: tuple[str, ...]
    ) -> int | None:
        for identifier in identifiers:
            slot = slots.get(identifier)
            if slot is not None:
                return slot
        return None

    @staticmethod
    def _register_identifiers(
        slots: dict[str, int | None], identifiers: tuple[str, ...], slot: int
    ) -> None:
        for identifier in identifiers:
            if identifier not in slots:
                slots[identifier] = slot
            elif slots[identifier] != slot:
                slots[identifier] = None

    def _slot(self, event: dict[str, Any], item: object = None) -> int:
        item_ids, call_ids = self._identifiers(event, item)
        output_index = self._output_index(event)
        output_index_is_known = output_index in self._slot_by_output_index
        slot = (
            self._slot_by_output_index.get(output_index)
            if output_index is not None
            else None
        )
        if slot is None:
            slot = self._mapped_slot(self._slot_by_item_id, item_ids)
        if slot is None and (not item_ids or output_index is not None):
            slot = self._mapped_slot(self._slot_by_call_id, call_ids)
        if (
            slot is not None
            and output_index is not None
            and not output_index_is_known
            and slot in self._slot_by_output_index.values()
        ):
            # A valid output index is authoritative. Reusing a provider-buggy
            # item/call ID here would hide duplicate calls from consent checks.
            slot = None
        if slot is None:
            slot = self._next_slot
            self._next_slot += 1
        if output_index is not None:
            self._slot_by_output_index[output_index] = slot
        self._register_identifiers(self._slot_by_item_id, item_ids, slot)
        self._register_identifiers(self._slot_by_call_id, call_ids, slot)
        return slot

    def _reset(self) -> None:
        self._calls.clear()
        self._slot_by_output_index.clear()
        self._slot_by_item_id.clear()
        self._slot_by_call_id.clear()
        self._next_slot = 0

    def _merge_item(self, slot: int, item: object, *, replace_arguments: bool) -> None:
        if not isinstance(item, dict) or item.get("type") != "function_call":
            return
        current = self._call(slot)
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
        if event_type in {"response.output_item.added", "response.output_item.done"}:
            item = event.get("item")
            self._merge_item(
                self._slot(event, item),
                item,
                replace_arguments=event_type == "response.output_item.done",
            )
        elif event_type == "response.function_call_arguments.delta":
            current = self._call(self._slot(event))
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
            current = self._call(self._slot(event))
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

    def replace_output_items(self, items: Iterable[object]) -> list[dict[str, Any]]:
        """Replace streamed partials with the authoritative terminal output."""

        self._reset()
        for index, item in enumerate(items):
            if not isinstance(item, dict) or item.get("type") != "function_call":
                continue
            event = {"output_index": index}
            self._merge_item(
                self._slot(event, item),
                item,
                replace_arguments=True,
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


def responses_request_context(
    messages: list[dict[str, Any]],
) -> ResponsesRequestContext:
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
    return ResponsesRequestContext(
        instructions=instructions,
        input_items=input_items,
    )


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
            row.update(safe_model_capabilities(item))
            models.append(row)
        return models

    def _request_body(
        self,
        *,
        messages: list[dict] | None,
        context: ResponsesRequestContext | None,
        model: str,
        stream: bool,
        options: dict[str, Any] | None,
        tools: list[dict[str, Any]] | None,
        tool_choice: str | dict[str, Any] | None,
    ) -> dict[str, Any]:
        """Build the canonical privacy-preserving Responses request body."""

        if context is not None and messages is not None:
            raise AppError(
                "Responses requests accept either messages or a prepared context, not both",
                code="validation",
                retryable=False,
            )
        if context is None:
            if messages is None:
                raise AppError(
                    "Responses requests require messages or a prepared context",
                    code="validation",
                    retryable=False,
                )
            context = responses_request_context(messages)

        body = _responses_options(options)
        if tools is not None:
            body["tools"] = _responses_tools(tools)
        if tool_choice is not None:
            body["tool_choice"] = _responses_tool_choice(tool_choice)
        body.update(
            {
                "model": model,
                "input": list(context.input_items),
                "stream": stream,
                # Kassiber is local-first. Provider-side response storage may
                # never be enabled by a caller-supplied option.
                "store": False,
            }
        )
        if context.instructions:
            body["instructions"] = context.instructions
        return body

    def chat(
        self,
        *,
        messages: list[dict] | None = None,
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
        context: ResponsesRequestContext | None = None,
        timeout: float | None = None,
    ) -> dict[str, Any]:
        """Non-streaming ``POST /v1/responses`` normalized for the daemon."""

        body = self._request_body(
            messages=messages,
            context=context,
            model=model,
            stream=False,
            options=options,
            tools=tools,
            tool_choice=tool_choice,
        )
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
        messages: list[dict] | None = None,
        model: str,
        options: dict[str, Any] | None = None,
        tools: list[dict[str, Any]] | None = None,
        tool_choice: str | dict[str, Any] | None = None,
        context: ResponsesRequestContext | None = None,
    ) -> Iterator[ChatDelta]:
        """Stream semantic events from ``POST /v1/responses``."""

        body = self._request_body(
            messages=messages,
            context=context,
            model=model,
            stream=True,
            options=options,
            tools=tools,
            tool_choice=tool_choice,
        )
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
                        had_streamed_calls = bool(tool_call_accumulator.snapshot())
                        calls = tool_call_accumulator.replace_output_items(
                            response_output or []
                        )
                        if calls or had_streamed_calls:
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
