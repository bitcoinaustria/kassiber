"""Austrian tax-year boundaries follow Europe/Vienna, not UTC substr."""

from __future__ import annotations

import unittest

from kassiber.core.austrian import tax_year_in_vienna, vienna_tax_year_utc_window


class AustrianTaxYearTests(unittest.TestCase):
    def test_utc_new_years_eve_maps_to_vienna_next_year(self):
        # 2024-12-31 23:30 UTC == 2025-01-01 00:30 Vienna.
        self.assertEqual(tax_year_in_vienna("2024-12-31T23:30:00Z"), 2025)

    def test_utc_new_years_day_afternoon_stays_same_year(self):
        self.assertEqual(tax_year_in_vienna("2025-01-01T12:00:00Z"), 2025)

    def test_vienna_window_covers_boundary(self):
        start, end = vienna_tax_year_utc_window(2025)
        self.assertLess(start, "2024-12-31T23:30:00Z")
        self.assertGreater(end, "2024-12-31T23:30:00Z")
        self.assertEqual(tax_year_in_vienna("2024-12-31T23:30:00Z"), 2025)


if __name__ == "__main__":
    unittest.main()
