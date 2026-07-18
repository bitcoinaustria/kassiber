"""Runtime orchestration for canonical custody quantity.

This layer adapts stored transaction rows to the pure evidence/arbitration
modules. It remains independent of RP2: quantity projections drive custody
views and report readiness, while RP2 continues to own tax-basis calculation.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from typing import Any, Iterable, Mapping, Sequence

from ..errors import AppError
from .custody_component_claims import compile_component_quantity_claims
from .custody_evidence import (
    CanonicalEventIssue,
    CanonicalQuantityInput,
    QuantityObservation,
    build_canonical_quantity_input,
    enriched_quantity_rows,
    resolve_protocol_scope,
)
from .custody_gap_holds import CustodyGapHold, compile_gap_candidate_holds
from .custody_gap_reviews import candidate_fingerprint
from .custody_gaps import CustodyGapSearchResult
from .custody_native_audit import compile_verified_native_claims
from .custody_quantity import (
    CONFLICTING,
    CUSTODY_SUSPENSE,
    ClaimPriority,
    EXTERNAL_CONFIRMED,
    INTERNAL_REVIEWED,
    INTERNAL_VERIFIED,
    UNRESOLVED_STATES,
    ArbitratedSlice,
    QuantityDomain,
    QuantityClaim,
    QuantityProjection,
    QuantitySlice,
    project_quantities,
)


def _field(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    if type(row) is dict:
        return row.get(key, default)
    getter = getattr(row, "get", None)
    if getter is not None:
        return getter(key, default)
    try:
        return row[key]
    except (KeyError, IndexError):
        return default


@dataclass(frozen=True)
class QuantityIssue:
    issue_id: str
    issue_type: str
    state: str
    asset: str | None
    amount_msat: int | None
    occurred_at: str
    transaction_ids: tuple[str, ...]
    reason: str
    details: Mapping[str, Any]


@dataclass(frozen=True)
class QuantityTaxEligibility:
    """The exact decision slices a future tax-event compiler may consume."""

    eligible_decisions: tuple[ArbitratedSlice, ...]
    ineligible_slices: tuple[QuantitySlice, ...]
    blocked_from: str | None
    barrier_event_key: tuple[str, str, str, str, str] | None = None
    pool_barriers: tuple[
        tuple["TaxExposurePool", tuple[str, str, str, str, str]], ...
    ] = ()

    def barrier_for(
        self,
        observation: QuantityObservation,
    ) -> tuple[str, str, str, str, str] | None:
        """Return the basis barrier for this observation's exposure pool."""

        pool = TaxExposurePool.from_observation(observation)
        return next(
            (
                barrier
                for candidate, barrier in self.pool_barriers
                if candidate == pool
            ),
            None,
        )


@dataclass(frozen=True, order=True)
class TaxExposurePool:
    """One profile-local tax-basis continuity domain.

    Rail is intentionally omitted. A reviewed Bitcoin/Liquid/Lightning bridge
    carries the same Bitcoin basis across rails, while network and non-Bitcoin
    asset exposure remain isolated. Profile scope prevents one book's unknown
    history from freezing another book if a wider row set reaches the runtime.
    """

    profile_id: str
    network: str
    exposure: str
    unit: str

    @classmethod
    def from_observation(cls, observation: QuantityObservation) -> "TaxExposurePool":
        domain = QuantityDomain.from_observation(observation)
        return cls(
            profile_id=observation.profile_id,
            network=domain.network,
            exposure=domain.exposure,
            unit=domain.unit,
        )


@dataclass(frozen=True)
class CanonicalQuantityState:
    canonical_input: CanonicalQuantityInput
    projection: QuantityProjection
    issues: tuple[QuantityIssue, ...]
    tax_eligibility: QuantityTaxEligibility
    gap_candidate_transaction_ids: tuple[str, ...] = ()
    gap_holds: tuple[CustodyGapHold, ...] = ()
    reviewed_conversion_pairs: tuple[Mapping[str, Any], ...] = ()
    reviewed_direct_payouts: tuple[Mapping[str, Any], ...] = ()

    @property
    def report_blocked(self) -> bool:
        return bool(self.issues)


