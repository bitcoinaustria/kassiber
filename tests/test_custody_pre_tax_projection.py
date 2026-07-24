from __future__ import annotations

from contextlib import contextmanager
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber.core.custody_gaps import EMPTY_GAP_SEARCH_RESULT
from kassiber.core.custody_quantity_runtime import (
    build_canonical_quantity_state as _build_canonical_quantity_state,
)
from kassiber.core.custody_tax_projection import compile_finalized_tax_projection
from kassiber.core.custody_evidence import build_canonical_quantity_input, enriched_quantity_rows
from kassiber.core.custody_quantity import (
    CUSTODY_SUSPENSE,
    ClaimPriority,
    QuantityClaim,
    QuantitySlice,
)
from kassiber.core.custody_interpreters import compile_custody_interpreters
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.rp2 import GenericRP2TaxEngine, _GenericRailCarryResult
from tests.custody_tax_helpers import (
    authoritative_chain_observation,
    finalized_tax_inputs,
)


def build_canonical_quantity_state(rows, **kwargs):
    kwargs.setdefault("gap_search_result", EMPTY_GAP_SEARCH_RESULT)
    return _build_canonical_quantity_state(rows, **kwargs)


def _row(
    row_id: str,
    wallet_id: str,
    direction: str,
    amount: int,
    occurred_at: str,
) -> dict[str, object]:
    return {
        "id": row_id,
        "wallet_id": wallet_id,
        "wallet_label": wallet_id,
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": 0,
        "amount_includes_fee": False,
        "occurred_at": occurred_at,
        "created_at": occurred_at,
        "external_id": row_id,
        "external_id_kind": "provider",
        "kind": "buy" if direction == "inbound" else "sell",
        "raw_json": {},
        "fiat_rate": 1.0,
    }


def _residual_state():
    rows = [
        _row("acquisition", "source", "inbound", 10_000, "2024-01-01T00:00:00Z"),
        _row("source-move", "source", "outbound", 10_000, "2025-01-01T00:00:00Z"),
        _row("retained", "destination", "inbound", 9_900, "2025-01-01T00:00:00Z"),
        _row("later-sale", "destination", "outbound", 9_900, "2025-01-02T00:00:00Z"),
    ]
    state = build_canonical_quantity_state(
        rows,
        effective_components=(
            {
                "id": "reviewed-component",
                "effective_state": "active",
                "legs": (
                    {"id": "source", "role": "source", "transaction_id": "source-move"},
                    {"id": "retained", "role": "retained", "transaction_id": "retained"},
                    {"id": "suspense", "role": "suspense", "amount_msat": 100},
                ),
                "allocations": (
                    {"id": "retained", "source_leg_id": "source", "sink_leg_id": "retained", "source_amount_msat": 9_900, "sink_amount_msat": 9_900},
                    {"id": "suspense", "source_leg_id": "source", "sink_leg_id": "suspense", "source_amount_msat": 100, "sink_amount_msat": 100},
                ),
            },
        ),
    )
    return rows, state


def test_residual_suspense_keeps_finalized_sibling_but_blocks_later_sale():
    rows, state = _residual_state()
    profile = {"id": "profile", "workspace_id": "workspace", "label": "Book"}

    projection = compile_finalized_tax_projection(profile, rows, state)

    assert {
        (row["journal_transaction_id"], row["amount"])
        for row in projection.rows
    } == {
        ("acquisition", 10_000),
        ("source-move", 9_900),
        ("retained", 9_900),
    }
    assert len(projection.intra_pairs) == 1
    assert all(row["journal_transaction_id"] != "later-sale" for row in projection.rows)
    assert any(
        item["transaction_id"] == "later-sale"
        and item["reason"] == "custody_basis_barrier"
        for item in projection.quarantines
    )


def test_suspense_principal_still_projects_separately_known_network_fee():
    rows = [
        _row("acquisition", "source", "inbound", 10_100, "2024-01-01T00:00:00Z"),
        _row("gap", "source", "outbound", 10_000, "2025-01-01T00:00:00Z"),
        _row("later-sale", "source", "outbound", 1_000, "2026-01-01T00:00:00Z"),
    ]
    rows[1]["fee"] = 100
    baseline = build_canonical_quantity_state(rows)
    gap = next(
        item
        for item in baseline.projection.observations
        if item.transaction_id == "gap"
    )
    state = build_canonical_quantity_state(
        rows,
        interpreter_claims=(
            QuantityClaim(
                claim_id="gap-suspense",
                source=QuantitySlice(gap.quantity_hash, 0, gap.principal_msat),
                state=CUSTODY_SUSPENSE,
                priority=ClaimPriority.ACCOUNTING_CONVENTION,
                reason="missing_wallet",
            ),
        ),
    )

    projection = compile_finalized_tax_projection(
        {"id": "profile", "workspace_id": "workspace", "label": "Book"},
        rows,
        state,
    )

    gap_rows = [
        row for row in projection.rows if row["journal_transaction_id"] == "gap"
    ]
    assert [(row["amount"], row["fee"]) for row in gap_rows] == [(0, 100)]
    assert all(
        row["journal_transaction_id"] != "later-sale" for row in projection.rows
    )
    assert any(
        item["transaction_id"] == "later-sale"
        and item["reason"] == "custody_basis_barrier"
        for item in projection.quarantines
    )


