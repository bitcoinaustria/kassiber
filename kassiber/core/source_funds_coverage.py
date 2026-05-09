"""Source-funds coverage view.

Aggregates each profile's inbound transactions into four buckets so users
can see, at a glance, how much of their holdings is actually traced
versus how much is still in review or untraced. This is the
proactive-counterpart to the existing reactive review queue: instead of
"here are 47 suggestions to look at", it answers "of your X BTC
holdings, Y is fully traced and Z still needs evidence".

Buckets:
- ``fully_traced``: walking back along reviewed links reaches at least
  one non-attestation root source (e.g. a fiat purchase, exchange
  withdrawal, mining payout).
- ``attested``: reachable roots are only attestation source types
  (``missing_history``, ``opening_balance_attestation``). Acceptable
  evidence for some recipients, weaker than primary documents.
- ``in_review``: there are suggestions, or partial reviewed coverage,
  but no reviewed path to a root source yet.
- ``untraced``: no links exist for this transaction at all.

The classifier is intentionally O(links + transactions) per profile and
reuses the same reviewed-link traversal shape as ``build_report``.
Bounded by ``MAX_DEPTH`` to match the report walker.
"""

from __future__ import annotations

import sqlite3
from collections import defaultdict
from typing import Any, Mapping

from ..msat import msat_to_btc
from .source_funds import ATTESTATION_SOURCE_TYPES


COVERAGE_BUCKETS = ("fully_traced", "attested", "in_review", "untraced")
DEFAULT_MAX_DEPTH = 16


def _btc(msat: int | None) -> float:
    if msat is None:
        return 0.0
    return float(msat_to_btc(int(msat)))


def _classify_transaction(
    conn: sqlite3.Connection,
    profile_id: str,
    target_tx_id: str,
    *,
    max_depth: int = DEFAULT_MAX_DEPTH,
) -> str:
    """Return the coverage bucket for one inbound transaction.

    The walk follows non-rejected links upward. ``visited`` is a tx-id
    set so we never spin on cycles (the report builder enforces no
    cycles via ``path_cycle``, but the classifier should be defensive
    anyway because reviewed-state changes asynchronously).
    """
    queue: list[tuple[str, int]] = [(target_tx_id, 0)]
    visited: set[str] = set()
    saw_any_link = False
    saw_suggestion = False
    saw_reviewed_real_source = False
    saw_reviewed_attestation = False
    saw_reviewed_parent_without_source = False

    while queue:
        tx_id, depth = queue.pop()
        if tx_id in visited:
            continue
        visited.add(tx_id)
        if depth > max_depth:
            saw_suggestion = saw_suggestion or False
            continue

        rows = conn.execute(
            """
            SELECT id, from_source_id, from_transaction_id, state
            FROM source_funds_links
            WHERE profile_id = ? AND to_transaction_id = ? AND state != 'rejected'
            """,
            (profile_id, tx_id),
        ).fetchall()
        if not rows:
            continue
        saw_any_link = True

        for row in rows:
            if row["state"] == "suggested":
                saw_suggestion = True
                continue
            # state == "reviewed"
            if row["from_source_id"]:
                source_row = conn.execute(
                    "SELECT source_type FROM source_funds_sources WHERE id = ?",
                    (row["from_source_id"],),
                ).fetchone()
                if source_row is None:
                    continue
                if source_row["source_type"] in ATTESTATION_SOURCE_TYPES:
                    saw_reviewed_attestation = True
                else:
                    saw_reviewed_real_source = True
            elif row["from_transaction_id"]:
                queue.append((row["from_transaction_id"], depth + 1))
                saw_reviewed_parent_without_source = True

    if saw_reviewed_real_source:
        return "fully_traced"
    if saw_reviewed_attestation and not saw_suggestion:
        return "attested"
    if not saw_any_link:
        return "untraced"
    # Reviewed parents but no terminal source reached, or only suggestions left.
    if saw_reviewed_parent_without_source and not saw_reviewed_real_source and not saw_reviewed_attestation:
        return "in_review"
    return "in_review"


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
    profile_id: str,
    *,
    max_depth: int = DEFAULT_MAX_DEPTH,
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
    """
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

    for row in inbound_rows:
        tx_id = row["id"]
        wallet_id = row["wallet_id"]
        wallet_label = row["wallet_label"]
        asset = row["asset"]
        amount = int(row["amount"])
        wallet_label_by_id[wallet_id] = wallet_label
        bucket = _classify_transaction(conn, profile_id, tx_id, max_depth=max_depth)

        wallet_key = (wallet_id, wallet_label, asset)
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
    }


def coverage_summary_text(coverage: Mapping[str, Any]) -> list[str]:
    """Render a CLI-friendly coverage summary as plain lines."""
    lines: list[str] = []
    totals = coverage.get("totals", {})
    buckets = totals.get("buckets", {})
    tx_count = int(totals.get("tx_count", 0))
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
