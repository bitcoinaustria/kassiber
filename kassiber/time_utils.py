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
`parse_timestamp`, `_parse_iso_datetime`, or `parse_iso_datetime_or_none`
so users get the intended validation-or-best-effort behavior.
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


def iso_to_unix(value):
    """Convert an optional ISO/RFC3339 timestamp to Unix seconds.

    Empty values mean "from genesis" for Core rescans and return ``0``.
    """
    if value in (None, ""):
        return 0
    parsed = parse_timestamp(value)
    raw = parsed[:-1] + "+00:00" if parsed.endswith("Z") else parsed
    return int(datetime.fromisoformat(raw).timestamp())


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


def parse_iso_datetime_or_none(value):
    """Best-effort ISO/RFC3339 parser for persisted daemon timestamps.

    Returns a timezone-aware UTC `datetime`, or `None` when the persisted
    value is empty or malformed. User-facing parsers should keep using
    `parse_timestamp` / `_parse_iso_datetime` so validation errors stay
    explicit.
    """
    if value in (None, ""):
        return None
    raw = str(value).strip()
    if not raw:
        return None
    if raw.endswith("Z"):
        raw = raw[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(raw)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _iso_z(dt):
    """Format a UTC `datetime` back to the canonical Z-suffixed string."""
    return dt.isoformat().replace("+00:00", "Z")


def timestamp_microseconds(value):
    """Exact instant for SQL bounds; malformed persisted values remain unknown.

    Use the persisted-timestamp parser and integer arithmetic rather than a
    floating Unix timestamp or SQLite's millisecond-resolution date functions.
    """
    try:
        parsed = parse_iso_datetime_or_none(value)
    except (OverflowError, ValueError):
        return None
    if parsed is None:
        return None
    return ((parsed.toordinal() - 1) * 86400 + parsed.hour * 3600
            + parsed.minute * 60 + parsed.second) * 1_000_000 + parsed.microsecond


def timestamp_predicate(conn, column, operator):
    """SQL for a timestamp bound; column/operator are internal SQL literals.

    Registration also supports disposable in-memory service connections. No
    schema or stored timestamp is changed. NULL excludes malformed values from
    bounded reads instead of guessing where they belong in time.
    """
    # Replacing a function while another statement is reading it is refused by
    # SQLite. Register once per connection, including raw service test stores.
    if not conn.execute(
        "SELECT 1 FROM pragma_function_list WHERE name = 'kassiber_timestamp_us'"
    ).fetchone():
        conn.create_function("kassiber_timestamp_us", 1, timestamp_microseconds, deterministic=True)
    return f"kassiber_timestamp_us({column}) {operator} kassiber_timestamp_us(?)"
