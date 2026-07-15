"""Exact custody-quantity arbitration independent of tax booking.

The tax engine answers what a finalized economic event means for lots.  This
module answers the earlier, country-neutral question: where did each exact
observed quantity go?  It deliberately has no SQLite or RP2 dependency.

Imported transaction rows become content-addressed observations.  Interpreters
may propose claims over half-open msat slices of an outbound observation.  One
arbiter selects the strongest claim for every slice, fails closed on equal-rank
overlap, prevents two sources from consuming the same inbound slice, and fills
every unclaimed residual with custody suspense.

The resulting postings preserve every observed wallet debit and credit even
when no tax classification is final.  Their per-asset sum is always zero:

    observed wallets + external/origin + fees + suspense/conflict == 0

This is the Gate-1 contract.  It is intentionally not wired into journals yet;
the integration must replace RP2-derived quantity views rather than decorate
or filter RP2 input rows.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from enum import IntEnum
from typing import Iterable, Mapping, Sequence

from .custody_evidence import (
    CanonicalEventIssue,
    CanonicalEventKey,
    CanonicalQuantityEvent,
    CanonicalQuantityInput,
    EvidenceSnapshot,
    QuantityObservation,
    build_canonical_quantity_input,
    canonical_event_key,
    canonical_evidence_payload,
    canonical_quantity_payload,
    observation_hash,
)


INTERNAL_VERIFIED = "internal_verified"
INTERNAL_REVIEWED = "internal_reviewed"
EXTERNAL_CONFIRMED = "external_confirmed"
EXTERNAL_PRESUMED = "external_presumed"
CUSTODY_CANDIDATE = "custody_candidate"
CUSTODY_SUSPENSE = "custody_suspense"
CONFLICTING = "conflicting"

CLAIM_STATES = frozenset(
    {
        INTERNAL_VERIFIED,
        INTERNAL_REVIEWED,
        EXTERNAL_CONFIRMED,
        EXTERNAL_PRESUMED,
        CUSTODY_CANDIDATE,
        CUSTODY_SUSPENSE,
    }
)
EXTERNAL_ECONOMIC_SUBTYPES = frozenset(
    {"payment", "disposal", "gift", "lost"}
)
TARGET_STATES = frozenset(
    {INTERNAL_VERIFIED, INTERNAL_REVIEWED, CUSTODY_CANDIDATE}
)
UNRESOLVED_STATES = frozenset(
    {CUSTODY_CANDIDATE, CUSTODY_SUSPENSE, CONFLICTING}
)
FINALIZED_STATES = frozenset(
    {
        INTERNAL_VERIFIED,
        INTERNAL_REVIEWED,
        EXTERNAL_CONFIRMED,
        EXTERNAL_PRESUMED,
    }
)


@dataclass(frozen=True, order=True)
class QuantityDomain:
    """The physical exposure a quantity claim is allowed to conserve.

    ``asset`` is intentionally not the domain.  BTC, LBTC, and a Lightning
    balance are all msat-denominated Bitcoin exposure, but only a reviewed
    cross-rail claim may bridge their rails.  Everything else remains scoped to
    its protocol network and asset exposure; a label/amount coincidence can
    never carry quantity between chains.
    """

    network: str
    exposure: str
    unit: str
    rail: str

    @classmethod
    def from_observation(cls, observation: QuantityObservation) -> "QuantityDomain":
        rail = observation.event_key.chain
        network = observation.event_key.network
        # Liquid's wire network names differ from Bitcoin's owner-domain names.
        # This only normalizes the explicitly reviewed BTC/LBTC bridge; it does
        # not make generic Liquid claims interchangeable with Bitcoin claims.
        network = {
            "liquidv1": "main",
            "liquidtestnet": "test",
            "elementsregtest": "regtest",
        }.get(network, network)
        exposure = "bitcoin" if observation.asset in {"BTC", "LBTC"} else (
            f"asset:{observation.asset}"
        )
        return cls(network=network, exposure=exposure, unit="msat", rail=rail)

    def compatible_with(self, other: "QuantityDomain", *, allow_cross_rail: bool) -> bool:
        if (self.network, self.exposure, self.unit) != (
            other.network,
            other.exposure,
            other.unit,
        ):
            return False
        return self.rail == other.rail or allow_cross_rail


class ClaimPriority(IntEnum):
    """Evidence order; lower numbers are stronger."""

    REVIEWED_COMPONENT = 10
    EXACT_NATIVE_EVENT = 20
    REVIEWED_PAIR = 30
    ACCOUNTING_CONVENTION = 40
    HEURISTIC_CANDIDATE = 50
    PRESUMED_EXTERNAL_FALLBACK = 60


STATE_PRIORITIES = {
    INTERNAL_VERIFIED: frozenset({ClaimPriority.EXACT_NATIVE_EVENT}),
    INTERNAL_REVIEWED: frozenset(
        {ClaimPriority.REVIEWED_COMPONENT, ClaimPriority.REVIEWED_PAIR}
    ),
    EXTERNAL_CONFIRMED: frozenset(
        {
            ClaimPriority.REVIEWED_COMPONENT,
            ClaimPriority.EXACT_NATIVE_EVENT,
            ClaimPriority.REVIEWED_PAIR,
        }
    ),
    EXTERNAL_PRESUMED: frozenset(
        {ClaimPriority.PRESUMED_EXTERNAL_FALLBACK}
    ),
    CUSTODY_CANDIDATE: frozenset({ClaimPriority.HEURISTIC_CANDIDATE}),
    CUSTODY_SUSPENSE: frozenset(
        {
            ClaimPriority.REVIEWED_COMPONENT,
            ClaimPriority.EXACT_NATIVE_EVENT,
            ClaimPriority.REVIEWED_PAIR,
            ClaimPriority.ACCOUNTING_CONVENTION,
        }
    ),
}


@dataclass(frozen=True, order=True)
class QuantitySlice:
    """A deterministic bookkeeping slice, not a claim about physical sat order."""

    observation_hash: str
    start_msat: int
    end_msat: int

    def __post_init__(self) -> None:
        if not self.observation_hash:
            raise ValueError("quantity slices require an observation hash")
        if (
            type(self.start_msat) is not int
            or type(self.end_msat) is not int
            or self.start_msat < 0
            or self.end_msat <= self.start_msat
        ):
            raise ValueError("quantity slices require a non-empty half-open msat range")

    @property
    def amount_msat(self) -> int:
        return self.end_msat - self.start_msat


@dataclass(frozen=True)
class QuantityClaim:
    claim_id: str
    source: QuantitySlice
    state: str
    priority: ClaimPriority
    reason: str
    target: QuantitySlice | None = None
    supporting_evidence_hashes: tuple[str, ...] = ()
    fallback: bool = False
    atomic_bundle_id: str | None = None
    destination_kind: str | None = None
    # Country-neutral economic meaning attached to an exact reviewed external
    # sink.  Custody finality does not imply that the jurisdiction knows how to
    # tax a gift or loss, but projection must never degrade either into a sale.
    external_economic_subtype: str | None = None
    transfer_kind: str | None = None
    transfer_policy: str | None = None
    component_id: str | None = None
    # A BTC/LBTC/Lightning bridge must be deliberately authored by an
    # interpreter that has reviewed/native rail evidence.  Generic claims stay
    # rail-local even where the numerical amount happens to match.
    allow_cross_rail: bool = False
    # Reviewed evidence is not silently priority-overridden.  A new claim may
    # replace another active interpretation only by naming it explicitly.
    supersedes_claim_ids: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        if not self.claim_id or not self.reason:
            raise ValueError("quantity claims require claim_id and reason")
        if not isinstance(self.priority, ClaimPriority):
            raise ValueError("quantity claims require a supported evidence priority")
        if self.state not in CLAIM_STATES:
            raise ValueError(f"unsupported quantity claim state: {self.state}")
        if self.priority not in STATE_PRIORITIES[self.state]:
            raise ValueError(
                f"{self.state} cannot use {self.priority.name.lower()} priority"
            )
        targetless_reviewed_retained = (
            self.state == INTERNAL_REVIEWED
            and self.priority == ClaimPriority.REVIEWED_COMPONENT
            and self.target is None
            and self.destination_kind == "retained_custody"
        )
        if (
            self.state in TARGET_STATES
            and self.target is None
            and not targetless_reviewed_retained
        ):
            raise ValueError(f"{self.state} claims require an observed target slice")
        if self.state not in TARGET_STATES and self.target is not None:
            raise ValueError(f"{self.state} claims cannot consume a target slice")
        if self.target is not None and self.target.amount_msat != self.source.amount_msat:
            raise ValueError("source and target quantity slices must conserve exact msat")
        if self.fallback and self.priority != ClaimPriority.PRESUMED_EXTERNAL_FALLBACK:
            raise ValueError("fallback claims must use presumed-external fallback priority")
        if self.state == EXTERNAL_PRESUMED and not self.fallback:
            raise ValueError("external_presumed is only valid as an explicit fallback")
        if self.atomic_bundle_id is not None and not self.atomic_bundle_id.strip():
            raise ValueError("atomic_bundle_id cannot be empty")
        if self.atomic_bundle_id is not None and self.fallback:
            raise ValueError("presumed-external fallbacks cannot join atomic bundles")
        if self.destination_kind not in {
            None,
            "external",
            "fee",
            "retained_custody",
        }:
            raise ValueError("quantity claim destination_kind is unsupported")
        if self.destination_kind in {"external", "fee"} and (
            self.state != EXTERNAL_CONFIRMED or self.target is not None
        ):
            raise ValueError(
                "only targetless external_confirmed claims may classify an "
                "external destination"
            )
        if self.destination_kind == "retained_custody" and not (
            self.state == INTERNAL_REVIEWED
            and self.priority == ClaimPriority.REVIEWED_COMPONENT
            and self.target is None
        ):
            raise ValueError(
                "retained_custody requires a targetless reviewed component claim"
            )
        if self.external_economic_subtype not in (
            {None} | EXTERNAL_ECONOMIC_SUBTYPES
        ):
            raise ValueError("quantity claim external economic subtype is unsupported")
        if self.external_economic_subtype is not None and not (
            self.state == EXTERNAL_CONFIRMED
            and self.priority == ClaimPriority.REVIEWED_COMPONENT
            and self.target is None
            and self.destination_kind == "external"
        ):
            raise ValueError(
                "external economic subtype requires a reviewed external component claim"
            )
        if self.allow_cross_rail and self.target is None:
            raise ValueError("only target claims can bridge custody rails")
        if any(not item for item in self.supersedes_claim_ids):
            raise ValueError("superseded claim ids cannot be empty")

    @property
    def effective_bundle_id(self) -> str | None:
        if self.atomic_bundle_id is not None:
            return self.atomic_bundle_id
        if self.priority in {
            ClaimPriority.REVIEWED_COMPONENT,
            ClaimPriority.REVIEWED_PAIR,
        }:
            return f"single:{self.claim_id}"
        return None


@dataclass(frozen=True)
class ArbitratedSlice:
    source: QuantitySlice
    state: str
    reason: str
    selected_claim_id: str | None = None
    atomic_bundle_id: str | None = None
    transfer_kind: str | None = None
    transfer_policy: str | None = None
    component_id: str | None = None
    target: QuantitySlice | None = None
    destination_kind: str | None = None
    external_economic_subtype: str | None = None
    contender_claim_ids: tuple[str, ...] = ()

    @property
    def finalized(self) -> bool:
        """Whether this exact slice may feed economic/tax projection."""

        return self.state in FINALIZED_STATES


@dataclass(frozen=True)
class QuantityPosting:
    posting_id: str
    asset: str
    location_kind: str
    location_id: str
    amount_msat: int
    state: str
    observation_hash: str | None = None
    domain: QuantityDomain | None = None


@dataclass(frozen=True)
class QuantityProjection:
    observations: tuple[QuantityObservation, ...]
    decisions: tuple[ArbitratedSlice, ...]
    postings: tuple[QuantityPosting, ...]
    claim_errors: tuple["QuantityClaimError", ...] = ()

    def totals_by_asset(self) -> dict[str, int]:
        totals: dict[str, int] = {}
        for posting in self.postings:
            totals[posting.asset] = totals.get(posting.asset, 0) + posting.amount_msat
        return totals

    def totals_by_domain(self) -> dict[tuple[str, str, str], int]:
        """Return conservation totals independent of custody rail.

        Rail remains part of claim compatibility, but an explicitly authorized
        BTC/LBTC/Lightning bridge balances the same Bitcoin exposure across
        different rails.
        """

        totals: dict[tuple[str, str, str], int] = {}
        for posting in self.postings:
            if posting.domain is None:
                continue
            key = (
                posting.domain.network,
                posting.domain.exposure,
                posting.domain.unit,
            )
            totals[key] = totals.get(key, 0) + posting.amount_msat
        return totals

    def unresolved_msat_by_asset(self) -> dict[str, int]:
        totals: dict[str, int] = {}
        by_hash = {item.quantity_hash: item for item in self.observations}
        for decision in self.decisions:
            if decision.state in UNRESOLVED_STATES:
                observation = by_hash[decision.source.observation_hash]
                totals[observation.asset] = (
                    totals.get(observation.asset, 0) + decision.source.amount_msat
                )
        return totals


@dataclass(frozen=True)
class QuantityClaimError:
    """One rejected claim bundle, isolated from independent arbitration."""

    bundle_id: str
    reasons: tuple[str, ...]
    claim_ids: tuple[str, ...]
    source_observation_hashes: tuple[str, ...]


def _validated_inputs(
    observations: Sequence[QuantityObservation],
    claims: Iterable[QuantityClaim],
) -> tuple[
    dict[str, QuantityObservation],
    tuple[QuantityClaim, ...],
    tuple[QuantityClaimError, ...],
]:
    by_hash: dict[str, QuantityObservation] = {}
    for observation in observations:
        observation._validate()
        if observation.quantity_hash in by_hash:
            raise ValueError(
                f"duplicate quantity observation hash: {observation.quantity_hash}"
            )
        by_hash[observation.quantity_hash] = observation

    normalized_claims = tuple(sorted(claims, key=lambda item: (item.priority, item.claim_id)))
    claim_id_counts: dict[str, int] = {}
    bundles: dict[str, list[QuantityClaim]] = {}
    for claim in normalized_claims:
        claim_id_counts[claim.claim_id] = claim_id_counts.get(claim.claim_id, 0) + 1
        bundle_id = claim.effective_bundle_id or f"claim:{claim.claim_id}"
        bundles.setdefault(bundle_id, []).append(claim)

    valid_claims: list[QuantityClaim] = []
    errors: list[QuantityClaimError] = []
    invalid_sources: dict[str, ClaimPriority] = {}
    suspense_priorities = STATE_PRIORITIES[CUSTODY_SUSPENSE]
    for bundle_id, members in sorted(bundles.items()):
        reasons: set[str] = set()
        priorities = {claim.priority for claim in members}
        if len(priorities) != 1:
            reasons.add("atomic_bundle_priority_mismatch")
        if any(claim_id_counts[claim.claim_id] > 1 for claim in members):
            reasons.add("duplicate_claim_id")
        source_hashes: set[str] = set()
        for claim in members:
            source = by_hash.get(claim.source.observation_hash)
            if source is None or source.direction != "outbound":
                reasons.add("claim_source_invalid")
            else:
                source_hashes.add(source.quantity_hash)
                if claim.source.end_msat > source.principal_msat:
                    reasons.add("claim_source_exceeds_principal")
            if claim.target is None:
                continue
            target = by_hash.get(claim.target.observation_hash)
            if target is None or target.direction != "inbound":
                reasons.add("claim_target_invalid")
                continue
            if claim.target.end_msat > target.principal_msat:
                reasons.add("claim_target_exceeds_principal")
            if source is not None and source.direction == "outbound":
                source_domain = QuantityDomain.from_observation(source)
                target_domain = QuantityDomain.from_observation(target)
                if not source_domain.compatible_with(
                    target_domain, allow_cross_rail=claim.allow_cross_rail
                ):
                    reasons.add("claim_domain_incompatible")
        if not reasons:
            valid_claims.extend(members)
            continue
        errors.append(
            QuantityClaimError(
                bundle_id=bundle_id,
                reasons=tuple(sorted(reasons)),
                claim_ids=tuple(sorted({claim.claim_id for claim in members})),
                source_observation_hashes=tuple(sorted(source_hashes)),
            )
        )
        eligible_priorities = [
            claim.priority
            for claim in members
            if claim.priority in suspense_priorities
        ]
        priority = (
            min(eligible_priorities)
            if eligible_priorities
            else ClaimPriority.ACCOUNTING_CONVENTION
        )
        for source_hash in source_hashes:
            prior = invalid_sources.get(source_hash)
            if prior is None or priority < prior:
                invalid_sources[source_hash] = priority

    for source_hash, priority in sorted(invalid_sources.items()):
        source = by_hash[source_hash]
        valid_claims.append(
            QuantityClaim(
                claim_id=f"malformed-claim-bundle:{source_hash}",
                source=QuantitySlice(source_hash, 0, source.principal_msat),
                state=CUSTODY_SUSPENSE,
                priority=priority,
                reason="malformed_claim_bundle",
                supporting_evidence_hashes=(source.evidence_detail_hash,),
            )
        )
    return (
        by_hash,
        tuple(sorted(valid_claims, key=lambda item: (item.priority, item.claim_id))),
        tuple(errors),
    )


def _source_decisions(
    by_hash: Mapping[str, QuantityObservation],
    claims: Sequence[QuantityClaim],
) -> list[ArbitratedSlice]:
    claims_by_source: dict[str, list[QuantityClaim]] = {}
    for claim in claims:
        claims_by_source.setdefault(claim.source.observation_hash, []).append(claim)

    decisions: list[ArbitratedSlice] = []
    for source in sorted(
        (item for item in by_hash.values() if item.direction == "outbound"),
        key=lambda item: (item.occurred_at, item.quantity_hash),
    ):
        source_claims = claims_by_source.get(source.quantity_hash, [])
        # The whole-row fallback remains available where a positive internal or
        # external classification leaves a source slice unclaimed. Boundaries
        # below split it around those stronger claims. A candidate or suspense
        # claim is different: it positively says this source has unresolved
        # custody history, so its uncovered remainder must stay suspense rather
        # than silently reverting to presumed disposal.
        if any(
            not claim.fallback
            and claim.state in {CUSTODY_CANDIDATE, CUSTODY_SUSPENSE}
            for claim in source_claims
        ):
            source_claims = [claim for claim in source_claims if not claim.fallback]
        boundaries = {0, source.principal_msat}
        for claim in source_claims:
            boundaries.update((claim.source.start_msat, claim.source.end_msat))
        ordered = sorted(boundaries)
        for start, end in zip(ordered, ordered[1:]):
            if start == end:
                continue
            segment = QuantitySlice(source.quantity_hash, start, end)
            contenders = [
                claim
                for claim in source_claims
                if claim.source.start_msat <= start and claim.source.end_msat >= end
            ]
            if not contenders:
                decisions.append(
                    ArbitratedSlice(
                        source=segment,
                        state=CUSTODY_SUSPENSE,
                        reason="unclaimed_source_residual",
                    )
                )
                continue
            # Any active reviewed/verified interpretations of the same source
            # slice must agree.  Evidence priority is an ordering aid, not a
            # permission to overwrite a different reviewed conclusion.  Exact
            # semantic duplicates coalesce; a newer interpretation has to name
            # the prior claim in ``supersedes_claim_ids``.
            authoritative = [
                claim
                for claim in contenders
                if claim.priority <= ClaimPriority.REVIEWED_PAIR
            ]
            if authoritative:
                def semantic_key(claim: QuantityClaim) -> tuple[object, ...]:
                    target = None
                    if claim.target is not None:
                        offset = start - claim.source.start_msat
                        target = (
                            claim.target.observation_hash,
                            claim.target.start_msat + offset,
                            claim.target.start_msat + offset + (end - start),
                        )
                    return (
                        claim.state,
                        target,
                        claim.destination_kind,
                        claim.external_economic_subtype,
                        claim.transfer_kind,
                        claim.transfer_policy,
                        claim.component_id,
                    )

                by_semantics: dict[tuple[object, ...], list[QuantityClaim]] = {}
                for claim in authoritative:
                    by_semantics.setdefault(semantic_key(claim), []).append(claim)
                if len(by_semantics) > 1:
                    active_ids = {claim.claim_id for claim in authoritative}
                    superseders = [
                        claim
                        for claim in authoritative
                        if active_ids - {claim.claim_id}
                        <= set(claim.supersedes_claim_ids)
                    ]
                    if len(superseders) != 1:
                        decisions.append(
                            ArbitratedSlice(
                                source=segment,
                                state=CONFLICTING,
                                reason="incompatible_reviewed_claims",
                                contender_claim_ids=tuple(sorted(active_ids)),
                            )
                        )
                        continue
                    contenders = [superseders[0]]
            strongest = min(claim.priority for claim in contenders)
            winners = [claim for claim in contenders if claim.priority == strongest]
            if len(winners) != 1:
                decisions.append(
                    ArbitratedSlice(
                        source=segment,
                        state=CONFLICTING,
                        reason="equal_priority_source_overlap",
                        contender_claim_ids=tuple(
                            sorted(claim.claim_id for claim in winners)
                        ),
                    )
                )
                continue
            winner = winners[0]
            target = None
            if winner.target is not None:
                offset = start - winner.source.start_msat
                target = QuantitySlice(
                    winner.target.observation_hash,
                    winner.target.start_msat + offset,
                    winner.target.start_msat + offset + segment.amount_msat,
                )
            decisions.append(
                ArbitratedSlice(
                    source=segment,
                    state=winner.state,
                    reason=winner.reason,
                    selected_claim_id=winner.claim_id,
                    atomic_bundle_id=winner.effective_bundle_id,
                    transfer_kind=winner.transfer_kind,
                    transfer_policy=winner.transfer_policy,
                    component_id=winner.component_id,
                    target=target,
                    destination_kind=winner.destination_kind,
                    external_economic_subtype=winner.external_economic_subtype,
                )
            )
    return decisions


def _fail_closed_destination_overlaps(
    decisions: Sequence[ArbitratedSlice],
) -> list[ArbitratedSlice]:
    conflicts: dict[int, set[str]] = {}

    def mark_cluster(cluster: Sequence[tuple[int, ArbitratedSlice]]) -> None:
        if len(cluster) < 2:
            return
        contender_ids = {
            item.selected_claim_id
            for _, item in cluster
            if item.selected_claim_id
        }
        for index, _ in cluster:
            conflicts.setdefault(index, set()).update(contender_ids)

    def target_range(indexed: tuple[int, ArbitratedSlice]) -> tuple[int, int]:
        target = indexed[1].target
        assert target is not None
        return target.start_msat, target.end_msat

    by_target: dict[str, list[tuple[int, ArbitratedSlice]]] = {}
    for index, decision in enumerate(decisions):
        if decision.target is not None:
            by_target.setdefault(decision.target.observation_hash, []).append(
                (index, decision)
            )
    for targeted in by_target.values():
        ordered = sorted(targeted, key=target_range)
        cluster: list[tuple[int, ArbitratedSlice]] = []
        cluster_end = -1
        for indexed in ordered:
            target = indexed[1].target
            assert target is not None
            if cluster and target.start_msat >= cluster_end:
                mark_cluster(cluster)
                cluster = []
            cluster.append(indexed)
            cluster_end = max(cluster_end if len(cluster) > 1 else -1, target.end_msat)
        mark_cluster(cluster)
    result: list[ArbitratedSlice] = []
    for index, decision in enumerate(decisions):
        contender_ids = conflicts.get(index)
        if contender_ids:
            result.append(
                replace(
                    decision,
                    state=CONFLICTING,
                    reason="destination_slice_claimed_twice",
                    selected_claim_id=None,
                    target=None,
                    destination_kind=None,
                    external_economic_subtype=None,
                    contender_claim_ids=tuple(sorted(contender_ids)),
                )
            )
        else:
            result.append(decision)
    return result


def _claim_fully_selected(
    claim: QuantityClaim,
    decisions: Sequence[ArbitratedSlice],
) -> bool:
    selected = [
        decision
        for decision in decisions
        if decision.selected_claim_id == claim.claim_id
    ]
    if sum(item.source.amount_msat for item in selected) != claim.source.amount_msat:
        return False
    if claim.target is None:
        return all(item.target is None for item in selected)
    return sum(
        item.target.amount_msat for item in selected if item.target is not None
    ) == claim.target.amount_msat


def _fail_closed_atomic_bundles(
    decisions: Sequence[ArbitratedSlice],
    claims: Sequence[QuantityClaim],
) -> list[ArbitratedSlice]:
    bundles: dict[str, list[QuantityClaim]] = {}
    for claim in claims:
        if claim.effective_bundle_id is not None:
            bundles.setdefault(claim.effective_bundle_id, []).append(claim)
    invalid = {
        bundle_id
        for bundle_id, members in bundles.items()
        if not all(_claim_fully_selected(member, decisions) for member in members)
    }
    if not invalid:
        return list(decisions)

    claim_to_bundle = {
        claim.claim_id: claim.effective_bundle_id
        for claim in claims
        if claim.effective_bundle_id in invalid
    }
    bundle_contenders = {
        bundle_id: tuple(sorted(member.claim_id for member in bundles[bundle_id]))
        for bundle_id in invalid
    }
    result: list[ArbitratedSlice] = []
    for decision in decisions:
        bundle_id = claim_to_bundle.get(decision.selected_claim_id or "")
        if bundle_id is None:
            result.append(decision)
            continue
        result.append(
            replace(
                decision,
                state=CONFLICTING,
                reason="atomic_bundle_incomplete",
                selected_claim_id=None,
                target=None,
                destination_kind=None,
                external_economic_subtype=None,
                contender_claim_ids=tuple(
                    sorted(
                        set(decision.contender_claim_ids)
                        | set(bundle_contenders[bundle_id])
                    )
                ),
            )
        )
    return result


def _uncovered_inbound_slices(
    observation: QuantityObservation,
    decisions: Sequence[ArbitratedSlice],
) -> list[QuantitySlice]:
    consumed = sorted(
        (
            decision.target
            for decision in decisions
            if decision.target is not None
            and decision.target.observation_hash == observation.quantity_hash
        ),
        key=lambda item: (item.start_msat, item.end_msat),
    )
    uncovered: list[QuantitySlice] = []
    cursor = 0
    for item in consumed:
        if item.start_msat > cursor:
            uncovered.append(
                QuantitySlice(observation.quantity_hash, cursor, item.start_msat)
            )
        cursor = item.end_msat
    if cursor < observation.principal_msat:
        uncovered.append(
            QuantitySlice(
                observation.quantity_hash,
                cursor,
                observation.principal_msat,
            )
        )
    return uncovered


def _build_postings(
    observations: Sequence[QuantityObservation],
    decisions: Sequence[ArbitratedSlice],
) -> tuple[QuantityPosting, ...]:
    by_hash = {item.quantity_hash: item for item in observations}
    postings: list[QuantityPosting] = []
    for observation in sorted(observations, key=lambda item: item.quantity_hash):
        postings.append(
            QuantityPosting(
                posting_id=f"observed:{observation.quantity_hash}",
                asset=observation.asset,
                location_kind="wallet",
                location_id=observation.wallet_id,
                amount_msat=observation.wallet_delta_msat,
                state="observed",
                observation_hash=observation.quantity_hash,
                domain=QuantityDomain.from_observation(observation),
            )
        )
        if observation.direction == "outbound" and observation.fee_msat:
            postings.append(
                QuantityPosting(
                    posting_id=f"fee:{observation.quantity_hash}",
                    asset=observation.asset,
                    location_kind="fee",
                    location_id="network_fee",
                    amount_msat=observation.fee_msat,
                    state=EXTERNAL_CONFIRMED,
                    observation_hash=observation.quantity_hash,
                    domain=QuantityDomain.from_observation(observation),
                )
            )

    for decision in decisions:
        if decision.target is not None:
            continue
        source = by_hash[decision.source.observation_hash]
        location_kind = decision.destination_kind or {
            EXTERNAL_CONFIRMED: "external",
            EXTERNAL_PRESUMED: "external",
            CUSTODY_CANDIDATE: "custody_candidate",
            CUSTODY_SUSPENSE: "custody_suspense",
            CONFLICTING: "conflicting",
        }.get(decision.state)
        if location_kind is None:
            raise ValueError(
                f"internal quantity decision has no observed target: {decision.state}"
            )
        postings.append(
            QuantityPosting(
                posting_id=(
                    f"decision:{decision.source.observation_hash}:"
                    f"{decision.source.start_msat}:{decision.source.end_msat}"
                ),
                asset=source.asset,
                location_kind=location_kind,
                location_id=decision.selected_claim_id or decision.reason,
                amount_msat=decision.source.amount_msat,
                state=decision.state,
                observation_hash=source.quantity_hash,
                domain=QuantityDomain.from_observation(source),
            )
        )

    for observation in observations:
        if observation.direction != "inbound":
            continue
        for item in _uncovered_inbound_slices(observation, decisions):
            postings.append(
                QuantityPosting(
                    posting_id=(
                        f"origin:{item.observation_hash}:"
                        f"{item.start_msat}:{item.end_msat}"
                    ),
                    asset=observation.asset,
                    location_kind="external_origin",
                    location_id="unclassified_origin",
                    amount_msat=-item.amount_msat,
                    state="unclassified_origin",
                    observation_hash=observation.quantity_hash,
                    domain=QuantityDomain.from_observation(observation),
                )
            )
    return tuple(sorted(postings, key=lambda item: item.posting_id))


def project_quantities(
    observations: Sequence[QuantityObservation],
    claims: Iterable[QuantityClaim],
) -> QuantityProjection:
    """Arbitrate exact slices and return a balanced quantity artifact."""

    by_hash, normalized_claims, claim_errors = _validated_inputs(
        observations, claims
    )
    decisions = _fail_closed_atomic_bundles(
        _fail_closed_destination_overlaps(
            _source_decisions(by_hash, normalized_claims)
        ),
        normalized_claims,
    )
    postings = _build_postings(tuple(by_hash.values()), decisions)
    projection = QuantityProjection(
        observations=tuple(sorted(by_hash.values(), key=lambda item: item.quantity_hash)),
        decisions=tuple(
            sorted(
                decisions,
                key=lambda item: (
                    item.source.observation_hash,
                    item.source.start_msat,
                    item.source.end_msat,
                ),
            )
        ),
        postings=postings,
        claim_errors=claim_errors,
    )
    unbalanced = {
        domain: amount
        for domain, amount in projection.totals_by_domain().items()
        if amount != 0
    }
    if unbalanced:
        raise AssertionError(f"custody quantity projection is unbalanced: {unbalanced}")
    return projection


__all__ = [
    "CLAIM_STATES",
    "CanonicalEventIssue",
    "CanonicalEventKey",
    "CanonicalQuantityEvent",
    "CanonicalQuantityInput",
    "CONFLICTING",
    "CUSTODY_CANDIDATE",
    "CUSTODY_SUSPENSE",
    "ClaimPriority",
    "EXTERNAL_CONFIRMED",
    "EXTERNAL_ECONOMIC_SUBTYPES",
    "EXTERNAL_PRESUMED",
    "EvidenceSnapshot",
    "FINALIZED_STATES",
    "INTERNAL_REVIEWED",
    "INTERNAL_VERIFIED",
    "QuantityClaim",
    "QuantityClaimError",
    "QuantityDomain",
    "QuantityObservation",
    "QuantityPosting",
    "QuantityProjection",
    "QuantitySlice",
    "TARGET_STATES",
    "UNRESOLVED_STATES",
    "build_canonical_quantity_input",
    "canonical_event_key",
    "canonical_evidence_payload",
    "canonical_quantity_payload",
    "observation_hash",
    "project_quantities",
]
