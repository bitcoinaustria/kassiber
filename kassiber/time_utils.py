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
`parse_legacy_timestamp` is only for re-reading values stored before their
write path validated them.
"""

import re
from datetime import datetime, timedelta, timezone
from email.utils import parsedate_tz

from .errors import AppError


UNKNOWN_OCCURRED_AT = "1970-01-01T00:00:00Z"

_LEGACY_TIMESTAMP = re.compile(
    r"(?P<year>\d{4})-?(?P<month>\d{2})-?(?P<day>\d{2})"
    r"(?:[T ](?P<hour>\d{2})(?::?(?P<minute>\d{2})"
    r"(?::?(?P<second>\d{2})(?:[.,](?P<fraction>\d+))?)?)?"
    r"\s*(?P<zone>Z|UTC|GMT|[+-]\d{2}(?::?\d{2})?)?)?"
)


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


_RFC2822_ZONE = re.compile(r"[+-]\d{4}|GMT|UT|UTC|Z")
_RFC2822_COLON_OFFSET = re.compile(r"[+-]\d{2}:\d{2}")


def _parse_rfc2822(raw):
    """An RFC 2822 date (`Mon, 01 Sep 2025 12:00:30 +0000`) as canonical UTC, or None.

    Only a zone that fixes the instant is accepted: a numeric offset (with or
    without a colon), GMT/UT/UTC/Z, or `-0000`. The standard library reads
    any other zone token as no zone at all, which would shift the instant.
    """
    tokens = raw.split()
    if not tokens:
        return None
    zone = tokens[-1].upper()
    if _RFC2822_COLON_OFFSET.fullmatch(zone):
        zone = zone.replace(":", "")
    if not _RFC2822_ZONE.fullmatch(zone):
        return None
    parsed = parsedate_tz(" ".join([*tokens[:-1], zone]))
    if parsed is None:
        return None
    offset = parsed[9]
    if offset is None and zone != "-0000":
        return None
    try:
        dt = datetime(*parsed[:6], tzinfo=timezone(timedelta(seconds=offset or 0)))
    except (ValueError, OverflowError):
        return None
    return _iso_z(dt.astimezone(timezone.utc))


def parse_legacy_timestamp(value):
    """Canonicalize a stored timestamp that predates write-time validation.

    Older reviews stored some typed timestamps (a direct payout's
    `payout_occurred_at`) verbatim. On top of what `parse_timestamp` takes,
    this accepts the forms a user plausibly typed and RP2's dateutil parsing
    accepted: surrounding whitespace, lowercase `t`/`z`, a space before the
    offset or `Z`, a trailing `UTC`/`GMT`, the basic format
    (`20250301T090030Z`), and RFC 2822 (`Mon, 01 Sep 2025 12:00:30 +0000`).
    Returns the canonical RFC3339 UTC string and raises the same validation
    `AppError` as `parse_timestamp` for anything else, including the
    day/month-ambiguous forms dateutil guessed at (`01.09.2025`).
    """
    raw = str(value or "").strip()
    match = _LEGACY_TIMESTAMP.fullmatch(raw.upper())
    if match is None:
        try:
            return parse_timestamp(raw)
        except AppError:
            rfc2822 = _parse_rfc2822(raw)
            if rfc2822 is None:
                raise
            return rfc2822
    parts = match.groupdict()
    text = f"{parts['year']}-{parts['month']}-{parts['day']}"
    if parts["hour"] is not None:
        text += f"T{parts['hour']}:{parts['minute'] or '00'}:{parts['second'] or '00'}"
        if parts["fraction"]:
            text += "." + parts["fraction"][:6].ljust(6, "0")
        zone = parts["zone"]
        if zone in ("Z", "UTC", "GMT"):
            text += "+00:00"
        elif zone:
            digits = zone[1:].replace(":", "")
            text += f"{zone[0]}{digits[:2]}:{digits[2:4] or '00'}"
    try:
        return parse_timestamp(text)
    except AppError as exc:
        # Name the stored value, not the rebuilt one (e.g. month 13).
        raise AppError(
            f"Invalid occurred_at/date value '{value}'",
            code="validation",
            hint="Use RFC3339 UTC like 2025-01-01T00:00:00Z",
        ) from exc


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
