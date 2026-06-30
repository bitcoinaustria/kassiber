from __future__ import annotations

import json
from dataclasses import dataclass
from decimal import Decimal
from typing import Any, Literal, Mapping, Optional, Sequence

from ..msat import msat_to_btc
from ..transfers import normalize_group_txid
from . import pricing
from .austrian import infer_outbound_regimes, infer_regime_from_timestamp, resolve_pool_id
from .loans import LOCK_SUPPRESS_ROLES, RELEASE_SUPPRESS_ROLES
from .privacy_hops import privacy_hop_evidence_from_row
from .transfer_matching import (
    DEFAULT_FEE_PCT_MAX,
    DEFAULT_FEE_SATS_MIN,
    fee_threshold_msat,
)

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
    # Bitcoin-backed-loan leg role (kassiber.core.loans.LEG_ROLES) when this
    # transaction is a leg of a loan. Drives engine classification: a
    # collateral lock/release is suppressed (coins stay in the owned pool,
    # encumbered), a liquidation/repay-sale falls through to the normal
    # disposal path. None when the transaction is not a loan leg.
    loan_leg_role: Optional[str] = None


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
    # Pool partition id to preserve across an intra-wallet move.
    at_pool: Optional[str] = None
    # Austrian regime (alt/neu) for the MOVE's taxable miner-fee disposal. The
    # move itself is non-taxable, but the fee is a disposal; without a regime
    # rp2's moving-average aborts the whole asset on "Ambiguous Austrian
    # disposal" when the wallet holds both Alt and Neu lots. None outside AT.
    at_regime: Optional[AtRegime] = None
    # Optional stable id for one logical movement split out of a multi-output
    # wallet transaction. Journal rows still point at the real out/in rows.
    transfer_id: Optional[str] = None
    # Optional logical group for derived multi-leg self-transfers. If any leg in
    # the group cannot be booked, the whole group must be deferred so synthetic
    # MOVE legs cannot partially replace one recorded transaction.
    group_id: Optional[str] = None
    # Real rows removed/replaced by this derived group. These must also be
    # surfaced if the synthetic MOVE group is blocked downstream.
    group_block_rows: tuple[Mapping[str, Any], ...] = ()
    # How this self-transfer was paired, for audit provenance:
    # "ownership_derived" when proven from the on-chain address graph; None for a
    # same-txid auto match or a user's manual pair.
    pairing_source: Optional[str] = None


@dataclass(frozen=True)
class NormalizedTaxAssetInputs:
    asset: str
    events: Sequence[NormalizedTaxEvent]
    transfers: Sequence[NormalizedTaxTransfer]
    ordered_items: Sequence[tuple[str, str]]
    quarantines: Sequence[dict[str, Any]]
    # Earliest occurred_at of any normalize-quarantined acquisition / disposal /
    # transfer leg for this asset. Any such drop leaves RP2's lot pool
    # inconsistent (a missing-basis acquisition, an unconsumed disposal, an
    # un-booked / partial transfer), so from that instant the cost basis a later
    # disposal would draw is untrustworthy under ANY accounting method. The
    # engine combines this with its own gate-level drops (unclassified income,
    # gift/lost) and conservatively quarantines later disposals as
    # `basis_provenance_incomplete` until the contaminating row is resolved.
    # None when nothing was quarantined.
    earliest_lot_contamination_at: Optional[str] = None


def build_tax_quarantine(
    profile: Mapping[str, Any],
    row: Mapping[str, Any],
    reason: str,
    detail: Mapping[str, Any],
) -> dict[str, Any]:
    # Synthetic, engine-only rows (split / direct-payout legs) carry a
    # journal_transaction_id pointing at the real transaction. A quarantine must
    # reference that real id so it satisfies journal_quarantines' FK to
    # transactions(id) — a synthetic id like "cross-split:...:out" would make the
    # insert fail and abort the whole `journals process`.
    transaction_id = _row_get(row, "journal_transaction_id") or row["id"]
    return {
        "transaction_id": transaction_id,
        "workspace_id": profile["workspace_id"],
        "profile_id": profile["id"],
        "reason": reason,
        "detail_json": json.dumps(detail, sort_keys=True),
    }


