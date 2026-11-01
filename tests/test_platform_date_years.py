"""Platform CSV regressions recorded by the Rust artifact port."""

from datetime import datetime, timezone
from types import SimpleNamespace

from kassiber.cli.handlers import _report_hooks
from kassiber.core import transaction_formats as formats
from kassiber.core.asset_presentation import describe_asset
from kassiber.db import open_db
from tests.test_custody_authored_migration import NOW, _scope, _tx


def test_platform_dates_always_have_a_four_digit_year():
    row = formats.PlatformRow(datetime(1, 1, 1, 1, 2, 3, tzinfo=timezone.utc), "Wallet", "tx", "deposit", "unbooked", describe_asset("BTC"), "BTC", "1", "", "EUR", "", "", "")
    assert formats.COINTRACKING.build_row(row)["Date"] == "0001-01-01 01:02:03"
    assert formats.BLOCKPIT.build_row(row)["Date (UTC)"] == "01.01.0001 01:02:03"
