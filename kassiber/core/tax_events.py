from __future__ import annotations

import json
from dataclasses import dataclass
from decimal import Decimal
from typing import Any, Literal, Mapping, Optional, Sequence

from ..msat import msat_to_btc
from . import pricing
from .austrian import infer_outbound_regimes, infer_regime_from_timestamp, resolve_pool_id
from .privacy_hops import privacy_hop_evidence_from_row

# Austrian tax-semantic markers carried on NormalizedTaxEvent / NormalizedTaxTransfer.
# The rp2 AT plugin reads these through the `notes` channel of InTransaction /
# OutTransaction; Kassiber serializes them at the rp2 adapter boundary (see
# kassiber/core/engines/rp2.py). Typed fields are the source of truth inside
# Kassiber — free-form description text is never parsed as protocol.
AtRegime = Literal["alt", "neu"]


@dataclass(frozen=True)
class NormalizedTaxEvent:
    transaction_id: str
    asset: str
    occurred_at: str
    wallet_id: str
    wallet_label: str
    direction: str
    amount: Decimal
    fee: Decimal
    spot_price: Decimal | None
    fiat_value: Decimal | None
    description: str
    raw_row: Mapping[str, Any]
    # Austrian regime classification. "alt" = Altvermögen (acquired on/before
    # 2021-02-28 Europe/Vienna, FIFO + 365-day Spekulationsfrist); "neu" =
    # Neuvermögen (acquired after the cutoff, moving-average pool). Populated
    # by Austrian classification in normalize_tax_asset_inputs when the
    # profile's tax_country is "at"; None for non-AT profiles or when rp2's
    # date-based inference should decide.
    at_regime: Optional[AtRegime] = None
    # Moving-average pool partition id (Neu only; ignored by rp2 for Alt).
    # Kassiber decides what a pool is — v1 uses wallet_id. None means
    # "absent marker", which rp2 treats as the `AT_DEFAULT_POOL` bucket.
    at_pool: Optional[str] = None
    # Non-empty id tagging one leg of a matched crypto-to-crypto swap.
    # On a Neu outgoing leg, rp2 emits a zero-gain GainLoss and depletes
    # the pool at its running average. None means "not a swap". Empty
    # string is invalid and would trigger rp2 RP2ValueError — the
    # normalization layer must synthesize a stable non-empty id when
    # tagging swap legs.
    at_swap_link: Optional[str] = None


@dataclass(frozen=True)
class NormalizedTaxTransfer:
    asset: str
    occurred_at: str
    out_transaction_id: str
    in_transaction_id: str
    from_wallet_id: str
    from_wallet_label: str
    to_wallet_id: str
    to_wallet_label: str
    sent: Decimal
    received: Decimal
    fee: Decimal
    spot_price: Decimal | None
    description: str
    external_id: str | None
    out_row: Mapping[str, Any]
    in_row: Mapping[str, Any]
    # Pool partition id to preserve across an intra-wallet move when
    # Kassiber models pools as per-wallet. Intra transfers don't have
    # a regime or swap-link concept; only the pool marker applies.
    at_pool: Optional[str] = None
    # Optional stable id for one logical movement split out of a multi-output
    # wallet transaction. Journal rows still point at the real out/in rows.
    transfer_id: Optional[str] = None


@dataclass(frozen=True)
class NormalizedTaxAssetInputs:
    asset: str
    events: Sequence[NormalizedTaxEvent]
    transfers: Sequence[NormalizedTaxTransfer]
    ordered_items: Sequence[tuple[str, str]]
    quarantines: Sequence[dict[str, Any]]


def build_tax_quarantine(
    profile: Mapping[str, Any],
    row: Mapping[str, Any],
    reason: str,
    detail: Mapping[str, Any],
) -> dict[str, Any]:
    return {
        "transaction_id": row["id"],
        "workspace_id": profile["workspace_id"],
        "profile_id": profile["id"],
        "reason": reason,
        "detail_json": json.dumps(detail, sort_keys=True),
    }


