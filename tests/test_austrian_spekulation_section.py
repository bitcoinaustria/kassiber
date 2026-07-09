"""alt_spekulation must land in taxable Spekulation, not non-taxable Alt."""

from __future__ import annotations

import unittest

from kassiber.core.reports import _austrian_tax_section_id


class AustrianSpekulationSectionTests(unittest.TestCase):
    def test_alt_spekulation_is_taxable_section_1_3(self):
        self.assertEqual(
            _austrian_tax_section_id({"at_category": "alt_spekulation"}),
            "1.3",
        )

    def test_alt_taxfree_stays_in_non_taxable_3_1(self):
        self.assertEqual(
            _austrian_tax_section_id({"at_category": "alt_taxfree"}),
            "3.1",
        )

    def test_neu_disposals_stay_in_1_1(self):
        self.assertEqual(
            _austrian_tax_section_id({"at_category": "neu_gain"}),
            "1.1",
        )


if __name__ == "__main__":
    unittest.main()