def _quarantine_partner_leg(
    quarantines: list[dict[str, Any]],
    profile: Mapping[str, Any],
    primary_row: Mapping[str, Any],
    out_row: Mapping[str, Any],
    in_row: Mapping[str, Any],
    reason: str,
    from_wallet: Mapping[str, Any],
    to_wallet: Mapping[str, Any],
    asset: str,
    group_id: str | None = None,
) -> None:
    """Quarantine the OTHER leg of a self-transfer pair that was not booked.

    The pair branch of ``normalize_tax_asset_inputs`` processes only the out row
    and skips the in row (``handled_pairs``), so quarantining a single leg would
    leave the partner (typically the recorded inbound receipt) neither booked nor
    flagged — a silent loss that later trips a spurious ``insufficient_lots`` on a
    genuine spend from the destination. Mirror the privacy-hop branch: when the
    pair is not booked as a transfer, surface BOTH legs for review.
    """
    partner = in_row if str(primary_row["id"]) == str(out_row["id"]) else out_row
    if str(partner["id"]) == str(primary_row["id"]):
        return
    detail = {
        "from_wallet": from_wallet["label"],
        "to_wallet": to_wallet["label"],
        "asset": asset,
        "direction": "transfer",
        "paired_leg": True,
    }
    if group_id:
        detail["transfer_group_id"] = group_id
    quarantines.append(
        build_tax_quarantine(
            profile,
            partner,
            reason,
            detail,
        )
    )


def _pair_group_id(pair: Mapping[str, Any]) -> str | None:
    raw = pair.get("group_id") if hasattr(pair, "get") else None
    if raw in (None, ""):
        return None
    return str(raw)


def _pair_group_block_rows(pair: Mapping[str, Any]) -> tuple[Mapping[str, Any], ...]:
    raw = pair.get("group_block_rows") if hasattr(pair, "get") else None
    if not raw:
        return ()
    return tuple(row for row in raw if isinstance(row, Mapping))


def _transfer_item_id(transfer: NormalizedTaxTransfer) -> str:
    return str(transfer.transfer_id or transfer.out_transaction_id)


def _with_transfer_group(
    detail: Mapping[str, Any], group_id: str | None
) -> dict[str, Any]:
    out = dict(detail)
    if group_id:
        out["transfer_group_id"] = group_id
    return out


def _append_group_block_quarantines(
    quarantines: list[dict[str, Any]],
    profile: Mapping[str, Any],
    pairs: Sequence[Mapping[str, Any]],
    *,
    group_id: str,
    blocked_by_reason: str,
    asset: str,
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
) -> None:
    """Flag every real row touched by a derived transfer group.

    A grouped fan-out/consolidation replaces one recorded transaction with
    several synthetic MOVE pairs. If one pair is not bookable, keeping sibling
    MOVEs would silently make a partial replacement. Existing quarantines for
    the triggering pair are preserved; this helper adds review rows for any
    unflagged sibling source/destination rows.
    """
    seen = {str(q["transaction_id"]) for q in quarantines}
    for pair in pairs:
        rows_to_flag = [pair["out"], pair["in"], *_pair_group_block_rows(pair)]
        for row in rows_to_flag:
            transaction_id = str(_row_get(row, "journal_transaction_id") or row["id"])
            if transaction_id in seen:
                continue
            seen.add(transaction_id)
            wallet = wallet_refs_by_id.get(str(_row_get(row, "wallet_id")))
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    row,
                    "derived_transfer_group_blocked",
                    {
                        "asset": asset,
                        "wallet": (
                            wallet["label"]
                            if wallet is not None and wallet.get("label")
                            else str(_row_get(row, "wallet_id"))
                        ),
                        "direction": "transfer",
                        "transfer_group_id": group_id,
                        "blocked_by_reason": blocked_by_reason,
                    },
                )
            )


