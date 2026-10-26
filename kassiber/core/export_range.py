"""Date ranges for exports, in the book's own calendar.

An export asks for "last month" or two calendar dates; this turns that into
exact UTC bounds. The calendar is the book's (Europe/Vienna for Austrian
books, UTC otherwise, the same as the year handoff) unless the caller names
another IANA timezone, so "this year" means the same thing in the desktop, the
CLI and every report. The start is inclusive and the end exclusive; weeks
start on Monday. No range means every transaction.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import date, datetime, timedelta, timezone
import re
from typing import Any, Mapping
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from ..errors import AppError
from ..time_utils import UNKNOWN_OCCURRED_AT
from .year_handoff import _period_for_profile

PRESETS = (
    "today",
    "yesterday",
    "this_week",
    "last_week",
    "this_month",
    "last_month",
    "this_quarter",
    "last_quarter",
    "this_year",
    "last_year",
)

_DAY = re.compile(r"\d{4}-\d{2}-\d{2}")


@dataclass(frozen=True)
class ExportRange:
    preset: str
    timezone: str
    start: datetime
    end: datetime
    first_day: date
    last_day: date

    def contains(self, occurred_at: Any) -> bool:
        if not str(occurred_at or ""):
            return False
        return self.start <= _moment(occurred_at) < self.end

    # A transaction with no known time is stored at UNKNOWN_OCCURRED_AT: missing
    # evidence, not a transaction in 1970. It may belong to any period.
    def may_contain(self, occurred_at: Any) -> bool:
        """In the period, or undated and so possibly in it: for fail-closed checks."""
        return is_undated(occurred_at) or self.contains(occurred_at)

    def precedes(self, occurred_at: Any) -> bool:
        """A dated transaction from before the period starts."""
        return not is_undated(occurred_at) and _moment(occurred_at) < self.start

    def leaves_out_undated(self, occurred_at: Any) -> bool:
        """An undated transaction this period's file does not list."""
        return is_undated(occurred_at) and not self.contains(occurred_at)

    @staticmethod
    def undated_warning(count: int) -> str:
        rows, verb = ("1 transaction", "is") if count == 1 else (f"{count} transactions", "are")
        return (
            f"{rows} without a known date could belong to this period but {verb} not "
            "in this file. Export All time to include undated transactions."
        )

    @property
    def label(self) -> str:
        """Short file-name label: 2025, 2025-10, 2025-Q3 or a date span."""
        first, last = self.first_day, self.last_day
        if first == date(first.year, 1, 1) and last == date(first.year, 12, 31):
            return str(first.year)
        if first.day == 1 and _add_months(first, 1) - timedelta(days=1) == last:
            return f"{first:%Y-%m}"
        if first.day == 1 and first.month % 3 == 1 and _add_months(first, 3) - timedelta(days=1) == last:
            return f"{first.year}-Q{(first.month - 1) // 3 + 1}"
        if first == last:
            return first.isoformat()
        return f"{first.isoformat()}_{last.isoformat()}"

    def payload(self) -> dict[str, Any]:
        return {
            "preset": self.preset,
            "timezone": self.timezone,
            "start": _iso(self.start),
            "end": _iso(self.end),
            "first_day": self.first_day.isoformat(),
            "last_day": self.last_day.isoformat(),
            "label": self.label,
        }


def _moment(occurred_at: Any) -> datetime:
    moment = datetime.fromisoformat(str(occurred_at).replace("Z", "+00:00"))
    return moment if moment.tzinfo is not None else moment.replace(tzinfo=timezone.utc)


def is_undated(occurred_at: Any) -> bool:
    return str(occurred_at or "") in ("", UNKNOWN_OCCURRED_AT)