def _spot_price_from_row(row: Mapping[str, Any], quantity: Decimal) -> Decimal | None:
    rate = pricing.decimal_from_exact(
        _row_get(row, "fiat_rate_exact"),
        _row_get(row, "fiat_rate"),
    )
    if rate is not None:
        if rate > 0:
            return rate
    value = pricing.decimal_from_exact(
        _row_get(row, "fiat_value_exact"),
        _row_get(row, "fiat_value"),
    )
    if value is not None and quantity > 0:
        if value > 0:
            return value / quantity
    return None


def _row_get(row: Mapping[str, Any], key: str) -> Any:
    if hasattr(row, "keys") and key not in row.keys():
        return None
    if hasattr(row, "get"):
        return row.get(key)
    return row[key]


def _pricing_needs_review(row: Mapping[str, Any]) -> bool:
    return _row_get(row, "pricing_quality") == pricing.QUALITY_COARSE_FALLBACK


def _pricing_review_detail(row: Mapping[str, Any], wallet_label: str, asset: str, direction: str) -> dict[str, Any]:
    return {
        "wallet": wallet_label,
        "asset": asset,
        "direction": direction,
        "required_for": "pricing_review",
        "pricing_quality": _row_get(row, "pricing_quality"),
        "pricing_source_kind": _row_get(row, "pricing_source_kind"),
        "pricing_provider": _row_get(row, "pricing_provider"),
        "pricing_pair": _row_get(row, "pricing_pair"),
        "pricing_timestamp": _row_get(row, "pricing_timestamp"),
        "pricing_granularity": _row_get(row, "pricing_granularity"),
        "pricing_method": _row_get(row, "pricing_method"),
    }


def _privacy_hop_evidence(row: Mapping[str, Any]) -> dict[str, Any] | None:
    return privacy_hop_evidence_from_row(row)


def _append_privacy_hop_quarantine(
    quarantines: list[dict[str, Any]],
    profile: Mapping[str, Any],
    row: Mapping[str, Any],
    wallet: Mapping[str, Any],
    asset: str,
    direction: str,
    evidence: Mapping[str, Any],
) -> None:
    quarantines.append(
        build_tax_quarantine(
            profile,
            row,
            "privacy_hop_unresolved",
            {
                "wallet": wallet["label"],
                "asset": asset,
                "direction": direction,
                **evidence,
            },
        )
    )


def _samourai_metadata(row: Mapping[str, Any]) -> dict[str, Any] | None:
    config_json = _row_get(row, "config_json")
    if not config_json:
        return None
    try:
        config = json.loads(config_json)
    except (TypeError, ValueError, json.JSONDecodeError):
        return None
    if not isinstance(config, dict):
        return None
    metadata = config.get("samourai")
    if not isinstance(metadata, dict):
        return None
    if metadata.get("role") != "child":
        return None
    group_id = str(metadata.get("group_id") or "").strip()
    section = str(metadata.get("section") or "").strip().lower()
    if not group_id or not section:
        return None
    return {"group_id": group_id, "section": section}


SamouraiGroupEntry = tuple[Mapping[str, Any], dict[str, Any]]


def _samourai_internal_privacy_groups(
    rows: Sequence[Mapping[str, Any]],
) -> list[list[SamouraiGroupEntry]]:
    grouped: dict[tuple[str, str], list[tuple[Mapping[str, Any], dict[str, Any]]]] = {}
    for row in rows:
        metadata = _samourai_metadata(row)
        external_id = str(_row_get(row, "external_id") or "").strip().lower()
        if metadata is None or not external_id:
            continue
        grouped.setdefault((metadata["group_id"], external_id), []).append((row, metadata))

    internal_groups: list[list[SamouraiGroupEntry]] = []
    for _, entries in grouped.items():
        if len(entries) < 2:
            continue
        outbound_sections = {
            metadata["section"]
            for row, metadata in entries
            if _row_get(row, "direction") == "outbound"
        }
        inbound_sections = {
            metadata["section"]
            for row, metadata in entries
            if _row_get(row, "direction") == "inbound"
        }
        if not outbound_sections or not inbound_sections:
            continue
        is_tx0 = "deposit" in outbound_sections and bool(
            inbound_sections & {"premix", "badbank"}
        )
        is_first_mix = "premix" in outbound_sections and "postmix" in inbound_sections
        is_remix = "postmix" in outbound_sections and "postmix" in inbound_sections
        is_whirlpool_cycle = bool(outbound_sections & {"premix", "postmix"}) and bool(
            inbound_sections & {"premix", "postmix"}
        )
        if is_tx0 or is_first_mix or is_remix or is_whirlpool_cycle:
            internal_groups.append(entries)
    return internal_groups


