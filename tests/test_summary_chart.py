import unittest
from decimal import Decimal

from kassiber._pdf_common import register_fonts, require_reportlab
from kassiber.summary_pdf_report import _compact_money, _line_chart, _series_bounds


class SeriesBoundsTests(unittest.TestCase):
    def test_positive_only_series_never_pads_below_zero(self):
        # One large outlier used to drive the padded floor negative.
        low, high = _series_bounds([Decimal("82800")] * 5 + [Decimal("4200000")])
        self.assertEqual(low, Decimal("0"))
        self.assertGreater(high, Decimal("4200000"))

    def test_series_with_real_negative_keeps_negative_floor(self):
        low, _high = _series_bounds([Decimal("-900"), Decimal("0"), Decimal("5100")])
        self.assertLess(low, Decimal("0"))


class CompactMoneyTests(unittest.TestCase):
    def test_sub_unit_keeps_two_decimals(self):
        self.assertEqual(_compact_money("EUR", Decimal("0.03")), "EUR 0.03")

    def test_true_zero_stays_integer(self):
        self.assertEqual(_compact_money("EUR", Decimal("0")), "EUR 0")

    def test_large_values_unchanged(self):
        self.assertEqual(_compact_money("EUR", Decimal("184203.55")), "EUR 184.2k")


class LineChartBoundsTests(unittest.TestCase):
    def setUp(self):
        self.rl = require_reportlab("summary chart test")
        self.rl["summary_fonts"] = register_fonts(self.rl)

    def _poly_ys(self, drawing):
        ys = []
        for shape in drawing.contents:
            if type(shape).__name__ == "PolyLine":
                pts = list(shape.points)
                ys.extend(pts[1::2])
        return ys

    def test_cost_basis_line_stays_within_the_drawing(self):
        # Underwater / early-DCA portfolio: cost basis climbs above market value.
        rows = [
            {"period": "2026-01", "market_value": "50000", "cumulative_cost_basis": "40000", "quantity": "1.0"},
            {"period": "2026-02", "market_value": "44000", "cumulative_cost_basis": "60000", "quantity": "1.1"},
            {"period": "2026-03", "market_value": "33500", "cumulative_cost_basis": "84000", "quantity": "1.2"},
        ]
        drawing = _line_chart(self.rl, "Total balance over time", rows, "EUR")
        ys = self._poly_ys(drawing)
        self.assertTrue(ys, "expected plotted polylines")
        # Every plotted point must stay inside the drawing canvas; before the
        # fix the cost-basis line ran far above it and across the page.
        self.assertLessEqual(max(ys), drawing.height + 0.5)
        self.assertGreaterEqual(min(ys), -0.5)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