def dedupe_quarantines(quarantines: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    """Collapse quarantines that share a ``transaction_id``.

    ``journal_quarantines.transaction_id`` is the PRIMARY KEY. Several engine
    paths can emit more than one quarantine for the SAME real transaction id —
    multiple synthetic legs (direct-payout proceeds rows, and on the split
    branch the ``cross-split:`` peg legs) map back to one real out tx, and any
    two gate/normalize drops keyed on the same row collide too. Inserting both
    rows would raise a UNIQUE constraint and abort the ENTIRE ``journals
    process`` — the exact all-or-nothing failure this hardening exists to
    remove. Collapse to one row per transaction, preserving the first-seen
    reason/detail and folding any *distinct* later reasons into
    ``detail_json['additional_reasons']`` so no review signal is silently lost.
    Exact duplicates are discarded; first-seen transaction order is preserved.
    """
    by_id: dict[str, dict[str, Any]] = {}
    for quarantine in quarantines:
        tx_id = quarantine["transaction_id"]
        existing = by_id.get(tx_id)
        if existing is None:
            by_id[tx_id] = dict(quarantine)
            continue
        if (
            quarantine["reason"] == existing["reason"]
            and quarantine["detail_json"] == existing["detail_json"]
        ):
            # Identical quarantine for the same transaction — nothing new.
            continue
        try:
            detail = json.loads(existing["detail_json"])
        except (ValueError, TypeError):
            detail = None
        if not isinstance(detail, dict):
            detail = {"detail": detail}
        try:
            extra_detail: Any = json.loads(quarantine["detail_json"])
        except (ValueError, TypeError):
            extra_detail = quarantine["detail_json"]
        detail.setdefault("additional_reasons", []).append(
            {"reason": quarantine["reason"], "detail": extra_detail}
        )
        existing["detail_json"] = json.dumps(detail, sort_keys=True)
    return list(by_id.values())


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


def _positive_fiat_value(row: Mapping[str, Any], fallback: Decimal) -> Decimal:
    """Recorded ``fiat_value`` only when strictly positive, else ``fallback``.

    A plain ``or`` fallback (the previous form) only catches ``0``/``None`` — a
    *negative* Decimal is truthy and would pass straight through to RP2's
    ``type_check_positive_decimal(non_zero=True)``, raising an uncaught
    ``RP2ValueError`` (the constructors run in the parse phase, outside the
    ``compute_tax`` try/except) that aborts the entire multi-asset report. A
    fiat value is never legitimately negative, so clamp to the spot-derived
    fallback instead of crashing.
    """
    value = pricing.decimal_from_exact(
        _row_get(row, "fiat_value_exact"),
        _row_get(row, "fiat_value"),
    )
    if value is not None and value > 0:
        return value
    return fallback


def _pricing_needs_review(row: Mapping[str, Any]) -> bool:
    return _row_get(row, "pricing_quality") == pricing.QUALITY_COARSE_FALLBACK


def _profile_requires_coarse_review(profile: Mapping[str, Any]) -> bool:
    """Coarse (daily/monthly/yearly) pricing is accepted by default; events are
    booked at the coarse spot price and flagged non-blockingly in the UI. Only a
    profile that opts into ``require_coarse_review`` quarantines them for manual
    pricing review (the previous always-on behavior)."""
    if not hasattr(profile, "keys") or "require_coarse_review" not in profile.keys():
        return False
    value = profile["require_coarse_review"]
    if isinstance(value, str):
        return value.strip().lower() in {"1", "true", "yes", "on"}
    return bool(value)


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
    outbound_regimes: Optional[Mapping[str, AtRegime]] = None,
) -> tuple[dict[str, list[NormalizedTaxTransfer]], dict[str, NormalizedTaxEvent]]:
    collected: dict[str, list[NormalizedTaxTransfer]] = {}
    fee_events: dict[str, NormalizedTaxEvent] = {}
    regime_by_row = outbound_regimes or {}

    def _samourai_fee_regime(out_row: Mapping[str, Any]) -> Optional[AtRegime]:
        # The Whirlpool privacy MOVE's miner fee is a taxable disposal; under AT
        # moving-average it needs a regime tag or rp2 aborts the whole asset on an
        # ambiguous disposal when both Alt and Neu lots exist. These transfers are
        # built here, bypassing the pair path, so stamp the regime directly.
        if not is_at:
            return None
        return regime_by_row.get(
            str(out_row["id"]), infer_regime_from_timestamp(out_row["occurred_at"])
        )
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
                        "protocol": "coinjoin",
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
            if (
                spot_price is not None
                and _pricing_needs_review(spot_price_row)
                and _profile_requires_coarse_review(profile)
            ):
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        spot_price_row,
                        "pricing_review_required",
                        _pricing_review_detail(
                            spot_price_row,
                            spot_price_wallet_label,
                            asset,
                            "coinjoin_transfer",
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
                            "required_for": "coinjoin_fee",
                            "protocol": "coinjoin",
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
                        or "Coinjoin privacy movement"
                    ),
                    external_id=_row_get(first_out, "external_id"),
                    out_row=first_out,
                    in_row=in_row,
                    at_pool=resolve_pool_id(from_wallet["id"]) if is_at else None,
                    at_regime=_samourai_fee_regime(first_out),
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
                        or "Coinjoin privacy fee"
                    ),
                    raw_row=first_out,
                    at_pool=resolve_pool_id(from_wallet["id"]) if is_at else None,
                    at_regime=_samourai_fee_regime(first_out),
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
                    or "Coinjoin privacy movement"
                ),
                external_id=_row_get(first_out, "external_id"),
                out_row=first_out,
                in_row=first_in,
                at_pool=resolve_pool_id(from_wallet["id"]) if is_at else None,
                at_regime=_samourai_fee_regime(first_out),
            )
        ]
    return collected, fee_events