def _samourai_internal_privacy_row_ids(
    groups: Sequence[Sequence[SamouraiGroupEntry]],
) -> set[str]:
    return {str(row["id"]) for group in groups for row, _ in group}


def _collect_samourai_internal_transfers(
    profile: Mapping[str, Any],
    asset: str,
    groups: Sequence[Sequence[SamouraiGroupEntry]],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    is_at: bool,
    quarantines: list[dict[str, Any]],
) -> tuple[dict[str, list[NormalizedTaxTransfer]], dict[str, NormalizedTaxEvent]]:
    collected: dict[str, list[NormalizedTaxTransfer]] = {}
    fee_events: dict[str, NormalizedTaxEvent] = {}
    for entries in groups:
        out_rows = [
            row
            for row, _ in entries
            if _row_get(row, "direction") == "outbound"
        ]
        in_rows = [
            row
            for row, _ in entries
            if _row_get(row, "direction") == "inbound"
        ]
        if not out_rows or not in_rows:
            continue

        first_out = out_rows[0]
        first_in = in_rows[0]
        from_wallet = wallet_refs_by_id[first_out["wallet_id"]]
        to_wallet = wallet_refs_by_id[first_in["wallet_id"]]
        sent = sum(
            msat_to_btc(row["amount"]) + msat_to_btc(row["fee"])
            for row in out_rows
        )
        received = sum(msat_to_btc(row["amount"]) for row in in_rows)
        if sent < received:
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    first_out,
                    "transfer_mismatch",
                    {
                        "from_wallet": from_wallet["label"],
                        "to_wallet": to_wallet["label"],
                        "sent": float(sent),
                        "received": float(received),
                        "protocol": "samourai_whirlpool",
                    },
                )
            )
            continue

        fee = sent - received
        spot_price = None
        spot_price_row = first_out
        spot_price_wallet_label = from_wallet["label"]
        if fee > 0:
            for candidate in [*out_rows, *in_rows]:
                quantity = msat_to_btc(candidate["amount"]) + msat_to_btc(
                    _row_get(candidate, "fee") or 0
                )
                candidate_price = _spot_price_from_row(candidate, quantity)
                if candidate_price is not None:
                    spot_price = candidate_price
                    spot_price_row = candidate
                    spot_price_wallet_label = wallet_refs_by_id[candidate["wallet_id"]][
                        "label"
                    ]
                    break
            if spot_price is not None and _pricing_needs_review(spot_price_row):
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        spot_price_row,
                        "pricing_review_required",
                        _pricing_review_detail(
                            spot_price_row,
                            spot_price_wallet_label,
                            asset,
                            "samourai_privacy_transfer",
                        ),
                    )
                )
                continue
            if spot_price is None:
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        first_out,
                        "missing_spot_price",
                        {
                            "from_wallet": from_wallet["label"],
                            "to_wallet": to_wallet["label"],
                            "asset": asset,
                            "direction": "transfer",
                            "required_for": "samourai_privacy_fee",
                            "protocol": "samourai_whirlpool",
                        },
                    )
                )
                continue

        to_wallet_labels = {
            wallet_refs_by_id[row["wallet_id"]]["label"] for row in in_rows
        }
        if len(in_rows) > 1 or len(to_wallet_labels) > 1:
            collected[str(first_out["id"])] = [
                NormalizedTaxTransfer(
                    asset=asset,
                    occurred_at=first_out["occurred_at"],
                    out_transaction_id=first_out["id"],
                    in_transaction_id=in_row["id"],
                    from_wallet_id=from_wallet["id"],
                    from_wallet_label=from_wallet["label"],
                    to_wallet_id=wallet_refs_by_id[in_row["wallet_id"]]["id"],
                    to_wallet_label=wallet_refs_by_id[in_row["wallet_id"]]["label"],
                    sent=msat_to_btc(in_row["amount"]),
                    received=msat_to_btc(in_row["amount"]),
                    fee=Decimal("0"),
                    spot_price=spot_price,
                    description=(
                        first_out["note"]
                        or first_out["description"]
                        or first_out["kind"]
                        or "Samourai Whirlpool privacy movement"
                    ),
                    external_id=_row_get(first_out, "external_id"),
                    out_row=first_out,
                    in_row=in_row,
                    at_pool=resolve_pool_id(from_wallet["id"]) if is_at else None,
                    transfer_id=f"{first_out['id']}::{in_row['id']}",
                )
                for in_row in in_rows
            ]
            if fee > 0:
                fee_events[str(first_out["id"])] = NormalizedTaxEvent(
                    transaction_id=first_out["id"],
                    asset=asset,
                    occurred_at=first_out["occurred_at"],
                    wallet_id=from_wallet["id"],
                    wallet_label=from_wallet["label"],
                    direction="outbound",
                    amount=Decimal("0"),
                    fee=fee,
                    spot_price=spot_price,
                    fiat_value=None,
                    description=(
                        first_out["note"]
                        or first_out["description"]
                        or first_out["kind"]
                        or "Samourai Whirlpool privacy fee"
                    ),
                    raw_row=first_out,
                    at_pool=resolve_pool_id(from_wallet["id"]) if is_at else None,
                )
            continue

        collected[str(first_out["id"])] = [
            NormalizedTaxTransfer(
                asset=asset,
                occurred_at=first_out["occurred_at"],
                out_transaction_id=first_out["id"],
                in_transaction_id=first_in["id"],
                from_wallet_id=from_wallet["id"],
                from_wallet_label=from_wallet["label"],
                to_wallet_id=to_wallet["id"],
                to_wallet_label=to_wallet["label"],
                sent=sent,
                received=received,
                fee=fee,
                spot_price=spot_price,
                description=(
                    first_out["note"]
                    or first_out["description"]
                    or first_out["kind"]
                    or "Samourai Whirlpool privacy movement"
                ),
                external_id=_row_get(first_out, "external_id"),
                out_row=first_out,
                in_row=first_in,
                at_pool=resolve_pool_id(from_wallet["id"]) if is_at else None,
            )
        ]
    return collected, fee_events


