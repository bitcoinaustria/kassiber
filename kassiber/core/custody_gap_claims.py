"""Compile promoted custody-gap suggestions into exact arbiter claims.

The matcher remains advisory.  This pure bridge accepts only a promotion-
eligible candidate, validates it against canonical quantity observations, and
emits one atomic heuristic bundle.  It never activates or persists a bridge.

Half-open slices are deterministic accounting coordinates, not assertions
about physical sat lineage.  Network fees remain observation facts and never
become candidate source slices.  Any source principal not covered by the
candidate is intentionally left unclaimed: the quantity arbiter then suppresses
the presumed-external fallback and places that residual in custody suspense.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Mapping, Sequence

from .custody_allocations import CustodyAllocationError, allocate_msat_fifo
from .custody_gaps import CustodyGapCandidate, custody_gap_id
from .custody_quantity import (
    CUSTODY_CANDIDATE,
    ClaimPriority,
    QuantityClaim,
    QuantityObservation,
    QuantitySlice,
)


class CustodyGapClaimCompileError(ValueError):
    """A candidate and its canonical observations do not agree atomically."""

    def __init__(self, message: str, *, details: Mapping[str, object] | None = None):
        super().__init__(message)
        self.code = "custody_gap_claim_compile"
        self.details = dict(details or {})


@dataclass(frozen=True)
class GapCandidateClaimCompilation:
    gap_id: str
    atomic_bundle_id: str | None
    claims: tuple[QuantityClaim, ...]
    retained_msat: int
    residual_msat: int
    excess_msat: int


def compile_gap_candidate_claims(
    candidate: CustodyGapCandidate,
    observations_by_transaction: Mapping[str, QuantityObservation],
) -> GapCandidateClaimCompilation:
    """Return one exact heuristic bundle, or no claims for a search-only hint.

    Validation completes before the first claim is returned.  Missing,
    contradictory, cross-asset, cross-chain, or cross-network observations
    therefore fail locally rather than producing a partial bundle.
    """

    if not isinstance(candidate, CustodyGapCandidate):
        raise CustodyGapClaimCompileError("candidate has an unsupported type")
    if not candidate.promotion_eligible:
        return GapCandidateClaimCompilation(
            gap_id=candidate.gap_id,
            atomic_bundle_id=None,
            claims=(),
            retained_msat=0,
            residual_msat=0,
            excess_msat=0,
        )

    sources = _candidate_observations(
        candidate.source_ids,
        observations_by_transaction,
        direction="outbound",
        candidate=candidate,
    )
    returns = _candidate_observations(
        candidate.return_ids,
        observations_by_transaction,
        direction="inbound",
        candidate=candidate,
    )
    _validate_candidate(candidate, sources, returns)

    bundle_id = f"candidate:{candidate.gap_id}"
    claims = _allocate_claims(
        candidate,
        sources,
        returns,
        bundle_id=bundle_id,
    )
    if sum(claim.source.amount_msat for claim in claims) != candidate.retained_msat:
        raise CustodyGapClaimCompileError(
            "candidate allocation did not cover its exact retained principal",
            details={"gap_id": candidate.gap_id},
        )
    return GapCandidateClaimCompilation(
        gap_id=candidate.gap_id,
        atomic_bundle_id=bundle_id,
        claims=claims,
        retained_msat=candidate.retained_msat,
        residual_msat=candidate.residual_msat,
        excess_msat=candidate.excess_msat,
    )


def _candidate_observations(
    transaction_ids: Sequence[str],
    observations_by_transaction: Mapping[str, QuantityObservation],
    *,
    direction: str,
    candidate: CustodyGapCandidate,
) -> tuple[QuantityObservation, ...]:
    if not transaction_ids or tuple(transaction_ids) != tuple(sorted(transaction_ids)):
        raise CustodyGapClaimCompileError(
            "candidate transaction ids must be non-empty and canonically ordered",
            details={"gap_id": candidate.gap_id, "direction": direction},
        )
    if len(set(transaction_ids)) != len(transaction_ids):
        raise CustodyGapClaimCompileError(
            "candidate transaction ids must be unique",
            details={"gap_id": candidate.gap_id, "direction": direction},
        )
    observations: list[QuantityObservation] = []
    for transaction_id in transaction_ids:
        observation = observations_by_transaction.get(transaction_id)
        if observation is None:
            raise CustodyGapClaimCompileError(
                "candidate references a missing canonical observation",
                details={
                    "gap_id": candidate.gap_id,
                    "transaction_id": transaction_id,
                },
            )
        try:
            observation._validate()
        except ValueError as exc:
            raise CustodyGapClaimCompileError(
                "candidate references an invalid canonical observation",
                details={
                    "gap_id": candidate.gap_id,
                    "transaction_id": transaction_id,
                    "reason": str(exc),
                },
            ) from exc
        if observation.transaction_id != transaction_id:
            raise CustodyGapClaimCompileError(
                "observation mapping key does not match transaction identity",
                details={
                    "gap_id": candidate.gap_id,
                    "transaction_id": transaction_id,
                    "observation_transaction_id": observation.transaction_id,
                },
            )
        if observation.direction != direction:
            raise CustodyGapClaimCompileError(
                "candidate observation has the wrong direction",
                details={
                    "gap_id": candidate.gap_id,
                    "transaction_id": transaction_id,
                    "expected": direction,
                    "actual": observation.direction,
                },
            )
        observations.append(observation)
    return tuple(observations)


def _validate_candidate(
    candidate: CustodyGapCandidate,
    sources: Sequence[QuantityObservation],
    returns: Sequence[QuantityObservation],
) -> None:
    if set(candidate.source_ids) & set(candidate.return_ids):
        raise CustodyGapClaimCompileError(
            "candidate source and return observations must be disjoint",
            details={"gap_id": candidate.gap_id},
        )
    expected_gap_id = custody_gap_id(
        candidate.profile_id,
        candidate.asset,
        candidate.source_ids,
        candidate.return_ids,
    )
    if candidate.gap_id != expected_gap_id:
        raise CustodyGapClaimCompileError(
            "candidate identity does not match its boundary transactions",
            details={
                "gap_id": candidate.gap_id,
                "expected_gap_id": expected_gap_id,
            },
        )

    observations = (*sources, *returns)
    quantity_hashes = [observation.quantity_hash for observation in observations]
    if len(set(quantity_hashes)) != len(quantity_hashes):
        raise CustodyGapClaimCompileError(
            "candidate boundary contains duplicate canonical quantity hashes",
            details={"gap_id": candidate.gap_id},
        )
    assets = {observation.asset for observation in observations}
    chains = {observation.event_key.chain for observation in observations}
    networks = {observation.event_key.network for observation in observations}
    if assets != {candidate.asset}:
        raise CustodyGapClaimCompileError(
            "candidate asset does not match every observation",
            details={"gap_id": candidate.gap_id, "assets": sorted(assets)},
        )
    if len(chains) != 1 or len(networks) != 1:
        raise CustodyGapClaimCompileError(
            "candidate observations cross chain or network scope",
            details={
                "gap_id": candidate.gap_id,
                "chains": sorted(chains),
                "networks": sorted(networks),
            },
        )
    source_wallet_ids = tuple(sorted({item.wallet_id for item in sources}))
    destination_wallet_ids = tuple(sorted({item.wallet_id for item in returns}))
    if (
        source_wallet_ids != candidate.source_wallet_ids
        or destination_wallet_ids != candidate.destination_wallet_ids
    ):
        raise CustodyGapClaimCompileError(
            "candidate wallet scope does not match canonical observations",
            details={
                "gap_id": candidate.gap_id,
                "source_wallet_ids": source_wallet_ids,
                "destination_wallet_ids": destination_wallet_ids,
            },
        )

    source_total = sum(observation.principal_msat for observation in sources)
    source_fee = sum(observation.fee_msat for observation in sources)
    source_debit = sum(-observation.wallet_delta_msat for observation in sources)
    return_total = sum(observation.principal_msat for observation in returns)
    retained = min(source_total, return_total)
    expected = {
        "source_total_msat": source_total,
        "source_fee_msat": source_fee,
        "source_debit_msat": source_debit,
        "return_total_msat": return_total,
        "retained_msat": retained,
        "residual_msat": source_total - retained,
        "excess_msat": return_total - retained,
        "coverage_ppm": retained * 1_000_000 // source_total,
    }
    actual = {field: getattr(candidate, field) for field in expected}
    if actual != expected:
        raise CustodyGapClaimCompileError(
            "candidate quantities do not match canonical observations",
            details={
                "gap_id": candidate.gap_id,
                "expected": expected,
                "actual": actual,
            },
        )
    if retained <= 0 or retained > source_total:
        raise CustodyGapClaimCompileError(
            "candidate retained quantity exceeds source principal",
            details={"gap_id": candidate.gap_id},
        )


def _allocate_claims(
    candidate: CustodyGapCandidate,
    sources: Sequence[QuantityObservation],
    returns: Sequence[QuantityObservation],
    *,
    bundle_id: str,
) -> tuple[QuantityClaim, ...]:
    try:
        allocation = allocate_msat_fifo(
            [
                (source.quantity_hash, source.principal_msat)
                for source in sources
            ],
            [
                (target.quantity_hash, target.principal_msat)
                for target in returns
            ],
            amount_msat=candidate.retained_msat,
        )
    except CustodyAllocationError as exc:
        raise CustodyGapClaimCompileError(
            "candidate observations cannot conserve retained principal",
            details={
                "gap_id": candidate.gap_id,
                "allocation_code": exc.code,
                **exc.details,
            },
        ) from exc
    sources_by_hash = {item.quantity_hash: item for item in sources}
    returns_by_hash = {item.quantity_hash: item for item in returns}
    claims: list[QuantityClaim] = []
    for cell in allocation.cells:
        source = sources_by_hash[cell.source_id]
        target = returns_by_hash[cell.sink_id]
        source_slice = QuantitySlice(
            source.quantity_hash,
            cell.source_start_msat,
            cell.source_end_msat,
        )
        target_slice = QuantitySlice(
            target.quantity_hash,
            cell.sink_start_msat,
            cell.sink_end_msat,
        )
        claims.append(
            QuantityClaim(
                claim_id=f"{bundle_id}:allocation:{len(claims):04d}",
                source=source_slice,
                target=target_slice,
                state=CUSTODY_CANDIDATE,
                priority=ClaimPriority.HEURISTIC_CANDIDATE,
                reason="promoted_missing_wallet_candidate",
                supporting_evidence_hashes=tuple(
                    sorted(
                        {
                            source.evidence_detail_hash,
                            target.evidence_detail_hash,
                        }
                    )
                ),
                atomic_bundle_id=bundle_id,
            )
        )
    return tuple(claims)


__all__ = [
    "CustodyGapClaimCompileError",
    "GapCandidateClaimCompilation",
    "compile_gap_candidate_claims",
]