def _iso(value: datetime) -> str:
    return value.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _add_months(day: date, months: int) -> date:
    index = day.year * 12 + day.month - 1 + months
    return date(index // 12, index % 12 + 1, 1)


def book_timezone(profile: Mapping[str, Any]) -> ZoneInfo:
    return _period_for_profile(profile, 2000).tz


def _zone(profile: Mapping[str, Any], timezone_name: str | None) -> ZoneInfo:
    if not timezone_name:
        return book_timezone(profile)
    try:
        return ZoneInfo(str(timezone_name))
    except (ZoneInfoNotFoundError, ValueError) as exc:
        raise AppError(
            f"Unknown timezone '{timezone_name}'",
            code="validation",
            hint="Use an IANA timezone name such as Europe/Vienna or UTC.",
            retryable=False,
        ) from exc


# Bounds that survive the timezone conversion and the exclusive end day.
_MIN_YEAR, _MAX_YEAR = 1900, 9998


def _parse_day(value: Any, name: str) -> date:
    # date.fromisoformat also takes week dates and compact forms (2025-W01-1,
    # 20250101); only the documented calendar form is a valid bound.
    text = str(value)
    try:
        if not _DAY.fullmatch(text):
            raise ValueError(text)
        day = date.fromisoformat(text)
    except ValueError as exc:
        raise AppError(
            f"{name} must be a date like 2025-01-31",
            code="validation",
            retryable=False,
        ) from exc
    if not _MIN_YEAR <= day.year <= _MAX_YEAR:
        raise AppError(
            f"{name} must be between {_MIN_YEAR} and {_MAX_YEAR}",
            code="validation",
            retryable=False,
        )
    return day


def _preset_days(preset: str, today: date) -> tuple[date, date]:
    """First and last calendar day (inclusive) of a preset around ``today``."""
    if preset == "today":
        return today, today
    if preset == "yesterday":
        day = today - timedelta(days=1)
        return day, day
    if preset in {"this_week", "last_week"}:
        monday = today - timedelta(days=today.weekday())
        if preset == "last_week":
            monday -= timedelta(days=7)
        return monday, monday + timedelta(days=6)
    if preset in {"this_month", "last_month"}:
        first = date(today.year, today.month, 1)
        if preset == "last_month":
            first = _add_months(first, -1)
        return first, _add_months(first, 1) - timedelta(days=1)
    if preset in {"this_quarter", "last_quarter"}:
        first = date(today.year, (today.month - 1) // 3 * 3 + 1, 1)
        if preset == "last_quarter":
            first = _add_months(first, -3)
        return first, _add_months(first, 3) - timedelta(days=1)
    year = today.year - (1 if preset == "last_year" else 0)
    return date(year, 1, 1), date(year, 12, 31)


def resolve_export_range(
    profile: Mapping[str, Any],
    *,
    preset: str | None = None,
    first_day: Any = None,
    last_day: Any = None,
    timezone_name: str | None = None,
    now: datetime | None = None,
) -> ExportRange | None:
    """The exact bounds of a preset or a custom day span; ``None`` for all time."""
    preset = str(preset or "").strip().lower() or None
    if preset in {None, "all"} and first_day is None and last_day is None:
        return None
    if preset not in {None, "custom"} and (first_day is not None or last_day is not None):
        # Never let a preset silently win over the days the caller also named.
        raise AppError(
            f"Range '{preset}' does not take a first or last day",
            code="validation",
            hint="Send either a preset or custom from/to days, not both.",
            retryable=False,
        )
    zone = _zone(profile, timezone_name)
    if preset == "custom" or (preset is None and (first_day is not None or last_day is not None)):
        if first_day is None or last_day is None:
            raise AppError(
                "A custom range needs both a first and a last day",
                code="validation",
                retryable=False,
            )
        first, last = _parse_day(first_day, "from"), _parse_day(last_day, "to")
        if last < first:
            raise AppError("The range ends before it starts", code="validation", retryable=False)
        preset = "custom"
    elif preset in PRESETS:
        today = (now or datetime.now(timezone.utc)).astimezone(zone).date()
        first, last = _preset_days(preset, today)
    else:
        raise AppError(
            f"Unknown range '{preset}'",
            code="validation",
            hint="Use one of: all, custom, " + ", ".join(PRESETS) + ".",
            retryable=False,
        )
    start = datetime(first.year, first.month, first.day, tzinfo=zone).astimezone(timezone.utc)
    after = last + timedelta(days=1)
    end = datetime(after.year, after.month, after.day, tzinfo=zone).astimezone(timezone.utc)
    return ExportRange(
        preset=preset,
        timezone=str(zone.key),
        start=start,
        end=end,
        first_day=first,
        last_day=last,
    )


__all__ = ["ExportRange", "PRESETS", "book_timezone", "resolve_export_range"]
