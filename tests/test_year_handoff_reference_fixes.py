"""Year handoff regressions found by the Rust workbook port."""

from kassiber.core.year_handoff import _period_for_profile

import pytest

from kassiber.core.year_handoff import _year_end_rates
from kassiber.db import open_db


def test_handoff_uses_normalized_tax_country_for_its_calendar():
    assert _period_for_profile({"tax_country": " at "}, 2024).calendar == "Europe/Vienna"


@pytest.mark.parametrize("country,boundary", [("generic", "2025-01-01T00:00:00Z"), ("at", "2024-12-31T23:00:00Z")])
def test_handoff_includes_the_final_fractional_second_quote(tmp_path, country, boundary):
    from datetime import datetime, timedelta

    conn = open_db(tmp_path / "data")
    try:
        end = datetime.fromisoformat(boundary.replace("Z", "+00:00"))
        for offset, rate in ((timedelta(seconds=-1), 100), (timedelta(microseconds=-1), 200), (timedelta(), 300)):
            conn.execute("INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at) VALUES('BTC-EUR',?,?,?,'manual','fixed')",
                         ((end + offset).isoformat().replace("+00:00", "Z"), rate, str(rate)))
        profile = {"id": "p", "tax_country": country, "fiat_currency": "EUR"}
        result = _year_end_rates(conn, profile, _period_for_profile(profile, 2024), {"BTC": {"BTC"}})
        assert result["BTC"]["rate"] == 200
    finally:
        conn.close()
