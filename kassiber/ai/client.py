"""OpenAI-compatible HTTP client over stdlib `urllib.request`.

HTTP providers speak one of two wire formats for inference, ``POST
/v1/responses`` (the Responses API) or ``POST /v1/chat/completions`` (Chat
Completions), and ``GET /v1/models`` for discovery. The daemon's tool loop
works on Responses-style input Items either way: the client translates them
to the wire format at the transport boundary and turns both streams into the
same ChatDelta events. Current Ollama, oMLX and OpenAI endpoints implement
Responses; OpenRouter's main API and most other OpenAI-compatible servers
speak Chat Completions.

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
from dataclasses import dataclass, field
import threading
from typing import Any, Iterable, Iterator

import urllib.error
import urllib.parse
import urllib.request

from .. import egress_policy
from ..egress_ledger import get_egress_ledger, http_request_bytes_out
from ..errors import AppError
from ..redaction import provider_error_body_preview
from .broker_client import BrokerAIClient
from .contracts import (
    AI_API_FORMAT_AUTO,
    AI_API_FORMAT_CHAT_COMPLETIONS,
    AI_API_FORMAT_RESPONSES,
    AI_WIRE_API_FORMATS,
    ChatDelta,
    DEFAULT_TIMEOUT_SECONDS,
    ResponsesRequestContext,
    http_reasoning_effort,
    is_cli_provider_locator,
)
from .model_metadata import http_model_capabilities


SSE_DONE_SENTINEL = "[DONE]"


def _url_origin(url: str) -> tuple[str, str, int | None]:
    parsed = urllib.parse.urlsplit(url)
    port = parsed.port
    if port is None:
        port = 443 if parsed.scheme.lower() == "https" else 80
    return parsed.scheme.lower(), (parsed.hostname or "").lower(), port


class _SameOriginRedirectHandler(urllib.request.HTTPRedirectHandler):
    """Provider acknowledgement never authorizes a different origin."""

    def __init__(self, origin_url: str):
        super().__init__()
        self._origin = _url_origin(origin_url)

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if _url_origin(newurl) != self._origin:
            if fp is not None:
                fp.close()
            raise AppError(
                "AI provider attempted an off-origin redirect",
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

    # Kassiber's xhigh/max/ultra go out as "high"; "auto" sends no effort.
    reasoning_effort = http_reasoning_effort(body.pop("reasoning_effort", None))
    if reasoning_effort is not None:
        reasoning = body.get("reasoning")
        if not isinstance(reasoning, dict):
            reasoning = {}
        else:
            reasoning = dict(reasoning)
        reasoning["effort"] = reasoning_effort
        # No `summary`: no model listing says which models produce reasoning
        # summaries, and OpenAI refuses them for an unverified organization,
        # which would cost the turn. Endpoints that stream reasoning on their
        # own (Ollama, oMLX) still show it.
        body["reasoning"] = reasoning

    for reserved in (
        "conversation",
        "include",
        "input",
        "instructions",
        "messages",
        "model",
        "previous_response_id",
        "store",
        "sensitive_context",
        # Broker-only: an HTTP request already sends `store: false` and keeps
        # no provider-side conversation, so Incognito changes nothing here.
        "stateless_session",
        # Broker-only (Codex service tier / Claude fast mode, native session
        # cursors): an OpenAI-compatible endpoint has no such field.
        "fast_mode",
        "provider_session_id",
        "stream",
        "tool_choice",
        "tools",
    ):
        body.pop(reserved, None)
    return body


# Asked for together with a reasoning effort, so the reasoning Items of a tool
# round can be replayed to an endpoint that stores nothing (`store: false`).
RESPONSES_REASONING_INCLUDE = "reasoning.encrypted_content"


def _replayable_input_items(
    items: Iterable[dict[str, Any]], *, keep_reasoning: bool = True
) -> list[dict[str, Any]]:
    """Input Items an endpoint can use without stored responses.

    With ``store: false`` a reasoning Item is only usable through its
    ``encrypted_content``. Replaying one without it makes the endpoint look
    its id up in storage that was never written ("Item with id 'rs_…' not
    found"), so such an Item is dropped rather than sent as a dangling id.
    """

    replayable: list[dict[str, Any]] = []
    for item in items:
        if isinstance(item, dict) and item.get("type") == "reasoning":
            encrypted = item.get("encrypted_content")
            if not keep_reasoning or not (isinstance(encrypted, str) and encrypted):
                continue
        replayable.append(item)
    return replayable


@dataclass
class _WireCompat:
    """Optional request parts an endpoint refused, sticky for one client.

    A refusal costs one retry without the part; later rounds of the same
    turn leave it out from the start.
    """

    include: bool = True
    # The reasoning effort and any reasoning replayed from earlier rounds.
    reasoning: bool = True
    # Chat Completions: send the token limit as max_completion_tokens.
    max_completion_tokens: bool = False


@dataclass(frozen=True)
class _PreparedRequest:
    path: str
    body: dict[str, Any]
    include_sent: bool = False
    # A reasoning effort, or reasoning replayed from an earlier round.
    reasoning_sent: bool = False
    # The user's reasoning effort is in the request.
    effort_sent: bool = False
    # A Chat Completions token limit sent as max_tokens.
    max_tokens_sent: bool = False


# Words an endpoint's refusal uses for a reasoning setting it does not take:
# OpenAI's "reasoning.effort is not supported", Ollama's "does not support
# thinking", a strict schema's "extra inputs ... reasoning_effort".
_REASONING_REFUSAL_WORDS = ("reasoning", "effort", "summary", "think")


def _error_status(exc: AppError) -> int | None:
    details = exc.details if isinstance(exc.details, dict) else {}
    status = details.get("status")
    return status if isinstance(status, int) and not isinstance(status, bool) else None


def _error_text(exc: AppError) -> str:
    """The provider's (redacted, bounded) error body and message, lowercased."""

    details = exc.details if isinstance(exc.details, dict) else {}
    body = details.get("body")
    return f"{body if isinstance(body, str) else ''} {exc}".lower()


# A 404 that names one of these is about the model or the tools the request
# asked for (OpenAI's model_not_found, Ollama's "model not found", OpenRouter's
# "No endpoints found that support tool use"), not a path the server lacks.
_NOT_A_MISSING_ROUTE_WORDS = ("model", "tool", "endpoints found")


def route_missing(exc: AppError) -> bool:
    """Whether a request failed because the server has no such path."""

    if _error_status(exc) not in (404, 405):
        return False
    text = _error_text(exc)
    return not any(word in text for word in _NOT_A_MISSING_ROUTE_WORDS)


# How servers without a path tend to say so in a body that is not a 404.
_UNSERVED_FORMAT_PHRASES = (
    "not found",
    "unexpected endpoint",
    "unknown endpoint",
    "unsupported endpoint",
    "not implemented",
)


def _format_not_served(exc: AppError) -> bool:
    """For the connection test: the endpoint does not speak this format at all.

    Broader than `route_missing`, which alone may switch a chat's format: the
    test is the explicit way to find the format, so a 400/501 that says the
    path is unknown, or a 200 that carried no answer in the format, counts too.
    """

    if route_missing(exc):
        return True
    details = exc.details if isinstance(exc.details, dict) else {}
    if details.get("reason") in {"no_stream_events", "no_answer"}:
        return True
    text = _error_text(exc)
    return (
        _error_status(exc) in (400, 404, 405, 501)
        and any(phrase in text for phrase in _UNSERVED_FORMAT_PHRASES)
        and not any(word in text for word in _NOT_A_MISSING_ROUTE_WORDS)
    )


def _tools_refused(exc: AppError) -> bool:
    """For the connection test: the endpoint or model refused the tool definition.

    Ollama says a model "does not support tools", vLLM wants
    --enable-auto-tool-choice, llama.cpp wants --jinja, and OpenRouter finds
    "no endpoints ... that support tool use".
    """

    if _error_status(exc) not in (400, 404, 422, 500, 501):
        return False
    text = _error_text(exc)
    return "tool" in text or "function call" in text or "function_call" in text


def _request_context(
    messages: list[dict] | None,
    context: ResponsesRequestContext | None,
) -> ResponsesRequestContext:
    """The one conversation a request carries: chat messages or a prepared context."""

    if context is not None and messages is not None:
        raise AppError(
            "AI requests accept either messages or a prepared context, not both",
            code="validation",
            retryable=False,
        )
    if context is None:
        if messages is None:
            raise AppError(
                "AI requests require messages or a prepared context",
                code="validation",
                retryable=False,
            )
        context = responses_request_context(messages)
    return context


def _response_output(payload: object) -> list[dict[str, Any]]:
    """The response's output Items, without a function call it never finished.

    An incomplete response (max_output_tokens, content_filter) can stop
    inside a function call whose arguments were cut off. That call is neither
    executed nor replayed: it would run on truncated arguments, and a replayed
    call without its output is refused by the next request.
    """

    if not isinstance(payload, dict) or not isinstance(payload.get("output"), list):
        return []
    incomplete = payload.get("status") == "incomplete"
    return [
        item
        for item in payload["output"]
        if isinstance(item, dict)
        and not (
            incomplete
            and item.get("type") == "function_call"
            and item.get("status") not in (None, "completed")
        )
    ]


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


# Why a Responses answer ended incomplete, as the finish reasons Chat
# Completions and the daemon already use.
_INCOMPLETE_FINISH_REASONS = {
    "max_output_tokens": "length",
    "content_filter": "content_filter",
}


def _response_finish_reason(payload: object) -> str | None:
    if not isinstance(payload, dict):
        return None
    status = payload.get("status")
    details = payload.get("incomplete_details")
    reason = details.get("reason") if isinstance(details, dict) else None
    if status == "incomplete":
        if isinstance(reason, str) and reason:
            return _INCOMPLETE_FINISH_REASONS.get(reason, reason)
        return status
    if _response_tool_calls(payload):
        return "tool_calls"
    if status == "completed":
        return "stop"
    if isinstance(reason, str) and reason:
        return _INCOMPLETE_FINISH_REASONS.get(reason, reason)
    return status if isinstance(status, str) else None


def _provider_error(
    error: dict[str, Any],
    *,
    invalid_message: str = "AI provider rejected the request",
) -> AppError:
    """Map an error object a provider sent inside a 200 stream or body."""

    raw_code = error.get("code")
    status = raw_code if isinstance(raw_code, int) and not isinstance(raw_code, bool) else None
    provider_code = str(raw_code if raw_code is not None else "").lower()
    error_type = str(error.get("type") or "").lower()
    encoded = json.dumps(error, sort_keys=True, default=str)
    preview, truncated = provider_error_body_preview(encoded)
    details = {
        "provider_code": provider_code or None,
        "body": preview,
        "body_truncated": truncated,
    }
    if status == 429 or "rate" in provider_code or "rate_limit" in error_type:
        return AppError(
            "AI provider is rate-limiting requests",
            code="ai_rate_limited",
            details=details,
            retryable=True,
        )
    if status in (401, 403) or any(
        marker in provider_code
        for marker in ("api_key", "auth", "forbidden", "permission")
    ):
        return AppError(
            "AI provider rejected the request as unauthorized",
            code="ai_auth_failed",
            details=details,
            retryable=False,
        )
    if (
        (status is not None and 400 <= status < 500)
        or provider_code in {"invalid_prompt", "invalid_request", "invalid_request_error"}
        or error_type == "invalid_request_error"
    ):
        return AppError(
            invalid_message,
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
    return _provider_error(
        error or {}, invalid_message="AI provider rejected the Responses request"
    )


# --- Connection test --------------------------------------------------------
#
# Listing models proves an endpoint is reachable, not that it can chat. The
# test therefore also sends one fixed request: no user data, a few tokens, and
# one trivial tool the model is not asked to call, to learn whether the
# endpoint takes tool definitions.

CONNECTION_TEST_PROMPT = "Reply with OK"
# Small enough to cost next to nothing; OpenAI's Responses API refuses fewer
# than 16 output tokens.
CONNECTION_TEST_MAX_TOKENS = 16
# A local server may have to load the model before it answers.
CONNECTION_TEST_TIMEOUT_SECONDS = 30.0
CONNECTION_TEST_TOOL = {
    "type": "function",
    "name": "connection_check",
    "description": "Not needed for the answer; offered only to check that tools are accepted.",
    "parameters": {"type": "object", "properties": {}, "additionalProperties": False},
    "strict": False,
}


def connection_test_model(requested: object, models: list[dict[str, Any]]) -> str | None:
    """The model a connection test asks: the given one, else the only listed one.

    With several listed models and none chosen the test does not guess: a
    random pick could be an embedding model, or one without tools.
    """

    if isinstance(requested, str) and requested.strip():
        return requested.strip()
    ids = [model.get("id") for model in models if isinstance(model.get("id"), str) and model.get("id")]
    return ids[0] if len(ids) == 1 else None


def run_connection_test(
    client: Any, *, http: bool, model: object = None
) -> dict[str, Any]:
    """List a provider's models and, over HTTP, send the fixed test request.

    Returns the test result without ``base_url``: the models, the format
    setting tested, the format that answered (``detected_api_format``),
    whether tools were accepted, and what the probe did.
    """

    models = client.list_models(strict=True)
    result: dict[str, Any] = {
        "model_count": len(models),
        "models": models,
        "check_kind": (
            "binary_presence"
            if any(row.get("check_kind") == "binary_presence" for row in models)
            else "models"
        ),
        "api_format": None,
        "detected_api_format": None,
        "tools_accepted": None,
        "probe": {"status": "skipped", "reason": "cli_provider", "model": None, "finish_reason": None},
    }
    if not http:
        return result
    result["api_format"] = getattr(client, "api_format", AI_API_FORMAT_AUTO)
    probe_model = connection_test_model(model, models)
    if probe_model is None:
        result["probe"]["reason"] = "no_model"
        return result
    outcome = client.probe(model=probe_model)
    result["detected_api_format"] = outcome["api_format"]
    result["tools_accepted"] = outcome["tools_accepted"]
    result["probe"] = {
        "status": "ok",
        "reason": None,
        "model": probe_model,
        "finish_reason": outcome["finish_reason"],
    }
    return result


# --- Chat Completions -------------------------------------------------------
#
# The daemon's conversation is a list of Responses-style input Items (messages,
# function calls and their outputs, reasoning). For an endpoint that speaks
# only Chat Completions the client turns those Items into `messages` and turns
# the streamed chunks back into the same ChatDelta events the Responses path
# emits, so the tool loop never sees which format the provider speaks.

# Generation options a Chat Completions request passes on when the caller set
# them. Everything else stays out: strict endpoints refuse unknown fields.
_CHAT_COMPLETIONS_OPTIONS = (
    "temperature",
    "top_p",
    "stop",
    "seed",
    "frequency_penalty",
    "presence_penalty",
    "response_format",
)
# Where non-standard Chat Completions servers stream reasoning text:
# DeepSeek, vLLM and llama.cpp use reasoning_content; OpenRouter and Ollama
# use reasoning. It is shown as reasoning, never as the answer, and replayed
# under the same field to the next tool round (DeepSeek refuses a tool round
# whose earlier reasoning is missing).
_CHAT_REASONING_FIELDS = ("reasoning_content", "reasoning")
# Finish reasons some servers use instead of OpenAI's names.
_CHAT_FINISH_REASONS = {
    "function_call": "tool_calls",
    "end_turn": "stop",
    "eos_token": "stop",
    "stop_sequence": "stop",
    "max_tokens": "length",
}


def _is_openrouter(base_url: str) -> bool:
    host = (urllib.parse.urlsplit(base_url).hostname or "").lower()
    return host == "openrouter.ai" or host.endswith(".openrouter.ai")


def _chat_text(content: object) -> str:
    """The text of an assistant or system message's content."""

    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return ""
    parts: list[str] = []
    for part in content:
        if not isinstance(part, dict):
            continue
        if part.get("type") in {"output_text", "input_text", "text"} and isinstance(
            part.get("text"), str
        ):
            parts.append(part["text"])
        elif part.get("type") == "refusal" and isinstance(part.get("refusal"), str):
            parts.append(part["refusal"])
    return "".join(parts)


def _chat_user_content(content: object) -> str | list[dict[str, Any]]:
    """Responses input content as Chat Completions text and image parts."""

    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return ""
    parts: list[dict[str, Any]] = []
    for part in content:
        if not isinstance(part, dict):
            continue
        kind = part.get("type")
        if kind in {"input_text", "text", "output_text"} and isinstance(part.get("text"), str):
            parts.append({"type": "text", "text": part["text"]})
        elif kind == "input_image":
            url = part.get("image_url")
            if isinstance(url, str) and url:
                image: dict[str, Any] = {"url": url}
                detail = part.get("detail")
                if isinstance(detail, str) and detail:
                    image["detail"] = detail
                parts.append({"type": "image_url", "image_url": image})
        elif kind == "image_url":
            parts.append(dict(part))
        elif kind == "input_file":
            file = {
                key: part[key]
                for key in ("file_id", "file_data", "filename")
                if isinstance(part.get(key), str) and part[key]
            }
            if file:
                parts.append({"type": "file", "file": file})
    return parts


def _reasoning_item_text(item: dict[str, Any]) -> str:
    parts: list[str] = []
    for source in (item.get("content"), item.get("summary")):
        if not isinstance(source, list):
            continue
        for part in source:
            if isinstance(part, dict) and isinstance(part.get("text"), str):
                parts.append(part["text"])
    return "".join(parts)


def chat_completions_messages(
    context: ResponsesRequestContext,
    *,
    replay_reasoning: bool = True,
) -> list[dict[str, Any]]:
    """Translate the daemon's input Items into Chat Completions messages.

    Instructions become a leading system message. An assistant message and
    the function calls and reasoning of the same round become one assistant
    message with ``tool_calls``; each function output becomes a ``tool``
    message answering its ``tool_call_id``. Reasoning is replayed only when it
    came from a Chat Completions answer (a Responses reasoning Item means
    nothing to this format).
    """

    messages: list[dict[str, Any]] = []
    if context.instructions:
        messages.append({"role": "system", "content": context.instructions})
    pending: dict[str, Any] | None = None

    def flush() -> None:
        nonlocal pending
        if pending is not None:
            if pending.get("tool_calls"):
                if not pending.get("content"):
                    pending["content"] = None
                messages.append(pending)
            elif pending.get("content"):
                messages.append(pending)
        pending = None

    def assistant() -> dict[str, Any]:
        nonlocal pending
        if pending is None:
            pending = {"role": "assistant", "content": None}
        return pending

    for item in context.input_items:
        if not isinstance(item, dict):
            continue
        kind = item.get("type") or ("message" if "role" in item else None)
        if kind == "message":
            role = item.get("role")
            if role == "assistant":
                text = _chat_text(item.get("content"))
                if pending is not None and (pending.get("content") or pending.get("tool_calls")):
                    flush()
                assistant()["content"] = text
                continue
            flush()
            if role in {"system", "developer"}:
                text = _chat_text(item.get("content"))
                if text:
                    messages.append({"role": "system", "content": text})
            elif role == "user":
                messages.append({"role": "user", "content": _chat_user_content(item.get("content"))})
            continue
        if kind == "function_call":
            call_id = item.get("call_id") or item.get("id")
            name = item.get("name")
            if not (isinstance(call_id, str) and call_id and isinstance(name, str) and name):
                continue
            arguments = item.get("arguments")
            assistant().setdefault("tool_calls", []).append(
                {
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments if isinstance(arguments, str) and arguments else "{}",
                    },
                }
            )
            continue
        if kind == "reasoning":
            replay = item.get("chat_completions_replay")
            if not replay_reasoning or not isinstance(replay, dict):
                continue
            if pending is not None and (pending.get("content") or pending.get("tool_calls")):
                flush()
            target = assistant()
            details = replay.get("reasoning_details")
            field = replay.get("field")
            text = _reasoning_item_text(item)
            if isinstance(details, list) and details:
                target["reasoning_details"] = details
            elif field in _CHAT_REASONING_FIELDS and text:
                target[field] = text
            continue
        if kind == "function_call_output":
            flush()
            call_id = item.get("call_id")
            if not (isinstance(call_id, str) and call_id):
                continue
            output = item.get("output")
            messages.append(
                {
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output if isinstance(output, str) else json.dumps(output),
                }
            )
    flush()
    return messages


def _chat_completions_tools(tools: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Flat Responses or nested function definitions as Chat Completions tools."""

    normalized: list[dict[str, Any]] = []
    for tool in tools:
        if not isinstance(tool, dict) or tool.get("type") != "function":
            continue
        function = tool.get("function") if isinstance(tool.get("function"), dict) else tool
        name = function.get("name")
        if not (isinstance(name, str) and name):
            continue
        definition: dict[str, Any] = {"name": name}
        if isinstance(function.get("description"), str):
            definition["description"] = function["description"]
        if isinstance(function.get("parameters"), dict):
            definition["parameters"] = function["parameters"]
        normalized.append({"type": "function", "function": definition})
    return normalized


def _chat_completions_tool_choice(tool_choice: str | dict[str, Any] | None) -> str | dict[str, Any]:
    if isinstance(tool_choice, str) and tool_choice:
        return tool_choice
    if isinstance(tool_choice, dict):
        function = tool_choice.get("function")
        name = function.get("name") if isinstance(function, dict) else tool_choice.get("name")
        if isinstance(name, str) and name:
            return {"type": "function", "function": {"name": name}}
    return "auto"


class ChatCompletionsToolCallAccumulator:
    """Accumulate streamed Chat Completions ``tool_calls`` deltas.

    A call is keyed by its ``index``. Its id, type and name usually arrive in
    the first chunk only, and its arguments arrive in pieces. A server that
    omits the index is matched by call id; a piece with neither continues the
    latest call, and a new id under an index already in use starts a new call
    (some servers send index 0 for every parallel call).
    """

    def __init__(self, *, id_prefix: str = "call_") -> None:
        self._id_prefix = id_prefix
        self._calls: dict[int, dict[str, Any]] = {}
        self._slot_by_index: dict[int, int] = {}
        self._slot_by_id: dict[str, int] = {}
        self._last_slot: int | None = None
        self._next_slot = 0

    def add(self, deltas: object) -> list[dict[str, Any]]:
        if not isinstance(deltas, list):
            return self.snapshot()
        for raw in deltas:
            if not isinstance(raw, dict):
                continue
            raw_id = raw.get("id")
            call_id = raw_id if isinstance(raw_id, str) and raw_id else None
            raw_index = raw.get("index")
            index = (
                raw_index
                if isinstance(raw_index, int) and not isinstance(raw_index, bool) and raw_index >= 0
                else None
            )
            slot = self._slot_by_index.get(index) if index is not None else None
            if (
                slot is not None
                and call_id is not None
                and self._calls[slot]["id"] not in (None, call_id)
            ):
                slot = None
            if slot is None and call_id is not None:
                slot = self._slot_by_id.get(call_id)
            if slot is None and index is None and call_id is None:
                slot = self._last_slot
            if slot is None:
                slot = self._next_slot
                self._next_slot += 1
                self._calls[slot] = {
                    "id": None,
                    "type": "function",
                    "function": {"name": "", "arguments": ""},
                }
            if index is not None:
                self._slot_by_index[index] = slot
            if call_id is not None:
                self._slot_by_id.setdefault(call_id, slot)
            self._last_slot = slot
            current = self._calls[slot]
            if call_id is not None:
                current["id"] = call_id
            function = raw.get("function")
            if not isinstance(function, dict):
                continue
            name = function.get("name")
            if isinstance(name, str) and name:
                current["function"]["name"] = name
            arguments = function.get("arguments")
            if isinstance(arguments, str):
                current["function"]["arguments"] += arguments
            elif isinstance(arguments, dict):
                current["function"]["arguments"] = json.dumps(arguments)
        return self.snapshot()

    def snapshot(self) -> list[dict[str, Any]]:
        return [
            {
                "id": call["id"] or f"{self._id_prefix}{slot}",
                "type": "function",
                "function": {
                    "name": call["function"]["name"],
                    "arguments": call["function"]["arguments"],
                },
            }
            for slot, call in sorted(self._calls.items())
        ]


class _ReasoningDetailsAccumulator:
    """Merge OpenRouter's streamed ``reasoning_details`` pieces by ``index``.

    Text-like fields continue across pieces; the others (type, format, id,
    signature) take their latest value. The merged blocks are replayed as
    they were produced, which OpenRouter needs for Claude's signed reasoning.
    """

    _TEXT_FIELDS = ("text", "summary", "data")

    def __init__(self) -> None:
        self._details: dict[Any, dict[str, Any]] = {}

    def add(self, pieces: object) -> None:
        if not isinstance(pieces, list):
            return
        for piece in pieces:
            if not isinstance(piece, dict):
                continue
            index = piece.get("index")
            key = (
                index
                if isinstance(index, int) and not isinstance(index, bool)
                else f"unindexed-{len(self._details)}"
            )
            current = self._details.setdefault(key, {})
            for name, value in piece.items():
                if name in self._TEXT_FIELDS and isinstance(value, str):
                    previous = current.get(name)
                    current[name] = (previous if isinstance(previous, str) else "") + value
                elif value is not None:
                    current[name] = value

    def result(self) -> list[dict[str, Any]]:
        return list(self._details.values())


def _chat_finish_reason(reason: object, has_calls: bool) -> str:
    finish = _CHAT_FINISH_REASONS.get(reason, reason) if isinstance(reason, str) and reason else None
    if has_calls and finish in (None, "stop"):
        return "tool_calls"
    return finish or "stop"


def _chat_completion_output(
    *,
    content: str,
    reasoning: str,
    reasoning_field: str | None,
    reasoning_details: list[dict[str, Any]],
    tool_calls: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """The answer as Responses-style output Items, for the tool loop's replay."""

    output: list[dict[str, Any]] = []
    if reasoning or reasoning_details:
        replay: dict[str, Any] = {"field": reasoning_field or "reasoning_content"}
        if reasoning_details:
            replay["reasoning_details"] = reasoning_details
        output.append(
            {
                "type": "reasoning",
                "summary": [],
                "content": [{"type": "reasoning_text", "text": reasoning}],
                # Read only by `chat_completions_messages`; a turn never
                # switches format, so no Responses endpoint receives it.
                "chat_completions_replay": replay,
            }
        )
    if content:
        output.append(
            {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": content}],
            }
        )
    for call in tool_calls:
        output.append(
            {
                "type": "function_call",
                "call_id": call["id"],
                "name": call["function"]["name"],
                "arguments": call["function"]["arguments"],
            }
        )
    return output


def _chat_completion_stream_error(event: dict[str, Any]) -> AppError | None:
    """An error object inside a stream chunk (OpenRouter, vLLM), if any."""

    error = event.get("error")
    if error is None and event.get("object") == "error":
        error = {key: value for key, value in event.items() if key != "object"}
    if error is None:
        return None
    if not isinstance(error, dict):
        error = {"message": str(error)}
    return _provider_error(error)


def _normalized_tool_calls(raw_calls: object, *, id_prefix: str) -> list[dict[str, Any]]:
    """Complete (non-streamed) Chat Completions tool calls in the daemon shape."""

    accumulator = ChatCompletionsToolCallAccumulator(id_prefix=id_prefix)
    if isinstance(raw_calls, list):
        accumulator.add(
            [
                {**call, "index": position}
                for position, call in enumerate(raw_calls)
                if isinstance(call, dict)
            ]
        )
    return accumulator.snapshot()


# --- Server-sent events -----------------------------------------------------

# Yielded by `_sse_payloads` for the `[DONE]` sentinel, so a caller can tell a
# finished stream from one that was cut off.
_SSE_DONE = object()


def _sse_payloads(lines: Iterable[str]) -> Iterator[Any]:
    """Yield each SSE ``data`` payload as a dict, then `_SSE_DONE` on ``[DONE]``.

    SSE grammar handled (per HTML living standard, narrowed to what
    OpenAI-compat servers actually emit):

    * Comments — lines starting with ``:`` are skipped (OpenRouter sends
      ``: OPENROUTER PROCESSING`` keep-alives).
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
                yield _SSE_DONE
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
    if done:
        yield _SSE_DONE


def parse_sse_chunks(lines: Iterable[str]) -> Iterator[dict]:
    """Yield JSON objects from an iterator over SSE text lines.

    Stops at the ``[DONE]`` sentinel; see `_sse_payloads` for the grammar.
    """
    for payload in _sse_payloads(lines):
        if payload is _SSE_DONE:
            return
        yield payload


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
class OpenAICompatibleClient:
    """OpenAI-compatible HTTP client for the Responses and Chat Completions APIs.

    Construct one per chat call (cheap). `base_url` should be the
    provider root including ``/v1`` (e.g. ``http://localhost:11434/v1``); a
    trailing slash is stripped. ``api_format`` is the provider's setting
    (``auto``, ``responses`` or ``chat_completions``) and
    ``detected_api_format`` the format last detected for its endpoint. The
    first request that is accepted fixes the format for the rest of the
    client's life, so the rounds of one turn never mix formats.
    """

    base_url: str
    api_key: str | None = None
    timeout: float = DEFAULT_TIMEOUT_SECONDS
    user_agent: str = "kassiber/ai"
    # A provider marked local (always a loopback URL). Only such a provider
    # keeps working in offline mode; a loopback gateway to a remote model does not.
    on_device: bool = False
    api_format: str = AI_API_FORMAT_AUTO
    detected_api_format: str | None = None
    _cancelled: threading.Event = field(default_factory=threading.Event, init=False, repr=False)
    _active_response: Any = field(default=None, init=False, repr=False)
    _compat: _WireCompat = field(default_factory=_WireCompat, init=False, repr=False)
    # Notices from non-streaming calls; streaming yields them as chunks.
    notices: list[str] = field(default_factory=list, init=False, repr=False)
    _wire_format: str | None = field(default=None, init=False, repr=False)
    _answers: int = field(default=0, init=False, repr=False)
    _fell_back: bool = field(default=False, init=False, repr=False)
    # Set once a Chat Completions answer arrived after the auto fallback, for
    # the caller to store on the provider row.
    newly_detected_api_format: str | None = field(default=None, init=False, repr=False)

    def cancel(self) -> None:
        self._cancelled.set()
        response = self._active_response
        if response is not None:
            # Buffered HTTP readers can hold their own lock. Closing them must
            # never stall the daemon's lock/cancel control thread.
            def close_response():
                try:
                    response.close()
                except OSError:
                    pass
            threading.Thread(target=close_response, daemon=True, name="kassiber-ai-http-cancel").start()

    def wire_format(self) -> str:
        """The format the next generation request speaks.

        An explicit setting wins; ``auto`` uses the detected format, else the
        Responses API. After a request is accepted it stays that format.
        """

        if self._wire_format is not None:
            return self._wire_format
        if self.api_format in AI_WIRE_API_FORMATS:
            return self.api_format
        if self.detected_api_format in AI_WIRE_API_FORMATS:
            return str(self.detected_api_format)
        return AI_API_FORMAT_RESPONSES

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
        if self._cancelled.is_set():
            raise AppError("AI request cancelled", code="ai_cancelled")
        # Offline mode keeps an on-device model and blocks the rest.
        egress_policy.require_online(
            "Remote AI providers are",
            on_device_url=self.base_url if self.on_device else None,
        )
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
            # The configured provider is the complete approved HTTP route.
            # Neither OS/environment proxies nor another redirect origin may
            # receive prompts, private tool context or provider credentials.
            opener = urllib.request.build_opener(
                urllib.request.ProxyHandler({}),
                _SameOriginRedirectHandler(self.base_url),
            )
            response = opener.open(request, timeout=request_timeout)
            self._active_response = response
            if self._cancelled.is_set():
                response.close()
                raise AppError("AI request cancelled", code="ai_cancelled")
            return response
        except urllib.error.HTTPError as exc:
            raise _http_error_app_error(exc) from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise _network_error_app_error(exc) from exc

    def list_models(self, *, strict: bool = False) -> list[dict]:
        """`GET /v1/models`.

        Default mode (``strict=False``) is forgiving: a 4xx response is
        treated as "this provider doesn't expose `/v1/models`" and returns
        ``[]`` so the picker can fall back to the configured default
        model. Some OpenAI-compatible servers and custom proxies skip the
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
            row.update(http_model_capabilities(item))
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

        context = _request_context(messages, context)
        body = _responses_options(options)
        if not self._compat.reasoning:
            body.pop("reasoning", None)
        reasoning = body.get("reasoning")
        if (
            self._compat.include
            and isinstance(reasoning, dict)
            and reasoning.get("effort")
        ):
            # Nothing is stored, so the next tool round can only carry this
            # round's reasoning forward as encrypted content.
            body["include"] = [RESPONSES_REASONING_INCLUDE]
        if tools is not None:
            body["tools"] = _responses_tools(tools)
        if tool_choice is not None:
            body["tool_choice"] = _responses_tool_choice(tool_choice)
        body.update(
            {
                "model": model,
                "input": _replayable_input_items(
                    context.input_items, keep_reasoning=self._compat.reasoning
                ),
                "stream": stream,
                # Kassiber is local-first. Provider-side response storage may
                # never be enabled by a caller-supplied option.
                "store": False,
            }
        )
        if context.instructions:
            body["instructions"] = context.instructions
        return body

    def _chat_completions_body(
        self,
        *,
        context: ResponsesRequestContext,
        model: str,
        stream: bool,
        options: dict[str, Any] | None,
        tools: list[dict[str, Any]] | None,
        tool_choice: str | dict[str, Any] | None,
    ) -> dict[str, Any]:
        """Build a minimal Chat Completions request body.

        Only what the turn needs: model, messages, stream, the tools and
        ``tool_choice`` when there are tools, generation options the caller
        set, and the reasoning effort. No ``store``: OpenAI does not store a
        Chat Completion unless asked to, and strict endpoints refuse fields
        they do not know.
        """

        options = dict(options or {})
        body: dict[str, Any] = {
            "model": model,
            "messages": chat_completions_messages(
                context, replay_reasoning=self._compat.reasoning
            ),
            "stream": stream,
        }
        for key in _CHAT_COMPLETIONS_OPTIONS:
            if options.get(key) is not None:
                body[key] = options[key]
        max_tokens = options.get("max_tokens")
        if max_tokens is None:
            max_tokens = options.get("max_output_tokens")
        if max_tokens is not None:
            # Most servers know only max_tokens; OpenAI's reasoning models
            # refuse it and name max_completion_tokens, which a retry sends.
            key = "max_completion_tokens" if self._compat.max_completion_tokens else "max_tokens"
            body[key] = max_tokens
        definitions = _chat_completions_tools(tools or [])
        if definitions:
            body["tools"] = definitions
            body["tool_choice"] = _chat_completions_tool_choice(tool_choice)
        effort = http_reasoning_effort(options.get("reasoning_effort"))
        if effort is not None and self._compat.reasoning:
            if _is_openrouter(self.base_url):
                # OpenRouter documents its unified reasoning object here.
                body["reasoning"] = {"effort": effort}
            else:
                body["reasoning_effort"] = effort
        return body

    def _prepare_responses(self, **request: Any) -> _PreparedRequest:
        body = self._request_body(**request)
        reasoning = body.get("reasoning")
        effort_sent = isinstance(reasoning, dict) and bool(reasoning.get("effort"))
        return _PreparedRequest(
            path="responses",
            body=body,
            include_sent="include" in body,
            reasoning_sent="reasoning" in body
            or any(
                isinstance(item, dict) and item.get("type") == "reasoning"
                for item in body["input"]
            ),
            effort_sent=effort_sent,
        )

    def _prepare_chat_completions(self, **request: Any) -> _PreparedRequest:
        body = self._chat_completions_body(**request)
        effort_sent = "reasoning_effort" in body or "reasoning" in body
        replayed = any(
            isinstance(message, dict)
            and any(key in message for key in ("reasoning_details", *_CHAT_REASONING_FIELDS))
            for message in body["messages"]
        )
        return _PreparedRequest(
            path="chat/completions",
            body=body,
            reasoning_sent=effort_sent or replayed,
            effort_sent=effort_sent,
            max_tokens_sent="max_tokens" in body,
        )

    def _prepare(self, api_format: str, **request: Any) -> _PreparedRequest:
        if api_format == AI_API_FORMAT_CHAT_COMPLETIONS:
            return self._prepare_chat_completions(**request)
        return self._prepare_responses(messages=None, **request)

    def _relax_refused_parts(
        self, exc: AppError, prepared: _PreparedRequest
    ) -> str | None:
        """Drop an optional part the endpoint refused and name it, else None.

        Only a 400/422 that names the part counts, and each part is dropped
        once for the client, so a request is retried at most once per part.
        """

        if _error_status(exc) not in (400, 422):
            return None
        text = _error_text(exc)
        if (
            prepared.max_tokens_sent
            and "max_completion_tokens" in text
            and not self._compat.max_completion_tokens
        ):
            self._compat.max_completion_tokens = True
            return "max_tokens"
        if prepared.include_sent and (
            "include" in text or "encrypted_content" in text
        ):
            self._compat.include = False
            return "include"
        if prepared.reasoning_sent and any(
            word in text for word in _REASONING_REFUSAL_WORDS
        ):
            self._compat.reasoning = False
            return "reasoning"
        return None

    def _post_generation(
        self,
        prepare: Any,
        *,
        stream: bool,
        timeout: float | None,
    ) -> tuple[Any, _PreparedRequest, list[str]]:
        """POST a generation request, retrying without parts it refuses.

        Returns the open response, the request that was accepted, and the
        notices the caller reports: a refused reasoning effort answered
        without it is ``reasoning_effort_unavailable``.
        """

        notices: list[str] = []
        while True:
            prepared = prepare()
            try:
                response = self._open(
                    prepared.path,
                    method="POST",
                    body=json.dumps(prepared.body).encode("utf-8"),
                    accept_sse=stream,
                    timeout=timeout,
                )
            except AppError as exc:
                refused = self._relax_refused_parts(exc, prepared)
                if refused is None:
                    raise
                if refused == "reasoning" and prepared.effort_sent:
                    notices.append("reasoning_effort_unavailable")
                continue
            return response, prepared, notices

    def _may_fall_back(self) -> bool:
        """Only an auto provider with nothing detected, before any answer."""

        return (
            self.api_format == AI_API_FORMAT_AUTO
            and self.detected_api_format is None
            and self._wire_format is None
        )

    def _start(
        self,
        *,
        context: ResponsesRequestContext,
        model: str,
        stream: bool,
        options: dict[str, Any] | None,
        tools: list[dict[str, Any]] | None,
        tool_choice: str | dict[str, Any] | None,
        timeout: float | None,
        api_format: str | None = None,
    ) -> tuple[str, Any, list[str]]:
        """Send one generation request; returns its format, response and notices.

        An ``auto`` provider with no detected format starts with the Responses
        API. If the server has no such path (404/405), the same request goes
        once to Chat Completions at the same base URL, the endpoint the user
        already chose; no other error falls back, and neither does an explicit
        format.
        """

        wire_format = api_format or self.wire_format()

        def send(chosen: str) -> tuple[Any, list[str]]:
            response, _prepared, notices = self._post_generation(
                lambda: self._prepare(
                    chosen,
                    context=context,
                    model=model,
                    stream=stream,
                    options=options,
                    tools=tools,
                    tool_choice=tool_choice,
                ),
                stream=stream,
                timeout=timeout,
            )
            return response, notices

        try:
            response, notices = send(wire_format)
        except AppError as exc:
            if not (
                api_format is None
                and wire_format == AI_API_FORMAT_RESPONSES
                and self._may_fall_back()
                and route_missing(exc)
            ):
                raise
            wire_format = AI_API_FORMAT_CHAT_COMPLETIONS
            try:
                response, notices = send(wire_format)
            except AppError as fallback_exc:
                if route_missing(fallback_exc):
                    raise AppError(
                        "AI provider has neither a Responses nor a Chat Completions endpoint at this base URL",
                        code="ai_request_invalid",
                        details=fallback_exc.details,
                        hint="Check that the base URL is the API root, usually ending in /v1.",
                        retryable=False,
                    ) from fallback_exc
                raise
            self._fell_back = True
        self._wire_format = wire_format
        return wire_format, response, notices

    def _note_answer(self, wire_format: str) -> None:
        """An answer arrived; after the auto fallback that is the detection."""

        if self._fell_back and wire_format == AI_API_FORMAT_CHAT_COMPLETIONS:
            self.newly_detected_api_format = AI_API_FORMAT_CHAT_COMPLETIONS

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
        """One blocking answer, normalized for the daemon in either format."""

        context = _request_context(messages, context)
        wire_format, response, notices = self._start(
            context=context,
            model=model,
            stream=False,
            options=options,
            tools=tools,
            tool_choice=tool_choice,
            timeout=timeout,
        )
        self.notices.extend(phase for phase in notices if phase not in self.notices)
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
                "AI provider returned an invalid response payload",
                code="ai_request_invalid",
                retryable=False,
            )
        if wire_format == AI_API_FORMAT_CHAT_COMPLETIONS:
            result = self._chat_completions_result(payload)
        else:
            result = self._responses_result(payload)
        self._note_answer(wire_format)
        return result

    @staticmethod
    def _responses_result(payload: dict[str, Any]) -> dict[str, Any]:
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

    def _chat_completions_result(self, payload: dict[str, Any]) -> dict[str, Any]:
        error = _chat_completion_stream_error(payload)
        if error is not None:
            raise error
        choices = payload.get("choices")
        choice = (
            choices[0]
            if isinstance(choices, list) and choices and isinstance(choices[0], dict)
            else None
        )
        if choice is None:
            raise AppError(
                "AI provider returned a Chat Completions payload without choices",
                code="ai_request_invalid",
                retryable=False,
            )
        message = choice.get("message") if isinstance(choice.get("message"), dict) else {}
        refusal = message.get("refusal")
        content = _chat_text(message.get("content")) + (
            refusal if isinstance(refusal, str) else ""
        )
        reasoning_field = next(
            (
                name
                for name in _CHAT_REASONING_FIELDS
                if isinstance(message.get(name), str) and message[name]
            ),
            None,
        )
        reasoning = message[reasoning_field] if reasoning_field else ""
        raw_details = message.get("reasoning_details")
        details = (
            [detail for detail in raw_details if isinstance(detail, dict)]
            if isinstance(raw_details, list)
            else []
        )
        self._answers += 1
        tool_calls = _normalized_tool_calls(
            message.get("tool_calls"), id_prefix=f"call_{self._answers}_"
        )
        finish_reason = _chat_finish_reason(choice.get("finish_reason"), bool(tool_calls))
        if finish_reason in ("length", "content_filter"):
            tool_calls = []
        result: dict[str, Any] = {
            "role": "assistant",
            "content": content,
            "finish_reason": finish_reason,
            "usage": payload.get("usage"),
            "response_output": _chat_completion_output(
                content=content,
                reasoning=reasoning,
                reasoning_field=reasoning_field,
                reasoning_details=details,
                tool_calls=tool_calls,
            ),
        }
        if reasoning:
            result["reasoning"] = reasoning
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
        """Stream one answer as provider-neutral ChatDelta events.

        Both formats emit the same events: content and reasoning deltas,
        tool-call snapshots, notices, and one terminal chunk carrying the
        finish reason and the output Items the tool loop replays.
        """

        yield from self._stream(
            context=_request_context(messages, context),
            model=model,
            options=options,
            tools=tools,
            tool_choice=tool_choice,
            timeout=self.timeout,
        )

    def _stream(
        self,
        *,
        context: ResponsesRequestContext,
        model: str,
        options: dict[str, Any] | None,
        tools: list[dict[str, Any]] | None,
        tool_choice: str | dict[str, Any] | None,
        timeout: float | None,
        api_format: str | None = None,
    ) -> Iterator[ChatDelta]:
        wire_format, response, notices = self._start(
            context=context,
            model=model,
            stream=True,
            options=options,
            tools=tools,
            tool_choice=tool_choice,
            timeout=timeout,
            api_format=api_format,
        )
        try:
            with response:
                for phase in notices:
                    yield ChatDelta(delta={}, finish_reason=None, raw={}, status_phase=phase)
                lines = (raw.decode("utf-8", errors="replace") for raw in response)
                if wire_format == AI_API_FORMAT_CHAT_COMPLETIONS:
                    yield from self._chat_completions_deltas(lines)
                else:
                    yield from self._responses_deltas(lines)
            if not self._cancelled.is_set():
                self._note_answer(wire_format)
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise _network_error_app_error(exc) from exc
        finally:
            self._active_response = None

    def probe(
        self,
        *,
        model: str,
        timeout: float = CONNECTION_TEST_TIMEOUT_SECONDS,
    ) -> dict[str, Any]:
        """Send the connection test's fixed request; returns what answered.

        ``auto`` tries the Responses API first and Chat Completions when the
        endpoint does not speak it; an explicit format is tried alone. The
        request carries `CONNECTION_TEST_TOOL`; an endpoint that refuses it
        is asked once more without, and ``tools_accepted`` says which.
        """

        formats = (
            [self.api_format]
            if self.api_format in AI_WIRE_API_FORMATS
            else list(AI_WIRE_API_FORMATS)
        )
        for position, wire_format in enumerate(formats):
            try:
                return self._probe_in(wire_format, model=model, timeout=timeout)
            except AppError as exc:
                if position + 1 < len(formats) and _format_not_served(exc):
                    continue
                raise
        raise AssertionError("unreachable")  # pragma: no cover

    def _probe_in(self, wire_format: str, *, model: str, timeout: float) -> dict[str, Any]:
        try:
            finish_reason = self._probe_once(wire_format, model=model, timeout=timeout, with_tool=True)
            tools_accepted = True
        except AppError as exc:
            if not _tools_refused(exc):
                raise
            finish_reason = self._probe_once(wire_format, model=model, timeout=timeout, with_tool=False)
            tools_accepted = False
        return {
            "api_format": wire_format,
            "tools_accepted": tools_accepted,
            "finish_reason": finish_reason,
        }

    def _probe_once(
        self, wire_format: str, *, model: str, timeout: float, with_tool: bool
    ) -> str | None:
        terminal: ChatDelta | None = None
        for chunk in self._stream(
            context=ResponsesRequestContext(
                instructions=None,
                input_items=[
                    {"type": "message", "role": "user", "content": CONNECTION_TEST_PROMPT}
                ],
            ),
            model=model,
            options={"max_tokens": CONNECTION_TEST_MAX_TOKENS},
            tools=[CONNECTION_TEST_TOOL] if with_tool else None,
            tool_choice="auto" if with_tool else None,
            timeout=timeout,
            api_format=wire_format,
        ):
            if chunk.response_output is not None:
                terminal = chunk
        if terminal is None:
            raise AppError(
                "AI provider did not finish an answer to the connection test",
                code="ai_request_invalid",
                details={"reason": "no_answer", "api_format": wire_format},
                hint="Check the base URL and the provider's API format.",
                retryable=False,
            )
        # Even a length stop proves the endpoint answers in this format.
        return terminal.finish_reason

    def _no_stream_events(self) -> AppError:
        return AppError(
            "AI provider answered without any stream events",
            code="ai_request_invalid",
            details={"reason": "no_stream_events"},
            hint="Check the base URL and the provider's API format.",
            retryable=False,
        )

    def _responses_deltas(self, lines: Iterable[str]) -> Iterator[ChatDelta]:
        tool_call_accumulator = ResponsesToolCallAccumulator()
        emitted_content = False
        emitted_reasoning = False
        received = False
        for event in _sse_payloads(lines):
            if event is _SSE_DONE:
                return
            received = True
            if self._cancelled.is_set():
                return
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
                # OpenRouter's documented reasoning stream event.
                "response.reasoning.delta",
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

            # An answer cut off by max_output_tokens or a content filter ends
            # with response.incomplete instead.
            completed = (
                event.get("response")
                if event_type in {"response.completed", "response.incomplete"}
                else None
            )
            response_output = _response_output(completed) if completed is not None else None
            finish_reason = _response_finish_reason(completed) if completed is not None else None
            if completed is not None:
                had_streamed_calls = bool(tool_call_accumulator.snapshot())
                calls = tool_call_accumulator.replace_output_items(response_output or [])
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
        if not received and not self._cancelled.is_set():
            raise self._no_stream_events()

    def _chat_completions_deltas(self, lines: Iterable[str]) -> Iterator[ChatDelta]:
        self._answers += 1
        calls = ChatCompletionsToolCallAccumulator(id_prefix=f"call_{self._answers}_")
        details = _ReasoningDetailsAccumulator()
        content_parts: list[str] = []
        reasoning_parts: list[str] = []
        reasoning_field: str | None = None
        finish: str | None = None
        usage: Any = None
        streamed_calls = False
        received = False
        ended = False
        for event in _sse_payloads(lines):
            if event is _SSE_DONE:
                ended = True
                break
            received = True
            if self._cancelled.is_set():
                return
            error = _chat_completion_stream_error(event)
            if error is not None:
                raise error
            if isinstance(event.get("usage"), dict):
                usage = event["usage"]
            choices = event.get("choices")
            choice = (
                choices[0]
                if isinstance(choices, list) and choices and isinstance(choices[0], dict)
                else None
            )
            if choice is None:
                # OpenAI's usage chunk carries no choices.
                continue
            piece = choice.get("delta") if isinstance(choice.get("delta"), dict) else {}
            delta: dict[str, Any] = {}
            text = "".join(
                value
                for value in (piece.get("content"), piece.get("refusal"))
                if isinstance(value, str)
            )
            if text:
                delta["content"] = text
                content_parts.append(text)
            for name in _CHAT_REASONING_FIELDS:
                value = piece.get(name)
                if isinstance(value, str) and value:
                    delta["reasoning"] = value
                    reasoning_parts.append(value)
                    reasoning_field = reasoning_field or name
                    break
            details.add(piece.get("reasoning_details"))
            if piece.get("tool_calls") is not None:
                snapshot = calls.add(piece.get("tool_calls"))
                if snapshot:
                    delta["tool_calls"] = snapshot
                    streamed_calls = True
            reason = choice.get("finish_reason")
            if isinstance(reason, str) and reason:
                if reason == "error":
                    # OpenRouter ends a failed stream this way; the error object
                    # itself is handled above when present.
                    raise _provider_error({"message": "finish_reason error"})
                finish = reason
            if delta:
                yield ChatDelta(delta=delta, finish_reason=None, raw=event)
        if self._cancelled.is_set():
            return
        if not received and not ended:
            raise self._no_stream_events()
        if not ended and finish is None:
            # Cut off before a finish reason or [DONE]: no terminal chunk, as
            # for a Responses stream that never completes.
            return
        final_calls = calls.snapshot()
        finish_reason = _chat_finish_reason(finish, bool(final_calls))
        if finish_reason in ("length", "content_filter"):
            # The message was cut off, so its last call may be too; none runs.
            final_calls = []
        terminal: dict[str, Any] = {}
        if final_calls or streamed_calls:
            terminal["tool_calls"] = final_calls
        yield ChatDelta(
            delta=terminal,
            finish_reason=finish_reason,
            raw={"usage": usage} if usage is not None else {},
            response_output=_chat_completion_output(
                content="".join(content_parts),
                reasoning="".join(reasoning_parts),
                reasoning_field=reasoning_field,
                reasoning_details=details.result(),
                tool_calls=final_calls,
            ),
        )


def ai_client_for_locator(
    base_url: str,
    *,
    api_key: str | None = None,
    timeout: float = DEFAULT_TIMEOUT_SECONDS,
    kind: str | None = None,
    api_format: str | None = None,
    detected_api_format: str | None = None,
):
    """Build the client for a provider locator.

    ``kind`` is the provider's privacy kind. Only ``"local"`` lets an HTTP
    provider through offline mode; omit it and the client counts as remote.
    ``api_format`` and ``detected_api_format`` choose the HTTP wire format;
    CLI locators ignore them.
    """
    if is_cli_provider_locator(base_url):
        return BrokerAIClient(locator=base_url, timeout=timeout)
    return OpenAICompatibleClient(
        base_url=base_url,
        api_key=api_key,
        timeout=timeout,
        on_device=kind == "local",
        api_format=api_format or AI_API_FORMAT_AUTO,
        detected_api_format=detected_api_format,
    )
