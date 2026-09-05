"""Real chain dependencies must survive same-block wallet round trips."""

import json
from dataclasses import replace
from types import SimpleNamespace

import pytest

from kassiber.core.engines import GenericRP2TaxEngine
from kassiber.core.transfer_chronology import chain_order_evidence, order_same_time_transfers
from tests import test_rp2_ownership_transfers as ownership_fixtures
from tests.custody_tax_helpers import authoritative_chain_observation, finalized_tax_inputs
from tests.test_rp2_ownership_transfers import (
    BTC, PROFILE, SCRIPT_A, SCRIPT_B, SCRIPT_C, WALLET_REFS,
    _fanout_index, _physical_txid, _row,
)


def _round_trip_rows(seed=1):
    rows = [_row("A", "inbound", BTC, external_id="acquisition")]
    rows[0]["occurred_at"] = "2025-12-31T00:00:00Z"
    previous = "acquisition"
    for label, source, destination, source_script, destination_script in (
        ("first", "A", "B", SCRIPT_A, SCRIPT_B),
        ("second", "B", "A", SCRIPT_B, SCRIPT_A),
        ("third", "A", "C", SCRIPT_A, SCRIPT_C),
    ):
        label = f"{label}-{seed}"
        graph = json.dumps({
            "txid": _physical_txid(label),
            "vin": [{"txid": _physical_txid(previous), "vout": 0,
                     "prevout": {"scriptpubkey": source_script, "value": BTC // 1000}}],
            "vout": [{"n": 0, "scriptpubkey": destination_script, "value": BTC // 1000}],
            "fee": 0,
        })
        rows.extend((
            _row(source, "outbound", BTC, external_id=label, raw_json=graph),
            _row(destination, "inbound", BTC, external_id=label, raw_json=graph),
        ))
        previous = label
    return rows


@pytest.mark.parametrize("seed", range(5))
@pytest.mark.parametrize("profile", [PROFILE, ownership_fixtures.AustrianSelfTransferEngineTest.AT_PROFILE], ids=["generic", "austrian"])
def test_same_block_round_trip_uses_chain_dependencies_in_every_import_order(seed, profile):
    rows = _round_trip_rows(seed)
    for ordered in (rows, list(reversed(rows)), rows[3:] + rows[:3]):
        state = GenericRP2TaxEngine(profile).build_ledger_state(
            finalized_tax_inputs(profile, rows=ordered,
                                 wallet_refs_by_id=WALLET_REFS,
                                 manual_pair_records=[], owned_index=_fanout_index())
        )
        assert not state.quarantines, state.quarantines
        assert sum(entry["entry_type"] == "transfer_out" for entry in state.entries) == 3
        holdings = {key[1]: totals["quantity"] for key, totals in state.wallet_holdings.items()}
        assert holdings.get("Cold", 0) == 0
        assert holdings.get("Hot", 0) == 0
        assert holdings["Savings"] == 1


def test_chronology_rejects_unattested_and_changed_graphs():
    row = _round_trip_rows()[3]
    assert chain_order_evidence(row).parents
    imported = {key: value for key, value in row.items() if not key.startswith("observation_")}
    assert chain_order_evidence(imported) is None
    assert chain_order_evidence({**row, "amount": 1}) is None
    graph = json.loads(row["raw_json"])
    graph["vin"][0]["txid"] = "ff" * 32
    assert chain_order_evidence({**row, "raw_json": json.dumps(graph)}) is None


@pytest.mark.parametrize("wrapper", ["tx", "ownership_graph"])
def test_chronology_accepts_authoritative_nested_core_graph(wrapper):
    row = _round_trip_rows()[3]
    expected = chain_order_evidence(row)
    nested = authoritative_chain_observation({**row, "raw_json": json.dumps({wrapper: json.loads(row["raw_json"])})})
    assert chain_order_evidence(nested) == expected


@pytest.mark.parametrize("network", ["main", "test"])
def test_chain_ordering_keeps_identical_txids_on_other_networks_separate(network):
    rows = _round_trip_rows()
    source, target = chain_order_evidence(rows[1]), chain_order_evidence(rows[3])
    source = replace(source, scope=(source.scope[0], network, source.scope[2], source.scope[3]))
    transfers = {
        "target": SimpleNamespace(from_wallet_id="X", to_wallet_id="Y", out_row={"custody_chain_order": target}),
        "source": SimpleNamespace(from_wallet_id="A", to_wallet_id="B", out_row={"custody_chain_order": source}),
    }
    items = [(0, ("transfer", "target")), (1, ("transfer", "source"))]
    expected = [item for _, item in items]
    if network == "main":
        expected.reverse()
    assert order_same_time_transfers(items, transfers) == expected


def test_same_event_allocations_do_not_invent_wallet_chronology():
    proof = chain_order_evidence(_round_trip_rows()[1])
    transfers = {
        "destination": SimpleNamespace(from_wallet_id="B", to_wallet_id="C", out_row={"custody_chain_order": proof}),
        "source": SimpleNamespace(from_wallet_id="A", to_wallet_id="B", out_row={"custody_chain_order": proof}),
    }
    items = [(0, ("transfer", "destination")), (1, ("transfer", "source"))]
    assert order_same_time_transfers(items, transfers) == [item for _, item in items]


def test_graphless_wallet_cycle_keeps_original_order():
    transfers = {
        "return": SimpleNamespace(from_wallet_id="B", to_wallet_id="A", out_row={}),
        "fund": SimpleNamespace(from_wallet_id="A", to_wallet_id="B", out_row={}),
    }
    items = [(0, ("transfer", "return")), (1, ("transfer", "fund"))]
    assert order_same_time_transfers(items, transfers) == [item for _, item in items]
