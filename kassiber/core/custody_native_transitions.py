"""Automatic custody edges from complete native HTLC evidence.

The full matcher graph determines exactness and conflicts before review vetoes,
occupied anchors or profile policy are considered. This module creates only
transient interpretations; it never authors a reviewed component or pair.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timezone
import hashlib
import json
from typing import Any, Mapping, Sequence

from ..asset_codes import canonical_bitcoin_asset
from ..tax_policy import recommended_pair_policy
from ..time_utils import parse_iso_datetime_or_none
from ..transfers import is_lightning_payment_hash_row
from .chain_observer.provenance import row_has_current_authoritative_observation
from .custody_evidence import QuantityObservation
from .onchain import stored_tx_mapping
from .transfer_matching import (
    CONFIDENCE_EXACT,
    METHOD_HTLC_REFUND,
    METHOD_PAYMENT_HASH,
    SwapCandidate,
    active_dismissed_pairs,
    suggest_swap_candidates,
)


NATIVE_TRANSITION_SOURCE = "native_htlc_transition"


def has_native_transition_evidence(rows: Sequence[Mapping[str, Any]]) -> bool:
    """Cheap presence gate including older raw-only refund evidence."""
    for row in rows:
        if any(
            key in row.keys() and row[key]
            for key in ("swap_refund_funding_txid", "swap_refund_funding_outpoint")
        ) or (
            "payment_hash_source" in row.keys()
            and row["payment_hash_source"] == "chain_script_unique_outpoint"
        ):
            return True
        raw = row["raw_json"] if "raw_json" in row.keys() else None
        # Most rows have no witness/attestation. Avoid parsing their graph at
        # all; refund recovery needs the rare legacy raw-only witness payload.
        if isinstance(raw, str) and not any(
            marker in raw for marker in ("htlc_spend", "witness")
        ):
            continue
        payload = stored_tx_mapping(raw)
        if payload is None:
            continue
        if payload.get("htlc_spend"):
            return True
        graph = payload.get("tx") if isinstance(payload.get("tx"), Mapping) else payload
        vins = graph.get("vin")
        if isinstance(vins, list) and any(
            isinstance(vin, Mapping) and (vin.get("witness") or vin.get("txinwitness"))
            for vin in vins
        ):
            return True
    return False


@dataclass(frozen=True)
class NativeTransitionProof:
    """A candidate bound to the canonical quantities used by claim arbitration."""

    candidate: SwapCandidate
    source_quantity_hash: str
    target_quantity_hash: str
    policy: str

    def validates(
        self,
        pair: Mapping[str, Any],
        source: QuantityObservation,
        target: QuantityObservation,
    ) -> bool:
        candidate = self.candidate
        return bool(
            candidate.confidence == CONFIDENCE_EXACT
            and candidate.conflict_size == 1
            and candidate.method in {METHOD_PAYMENT_HASH, METHOD_HTLC_REFUND}
            and source.quantity_hash == self.source_quantity_hash
            and target.quantity_hash == self.target_quantity_hash
            and pair.get("kind") == candidate.default_kind
            and pair.get("policy") == self.policy
            and pair.get("out_amount_msat") == source.principal_msat
        )


def _native_endpoint(row: Mapping[str, Any], observation: QuantityObservation) -> bool:
    return is_lightning_payment_hash_row(row) or (
        observation.authoritative_chain_observation
        and row_has_current_authoritative_observation(row)
    )


def compile_native_transitions(
    rows: Sequence[Mapping[str, Any]],
    observations: Mapping[str, QuantityObservation],
    *,
    profile: Mapping[str, Any],
    occupied_ids: set[str],
    dismissals: Sequence[Mapping[str, Any]] = (),
) -> tuple[
    list[dict[str, Any]],
    dict[tuple[str, str], NativeTransitionProof],
    tuple[Mapping[str, Any], ...],
]:
    """Interpret only solo native exact edges from the original population."""

    # Neither dismissed nor occupied legs may disappear before conflict and
    # payment-hash cardinality checks. Otherwise a reviewed sibling would make
    # the remaining ambiguous edge look exact on the next journal rebuild.
    candidates = suggest_swap_candidates(rows, include_heuristics=False)
    dismissed = active_dismissed_pairs(dismissals)
    rows_by_id = {str(row["id"]): row for row in rows}
    pairs: list[dict[str, Any]] = []
    proofs: dict[tuple[str, str], NativeTransitionProof] = {}
    quarantines: list[Mapping[str, Any]] = []
    for candidate in candidates:
        key = (candidate.out_id, candidate.in_id)
        if (
            candidate.confidence != CONFIDENCE_EXACT
            or candidate.conflict_size != 1
            or candidate.method not in {METHOD_PAYMENT_HASH, METHOD_HTLC_REFUND}
            or key in dismissed
            or occupied_ids.intersection(key)
            or canonical_bitcoin_asset(candidate.out_asset) is None
            or canonical_bitcoin_asset(candidate.in_asset) is None
        ):
            continue
        source, target = observations.get(key[0]), observations.get(key[1])
        out_row, in_row = rows_by_id[key[0]], rows_by_id[key[1]]
        if (
            source is None or target is None
            or not _native_endpoint(out_row, source)
            or not _native_endpoint(in_row, target)
        ):
            continue
        source_when = parse_iso_datetime_or_none(source.occurred_at)
        target_when = parse_iso_datetime_or_none(target.occurred_at)
        shortfall = source.principal_msat - target.principal_msat
        unresolved_reason = None
        required_for = None
        if source.principal_msat <= 0 or target.principal_msat <= 0 or shortfall < 0:
            # Raw candidate amounts can include source fees. Whole-row proof
            # must still conserve after the canonical boundary normalizes them;
            # clamping would invent a partial MOVE plus a fresh acquisition.
            unresolved_reason = "native_transition_amount_mismatch"
            required_for = "conserving_native_transition_quantities"
        elif shortfall > 0 and (
            source_when is None or target_when is None
            or source_when.astimezone(timezone.utc).date()
            != target_when.astimezone(timezone.utc).date()
        ):
            # The current pair projection combines fee slices on the source
            # date. The route is proven, but placing a later claim/refund loss
            # on that date would fabricate fee timing (including the tax year).
            # Keep both anchors held until explicit fee timing is representable.
            unresolved_reason = "native_transition_fee_timing_unresolved"
            required_for = "explicit_transition_fee_timeline"
        if unresolved_reason:
            for row in (out_row, in_row):
                quarantines.append({
                    "transaction_id": row["id"],
                    "workspace_id": row["workspace_id"],
                    "profile_id": row["profile_id"],
                    "reason": unresolved_reason,
                    "detail_json": json.dumps({
                        "out_transaction_id": key[0], "in_transaction_id": key[1],
                        "source_occurred_at": source.occurred_at,
                        "target_occurred_at": target.occurred_at,
                        "source_principal_msat": source.principal_msat,
                        "target_principal_msat": target.principal_msat,
                        **({"unallocated_fee_msat": shortfall} if shortfall > 0 else {}),
                        "required_for": required_for,
                    }, sort_keys=True),
                })
            continue
        policy = recommended_pair_policy(profile, candidate.out_asset, candidate.in_asset)
        pair_digest = hashlib.sha256(json.dumps(
            [candidate.method, *key], separators=(",", ":")
        ).encode()).hexdigest()
        pairs.append({
            "out": out_row, "in": in_row,
            "source": NATIVE_TRANSITION_SOURCE,
            "pair_id": f"native-htlc-{pair_digest}",
            "kind": candidate.default_kind,
            "policy": policy,
            "out_amount_msat": source.principal_msat,
        })
        proofs[key] = NativeTransitionProof(
            candidate, source.quantity_hash, target.quantity_hash, policy
        )
    return pairs, proofs, tuple(quarantines)
