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
from dataclasses import dataclass
from typing import Any, Iterable, Iterator

import urllib.error
import urllib.request

from ..errors import AppError


DEFAULT_TIMEOUT_SECONDS = 120
DEFAULT_INACTIVITY_TIMEOUT_SECONDS = 90
SSE_DONE_SENTINEL = "[DONE]"


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

    def _open(self, path: str, *, method: str, body: bytes | None, accept_sse: bool):
        url = f"{self.base_url.rstrip('/')}/{path.lstrip('/')}"
        request = urllib.request.Request(
            url,
            data=body,
            method=method,
            headers=self._headers(json_body=body is not None, accept_sse=accept_sse),
        )
        try:
            return urllib.request.urlopen(request, timeout=self.timeout)
        except urllib.error.HTTPError as exc:
            raise _http_error_app_error(exc) from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise _network_error_app_error(exc) from exc

    def list_models(self) -> list[dict]:
        """Best-effort `GET /v1/models`. Returns ``[]`` on graceful failure."""
        try:
            response = self._open("models", method="GET", body=None, accept_sse=False)
        except AppError as exc:
            if exc.code in ("ai_request_invalid",):
                return []
            raise
        with response:
            try:
                payload = json.loads(response.read().decode("utf-8", errors="replace"))
            except json.JSONDecodeError:
                return []
        data = payload.get("data") if isinstance(payload, dict) else None
        if not isinstance(data, list):
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
            models.append(row)
        return models

    def chat(
        self,
        *,
        messages: list[dict],
        model: str,
        options: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Non-streaming `POST /v1/chat/completions`. Returns the assistant message."""
        body = {"model": model, "messages": messages, "stream": False}
        if options:
            body.update(options)
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
        return {
            "role": message.get("role") or "assistant",
            "content": message.get("content") or "",
            "finish_reason": choice.get("finish_reason"),
            "usage": payload.get("usage") if isinstance(payload, dict) else None,
        }

    def stream_chat(
        self,
        *,
        messages: list[dict],
        model: str,
        options: dict[str, Any] | None = None,
    ) -> Iterator[ChatDelta]:
        """Streaming `POST /v1/chat/completions`. Yields one ChatDelta per SSE chunk."""
        body = {"model": model, "messages": messages, "stream": True}
        if options:
            body.update(options)
        response = self._open(
            "chat/completions",
            method="POST",
            body=json.dumps(body).encode("utf-8"),
            accept_sse=True,
        )
        with response:
            line_iter = (raw.decode("utf-8", errors="replace") for raw in response)
            for chunk in parse_sse_chunks(line_iter):
                choices = chunk.get("choices") if isinstance(chunk, dict) else None
                if not isinstance(choices, list) or not choices:
                    continue
                choice = choices[0] if isinstance(choices[0], dict) else {}
                delta = choice.get("delta")
                if not isinstance(delta, dict):
                    delta = {}
                yield ChatDelta(
                    delta=delta,
                    finish_reason=choice.get("finish_reason"),
                    raw=chunk,
                )
