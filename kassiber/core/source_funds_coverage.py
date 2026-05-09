"""Source-funds coverage view.

Aggregates each profile's inbound transactions into four buckets so users
can see, at a glance, how much of their holdings is actually traced
versus how much is still in review or untraced. This is the
proactive-counterpart to the existing reactive review queue: instead of
"here are 47 suggestions to look at", it answers "of your X BTC
holdings, Y is fully traced and Z still needs evidence".

Buckets:
- ``fully_traced``: ``build_report`` would emit ``exportable=True`` for
  this transaction AND the resulting source_mix contains only
  non-attestation source types.
- ``attested``: ``build_report`` would emit ``exportable=True`` AND at
  least one source_mix entry is an attestation source type
  (``missing_history``, ``opening_balance_attestation``).
- ``in_review``: ``build_report`` would emit at least one blocker, but
  some non-rejected link exists.
- ``untraced``: no non-rejected links exist for this transaction.
- ``not_classified``: the transaction was skipped because the
  ``max_transactions`` cap was hit. Counted into totals so that
  percentages computed off ``totals.amount`` always sum to 100;
  truncated responses must surface this so users don't read a
  partial classification as full coverage.

By delegating classification to ``build_report``, coverage stays in
lockstep with the export gate. Anything ``build_report`` would block
(heuristic allocation, unreviewed suggestion, asset/chronology
mismatch, unconfirmed chain observation, source over-allocation,
path_truncated, ...) classifies as ``in_review`` here. There is no
separate predicate to drift.
"""

from __future__ import annotations

import sqlite3
from collections import defaultdict
from typing import Any, Mapping

from ..errors import AppError
from ..msat import msat_to_btc
from .source_funds import ATTESTATION_SOURCE_TYPES, SourceFundsHooks, build_report


COVERAGE_BUCKETS = ("fully_traced", "attested", "in_review", "untraced", "not_classified")
# Match build_report's default so a path that would be path_truncated on
# export does not look fully_traced in coverage.
DEFAULT_MAX_DEPTH = 8
# Hard cap on the number of inbound transactions classified per call.
# Coverage delegates to build_report per tx, which walks the whole link
# subtree; on profiles with thousands of inbound rows the synchronous
# request would stall the UI. When this cap is hit, the response carries
# `truncated=True` and the remaining inbound amount is bucketed into a
# new ``not_classified`` total so the UI can prompt for an explicit
# full recompute instead of silently misreporting readiness.
DEFAULT_MAX_TRANSACTIONS = 2000


def _btc(msat: int | None) -> float:
    if msat is None:
        return 0.0
    return float(msat_to_btc(int(msat)))


def _has_any_link(conn: sqlite3.Connection, profile_id: str, tx_id: str) -> bool:
    row = conn.execute(
        """
        SELECT 1 FROM source_funds_links
        WHERE profile_id = ? AND to_transaction_id = ? AND state != 'rejected'
        LIMIT 1
        """,
        (profile_id, tx_id),
    ).fetchone()
    return row is not None


def _classify_via_report(
    report: Mapping[str, Any],
) -> str:
    """Return the coverage bucket given a ``build_report`` envelope.

    The mapping is deliberate: anything the export gate would block
    classifies as ``in_review`` here. A transaction is only
    ``fully_traced`` when its source_mix is composed entirely of
    non-attestation sources (i.e. a recipient that rejects
    attestations would still accept this disclosure).
    """
    explain_gates = report.get("explain_gates") or {}
    if not explain_gates.get("exportable"):
        return "in_review"
    source_mix = report.get("source_mix") or []
    has_real = any(
        item.get("source_type") not in ATTESTATION_SOURCE_TYPES
        for item in source_mix
    )
    has_attestation = any(
        item.get("source_type") in ATTESTATION_SOURCE_TYPES
        for item in source_mix
    )
    if has_real and not has_attestation:
        return "fully_traced"
    if has_attestation:
        return "attested"
    # Exportable with no source_mix entries shouldn't happen in practice,
    # but if it does (e.g. a future report shape), treat conservatively.
    return "in_review"


