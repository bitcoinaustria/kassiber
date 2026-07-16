"""Compile reviewed custody-component allocations into exact quantity claims.

This is the narrow seam between the authored, immutable component substrate
and the country-neutral quantity arbitrator. It intentionally emits no journal
or RP2 rows. Network fees remain observation facts; a reviewed conversion may
explicitly allocate source principal to a fee. Suspense remains a no-target
claim.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Sequence

from ..errors import AppError
from .custody_allocations import infer_component_allocations
from .custody_quantity import (
    CUSTODY_SUSPENSE,
    EXTERNAL_CONFIRMED,
    INTERNAL_REVIEWED,
    ClaimPriority,
    QuantityClaim,
    QuantityDomain,
    QuantityObservation,
    QuantitySlice,
)


@dataclass(frozen=True)
class ComponentClaimCompilation:
    component_id: str
    claims: tuple[QuantityClaim, ...]
    suspense_msat: int
    reviewed_conversion_pairs: tuple[Mapping[str, Any], ...] = ()
    reviewed_direct_payouts: tuple[Mapping[str, Any], ...] = ()


def _error(message: str, **details: Any) -> AppError:
    return AppError(
        message,
        code="custody_component_claim_compile",
        hint="Review the component's exact source, destination, and suspense allocations.",
        details=details,
    )


def _transaction_id(leg: Mapping[str, Any]) -> str:
    return str(leg.get("anchor_transaction_id") or leg.get("transaction_id") or "")


def _location_key(leg: Mapping[str, Any]) -> tuple[str, str, str, str, str]:
    location = str(leg.get("wallet_id") or leg.get("location_ref") or "")
    if not location:
        raise _error(
            "transaction-less route leg needs a custody location",
            leg_id=leg.get("id"),
        )
    return (
        location,
        str(leg.get("asset") or "").upper(),
        str(leg.get("exposure") or ""),
        str(leg.get("conservation_unit") or "msat"),
        str(leg.get("rail") or ""),
    )


def _direct_payout_record(
    term: Mapping[str, Any],
    sink: Mapping[str, Any],
    source_observation: QuantityObservation,
    *,
    component_id: str,
    source_amount_msat: int,
    sink_amount_msat: int,
) -> dict[str, Any]:
    return {
        "id": str(term.get("legacy_source_id")),
        "component_id": component_id,
        "out_transaction_id": source_observation.anchor_transaction_id,
        "deleted_at": None,
        "kind": str(term.get("review_kind") or "direct-swap-payout"),
        "policy": str(term.get("tax_policy") or "taxable"),
        "out_amount": source_amount_msat,
        "payout_asset": term.get("payout_asset") or sink.get("asset"),
        "payout_amount": term.get("payout_amount_msat") or sink_amount_msat,
        "payout_occurred_at": term.get("payout_occurred_at")
        or sink.get("occurred_at"),
        "payout_fiat_value": term.get("payout_fiat_value_exact"),
        "payout_external_id": term.get("payout_external_id"),
        "counterparty": term.get("counterparty"),
        "swap_fee_msat": term.get("swap_fee_msat"),
        "swap_fee_kind": term.get("swap_fee_kind"),
        "notes": term.get("review_notes"),
        "confidence_at_review": term.get("confidence_at_review"),
        "review_source": term.get("review_source"),
    }


def _flatten_transactionless_routes(
    component_id: str,
    legs: Mapping[str, Mapping[str, Any]],
    allocations: Sequence[Mapping[str, Any]],
    *,
    conservation_mode: str,
    finalized_retained_leg_ids: frozenset[str] = frozenset(),
) -> tuple[dict[str, Any], ...]:
    """Collapse reviewed missing-wallet hops into observed endpoint flows."""

    pending = [dict(item) for item in allocations]
    pools: dict[
        tuple[str, str, str, str, str],
        list[tuple[str, int, tuple[str, ...]]],
    ] = {}
    flattened: list[dict[str, Any]] = []

    def available(key: tuple[str, str, str, str, str]) -> int:
        return sum(amount for _source, amount, _path in pools.get(key, ()))

    def take(
        key: tuple[str, str, str, str, str], amount: int
    ) -> list[tuple[str, int, tuple[str, ...]]]:
        queue = pools.setdefault(key, [])
        result: list[tuple[str, int, tuple[str, ...]]] = []
        remaining = amount
        while remaining:
            source_leg_id, token_amount, path = queue.pop(0)
            used = min(remaining, token_amount)
            result.append((source_leg_id, used, path))
            remaining -= used
            if used < token_amount:
                queue.insert(0, (source_leg_id, token_amount - used, path))
        return result

    while pending:
        progressed = False
        for allocation in list(pending):
            source_id = str(allocation.get("source_leg_id") or "")
            sink_id = str(allocation.get("sink_leg_id") or "")
            source = legs.get(source_id)
            sink = legs.get(sink_id)
            if source is None or sink is None or source.get("role") != "source":
                raise _error(
                    "component allocation endpoints are invalid",
                    component_id=component_id,
                    allocation_id=allocation.get("id"),
                )
            source_amount = int(allocation.get("source_amount_msat") or 0)
            sink_amount = int(allocation.get("sink_amount_msat") or 0)
            if source_amount <= 0 and sink_amount <= 0:
                pending.remove(allocation)
                progressed = True
                continue
            if source_amount != sink_amount:
                if conservation_mode != "conversion":
                    raise _error(
                        "quantity claim allocation does not conserve exact msat",
                        component_id=component_id,
                        allocation_id=allocation.get("id"),
                    )
                # A reviewed conversion conserves its exact valuation, not an
                # invented equality between unlike asset quantities.  It must
                # start from an observed source so the outgoing quantity can
                # be selected exactly.  The observed destination is checked
                # below when the tax relation is compiled.
                if not _transaction_id(source):
                    raise _error(
                        "transaction-less conversion sources cannot emit tax claims",
                        component_id=component_id,
                        allocation_id=allocation.get("id"),
                    )
                flattened.append(allocation)
                pending.remove(allocation)
                progressed = True
                continue

            allocation_id = str(allocation.get("id") or len(flattened))
            if _transaction_id(source):
                tokens = [(source_id, source_amount, (allocation_id,))]
            else:
                source_key = _location_key(source)
                if available(source_key) < source_amount:
                    continue
                tokens = [
                    (origin, amount, (*path, allocation_id))
                    for origin, amount, path in take(source_key, source_amount)
                ]

            sink_role = str(sink.get("role") or "")
            if (
                sink_role in {"destination", "retained"}
                and not _transaction_id(sink)
                and sink_id not in finalized_retained_leg_ids
            ):
                pools.setdefault(_location_key(sink), []).extend(tokens)
            else:
                for token_index, (origin, amount, path) in enumerate(tokens):
                    flattened.append(
                        {
                            **allocation,
                            "id": (
                                f"route:{':'.join(path)}:{token_index}"
                                if len(path) > 1
                                else allocation.get("id")
                            ),
                            "source_leg_id": origin,
                            "source_amount_msat": amount,
                            "sink_amount_msat": amount,
                        }
                    )
            pending.remove(allocation)
            progressed = True
        if not progressed:
            raise _error(
                "transaction-less component route is incomplete or ambiguous",
                component_id=component_id,
                pending_allocation_ids=sorted(
                    str(item.get("id") or "") for item in pending
                ),
            )

    stranded = {
        "|".join(key): available(key)
        for key in sorted(pools)
        if available(key) > 0
    }
    if stranded:
        raise _error(
            "transaction-less component route ends without an observed or classified sink",
            component_id=component_id,
            stranded_msat=stranded,
        )
    return tuple(flattened)


def compile_component_quantity_claims(
    component: Mapping[str, Any],
    observations_by_transaction: Mapping[str, QuantityObservation],
) -> ComponentClaimCompilation:
    """Return one atomic reviewed claim bundle for direct observed allocations.

    Half-open msat slices are deterministic accounting coordinates, not claims
    about physical sat ordering. Reviewed transaction-less intermediate routes
    are collapsed to their real observed endpoints before claims are emitted.
    """

    component_id = str(component.get("id") or "")
    if not component_id:
        raise _error("component id is required")
    if component.get("effective_state") != "active":
        raise _error(
            "only an effective active component can emit quantity claims",
            component_id=component_id,
            effective_state=component.get("effective_state"),
        )

    legs = {str(leg["id"]): leg for leg in component.get("legs", ())}
    is_guided_gap_component = (
        str(component.get("component_type") or "") == "manual_bridge"
        and str(component.get("evidence_kind") or "") == "custody_gap_review"
    )
    evidence = component.get("evidence")
    residual_review = (
        evidence.get("residual_classification")
        if isinstance(evidence, Mapping)
        else None
    )
    residual_classification = (
        str(residual_review.get("classification") or "")
        if isinstance(residual_review, Mapping)
        else ""
    )
    leg_residual_classifications = {
        str(leg.get("notes") or "").removeprefix("reviewed_residual:")
        for leg in legs.values()
        if is_guided_gap_component
        and str(leg.get("notes") or "").startswith("reviewed_residual:")
    }
    if not residual_classification and len(leg_residual_classifications) == 1:
        residual_classification = next(iter(leg_residual_classifications))
    finalized_retained_leg_ids = frozenset(
        leg_id
        for leg_id, leg in legs.items()
        if leg.get("role") == "retained"
        and not _transaction_id(leg)
        and str(leg.get("notes") or "")
        == "reviewed_residual:retained_custody"
    ) | frozenset(
        str(term.get("target_leg_id"))
        for term in component.get("economic_terms", ())
        if term.get("term_kind") == "direct_swap_payout"
        and term.get("target_leg_id") not in (None, "")
    )
    explicit_allocations = component.get("allocations", ())
    if not explicit_allocations and any(
        leg.get("role") == "suspense"
        and int(leg.get("amount_msat") or 0) > 0
        for leg in legs.values()
    ):
        raise _error(
            "suspense components require explicit allocations",
            component_id=component_id,
        )
    try:
        conservation_mode = str(component.get("conservation_mode") or "quantity")
        allocations = _flatten_transactionless_routes(
            component_id,
            legs,
            infer_component_allocations(component),
            conservation_mode=conservation_mode,
            finalized_retained_leg_ids=finalized_retained_leg_ids,
        )
    except Exception as exc:
        if not all(hasattr(exc, name) for name in ("code", "message", "details")):
            raise
        raise _error(
            str(exc.message),
            component_id=component_id,
            projection_code=str(exc.code),
            **dict(exc.details),
        ) from exc

    def observation(transaction_id: Any) -> QuantityObservation:
        transaction_key = str(transaction_id or "")
        item = observations_by_transaction.get(transaction_key)
        if item is None:
            raise _error(
                "component allocation references an unavailable observation",
                component_id=component_id,
                transaction_id=transaction_key,
            )
        return item

    source_cursors: dict[str, int] = {}
    target_cursors: dict[str, int] = {}
    claims: list[QuantityClaim] = []
    reviewed_conversion_pairs: list[Mapping[str, Any]] = []
    reviewed_direct_payouts: list[Mapping[str, Any]] = []
    suspense_msat = 0
    bundle_id = f"component:{component_id}"
    terms_by_edge = {
        (str(term.get("source_leg_id")), str(term.get("target_leg_id"))): term
        for term in component.get("economic_terms", ())
    }

    for index, allocation in enumerate(allocations):
        source = legs.get(str(allocation.get("source_leg_id")))
        sink = legs.get(str(allocation.get("sink_leg_id")))
        if source is None or sink is None or source.get("role") != "source":
            raise _error(
                "component allocation endpoints are invalid",
                component_id=component_id,
                allocation_id=allocation.get("id"),
            )
        source_amount = int(allocation.get("source_amount_msat") or 0)
        sink_amount = int(allocation.get("sink_amount_msat") or 0)
        if source_amount <= 0 and sink_amount <= 0:
            continue
        if conservation_mode != "conversion" and source_amount != sink_amount:
            raise _error(
                "quantity claim allocation does not conserve exact msat",
                component_id=component_id,
                allocation_id=allocation.get("id"),
            )
        source_transaction_id = _transaction_id(source)
        if not source_transaction_id:
            raise _error(
                "flattened component source is not observed",
                component_id=component_id,
                source_leg_id=source.get("id"),
            )
        source_observation = observation(source_transaction_id)
        if source_observation.direction != "outbound":
            raise _error(
                "quantity claim source observation is not outbound",
                component_id=component_id,
                source_leg_id=source.get("id"),
            )
        if conservation_mode != "conversion" and sink.get("role") == "fee":
            # The observation projector emits network fees independently of
            # principal. The authored fee allocation proves exact boundary
            # coverage but must not consume the principal slice cursor.
            if source_amount != source_observation.fee_msat:
                raise _error(
                    "component fee allocation does not match the observed fee",
                    component_id=component_id,
                    transaction_id=source_observation.transaction_id,
                    claimed_fee_msat=source_amount,
                    observed_fee_msat=source_observation.fee_msat,
                )
            continue
        source_start = source_cursors.get(source_observation.quantity_hash, 0)
        source_end = source_start + source_amount
        if source_end > source_observation.principal_msat:
            raise _error(
                "component claims exceed observed source principal",
                component_id=component_id,
                transaction_id=source_observation.transaction_id,
                claimed_msat=source_end,
                principal_msat=source_observation.principal_msat,
            )
        source_cursors[source_observation.quantity_hash] = source_end
        source_slice = QuantitySlice(
            source_observation.quantity_hash, source_start, source_end
        )

        sink_role = str(sink.get("role") or "")
        target_slice = None
        state: str
        reason: str
        target_observation: QuantityObservation | None = None
        supporting_hashes = (source_observation.evidence_detail_hash,)
        if sink_role in {"destination", "retained"}:
            sink_transaction_id = _transaction_id(sink)
            if not sink_transaction_id:
                term = terms_by_edge.get(
                    (str(source.get("id")), str(sink.get("id"))), {}
                )
                if (
                    conservation_mode == "conversion"
                    and term.get("term_kind") == "direct_swap_payout"
                ):
                    state = EXTERNAL_CONFIRMED
                    reason = "reviewed_direct_payout_source"
                    reviewed_direct_payouts.append(
                        _direct_payout_record(
                            term,
                            sink,
                            source_observation,
                            component_id=component_id,
                            source_amount_msat=source_amount,
                            sink_amount_msat=sink_amount,
                        )
                    )
                elif str(sink.get("id") or "") not in finalized_retained_leg_ids:
                    raise _error(
                        "flattened component target is not observed",
                        component_id=component_id,
                        sink_leg_id=sink.get("id"),
                    )
                else:
                    state = INTERNAL_REVIEWED
                    reason = "reviewed_retained_custody"
            else:
                target_observation = observation(sink_transaction_id)
                if target_observation.direction != "inbound":
                    raise _error(
                        "quantity claim target observation is not inbound",
                        component_id=component_id,
                        sink_leg_id=sink.get("id"),
                    )
                target_start = target_cursors.get(target_observation.quantity_hash, 0)
                target_end = target_start + sink_amount
                if target_end > target_observation.principal_msat:
                    raise _error(
                        "component claims exceed observed target principal",
                        component_id=component_id,
                        transaction_id=target_observation.transaction_id,
                        claimed_msat=target_end,
                        principal_msat=target_observation.principal_msat,
                    )
                target_cursors[target_observation.quantity_hash] = target_end
                if conservation_mode == "conversion":
                    # The inbound quantity remains an independent acquisition.
                    # Recording it as a target slice would assert that unlike
                    # quantities are the same conserved object and suppress the
                    # acquisition from tax projection.
                    state = EXTERNAL_CONFIRMED
                    reason = "reviewed_taxable_conversion_source"
                    allocation_key = str(allocation.get("id") or index)
                    term = terms_by_edge.get(
                        (str(source.get("id")), str(sink.get("id"))), {}
                    )
                    reviewed_conversion_pairs.append(
                        {
                            "pair_id": str(term.get("legacy_source_id") or (
                                f"component:{component_id}:conversion:{allocation_key}"
                            )),
                            "component_id": component_id,
                            "out_id": source_observation.anchor_transaction_id,
                            "in_id": target_observation.anchor_transaction_id,
                            "out_amount": source_amount,
                            "in_amount": sink_amount,
                            "out_asset": source_observation.asset,
                            "in_asset": target_observation.asset,
                            "policy": str(
                                term.get("tax_policy")
                                or component.get("conversion_policy")
                                or "taxable"
                            ),
                            "kind": str(
                                term.get("review_kind")
                                or component.get("component_type")
                                or "swap"
                            ),
                            "swap_fee_msat": term.get("swap_fee_msat"),
                            "swap_fee_kind": term.get("swap_fee_kind"),
                            "notes": term.get("review_notes"),
                            "confidence_at_review": term.get("confidence_at_review"),
                            "review_source": term.get("review_source"),
                        }
                    )
                else:
                    target_slice = QuantitySlice(
                        target_observation.quantity_hash, target_start, target_end
                    )
                    state = INTERNAL_REVIEWED
                    reason = "reviewed_custody_component"
                supporting_hashes = (
                    source_observation.evidence_detail_hash,
                    target_observation.evidence_detail_hash,
                )
        elif sink_role == "fee" and conservation_mode == "conversion":
            if source_amount != sink_amount:
                raise _error(
                    "conversion fee allocation must conserve its source quantity",
                    component_id=component_id,
                    allocation_id=allocation.get("id"),
                )
            state = EXTERNAL_CONFIRMED
            reason = "reviewed_conversion_fee"
        elif sink_role == "suspense":
            state = CUSTODY_SUSPENSE
            reason = "reviewed_residual_suspense"
            suspense_msat += source_amount
        elif sink_role == "external":
            state = EXTERNAL_CONFIRMED
            reason = "reviewed_external_component_allocation"
            term = terms_by_edge.get(
                (str(source.get("id")), str(sink.get("id"))), {}
            )
            if term.get("term_kind") == "direct_swap_payout":
                reason = "reviewed_direct_payout_source"
                reviewed_direct_payouts.append(
                    _direct_payout_record(
                        term,
                        sink,
                        source_observation,
                        component_id=component_id,
                        source_amount_msat=source_amount,
                        sink_amount_msat=sink_amount,
                    )
                )
        elif sink_role == "unresolved":
            raise _error(
                "an unresolved draft leg cannot emit an active quantity claim",
                component_id=component_id,
                sink_leg_id=sink.get("id"),
            )
        else:
            raise _error(
                "component sink role cannot emit a quantity claim",
                component_id=component_id,
                sink_leg_id=sink.get("id"),
                role=sink_role,
            )

        claims.append(
            QuantityClaim(
                claim_id=(
                    f"component:{component_id}:allocation:"
                    f"{allocation.get('id') or index}"
                ),
                source=source_slice,
                target=target_slice,
                state=state,
                priority=ClaimPriority.REVIEWED_COMPONENT,
                reason=reason,
                supporting_evidence_hashes=tuple(sorted(supporting_hashes)),
                atomic_bundle_id=bundle_id,
                component_id=component_id,
                destination_kind=(
                    "fee"
                    if conservation_mode == "conversion" and sink_role == "fee"
                    else (
                        "external"
                        if sink_role == "external"
                        or (
                            conservation_mode == "conversion"
                            and sink_role in {"destination", "retained"}
                        )
                        else (
                            "retained_custody"
                            if sink_role == "retained" and target_slice is None
                            else None
                        )
                    )
                ),
                external_economic_subtype=(
                    {
                        "external_payment": "payment",
                        "external_disposal": "disposal",
                        "external_gift": "gift",
                        "external_loss": "lost",
                    }.get(residual_classification)
                    if sink_role == "external"
                    and str(sink.get("notes") or "").startswith(
                        "reviewed_residual:"
                    )
                    else None
                ),
                transfer_kind=(
                    str(component.get("component_type") or "swap")
                    if conservation_mode == "conversion"
                    else None
                ),
                transfer_policy=(
                    str(component.get("conversion_policy") or "taxable")
                    if conservation_mode == "conversion"
                    else None
                ),
                allow_cross_rail=(
                    target_slice is not None
                    and target_observation is not None
                    and QuantityDomain.from_observation(source_observation).rail
                    != QuantityDomain.from_observation(target_observation).rail
                ),
            )
        )

    # An active component must describe every principal slice itself. This is
    # stronger than letting the arbitrator manufacture a residual fallback.
    for transaction_id, source_observation in observations_by_transaction.items():
        if source_observation.direction != "outbound":
            continue
        if not any(
            _transaction_id(leg) == transaction_id
            and leg.get("role") == "source"
            for leg in legs.values()
        ):
            continue
        claimed = source_cursors.get(source_observation.quantity_hash, 0)
        if claimed != source_observation.principal_msat:
            raise _error(
                "component did not claim the complete observed source principal",
                component_id=component_id,
                transaction_id=transaction_id,
                claimed_msat=claimed,
                principal_msat=source_observation.principal_msat,
            )

    return ComponentClaimCompilation(
        component_id=component_id,
        claims=tuple(claims),
        suspense_msat=suspense_msat,
        reviewed_conversion_pairs=tuple(reviewed_conversion_pairs),
        reviewed_direct_payouts=tuple(reviewed_direct_payouts),
    )


__all__ = ["ComponentClaimCompilation", "compile_component_quantity_claims"]