def _owned_fanout_row_ids(
    rows: Sequence[Mapping[str, Any]],
    pair_by_row: Mapping[str, Any],
    samourai_internal_row_ids: set[str],
) -> set[str]:
    """Ids of rows in a same-(external_id, asset) group that moves coins across
    two or more owned wallets but is NOT a clean 1-out/1-in self-transfer.

    ``detect_intra_transfers`` only pairs the exactly-one-out/one-in shape, so a
    fan-out (1->N owned wallets) or consolidation (N->1) is skipped and would
    otherwise be booked as a standalone SELL plus fresh BUYs — destroying cost
    basis and inventing a phantom gain. These need explicit per-leg pairing or
    splitting, so they are quarantined instead. Groups already handled by a pair
    or the Samourai splitter are left alone.
    """
    groups: dict[tuple[str, str], list[Mapping[str, Any]]] = {}
    for row in rows:
        external_id = _row_get(row, "external_id")
        if not external_id:
            continue
        groups.setdefault((normalize_group_txid(external_id), row["asset"]), []).append(row)
    fanout_ids: set[str] = set()
    for group in groups.values():
        if len(group) < 2:
            continue
        if any(
            row["id"] in samourai_internal_row_ids or row["id"] in pair_by_row
            for row in group
        ):
            continue
        outs = [
            row
            for row in group
            if _row_get(row, "direction") == "outbound" and (row["amount"] or 0) > 0
        ]
        # Symmetric with the outbound filter and with detect_intra_transfers: a
        # non-positive inbound is never a real receiving leg, so it must not
        # inflate the inbound count and flip a clean self-transfer into a
        # spurious owned_fanout_unresolved quarantine.
        ins = [
            row
            for row in group
            if _row_get(row, "direction") == "inbound" and (row["amount"] or 0) > 0
        ]
        wallets = {row["wallet_id"] for row in group}
        if outs and ins and (len(outs) > 1 or len(ins) > 1) and len(wallets) >= 2:
            fanout_ids.update(row["id"] for row in group)
    return fanout_ids