def _classify_transaction(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    profile_id: str,
    target_tx_id: str,
    *,
    max_depth: int = DEFAULT_MAX_DEPTH,
) -> str:
    """Classify one inbound transaction by delegating to ``build_report``.

    Coverage classification reuses the same gate the export PDF uses,
    so any drift is impossible by construction. ``build_report`` is
    called with ``save_case=False`` so coverage is read-only.

    On any AppError (invalid target, depth overflow with no path,
    pricing missing, etc.), the transaction is classified by its link
    presence: links exist => ``in_review``, no links => ``untraced``.
    This mirrors what users see in the wizard for that transaction.
    """
    if not _has_any_link(conn, profile_id, target_tx_id):
        return "untraced"
    try:
        report = build_report(
            conn,
            workspace_ref,
            profile_ref,
            hooks,
            target_transaction_ref=target_tx_id,
            reveal_mode="standard",
            max_depth=max_depth,
            save_case=False,
        )
    except AppError:
        return "in_review"
    return _classify_via_report(report)


def _empty_bucket() -> dict[str, Any]:
    return {
        "amount_msat": 0,
        "amount": 0.0,
        "tx_count": 0,
    }


def _empty_breakdown() -> dict[str, dict[str, Any]]:
    return {bucket: _empty_bucket() for bucket in COVERAGE_BUCKETS}


def _materialize(buckets: Mapping[str, Mapping[str, Any]]) -> dict[str, dict[str, Any]]:
    out: dict[str, dict[str, Any]] = {}
    for name, values in buckets.items():
        amount_msat = int(values["amount_msat"])
        out[name] = {
            "amount_msat": amount_msat,
            "amount": _btc(amount_msat),
            "tx_count": int(values["tx_count"]),
        }
    return out


