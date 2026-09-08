"""Prove when native wallet history can replace a reviewed shortcut in memory.

This verifies the reviewed allocations against already compiled native claims;
it does not discover transfers or infer ownership. Wallet-internal carry uses
the shared exact allocation convention. Every intermediate debit, receipt and
fee must be accounted for before the authored shortcut can yield to that route.
"""

from __future__ import annotations

from bisect import bisect_left, bisect_right
from collections import defaultdict
from dataclasses import dataclass
import hashlib
from itertools import groupby
import json
from typing import Any, Mapping, Sequence

from ..errors import AppError
from .custody_allocations import allocate_msat_fifo
from .custody_component_claims import compile_component_quantity_claims
from .custody_evidence import QuantityObservation
from .custody_quantity import CUSTODY_SUSPENSE, ClaimPriority, INTERNAL_VERIFIED, QuantityClaim
from .transfer_chronology import chain_order_evidence, order_same_time_transfers


def _anchor(leg: Mapping[str, Any]) -> str:
    return str(leg.get("anchor_transaction_id") or leg.get("transaction_id") or "")


def _pool(observation: QuantityObservation) -> tuple[str, str, str, str]:
    return (observation.wallet_id, observation.asset, observation.event_key.chain, observation.event_key.network)


@dataclass(frozen=True)
class ReconciledAllocation:
    source_transaction_id: str
    target_transaction_id: str | None
    asset: str
    amount_msat: int
    destination_kind: str


@dataclass(frozen=True)
class NativeComponentReconciliation:
    """A derived certificate, stored with the journal rather than authorship."""

    component_id: str
    component_revision: int
    native_claim_ids: tuple[str, ...]
    native_claim_amounts: tuple[tuple[str, int], ...]
    transaction_ids: tuple[str, ...]
    reviewed_allocation_digest: str
    allocations: tuple[ReconciledAllocation, ...]


@dataclass(frozen=True)
class _NativeTransferOrder:
    out_row: Mapping[str, Any]
    from_wallet_id: str
    to_wallet_id: str


def _ordered_native_sources(
    source_hashes: set[str],
    observations: Mapping[str, QuantityObservation],
    outgoing: Mapping[str, Sequence[QuantityClaim]],
    rows_by_transaction: Mapping[str, Mapping[str, Any]],
) -> tuple[str, ...]:
    """Use the same physical-before-wallet chronology as finalized MOVE rows."""

    stable = sorted(source_hashes, key=lambda key: (
        observations[key].occurred_at, observations[key].event_key.native_event_id, key,
    ))
    result: list[str] = []
    for _at, group in groupby(stable, key=lambda key: observations[key].occurred_at):
        sources = tuple(group)
        if len(sources) == 1:
            result.extend(sources)
            continue
        transfers = {}
        source_by_claim = {}
        for source_hash in sources:
            source = observations[source_hash]
            proof = chain_order_evidence(rows_by_transaction.get(source.anchor_transaction_id, {}))
            for claim in outgoing[source_hash]:
                target = observations[claim.target.observation_hash]
                transfers[claim.claim_id] = _NativeTransferOrder(
                    {"custody_chain_order": proof}, source.wallet_id, target.wallet_id,
                )
                source_by_claim[claim.claim_id] = source_hash
        ordered = order_same_time_transfers(
            [(index, ("transfer", claim_id)) for index, claim_id in enumerate(transfers)],
            transfers,
        )
        result.extend(dict.fromkeys(source_by_claim[claim_id] for _kind, claim_id in ordered))
    return tuple(result)


