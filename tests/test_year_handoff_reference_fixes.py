"""Year handoff regressions found by the Rust workbook port."""

from kassiber.core.year_handoff import _period_for_profile


def test_handoff_uses_normalized_tax_country_for_its_calendar():
    assert _period_for_profile({"tax_country": " at "}, 2024).calendar == "Europe/Vienna"
