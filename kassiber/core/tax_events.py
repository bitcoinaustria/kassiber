from __future__ import annotations

import json
from dataclasses import dataclass
from decimal import Decimal
from typing import Any, Literal, Mapping, Optional, Sequence

from ..msat import dec, msat_to_btc
from .austrian import infer_outbound_regimes, infer_regime_from_timestamp, resolve_pool_id

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
    # Carried basis in fiat for the incoming leg of a swap. When set, it
    # overrides `fiat_value` as the basis seeded into rp2's InTransaction,
    # so the destination asset's pool inherits the outgoing asset's basis
    # (§ 27b Abs 3 Z 2 EStG). None means "use fiat_value" (spot-at-receipt).
    carried_basis_fiat: Optional[Decimal] = None


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
    if row["fiat_rate"] is not None:
        rate = dec(row["fiat_rate"])
        if rate > 0:
            return rate
    if row["fiat_value"] is not None and quantity > 0:
        value = dec(row["fiat_value"])
        if value > 0:
            return value / quantity
    return None


def normalize_tax_asset_inputs(
    profile: Mapping[str, Any],
    asset: str,
    rows: Sequence[Mapping[str, Any]],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    intra_pairs: Sequence[Mapping[str, Any]],
    at_swap_link_by_row_id: Optional[Mapping[str, str]] = None,
    at_carried_basis_by_row_id: Optional[Mapping[str, Decimal]] = None,
) -> NormalizedTaxAssetInputs:
    tax_country = ""
    if hasattr(profile, "keys") and "tax_country" in profile.keys():
        tax_country = str(profile["tax_country"] or "").strip().lower()
    is_at = tax_country == "at"
    swap_link_map = at_swap_link_by_row_id or {}
    carried_basis_map = at_carried_basis_by_row_id or {}
    outbound_regimes = infer_outbound_regimes(rows) if is_at else {}
    events: list[NormalizedTaxEvent] = []
    transfers: list[NormalizedTaxTransfer] = []
    ordered_items: list[tuple[str, str]] = []
    quarantines: list[dict[str, Any]] = []

    pair_by_row: dict[str, tuple[str, Mapping[str, Any]]] = {}
    for pair in intra_pairs:
        pair_by_row[pair["out"]["id"]] = ("out", pair)
        pair_by_row[pair["in"]["id"]] = ("in", pair)
    handled_pairs: set[tuple[str, str]] = set()

    for row in rows:
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
            spot_price = _spot_price_from_row(out_row, msat_to_btc(out_row["amount"]))
            if spot_price is None:
                spot_price = _spot_price_from_row(in_row, msat_to_btc(in_row["amount"]))
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
        if direction == "inbound":
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
            fiat_value = dec(row["fiat_value"]) if row["fiat_value"] is not None else amount * spot_price
        elif direction == "outbound":
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
            fiat_value = dec(row["fiat_value"]) if row["fiat_value"] is not None else amount * spot_price
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
        carried_basis_fiat = None
        if is_at:
            at_pool = resolve_pool_id(wallet["id"])
            if direction == "inbound":
                at_regime = infer_regime_from_timestamp(row["occurred_at"])
            else:
                at_regime = outbound_regimes.get(row["id"], infer_regime_from_timestamp(row["occurred_at"]))
            linked = swap_link_map.get(row["id"])
            if linked:
                at_swap_link = linked
            carried = carried_basis_map.get(row["id"])
            if carried is not None and direction == "inbound":
                carried_basis_fiat = carried
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
                carried_basis_fiat=carried_basis_fiat,
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
