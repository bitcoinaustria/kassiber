"""Finalized-tax projection contract.

This is the hard boundary between custody arbitration and tax lot calculation.
It converts only selected finalized quantity slices into RP2-shaped event rows
and MOVE pairs.  Raw transaction rows intentionally have no route through the
production tax-engine input contract.
"""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal
from typing import Any, Mapping, Sequence

from ..msat import msat_to_btc
from ..transfers import is_bitcoin_rail_pair
from .privacy_hops import privacy_hop_evidence_from_row
from .custody_evidence import row_principal_msat
from .custody_quantity import (
    EXTERNAL_CONFIRMED,
    EXTERNAL_PRESUMED,
    INTERNAL_REVIEWED,
    INTERNAL_VERIFIED,
    ArbitratedSlice,
    QuantityObservation,
    QuantitySlice,
)
from .custody_quantity_runtime import CanonicalQuantityState


def _field(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    if hasattr(row, "keys") and key not in row.keys():
        return default
    if hasattr(row, "get"):
        return row.get(key, default)
    return row[key]


def _slice_key(item: QuantitySlice) -> tuple[str, int, int]:
    return (item.observation_hash, item.start_msat, item.end_msat)


def _valid_direct_payouts_by_source(
    records: Sequence[Mapping[str, Any]],
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> dict[str, Mapping[str, Any]]:
    """Return only positive, source-bounded reviewed payout allocations."""

    valid: dict[str, Mapping[str, Any]] = {}
    for record in records:
        source_id = str(_field(record, "out_transaction_id") or "")
        source_row = rows_by_id.get(source_id)
        if source_row is None:
            continue
        source_amount = row_principal_msat(source_row)
        reviewed_value = _field(record, "out_amount")
        reviewed_amount = (
            source_amount
            if reviewed_value in (None, "")
            else int(reviewed_value)
        )
        if 0 < reviewed_amount <= source_amount:
            valid[source_id] = record
    return valid


@dataclass(frozen=True)
class FinalizedTaxProjection:
    """The only production input accepted by a tax engine.

    ``rows`` is a tax-event projection, not a collection of imported rows.
    Every row carries ``custody_finalized_tax_projection=True`` and is either a
    selected external slice, an unconsumed inbound slice, or one side of a
    selected move.  ``intra_pairs`` contains only selected same-asset moves.
    """

    rows: tuple[Mapping[str, Any], ...]
    intra_pairs: tuple[Mapping[str, Any], ...]
    cross_asset_pairs: tuple[Mapping[str, Any], ...]
    quarantines: tuple[Mapping[str, Any], ...]
    selected_move_ids: tuple[str, ...]

    def __post_init__(self) -> None:
        row_ids: set[str] = set()
        for row in self.rows:
            if not bool(_field(row, "custody_finalized_tax_projection", False)):
                raise TypeError("tax engines accept only finalized custody projection rows")
            row_id = str(_field(row, "id") or "")
            if not row_id or row_id in row_ids:
                raise ValueError("finalized tax projection rows need unique ids")
            row_ids.add(row_id)
        for pair in self.intra_pairs:
            out_row, in_row = pair.get("out"), pair.get("in")
            if not isinstance(out_row, Mapping) or not isinstance(in_row, Mapping):
                raise ValueError("finalized move pairs require projected out and in rows")
            if str(_field(out_row, "id")) not in row_ids or str(_field(in_row, "id")) not in row_ids:
                raise ValueError("finalized move pairs cannot reference raw/unprojected rows")


def _base_row(
    observation: QuantityObservation,
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> dict[str, Any]:
    source = rows_by_id.get(observation.transaction_id) or rows_by_id.get(
        observation.anchor_transaction_id
    )
    result = dict(source or {})
    result.update(
        {
            "wallet_id": observation.wallet_id,
            "asset": observation.asset,
            "direction": observation.direction,
            "occurred_at": observation.occurred_at,
            "external_id": result.get("external_id") or observation.event_key.native_event_id,
            "raw_json": result.get("raw_json") or {},
            "amount_includes_fee": False,
        }
    )
    return result


def _projected_slice_row(
    observation: QuantityObservation,
    quantity: QuantitySlice,
    rows_by_id: Mapping[str, Mapping[str, Any]],
    *,
    side: str,
    fee_msat: int = 0,
) -> dict[str, Any]:
    base = _base_row(observation, rows_by_id)
    amount_msat = quantity.amount_msat
    original_amount = int(base.get("amount") or observation.amount_msat or 0)
    # An importer may have supplied only an absolute fiat value.  Slice it
    # deterministically so RP2 never sees a 9.9-BTC tax event with the 10-BTC
    # cash value.  Unit rates remain valid unchanged.
    if original_amount > 0 and base.get("fiat_rate") in (None, ""):
        value = base.get("fiat_value_exact")
        if value in (None, ""):
            value = base.get("fiat_value")
        if value not in (None, ""):
            try:
                rate = Decimal(str(value)) / msat_to_btc(original_amount)
            except (ArithmeticError, ValueError):
                rate = None
            if rate is not None:
                base["fiat_rate"] = float(rate)
                base["fiat_rate_exact"] = format(rate, "f")
    base["fiat_value"] = None
    base["fiat_value_exact"] = None
    base.update(
        {
            "id": (
                f"custody-tax:{quantity.observation_hash}:"
                f"{quantity.start_msat}:{quantity.end_msat}:{side}"
            ),
            "journal_transaction_id": observation.anchor_transaction_id,
            "amount": amount_msat,
            "fee": fee_msat,
            "amount_includes_fee": False,
            "custody_finalized_tax_projection": True,
            "custody_quantity_hash": observation.quantity_hash,
            "custody_slice_start_msat": quantity.start_msat,
            "custody_slice_end_msat": quantity.end_msat,
        }
    )
    return base


def _projected_fee_row(
    observation: QuantityObservation,
    amount_msat: int,
    rows_by_id: Mapping[str, Mapping[str, Any]],
    *,
    side: str,
) -> dict[str, Any]:
    base = _base_row(observation, rows_by_id)
    base.update(
        {
            "id": f"custody-tax:{observation.quantity_hash}:{side}:{amount_msat}",
            "journal_transaction_id": observation.anchor_transaction_id,
            "amount": 0,
            "fee": amount_msat,
            "fiat_value": None,
            "fiat_value_exact": None,
            "amount_includes_fee": False,
            "custody_finalized_tax_projection": True,
            "custody_quantity_hash": observation.quantity_hash,
            "custody_slice_start_msat": 0,
            "custody_slice_end_msat": 0,
        }
    )
    return base


def _direct_payout_settlement_rows(
    source: QuantityObservation,
    payout: Mapping[str, Any],
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> tuple[dict[str, Any], dict[str, Any]] | None:
    payout_amount_msat = int(_field(payout, "payout_amount") or 0)
    payout_value_raw = _field(payout, "payout_fiat_value")
    if payout_amount_msat <= 0 or payout_value_raw in (None, ""):
        return None
    payout_value = Decimal(str(payout_value_raw))
    if payout_value <= 0:
        return None
    payout_rate = payout_value / msat_to_btc(payout_amount_msat)
    payout_id = str(_field(payout, "id") or source.anchor_transaction_id)
    payout_at = str(_field(payout, "payout_occurred_at") or source.occurred_at)
    base = _base_row(source, rows_by_id)
    base.update(
        {
            "asset": str(_field(payout, "payout_asset") or "").upper(),
            "amount": payout_amount_msat,
            "fee": 0,
            "fiat_rate": float(payout_rate),
            "fiat_rate_exact": format(payout_rate, "f"),
            "fiat_value": float(payout_value),
            "fiat_value_exact": format(payout_value, "f"),
            "occurred_at": payout_at,
            "external_id": _field(payout, "payout_external_id")
            or f"direct-payout:{payout_id}",
            "journal_transaction_id": source.anchor_transaction_id,
            "amount_includes_fee": False,
            "custody_finalized_tax_projection": True,
            "custody_quantity_hash": source.quantity_hash,
            "custody_slice_start_msat": 0,
            "custody_slice_end_msat": payout_amount_msat,
        }
    )
    inbound = {
        **base,
        "id": f"direct-payout:{payout_id}:in",
        "direction": "inbound",
        "kind": "direct_swap_payout_in",
    }
    outbound = {
        **base,
        "id": f"direct-payout:{payout_id}:out",
        "direction": "outbound",
        "kind": "sell",
    }
    return inbound, outbound


def _quarantine(
    profile: Mapping[str, Any], transaction_id: str, reason: str, detail: Mapping[str, Any]) -> dict[str, Any]:
    import json

    return {
        "transaction_id": transaction_id,
        "workspace_id": profile["workspace_id"],
        "profile_id": profile["id"],
        "reason": reason,
        "detail_json": json.dumps(dict(detail), sort_keys=True),
    }


def compile_finalized_tax_projection(
    profile: Mapping[str, Any],
    rows: Sequence[Mapping[str, Any]],
    state: CanonicalQuantityState,
    *,
    non_event_transaction_ids: Sequence[str] = (),
    blocked_transaction_ids: Sequence[str] = (),
    interpreter_quarantines: Sequence[Mapping[str, Any]] = (),
    direct_payout_records: Sequence[Mapping[str, Any]] = (),
) -> FinalizedTaxProjection:
    """Compile selected custody slices into the irreversible tax input."""

    rows_by_id = {str(_field(row, "id") or ""): row for row in rows}
    observations = {item.quantity_hash: item for item in state.projection.observations}
    eligible = {_slice_key(item.source) for item in state.tax_eligibility.eligible_decisions}
    non_events = {str(item) for item in non_event_transaction_ids if item}
    explicitly_blocked = {str(item) for item in blocked_transaction_ids if item}
    blocked_anchor_ids = {
        transaction_id
        for issue in state.issues
        if issue.reason.startswith("custody_component_")
        or issue.issue_type in {
            "component_claim_compile_failed",
            "quantity_claim_bundle_invalid",
        }
        for transaction_id in issue.transaction_ids
    }
    payouts_by_source = _valid_direct_payouts_by_source(
        direct_payout_records,
        rows_by_id,
    )

    issue_reasons_by_transaction: dict[str, set[str]] = {}
    for issue in state.issues:
        if issue.issue_type == "custody_interpreter_blocked":
            # The interpreter supplied the more specific, user-actionable
            # quarantine record above; do not add a generic duplicate.
            continue
        for transaction_id in issue.transaction_ids:
            issue_reasons_by_transaction.setdefault(transaction_id, set()).add(
                issue.reason
            )
    decisions = tuple(
        sorted(
            state.projection.decisions,
            key=lambda item: (
                observations[item.source.observation_hash].occurred_at,
                item.source.observation_hash,
                item.source.start_msat,
                item.source.end_msat,
            ),
        )
    )
    # A rejected edge invalidates the whole connected custody path.  Booking a
    # reviewed prefix while quarantining only its rejected suffix would create
    # a carrying-value MOVE into an observation whose onward ownership is
    # unresolved.  Close the block set over selected internal moves before any
    # row is projected so the component fails atomically.
    move_neighbors: dict[str, set[str]] = {}
    for decision in decisions:
        if decision.target is None or decision.state not in {
            INTERNAL_VERIFIED,
            INTERNAL_REVIEWED,
        }:
            continue
        source = observations[decision.source.observation_hash]
        target = observations[decision.target.observation_hash]
        source_id = source.anchor_transaction_id
        target_id = target.anchor_transaction_id
        move_neighbors.setdefault(source_id, set()).add(target_id)
        move_neighbors.setdefault(target_id, set()).add(source_id)
    effectively_blocked = set(explicitly_blocked)
    frontier = list(explicitly_blocked)
    while frontier:
        transaction_id = frontier.pop()
        for neighbor in move_neighbors.get(transaction_id, ()):
            if neighbor not in effectively_blocked:
                effectively_blocked.add(neighbor)
                frontier.append(neighbor)
    event_order_by_hash: dict[str, tuple[str, str, str, str, str]] = {}
    for event in state.canonical_input.events:
        event_order = (
            min((leg.occurred_at for leg in event.legs), default=""),
            event.event_key.chain,
            event.event_key.network,
            event.event_key.native_namespace,
            event.event_key.native_event_id,
        )
        for leg in event.legs:
            event_order_by_hash[leg.quantity_hash] = event_order
    def is_blocked_by_basis_barrier(observation: QuantityObservation) -> bool:
        basis_barrier = state.tax_eligibility.barrier_for(observation)
        if basis_barrier is None:
            return False
        order = event_order_by_hash.get(observation.quantity_hash)
        if order is None:
            return True
        # The tie-break fields make replay deterministic; they do not establish
        # chronology between distinct events recorded at the same timestamp.
        # Only the barrier event itself may retain its finalized sibling slices.
        return order[0] > basis_barrier[0] or (
            order[0] == basis_barrier[0] and order != basis_barrier
        )

    projection_rows: list[Mapping[str, Any]] = []
    intra_pairs: list[Mapping[str, Any]] = []
    cross_asset_pairs: list[Mapping[str, Any]] = []
    quarantines: dict[tuple[str, str], Mapping[str, Any]] = {}
    interpreter_quarantine_keys: set[tuple[str, str]] = set()
    selected_moves: list[str] = []
    allocated_fee_sources: set[str] = set()
    fee_by_pair_bundle: dict[str, int] = {}
    consumed_fee_bundles: set[str] = set()

    for item in interpreter_quarantines:
        transaction_id = str(_field(item, "transaction_id") or "")
        reason = str(_field(item, "reason") or "custody_interpreter_blocked")
        if transaction_id:
            key = (transaction_id, reason)
            quarantines[key] = dict(item)
            interpreter_quarantine_keys.add(key)

    for transaction_id in sorted(effectively_blocked - explicitly_blocked):
        if transaction_id not in rows_by_id:
            continue
        reason = "transfer_pair_dependency_blocked"
        quarantines[(transaction_id, reason)] = _quarantine(
            profile,
            transaction_id,
            reason,
            {
                "required_for": "complete_transfer_component",
                "blocked_by_transaction_ids": sorted(explicitly_blocked),
            },
        )

    def pair_bundle(decision: ArbitratedSlice) -> str | None:
        if decision.atomic_bundle_id:
            return decision.atomic_bundle_id
        claim_id = str(decision.selected_claim_id or "")
        if claim_id.startswith("pair:"):
            parts = claim_id.split(":", 2)
            if len(parts) >= 2:
                return f"pair:{parts[1]}"
        return None

    for decision in decisions:
        if decision.destination_kind != "fee":
            continue
        bundle = pair_bundle(decision)
        if bundle is not None:
            fee_by_pair_bundle[bundle] = (
                fee_by_pair_bundle.get(bundle, 0) + decision.source.amount_msat
            )

    def fee_for(decision: ArbitratedSlice) -> int:
        source = observations[decision.source.observation_hash]
        if source.quantity_hash in allocated_fee_sources:
            return 0
        allocated_fee_sources.add(source.quantity_hash)
        return source.fee_msat

    def quarantine_basis_barrier(source: QuantityObservation) -> None:
        if source.anchor_transaction_id not in rows_by_id:
            return
        source_barrier = state.tax_eligibility.barrier_for(source)
        source_reasons = issue_reasons_by_transaction.get(
            source.anchor_transaction_id, set()
        )
        reason = (
            "transfer_fee_implausible"
            if "transfer_fee_implausible" in source_reasons
            else "custody_basis_barrier"
        )
        key = (source.anchor_transaction_id, reason)
        quarantines.setdefault(
            key,
            _quarantine(
                profile,
                source.anchor_transaction_id,
                reason,
                {
                    "required_for": "resolved_prior_custody_basis",
                    "barrier_event": source_barrier,
                    "source_quantity_hash": source.quantity_hash,
                },
            ),
        )

    def blocked_decision_may_project(
        decision: ArbitratedSlice,
        source: QuantityObservation,
        target: QuantityObservation | None,
    ) -> bool:
        """Keep component-atomic interpreter failures out of tax projection."""

        blocked_ids = {
            observation.anchor_transaction_id
            for observation in (source, target)
            if observation is not None
            and observation.anchor_transaction_id in effectively_blocked
        }
        if not blocked_ids:
            return True
        return False

    for decision in decisions:
        source = observations[decision.source.observation_hash]
        target = (
            observations[decision.target.observation_hash]
            if decision.target is not None
            else None
        )
        if not blocked_decision_may_project(decision, source, target):
            continue
        is_eligible = _slice_key(decision.source) in eligible
        if not is_eligible:
            quarantine_basis_barrier(source)
            continue
        if decision.target is not None and decision.state in {
            INTERNAL_VERIFIED,
            INTERNAL_REVIEWED,
        }:
            assert target is not None
            bundle = pair_bundle(decision)
            reviewed_fee = (
                0
                if bundle in consumed_fee_bundles
                else fee_by_pair_bundle.get(bundle or "", 0)
            )
            if bundle is not None and reviewed_fee:
                consumed_fee_bundles.add(bundle)
            raw_fee = fee_for(decision)
            projected_fee = (
                reviewed_fee
                if reviewed_fee
                and decision.component_id is not None
                and decision.reason == "reviewed_custody_component"
                and decision.transfer_kind != "swap-refund"
                else raw_fee + reviewed_fee
            )
            out_row = _projected_slice_row(
                source,
                decision.source,
                rows_by_id,
                side="move-out",
                # An ordinary reviewed custody component's fee allocation
                # preserves the same observed miner fee inside the authored
                # bundle, so it replaces the raw sibling. Reviewed conversion
                # losses, native graph residuals, and failed-swap refunds are
                # custody losses distinct from the outbound miner fee and
                # remain additive.
                fee_msat=projected_fee,
            )
            in_row = _projected_slice_row(
                target, decision.target, rows_by_id, side="move-in"
            )
            projection_rows.extend((out_row, in_row))
            move_id = decision.selected_claim_id or out_row["id"]
            selected_moves.append(move_id)
            if source.asset == target.asset:
                # The custody arbitrator has already selected this exact move.
                # Preserve that fact for the legacy tax-event adapter so it
                # cannot re-quarantine a finalized native/reviewed CoinJoin
                # path merely because the projected rows retain privacy tags.
                privacy_kind = (
                    "custody-reviewed-coinjoin"
                    if privacy_hop_evidence_from_row(out_row) is not None
                    or privacy_hop_evidence_from_row(in_row) is not None
                    else None
                )
                intra_pairs.append(
                    {
                        "out": out_row,
                        "in": in_row,
                        "pair_id": move_id,
                        "source": decision.reason,
                        "policy": "carrying-value",
                        "group_id": decision.atomic_bundle_id,
                        **({"kind": privacy_kind} if privacy_kind else {}),
                    }
                )
            else:
                component_id = None
                if move_id.startswith("component:"):
                    component_id = move_id.split(":", 2)[1]
                durable_pair_id = move_id
                if move_id.startswith("pair:"):
                    durable_pair_id = move_id.split(":", 2)[1]
                cross_asset_pairs.append(
                    {
                        "pair_id": durable_pair_id,
                        "component_id": decision.component_id or component_id,
                        "out_id": out_row["id"],
                        "in_id": in_row["id"],
                        "out_transaction_id": source.anchor_transaction_id,
                        "in_transaction_id": target.anchor_transaction_id,
                        "out_asset": source.asset,
                        "in_asset": target.asset,
                        "policy": decision.transfer_policy or "carrying-value",
                        "kind": decision.transfer_kind or "custody_cross_rail",
                    }
                )
            continue
        if source.anchor_transaction_id in non_events:
            non_event_fee = fee_for(decision)
            if non_event_fee:
                projection_rows.append(
                    _projected_fee_row(
                        source,
                        non_event_fee,
                        rows_by_id,
                        side="non-event-fee",
                    )
                )
            continue
        if decision.state in {EXTERNAL_CONFIRMED, EXTERNAL_PRESUMED}:
            if decision.destination_kind == "fee":
                bundle = pair_bundle(decision)
                if bundle is not None and bundle in consumed_fee_bundles:
                    continue
                projection_rows.append(
                    _projected_fee_row(
                        source,
                        decision.source.amount_msat,
                        rows_by_id,
                        side="reviewed-fee",
                    )
                )
                continue
            payout = payouts_by_source.get(source.anchor_transaction_id)
            payout_asset = (
                str(_field(payout, "payout_asset") or "").upper()
                if payout is not None
                else ""
            )
            is_cross_asset_carry = bool(
                payout
                and _field(payout, "policy") == "carrying-value"
                and source.asset != payout_asset
                and (
                    str(_field(profile, "tax_country") or "").lower() == "at"
                    or is_bitcoin_rail_pair(source.asset, payout_asset)
                )
            )
            if is_cross_asset_carry:
                settlement = _direct_payout_settlement_rows(
                    source, payout, rows_by_id
                )
                if settlement is None:
                    quarantines.setdefault(
                        (source.anchor_transaction_id, "at_swap_price_required"),
                        _quarantine(
                            profile,
                            source.anchor_transaction_id,
                            "at_swap_price_required",
                            {
                                "required_for": "direct_payout_carrying_value",
                                "payout_id": _field(payout, "id"),
                            },
                        ),
                    )
                    continue
                projected_source = _projected_slice_row(
                    source,
                    decision.source,
                    rows_by_id,
                    side="direct-payout-source",
                    fee_msat=fee_for(decision),
                )
                payout_in, payout_out = settlement
                projection_rows.extend((projected_source, payout_in, payout_out))
                pair_id = f"direct-payout:{_field(payout, 'id')}"
                cross_asset_pairs.append(
                    {
                        "pair_id": pair_id,
                        "component_id": None,
                        "kind": _field(payout, "kind") or "direct-swap-payout",
                        "policy": "carrying-value",
                        "out_id": projected_source["id"],
                        "in_id": payout_in["id"],
                        "out_transaction_id": source.anchor_transaction_id,
                        "in_transaction_id": None,
                        "out_asset": source.asset,
                        "in_asset": payout_asset,
                    }
                )
                continue
            projected = _projected_slice_row(
                source,
                decision.source,
                rows_by_id,
                side="external",
                fee_msat=fee_for(decision),
            )
            if decision.external_economic_subtype is not None:
                # Reviewed custody finality and tax meaning are deliberately
                # separate, but the closed economic subtype must survive into
                # normalization.  In particular gift/lost must never inherit a
                # generic outbound kind and silently become a market sale.
                projected["kind"] = decision.external_economic_subtype
                projected["custody_external_economic_subtype"] = (
                    decision.external_economic_subtype
                )
                # The exact reviewed external sink closes the former privacy
                # ownership question for this slice.  Do not let the raw
                # boundary marker re-open it downstream and mask the reviewed
                # payment/gift/loss semantics.
                projected["privacy_boundary"] = None
            payout_value = _field(payout, "payout_fiat_value") if payout else None
            if payout_value not in (None, "") and decision.source.amount_msat > 0:
                value = Decimal(str(payout_value))
                rate = value / msat_to_btc(decision.source.amount_msat)
                projected.update(
                    {
                        "fiat_rate": float(rate),
                        "fiat_rate_exact": format(rate, "f"),
                        "fiat_value": float(value),
                        "fiat_value_exact": format(value, "f"),
                        "pricing_method": "direct_payout_review",
                    }
                )
            projection_rows.append(projected)

    # Inbound slices not consumed by a selected custody move are genuine tax
    # acquisition candidates.  A rowless virtual target is never promoted into
    # an acquisition if its native claim failed/was blocked.
    for observation in sorted(
        state.projection.observations,
        key=lambda item: (item.occurred_at, item.quantity_hash),
    ):
        if observation.direction != "inbound" or observation.transaction_id not in rows_by_id:
            continue
        if observation.anchor_transaction_id in non_events:
            continue
        if is_blocked_by_basis_barrier(observation):
            continue
        if observation.anchor_transaction_id in blocked_anchor_ids or (
            observation.anchor_transaction_id in effectively_blocked
        ):
            continue
        cursor = 0
        targets = sorted(
            (
                item.target
                for item in decisions
                if item.target is not None
                and item.target.observation_hash == observation.quantity_hash
            ),
            key=lambda item: (item.start_msat, item.end_msat),
        )
        for target in targets:
            if target.start_msat > cursor:
                candidate = QuantitySlice(observation.quantity_hash, cursor, target.start_msat)
                projection_rows.append(_projected_slice_row(observation, candidate, rows_by_id, side="inbound"))
            cursor = max(cursor, target.end_msat)
        if cursor < observation.principal_msat:
            candidate = QuantitySlice(observation.quantity_hash, cursor, observation.principal_msat)
            projection_rows.append(_projected_slice_row(observation, candidate, rows_by_id, side="inbound"))

    # A known network fee is a finalized sibling of the principal decision.
    # If that principal remains in custody suspense, the fee still left the
    # wallet at the barrier event and can consume the basis known immediately
    # before it. Later events remain blocked by the normal pool barrier.
    projected_hashes = {
        str(_field(row, "custody_quantity_hash") or "") for row in projection_rows
    }
    for observation in state.projection.observations:
        if (
            observation.direction == "outbound"
            and observation.fee_msat > 0
            and observation.quantity_hash not in projected_hashes
            and observation.anchor_transaction_id not in effectively_blocked
            and observation.anchor_transaction_id not in non_events
            and not is_blocked_by_basis_barrier(observation)
        ):
            projection_rows.append(
                _projected_fee_row(
                    observation,
                    observation.fee_msat,
                    rows_by_id,
                    side="fee-only",
                )
            )

    for issue in state.issues:
        if issue.issue_type == "custody_interpreter_blocked":
            continue
        for transaction_id in issue.transaction_ids:
            if transaction_id not in rows_by_id:
                continue
            quarantine_reason = (
                "transfer_fee_implausible"
                if issue.reason == "transfer_fee_implausible"
                else "custody_quantity_unresolved"
            )
            key = (transaction_id, quarantine_reason)
            quarantines.setdefault(
                key,
                _quarantine(
                    profile,
                    transaction_id,
                    quarantine_reason,
                    {
                        "blocker_code": issue.reason,
                        "required_for": "finalized_custody_tax_projection",
                        "issue_id": issue.issue_id,
                        **(
                            {
                                "resolution": (
                                    "review the authored component evidence and create "
                                    "or supersede the revision"
                                )
                            }
                            if issue.reason.startswith("custody_component_")
                            or issue.issue_type == "component_claim_compile_failed"
                            else {}
                        ),
                    },
                ),
            )

    # Taxable cross-asset reviews do not assert continuing custody, so they do
    # not emit an internal quantity claim.  Preserve them as audit/tax-relation
    # metadata only when both finalized economic legs survived every custody
    # gate.  Carrying-value relations are already emitted from the selected
    # internal move above and are deduplicated by their durable pair id.
    existing_cross_pair_ids = {
        str(pair.get("pair_id") or "") for pair in cross_asset_pairs
    }
    projected_by_anchor: dict[str, list[Mapping[str, Any]]] = {}
    for projected_row in projection_rows:
        anchor = str(_field(projected_row, "journal_transaction_id") or "")
        if anchor:
            projected_by_anchor.setdefault(anchor, []).append(projected_row)
    for pair in state.reviewed_conversion_pairs:
        pair_id = str(_field(pair, "pair_id") or "")
        if not pair_id or pair_id in existing_cross_pair_ids:
            continue
        out_transaction_id = str(_field(pair, "out_id") or "")
        in_transaction_id = str(_field(pair, "in_id") or "")
        out_candidates = projected_by_anchor.get(out_transaction_id, [])
        in_candidates = projected_by_anchor.get(in_transaction_id, [])
        reviewed_out_amount = _field(pair, "out_amount")
        if reviewed_out_amount not in (None, ""):
            out_candidates = [
                row
                for row in out_candidates
                if int(_field(row, "amount") or 0) == int(reviewed_out_amount)
            ]
        reviewed_in_amount = _field(pair, "in_amount")
        if reviewed_in_amount not in (None, ""):
            in_candidates = [
                row
                for row in in_candidates
                if int(_field(row, "amount") or 0) == int(reviewed_in_amount)
            ]
        out_candidates = [
            row for row in out_candidates if _field(row, "direction") == "outbound"
        ]
        in_candidates = [
            row for row in in_candidates if _field(row, "direction") == "inbound"
        ]
        if len(out_candidates) != 1 or len(in_candidates) != 1:
            continue
        out_row, in_row = out_candidates[0], in_candidates[0]
        cross_asset_pairs.append(
            {
                "pair_id": pair_id,
                "component_id": _field(pair, "component_id"),
                "out_id": out_row["id"],
                "in_id": in_row["id"],
                "out_transaction_id": out_transaction_id,
                "in_transaction_id": in_transaction_id,
                "out_asset": _field(pair, "out_asset") or _field(out_row, "asset"),
                "in_asset": _field(pair, "in_asset") or _field(in_row, "asset"),
                "policy": _field(pair, "policy") or "taxable",
                "kind": _field(pair, "kind") or "swap",
            }
        )
        existing_cross_pair_ids.add(pair_id)
    return FinalizedTaxProjection(
        rows=tuple(sorted(projection_rows, key=lambda row: (str(row["occurred_at"]), str(row["id"])))),
        intra_pairs=tuple(intra_pairs),
        cross_asset_pairs=tuple(cross_asset_pairs),
        quarantines=tuple(
            sorted(
                quarantines.values(),
                key=lambda item: (
                    str(item["transaction_id"]),
                    0
                    if (str(item["transaction_id"]), str(item["reason"]))
                    in interpreter_quarantine_keys
                    else 1,
                    str(item["reason"]),
                ),
            )
        ),
        selected_move_ids=tuple(sorted(selected_moves)),
    )


__all__ = ["FinalizedTaxProjection", "compile_finalized_tax_projection"]
