"""Small retry/backoff parsing helpers."""

from __future__ import annotations

from datetime import datetime, timezone
from email.utils import parsedate_to_datetime
from typing import Any, Mapping


def retry_after_seconds(value: Any, *, now: datetime | None = None) -> int | None:
    """Parse an HTTP Retry-After header value into non-negative seconds."""
    if value in (None, ""):
        return None
    raw = str(value).strip()
    if not raw:
        return None
    try:
        return max(0, int(raw))
    except ValueError:
        pass
    try:
        parsed = parsedate_to_datetime(raw)
    except (TypeError, ValueError, IndexError, OverflowError):
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    reference = now or datetime.now(parsed.tzinfo)
    if reference.tzinfo is None:
        reference = reference.replace(tzinfo=timezone.utc)
    reference = reference.astimezone(parsed.tzinfo)
    return max(0, int((parsed - reference).total_seconds()))


def retry_after_seconds_from_headers(headers: Mapping[str, Any] | None) -> int | None:
    if headers is None:
        return None
    getter = getattr(headers, "get", None)
    if getter is None:
        return None
    return retry_after_seconds(getter("Retry-After"))


def retry_after_seconds_from_http_error(exc: Any) -> int | None:
    return retry_after_seconds_from_headers(getattr(exc, "headers", None))