def normalize_tax_asset_inputs(
    profile: Mapping[str, Any],
    asset: str,
    rows: Sequence[Mapping[str, Any]],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    intra_pairs: Sequence[Mapping[str, Any]],
    at_regime_by_row_id: Optional[Mapping[str, AtRegime]] = None,
    at_swap_link_by_row_id: Optional[Mapping[str, str]] = None,
) -> NormalizedTaxAssetInputs:
    tax_country = ""
    if hasattr(profile, "keys") and "tax_country" in profile.keys():
        tax_country = str(profile["tax_country"] or "").strip().lower()
    is_at = tax_country == "at"
    regime_map = at_regime_by_row_id or {}
    swap_link_map = at_swap_link_by_row_id or {}
    outbound_regimes = infer_outbound_regimes(rows) if is_at else {}
    events: list[NormalizedTaxEvent] = []
    transfers: list[NormalizedTaxTransfer] = []
    ordered_items: list[tuple[str, str]] = []
    quarantines: list[dict[str, Any]] = []
    samourai_internal_groups = _samourai_internal_privacy_groups(rows)
    samourai_internal_row_ids = _samourai_internal_privacy_row_ids(
        samourai_internal_groups
    )
    samourai_transfer_by_out_id, samourai_fee_event_by_out_id = (
        _collect_samourai_internal_transfers(
            profile,
            asset,
            samourai_internal_groups,
            wallet_refs_by_id,
            is_at,
            quarantines,
        )
    )

    pair_by_row: dict[str, tuple[str, Mapping[str, Any]]] = {}
    for pair in intra_pairs:
        pair_by_row[pair["out"]["id"]] = ("out", pair)
        pair_by_row[pair["in"]["id"]] = ("in", pair)
    handled_pairs: set[tuple[str, str]] = set()

    for row in rows:
        if row["id"] in samourai_internal_row_ids:
            for transfer in samourai_transfer_by_out_id.get(row["id"], []):
                transfers.append(transfer)
                ordered_items.append(
                    ("transfer", transfer.transfer_id or transfer.out_transaction_id)
                )
            event = samourai_fee_event_by_out_id.get(row["id"])
            if event is not None:
                events.append(event)
                ordered_items.append(("event", row["id"]))
            continue
        role_pair = pair_by_row.get(row["id"])
        if role_pair is not None:
            _, pair = role_pair
            pair_key = (pair["out"]["id"], pair["in"]["id"])
            if pair_key in handled_pairs:
                continue
            handled_pairs.add(pair_key)

            out_row = pair["out"]
            in_row = pair["in"]
            from_wallet = wallet_refs_by_id[out_row["wallet_id"]]
            to_wallet = wallet_refs_by_id[in_row["wallet_id"]]
            out_privacy_hop = _privacy_hop_evidence(out_row)
            in_privacy_hop = _privacy_hop_evidence(in_row)
            if out_privacy_hop or in_privacy_hop:
                if out_privacy_hop:
                    _append_privacy_hop_quarantine(
                        quarantines,
                        profile,
                        out_row,
                        from_wallet,
                        asset,
                        "transfer",
                        out_privacy_hop,
                    )
                if in_privacy_hop:
                    _append_privacy_hop_quarantine(
                        quarantines,
                        profile,
                        in_row,
                        to_wallet,
                        asset,
                        "transfer",
                        in_privacy_hop,
                    )
                continue
            sent = msat_to_btc(out_row["amount"]) + msat_to_btc(out_row["fee"])
            received = msat_to_btc(in_row["amount"])
            if sent < received:
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        out_row,
                        "transfer_mismatch",
                        {
                            "from_wallet": from_wallet["label"],
                            "to_wallet": to_wallet["label"],
                            "sent": float(sent),
                            "received": float(received),
                        },
                    )
                )
                continue

            fee = sent - received
            spot_price_row = out_row
            spot_price_wallet_label = from_wallet["label"]
            spot_price = _spot_price_from_row(out_row, msat_to_btc(out_row["amount"]))
            if spot_price is None:
                spot_price = _spot_price_from_row(in_row, msat_to_btc(in_row["amount"]))
                spot_price_row = in_row
                spot_price_wallet_label = to_wallet["label"]
            if fee > 0 and spot_price is not None and _pricing_needs_review(spot_price_row):
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        spot_price_row,
                        "pricing_review_required",
                        _pricing_review_detail(
                            spot_price_row,
                            spot_price_wallet_label,
                            asset,
                            "transfer",
                        ),
                    )
                )
                continue
            if spot_price is None and fee > 0:
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        out_row,
                        "missing_spot_price",
                        {
                            "from_wallet": from_wallet["label"],
                            "to_wallet": to_wallet["label"],
                            "asset": asset,
                            "direction": "transfer",
                            "required_for": "transfer_fee",
                        },
                    )
                )
                continue

            description = (
                out_row["note"]
                or out_row["description"]
                or out_row["kind"]
                or f"Transfer {from_wallet['label']} -> {to_wallet['label']}"
            )
            transfers.append(
                NormalizedTaxTransfer(
                    asset=asset,
                    occurred_at=out_row["occurred_at"],
                    out_transaction_id=out_row["id"],
                    in_transaction_id=in_row["id"],
                    from_wallet_id=from_wallet["id"],
                    from_wallet_label=from_wallet["label"],
                    to_wallet_id=to_wallet["id"],
                    to_wallet_label=to_wallet["label"],
                    sent=sent,
                    received=received,
                    fee=fee,
                    spot_price=spot_price,
                    description=description,
                    external_id=out_row["external_id"],
                    out_row=out_row,
                    in_row=in_row,
                    at_pool=resolve_pool_id(from_wallet["id"]) if is_at else None,
                )
            )
            ordered_items.append(("transfer", out_row["id"]))
            continue

        wallet = wallet_refs_by_id[row["wallet_id"]]
        amount = msat_to_btc(row["amount"])
        fee = msat_to_btc(row["fee"])
        description = row["note"] or row["description"] or row["kind"] or row["id"]
        direction = row["direction"]
        privacy_hop = _privacy_hop_evidence(row)
        if privacy_hop:
            _append_privacy_hop_quarantine(
                quarantines,
                profile,
                row,
                wallet,
                asset,
                direction,
                privacy_hop,
            )
            continue
        if direction == "inbound":
            if _pricing_needs_review(row):
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        row,
                        "pricing_review_required",
                        _pricing_review_detail(row, wallet["label"], asset, direction),
                    )
                )
                continue
            spot_price = _spot_price_from_row(row, amount)
            if spot_price is None:
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        row,
                        "missing_spot_price",
                        {
                            "wallet": wallet["label"],
                            "asset": asset,
                            "direction": direction,
                            "required_for": "acquisition",
                        },
                    )
                )
                continue
            fiat_value = pricing.decimal_from_exact(
                _row_get(row, "fiat_value_exact"),
                _row_get(row, "fiat_value"),
            ) or amount * spot_price
        elif direction == "outbound":
            if _pricing_needs_review(row):
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        row,
                        "pricing_review_required",
                        _pricing_review_detail(row, wallet["label"], asset, direction),
                    )
                )
                continue
            needed = amount + fee
            if needed <= 0:
                continue
            spot_price = _spot_price_from_row(row, amount if amount > 0 else fee)
            if spot_price is None:
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        row,
                        "missing_spot_price",
                        {
                            "wallet": wallet["label"],
                            "asset": asset,
                            "direction": direction,
                            "required_for": "disposal",
                        },
                    )
                )
                continue
            fiat_value = pricing.decimal_from_exact(
                _row_get(row, "fiat_value_exact"),
                _row_get(row, "fiat_value"),
            ) or amount * spot_price
        else:
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    row,
                    "unsupported_tax_direction",
                    {
                        "wallet": wallet["label"],
                        "asset": asset,
                        "direction": direction,
                    },
                )
            )
            continue

        at_regime = None
        at_pool = None
        at_swap_link = None
        if is_at:
            at_pool = resolve_pool_id(wallet["id"])
            regime_override = _row_get(row, "at_regime_override")
            if direction == "inbound":
                at_regime = infer_regime_from_timestamp(row["occurred_at"])
            else:
                at_regime = regime_map.get(
                    row["id"],
                    outbound_regimes.get(row["id"], infer_regime_from_timestamp(row["occurred_at"])),
                )
            if regime_override in ("alt", "neu"):
                at_regime = regime_override
            linked = swap_link_map.get(row["id"])
            if linked:
                at_swap_link = linked
        events.append(
            NormalizedTaxEvent(
                transaction_id=row["id"],
                asset=asset,
                occurred_at=row["occurred_at"],
                wallet_id=wallet["id"],
                wallet_label=wallet["label"],
                direction=direction,
                amount=amount,
                fee=fee,
                spot_price=spot_price,
                fiat_value=fiat_value,
                description=description,
                raw_row=row,
                at_regime=at_regime,
                at_pool=at_pool,
                at_swap_link=at_swap_link,
            )
        )
        ordered_items.append(("event", row["id"]))

    return NormalizedTaxAssetInputs(
        asset=asset,
        events=events,
        transfers=transfers,
        ordered_items=ordered_items,
        quarantines=quarantines,
    )


__all__ = [
    "NormalizedTaxAssetInputs",
    "NormalizedTaxEvent",
    "NormalizedTaxTransfer",
    "build_tax_quarantine",
    "normalize_tax_asset_inputs",
]
