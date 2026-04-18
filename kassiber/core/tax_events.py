from __future__ import annotations

import json
from dataclasses import dataclass
from decimal import Decimal
from typing import Any, Mapping, Sequence

from ..msat import dec, msat_to_btc


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


@dataclass(frozen=True)
class NormalizedTaxAssetInputs:
    asset: str
    events: Sequence[NormalizedTaxEvent]
    transfers: Sequence[NormalizedTaxTransfer]
    ordered_items: Sequence[tuple[str, str]]
    quarantines: Sequence[dict[str, Any]]
    row_by_id: Mapping[str, Mapping[str, Any]]


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
) -> NormalizedTaxAssetInputs:
    events: list[NormalizedTaxEvent] = []
    transfers: list[NormalizedTaxTransfer] = []
    ordered_items: list[tuple[str, str]] = []
    quarantines: list[dict[str, Any]] = []
    row_by_id = {row["id"]: row for row in rows}

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
            )
        )
        ordered_items.append(("event", row["id"]))

    return NormalizedTaxAssetInputs(
        asset=asset,
        events=events,
        transfers=transfers,
        ordered_items=ordered_items,
        quarantines=quarantines,
        row_by_id=row_by_id,
    )


__all__ = [
    "NormalizedTaxAssetInputs",
    "NormalizedTaxEvent",
    "NormalizedTaxTransfer",
    "build_tax_quarantine",
    "normalize_tax_asset_inputs",
]