def reconcile_native_components(
    components: Sequence[Mapping[str, Any]],
    observations: Mapping[str, QuantityObservation],
    native_claims: Sequence[QuantityClaim],
    *,
    blocked_transaction_ids: set[str],
    rows_by_transaction: Mapping[str, Mapping[str, Any]],
) -> tuple[NativeComponentReconciliation, ...]:
    """Select only complete native routes reproducing every reviewed allocation.

    The return value is a projection choice, never an authored state change.
    Partial routes, competing component anchors, non-native edges, unexplained
    wallet activity and any fee/allocation mismatch leave the shortcut intact.
    """

    by_hash = {item.quantity_hash: item for item in observations.values()}
    protected = {_anchor(leg) for item in components for leg in item.get("legs", ())} - {""}
    outgoing: dict[str, list[QuantityClaim]] = defaultdict(list)
    incoming: dict[str, list[QuantityClaim]] = defaultdict(list)
    claim_amounts: dict[str, int] = {}
    for claim in native_claims:
        if claim.target is None or claim.priority != ClaimPriority.EXACT_NATIVE_EVENT or claim.state != INTERNAL_VERIFIED:
            continue
        source = by_hash[claim.source.observation_hash]
        target = by_hash[claim.target.observation_hash]
        if (
            not source.authoritative_chain_observation
            or not target.authoritative_chain_observation
            or source.event_key != target.event_key
            or source.event_key.native_namespace != "chain"
            or source.event_key.chain not in {"bitcoin", "liquid"}
            or source.asset != target.asset
            or source.wallet_id == target.wallet_id
            or {source.transaction_id, target.transaction_id} & blocked_transaction_ids
        ):
            continue
        outgoing[source.quantity_hash].append(claim)
        incoming[target.quantity_hash].append(claim)
        claim_amounts[claim.claim_id] = claim.source.amount_msat

    def complete(claims: Sequence[QuantityClaim], amount: int, *, target: bool = False) -> bool:
        cursor = 0
        slices = sorted((item.target if target else item.source for item in claims), key=lambda item: item.start_msat)
        for item in slices:
            if item.start_msat != cursor:
                return False
            cursor = item.end_msat
        return cursor == amount

    # A partially inferred outgoing row could still contain a real external
    # payment; it cannot be used as a complete intermediate step.
    complete_sources = {
        key for key, claims in outgoing.items()
        if complete(claims, by_hash[key].principal_msat)
        and by_hash[key].fee_attribution == "exact"
        and by_hash[key].fee_msat >= 0
        and all(complete(incoming[item.target.observation_hash], by_hash[item.target.observation_hash].principal_msat, target=True) for item in claims)
    }
    if not complete_sources:
        return ()
    ordered_sources = _ordered_native_sources(complete_sources, by_hash, outgoing, rows_by_transaction)
    source_times = tuple(by_hash[key].occurred_at for key in ordered_sources)
    activity_by_pool: dict[tuple[str, str, str, str], list[QuantityObservation]] = defaultdict(list)
    for item in by_hash.values():
        if item.principal_msat or item.fee_msat:
            activity_by_pool[_pool(item)].append(item)
    for items in activity_by_pool.values():
        items.sort(key=lambda item: item.occurred_at)
    selected: list[NativeComponentReconciliation] = []
    selected_rows: set[str] = set()
    for component in components:
        if (
            component.get("effective_state") != "active"
            or component.get("conservation_mode") != "quantity"
            or component.get("economic_terms")
        ):
            continue
        anchors = {_anchor(leg) for leg in component.get("legs", ())} - {""}
        if anchors & blocked_transaction_ids or anchors & selected_rows:
            continue
        try:
            reviewed = compile_component_quantity_claims(
                component, {key: observations[key] for key in anchors if key in observations},
            )
        except (AppError, TypeError, ValueError):
            continue
        expected: dict[tuple[str, str], int] = defaultdict(int)
        source_hashes: set[str] = set()
        target_hashes: set[str] = set()
        supported = True
        for claim in reviewed.claims:
            source_hashes.add(claim.source.observation_hash)
            if claim.target is not None:
                target_hashes.add(claim.target.observation_hash)
                sink = claim.target.observation_hash
            elif claim.destination_kind == "fee" and claim.reason == "reviewed_network_fee":
                sink = "fee"
            elif claim.state == CUSTODY_SUSPENSE and claim.reason == "reviewed_residual_suspense":
                # An unresolved reviewed remainder can be discharged by exact
                # native fees, never by an amount-difference estimate. The
                # route below must actually consume this source's quantity in
                # observer-authoritative, exactly attributed network fees.
                sink = "fee"
            else:
                supported = False
                break
            expected[(claim.source.observation_hash, sink)] += claim.source.amount_msat
        if not supported or not source_hashes or not target_hashes or not source_hashes <= complete_sources:
            continue
        for source_hash in source_hashes:
            # Plain source-anchor fees are projected independently, while
            # reviewed residual fees above consume source principal.
            if by_hash[source_hash].fee_msat:
                expected[(source_hash, "fee")] += by_hash[source_hash].fee_msat
        start = min(by_hash[key].occurred_at for key in source_hashes)
        end = max(by_hash[key].occurred_at for key in target_hashes)
        pools: dict[tuple[str, str, str, str], dict[str, int]] = defaultdict(dict)
        actual: dict[tuple[str, str], int] = defaultdict(int)
        used: set[str] = set()
        used_claims: set[str] = set()
        seeded: set[str] = set()
        intermediate_pools: set[tuple[str, str, str, str]] = set()
        failed = False
        # Same-time ordering above remains intact; disjoint reviewed intervals
        # need not visit the rest of the book's native history for every route.
        for index in range(bisect_left(source_times, start), bisect_right(source_times, end)):
            source_hash = ordered_sources[index]
            source = by_hash[source_hash]
            pool_key = _pool(source)
            credits = pools[pool_key]
            if source_hash in source_hashes:
                # Source observations must be fully covered by this component.
                expected_debit = sum(amount for (key, _sink), amount in expected.items() if key == source_hash)
                if expected_debit != source.principal_msat + source.fee_msat:
                    failed = True
                    break
                credits[source_hash] = credits.get(source_hash, 0) + expected_debit
                seeded.add(source_hash)
            elif not credits:
                continue
            elif source.transaction_id in protected and source.transaction_id not in anchors:
                failed = True
                break
            claims = sorted(outgoing[source_hash], key=lambda claim: claim.source.start_msat)
            sinks = [(str(i), claim.source.amount_msat) for i, claim in enumerate(claims)]
            if source.fee_msat:
                sinks.append(("fee", source.fee_msat))
            if sum(credits.values()) < source.principal_msat + source.fee_msat:
                failed = True
                break
            allocated = allocate_msat_fifo(tuple(credits.items()), sinks)
            pools[pool_key] = {key: amount for key, amount in allocated.source_remaining if amount}
            used.add(source.transaction_id)
            used_claims.update(claim.claim_id for claim in claims)
            for cell in allocated.cells:
                if cell.sink_id == "fee":
                    actual[(cell.source_id, "fee")] += cell.amount_msat
                    continue
                target = by_hash[claims[int(cell.sink_id)].target.observation_hash]
                used.add(target.transaction_id)
                if (
                    target.transaction_id in selected_rows
                    or (target.transaction_id in protected and target.transaction_id not in anchors)
                ):
                    failed = True
                    break
                if target.quantity_hash in target_hashes:
                    actual[(cell.source_id, target.quantity_hash)] += cell.amount_msat
                else:
                    target_pool = _pool(target)
                    intermediate_pools.add(target_pool)
                    balances = pools[target_pool]
                    balances[cell.source_id] = balances.get(cell.source_id, 0) + cell.amount_msat
            if failed:
                break
        if failed or seeded != source_hashes or actual != expected or any(pools.values()) or not used - anchors:
            continue
        # Do not infer a route through a wallet containing additional observed
        # activity inside this interval. Those quantities need their own review
        # or a future finer-grained physical-outpoint proof.
        for pool_key in intermediate_pools:
            activity = activity_by_pool[pool_key]
            lower = bisect_left(activity, start, key=lambda item: item.occurred_at)
            upper = bisect_right(activity, end, key=lambda item: item.occurred_at)
            if any(activity[index].transaction_id not in used for index in range(lower, upper)):
                failed = True
                break
        if failed:
            continue
        digest = hashlib.sha256(json.dumps(
            [[source, target, amount] for (source, target), amount in sorted(expected.items())],
            separators=(",", ":"),
        ).encode()).hexdigest()
        selected.append(NativeComponentReconciliation(
            component_id=str(component["id"]),
            component_revision=int(component["revision"]),
            native_claim_ids=tuple(sorted(used_claims)),
            native_claim_amounts=tuple((key, claim_amounts[key]) for key in sorted(used_claims)),
            transaction_ids=tuple(sorted(used)),
            reviewed_allocation_digest=digest,
            allocations=tuple(
                ReconciledAllocation(
                    source_transaction_id=by_hash[source].anchor_transaction_id,
                    target_transaction_id=(by_hash[target].anchor_transaction_id if target != "fee" else None),
                    asset=by_hash[source].asset,
                    amount_msat=amount,
                    destination_kind="fee" if target == "fee" else "wallet",
                )
                for (source, target), amount in sorted(expected.items())
            ),
        ))
        selected_rows.update(used)
    return tuple(sorted(selected, key=lambda item: item.component_id))