def test_reviewed_component_fee_replaces_raw_fee_in_move_projection():
    rows = [
        _row("acquisition", "source", "inbound", 1_010, "2024-01-01T00:00:00Z"),
        _row("out", "source", "outbound", 1_000, "2025-01-01T00:00:00Z"),
        _row("in", "destination", "inbound", 900, "2025-01-01T00:01:00Z"),
    ]
    rows[1]["fee"] = 10
    wallet_refs = {
        wallet_id: {
            "id": wallet_id,
            "label": wallet_id,
            "kind": "descriptor",
            "config_json": "{}",
            "wallet_account_id": "account",
            "account_code": "treasury",
            "account_label": "Treasury",
        }
        for wallet_id in ("source", "destination")
    }

    inputs = finalized_tax_inputs(
        {"id": "profile", "workspace_id": "workspace", "label": "Book"},
        rows=rows,
        wallet_refs_by_id=wallet_refs,
        manual_pair_records=(
            {
                "id": "reviewed-pair",
                "out_transaction_id": "out",
                "in_transaction_id": "in",
                "kind": "manual",
                "policy": "carrying-value",
                "out_amount": 900,
            },
        ),
    )

    move_out = next(
        row
        for row in inputs.finalized_tax_projection.rows
        if row["journal_transaction_id"] == "out" and row["amount"] == 900
    )
    assert move_out["fee"] == 10


def test_reviewed_conversion_fee_remains_additive_to_raw_miner_fee():
    rows = [
        _row("acquisition", "source", "inbound", 1_010, "2024-01-01T00:00:00Z"),
        _row("out", "source", "outbound", 1_000, "2025-01-01T00:00:00Z"),
        _row("in", "destination", "inbound", 900, "2025-01-01T00:01:00Z"),
    ]
    rows[1]["fee"] = 10
    state = build_canonical_quantity_state(
        rows,
        effective_components=(
            {
                "id": "reviewed-conversion",
                "component_type": "swap",
                "conservation_mode": "conversion",
                "conversion_policy": "carrying-value",
                "conversion_reviewed": True,
                "effective_state": "active",
                "legs": (
                    {
                        "id": "source",
                        "role": "source",
                        "transaction_id": "out",
                    },
                    {
                        "id": "retained",
                        "role": "destination",
                        "transaction_id": "in",
                    },
                    {"id": "fee", "role": "fee", "amount_msat": 100},
                ),
                "allocations": (
                    {
                        "id": "retained",
                        "source_leg_id": "source",
                        "sink_leg_id": "retained",
                        "source_amount_msat": 900,
                        "sink_amount_msat": 900,
                    },
                    {
                        "id": "fee",
                        "source_leg_id": "source",
                        "sink_leg_id": "fee",
                        "source_amount_msat": 100,
                        "sink_amount_msat": 100,
                    },
                ),
            },
        ),
    )

    projection = compile_finalized_tax_projection(
        {"id": "profile", "workspace_id": "workspace", "label": "Book"},
        rows,
        state,
    )

    move_out = next(
        row
        for row in projection.rows
        if row["journal_transaction_id"] == "out" and row["amount"] == 900
    )
    assert move_out["fee"] == 110


def test_swap_refund_residual_remains_additive_to_raw_miner_fee():
    rows = [
        _row("acquisition", "wallet", "inbound", 1_010, "2024-01-01T00:00:00Z"),
        _row("out", "wallet", "outbound", 1_000, "2025-01-01T00:00:00Z"),
        _row("refund", "wallet", "inbound", 998, "2025-01-01T00:01:00Z"),
    ]
    rows[1]["fee"] = 1
    wallet_refs = {
        "wallet": {
            "id": "wallet",
            "label": "wallet",
            "kind": "custom",
            "config_json": "{}",
            "wallet_account_id": "account",
            "account_code": "treasury",
            "account_label": "Treasury",
        }
    }

    inputs = finalized_tax_inputs(
        {"id": "profile", "workspace_id": "workspace", "label": "Book"},
        rows=rows,
        wallet_refs_by_id=wallet_refs,
        manual_pair_records=(
            {
                "id": "reviewed-refund",
                "out_transaction_id": "out",
                "in_transaction_id": "refund",
                "kind": "swap-refund",
                "policy": "carrying-value",
            },
        ),
    )

    move_out = next(
        row
        for row in inputs.finalized_tax_projection.rows
        if row["journal_transaction_id"] == "out" and row["amount"] == 998
    )
    assert move_out["fee"] == 3