def canonical_internal_transfer_rows(
    state: CanonicalQuantityState,
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]] | None = None,
) -> tuple[dict[str, Any], ...]:
    """Return finalized custody edges independently of tax-basis readiness.

    An earlier suspense slice may correctly stop RP2 from projecting a later
    MOVE while the custody fact itself remains exact. Keeping both states on
    one row prevents UI, AI, CLI, and integration checks from mistaking a tax
    basis barrier for missing ownership evidence.
    """

    refs = wallet_refs_by_id or {}
    observations = {
        item.quantity_hash: item for item in state.projection.observations
    }
    eligible = set(state.tax_eligibility.eligible_decisions)
    rows: list[dict[str, Any]] = []
    for decision in state.projection.decisions:
        if (
            decision.state not in {INTERNAL_VERIFIED, INTERNAL_REVIEWED}
            or decision.target is None
        ):
            continue
        source = observations[decision.source.observation_hash]
        target = observations[decision.target.observation_hash]
        source_ref = refs.get(source.wallet_id, {})
        target_ref = refs.get(target.wallet_id, {})
        source_domain = QuantityDomain.from_observation(source)
        rows.append(
            {
                "out_transaction_id": source.anchor_transaction_id,
                "in_transaction_id": target.anchor_transaction_id,
                "occurred_at": source.occurred_at,
                "asset": source.asset,
                "amount_msat": decision.source.amount_msat,
                "from_wallet_id": source.wallet_id,
                "from_wallet": _field(source_ref, "label", source.wallet_id),
                "to_wallet_id": target.wallet_id,
                "to_wallet": _field(target_ref, "label", target.wallet_id),
                "custody_state": decision.state,
                "basis_state": (
                    "eligible"
                    if decision in eligible
                    else "blocked_by_prior_custody_basis"
                ),
                "evidence_reason": decision.reason,
                "network": source_domain.network,
                "rail": source_domain.rail,
                **(
                    {"atomic_bundle_id": decision.atomic_bundle_id}
                    if decision.atomic_bundle_id
                    else {}
                ),
                **(
                    {"component_id": decision.component_id}
                    if decision.component_id
                    else {}
                ),
            }
        )
    return tuple(
        sorted(
            rows,
            key=lambda item: (
                item["occurred_at"],
                item["out_transaction_id"],
                item["in_transaction_id"],
                item["amount_msat"],
            ),
        )
    )


