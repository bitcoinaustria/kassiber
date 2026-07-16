"""Compile advisory custody-gap candidates into non-authoritative holds.

Suggestions may identify exact observed boundaries that need review, but they
must never assert lineage between those boundaries.  This module validates a
promotion-eligible derived candidate against canonical observations and emits
only independent source/return holds.  A reviewed component remains the sole
way to bridge a missing custody interval and carry basis.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Mapping, Sequence

from .custody_evidence import QuantityObservation
from .custody_gaps import CustodyGapCandidate, custody_gap_id
from .custody_quantity import QuantitySlice


class CustodyGapHoldCompileError(ValueError):
    """A candidate and its canonical boundary observations do not agree."""

    def __init__(self, message: str, *, details: Mapping[str, object] | None = None):
        super().__init__(message)
        self.code = "custody_gap_hold_compile"
        self.details = dict(details or {})


@dataclass(frozen=True)
class CustodyGapHold:
    """One exact observed boundary held for review, with no counter-edge."""

    hold_id: str
    gap_id: str
    transaction_id: str
    direction: str
    quantity: QuantitySlice
    evidence_detail_hash: str


@dataclass(frozen=True)
class GapCandidateHoldCompilation:
    gap_id: str
    holds: tuple[CustodyGapHold, ...]
    retained_msat: int
    residual_msat: int
    excess_msat: int


def compile_gap_candidate_holds(
    candidate: CustodyGapCandidate,
    observations_by_transaction: Mapping[str, QuantityObservation],
) -> GapCandidateHoldCompilation:
    """Return independent boundary holds for one validated derived candidate."""

    if not isinstance(candidate, CustodyGapCandidate):
        raise CustodyGapHoldCompileError("candidate has an unsupported type")
    if not candidate.promotion_eligible:
        return GapCandidateHoldCompilation(candidate.gap_id, (), 0, 0, 0)

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
    holds = tuple(
        CustodyGapHold(
            hold_id=f"gap-hold:{candidate.gap_id}:{observation.quantity_hash}",
            gap_id=candidate.gap_id,
            transaction_id=observation.transaction_id,
            direction=observation.direction,
            quantity=QuantitySlice(
                observation.quantity_hash,
                0,
                observation.principal_msat,
            ),
            evidence_detail_hash=observation.evidence_detail_hash,
        )
        for observation in (*sources, *returns)
        if observation.principal_msat > 0
    )
    return GapCandidateHoldCompilation(
        gap_id=candidate.gap_id,
        holds=holds,
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
        raise CustodyGapHoldCompileError(
            "candidate transaction ids must be non-empty and canonically ordered",
            details={"gap_id": candidate.gap_id, "direction": direction},
        )
    if len(set(transaction_ids)) != len(transaction_ids):
        raise CustodyGapHoldCompileError(
            "candidate transaction ids must be unique",
            details={"gap_id": candidate.gap_id, "direction": direction},
        )
    observations: list[QuantityObservation] = []
    for transaction_id in transaction_ids:
        observation = observations_by_transaction.get(transaction_id)
        if observation is None:
            raise CustodyGapHoldCompileError(
                "candidate references a missing canonical observation",
                details={
                    "gap_id": candidate.gap_id,
                    "transaction_id": transaction_id,
                },
            )
        try:
            observation._validate()
        except ValueError as exc:
            raise CustodyGapHoldCompileError(
                "candidate references an invalid canonical observation",
                details={
                    "gap_id": candidate.gap_id,
                    "transaction_id": transaction_id,
                    "reason": str(exc),
                },
            ) from exc
        if observation.transaction_id != transaction_id:
            raise CustodyGapHoldCompileError(
                "observation mapping key does not match transaction identity",
                details={
                    "gap_id": candidate.gap_id,
                    "transaction_id": transaction_id,
                    "observation_transaction_id": observation.transaction_id,
                },
            )
        if observation.direction != direction:
            raise CustodyGapHoldCompileError(
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
        raise CustodyGapHoldCompileError(
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
        raise CustodyGapHoldCompileError(
            "candidate identity does not match its boundary transactions",
            details={"gap_id": candidate.gap_id, "expected_gap_id": expected_gap_id},
        )

    observations = (*sources, *returns)
    quantity_hashes = [item.quantity_hash for item in observations]
    if len(set(quantity_hashes)) != len(quantity_hashes):
        raise CustodyGapHoldCompileError(
            "candidate boundary contains duplicate canonical quantity hashes",
            details={"gap_id": candidate.gap_id},
        )
    assets = {item.asset for item in observations}
    chains = {item.event_key.chain for item in observations}
    networks = {item.event_key.network for item in observations}
    if assets != {candidate.asset}:
        raise CustodyGapHoldCompileError(
            "candidate asset does not match every observation",
            details={"gap_id": candidate.gap_id, "assets": sorted(assets)},
        )
    if len(chains) != 1 or len(networks) != 1:
        raise CustodyGapHoldCompileError(
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
        raise CustodyGapHoldCompileError(
            "candidate wallet scope does not match canonical observations",
            details={
                "gap_id": candidate.gap_id,
                "source_wallet_ids": source_wallet_ids,
                "destination_wallet_ids": destination_wallet_ids,
            },
        )

    source_total = sum(item.principal_msat for item in sources)
    source_fee = sum(item.fee_msat for item in sources)
    source_debit = sum(-item.wallet_delta_msat for item in sources)
    return_total = sum(item.principal_msat for item in returns)
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
        raise CustodyGapHoldCompileError(
            "candidate quantities do not match canonical observations",
            details={"gap_id": candidate.gap_id, "expected": expected, "actual": actual},
        )


__all__ = [
    "CustodyGapHold",
    "CustodyGapHoldCompileError",
    "GapCandidateHoldCompilation",
    "compile_gap_candidate_holds",
]