def test_basis_barrier_fails_closed_for_distinct_same_timestamp_event():
    barrier_at = "2025-01-01T00:00:00Z"
    rows = [
        _row("acquisition", "source", "inbound", 100, "2024-01-01T00:00:00Z"),
        # This id sorts before the barrier id, but that deterministic tie-break
        # cannot prove that the disposal happened first in real time.
        _row("a-same-time-sale", "source", "outbound", 10, barrier_at),
        _row("z-gap", "source", "outbound", 20, barrier_at),
    ]
    baseline = build_canonical_quantity_state(rows)
    observations = {
        item.transaction_id: item for item in baseline.projection.observations
    }
    gap = observations["z-gap"]
    state = build_canonical_quantity_state(
        rows,
        interpreter_claims=[
            QuantityClaim(
                claim_id="same-time-gap-suspense",
                source=QuantitySlice(gap.quantity_hash, 0, gap.principal_msat),
                state=CUSTODY_SUSPENSE,
                priority=ClaimPriority.ACCOUNTING_CONVENTION,
                reason="missing_wallet",
            )
        ],
    )

    same_time_hash = observations["a-same-time-sale"].quantity_hash
    assert any(
        item.observation_hash == same_time_hash
        for item in state.tax_eligibility.ineligible_slices
    )

    projection = compile_finalized_tax_projection(
        {"id": "profile", "workspace_id": "workspace", "label": "Book"},
        rows,
        state,
    )
    assert all(
        row["journal_transaction_id"] != "a-same-time-sale"
        for row in projection.rows
    )
    assert any(
        item["transaction_id"] == "a-same-time-sale"
        and item["reason"] == "custody_basis_barrier"
        for item in projection.quarantines
    )


def test_basis_barrier_does_not_suppress_unrelated_asset_projection():
    rows = [
        _row("btc-acquisition", "source", "inbound", 100, "2024-01-01T00:00:00Z"),
        _row("btc-gap", "source", "outbound", 20, "2025-01-01T00:00:00Z"),
        _row("btc-later", "source", "outbound", 30, "2026-01-01T00:00:00Z"),
        _row("usdt-acquisition", "stable", "inbound", 50, "2025-06-01T00:00:00Z"),
        _row("usdt-later", "stable", "outbound", 40, "2026-01-01T00:00:00Z"),
    ]
    for row in rows[-2:]:
        row["asset"] = "USDT"
    baseline = build_canonical_quantity_state(rows)
    gap = next(
        item for item in baseline.projection.observations
        if item.transaction_id == "btc-gap"
    )
    state = build_canonical_quantity_state(
        rows,
        interpreter_claims=[
            QuantityClaim(
                claim_id="btc-gap-suspense",
                source=QuantitySlice(gap.quantity_hash, 0, 20),
                state=CUSTODY_SUSPENSE,
                priority=ClaimPriority.ACCOUNTING_CONVENTION,
                reason="missing_wallet",
            )
        ],
    )

    projection = compile_finalized_tax_projection(
        {"id": "profile", "workspace_id": "workspace", "label": "Book"},
        rows,
        state,
    )
    projected_ids = {row["journal_transaction_id"] for row in projection.rows}

    assert {"btc-acquisition", "usdt-acquisition", "usdt-later"} <= projected_ids
    assert "btc-gap" not in projected_ids
    assert "btc-later" not in projected_ids
    assert len(state.tax_eligibility.pool_barriers) == 1
    assert any(
        item["transaction_id"] == "btc-later"
        and item["reason"] == "custody_basis_barrier"
        for item in projection.quarantines
    )
    assert all(item["transaction_id"] != "usdt-later" for item in projection.quarantines)