def _issue_id(parts: Iterable[Any]) -> str:
    encoded = json.dumps(
        [str(value) for value in parts],
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _rejected_event_issue(
    issue: CanonicalEventIssue,
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> QuantityIssue:
    occurred_at = min(
        (
            str(_field(rows_by_id[tx_id], "occurred_at") or "")
            for tx_id in issue.transaction_ids
            if tx_id in rows_by_id
        ),
        default="",
    )
    return QuantityIssue(
        issue_id=_issue_id(("canonical", issue.event_key, issue.code)),
        issue_type="canonical_event_rejected",
        state=CONFLICTING,
        asset=None,
        amount_msat=None,
        occurred_at=occurred_at,
        transaction_ids=issue.transaction_ids,
        reason=issue.code,
        details={
            "message": issue.message,
            "event_scope": {
                "chain": issue.event_key.chain,
                "network": issue.event_key.network,
                "namespace": issue.event_key.native_namespace,
            },
            **dict(issue.details),
        },
    )


def _decision_issue(
    decision: ArbitratedSlice,
    observation: QuantityObservation,
) -> QuantityIssue:
    return QuantityIssue(
        issue_id=_issue_id(
            (
                "decision",
                decision.source.observation_hash,
                decision.source.start_msat,
                decision.source.end_msat,
                decision.state,
            )
        ),
        issue_type="unresolved_quantity",
        state=decision.state,
        asset=observation.asset,
        amount_msat=decision.source.amount_msat,
        occurred_at=observation.occurred_at,
        transaction_ids=(observation.transaction_id,),
        reason=decision.reason,
        details={
            "selected_claim_id": decision.selected_claim_id,
            "contender_claim_ids": list(decision.contender_claim_ids),
        },
    )


def _component_claims_and_issues(
    components: Sequence[Mapping[str, Any]],
    canonical: CanonicalQuantityInput,
    authored_evidence: Mapping[str, Sequence[Mapping[str, Any]]] | None = None,
) -> tuple[
    tuple[QuantityClaim, ...],
    tuple[QuantityIssue, ...],
    tuple[Mapping[str, Any], ...],
    tuple[Mapping[str, Any], ...],
]:
    observations_by_hash = {
        item.quantity_hash: item for item in canonical.observations
    }
    observations_by_transaction = {
        transaction_id: observations_by_hash[quantity_hash]
        for event in canonical.events
        for transaction_id, quantity_hash in event.observation_aliases
    }
    claims: list[QuantityClaim] = []
    issues: list[QuantityIssue] = []
    reviewed_conversion_pairs: list[Mapping[str, Any]] = []
    reviewed_direct_payouts: list[Mapping[str, Any]] = []
    for component in sorted(components, key=lambda item: str(item.get("id") or "")):
        component_id = str(component.get("id") or "")
        transaction_ids = tuple(
            sorted(
                {
                    str(
                        leg.get("anchor_transaction_id")
                        or leg.get("transaction_id")
                    )
                    for leg in component.get("legs", ())
                    if (
                        leg.get("anchor_transaction_id")
                        or leg.get("transaction_id")
                    )
                    not in (None, "")
                }
            )
        )
        if component.get("effective_state") != "active":
            evidence_status = component.get("evidence_status") or {}
            component_error = AppError(
                "authored active component is not locally effective",
                code="custody_component_authored_active_invalid",
                details={
                    "component_id": component_id,
                    "evidence_status": evidence_status.get("status"),
                },
            )
        else:
            # Materialization is the single evidence authority. It compares
            # the replicated, author-bound commitments with this replica's
            # canonical anchors before setting ``effective_state=active``.
            # Raw activation snapshots are author-local audit material and
            # intentionally do not replicate, so journal projection must not
            # introduce a second snapshot gate here.
            component_error = _component_evidence_drift(
                component,
                transaction_ids,
                canonical,
                authored_evidence,
            )
        try:
            if component_error is not None:
                raise component_error
            compiled = compile_component_quantity_claims(
                component,
                observations_by_transaction,
            )
        except (AppError, TypeError, ValueError) as exc:
            error_code = str(
                getattr(exc, "code", "custody_component_claim_compile")
            )
            source_observations = {
                observations_by_transaction[transaction_id].quantity_hash:
                observations_by_transaction[transaction_id]
                for transaction_id in transaction_ids
                if transaction_id in observations_by_transaction
                and observations_by_transaction[transaction_id].direction == "outbound"
            }
            claims.extend(
                QuantityClaim(
                    claim_id=(
                        f"component:{component_id}:compile-failed:"
                        f"{observation.quantity_hash}"
                    ),
                    source=QuantitySlice(
                        observation.quantity_hash,
                        0,
                        observation.principal_msat,
                    ),
                    state=CUSTODY_SUSPENSE,
                    priority=ClaimPriority.REVIEWED_COMPONENT,
                    reason=error_code,
                    atomic_bundle_id=f"component:{component_id}:compile-failed",
                )
                for observation in source_observations.values()
                if observation.principal_msat > 0
            )
            occurred_at = min(
                (item.occurred_at for item in source_observations.values()),
                default="",
            )
            details = getattr(exc, "details", None)
            issues.append(
                QuantityIssue(
                    issue_id=_issue_id(("component_compile", component_id)),
                    issue_type=(
                        "custody_component_evidence_drift"
                        if error_code == "custody_component_evidence_drift"
                        else "component_claim_compile_failed"
                    ),
                    state=CONFLICTING,
                    asset=(
                        next(iter(source_observations.values())).asset
                        if source_observations
                        else None
                    ),
                    # The exact source slices are represented immediately
                    # below by fail-closed suspense decisions.  Leaving this
                    # diagnostic issue unquantified avoids counting the same
                    # principal twice in readiness totals.
                    amount_msat=None,
                    occurred_at=occurred_at,
                    transaction_ids=transaction_ids,
                    reason=error_code,
                    details={
                        "component_id": component_id,
                        "message": str(exc),
                        "compiler_details": dict(details or {}),
                    },
                )
            )
        else:
            claims.extend(compiled.claims)
            reviewed_conversion_pairs.extend(compiled.reviewed_conversion_pairs)
            reviewed_direct_payouts.extend(compiled.reviewed_direct_payouts)
    return (
        tuple(claims),
        tuple(issues),
        tuple(reviewed_conversion_pairs),
        tuple(reviewed_direct_payouts),
    )


def _component_evidence_drift(
    component: Mapping[str, Any],
    transaction_ids: Sequence[str],
    canonical: CanonicalQuantityInput,
    authored_evidence: Mapping[str, Sequence[Mapping[str, Any]]] | None,
) -> AppError | None:
    """Apply the stronger author-local quantity/identity check when available.

    Replicated author commitments are validated while the component is
    materialized and control ``effective_state``. Raw snapshots additionally
    bind private fingerprint/raw-JSON detail on the authoring device. Those
    details remain immutable audit evidence, but observation lifecycle changes
    are not ownership contradictions: confirmation, timestamps, and raw graph
    enrichment may change while the committed physical quantity stays exact.
    Only a missing or changed quantity hash invalidates the component here.
    Retractions already fail closed through ``effective_state`` before this
    function is reached.
    """

    if authored_evidence is None:
        return None
    component_id = str(component.get("id") or "")
    stored = authored_evidence.get(component_id, ())
    if not stored:
        return None
    relevant_hashes = {
        quantity_hash
        for event in canonical.events
        for transaction_id, quantity_hash in event.observation_aliases
        if transaction_id in transaction_ids
    }
    current = {
        snapshot.quantity_hash
        for event in canonical.events
        for snapshot in event.evidence_snapshots
        if snapshot.quantity_hash in relevant_hashes
    }
    expected = {
        str(item.get("quantity_hash") or "")
        for item in stored
    }
    if current == expected:
        return None
    return AppError(
        "component activation quantity or identity no longer matches current observations",
        code="custody_component_evidence_drift",
        details={
            "component_id": component_id,
            "drift_kind": "evidence_quantity_changed_or_missing",
            "expected_snapshot_count": len(expected),
            "current_snapshot_count": len(current),
        },
    )


def _observations_by_transaction(
    canonical: CanonicalQuantityInput,
) -> dict[str, QuantityObservation]:
    observations_by_hash = {
        item.quantity_hash: item for item in canonical.observations
    }
    return {
        transaction_id: observations_by_hash[quantity_hash]
        for event in canonical.events
        for transaction_id, quantity_hash in event.observation_aliases
    }


def _gap_candidate_holds_and_issues(
    canonical: CanonicalQuantityInput,
    *,
    ignored_transaction_ids: Iterable[str],
    dismissed_fingerprints: Mapping[str, str],
    search_result: CustodyGapSearchResult,
) -> tuple[
    tuple[QuantityClaim, ...],
    tuple[QuantityIssue, ...],
    tuple[str, ...],
    tuple[CustodyGapHold, ...],
]:
    """Compile structured suggestions into independent, non-lineage holds."""

    ignored = tuple(sorted({str(item) for item in ignored_transaction_ids if item}))
    capacity_blocking_source_ids: set[str] = set()
    candidates = search_result.accounting_candidates
    if not search_result.search_complete:
        # Candidate search is advisory. Capacity says only that suggestions are
        # incomplete; it is not itself evidence about any physical quantity.
        # A population ceiling can, however, be reached *after* structured
        # candidates were completely scored. Preserve those candidates for
        # canonical accounting even though the UI queue remains bounded.
        capacity_blocking_source_ids.update(search_result.blocking_source_ids)
        if not candidates and not capacity_blocking_source_ids:
            return (), (), (), ()

    # The shared projection intentionally retains unmatched/suspense
    # boundaries for review presentation. Existing interpreter blockers still
    # own those exact accounting slices, so do not add a competing gap hold.
    ignored_set = set(ignored)
    candidates = tuple(
        candidate
        for candidate in candidates
        if not ignored_set.intersection(
            (*candidate.source_ids, *candidate.return_ids)
        )
    )
    capacity_blocking_source_ids.difference_update(ignored_set)

    observations = _observations_by_transaction(canonical)
    claims: list[QuantityClaim] = []
    issues: list[QuantityIssue] = []
    holds: list[CustodyGapHold] = []
    candidate_transaction_ids: set[str] = set()
    # Capacity cannot prove a transfer, but it also must not silently finalize
    # a typed privacy/Samourai boundary as an external disposal. Hold only the
    # affected source principal in suspense; do not select an arbitrary sampled
    # return. Dismissing one sampled relationship cannot classify the source as
    # external while omitted candidates remain possible.
    for transaction_id in sorted(capacity_blocking_source_ids):
        observation = observations.get(transaction_id)
        if (
            observation is None
            or observation.direction != "outbound"
            or observation.principal_msat <= 0
        ):
            continue
        candidate_transaction_ids.add(transaction_id)
        quantity = QuantitySlice(
            observation.quantity_hash,
            0,
            observation.principal_msat,
        )
        hold = CustodyGapHold(
            hold_id="gap-capacity-hold:" + observation.quantity_hash,
            gap_id="capacity-incomplete",
            transaction_id=observation.transaction_id,
            direction="outbound",
            quantity=quantity,
            evidence_detail_hash=observation.evidence_detail_hash,
        )
        holds.append(hold)
        claims.append(
            _source_hold_claim(hold, "custody_gap_search_incomplete_structured_source")
        )
    for candidate in candidates:
        if not candidate.promotion_eligible:
            continue
        if dismissed_fingerprints.get(candidate.gap_id) == candidate_fingerprint(
            candidate
        ):
            continue
        # Promotion eligibility itself is enough to hold both boundaries out
        # of RP2. A compiler disagreement is a blocker, not permission to book
        # those raw rows as an unrelated disposal and acquisition.
        candidate_transaction_ids.update(candidate.source_ids)
        candidate_transaction_ids.update(candidate.return_ids)
        try:
            compiled = compile_gap_candidate_holds(candidate, observations)
        except (TypeError, ValueError) as exc:
            error_code = str(
                getattr(exc, "code", "custody_gap_hold_compile")
            )
            source_observations = {
                observations[transaction_id].quantity_hash:
                observations[transaction_id]
                for transaction_id in candidate.source_ids
                if transaction_id in observations
                and observations[transaction_id].direction == "outbound"
            }
            bundle_id = f"candidate:{candidate.gap_id}:compile-failed"
            claims.extend(
                QuantityClaim(
                    claim_id=(
                        f"{bundle_id}:{observation.quantity_hash}"
                    ),
                    source=QuantitySlice(
                        observation.quantity_hash,
                        0,
                        observation.principal_msat,
                    ),
                    state=CUSTODY_SUSPENSE,
                    priority=ClaimPriority.ACCOUNTING_CONVENTION,
                    reason=error_code,
                    atomic_bundle_id=bundle_id,
                )
                for observation in source_observations.values()
                if observation.principal_msat > 0
            )
            issues.append(
                QuantityIssue(
                    issue_id=_issue_id(("gap_compile", candidate.gap_id)),
                    issue_type="custody_gap_hold_compile_failed",
                    state=CONFLICTING,
                    asset=candidate.asset,
                    # Exact source principal is represented by the suspense
                    # decisions above. Keep this compiler diagnostic
                    # unquantified so readiness totals count it only once.
                    amount_msat=None,
                    occurred_at=candidate.started_at,
                    transaction_ids=tuple(
                        sorted((*candidate.source_ids, *candidate.return_ids))
                    ),
                    reason=error_code,
                    details={
                        "gap_id": candidate.gap_id,
                        "message": str(exc),
                        "compiler_details": dict(
                            getattr(exc, "details", None) or {}
                        ),
                    },
                )
            )
        else:
            holds.extend(compiled.holds)
            issues.append(
                QuantityIssue(
                    issue_id=_issue_id(("custody_gap_hold", candidate.gap_id)),
                    issue_type="custody_gap_review_hold",
                    state=CUSTODY_SUSPENSE,
                    asset=candidate.asset,
                    # Source decisions quantify suspense exactly. This issue
                    # scopes both boundaries and is not counted a second time.
                    amount_msat=None,
                    occurred_at=candidate.started_at,
                    transaction_ids=tuple(
                        sorted((*candidate.source_ids, *candidate.return_ids))
                    ),
                    reason="custody_gap_review_required",
                    details={
                        "gap_id": candidate.gap_id,
                        "retained_msat": candidate.retained_msat,
                        "residual_msat": candidate.residual_msat,
                        "excess_msat": candidate.excess_msat,
                        "held_transaction_ids": [
                            hold.transaction_id for hold in compiled.holds
                        ],
                    },
                )
            )

    # Conflicting candidates can name the same source. One source hold is
    # enough to suppress its presumed-external default; candidate identity
    # remains on typed issues and independent return holds.
    source_holds: dict[str, CustodyGapHold] = {}
    for hold in holds:
        if hold.direction == "outbound":
            source_holds.setdefault(hold.quantity.observation_hash, hold)
    claimed_slices = {claim.source for claim in claims}
    claims.extend(
        _source_hold_claim(hold, "custody_gap_review_required")
        for hold in source_holds.values()
        if hold.quantity not in claimed_slices
    )
    return (
        tuple(claims),
        tuple(issues),
        tuple(sorted(candidate_transaction_ids)),
        tuple(sorted(holds, key=lambda item: item.hold_id)),
    )


def _source_hold_claim(hold: CustodyGapHold, reason: str) -> QuantityClaim:
    """Adapt a source hold to suspense arbitration without creating an edge."""

    if hold.direction != "outbound":
        raise ValueError("only outbound holds participate in source arbitration")
    return QuantityClaim(
        claim_id=hold.hold_id,
        source=hold.quantity,
        state=CUSTODY_SUSPENSE,
        priority=ClaimPriority.ACCOUNTING_CONVENTION,
        reason=reason,
    )


def _tax_eligibility(
    canonical: CanonicalQuantityInput,
    projection: QuantityProjection,
    issues: Sequence[QuantityIssue],
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> QuantityTaxEligibility:
    """Apply deterministic event-order barriers per tax exposure pool.

    A timestamp alone is not an order: two final legs of the same physical
    event must remain usable when a sibling slice is suspense.  Canonical event
    identity supplies that tie-breaker. The first unresolved event blocks only
    later events in the same profile/network/exposure pool, never finalized
    siblings of the issue event or unrelated assets and profiles.
    """

    event_order_by_hash: dict[str, tuple[str, str, str, str, str]] = {}
    transaction_to_hash: dict[str, str] = {}
    pool_by_hash: dict[str, TaxExposurePool] = {}
    for event in canonical.events:
        event_order = (
            min((leg.occurred_at for leg in event.legs), default=""),
            event.event_key.chain,
            event.event_key.network,
            event.event_key.native_namespace,
            event.event_key.native_event_id,
        )
        for transaction_id, quantity_hash in event.observation_aliases:
            transaction_to_hash[transaction_id] = quantity_hash
        for leg in event.legs:
            event_order_by_hash[leg.quantity_hash] = event_order
            pool_by_hash[leg.quantity_hash] = TaxExposurePool.from_observation(leg)
    barriers_by_pool: dict[
        TaxExposurePool, tuple[str, str, str, str, str]
    ] = {}
    directly_blocked_hashes: set[str] = set()

    def row_pool(row: Mapping[str, Any]) -> TaxExposurePool | None:
        """Resolve a rejected row's pool without accepting its event identity."""

        try:
            # A temporary observation is unnecessary and would require valid
            # quantity semantics. Resolve the same domain fields directly.
            scope = resolve_protocol_scope(row)
            asset = str(_field(row, "asset") or "").upper()
            if not asset:
                return None
            network = {
                "liquidv1": "main",
                "liquidtestnet": "test",
                "elementsregtest": "regtest",
            }.get(scope.network, scope.network)
            exposure = "bitcoin" if asset in {"BTC", "LBTC"} else f"asset:{asset}"
            return TaxExposurePool(
                profile_id=str(_field(row, "profile_id") or ""),
                network=network,
                exposure=exposure,
                unit="msat",
            )
        except (TypeError, ValueError):
            return None

    def register(
        pool: TaxExposurePool,
        barrier: tuple[str, str, str, str, str],
    ) -> None:
        current = barriers_by_pool.get(pool)
        if current is None or barrier < current:
            barriers_by_pool[pool] = barrier

    for issue in issues:
        if issue.issue_type in {
            "native_audit_evidence_invalid",
            "custody_gap_review_hold",
            "custody_gap_hold_compile_failed",
        }:
            directly_blocked_hashes.update(
                transaction_to_hash[transaction_id]
                for transaction_id in issue.transaction_ids
                if transaction_id in transaction_to_hash
            )
        known = [
            (
                pool_by_hash[transaction_to_hash[transaction_id]],
                event_order_by_hash[transaction_to_hash[transaction_id]],
            )
            for transaction_id in issue.transaction_ids
            if transaction_id in transaction_to_hash
            and transaction_to_hash[transaction_id] in event_order_by_hash
        ]
        if known:
            for pool in {pool for pool, _barrier in known}:
                register(
                    pool,
                    min(barrier for candidate, barrier in known if candidate == pool),
                )
            continue
        if not issue.occurred_at:
            continue
        synthetic = (issue.occurred_at, "", "", "", "")
        rejected_pools = {
            pool
            for transaction_id in issue.transaction_ids
            if transaction_id in rows_by_id
            for pool in (row_pool(rows_by_id[transaction_id]),)
            if pool is not None
        }
        if not rejected_pools and issue.asset:
            rejected_pools = {
                pool_by_hash[item.quantity_hash]
                for item in projection.observations
                if item.asset == issue.asset
            }
        if not rejected_pools:
            # A truly unscoped canonical contradiction is the exceptional
            # fail-closed case: its exposure is unknowable, so every observed
            # pool in this already profile-scoped build is affected.
            rejected_pools = set(pool_by_hash.values())
        for pool in rejected_pools:
            register(pool, synthetic)

    barrier = min(barriers_by_pool.values()) if barriers_by_pool else None
    blocked_from = barrier[0] if barrier is not None else None

    def eligible_decision(item: ArbitratedSlice) -> bool:
        if not item.finalized:
            return False
        # A presumed-external default at the first barrier event is normally useful:
        # an unresolved sibling slice must not erase unrelated conclusions from
        # the same physical transaction. It is not useful when the issue names
        # this observation itself. In that case (for example a failed native
        # ownership proof) projecting that default would book the exact
        # quantity whose custody evidence failed.
        if item.source.observation_hash in directly_blocked_hashes:
            return False
        item_barrier = barriers_by_pool.get(
            pool_by_hash[item.source.observation_hash]
        )
        if item_barrier is None:
            return True
        order = event_order_by_hash[item.source.observation_hash]
        # Only the timestamp is temporal evidence. The remaining tuple fields
        # make replay deterministic, but using them to order distinct events at
        # the same instant would grant basis eligibility by arbitrary event id.
        # Keep the barrier event's finalized sibling slices, and fail closed for
        # every other event sharing that timestamp.
        return order[0] < item_barrier[0] or order == item_barrier

    eligible = tuple(
        item
        for item in projection.decisions
        if eligible_decision(item)
    )
    eligible_slices = {item.source for item in eligible}
    ineligible = tuple(
        sorted(
            item.source
            for item in projection.decisions
            if item.source not in eligible_slices
        )
    )
    return QuantityTaxEligibility(
        eligible,
        ineligible,
        blocked_from,
        barrier,
        tuple(sorted(barriers_by_pool.items())),
    )


def build_canonical_quantity_state(
    rows: Sequence[Mapping[str, Any]],
    *,
    gap_search_result: CustodyGapSearchResult,
    canonical_input: CanonicalQuantityInput | None = None,
    interpreter_claims: Iterable[QuantityClaim] = (),
    effective_components: Sequence[Mapping[str, Any]] = (),
    native_evidence: Sequence[Mapping[str, Any]] = (),
    interpreter_blockers: Sequence[Mapping[str, Any]] = (),
    ignored_gap_transaction_ids: Iterable[str] = (),
    component_evidence_snapshots: (
        Mapping[str, Sequence[Mapping[str, Any]]] | None
    ) = None,
    dismissed_gap_fingerprints: Mapping[str, str] | None = None,
) -> CanonicalQuantityState:
    """Build the canonical quantity projection and tax-eligibility boundary."""

    interpreter_claims = tuple(interpreter_claims)
    if canonical_input is None:
        safe_rows = enriched_quantity_rows(rows)
        canonical = build_canonical_quantity_input(safe_rows)
    else:
        # The core journal builder already enriched these exact rows to create
        # this immutable input. Reuse both together instead of hashing and
        # validating every observation a second time.
        safe_rows = tuple(rows)
        canonical = canonical_input
    (
        component_claims,
        component_issues,
        reviewed_conversion_pairs,
        reviewed_direct_payouts,
    ) = (
        _component_claims_and_issues(
            effective_components,
            canonical,
            component_evidence_snapshots,
        )
    )
    component_transaction_ids = {
        str(leg.get("anchor_transaction_id") or leg.get("transaction_id"))
        for component in effective_components
        for leg in component.get("legs", ())
        if (leg.get("anchor_transaction_id") or leg.get("transaction_id"))
        not in (None, "")
    }
    ignored_transaction_ids = set(component_transaction_ids)
    ignored_transaction_ids.update(
        str(item) for item in ignored_gap_transaction_ids if item
    )
    observations_by_hash = {
        item.quantity_hash: item for item in canonical.observations
    }
    for claim in interpreter_claims:
        source = observations_by_hash.get(claim.source.observation_hash)
        if source is not None:
            ignored_transaction_ids.add(source.transaction_id)
        if claim.target is not None:
            target = observations_by_hash.get(claim.target.observation_hash)
            if target is not None:
                ignored_transaction_ids.add(target.transaction_id)
    gap_hold_claims, gap_issues, gap_candidate_transaction_ids, gap_holds = (
        _gap_candidate_holds_and_issues(
            canonical,
            ignored_transaction_ids=ignored_transaction_ids,
            dismissed_fingerprints=dismissed_gap_fingerprints or {},
            search_result=gap_search_result,
        )
    )
    native_audit = compile_verified_native_claims(
        canonical,
        native_evidence,
        component_transaction_ids=component_transaction_ids,
        reserved_source_msat={},
    )
    canonical = native_audit.canonical_input
    claims = (
        *component_claims,
        *gap_hold_claims,
        *native_audit.claims,
        *interpreter_claims,
    )
    projection = project_quantities(canonical.observations, claims)
    rows_by_id = {
        str(_field(row, "id") or ""): row
        for row in safe_rows
        if _field(row, "id") not in (None, "")
    }
    by_hash = {item.quantity_hash: item for item in projection.observations}
    issues = [
        _rejected_event_issue(issue, rows_by_id)
        for issue in canonical.rejected_events
    ]
    issues.extend(component_issues)
    issues.extend(gap_issues)
    issues.extend(
        QuantityIssue(
            issue_id=item.issue_id,
            issue_type="native_audit_evidence_invalid",
            state=CONFLICTING,
            asset=item.asset,
            amount_msat=item.amount_msat,
            occurred_at=item.occurred_at,
            transaction_ids=item.transaction_ids,
            reason=item.reason,
            details=dict(item.details),
        )
        for item in native_audit.issues
    )
    for error in projection.claim_errors:
        involved_observations = [
            by_hash[quantity_hash]
            for quantity_hash in error.involved_observation_hashes
            if quantity_hash in by_hash
        ]
        assets = {item.asset for item in involved_observations}
        issues.append(
            QuantityIssue(
                issue_id=_issue_id(("claim_bundle_invalid", error.bundle_id)),
                issue_type="quantity_claim_bundle_invalid",
                state=CONFLICTING,
                asset=next(iter(assets)) if len(assets) == 1 else None,
                amount_msat=None,
                occurred_at=min(
                    (item.occurred_at for item in involved_observations),
                    default="",
                ),
                transaction_ids=tuple(
                    sorted(
                        {
                            item.anchor_transaction_id
                            for item in involved_observations
                        }
                    )
                ),
                reason="malformed_claim_bundle",
                details={
                    "bundle_id": error.bundle_id,
                    "claim_ids": list(error.claim_ids),
                    "validation_reasons": list(error.reasons),
                },
            )
        )
    for ordinal, blocker in enumerate(interpreter_blockers):
        transaction_id = str(_field(blocker, "transaction_id") or "")
        row = rows_by_id.get(transaction_id, {})
        reason = str(
            _field(blocker, "reason") or "custody_interpreter_blocked"
        )
        raw_detail = _field(blocker, "detail_json")
        try:
            details = json.loads(raw_detail or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            details = {"unparsed_detail": str(raw_detail or "")}
        issues.append(
            QuantityIssue(
                issue_id=_issue_id(
                    ("custody_interpreter_blocked", transaction_id, reason, ordinal)
                ),
                issue_type="custody_interpreter_blocked",
                state=CONFLICTING,
                asset=str(_field(row, "asset") or "").upper() or None,
                amount_msat=(
                    (
                        int(_field(row, "amount") or 0)
                        if int(_field(row, "amount") or 0) > 0
                        else None
                    )
                    if row
                    else None
                ),
                occurred_at=str(_field(row, "occurred_at") or ""),
                transaction_ids=(transaction_id,) if transaction_id else (),
                reason=reason,
                details=(details if isinstance(details, Mapping) else {}),
            )
        )
    issues.extend(
        _decision_issue(decision, by_hash[decision.source.observation_hash])
        for decision in projection.decisions
        if decision.state in UNRESOLVED_STATES
    )
    ordered_issues = tuple(
        sorted(issues, key=lambda item: (item.occurred_at, item.issue_id))
    )
    return CanonicalQuantityState(
        canonical_input=canonical,
        projection=projection,
        issues=ordered_issues,
        tax_eligibility=_tax_eligibility(
            canonical,
            projection,
            ordered_issues,
            rows_by_id,
        ),
        gap_candidate_transaction_ids=gap_candidate_transaction_ids,
        gap_holds=gap_holds,
        reviewed_conversion_pairs=reviewed_conversion_pairs,
        reviewed_direct_payouts=reviewed_direct_payouts,
    )


@dataclass(frozen=True)
class WalletBalanceDifference:
    wallet_id: str
    asset: str
    canonical_msat: int
    current_msat: int
    reason: str | None


def compare_wallet_balances(
    state: CanonicalQuantityState,
    current_balances: Mapping[tuple[str, str], int],
    *,
    known_non_event_reasons: Mapping[str, str] | None = None,
) -> tuple[WalletBalanceDifference, ...]:
    """Explain canonical/RP2 differences without treating RP2 as authoritative."""

    known_non_event_reasons = dict(known_non_event_reasons or {})
    observations = {item.quantity_hash: item for item in state.projection.observations}
    canonical_balances: dict[tuple[str, str], int] = {}
    reasons: dict[tuple[str, str], set[str]] = {}
    for posting in state.projection.postings:
        if posting.location_kind != "wallet":
            continue
        key = (posting.location_id, posting.asset)
        canonical_balances[key] = (
            canonical_balances.get(key, 0) + posting.amount_msat
        )
        if posting.observation_hash in observations:
            transaction_id = observations[posting.observation_hash].transaction_id
            reason = known_non_event_reasons.get(transaction_id)
            if reason:
                reasons.setdefault(key, set()).add(reason)
    differences = []
    for wallet_id, asset in sorted(set(canonical_balances) | set(current_balances)):
        canonical_amount = canonical_balances.get((wallet_id, asset), 0)
        current_amount = int(current_balances.get((wallet_id, asset), 0))
        if canonical_amount == current_amount:
            continue
        named = sorted(reasons.get((wallet_id, asset), ()))
        differences.append(
            WalletBalanceDifference(
                wallet_id=wallet_id,
                asset=asset,
                canonical_msat=canonical_amount,
                current_msat=current_amount,
                reason=named[0] if len(named) == 1 else None,
            )
        )
    return tuple(differences)


__all__ = [
    "QuantityIssue",
    "QuantityTaxEligibility",
    "TaxExposurePool",
    "CanonicalQuantityState",
    "WalletBalanceDifference",
    "build_canonical_quantity_state",
    "compare_wallet_balances",
    "enriched_quantity_rows",
]
