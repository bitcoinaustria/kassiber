"""Timestamp helpers used across kassiber.

Kassiber stores and emits timestamps as RFC3339 UTC strings with a trailing
`Z` (no fractional seconds). These helpers are the single place that
converts between:

  - arbitrary ISO 8601 strings users paste in (with or without Z, with or
    without fractional seconds, with or without a date-only form)
  - `datetime` instances in UTC
  - the canonical `YYYY-MM-DDTHH:MM:SSZ` string stored in the DB and
    returned in envelopes

Call sites should never call `datetime.fromisoformat` directly; use
`parse_timestamp` or `_parse_iso_datetime` so users get a consistent
validation error envelope.
"""

from datetime import datetime, timezone

from .errors import AppError


UNKNOWN_OCCURRED_AT = "1970-01-01T00:00:00Z"


def now_iso():
    """Current UTC time as an RFC3339 Z-suffixed string (no microseconds)."""
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def parse_timestamp(value):
    """Lenient ISO parser for user/wallet import input.

    Accepts date-only (`YYYY-MM-DD`), `Z`-suffixed, or offset forms, and
    returns the canonical RFC3339 UTC Z-suffixed second-precision string.

    Raises `AppError` (surfaced as a validation envelope) if the value is
    empty or unparseable.
    """
    if not value:
        raise AppError("Missing occurred_at/date value")
    raw = str(value).strip()
    if len(raw) == 10:
        raw = f"{raw}T00:00:00+00:00"
    elif raw.endswith("Z"):
        raw = raw[:-1] + "+00:00"
    try:
        dt = datetime.fromisoformat(raw)
    except ValueError as exc:
        raise AppError(
            f"Invalid occurred_at/date value '{value}'",
            code="validation",
            hint="Use RFC3339 UTC like 2025-01-01T00:00:00Z",
        ) from exc
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    else:
        dt = dt.astimezone(timezone.utc)
    return dt.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def timestamp_to_iso(value, default=UNKNOWN_OCCURRED_AT):
    """Convert a unix-epoch integer (or integer-like) to RFC3339 UTC.

    Returns `default` when the input is falsy — used to represent
    unknown / un-confirmed block times without breaking downstream
    timestamp-comparisons.
    """
    if value in (None, "", 0, "0"):
        return default
    return datetime.fromtimestamp(int(value), tz=timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _parse_iso_datetime(value, field_name):
    """Strict ISO parser for CLI filter flags (e.g. `--start`, `--end`).

    Returns a timezone-aware UTC `datetime`, or `None` for empty input.
    Raises `AppError(code='validation')` with an RFC3339 hint on parse
    failure so the user gets an actionable error envelope.
    """
    if value in (None, ""):
        return None
    raw = value.strip()
    if raw.endswith("Z"):
        raw = raw[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(raw)
    except ValueError as exc:
        raise AppError(
            f"Invalid {field_name} timestamp '{value}'",
            code="validation",
            hint="Use RFC3339 UTC like 2025-01-01T00:00:00Z",
        ) from exc
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _iso_z(dt):
    """Format a UTC `datetime` back to the canonical Z-suffixed string."""
    return dt.isoformat().replace("+00:00", "Z")
