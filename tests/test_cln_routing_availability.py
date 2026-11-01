"""CLN report acquisition and time-window regressions from the Rust port."""

from dataclasses import replace
from unittest.mock import Mock

import pytest

from kassiber.core.lightning import build_profitability_report
from kassiber.core.lightning import cln
from kassiber.errors import AppError


def _snapshot(payloads):
    return cln.fetch_core_lightning_snapshot({}, rpc_call=lambda method, args: payloads.get(method, {}))


@pytest.mark.parametrize("method", ["listforwards", "listpays", "bkpr-listincome"])
def test_cln_partial_routing_resources_refuse_profitability(method):
    raw = replace(_snapshot({"listforwards": {"forwards": [{"status": "settled", "fee_msat": 2000, "resolved_time": 1736424000}]}}), errors={method: "unavailable"})
    snapshot = cln.build_node_snapshot(raw, window_days=30)
    assert snapshot.routing is None
    with pytest.raises(AppError) as error:
        build_profitability_report(connection_id="node", connection_label="Node", connection_kind="coreln", snapshot=snapshot)
    assert error.value.code == "lightning_routing_unavailable"


def test_complete_empty_cln_window_is_a_known_zero_report():
    snapshot = cln.build_node_snapshot(_snapshot({}), window_days=30)
    assert snapshot.routing is not None
    assert snapshot.routing.forward_count == snapshot.routing.net_profit_sat == 0
