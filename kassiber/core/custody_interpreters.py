"""Pre-tax custody interpreter adapters.

Every adapter in this module emits exact :class:`QuantityClaim` values (or a
native-evidence request for a rowless owned destination).  It deliberately
does not import RP2 and it never reads the engine's rendered MOVE audit.  The
claim set is the only input to custody arbitration.
"""

from __future__ import annotations

from dataclasses import dataclass
import json
from typing import Any, Mapping, Sequence

from ..msat import msat_to_btc
from ..transfers import (
    apply_manual_pairs,
    bitcoin_network_domain,
    canonical_payment_hash,
    detect_intra_transfers,
    is_lightning_payment_hash_row,
)
from .loans import (
    CHANNEL_CLOSE,
    CHANNEL_CLOSE_MISMATCH,
    CHANNEL_OPEN,
    CHANNEL_OPEN_MISMATCH,
)
from .custody_evidence import CanonicalQuantityInput, QuantityObservation
from .custody_quantity import (
    CUSTODY_SUSPENSE,
    EXTERNAL_CONFIRMED,
    ClaimPriority,
    INTERNAL_REVIEWED,
    INTERNAL_VERIFIED,
    QuantityClaim,
    QuantityDomain,
    QuantitySlice,
)
from .ownership_transfers import derive_profile_transfers
from .privacy_hops import privacy_hop_evidence_from_row


