from __future__ import annotations

import json
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
    INTERNAL_VERIFIED,
    ClaimPriority,
    QuantityClaim,
    QuantitySlice,
)
from kassiber.core.custody_interpreters import compile_custody_interpreters
from kassiber.transfers import (
    _SYNTHETIC_TRANSFER_ID_PREFIXES,
    onchain_transfer_scope,
)
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


def test_every_projected_row_id_is_synthetic_and_pairs_are_edge_disjoint():
    """Pin the two invariants a large amount of tax_events.py silently rests on.

    `normalize_tax_asset_inputs` only ever sees a `FinalizedTaxProjection`, and
    several of its branches are reachable only for rows that resolve an
    `onchain_transfer_scope` or that share a pair leg. Neither is possible today:

    1. every projected row id is re-stamped with a member of
       `_SYNTHETIC_TRANSFER_ID_PREFIXES`, and `onchain_transfer_scope` returns
       `None` for those prefixes — so the txid-scope-gated paths (the
       Samourai/Whirlpool splitter, `_owned_fanout_row_ids`) are inert;
    2. each decision mints its own fresh move rows, so the pair graph is a
       perfect matching and no multi-pair component can form — which is what
       makes `_build_manual_multi_pair_transfers`' multi-pair branch inert.

    Nothing in `custody_tax_projection` asserts either property, so without this
    test the inertness is an unasserted cross-module coupling. Anyone deleting
    that dead code needs this guard; anyone breaking these invariants needs to
    know the dead code became live again.
    """

    rows, state = _residual_state()
    profile = {"id": "profile", "workspace_id": "workspace", "label": "Book"}

    projection = compile_finalized_tax_projection(profile, rows, state)

    assert projection.rows, "fixture must project at least one row"
    for row in projection.rows:
        row_id = str(row["id"])
        assert row_id.startswith(_SYNTHETIC_TRANSFER_ID_PREFIXES), row_id
        assert onchain_transfer_scope(row) is None, row_id

    leg_ids = [
        str((pair[side] or {})["id"])
        for pair in projection.intra_pairs
        for side in ("out", "in")
    ]
    assert len(leg_ids) == len(set(leg_ids)), (
        f"pair legs must be edge-disjoint, got {leg_ids}"
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


def _whirlpool_tx0_compilation(*, fee_attribution: str):
    """A Tx0 fan-out whose coordinator fee is the unallocated residual."""

    txid = "ef" * 32

    def leg(row_id, wallet, section, direction, amount):
        row = {
            **_row(row_id, wallet, direction, amount, "2025-01-01T00:00:00Z"),
            "external_id": txid,
            "external_id_kind": "txid",
            "raw_json": {"txid": txid, "network": "main", "chain": "bitcoin"},
            "config_json": json.dumps(
                {"samourai": {"role": "child", "section": section, "group_id": "wp"}}
            ),
        }
        return authoritative_chain_observation(row, fee_attribution=fee_attribution)

    rows = [
        leg("dep-out", "deposit", "deposit", "outbound", 10_000_000),
        leg("pre-in", "premix", "premix", "inbound", 9_500_000),
        leg("bad-in", "badbank", "badbank", "inbound", 450_000),
    ]
    refs = {
        wallet: {
            "id": wallet,
            "label": wallet,
            "wallet_account_id": "account",
            "account_code": "treasury",
            "account_label": "Treasury",
        }
        for wallet in ("deposit", "premix", "badbank")
    }
    canonical = build_canonical_quantity_input(enriched_quantity_rows(rows))
    compiled = compile_custody_interpreters(rows, canonical, wallet_refs_by_id=refs)
    state = build_canonical_quantity_state(
        rows, interpreter_claims=compiled.claims
    )
    return compiled, state


def test_inexact_whirlpool_coordinator_fee_keeps_its_group_moves():
    """A non-exact coordinator fee must not destroy the group's MOVEs.

    Regression: the residual claim was bundled under the same `pair-group:` id as
    the group's MOVE claims, but its priority is conditional while `_pair_claims`
    always emits MOVEs at EXACT_NATIVE_EVENT. The arbiter discards any bundle
    whose members disagree on priority, so an ordinary compatibility-observer
    sync (fee_attribution='unknown') replaced two correct internal MOVEs with a
    single suspense claim — and left blocked ids and quarantines empty, so
    nothing told the user why every report was blocked.
    """

    compiled, state = _whirlpool_tx0_compilation(fee_attribution="unknown")

    # Behaviour first, because that is what the fix is for: both the premix and
    # badbank receipts must still be covered by an internal MOVE decision...
    decisions = state.projection.decisions
    internal = [d for d in decisions if str(d.state) == INTERNAL_VERIFIED]
    assert len(internal) == 2, [str(d.state) for d in decisions]
    # ...and the coordinator fee stays explicit suspense rather than silently
    # becoming a disposal, so the report barrier survives without a basis edge.
    assert [str(d.state) for d in decisions].count(CUSTODY_SUSPENSE) == 1

    # Then the mechanism, so a future regression is diagnosable and not just red:
    # the collision was only possible because both priorities shared one bundle.
    assert len({claim.priority for claim in compiled.claims}) > 1
    assert not [
        claim
        for claim in compiled.claims
        if claim.priority != ClaimPriority.EXACT_NATIVE_EVENT
        and claim.atomic_bundle_id is not None
    ]
    assert not [
        issue for issue in state.issues if "bundle" in str(issue.reason)
    ], [str(issue.reason) for issue in state.issues]


def test_exact_whirlpool_coordinator_fee_stays_atomic_with_its_group():
    """The negative control: an exact fee must remain one atomic unit.

    Unbundling it unconditionally would let the fee arbitrate independently of
    the MOVEs it belongs to, so the fix must keep the exact case bundled.
    """

    compiled, _state = _whirlpool_tx0_compilation(fee_attribution="exact")

    bundles = {claim.atomic_bundle_id for claim in compiled.claims}
    assert bundles == {f"pair-group:samourai:wp:{'ef' * 32}"}
    assert len({claim.priority for claim in compiled.claims}) == 1


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


def _privacy_fanout_rows(*, authoritative_destinations: bool):
    """One authoritative 1:N chain event where every leg is privacy-tagged."""

    txid = "cd" * 32
    chain_raw = {"txid": txid, "network": "main", "chain": "bitcoin"}
    privacy = {"privacy_boundary": "coinjoin"}

    def leg(row_id, wallet, direction, amount):
        return {
            **_row(row_id, wallet, direction, amount, "2025-01-01T00:00:00Z"),
            **privacy,
            "external_id": txid,
            "external_id_kind": "txid",
            "raw_json": dict(chain_raw),
        }

    source = authoritative_chain_observation(leg("fanout-out", "source", "outbound", 10_000))
    destinations = [
        leg("fanout-in-1", "dest-a", "inbound", 6_000),
        leg("fanout-in-2", "dest-b", "inbound", 4_000),
    ]
    if authoritative_destinations:
        destinations = [authoritative_chain_observation(row) for row in destinations]
    rows = [source, *destinations]
    refs = {
        wallet: {
            "id": wallet,
            "label": wallet,
            "wallet_account_id": "account",
            "account_code": "treasury",
            "account_label": "Treasury",
        }
        for wallet in ("source", "dest-a", "dest-b")
    }
    canonical = build_canonical_quantity_input(enriched_quantity_rows(rows))
    return compile_custody_interpreters(rows, canonical, wallet_refs_by_id=refs)


def test_proven_privacy_tagged_fanout_clears_the_generic_privacy_blocker():
    """A derived 1:N MOVE is exactly as strong as the 1:1 case.

    Regression: the privacy-hop blocker used to be resolved before derivation, so
    only ``detect_intra_transfers``' 1-out/1-in shape could clear it. A proven
    fan-out was booked as a MOVE *and* blocked every report on cardinality alone.
    """

    compiled = _privacy_fanout_rows(authoritative_destinations=True)

    assert [item["reason"] for item in compiled.quarantines] == []
    assert compiled.blocked_transaction_ids == ()
    assert len(compiled.claims) >= 1


def test_unproven_privacy_tagged_fanout_still_blocks():
    """The negative control for the fix above.

    Clearing the blocker must require an authoritative observation on *both*
    ends. A destination we have not observed is still an unexplained privacy hop.
    """

    compiled = _privacy_fanout_rows(authoritative_destinations=False)

    assert "privacy_hop_unresolved" in {
        item["reason"] for item in compiled.quarantines
    }
    assert compiled.blocked_transaction_ids != ()