def test_rp2_boundary_spy_never_receives_residual_or_later_basis_consumer():
    rows, state = _residual_state()
    profile = {
        "id": "profile",
        "workspace_id": "workspace",
        "label": "Book",
        "tax_country": "generic",
        "gains_algorithm": "FIFO",
        "fiat_currency": "EUR",
    }
    projection = compile_finalized_tax_projection(profile, rows, state)
    wallet_refs = {
        "source": {"id": "source", "label": "source", "account_code": "treasury", "account_label": "Treasury", "wallet_account_id": "account"},
        "destination": {"id": "destination", "label": "destination", "account_code": "treasury", "account_label": "Treasury", "wallet_account_id": "account"},
    }
    captured: list[dict[str, object]] = []

    def spy_prepare(_profile, rows_by_asset, *_args, **_kwargs):
        captured.extend(row for asset_rows in rows_by_asset.values() for row in asset_rows)
        return []

    @contextmanager
    def configuration(*_args, **_kwargs):
        yield SimpleNamespace(country=SimpleNamespace(validate_input_data=lambda _items: None))

    with (
        patch("kassiber.core.engines.rp2._rp2_configuration", side_effect=configuration),
        patch("kassiber.core.engines.rp2._apply_generic_bitcoin_rail_carry_values", side_effect=lambda _p, rows_for_engine, *_a, **_k: _GenericRailCarryResult(list(rows_for_engine), set(), [])),
        patch("kassiber.core.engines.rp2._prepare_assets", side_effect=spy_prepare),
        patch("kassiber.core.engines.rp2._validate_prepared_rp2_inputs"),
        patch("kassiber.core.engines.rp2._rp2_asset_states_from_prepared", return_value={}),
    ):
        GenericRP2TaxEngine(profile).build_ledger_state(
            TaxEngineLedgerInputs(
                finalized_tax_projection=projection,
                wallet_refs_by_id=wallet_refs,
            )
        )

    anchored = {row["journal_transaction_id"] for row in captured}
    assert anchored == {"acquisition", "source-move", "retained"}
    assert "later-sale" not in anchored
    assert all(int(row["amount"]) != 100 for row in captured)


def test_tax_engine_contract_rejects_raw_rows_at_construction():
    with pytest.raises(TypeError):
        TaxEngineLedgerInputs(rows=(), wallet_refs_by_id={})  # type: ignore[call-arg]


def test_same_timestamp_native_siblings_compile_before_rp2_without_audit_input():
    txid = "ab" * 32
    rows = [
        _row("acquisition", "source", "inbound", 1_000, "2024-01-01T00:00:00Z"),
        authoritative_chain_observation({
            **_row("out", "source", "outbound", 1_000, "2025-01-01T00:00:00Z"),
            "external_id": txid,
            "external_id_kind": "txid",
            "raw_json": {"txid": txid, "network": "main", "chain": "bitcoin"},
        }),
        authoritative_chain_observation({
            **_row("in", "destination", "inbound", 1_000, "2025-01-01T00:00:00Z"),
            "external_id": txid,
            "external_id_kind": "txid",
            "raw_json": {"txid": txid, "network": "main", "chain": "bitcoin"},
        }),
    ]
    refs = {
        wallet: {"id": wallet, "label": wallet, "wallet_account_id": "account", "account_code": "treasury", "account_label": "Treasury"}
        for wallet in ("source", "destination")
    }
    canonical = build_canonical_quantity_input(enriched_quantity_rows(rows))
    compiled = compile_custody_interpreters(rows, canonical, wallet_refs_by_id=refs)
    state = build_canonical_quantity_state(rows, interpreter_claims=compiled.claims)
    profile = {"id": "profile", "workspace_id": "workspace", "label": "Book"}
    projection = compile_finalized_tax_projection(profile, rows, state)

    assert compiled.native_audits == ()
    assert len(compiled.claims) == 1
    assert len(projection.intra_pairs) == 1
    assert {row["journal_transaction_id"] for row in projection.rows} == {
        "acquisition", "out", "in"
    }


def test_unreviewed_privacy_hop_is_a_specific_pre_tax_blocker():
    row = {
        **_row("coinjoin", "source", "outbound", 1_000, "2025-01-01T00:00:00Z"),
        "privacy_boundary": "coinjoin",
        "raw_json": {"source": "wasabi", "islikelycoinjoin": True},
    }
    refs = {
        "source": {
            "id": "source",
            "label": "Source",
            "wallet_account_id": "account",
            "account_code": "treasury",
            "account_label": "Treasury",
        }
    }
    canonical = build_canonical_quantity_input(enriched_quantity_rows([row]))

    compiled = compile_custody_interpreters(
        [row], canonical, wallet_refs_by_id=refs
    )

    assert compiled.blocked_transaction_ids == ("coinjoin",)
    assert len(compiled.quarantines) == 1
    assert compiled.quarantines[0]["reason"] == "privacy_hop_unresolved"