def compute_coverage(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: SourceFundsHooks,
    *,
    max_depth: int = DEFAULT_MAX_DEPTH,
    max_transactions: int = DEFAULT_MAX_TRANSACTIONS,
) -> dict[str, Any]:
    """Compute coverage buckets for every inbound transaction in a profile.

    Returns:
        {
            "by_wallet": [
                {
                    "wallet_id": ..., "wallet_label": ..., "asset": ...,
                    "buckets": {"fully_traced": {...}, "attested": {...}, ...},
                    "total_inbound_msat": int, "total_inbound": float,
                },
                ...
            ],
            "by_asset": [{"asset": ..., "buckets": {...}, ...}, ...],
            "totals": {"buckets": {...}, "tx_count": int},
        }

    Buckets are mutually exclusive and exhaustive over the inbound tx
    set, which makes the percentages users see in the UI add up to 100.

    Each transaction is classified by running ``build_report`` against
    it. This guarantees coverage and the export gate cannot drift -
    everything the gate would block falls to ``in_review``.
    """
    _, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    profile_id = profile["id"]
    inbound_rows = conn.execute(
        """
        SELECT t.id, t.wallet_id, t.asset, t.amount, w.label AS wallet_label
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND t.direction = 'inbound' AND t.excluded = 0
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        (profile_id,),
    ).fetchall()

    by_wallet_asset: dict[tuple[str, str, str], dict[str, dict[str, Any]]] = defaultdict(_empty_breakdown)
    by_asset: dict[str, dict[str, dict[str, Any]]] = defaultdict(_empty_breakdown)
    totals: dict[str, dict[str, Any]] = _empty_breakdown()
    wallet_label_by_id: dict[str, str] = {}
    wallet_total_inbound: dict[tuple[str, str, str], int] = defaultdict(int)
    asset_total_inbound: dict[str, int] = defaultdict(int)
    tx_count_total = 0
    inbound_total_msat = sum(int(row["amount"]) for row in inbound_rows)
    inbound_total_count = len(inbound_rows)
    truncated = False
    not_classified_msat = 0
    not_classified_count = 0

    for index, row in enumerate(inbound_rows):
        tx_id = row["id"]
        wallet_id = row["wallet_id"]
        wallet_label = row["wallet_label"]
        asset = row["asset"]
        amount = int(row["amount"])
        wallet_label_by_id[wallet_id] = wallet_label
        wallet_key = (wallet_id, wallet_label, asset)

        if index >= max_transactions:
            truncated = True
            not_classified_count += 1
            not_classified_msat += amount
            by_wallet_asset[wallet_key]["not_classified"]["amount_msat"] += amount
            by_wallet_asset[wallet_key]["not_classified"]["tx_count"] += 1
            by_asset[asset]["not_classified"]["amount_msat"] += amount
            by_asset[asset]["not_classified"]["tx_count"] += 1
            totals["not_classified"]["amount_msat"] += amount
            totals["not_classified"]["tx_count"] += 1
            wallet_total_inbound[wallet_key] += amount
            asset_total_inbound[asset] += amount
            tx_count_total += 1
            continue
        bucket = _classify_transaction(
            conn,
            workspace_ref,
            profile_ref,
            hooks,
            profile_id,
            tx_id,
            max_depth=max_depth,
        )

        by_wallet_asset[wallet_key][bucket]["amount_msat"] += amount
        by_wallet_asset[wallet_key][bucket]["tx_count"] += 1
        by_asset[asset][bucket]["amount_msat"] += amount
        by_asset[asset][bucket]["tx_count"] += 1
        totals[bucket]["amount_msat"] += amount
        totals[bucket]["tx_count"] += 1

        wallet_total_inbound[wallet_key] += amount
        asset_total_inbound[asset] += amount
        tx_count_total += 1

    by_wallet_out: list[dict[str, Any]] = []
    for (wallet_id, wallet_label, asset), buckets in sorted(
        by_wallet_asset.items(),
        key=lambda item: (item[0][1], item[0][2], item[0][0]),
    ):
        total_msat = wallet_total_inbound[(wallet_id, wallet_label, asset)]
        by_wallet_out.append(
            {
                "wallet_id": wallet_id,
                "wallet_label": wallet_label,
                "asset": asset,
                "buckets": _materialize(buckets),
                "total_inbound_msat": total_msat,
                "total_inbound": _btc(total_msat),
            }
        )

    by_asset_out: list[dict[str, Any]] = []
    for asset, buckets in sorted(by_asset.items()):
        total_msat = asset_total_inbound[asset]
        by_asset_out.append(
            {
                "asset": asset,
                "buckets": _materialize(buckets),
                "total_inbound_msat": total_msat,
                "total_inbound": _btc(total_msat),
            }
        )

    totals_msat = sum(int(values["amount_msat"]) for values in totals.values())
    return {
        "by_wallet": by_wallet_out,
        "by_asset": by_asset_out,
        "totals": {
            "buckets": _materialize(totals),
            "tx_count": tx_count_total,
            "amount_msat": totals_msat,
            "amount": _btc(totals_msat),
        },
        "limits": {
            "max_depth": max_depth,
            "max_transactions": max_transactions,
        },
        "truncation": {
            "truncated": truncated,
            "inbound_total_count": inbound_total_count,
            "inbound_total_msat": inbound_total_msat,
            "inbound_total": _btc(inbound_total_msat),
            "not_classified_count": not_classified_count,
            "not_classified_msat": not_classified_msat,
            "not_classified": _btc(not_classified_msat),
        },
    }


def coverage_summary_text(coverage: Mapping[str, Any]) -> list[str]:
    """Render a CLI-friendly coverage summary as plain lines."""
    lines: list[str] = []
    totals = coverage.get("totals", {})
    buckets = totals.get("buckets", {})
    tx_count = int(totals.get("tx_count", 0))
    truncation = coverage.get("truncation") or {}
    if truncation.get("truncated"):
        not_classified_count = int(truncation.get("not_classified_count", 0))
        inbound_total_count = int(truncation.get("inbound_total_count", 0))
        lines.append(
            f"Coverage truncated: classified {tx_count} of {inbound_total_count} inbound tx; "
            f"{not_classified_count} not classified."
        )
    lines.append(f"Inbound transactions: {tx_count}")
    for name in COVERAGE_BUCKETS:
        bucket = buckets.get(name, {})
        amount = float(bucket.get("amount", 0.0))
        count = int(bucket.get("tx_count", 0))
        lines.append(f"  {name}: {amount:.8f} ({count} tx)")
    by_asset = coverage.get("by_asset") or []
    if by_asset:
        lines.append("")
        lines.append("By asset:")
        for entry in by_asset:
            asset = entry.get("asset", "?")
            asset_buckets = entry.get("buckets", {})
            lines.append(f"  {asset}:")
            for name in COVERAGE_BUCKETS:
                bucket = asset_buckets.get(name, {})
                amount = float(bucket.get("amount", 0.0))
                count = int(bucket.get("tx_count", 0))
                lines.append(f"    {name}: {amount:.8f} ({count} tx)")
    return lines