def _field(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    if hasattr(row, "keys") and key not in row.keys():
        return default
    if hasattr(row, "get"):
        return row.get(key, default)
    return row[key]


@dataclass(frozen=True)
class CustodyInterpreterCompilation:
    """Candidate claims and rowless native destinations before arbitration."""

    claims: tuple[QuantityClaim, ...]
    native_audits: tuple[Mapping[str, Any], ...]
    cross_asset_pairs: tuple[Mapping[str, Any], ...]
    non_event_transaction_ids: tuple[str, ...]
    blocked_transaction_ids: tuple[str, ...] = ()
    direct_payout_conflict_transaction_ids: tuple[str, ...] = ()
    quarantines: tuple[Mapping[str, Any], ...] = ()

    @property
    def blocking_quarantines(self) -> tuple[Mapping[str, Any], ...]:
        blocked = set(self.blocked_transaction_ids)
        return tuple(
            item
            for item in self.quarantines
            if str(_field(item, "transaction_id") or "") in blocked
        )


def _observations_by_transaction(
    canonical: CanonicalQuantityInput,
) -> dict[str, QuantityObservation]:
    by_hash = {item.quantity_hash: item for item in canonical.observations}
    return {
        transaction_id: by_hash[quantity_hash]
        for event in canonical.events
        for transaction_id, quantity_hash in event.observation_aliases
    }


def _pair_source(pair: Mapping[str, Any]) -> str:
    return str(_field(pair, "source") or _field(pair, "pair_source") or "")


def _pair_is_reviewed(pair: Mapping[str, Any]) -> bool:
    source = _pair_source(pair)
    return bool(
        _field(pair, "pair_id")
        or source in {"manual", "bulk_exact", "bulk_selected", "rule_auto"}
    )


def _is_reviewed_privacy_kind(value: Any) -> bool:
    kind = str(value or "").strip().lower()
    return kind in {"coinjoin", "whirlpool"} or "coinjoin" in kind


def _pair_priority(pair: Mapping[str, Any]) -> tuple[str, ClaimPriority, str]:
    source = _pair_source(pair)
    if _pair_is_reviewed(pair):
        return (
            INTERNAL_REVIEWED,
            ClaimPriority.REVIEWED_PAIR,
            source or "reviewed_transfer_pair",
        )
    return (
        INTERNAL_VERIFIED,
        ClaimPriority.EXACT_NATIVE_EVENT,
        source or "row_matched",
    )


def _is_exact_recorded_pair(
    pair: Mapping[str, Any],
    out_row: Mapping[str, Any],
    in_row: Mapping[str, Any],
    source: QuantityObservation,
    target: QuantityObservation,
) -> bool:
    """Revalidate exact two-ended evidence at the claim boundary.

    ``pair.source`` is useful audit text, not an authority token. Rechecking
    the physical identity here prevents a future caller from upgrading an
    amount/time candidate merely by copying a trusted source label.
    """

    source_name = _pair_source(pair)
    if source_name == "row_matched":
        return bool(
            source.event_key == target.event_key
            and source.event_key.native_namespace == "chain"
            and source.event_key.chain in {"bitcoin", "liquid"}
            and source.wallet_id != target.wallet_id
        )
    if source_name != "lightning_payment_hash":
        return False
    out_hash = canonical_payment_hash(_field(out_row, "payment_hash"))
    in_hash = canonical_payment_hash(_field(in_row, "payment_hash"))
    return bool(
        out_hash is not None
        and out_hash == in_hash
        and is_lightning_payment_hash_row(out_row)
        and is_lightning_payment_hash_row(in_row)
        and source.asset == target.asset
        and source.principal_msat == target.principal_msat
        and bitcoin_network_domain(out_row) is not None
        and bitcoin_network_domain(out_row) == bitcoin_network_domain(in_row)
    )


def _row_id(row: Mapping[str, Any]) -> str:
    return str(_field(row, "id") or "")


def _anchor_id(row: Mapping[str, Any]) -> str:
    return str(_field(row, "journal_transaction_id") or _row_id(row))


def _samourai_privacy_pairs(
    rows: Sequence[Mapping[str, Any]],
    observations: Mapping[str, QuantityObservation],
) -> tuple[
    list[Mapping[str, Any]],
    list[QuantityClaim],
    set[str],
    set[str],
]:
    """Compile tracked Whirlpool lifecycle fan-outs before generic matching."""

    grouped: dict[tuple[str, object], list[tuple[Mapping[str, Any], str]]] = {}
    for row in rows:
        try:
            config = json.loads(_field(row, "config_json") or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        metadata = config.get("samourai") if isinstance(config, Mapping) else None
        if not isinstance(metadata, Mapping) or metadata.get("role") != "child":
            continue
        group_id = str(metadata.get("group_id") or "").strip()
        section = str(metadata.get("section") or "").strip().lower()
        observation = observations.get(_anchor_id(row))
        if not group_id or not section or observation is None:
            continue
        grouped.setdefault((group_id, observation.event_key), []).append((row, section))

    pairs: list[Mapping[str, Any]] = []
    fee_claims: list[QuantityClaim] = []
    candidate_ids: set[str] = set()
    touched_ids: set[str] = set()
    for (wallet_group_id, event_key), entries in sorted(
        grouped.items(), key=lambda item: (item[0][0], str(item[0][1]))
    ):
        outs = [row for row, _section in entries if _field(row, "direction") == "outbound"]
        ins = [row for row, _section in entries if _field(row, "direction") == "inbound"]
        out_sections = {
            section for row, section in entries if _field(row, "direction") == "outbound"
        }
        in_sections = {
            section for row, section in entries if _field(row, "direction") == "inbound"
        }
        is_tx0 = "deposit" in out_sections and bool(
            in_sections & {"premix", "badbank"}
        )
        lifecycle = (
            is_tx0
            or ("premix" in out_sections and "postmix" in in_sections)
            or ("postmix" in out_sections and "postmix" in in_sections)
        )
        if not lifecycle or len(outs) != 1 or not ins:
            continue
        out_row = outs[0]
        source = observations.get(_anchor_id(out_row))
        if source is None:
            continue
        group_id = (
            f"samourai:{wallet_group_id}:{event_key.native_event_id}"
        )
        group_candidate_ids = {
            _anchor_id(row) for row in (*outs, *ins) if _anchor_id(row)
        }
        candidate_ids.update(group_candidate_ids)
        allocated = 0
        allocated_targets: list[QuantityObservation] = []
        group_touched_ids: set[str] = set()
        authoritative_native_group = source.authoritative_chain_observation
        for in_row in sorted(ins, key=_row_id):
            target = observations.get(_anchor_id(in_row))
            if target is None or target.asset != source.asset:
                authoritative_native_group = False
                continue
            amount_msat = target.principal_msat
            if amount_msat <= 0 or allocated + amount_msat > source.principal_msat:
                authoritative_native_group = False
                continue
            if (
                not target.authoritative_chain_observation
                or target.event_key != source.event_key
            ):
                authoritative_native_group = False
            pairs.append(
                {
                    "out": out_row,
                    "in": in_row,
                    "group_id": group_id,
                    "source": "samourai_internal",
                    "out_amount": amount_msat,
                    "allow_unclaimed_residual": True,
                }
            )
            allocated += amount_msat
            allocated_targets.append(target)
            group_touched_ids.update((_anchor_id(out_row), _anchor_id(in_row)))
        residual_msat = source.principal_msat - allocated
        if allocated and residual_msat > 0:
            exact_coordinator_fee = bool(
                is_tx0
                and authoritative_native_group
                and source.fee_attribution == "exact"
            )
            supporting_evidence_hashes = {source.evidence_detail_hash}
            supporting_evidence_hashes.update(
                target.evidence_detail_hash for target in allocated_targets
            )
            fee_claims.append(
                QuantityClaim(
                    claim_id=f"samourai-fee:{source.quantity_hash}:{residual_msat}",
                    source=QuantitySlice(
                        source.quantity_hash,
                        allocated,
                        source.principal_msat,
                    ),
                    state=(
                        EXTERNAL_CONFIRMED
                        if exact_coordinator_fee
                        else CUSTODY_SUSPENSE
                    ),
                    priority=(
                        ClaimPriority.EXACT_NATIVE_EVENT
                        if exact_coordinator_fee
                        else ClaimPriority.ACCOUNTING_CONVENTION
                    ),
                    reason=(
                        "samourai_coordinator_fee"
                        if exact_coordinator_fee
                        else "implicit_wallet_delta_unallocated"
                    ),
                    supporting_evidence_hashes=tuple(
                        sorted(supporting_evidence_hashes)
                    ),
                    atomic_bundle_id=f"pair-group:{group_id}",
                    # A targetless fee is a finalized external classification.
                    # An imported or implicit wallet delta is only a custody
                    # discrepancy and must remain destination-neutral suspense.
                    destination_kind="fee" if exact_coordinator_fee else None,
                )
            )
        # Structured wallet metadata groups candidates; it is not authority.
        # Suppress the ordinary privacy-hop blocker only after every used leg
        # in a Tx0 group was independently observed at the canonical boundary.
        if authoritative_native_group:
            touched_ids.update(group_touched_ids)
    return pairs, fee_claims, candidate_ids, touched_ids


def _exact_native_pair_ids(
    pairs: Sequence[Mapping[str, Any]],
    observations: Mapping[str, QuantityObservation],
) -> set[str]:
    """Return pair anchors proven to be two legs of one native chain event.

    Privacy metadata is a conservative blocker when a row is otherwise
    unexplained.  It must not overrule a canonical Bitcoin/Liquid txid that is
    observed on both imported profile wallets.  Requiring the same canonical
    chain event here is important: equal CoinJoin denominations, nearby
    timestamps, or the generic amount matcher alone are not ownership proof.
    """

    resolved: set[str] = set()
    for pair in pairs:
        out_id = _anchor_id(_field(pair, "out", {}) or {})
        in_id = _anchor_id(_field(pair, "in", {}) or {})
        source = observations.get(out_id)
        target = observations.get(in_id)
        if (
            source is None
            or target is None
            or source.direction != "outbound"
            or target.direction != "inbound"
            or not source.authoritative_chain_observation
            or not target.authoritative_chain_observation
            or source.event_key != target.event_key
            or source.event_key.native_namespace != "chain"
            or source.event_key.chain not in {"bitcoin", "liquid"}
        ):
            continue
        resolved.update((out_id, in_id))
    return resolved


def _pair_claims(
    pairs: Sequence[Mapping[str, Any]],
    observations: Mapping[str, QuantityObservation],
    *,
    excluded_transaction_ids: set[str],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
) -> tuple[
    tuple[QuantityClaim, ...],
    tuple[str, ...],
    tuple[Mapping[str, Any], ...],
]:
    """Compile ordinary recorded pairs without manufacturing fee residuals."""

    source_cursor: dict[str, int] = {}
    target_cursor: dict[str, int] = {}
    claims: list[QuantityClaim] = []
    blocked_transaction_ids: set[str] = set()
    quarantines: list[Mapping[str, Any]] = []
    ordered_pairs = sorted(
        pairs,
        key=lambda item: (
            1 if _pair_is_reviewed(item) else 0,
            str(_field(_field(item, "out", {}) or {}, "occurred_at") or ""),
            _row_id(_field(item, "out", {}) or {}),
            _row_id(_field(item, "in", {}) or {}),
            str(_field(item, "pair_id") or ""),
        ),
    )
    source_last_ordinal: dict[str, int] = {}
    implicit_sources: dict[str, QuantityObservation] = {}
    for ordinal, pair in enumerate(ordered_pairs):
        source_id = _anchor_id(_field(pair, "out", {}) or {})
        source_last_ordinal[source_id] = ordinal

    for ordinal, pair in enumerate(ordered_pairs):
        out_row = _field(pair, "out", {}) or {}
        in_row = _field(pair, "in", {}) or {}
        out_id, in_id = _anchor_id(out_row), _anchor_id(in_row)
        if not out_id or not in_id or {out_id, in_id} & excluded_transaction_ids:
            continue
        source, target = observations.get(out_id), observations.get(in_id)
        # Rowless owned destinations are compiled by custody_native_audit below.
        invalid_reviewed_pair = _pair_is_reviewed(pair) and (
            source is None
            or target is None
            or source.direction != "outbound"
            or target.direction != "inbound"
        )
        if invalid_reviewed_pair:
            detail = {
                "required_for": "valid_reviewed_transfer_legs",
                "out_direction": source.direction if source is not None else None,
                "in_direction": target.direction if target is not None else None,
            }
            for transaction_id, row, paired_leg in (
                (out_id, out_row, False),
                (in_id, in_row, True),
            ):
                if not transaction_id:
                    continue
                quarantines.append(
                    {
                        "transaction_id": transaction_id,
                        "workspace_id": _field(row, "workspace_id"),
                        "profile_id": _field(row, "profile_id"),
                        "reason": "transfer_pair_leg_invalid",
                        "detail_json": json.dumps(
                            {
                                **detail,
                                **({"paired_leg": True} if paired_leg else {}),
                            },
                            sort_keys=True,
                        ),
                    }
                )
            blocked_transaction_ids.update((out_id, in_id))
            continue
        if source is None or target is None:
            continue
        if source.direction != "outbound" or target.direction != "inbound":
            continue
        source_name = _pair_source(pair)
        exact_native_pair = (
            source.authoritative_chain_observation
            and target.authoritative_chain_observation
            and source.event_key == target.event_key
            and source.event_key.native_namespace == "chain"
            and source.event_key.chain in {"bitcoin", "liquid"}
        )
        exact_recorded_pair = _is_exact_recorded_pair(
            pair, out_row, in_row, source, target
        )
        if (
            not _pair_is_reviewed(pair)
            and source_name != "channel_lifecycle"
            and not exact_recorded_pair
            and not exact_native_pair
        ):
            # Amount/time coincidence and imported graph-shaped JSON are
            # suggestions, never native ownership proof. A shared canonical
            # txid across two observed wallet rows and a native-node payment
            # hash are different: both endpoints are independently observed,
            # so no graph-derived destination is being manufactured.
            continue
        source_start = source_cursor.get(source.quantity_hash, 0)
        target_start = target_cursor.get(target.quantity_hash, 0)
        requested = _field(pair, "out_amount_msat")
        if requested in (None, ""):
            requested = _field(pair, "out_amount")
        available_source = source.principal_msat - source_start
        available_target = target.principal_msat - target_start
        pair_kind = str(_field(pair, "kind") or "")
        transition_kind = pair_kind in {
            "chain-swap",
            "peg-in",
            "peg-out",
            "reverse-submarine-swap",
            "submarine-swap",
            "swap-refund",
        }
        requested_msat = (
            available_source
            if requested in (None, "") and transition_kind
            else (
                min(available_source, available_target)
                if requested in (None, "")
                else int(requested)
            )
        )
        amount_msat = min(
            requested_msat,
            available_source,
            available_target,
        )
        if amount_msat <= 0:
            continue
        state, priority, reason = _pair_priority(pair)
        pair_id = str(_field(pair, "pair_id") or f"{out_id}:{in_id}:{ordinal}")
        pair_group_id = str(_field(pair, "group_id") or "")
        atomic_bundle_id = (
            f"pair-group:{pair_group_id}"
            if pair_group_id
            else f"pair:{pair_id}"
        )
        source_domain = QuantityDomain.from_observation(source)
        target_domain = QuantityDomain.from_observation(target)
        if source_domain.exposure != target_domain.exposure:
            # A reviewed crypto-to-crypto conversion is an economic/tax
            # relation, not conservation of the same physical quantity.  Its
            # metadata is preserved separately for the tax engine below.
            continue
        if (source_domain.network, source_domain.unit) != (
            target_domain.network,
            target_domain.unit,
        ):
            detail = {
                "source_network": source_domain.network,
                "target_network": target_domain.network,
                "required_for": "compatible_custody_domains",
            }
            for transaction_id, row, paired_leg in (
                (out_id, out_row, False),
                (in_id, in_row, True),
            ):
                quarantines.append(
                    {
                        "transaction_id": transaction_id,
                        "workspace_id": _field(row, "workspace_id"),
                        "profile_id": _field(row, "profile_id"),
                        "reason": "transfer_network_mismatch",
                        "detail_json": json.dumps(
                            {
                                **detail,
                                **({"paired_leg": True} if paired_leg else {}),
                            },
                            sort_keys=True,
                        ),
                    }
                )
            blocked_transaction_ids.update((out_id, in_id))
            continue
        allow_cross_rail = source_domain.rail != target_domain.rail
        # A cross-asset link carries custody only when it was explicitly
        # reviewed as carrying value.  The quantity domain validates the
        # Bitcoin-network/msat restriction independently.
        if allow_cross_rail and str(_field(pair, "policy") or "") != "carrying-value":
            continue
        inferred_fee_msat = max(0, min(requested_msat, available_source) - amount_msat)
        unclaimed_after_pair = available_source - amount_msat - inferred_fee_msat
        fee_tolerance_msat = max(source.principal_msat // 100, 2_500_000)
        if (
            not _pair_is_reviewed(pair)
            and not bool(_field(pair, "allow_unclaimed_residual", False))
            and source_last_ordinal.get(out_id) == ordinal
            and unclaimed_after_pair > fee_tolerance_msat
        ):
            detail = {
                "from_wallet": _field(
                    wallet_refs_by_id.get(source.wallet_id, {}),
                    "label",
                    source.wallet_id,
                ),
                "to_wallet": _field(
                    wallet_refs_by_id.get(target.wallet_id, {}),
                    "label",
                    target.wallet_id,
                ),
                "sent": float(msat_to_btc(source.principal_msat)),
                "received": float(msat_to_btc(amount_msat)),
                "implied_fee": float(msat_to_btc(unclaimed_after_pair)),
                "fee_ceiling": float(msat_to_btc(fee_tolerance_msat)),
                "required_for": "complete_transfer_component",
            }
            for transaction_id, row, paired_leg in (
                (out_id, out_row, False),
                (in_id, in_row, True),
            ):
                quarantines.append(
                    {
                        "transaction_id": transaction_id,
                        "workspace_id": _field(row, "workspace_id"),
                        "profile_id": _field(row, "profile_id"),
                        "reason": "transfer_fee_implausible",
                        "detail_json": json.dumps(
                            {
                                **detail,
                                **({"paired_leg": True} if paired_leg else {}),
                            },
                            sort_keys=True,
                        ),
                    }
                )
            claims.append(
                QuantityClaim(
                    claim_id=f"pair:{pair_id}:implausible-fee",
                    source=QuantitySlice(
                        source.quantity_hash,
                        source_start,
                        source.principal_msat,
                    ),
                    state=CUSTODY_SUSPENSE,
                    priority=ClaimPriority.EXACT_NATIVE_EVENT,
                    reason="transfer_fee_implausible",
                    supporting_evidence_hashes=tuple(
                        sorted(
                            {
                                source.evidence_detail_hash,
                                target.evidence_detail_hash,
                            }
                        )
                    ),
                    atomic_bundle_id=atomic_bundle_id,
                )
            )
            source_cursor[source.quantity_hash] = source.principal_msat
            blocked_transaction_ids.update((out_id, in_id))
            continue
        claims.append(
            QuantityClaim(
                claim_id=f"pair:{pair_id}:slice:{source_start}:{amount_msat}",
                source=QuantitySlice(source.quantity_hash, source_start, source_start + amount_msat),
                target=QuantitySlice(target.quantity_hash, target_start, target_start + amount_msat),
                state=state,
                priority=priority,
                reason=reason,
                supporting_evidence_hashes=tuple(
                    sorted({source.evidence_detail_hash, target.evidence_detail_hash})
                ),
                atomic_bundle_id=atomic_bundle_id,
                allow_cross_rail=allow_cross_rail,
                transfer_kind=str(_field(pair, "kind") or "") or None,
                transfer_policy=str(_field(pair, "policy") or "") or None,
                component_id=str(_field(pair, "component_id") or "") or None,
            )
        )
        source_cursor[source.quantity_hash] = source_start + amount_msat
        target_cursor[target.quantity_hash] = target_start + amount_msat
        if source.fee_attribution == "implicit_wallet_delta":
            implicit_sources[source.quantity_hash] = source
        if inferred_fee_msat and (
            transition_kind or not _pair_is_reviewed(pair)
        ) and source.fee_attribution != "implicit_wallet_delta":
            fee_start = source_start + amount_msat
            claims.append(
                QuantityClaim(
                    claim_id=f"pair:{pair_id}:fee:{fee_start}:{inferred_fee_msat}",
                    source=QuantitySlice(
                        source.quantity_hash,
                        fee_start,
                        fee_start + inferred_fee_msat,
                    ),
                    state=EXTERNAL_CONFIRMED,
                    priority=priority,
                    reason="reviewed_transfer_fee",
                    supporting_evidence_hashes=tuple(
                        sorted(
                            {
                                source.evidence_detail_hash,
                                target.evidence_detail_hash,
                            }
                        )
                    ),
                    atomic_bundle_id=atomic_bundle_id,
                    destination_kind="fee",
                )
            )
            source_cursor[source.quantity_hash] += inferred_fee_msat
    for source_hash, source in sorted(implicit_sources.items()):
        residual_start = source_cursor.get(source_hash, 0)
        if residual_start >= source.principal_msat:
            continue
        claims.append(
            QuantityClaim(
                claim_id=f"implicit-wallet-delta:{source_hash}:{residual_start}",
                source=QuantitySlice(
                    source_hash,
                    residual_start,
                    source.principal_msat,
                ),
                state=CUSTODY_SUSPENSE,
                priority=ClaimPriority.ACCOUNTING_CONVENTION,
                reason="implicit_wallet_delta_unallocated",
                supporting_evidence_hashes=(source.evidence_detail_hash,),
            )
        )
    return (
        tuple(claims),
        tuple(sorted(blocked_transaction_ids)),
        tuple(quarantines),
    )


def _native_audits_for_rowless_pairs(
    pairs: Sequence[Mapping[str, Any]],
    observations: Mapping[str, QuantityObservation],
    *,
    excluded_transaction_ids: set[str],
) -> tuple[Mapping[str, Any], ...]:
    """Adapt graph/channel rowless destinations to native evidence requests.

    The older path created these records *after* RP2 had already accepted a
    MOVE.  Here they are source evidence for the arbiter.  The native adapter
    validates every amount and creates the virtual inbound observation before
    any tax input exists.
    """

    accepted_sources = {
        "ownership_derived",
        "multi_source_consolidation",
        "recorded_fanout",
        "channel_lifecycle",
    }
    audits: list[Mapping[str, Any]] = []
    for pair in pairs:
        source_name = _pair_source(pair)
        if source_name not in accepted_sources:
            continue
        out_row = _field(pair, "out", {}) or {}
        in_row = _field(pair, "in", {}) or {}
        out_anchor = _anchor_id(out_row)
        in_anchor = _anchor_id(in_row)
        out_id = _row_id(out_row)
        in_id = _row_id(in_row)
        if (
            source_name == "channel_lifecycle"
            and out_id not in observations
            and in_anchor in observations
            and observations[in_anchor].direction == "inbound"
        ):
            target = observations[in_anchor]
            received = int(_field(in_row, "amount") or 0)
            fee = int(_field(out_row, "fee") or 0)
            if received <= 0:
                continue
            audits.append(
                {
                    "pairing_source": source_name,
                    "virtual_source": True,
                    "out_id": out_id,
                    "in_id": in_id,
                    "out_anchor_transaction_id": out_id,
                    "in_anchor_transaction_id": in_anchor,
                    "from_wallet_id": str(_field(out_row, "wallet_id") or ""),
                    "to_wallet_id": target.wallet_id,
                    "asset": target.asset,
                    "occurred_at": target.occurred_at,
                    "crypto_received_msat": received,
                    "crypto_fee_msat": fee,
                    "crypto_sent_msat": received + fee,
                }
            )
            continue
        if out_anchor in excluded_transaction_ids or out_anchor not in observations:
            continue
        source = observations[out_anchor]
        if (
            source_name != "channel_lifecycle"
            and not source.authoritative_chain_observation
        ):
            continue
        target_is_observed = in_id in observations or (
            in_anchor != out_anchor and in_anchor in observations
        )
        if source.direction != "outbound" or target_is_observed:
            continue
        received = int(_field(in_row, "amount") or 0)
        fee = int(_field(out_row, "fee") or 0)
        if received <= 0:
            continue
        audits.append(
            {
                "pairing_source": source_name,
                "out_id": out_id,
                "in_id": _row_id(in_row),
                "out_anchor_transaction_id": out_anchor,
                "in_anchor_transaction_id": in_anchor,
                "from_wallet_id": source.wallet_id,
                "to_wallet_id": str(_field(in_row, "wallet_id") or ""),
                "asset": str(_field(in_row, "asset") or source.asset).upper(),
                "occurred_at": source.occurred_at,
                "crypto_received_msat": received,
                "crypto_fee_msat": fee,
                "crypto_sent_msat": received + fee,
            }
        )
    return tuple(audits)


def compile_custody_interpreters(
    rows: Sequence[Mapping[str, Any]],
    canonical: CanonicalQuantityInput,
    *,
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    manual_pair_records: Sequence[Mapping[str, Any]] = (),
    owned_index: Any = None,
    channel_transfer_pairs: Sequence[Mapping[str, Any]] = (),
    channel_roles: Mapping[str, str] | None = None,
    loan_legs: Sequence[Mapping[str, Any]] = (),
    direct_payout_records: Sequence[Mapping[str, Any]] = (),
    component_transaction_ids: Sequence[str] = (),
) -> CustodyInterpreterCompilation:
    """Compile every non-component custody interpreter before arbitration."""

    excluded = {str(item) for item in component_transaction_ids if item}
    observations = _observations_by_transaction(canonical)
    (
        samourai_pairs,
        samourai_fee_claims,
        samourai_candidate_ids,
        samourai_touched_ids,
    ) = (
        _samourai_privacy_pairs(rows, observations)
    )
    auto_pairs, _ = detect_intra_transfers(rows)
    rows_by_id = {str(_field(row, "id") or ""): row for row in rows}
    resolved_privacy_ids = {
        str(_field(record, key) or "")
        for record in manual_pair_records
        if _is_reviewed_privacy_kind(_field(record, "kind"))
        for key in ("out_transaction_id", "in_transaction_id")
    }
    # Structured Samourai groups get their own exact quarantine below. Avoid a
    # second generic privacy-hop blocker for the same rows; authoritative groups
    # are resolved, while unverified groups remain blocked by the specific
    # native-event requirement.
    resolved_privacy_ids.update(samourai_candidate_ids)
    # Canonical native-event identity is stronger than the generic privacy-hop
    # warning. This does not trust amount-only CoinJoin matches: both rows must
    # resolve to the same protocol-qualified txid above.
    resolved_privacy_ids.update(_exact_native_pair_ids(auto_pairs, observations))
    resolved_privacy_ids.update(excluded)
    privacy_quarantines = tuple(
        {
            "transaction_id": str(_field(row, "id") or ""),
            "workspace_id": _field(row, "workspace_id"),
            "profile_id": _field(row, "profile_id"),
            "reason": "privacy_hop_unresolved",
            "detail_json": json.dumps(
                {
                    "wallet": _field(
                        wallet_refs_by_id.get(
                            str(_field(row, "wallet_id") or ""), {}
                        ),
                        "label",
                        _field(row, "wallet_id"),
                    ),
                    "asset": str(_field(row, "asset") or "").upper(),
                    "direction": _field(row, "direction"),
                    **(privacy_hop_evidence_from_row(row) or {}),
                },
                sort_keys=True,
            ),
        }
        for row in rows
        if str(_field(row, "id") or "") not in resolved_privacy_ids
        and privacy_hop_evidence_from_row(row) is not None
    )
    privacy_blocked_ids = {
        str(_field(item, "transaction_id") or "")
        for item in privacy_quarantines
    }
    samourai_unverified_ids = samourai_candidate_ids - samourai_touched_ids - excluded
    samourai_unverified_quarantines = tuple(
        {
            "transaction_id": transaction_id,
            "workspace_id": _field(rows_by_id.get(transaction_id, {}), "workspace_id"),
            "profile_id": _field(rows_by_id.get(transaction_id, {}), "profile_id"),
            "reason": "samourai_native_event_unverified",
            "detail_json": json.dumps(
                {
                    "required_for": "authoritative_same_event_observation",
                    "resolution": "sync every imported Samourai source with a chain observer",
                },
                sort_keys=True,
            ),
        }
        for transaction_id in sorted(samourai_unverified_ids)
    )
    channel_roles = {
        str(transaction_id): str(role)
        for transaction_id, role in (channel_roles or {}).items()
    }
    channel_role_quarantines = tuple(
        {
            "transaction_id": transaction_id,
            "workspace_id": _field(rows_by_id.get(transaction_id, {}), "workspace_id"),
            "profile_id": _field(rows_by_id.get(transaction_id, {}), "profile_id"),
            "reason": (
                "channel_open_unresolved"
                if role == CHANNEL_OPEN_MISMATCH
                else "channel_close_unresolved"
            ),
            "detail_json": json.dumps(
                {
                    "channel_role": role,
                    "required_for": "complete_channel_lifecycle",
                },
                sort_keys=True,
            ),
        }
        for transaction_id, role in sorted(channel_roles.items())
        if role in {CHANNEL_OPEN_MISMATCH, CHANNEL_CLOSE_MISMATCH}
    )
    channel_blocked_ids = {
        str(item["transaction_id"]) for item in channel_role_quarantines
    }
    whole_payout_source_ids: set[str] = set()
    partial_payout_source_ids: set[str] = set()
    invalid_payout_source_ids: set[str] = set()
    direct_payout_quarantines: list[Mapping[str, Any]] = []
    for record in direct_payout_records:
        out_id = str(_field(record, "out_transaction_id") or "")
        source_row = rows_by_id.get(out_id)
        if source_row is None:
            continue
        source_amount = int(_field(source_row, "amount") or 0)
        reviewed_amount = _field(record, "out_amount")
        reviewed_amount = (
            source_amount if reviewed_amount in (None, "") else int(reviewed_amount)
        )
        if reviewed_amount <= 0 or reviewed_amount > source_amount:
            invalid_payout_source_ids.add(out_id)
            direct_payout_quarantines.append(
                {
                    "transaction_id": out_id,
                    "workspace_id": _field(source_row, "workspace_id"),
                    "profile_id": _field(source_row, "profile_id"),
                    "reason": "direct_payout_out_amount_invalid",
                    "detail_json": json.dumps(
                        {
                            "payout_id": _field(record, "id"),
                            "out_amount_msat": reviewed_amount,
                            "full_out_amount_msat": source_amount,
                        },
                        sort_keys=True,
                    ),
                }
            )
            continue
        if source_amount > 0 and reviewed_amount == source_amount:
            whole_payout_source_ids.add(out_id)
        elif 0 < reviewed_amount < source_amount:
            partial_payout_source_ids.add(out_id)
    direct_payout_conflict_ids = {
        _anchor_id(_field(pair, "in", {}) or {})
        for pair in auto_pairs
        if _anchor_id(_field(pair, "out", {}) or {}) in whole_payout_source_ids
    }
    direct_payout_conflict_quarantines = tuple(
        {
            "transaction_id": transaction_id,
            "workspace_id": _field(rows_by_id.get(transaction_id, {}), "workspace_id"),
            "profile_id": _field(rows_by_id.get(transaction_id, {}), "profile_id"),
            "reason": "direct_payout_conflicting_receipt",
            "detail_json": json.dumps(
                {"required_for": "direct_payout_review"}, sort_keys=True
            ),
        }
        for transaction_id in sorted(direct_payout_conflict_ids)
    )
    auto_pairs = [
        pair
        for pair in auto_pairs
        if _anchor_id(_field(pair, "out", {}) or {}) not in whole_payout_source_ids
        and _anchor_id(_field(pair, "in", {}) or {}) not in whole_payout_source_ids
    ]
    auto_pairs = [
        {
            **dict(pair),
            **(
                {"allow_unclaimed_residual": True}
                if _anchor_id(_field(pair, "out", {}) or {})
                in partial_payout_source_ids
                else {}
            ),
        }
        for pair in auto_pairs
    ]
    same_asset_pairs, cross_asset_pairs = apply_manual_pairs(
        rows, auto_pairs, manual_pair_records
    )
    partial_manual_sources = {
        str(_field(record, "out_transaction_id") or "")
        for record in manual_pair_records
        if _field(record, "out_amount") not in (None, "")
        and int(_field(record, "out_amount"))
        < int(
            _field(
                rows_by_id.get(
                    str(_field(record, "out_transaction_id") or ""), {}
                ),
                "amount",
                0,
            )
            or 0
        )
    }
    manual_targets = {
        str(_field(record, "in_transaction_id") or "")
        for record in manual_pair_records
    }
    existing_pair_anchors = {
        (_anchor_id(_field(pair, "out", {}) or {}), _anchor_id(_field(pair, "in", {}) or {}))
        for pair in same_asset_pairs
    }
    same_asset_pairs.extend(
        pair
        for pair in auto_pairs
        if _anchor_id(_field(pair, "out", {}) or {}) in partial_manual_sources
        and _anchor_id(_field(pair, "in", {}) or {}) not in manual_targets
        and (
            _anchor_id(_field(pair, "out", {}) or {}),
            _anchor_id(_field(pair, "in", {}) or {}),
        )
        not in existing_pair_anchors
    )
    reviewed_cross_pairs = [
        {
            **dict(pair),
            "out": rows_by_id.get(str(_field(pair, "out_id") or ""), {}),
            "in": rows_by_id.get(str(_field(pair, "in_id") or ""), {}),
            "source": "manual",
        }
        for pair in cross_asset_pairs
    ]
    # Exact transaction-graph ownership gets the first opportunity to explain
    # an automatic row match. A 1:N spend may look like a valid A->B pair until
    # the graph reveals the sibling C output; feeding every auto pair into the
    # handled set first would hide that stronger evidence.
    paired_ids: set[str] = set()
    paired_ids.update(samourai_touched_ids)
    paired_ids.update(whole_payout_source_ids)
    paired_ids.update(direct_payout_conflict_ids)
    paired_ids.update(
        str(_field(record, key) or "")
        for record in manual_pair_records
        for key in ("out_transaction_id", "in_transaction_id")
    )
    derivation = derive_profile_transfers(
        rows,
        index=owned_index,
        wallet_refs_by_id=wallet_refs_by_id,
        already_paired_ids=paired_ids,
    )
    ownership_pairs = []
    for pair in derivation.ownership.derived_pairs:
        item = dict(pair)
        source_anchor_id = _anchor_id(_field(item, "out", {}) or {})
        source_override = derivation.ownership.out_row_overrides.get(
            source_anchor_id
        )
        if source_override is not None:
            item.setdefault("group_id", f"ownership-event:{source_anchor_id}")
            # The override is the graph-proven external remainder. The MOVE
            # consumes only its owned slice; the baseline fallback must remain
            # eligible to classify the unclaimed source slice independently.
            item.setdefault("allow_unclaimed_residual", True)
        ownership_pairs.append(item)
    derived_pairs = [
        *derivation.consolidation.derived_pairs,
        *ownership_pairs,
        *derivation.fanout.derived_pairs,
        *channel_transfer_pairs,
    ]
    derivation_blocks = (
        *derivation.consolidation.blocked_sources,
        *derivation.ownership.blocked_sources,
        *derivation.fanout.blocked_sources,
    )
    event_transaction_ids_by_member = {
        transaction_id: tuple(
            sorted(item_id for item_id, _quantity_hash in event.observation_aliases)
        )
        for event in canonical.events
        for transaction_id, _quantity_hash in event.observation_aliases
    }
    derivation_quarantines_by_key: dict[tuple[str, str], Mapping[str, Any]] = {}
    for blocked in derivation_blocks:
        blocked_row = _field(blocked, "row") or {}
        blocked_id = _row_id(blocked_row)
        reason = str(
            _field(blocked, "reason") or "ownership_transfer_unresolved"
        )
        detail = dict(_field(blocked, "detail") or {})
        group_ids = event_transaction_ids_by_member.get(blocked_id, (blocked_id,))
        detail.setdefault("atomic_event_transaction_ids", list(group_ids))
        for transaction_id in group_ids:
            row = rows_by_id.get(transaction_id, blocked_row)
            derivation_quarantines_by_key[(transaction_id, reason)] = {
                "transaction_id": transaction_id,
                "workspace_id": _field(row, "workspace_id"),
                "profile_id": _field(row, "profile_id"),
                "reason": reason,
                "detail_json": json.dumps(detail, sort_keys=True),
            }
    derivation_quarantines = tuple(
        derivation_quarantines_by_key[key]
        for key in sorted(derivation_quarantines_by_key)
    )
    derivation_blocked_ids = {
        str(_field(item, "transaction_id") or "")
        for item in derivation_quarantines
        if _field(item, "transaction_id") not in (None, "")
    }
    derivation_touched_ids = set(derivation_blocked_ids)
    derivation_touched_ids.update(
        transaction_id
        for pair in derived_pairs
        for transaction_id in (
            _anchor_id(_field(pair, "out", {}) or {}),
            _anchor_id(_field(pair, "in", {}) or {}),
        )
        if transaction_id
    )
    same_asset_pairs = [
        pair
        for pair in same_asset_pairs
        if _pair_is_reviewed(pair)
        or not {
            _anchor_id(_field(pair, "out", {}) or {}),
            _anchor_id(_field(pair, "in", {}) or {}),
        }
        & derivation_touched_ids
    ]
    claims, blocked_transaction_ids, pair_quarantines = _pair_claims(
        [
            *same_asset_pairs,
            *samourai_pairs,
            *reviewed_cross_pairs,
            *derived_pairs,
        ],
        observations,
        excluded_transaction_ids=excluded,
        wallet_refs_by_id=wallet_refs_by_id,
    )
    claims = tuple(
        (
            *claims,
            *samourai_fee_claims,
        )
    )
    native_audits = _native_audits_for_rowless_pairs(
        derived_pairs,
        observations,
        excluded_transaction_ids=excluded,
    )
    # Loan rows are non-taxable custody roles.  They are deliberately surfaced
    # to the finalized projection instead of being hidden in RP2's old role
    # suppression; a future location-specific loan interpreter can add a
    # virtual custody target without changing the tax boundary.
    non_event_ids = tuple(
        sorted(
            {
                str(_field(leg, "transaction_id") or "")
                for leg in loan_legs
                if _field(leg, "transaction_id") not in (None, "")
            }
            | {
                transaction_id
                for transaction_id, role in channel_roles.items()
                if role in {CHANNEL_OPEN, CHANNEL_CLOSE}
            }
        )
    )
    return CustodyInterpreterCompilation(
        claims=claims,
        native_audits=native_audits,
        cross_asset_pairs=tuple(cross_asset_pairs),
        non_event_transaction_ids=non_event_ids,
        blocked_transaction_ids=tuple(
            sorted(
                {
                    *blocked_transaction_ids,
                    *channel_blocked_ids,
                    *direct_payout_conflict_ids,
                    *invalid_payout_source_ids,
                    *derivation_blocked_ids,
                    *privacy_blocked_ids,
                    *samourai_unverified_ids,
                }
            )
        ),
        direct_payout_conflict_transaction_ids=tuple(
            sorted(direct_payout_conflict_ids)
        ),
        quarantines=tuple(
            (
                *derivation_quarantines,
                *pair_quarantines,
                *channel_role_quarantines,
                *direct_payout_conflict_quarantines,
                *direct_payout_quarantines,
                *privacy_quarantines,
                *samourai_unverified_quarantines,
            )
        ),
    )


__all__ = ["CustodyInterpreterCompilation", "compile_custody_interpreters"]
