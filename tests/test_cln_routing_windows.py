"""CLN report acquisition and time-window regressions from the Rust port."""

from dataclasses import replace
from unittest.mock import Mock

import pytest

from kassiber.core.lightning import build_profitability_report
from kassiber.core.lightning import cln
from kassiber.errors import AppError


def _snapshot(payloads):
    return cln.fetch_core_lightning_snapshot({}, rpc_call=lambda method, args: payloads.get(method, {}))


def test_cln_routing_window_filters_revenue_costs_and_channel_earnings(monkeypatch):
    monkeypatch.setattr(cln, "now_iso", lambda: "2025-01-10T00:00:00Z")
    recent, old = 1736424000, 1735689600  # Jan 9 noon and Jan 1 UTC.
    raw = _snapshot({
        "listpeerchannels": {"channels": [{"short_channel_id": "1x1x0", "state": "CHANNELD_NORMAL"}]},
        "listforwards": {"forwards": [{"status": "settled", "fee_msat": fee, "resolved_time": stamp, "out_channel": "1x1x0"} for stamp, fee in ((recent, 2000), (old, 3000))]},
        "listpays": {"pays": [{"status": "complete", "amount_msat": 1000, "amount_sent_msat": 2000, "completed_at": stamp} for stamp in (recent, old)]},
        "bkpr-listincome": {"income_events": [{"tag": "onchain_fee", "debit_msat": 1000, "timestamp": stamp} for stamp in (recent, old)]},
    })
    day = cln.build_node_snapshot(raw, window_days=1)
    year = cln.build_node_snapshot(raw, window_days=365)
    assert day.routing.routing_revenue_sat == 2
    assert day.routing.payment_cost_sat == day.routing.onchain_cost_sat == 1
    assert day.routing.forward_count == day.routing.payment_count == 1
    assert day.channels[0].earned_routing_sat == 2
    assert year.routing.routing_revenue_sat == 5
    assert year.channels[0].earned_routing_sat == 5
