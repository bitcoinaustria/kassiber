"""Valid calendar endpoints stay usable by probable matching."""

from datetime import datetime, timezone

import pytest

from kassiber.core.cross_check import _BookRow, _Index, _ToolRow


@pytest.mark.parametrize("moment", [datetime.min.replace(tzinfo=timezone.utc), datetime.max.replace(tzinfo=timezone.utc)])
def test_probable_match_window_is_clamped_at_calendar_limits(moment):
    from datetime import timedelta

    row = _BookRow("row", "Wallet", "wallet", "", moment, "inbound", "bitcoin:btc", 1000, 0, "unbooked", None, False)
    tool = _ToolRow(1, "Tool", "Deposit", "", moment, "inbound", "bitcoin:btc", 1000)
    assert _Index([row]).probable(tool, timedelta()) is row
