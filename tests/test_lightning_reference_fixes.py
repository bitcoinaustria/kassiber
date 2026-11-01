"""Lightning regressions discovered by the Rust reference comparisons."""

from dataclasses import replace
from decimal import Decimal, localcontext

import pytest

from kassiber.core.lightning import build_profitability_report, profitability_csv_rows
from tests.test_lightning_scaffold import _snapshot_with_routing


@pytest.mark.parametrize("amount", [900719925474099301, -900719925474099301, 10**40 + 1])
def test_profitability_csv_preserves_exact_sat_in_every_btc_column(amount):
    snapshot = _snapshot_with_routing()
    snapshot = replace(
        snapshot,
        routing=replace(snapshot.routing, routing_revenue_sat=amount),
        channels=(replace(snapshot.channels[0], earned_routing_sat=amount),),
    )
    report = build_profitability_report(
        connection_id="node", connection_label="Node", connection_kind="lnd", snapshot=snapshot,
    )
    with localcontext() as ctx:
        ctx.prec = 100
        for row in profitability_csv_rows(report):
            if row[3] and row[0] != "section":
                assert Decimal(row[3]) * 100_000_000 == Decimal(row[2])
