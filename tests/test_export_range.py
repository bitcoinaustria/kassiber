"""Export date ranges resolve in the book's calendar."""

from __future__ import annotations

import unittest
from datetime import date, datetime, timezone

from kassiber.core.export_range import resolve_export_range
from kassiber.errors import AppError

AT = {"tax_country": "at"}
GENERIC = {"tax_country": "generic"}
# 23:30 in Vienna on New Year's Eve.
# 22:30Z on 31 December is 23:30 in Vienna, still 2025 locally.
NOW = datetime(2025, 12, 31, 22, 30, tzinfo=timezone.utc)


def _range(profile, **kwargs):
    return resolve_export_range(profile, now=NOW, **kwargs)


class ExportRangeTests(unittest.TestCase):
    def test_no_range_is_every_transaction(self):
        self.assertIsNone(_range(AT))
        self.assertIsNone(_range(AT, preset="all"))

    def test_presets_follow_the_book_calendar(self):
        month = _range(AT, preset="this_month")
        self.assertEqual((month.first_day, month.last_day), (date(2025, 12, 1), date(2025, 12, 31)))
        # Vienna midnight is 23:00Z in winter.
        self.assertEqual(month.payload()["start"], "2025-11-30T23:00:00Z")
        self.assertEqual(month.payload()["end"], "2025-12-31T23:00:00Z")
        self.assertEqual(month.timezone, "Europe/Vienna")
        self.assertEqual(month.label, "2025-12")
        last_quarter = _range(AT, preset="last_quarter")
        self.assertEqual((last_quarter.first_day, last_quarter.last_day), (date(2025, 7, 1), date(2025, 9, 30)))
        self.assertEqual(last_quarter.label, "2025-Q3")
        self.assertEqual(_range(AT, preset="last_year").label, "2024")
        week = _range(AT, preset="this_week")
        self.assertEqual(week.first_day.weekday(), 0)
        self.assertEqual(_range(GENERIC, preset="this_year").payload()["start"], "2025-01-01T00:00:00Z")

    def test_the_day_belongs_to_the_calendar_not_utc(self):
        # 23:30Z on 31 December 2025 is already 2026 in Vienna.
        at_2026 = _range(AT, first_day="2026-01-01", last_day="2026-01-01")
        self.assertTrue(at_2026.contains("2025-12-31T23:30:00Z"))
        self.assertFalse(_range(GENERIC, first_day="2026-01-01", last_day="2026-01-01").contains("2025-12-31T23:30:00Z"))
        self.assertFalse(at_2026.contains("2026-01-01T23:00:00Z"))

    def test_an_undated_transaction_may_belong_to_any_period(self):
        # UNKNOWN_OCCURRED_AT is missing evidence, not a transaction in 1970.
        year = _range(AT, preset="this_year")
        self.assertFalse(year.contains("1970-01-01T00:00:00Z"))
        self.assertTrue(year.may_contain("1970-01-01T00:00:00Z"))
        self.assertTrue(year.leaves_out_undated("1970-01-01T00:00:00Z"))
        self.assertTrue(year.may_contain("2025-06-01T10:00:00Z"))
        self.assertFalse(year.may_contain("2024-06-01T10:00:00Z"))
        self.assertFalse(year.leaves_out_undated("2024-06-01T10:00:00Z"))
        self.assertIn("2 transactions", year.undated_warning(2))

    def test_custom_spans_and_timezones_are_validated(self):
        span = _range(AT, preset="custom", first_day="2025-03-01", last_day="2025-03-15", timezone_name="UTC")
        self.assertEqual(span.payload()["start"], "2025-03-01T00:00:00Z")
        self.assertEqual(span.label, "2025-03-01_2025-03-15")
        for kwargs in (
            {"first_day": "2025-03-15", "last_day": "2025-03-01"},
            {"first_day": "2025-03-01"},
            {"preset": "fortnight"},
            {"preset": "this_month", "timezone_name": "Mars/Olympus"},
            {"first_day": "01.03.2025", "last_day": "2025-03-02"},
        ):
            with self.subTest(**kwargs), self.assertRaises(AppError):
                _range(AT, **kwargs)


if __name__ == "__main__":
    unittest.main()