def normalize_tax_asset_inputs(
    profile: Mapping[str, Any],
    asset: str,
    rows: Sequence[Mapping[str, Any]],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
    intra_pairs: Sequence[Mapping[str, Any]],
    at_regime_by_row_id: Optional[Mapping[str, AtRegime]] = None,
    at_swap_link_by_row_id: Optional[Mapping[str, str]] = None,
    loan_leg_by_transaction_id: Optional[Mapping[str, str]] = None,
) -> NormalizedTaxAssetInputs:
    tax_country = ""
    if hasattr(profile, "keys") and "tax_country" in profile.keys():
        tax_country = str(profile["tax_country"] or "").strip().lower()
    is_at = tax_country == "at"
    regime_map = at_regime_by_row_id or {}
    swap_link_map = at_swap_link_by_row_id or {}
    loan_leg_map = loan_leg_by_transaction_id or {}
    # Suppressed loan legs (collateral lock/release and friends) are non-events:
    # the coins never leave the owned pool. Exclude them from regime/inventory
    # inference so a lock does not "consume" Alt inventory and a release does not
    # add phantom Neu inventory — otherwise a later real sale is assigned a regime
    # whose pool is empty and rp2 aborts with "in < taxable".
    suppressed_loan_ids = {
        str(tx_id)
        for tx_id, role in loan_leg_map.items()
        if role in LOCK_SUPPRESS_ROLES or role in RELEASE_SUPPRESS_ROLES
    }
    regime_rows = (
        [row for row in rows if str(row["id"]) not in suppressed_loan_ids]
        if suppressed_loan_ids
        else list(rows)
    )
    # Austrian regime inference walks rows in order and depletes the Alt/Neu
    # pools positionally, so feed it a CHRONOLOGICAL, economically-meaningful
    # order — acquisitions before disposals at an equal timestamp — instead of
    # inheriting the DB tiebreak (occurred_at, created_at, id). Otherwise a Neu
    # acquisition sharing a timestamp with a disposal/move could be processed
    # AFTER it purely by transaction id, flipping the disposal's regime (and an
    # Austrian self-transfer fee) between neu_gain (KZ 174, 27.5%) and
    # alt_taxfree. Mirrors the engine gate's inbound-before-outbound ordering.
    if is_at:
        regime_rows = sorted(
            regime_rows,
            key=lambda r: (
                str(_row_get(r, "occurred_at") or ""),
                0 if str(_row_get(r, "direction") or "") == "inbound" else 1,
                str(_row_get(r, "id")),
            ),
        )
    outbound_regimes = infer_outbound_regimes(regime_rows, intra_pairs) if is_at else {}
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
            outbound_regimes,
        )
    )

    pair_by_row: dict[str, tuple[str, Mapping[str, Any]]] = {}
    pairs_by_transfer_group: dict[str, list[Mapping[str, Any]]] = {}
    for pair in intra_pairs:
        pair_by_row[pair["out"]["id"]] = ("out", pair)
        pair_by_row[pair["in"]["id"]] = ("in", pair)
        group_id = _pair_group_id(pair)
        if group_id:
            pairs_by_transfer_group.setdefault(group_id, []).append(pair)
    handled_pairs: set[tuple[str, str]] = set()
    blocked_transfer_group_reasons: dict[str, str] = {}
    fanout_row_ids = _owned_fanout_row_ids(rows, pair_by_row, samourai_internal_row_ids)

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
        if row["id"] in fanout_row_ids:
            # One on-chain tx that moves coins across several owned wallets.
            # Booking each leg standalone would destroy basis; quarantine the
            # whole group for explicit pairing/splitting instead.
            quarantines.append(
                build_tax_quarantine(
                    profile,
                    row,
                    "owned_fanout_unresolved",
                    {
                        "wallet": wallet_refs_by_id[row["wallet_id"]]["label"],
                        "asset": asset,
                        "direction": _row_get(row, "direction"),
                        "external_id": str(_row_get(row, "external_id") or ""),
                    },
                )
            )
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
            group_id = _pair_group_id(pair)
            from_wallet = wallet_refs_by_id[out_row["wallet_id"]]
            to_wallet = wallet_refs_by_id[in_row["wallet_id"]]
            out_privacy_hop = _privacy_hop_evidence(out_row)
            in_privacy_hop = _privacy_hop_evidence(in_row)
            if out_privacy_hop or in_privacy_hop:
                if group_id:
                    blocked_transfer_group_reasons.setdefault(
                        group_id, "privacy_hop_unresolved"
                    )
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
                if group_id:
                    blocked_transfer_group_reasons.setdefault(
                        group_id, "transfer_mismatch"
                    )
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        out_row,
                        "transfer_mismatch",
                        _with_transfer_group(
                            {
                                "from_wallet": from_wallet["label"],
                                "to_wallet": to_wallet["label"],
                                "sent": float(sent),
                                "received": float(received),
                            },
                            group_id,
                        ),
                    )
                )
                _quarantine_partner_leg(
                    quarantines, profile, out_row, out_row, in_row,
                    "transfer_mismatch", from_wallet, to_wallet, asset, group_id,
                )
                continue

            fee = sent - received
            # The recorded on-chain fee (out_row["fee"]) legitimately explains
            # part of the implied fee, so only the UNRECOGNIZED outflow — the
            # amount that left the source beyond what the recipient got and the
            # recorded miner fee (i.e. out.amount - in.amount) — is suspicious. A
            # small move with a high recorded network fee is fine; but when this
            # excess blows past the swap-fee tolerance (max(1%, 2500 sats)) the
            # outbound almost certainly fanned out to an unrecognized recipient (a
            # cross-asset peg to a Liquid federation address, or a payment) that
            # this 1-out/1-in pairing would otherwise absorb as a giant "fee" and
            # tax as a disposal. Quarantine for explicit review (the user splits
            # it into the real self-transfer + a cross-asset pair / swap payout).
            #
            # Exception: when the source backend reports `amount` as a net wallet
            # delta with the fee folded in (BTCPay; `amount_includes_fee`), the
            # out/in gap IS the miner fee by construction — the fee lives in
            # `amount`, not the separate `fee` column — so there is no
            # "unrecognized" residual to flag. Treat it as fully recognized;
            # `fee = sent - received` already books that miner fee correctly.
            if _row_get(out_row, "amount_includes_fee"):
                unrecognized_outflow = Decimal("0")
            else:
                unrecognized_outflow = msat_to_btc(out_row["amount"]) - msat_to_btc(
                    in_row["amount"]
                )
            fee_ceiling = msat_to_btc(
                fee_threshold_msat(
                    int(out_row["amount"] or 0),
                    DEFAULT_FEE_PCT_MAX,
                    DEFAULT_FEE_SATS_MIN,
                )
            )
            if unrecognized_outflow > fee_ceiling:
                if group_id:
                    blocked_transfer_group_reasons.setdefault(
                        group_id, "transfer_fee_implausible"
                    )
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        out_row,
                        "transfer_fee_implausible",
                        _with_transfer_group(
                            {
                                "from_wallet": from_wallet["label"],
                                "to_wallet": to_wallet["label"],
                                "asset": asset,
                                "sent": float(sent),
                                "received": float(received),
                                "implied_fee": float(fee),
                                "unrecognized_outflow": float(unrecognized_outflow),
                                "fee_ceiling": float(fee_ceiling),
                                "required_for": "transfer_fee_review",
                            },
                            group_id,
                        ),
                    )
                )
                _quarantine_partner_leg(
                    quarantines, profile, out_row, out_row, in_row,
                    "transfer_fee_implausible", from_wallet, to_wallet, asset, group_id,
                )
                continue
            spot_price_row = out_row
            spot_price_wallet_label = from_wallet["label"]
            spot_price = _spot_price_from_row(out_row, msat_to_btc(out_row["amount"]))
            if spot_price is None:
                spot_price = _spot_price_from_row(in_row, msat_to_btc(in_row["amount"]))
                spot_price_row = in_row
                spot_price_wallet_label = to_wallet["label"]
            # When the fee can't be priced, quarantine the whole transfer rather
            # than book a mis-priced fee or emit a partial MOVE: a zero-fee MOVE
            # would leave the un-moved fee quantity in the source (overstating
            # holdings / double-spendable). The transfer is deferred until the
            # fee is priced; the per-account gate quarantines any dependent
            # destination disposal gracefully in the meantime (no crash).
            if (
                fee > 0
                and spot_price is not None
                and _pricing_needs_review(spot_price_row)
                and _profile_requires_coarse_review(profile)
            ):
                if group_id:
                    blocked_transfer_group_reasons.setdefault(
                        group_id, "pricing_review_required"
                    )
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        spot_price_row,
                        "pricing_review_required",
                        _with_transfer_group(
                            _pricing_review_detail(
                                spot_price_row,
                                spot_price_wallet_label,
                                asset,
                                "transfer",
                            ),
                            group_id,
                        ),
                    )
                )
                _quarantine_partner_leg(
                    quarantines, profile, spot_price_row, out_row, in_row,
                    "pricing_review_required", from_wallet, to_wallet, asset, group_id,
                )
                continue
            if spot_price is None and fee > 0:
                if group_id:
                    blocked_transfer_group_reasons.setdefault(
                        group_id, "missing_spot_price"
                    )
                quarantines.append(
                    build_tax_quarantine(
                        profile,
                        out_row,
                        "missing_spot_price",
                        _with_transfer_group(
                            {
                                "from_wallet": from_wallet["label"],
                                "to_wallet": to_wallet["label"],
                                "asset": asset,
                                "direction": "transfer",
                                "required_for": "transfer_fee",
                            },
                            group_id,
                        ),
                    )
                )
                _quarantine_partner_leg(
                    quarantines, profile, out_row, out_row, in_row,
                    "missing_spot_price", from_wallet, to_wallet, asset, group_id,
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
                    at_regime=outbound_regimes.get(str(out_row["id"])) if is_at else None,
                    group_id=group_id,
                    group_block_rows=_pair_group_block_rows(pair),
                    pairing_source=(
                        pair.get("source") if hasattr(pair, "get") else None
                    ),
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
            if _pricing_needs_review(row) and _profile_requires_coarse_review(profile):
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
            fiat_value = _positive_fiat_value(row, amount * spot_price)
        elif direction == "outbound":
            if _pricing_needs_review(row) and _profile_requires_coarse_review(profile):
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
            fiat_value = _positive_fiat_value(row, amount * spot_price)
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
        # Loan-leg role is country-agnostic (loans apply to generic + AT profiles).
        loan_leg_role = loan_leg_map.get(row["id"])
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
                loan_leg_role=loan_leg_role,
            )
        )
        ordered_items.append(("event", row["id"]))

    if blocked_transfer_group_reasons:
        blocked_group_ids = set(blocked_transfer_group_reasons)
        removed_transfer_item_ids = {
            _transfer_item_id(transfer)
            for transfer in transfers
            if transfer.group_id in blocked_group_ids
        }
        if removed_transfer_item_ids:
            transfers = [
                transfer
                for transfer in transfers
                if transfer.group_id not in blocked_group_ids
            ]
            ordered_items = [
                item
                for item in ordered_items
                if not (item[0] == "transfer" and item[1] in removed_transfer_item_ids)
            ]
        for group_id, reason in blocked_transfer_group_reasons.items():
            _append_group_block_quarantines(
                quarantines,
                profile,
                pairs_by_transfer_group.get(group_id, ()),
                group_id=group_id,
                blocked_by_reason=reason,
                asset=asset,
                wallet_refs_by_id=wallet_refs_by_id,
            )

    # Any quarantined acquisition / disposal / transfer leg leaves the lot pool
    # inconsistent from its occurred_at on. Derive the earliest such instant
    # directly from the quarantine set (method-agnostic) rather than enumerating
    # individual reasons, so a newly-added quarantine reason can't silently slip
    # past the basis-provenance guard.
    quarantined_ids = {str(q["transaction_id"]) for q in quarantines}
    contamination_times = [
        row["occurred_at"]
        for row in rows
        if str(row["id"]) in quarantined_ids
        and _row_get(row, "direction") in ("inbound", "outbound")
    ]

    return NormalizedTaxAssetInputs(
        asset=asset,
        events=events,
        transfers=transfers,
        ordered_items=ordered_items,
        quarantines=quarantines,
        earliest_lot_contamination_at=(
            min(contamination_times) if contamination_times else None
        ),
    )


__all__ = [
    "NormalizedTaxAssetInputs",
    "NormalizedTaxEvent",
    "NormalizedTaxTransfer",
    "build_tax_quarantine",
    "normalize_tax_asset_inputs",
]
