"""Journal quarantine records: their shape and their collapse per transaction.

The tax engine's reader (``engines/ledger_response.py``) renders the engine's
quarantines through ``build_tax_quarantine``; ``custody_journal.py`` and
``review_workflow.py`` collapse a ledger's quarantines with
``dedupe_quarantines`` before storing or previewing them.
``custody_tax_projection.py`` explains a standalone event's missing or
review-required price with ``transaction_price_quarantine``.
"""

from __future__ import annotations

import json
from decimal import Decimal
from typing import Any, Mapping, Sequence

from ..msat import msat_to_btc
from . import pricing, quarantine_catalog


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


def _row_get(row: Mapping[str, Any], key: str) -> Any:
    if hasattr(row, "keys") and key not in row.keys():
        return None
    if hasattr(row, "get"):
        return row.get(key)
    return row[key]


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
    A root cause outranks a downstream consequence (for example a custody gap
    hold over the basis barrier it raises): the stored reason must say what to
    fix, not what follows from it. Exact duplicates are discarded; first-seen
    transaction order is preserved.
    """

    def parsed(value: Any) -> Any:
        try:
            return json.loads(value)
        except (ValueError, TypeError):
            return value

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
        primary, secondary = existing, dict(quarantine)
        if quarantine_catalog.is_downstream(
            str(existing["reason"])
        ) and not quarantine_catalog.is_downstream(str(quarantine["reason"])):
            primary, secondary = dict(quarantine), existing
        detail = parsed(primary["detail_json"])
        if not isinstance(detail, dict):
            detail = {"detail": detail}
        additional = list(detail.pop("additional_reasons", []) or [])
        secondary_detail = parsed(secondary["detail_json"])
        carried: list[Any] = []
        if isinstance(secondary_detail, dict):
            carried = list(secondary_detail.pop("additional_reasons", []) or [])
        additional.append({"reason": secondary["reason"], "detail": secondary_detail})
        additional.extend(carried)
        detail["additional_reasons"] = additional
        primary["detail_json"] = json.dumps(detail, sort_keys=True)
        by_id[tx_id] = primary
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


def _row_msat(row: Mapping[str, Any], key: str) -> int:
    return int(_row_get(row, key) or 0)


def transaction_price_quarantine(
    profile: Mapping[str, Any], row: Mapping[str, Any], wallet_label: str,
) -> dict[str, Any] | None:
    """Explain a standalone event's price even when prior basis blocks it."""
    direction = _row_get(row, "direction")
    if direction not in {"inbound", "outbound"}:
        return None
    amount = msat_to_btc(_row_msat(row, "amount"))
    fee = msat_to_btc(_row_msat(row, "fee"))
    if direction == "outbound" and amount + fee <= 0:
        return None
    asset = str(row["asset"])
    if _pricing_needs_review(row) and _profile_requires_coarse_review(profile):
        return build_tax_quarantine(profile, row, "pricing_review_required",
            _pricing_review_detail(row, wallet_label, asset, direction))
    if _spot_price_from_row(row, amount if amount > 0 else fee) is None:
        return build_tax_quarantine(profile, row, "missing_spot_price", {
            "wallet": wallet_label, "asset": asset, "direction": direction,
            "required_for": "acquisition" if direction == "inbound" else "disposal",
        })
    return None


__all__ = [
    "build_tax_quarantine",
    "dedupe_quarantines",
    "transaction_price_quarantine",
]
