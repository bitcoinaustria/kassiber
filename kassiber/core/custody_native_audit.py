"""Compile graph-proven custody moves into canonical quantity evidence.

Ownership derivation can safely recognize an owned on-chain output before that
destination wallet has imported its own transaction row.  RP2 still receives a
synthetic inbound leg so it can carry basis, but the canonical quantity model
must not invent a free-standing receipt.  This adapter derives one aggregate
inbound observation from the already-accepted native-policy audit and anchors
it to the source transaction's physical event identity.

Only the two graph interpreters that prove wallet ownership may create such an
observation.  Labels, amounts, generic transfer pairing, and coverage guesses
are deliberately insufficient.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
import hashlib
import json
from typing import Any, Iterable, Mapping, Sequence

from .custody_evidence import (
    CanonicalQuantityInput,
    EvidenceSnapshot,
    QuantityObservation,
)
from .custody_quantity import (
    CUSTODY_SUSPENSE,
    ClaimPriority,
    INTERNAL_VERIFIED,
    QuantityClaim,
    QuantitySlice,
)


VERIFIED_NATIVE_PAIRING_SOURCES = frozenset(
    {
        "ownership_derived",
        "multi_source_consolidation",
        "recorded_fanout",
        "channel_lifecycle",
    }
)


@dataclass(frozen=True)
class NativeAuditIssue:
    issue_id: str
    reason: str
    occurred_at: str
    transaction_ids: tuple[str, ...]
    asset: str | None
    amount_msat: int | None
    details: Mapping[str, Any]


@dataclass(frozen=True)
class NativeAuditCompilation:
    canonical_input: CanonicalQuantityInput
    claims: tuple[QuantityClaim, ...]
    issues: tuple[NativeAuditIssue, ...]


@dataclass(frozen=True)
class _NativeLeg:
    evidence_id: str
    out_id: str
    in_id: str
    source_transaction_id: str
    source: QuantityObservation
    target_wallet_id: str
    asset: str
    received_msat: int
    fee_msat: int
    sent_msat: int
    pairing_source: str
    occurred_at: str


def _hash_payload(value: Any) -> str:
    encoded = json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _issue(
    reason: str,
    *,
    audit: Mapping[str, Any],
    transaction_ids: Iterable[str],
    asset: str | None = None,
    amount_msat: int | None = None,
    **details: Any,
) -> NativeAuditIssue:
    ids = tuple(sorted({str(item) for item in transaction_ids if item}))
    issue_id = _hash_payload(
        {
            "schema_version": 1,
            "kind": "native_audit_issue",
            "reason": reason,
            "transaction_ids": ids,
            "pairing_source": str(audit.get("pairing_source") or ""),
        }
    )
    return NativeAuditIssue(
        issue_id=issue_id,
        reason=reason,
        occurred_at=str(audit.get("occurred_at") or ""),
        transaction_ids=ids,
        asset=asset,
        amount_msat=amount_msat,
        details=details,
    )


def _exact_msat(audit: Mapping[str, Any], field: str) -> int | None:
    value = audit.get(field)
    if type(value) is not int or value < 0:
        return None
    return value


def _observations_by_transaction(
    canonical: CanonicalQuantityInput,
) -> dict[str, QuantityObservation]:
    observations = {
        observation.quantity_hash: observation
        for observation in canonical.observations
    }
    return {
        transaction_id: observations[quantity_hash]
        for event in canonical.events
        for transaction_id, quantity_hash in event.observation_aliases
    }


def _augment_virtual_channel_sources(
    canonical: CanonicalQuantityInput,
    intra_audit: Sequence[Mapping[str, Any]],
) -> tuple[CanonicalQuantityInput, list[NativeAuditIssue]]:
    """Materialize the node-side source of a verified channel close MOVE."""

    events = {event.event_key: event for event in canonical.events}
    by_transaction = _observations_by_transaction(canonical)
    issues: list[NativeAuditIssue] = []
    for audit in sorted(
        (
            item
            for item in intra_audit
            if item.get("virtual_source") is True
            and item.get("pairing_source") == "channel_lifecycle"
        ),
        key=lambda item: (
            str(item.get("occurred_at") or ""),
            str(item.get("out_id") or ""),
        ),
    ):
        out_id = str(audit.get("out_id") or "")
        target_id = str(audit.get("in_anchor_transaction_id") or "")
        target = by_transaction.get(target_id)
        received_msat = _exact_msat(audit, "crypto_received_msat")
        fee_msat = _exact_msat(audit, "crypto_fee_msat")
        sent_msat = _exact_msat(audit, "crypto_sent_msat")
        from_wallet_id = str(audit.get("from_wallet_id") or "")
        if (
            not out_id
            or target is None
            or target.direction != "inbound"
            or not from_wallet_id
            or received_msat is None
            or received_msat <= 0
            or fee_msat is None
            or sent_msat != received_msat + fee_msat
            or str(audit.get("asset") or "").upper() != target.asset
            or str(audit.get("to_wallet_id") or "") != target.wallet_id
            or str(audit.get("occurred_at") or "") != target.occurred_at
        ):
            issues.append(
                _issue(
                    "native_audit_virtual_source_invalid",
                    audit=audit,
                    transaction_ids=(out_id, target_id),
                    asset=(target.asset if target is not None else None),
                    amount_msat=received_msat,
                )
            )
            continue
        event = events[target.event_key]
        aliases = dict(event.observation_aliases)
        prior_hash = aliases.get(out_id)
        if prior_hash is not None:
            prior = next(
                (
                    observation
                    for observation in event.legs
                    if observation.quantity_hash == prior_hash
                ),
                None,
            )
            if (
                prior is not None
                and prior.direction == "outbound"
                and prior.wallet_id == from_wallet_id
                and prior.asset == target.asset
                and prior.principal_msat == received_msat
                and prior.fee_msat == fee_msat
            ):
                continue
            issues.append(
                _issue(
                    "native_audit_virtual_source_contradiction",
                    audit=audit,
                    transaction_ids=(out_id, target_id),
                    asset=target.asset,
                    amount_msat=received_msat,
                )
            )
            continue
        proof = {
            "schema_version": 1,
            "kind": "verified_channel_close_source",
            "source_wallet_id": from_wallet_id,
            "target_quantity_hash": target.quantity_hash,
            "received_msat": received_msat,
            "fee_msat": fee_msat,
        }
        proof_hash = _hash_payload(proof)
        source = QuantityObservation.from_transaction(
            {
                "id": out_id,
                "journal_transaction_id": target.anchor_transaction_id,
                "wallet_id": from_wallet_id,
                "direction": "outbound",
                "asset": target.asset,
                "amount": received_msat,
                "fee": fee_msat,
                "amount_includes_fee": False,
                "occurred_at": target.occurred_at,
                "fingerprint": f"native-channel-close-out:{proof_hash}",
                "kind": "self_transfer_out",
                "raw_json": {
                    "txid": target.event_key.native_event_id,
                    "_kassiber_quantity_evidence": proof,
                },
            },
            target.event_key,
        )
        aliases[out_id] = source.quantity_hash
        snapshot = EvidenceSnapshot(
            source.quantity_hash,
            source.evidence_detail_hash,
            source.evidence_payload_json,
        )
        events[target.event_key] = replace(
            event,
            legs=tuple(
                sorted(
                    (*event.legs, source),
                    key=lambda item: (
                        item.wallet_id,
                        item.direction,
                        item.asset,
                        item.quantity_hash,
                    ),
                )
            ),
            evidence_snapshots=tuple(
                sorted(
                    (*event.evidence_snapshots, snapshot),
                    key=lambda item: (item.detail_hash, item.quantity_hash),
                )
            ),
            observation_aliases=tuple(sorted(aliases.items())),
        )
        by_transaction[out_id] = source
    return (
        CanonicalQuantityInput(
            events=tuple(events[key] for key in sorted(events)),
            rejected_events=canonical.rejected_events,
        ),
        issues,
    )
def _parse_legs(
    canonical: CanonicalQuantityInput,
    intra_audit: Sequence[Mapping[str, Any]],
    component_transaction_ids: set[str],
) -> tuple[list[_NativeLeg], list[NativeAuditIssue]]:
    by_transaction = _observations_by_transaction(canonical)
    legs: list[_NativeLeg] = []
    issues: list[NativeAuditIssue] = []
    ordered = sorted(
        (
            audit
            for audit in intra_audit
            if str(audit.get("pairing_source") or "")
            in VERIFIED_NATIVE_PAIRING_SOURCES
        ),
        key=lambda audit: (
            str(audit.get("occurred_at") or ""),
            str(audit.get("out_id") or ""),
            str(audit.get("in_id") or ""),
        ),
    )
    for audit in ordered:
        out_id = str(audit.get("out_id") or "")
        in_id = str(audit.get("in_id") or "")
        source_id = str(audit.get("out_anchor_transaction_id") or "")
        if source_id in component_transaction_ids:
            continue
        source = by_transaction.get(source_id)
        ids = (source_id, out_id, in_id)
        if not source_id or source is None or source.direction != "outbound":
            issues.append(
                _issue(
                    "native_audit_source_anchor_missing",
                    audit=audit,
                    transaction_ids=ids,
                    asset=str(audit.get("asset") or "").upper() or None,
                )
            )
            continue
        if (
            str(audit.get("pairing_source") or "") != "channel_lifecycle"
            and not source.authoritative_chain_observation
        ):
            issues.append(
                _issue(
                    "native_audit_authoritative_observation_missing",
                    audit=audit,
                    transaction_ids=ids,
                    asset=source.asset,
                    authority_reason=source.observation_authority_reason,
                )
            )
            continue
        event_key = source.event_key
        physical_txid = event_key.native_event_id
        if (
            event_key.chain not in {"bitcoin", "liquid"}
            or event_key.native_namespace != "chain"
            or len(physical_txid) != 64
            or any(char not in "0123456789abcdef" for char in physical_txid)
        ):
            issues.append(
                _issue(
                    "native_audit_physical_scope_invalid",
                    audit=audit,
                    transaction_ids=ids,
                    asset=source.asset,
                    chain=event_key.chain,
                    network=event_key.network,
                    native_namespace=event_key.native_namespace,
                )
            )
            continue
        asset = str(audit.get("asset") or "").upper()
        from_wallet_id = str(audit.get("from_wallet_id") or "")
        target_wallet_id = str(audit.get("to_wallet_id") or "")
        received_msat = _exact_msat(audit, "crypto_received_msat")
        fee_msat = _exact_msat(audit, "crypto_fee_msat")
        sent_msat = _exact_msat(audit, "crypto_sent_msat")
        if (
            not out_id
            or not in_id
            or not target_wallet_id
            or from_wallet_id != source.wallet_id
            or asset != source.asset
            or received_msat is None
            or fee_msat is None
            or sent_msat is None
            or received_msat <= 0
            or sent_msat != received_msat + fee_msat
            or str(audit.get("occurred_at") or "") != source.occurred_at
        ):
            issues.append(
                _issue(
                    "native_audit_leg_invalid",
                    audit=audit,
                    transaction_ids=ids,
                    asset=source.asset,
                    amount_msat=received_msat,
                    source_wallet_id=source.wallet_id,
                    audit_from_wallet_id=from_wallet_id,
                    target_wallet_id=target_wallet_id,
                    source_asset=source.asset,
                    audit_asset=asset,
                    exact_msat_fields_present=all(
                        value is not None
                        for value in (received_msat, fee_msat, sent_msat)
                    ),
                )
            )
            continue
        evidence_id = _hash_payload(
            {
                "schema_version": 1,
                "kind": "verified_native_intra_leg",
                "pairing_source": str(audit["pairing_source"]),
                "source_quantity_hash": source.quantity_hash,
                "out_id": out_id,
                "in_id": in_id,
                "target_wallet_id": target_wallet_id,
                "asset": asset,
                "received_msat": received_msat,
                "fee_msat": fee_msat,
            }
        )
        legs.append(
            _NativeLeg(
                evidence_id=evidence_id,
                out_id=out_id,
                in_id=in_id,
                source_transaction_id=source_id,
                source=source,
                target_wallet_id=target_wallet_id,
                asset=asset,
                received_msat=received_msat,
                fee_msat=fee_msat,
                sent_msat=sent_msat,
                pairing_source=str(audit["pairing_source"]),
                occurred_at=source.occurred_at,
            )
        )
    return legs, issues


def _validated_source_groups(
    legs: Sequence[_NativeLeg],
    reserved_source_msat: Mapping[str, int],
) -> tuple[list[_NativeLeg], list[NativeAuditIssue]]:
    by_source: dict[str, list[_NativeLeg]] = {}
    for leg in legs:
        by_source.setdefault(leg.source.quantity_hash, []).append(leg)
    valid: list[_NativeLeg] = []
    issues: list[NativeAuditIssue] = []
    for source_hash, members in sorted(by_source.items()):
        members = sorted(members, key=lambda item: item.evidence_id)
        source = members[0].source
        received = sum(item.received_msat for item in members)
        audited_fee = sum(item.fee_msat for item in members)
        reserved = int(reserved_source_msat.get(source_hash, 0))
        residual = source.principal_msat - reserved - received
        valid_group = (
            0 <= reserved <= source.principal_msat
            and residual >= 0
            and audited_fee == source.fee_msat
            and (
                residual == 0
                or all(
                    item.pairing_source == "ownership_derived"
                    for item in members
                )
            )
        )
        if valid_group:
            valid.extend(members)
            continue
        audit = {
            "pairing_source": members[0].pairing_source,
            "occurred_at": source.occurred_at,
        }
        issues.append(
            _issue(
                "native_audit_source_conservation_invalid",
                audit=audit,
                transaction_ids=(
                    source.transaction_id,
                    *(item.out_id for item in members),
                    *(item.in_id for item in members),
                ),
                asset=source.asset,
                amount_msat=source.principal_msat,
                source_quantity_hash=source_hash,
                source_principal_msat=source.principal_msat,
                source_fee_msat=source.fee_msat,
                received_msat=received,
                audited_fee_msat=audited_fee,
                reserved_source_msat=reserved,
                residual_msat=residual,
            )
        )
    return valid, issues


def _augment_targets(
    canonical: CanonicalQuantityInput,
    legs: Sequence[_NativeLeg],
) -> tuple[
    CanonicalQuantityInput,
    dict[str, str],
    list[NativeAuditIssue],
    set[str],
]:
    by_slot: dict[tuple[Any, str, str], list[_NativeLeg]] = {}
    for leg in legs:
        by_slot.setdefault(
            (leg.source.event_key, leg.target_wallet_id, leg.asset), []
        ).append(leg)

    target_hash_by_evidence: dict[str, str] = {}
    invalid_evidence: set[str] = set()
    issues: list[NativeAuditIssue] = []
    events = {event.event_key: event for event in canonical.events}
    slot_items = sorted(
        by_slot.items(),
        key=lambda item: (item[0][0], item[0][1], item[0][2]),
    )

    # Validate every target slot before materializing any synthetic inbound.
    # A contradiction invalidates the whole physical source interpretation;
    # preflighting avoids leaving an earlier sibling target behind as an
    # uncovered inbound/external-origin posting.
    invalid_slots: set[tuple[Any, str, str]] = set()
    alias_owners: dict[Any, dict[str, tuple[Any, str, str] | None]] = {
        event.event_key: {
            transaction_id: None
            for transaction_id, _quantity_hash in event.observation_aliases
        }
        for event in canonical.events
    }
    original_aliases = {
        event.event_key: dict(event.observation_aliases)
        for event in canonical.events
    }
    for slot, raw_members in slot_items:
        event_key, wallet_id, asset = slot
        members = sorted(raw_members, key=lambda item: item.evidence_id)
        event = events[event_key]
        amount_msat = sum(item.received_msat for item in members)
        existing = [
            observation
            for observation in event.legs
            if observation.direction == "inbound"
            and observation.wallet_id == wallet_id
            and observation.asset == asset
        ]
        if existing and (
            len(existing) != 1 or existing[0].principal_msat != amount_msat
        ):
            invalid_slots.add(slot)
            issues.append(
                _issue(
                    "native_audit_target_contradiction",
                    audit={
                        "pairing_source": members[0].pairing_source,
                        "occurred_at": members[0].occurred_at,
                    },
                    transaction_ids=(
                        *(item.source_transaction_id for item in members),
                        *(item.in_id for item in members),
                    ),
                    asset=asset,
                    amount_msat=amount_msat,
                    wallet_id=wallet_id,
                    observed_target_amounts_msat=[
                        item.principal_msat for item in existing
                    ],
                    chain=event_key.chain,
                    network=event_key.network,
                )
            )
            continue
        if existing:
            continue

        conflicting_aliases: list[dict[str, Any]] = []
        owners = alias_owners[event_key]
        aliases = original_aliases[event_key]
        for member in members:
            if member.in_id not in owners:
                continue
            owner_slot = owners[member.in_id]
            if owner_slot == slot:
                continue
            detail: dict[str, Any] = {"transaction_id": member.in_id}
            if owner_slot is None:
                detail["quantity_hash"] = aliases[member.in_id]
            else:
                detail.update(
                    {
                        "target_wallet_id": owner_slot[1],
                        "asset": owner_slot[2],
                    }
                )
                invalid_slots.add(owner_slot)
            conflicting_aliases.append(detail)
        if conflicting_aliases:
            invalid_slots.add(slot)
            issues.append(
                _issue(
                    "native_audit_alias_contradiction",
                    audit={
                        "pairing_source": members[0].pairing_source,
                        "occurred_at": members[0].occurred_at,
                    },
                    transaction_ids=(
                        *(item.source_transaction_id for item in members),
                        *(item.in_id for item in members),
                    ),
                    asset=asset,
                    amount_msat=amount_msat,
                    wallet_id=wallet_id,
                    conflicting_aliases=conflicting_aliases,
                    chain=event_key.chain,
                    network=event_key.network,
                )
            )
            continue
        for member in members:
            owners[member.in_id] = slot

    invalid_source_hashes = {
        member.source.quantity_hash
        for slot, members in slot_items
        if slot in invalid_slots
        for member in members
    }
    # Source interpretations and aggregate destination slots form a bipartite
    # graph.  Close invalidity over the connected component so a contradicted
    # source cannot leave a sibling target behind, and an aggregate target
    # cannot retain only the non-contradicted share of an inseparable amount.
    changed = True
    while changed:
        changed = False
        for slot, members in slot_items:
            if slot in invalid_slots:
                continue
            if any(
                member.source.quantity_hash in invalid_source_hashes
                for member in members
            ):
                invalid_slots.add(slot)
                invalid_source_hashes.update(
                    member.source.quantity_hash for member in members
                )
                changed = True
    invalid_evidence.update(
        member.evidence_id
        for slot, members in slot_items
        if slot in invalid_slots
        for member in members
    )

    for (event_key, wallet_id, asset), raw_members in slot_items:
        if (event_key, wallet_id, asset) in invalid_slots:
            continue
        members = sorted(raw_members, key=lambda item: item.evidence_id)
        event = events[event_key]
        amount_msat = sum(item.received_msat for item in members)
        existing = [
            observation
            for observation in event.legs
            if observation.direction == "inbound"
            and observation.wallet_id == wallet_id
            and observation.asset == asset
        ]
        if existing:
            target = existing[0]
        else:
            aliases = dict(event.observation_aliases)
            proof = {
                "schema_version": 1,
                "kind": "verified_native_owned_destination",
                "event": {
                    "chain": event_key.chain,
                    "network": event_key.network,
                    "native_namespace": event_key.native_namespace,
                    "native_event_id": event_key.native_event_id,
                },
                "wallet_id": wallet_id,
                "asset": asset,
                "amount_msat": amount_msat,
                "legs": [
                    {
                        "evidence_id": item.evidence_id,
                        "pairing_source": item.pairing_source,
                        "source_quantity_hash": item.source.quantity_hash,
                        "received_msat": item.received_msat,
                    }
                    for item in members
                ],
            }
            proof_hash = _hash_payload(proof)
            target = QuantityObservation.from_transaction(
                {
                    "id": f"native-owned-in:{proof_hash}",
                    "journal_transaction_id": members[0].source_transaction_id,
                    "wallet_id": wallet_id,
                    "direction": "inbound",
                    "asset": asset,
                    "amount": amount_msat,
                    "fee": 0,
                    "amount_includes_fee": False,
                    "occurred_at": min(item.occurred_at for item in members),
                    "fingerprint": f"native-owned-in:{proof_hash}",
                    "kind": "self_transfer_in",
                    "raw_json": {
                        "txid": event_key.native_event_id,
                        "_kassiber_quantity_evidence": proof,
                    },
                },
                event_key,
            )
            snapshot = EvidenceSnapshot(
                target.quantity_hash,
                target.evidence_detail_hash,
                target.evidence_payload_json,
            )
            for member in members:
                aliases[member.in_id] = target.quantity_hash
            event = replace(
                event,
                legs=tuple(
                    sorted(
                        (*event.legs, target),
                        key=lambda item: (
                            item.wallet_id,
                            item.direction,
                            item.asset,
                            item.quantity_hash,
                        ),
                    )
                ),
                evidence_snapshots=tuple(
                    sorted(
                        (*event.evidence_snapshots, snapshot),
                        key=lambda item: (item.detail_hash, item.quantity_hash),
                    )
                ),
                observation_aliases=tuple(sorted(aliases.items())),
            )
            events[event_key] = event
        for member in members:
            target_hash_by_evidence[member.evidence_id] = target.quantity_hash

    augmented = CanonicalQuantityInput(
        events=tuple(events[key] for key in sorted(events)),
        rejected_events=canonical.rejected_events,
    )
    return augmented, target_hash_by_evidence, issues, invalid_evidence


def compile_verified_native_claims(
    canonical: CanonicalQuantityInput,
    intra_audit: Sequence[Mapping[str, Any]],
    *,
    component_transaction_ids: Iterable[str] = (),
    reserved_source_msat: Mapping[str, int] | None = None,
) -> NativeAuditCompilation:
    """Return canonical targets and exact claims for pre-tax native evidence."""

    reserved = dict(reserved_source_msat or {})
    canonical, virtual_source_issues = _augment_virtual_channel_sources(
        canonical,
        intra_audit,
    )
    legs, issues = _parse_legs(
        canonical,
        intra_audit,
        {str(item) for item in component_transaction_ids if item},
    )
    issues[:0] = virtual_source_issues
    legs, conservation_issues = _validated_source_groups(legs, reserved)
    issues.extend(conservation_issues)
    augmented, target_by_evidence, target_issues, invalid = _augment_targets(
        canonical,
        legs,
    )
    issues.extend(target_issues)
    # One physical source interpretation is atomic.  If any owned destination
    # contradicts real evidence, do not retain its siblings and then relabel
    # the rejected slice as a verified external payment.
    invalid_source_hashes = {
        item.source.quantity_hash
        for item in legs
        if item.evidence_id in invalid
    }
    valid_legs = [
        item
        for item in legs
        if item.evidence_id not in invalid
        and item.source.quantity_hash not in invalid_source_hashes
    ]
    observations = {
        item.quantity_hash: item for item in augmented.observations
    }
    source_cursors: dict[str, int] = {}
    target_cursors: dict[str, int] = {}
    claims: list[QuantityClaim] = []
    by_source: dict[str, list[_NativeLeg]] = {}
    for leg in valid_legs:
        by_source.setdefault(leg.source.quantity_hash, []).append(leg)
    for source_hash, members in sorted(by_source.items()):
        members = sorted(members, key=lambda item: item.evidence_id)
        source = observations[source_hash]
        bundle_id = f"engine-native:{source_hash}"
        for leg in members:
            target_hash = target_by_evidence[leg.evidence_id]
            target = observations[target_hash]
            source_start = source_cursors.get(source_hash, 0)
            target_start = target_cursors.get(target_hash, 0)
            source_end = source_start + leg.received_msat
            target_end = target_start + leg.received_msat
            claims.append(
                QuantityClaim(
                    claim_id=f"engine-native:{leg.evidence_id}:retained",
                    source=QuantitySlice(source_hash, source_start, source_end),
                    target=QuantitySlice(target_hash, target_start, target_end),
                    state=INTERNAL_VERIFIED,
                    priority=ClaimPriority.EXACT_NATIVE_EVENT,
                    reason=leg.pairing_source,
                    atomic_bundle_id=bundle_id,
                )
            )
            source_cursors[source_hash] = source_end
            target_cursors[target_hash] = target_end

        available_end = source.principal_msat - int(reserved.get(source_hash, 0))
        residual_start = source_cursors.get(source_hash, 0)
        if (
            source.fee_attribution == "implicit_wallet_delta"
            and residual_start < available_end
        ):
            claims.append(
                QuantityClaim(
                    claim_id=f"engine-native:{source_hash}:implicit-residual",
                    source=QuantitySlice(source_hash, residual_start, available_end),
                    state=CUSTODY_SUSPENSE,
                    # The observer proves the residual wallet delta exactly;
                    # only its destination remains unknown. Keep the whole
                    # physical-source interpretation at one native-evidence
                    # rank so the atomic bundle can fail closed as a unit.
                    priority=ClaimPriority.EXACT_NATIVE_EVENT,
                    reason="implicit_wallet_delta_unallocated",
                    atomic_bundle_id=bundle_id,
                )
            )

    return NativeAuditCompilation(
        canonical_input=augmented,
        claims=tuple(sorted(claims, key=lambda item: item.claim_id)),
        issues=tuple(sorted(issues, key=lambda item: item.issue_id)),
    )


__all__ = [
    "NativeAuditCompilation",
    "NativeAuditIssue",
    "VERIFIED_NATIVE_PAIRING_SOURCES",
    "compile_verified_native_claims",
]
