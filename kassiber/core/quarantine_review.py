"""Explain the stored quarantine: its cause, root, evidence and next action.

This reads the current derived journal state (``journal_quarantines``,
``journal_quantity_issues`` and ``journal_quantity_postings``) plus the book's
wallets. It performs no network I/O and never changes accounting state: every
action it lists points at an existing, separately confirmed workflow.

The classification vocabulary lives in :mod:`kassiber.core.quarantine_catalog`.
"""

from __future__ import annotations

import json
import sqlite3
from decimal import Decimal, InvalidOperation
from typing import Any, Mapping

from ..redaction import redact_operational_text
from . import custody_journal
from . import freshness as core_freshness
from . import quarantine_catalog as catalog
from .transaction_kinds import INBOUND_KIND_TO_RP2_TYPE

MAX_GROUPS = 50
MAX_ASSUMPTION_ITEMS = 50
# A cause names this many of its root transactions; the count covers the rest.
MAX_GROUP_ROOTS = 25

# Which rows a page lists. ``attention`` is what the user can act on: the root
# causes, plus any downstream row whose root could not be named (hiding those
# would make an unexplained hold look like nothing). ``waiting`` is every row
# that only follows a named root and clears with it.
SCOPE_ALL = "all"
SCOPE_ATTENTION = "attention"
SCOPE_WAITING = "waiting"
SCOPES = (SCOPE_ALL, SCOPE_ATTENTION, SCOPE_WAITING)

# Outbound kinds that carry no economic meaning of their own. Such a row with
# no owned destination is booked as a disposal only by presumption.
_GENERIC_OUTBOUND_KINDS = frozenset(
    {"", "withdrawal", "withdraw", "send", "sent", "transfer", "transfer_out", "unknown"}
)


def _parse_detail(value: Any) -> dict[str, Any]:
    if isinstance(value, Mapping):
        return dict(value)
    try:
        parsed = json.loads(value or "{}")
    except (TypeError, ValueError):
        return {}
    return parsed if isinstance(parsed, dict) else {}


def _asset_msat(value: Any) -> int | None:
    """Convert an engine detail quantity (decimal asset units) to msat."""

    if value is None or isinstance(value, bool):
        return None
    try:
        amount = Decimal(str(value))
    except (InvalidOperation, ValueError):
        return None
    if not amount.is_finite():
        return None
    return int((amount * Decimal(100_000_000_000)).to_integral_value())


def _normalized_kind(row: Mapping[str, Any]) -> str:
    kind = row["kind_override"] if row["kind_override"] not in (None, "") else row["kind"]
    return str(kind or "").strip().lower().replace("-", "_").replace(" ", "_")


def _wallets(conn: sqlite3.Connection, profile_id: str) -> dict[str, dict[str, Any]]:
    wallets: dict[str, dict[str, Any]] = {}
    for row in conn.execute(
        "SELECT id, label, config_json FROM wallets WHERE profile_id = ?",
        (profile_id,),
    ):
        config = _parse_detail(row["config_json"])
        wallets[str(row["id"])] = {
            "id": str(row["id"]),
            "label": str(row["label"]),
            "deprecated": bool(config.get("deprecated")),
        }
    return wallets


def _blocking_transaction_ids(conn: sqlite3.Connection, profile_id: str) -> set[str]:
    blocking: set[str] = set()
    try:
        rows = conn.execute(
            "SELECT transaction_ids_json FROM journal_quantity_issues WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
    except sqlite3.OperationalError:
        return blocking
    for row in rows:
        try:
            ids = json.loads(row["transaction_ids_json"] or "[]")
        except (TypeError, ValueError):
            continue
        if isinstance(ids, list):
            blocking.update(str(item) for item in ids if item)
    return blocking


def pairs_by_transaction(conn: sqlite3.Connection, profile_id: str) -> dict[str, Mapping[str, Any]]:
    """Each paired transaction's current pair review, keyed by either leg."""

    from . import custody_authored_migration

    try:
        records = custody_authored_migration.list_pair_review_records(
            conn, profile_id=profile_id
        )
    except sqlite3.OperationalError:
        return {}
    pairs: dict[str, Mapping[str, Any]] = {}
    for record in records:
        for key in ("out_transaction_id", "in_transaction_id"):
            if record.get(key):
                pairs.setdefault(str(record[key]), record)
    return pairs


def pairs_by_id(conn: sqlite3.Connection, profile_id: str) -> dict[str, Mapping[str, Any]]:
    """Every current pair review, by its id."""

    return {str(pair["id"]): pair for pair in pairs_by_transaction(conn, profile_id).values()}


def pair_evidence(transaction_id: str, pair: Mapping[str, Any]) -> dict[str, Any]:
    """What a suspense-holding pair looks like, for the user to judge it.

    Two legs of one movement between your wallets share one on-chain txid and
    the receipt cannot come before the spend; a pair that breaks either is
    most likely two unrelated transactions joined by mistake.
    """

    out_id = str(pair.get("out_transaction_id") or "")
    in_id = str(pair.get("in_transaction_id") or "")
    evidence: dict[str, Any] = {
        "pair_id": str(pair["id"]),
        "pair_counterpart_transaction_id": in_id if transaction_id == out_id else out_id,
        # Both sides as the book holds them, so the owner can compare them.
        "pair_legs": {
            side: {
                "transaction_id": str(pair.get(f"{side}_transaction_id") or ""),
                "wallet": str(pair.get(f"{side}_wallet") or ""),
                "asset": str(pair.get(f"{side}_asset") or ""),
                "amount_msat": int(pair.get(f"{side}_full_amount_msat") or 0),
                "occurred_at": pair.get(f"{side}_occurred_at"),
                "external_id": str(pair.get(f"{side}_external_id") or ""),
            }
            for side in ("out", "in")
        },
    }
    out_txid = _canonical_txid(pair.get("out_external_id"))
    in_txid = _canonical_txid(pair.get("in_external_id"))
    if out_txid and in_txid and _same_chain_assets(pair):
        evidence["pair_txids_differ"] = out_txid != in_txid
    out_at = str(pair.get("out_occurred_at") or "")
    in_at = str(pair.get("in_occurred_at") or "")
    if out_at and in_at:
        evidence["pair_receipt_before_spend"] = in_at < out_at
    return evidence


def _canonical_txid(value: Any) -> str | None:
    text = str(value or "").strip().lower()
    if len(text) == 64 and all(char in "0123456789abcdef" for char in text):
        return text
    return None


def _same_chain_assets(pair: Mapping[str, Any]) -> bool:
    # A peg or swap moves between chains with two txids by design.
    return str(pair.get("out_asset") or "").upper() == str(pair.get("in_asset") or "").upper()


def _journal_freshness(conn: sqlite3.Connection, profile: Mapping[str, Any]) -> dict[str, Any]:
    state = custody_journal.projection_freshness(conn, str(profile["id"]))
    source = core_freshness.get_source_state(
        conn, str(profile["id"]), core_freshness.journal_source_key(str(profile["id"]))
    )
    last_error = None
    if source and source.get("last_error_at"):
        error_at = str(source["last_error_at"])
        success_at = str(source.get("last_success_at") or "")
        processed_at = str(state.get("last_processed_at") or "")
        if error_at > success_at and error_at > processed_at:
            last_error = {
                "code": str(source.get("last_error_code") or "journal_refresh_failed"),
                "message": redact_operational_text(
                    str(source.get("last_error_message") or "Journal processing failed.")
                ),
                "at": error_at,
            }
    return {
        "needs_processing": bool(state.get("needs_processing")),
        "status": state.get("status"),
        "last_processed_at": state.get("last_processed_at"),
        "last_error": last_error,
    }


def _evidence(
    reason: str,
    detail: Mapping[str, Any],
    row: Mapping[str, Any],
    wallets: Mapping[str, Mapping[str, Any]],
) -> dict[str, Any]:
    evidence: dict[str, Any] = {}
    wallet_label = detail.get("wallet") or detail.get("from_wallet") or row["wallet"]
    if wallet_label:
        evidence["wallet_label"] = str(wallet_label)
    missing_ids = detail.get("missing_source_wallet_ids")
    if isinstance(missing_ids, list) and missing_ids:
        evidence["missing_source_wallets"] = [
            {
                "id": str(wallet_id),
                "label": str(wallets.get(str(wallet_id), {}).get("label") or wallet_id),
                "deprecated": bool(wallets.get(str(wallet_id), {}).get("deprecated")),
            }
            for wallet_id in missing_ids
        ]
    if detail.get("blocker_code"):
        evidence["blocker_code"] = str(detail["blocker_code"])
        evidence["blocker_codes"] = [
            str(code) for code in (detail.get("blocker_codes") or [detail["blocker_code"]])
        ]
    gap_ids = detail.get("gap_ids")
    if isinstance(gap_ids, list) and gap_ids:
        evidence["gap_id"] = str(gap_ids[0])
    # Engine quantities are decimal units of the row's asset; only Bitcoin
    # units convert to msat. Other assets keep their amounts out of evidence.
    if str(detail.get("asset") or row["asset"] or "").upper() in {"BTC", "LBTC"}:
        required = _asset_msat(detail.get("required"))
        if required is not None:
            evidence["required_msat"] = required
        available = _asset_msat(
            detail.get("available", detail.get("priced_available"))
        )
        if available is not None:
            evidence["available_msat"] = available
    if detail.get("lot_state_uncertain_since"):
        evidence["lot_state_uncertain_since"] = str(detail["lot_state_uncertain_since"])
    counterparts = [
        str(detail[key])
        for key in ("out_transaction_id", "in_transaction_id")
        if detail.get(key) and str(detail[key]) != str(row["transaction_id"])
    ]
    for key in ("outbound_ids", "blocked_by_transaction_ids"):
        values = detail.get(key)
        if isinstance(values, list):
            counterparts.extend(
                str(item) for item in values if str(item) != str(row["transaction_id"])
            )
    if counterparts:
        evidence["counterpart_transaction_ids"] = sorted(set(counterparts))
    if detail.get("blocked_by_reason"):
        evidence["blocked_by_reason"] = str(detail["blocked_by_reason"])
    for key in ("row_amount_msat", "owned_receipts_msat", "owned_outputs_msat"):
        if isinstance(detail.get(key), int):
            evidence[key] = int(detail[key])
    return evidence


def _actions(
    info: catalog.ReasonInfo,
    item: Mapping[str, Any],
    evidence: Mapping[str, Any],
    root: Mapping[str, Any] | None,
    wallets: Mapping[str, Mapping[str, Any]],
) -> list[dict[str, Any]]:
    actions: list[dict[str, Any]] = []
    transaction_id = str(item["transaction_id"])
    own_wallet = wallets.get(str(item["wallet_id"] or ""), {})
    for kind in info.actions:
        if kind == catalog.ACTION_SYNC_WALLET:
            targets = evidence.get("missing_source_wallets") or (
                [own_wallet] if own_wallet else []
            )
            for wallet in targets:
                actions.append(
                    {"kind": kind, "wallet_id": wallet["id"], "wallet_label": wallet["label"]}
                )
        elif kind == catalog.ACTION_IMPORT_HISTORY:
            target = (evidence.get("missing_source_wallets") or [own_wallet or None])[0]
            if target:
                actions.append(
                    {"kind": kind, "wallet_id": target["id"], "wallet_label": target["label"]}
                )
            else:
                actions.append({"kind": kind})
        elif kind == catalog.ACTION_REVIEW_CUSTODY_GAP:
            if evidence.get("gap_id"):
                actions.append({"kind": kind, "gap_id": evidence["gap_id"]})
        elif kind == catalog.ACTION_RESOLVE_ROOT:
            if root is not None:
                actions.append({"kind": kind, "transaction_id": root["transaction_id"]})
        elif kind == catalog.ACTION_REVIEW_PAIR:
            if evidence.get("pair_id"):
                actions.append(
                    {
                        "kind": kind,
                        "transaction_id": transaction_id,
                        "pair_id": evidence["pair_id"],
                    }
                )
        elif kind in {catalog.ACTION_CONNECT_WALLET, catalog.ACTION_WAIT_FOR_CONFIRMATION}:
            actions.append({"kind": kind})
        else:
            actions.append({"kind": kind, "transaction_id": transaction_id})
    return actions


def _group_key(item: Mapping[str, Any]) -> str:
    reason = str(item["reason"])
    evidence = item["evidence"]
    if reason == "custody_quantity_unresolved":
        return f"{reason}:{evidence.get('blocker_code', '')}:{evidence.get('gap_id', '')}"
    if reason == "ownership_transfer_source_missing":
        wallet_ids = sorted(
            wallet["id"] for wallet in evidence.get("missing_source_wallets", ())
        )
        return f"{reason}:{','.join(wallet_ids)}"
    if reason in {"insufficient_lots", "missing_cost_basis"}:
        return f"{reason}:{item['asset']}:{evidence.get('wallet_label', '')}"
    return reason


def _root_summary(row: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "transaction_id": str(row["transaction_id"]),
        "reason": str(row["reason"]),
        "occurred_at": row["occurred_at"],
        "wallet": row["wallet"],
        "external_id": row["external_id"] or "",
    }


def _assumptions(
    conn: sqlite3.Connection,
    profile_id: str,
    quarantined: set[str],
) -> dict[str, Any]:
    """Bitcoin outflows and inflows booked by presumption rather than evidence.

    An unmatched outflow is booked as a disposal because no owned destination
    is known; an inflow without an explicit economic kind is booked as a
    purchase at market value. Neither is a quarantine; both change tax results
    when the presumption is wrong (for example an unconnected own wallet).
    """

    def collect(state: str, direction: str, generic: Any) -> dict[str, Any]:
        try:
            rows = conn.execute(
                """
                SELECT p.transaction_id, SUM(ABS(p.amount_msat)) AS amount_msat,
                       t.occurred_at, t.external_id, t.kind, t.kind_override,
                       w.label AS wallet
                FROM journal_quantity_postings p
                JOIN transactions t ON t.id = p.transaction_id
                JOIN wallets w ON w.id = t.wallet_id
                WHERE p.profile_id = ? AND p.state = ? AND t.direction = ?
                  AND t.excluded = 0 AND upper(t.asset) IN ('BTC', 'LBTC')
                GROUP BY p.transaction_id
                ORDER BY t.occurred_at ASC, p.transaction_id ASC
                """,
                (profile_id, state, direction),
            ).fetchall()
        except sqlite3.OperationalError:
            rows = []
        items = [
            row
            for row in rows
            if str(row["transaction_id"]) not in quarantined and generic(_normalized_kind(row))
        ]
        return {
            "count": len(items),
            "amount_msat": sum(int(row["amount_msat"] or 0) for row in items),
            "items": [
                {
                    "transaction_id": str(row["transaction_id"]),
                    "occurred_at": row["occurred_at"],
                    "wallet": row["wallet"],
                    "amount_msat": int(row["amount_msat"] or 0),
                    "external_id": row["external_id"] or "",
                }
                for row in items[:MAX_ASSUMPTION_ITEMS]
            ],
        }

    return {
        "presumed_external_outbound": collect(
            "external_presumed",
            "outbound",
            lambda kind: kind in _GENERIC_OUTBOUND_KINDS,
        ),
        "unclassified_inbound": collect(
            "unclassified_origin",
            "inbound",
            lambda kind: kind not in INBOUND_KIND_TO_RP2_TYPE,
        ),
    }


def review_quarantine(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    *,
    limit: int,
    offset: int = 0,
    scope: str = SCOPE_ALL,
) -> dict[str, Any]:
    """Return the explained quarantine page plus whole-book summaries.

    ``scope`` picks which rows the page lists (see :data:`SCOPES`); every
    summary still covers the whole book.
    """

    if scope not in SCOPES:
        raise ValueError(f"unknown quarantine scope: {scope}")
    profile_id = str(profile["id"])
    wallets = _wallets(conn, profile_id)
    blocking = _blocking_transaction_ids(conn, profile_id)
    pairs = pairs_by_transaction(conn, profile_id)
    rows = conn.execute(
        """
        SELECT
            q.transaction_id, q.reason, q.detail_json, q.created_at,
            t.external_id, t.occurred_at, t.confirmed_at, t.direction,
            t.asset, t.amount, t.fee, t.wallet_id,
            w.label AS wallet
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ?
        """,
        (profile_id,),
    ).fetchall()
    by_id = {str(row["transaction_id"]): row for row in rows}
    details = {str(row["transaction_id"]): _parse_detail(row["detail_json"]) for row in rows}

    def is_root_row(transaction_id: str) -> bool:
        row = by_id.get(transaction_id)
        return row is not None and not catalog.is_downstream(str(row["reason"]))

    def resolve_root(
        transaction_id: str,
        reason: str,
        detail: Mapping[str, Any],
        visited: frozenset[str] = frozenset(),
    ) -> str | None:
        visited = visited | {transaction_id}
        candidates: list[str] = []
        if reason == "custody_basis_barrier":
            candidates = [str(item) for item in detail.get("root_transaction_ids") or []]
        elif reason == "transfer_pair_dependency_blocked":
            candidates = [str(item) for item in detail.get("blocked_by_transaction_ids") or []]
        elif reason == "derived_transfer_group_blocked":
            group_id = detail.get("transfer_group_id")
            candidates = [
                other_id
                for other_id, other in details.items()
                if group_id and other.get("transfer_group_id") == group_id
            ]
        elif reason == "basis_provenance_incomplete":
            # The engine records only the contamination timestamp. Name a root
            # only when exactly one root row of this asset sits there; a
            # guess could send the owner to an unrelated batched row.
            since = detail.get("lot_state_uncertain_since")
            asset = by_id[transaction_id]["asset"]
            matches = [
                other_id
                for other_id, other in by_id.items()
                if since
                and other["occurred_at"] == since
                and other["asset"] == asset
                and is_root_row(other_id)
            ]
            candidates = matches if len(matches) == 1 else []
        roots = [
            candidate
            for candidate in candidates
            if candidate not in visited and is_root_row(candidate)
        ]
        if not roots:
            # Dependencies can chain (A holds B, B holds C): follow a
            # downstream candidate to its own root, stopping at cycles.
            for candidate in candidates:
                if candidate in visited or candidate not in by_id:
                    continue
                nested = resolve_root(
                    candidate,
                    str(by_id[candidate]["reason"]),
                    details[candidate],
                    visited,
                )
                if nested is not None and nested not in visited:
                    roots.append(nested)
        if not roots:
            return None
        return min(roots, key=lambda item: (str(by_id[item]["occurred_at"] or ""), item))

    items: list[dict[str, Any]] = []
    for row in rows:
        transaction_id = str(row["transaction_id"])
        reason = str(row["reason"])
        detail = details[transaction_id]
        info = catalog.reason_info(reason, detail)
        root_id = resolve_root(transaction_id, reason, detail) if info.downstream else None
        root = _root_summary(by_id[root_id]) if root_id is not None else None
        evidence = _evidence(reason, detail, row, wallets)
        if evidence.get("blocker_code") == "reviewed_residual_suspense" and transaction_id in pairs:
            evidence.update(pair_evidence(transaction_id, pairs[transaction_id]))
        additional = [
            str(entry.get("reason"))
            for entry in detail.get("additional_reasons") or []
            if isinstance(entry, Mapping) and entry.get("reason")
        ]
        item = {
            "transaction_id": transaction_id,
            "external_id": row["external_id"] or "",
            "occurred_at": row["occurred_at"],
            "confirmed_at": row["confirmed_at"],
            "wallet": row["wallet"],
            "wallet_id": row["wallet_id"],
            "direction": row["direction"],
            "asset": row["asset"],
            "amount_msat": int(row["amount"] or 0),
            "fee_msat": int(row["fee"] or 0),
            "reason": reason,
            "detail": detail,
            "created_at": row["created_at"],
            "category": info.category,
            "blocks_reports": transaction_id in blocking,
            "is_downstream": info.downstream,
            "root": root,
            "reasons": [reason, *[item for item in additional if item != reason]],
            "evidence": evidence,
        }
        item["actions"] = _actions(info, item, evidence, root, wallets)
        # Wallet ids travel only inside the actions that need them.
        item.pop("wallet_id")
        items.append(item)

    items.sort(
        key=lambda item: (
            1 if item["is_downstream"] else 0,
            0 if item["blocks_reports"] else 1,
            str(item["occurred_at"] or ""),
            item["transaction_id"],
        )
    )

    items_by_id = {item["transaction_id"]: item for item in items}
    groups: dict[str, dict[str, Any]] = {}
    for item in items:
        anchor = item["root"]["transaction_id"] if item["root"] else None
        if item["is_downstream"] and anchor is not None:
            anchor_item = items_by_id.get(anchor)
            key = _group_key(anchor_item) if anchor_item else f"downstream:{item['reason']}"
        elif item["is_downstream"]:
            key = f"downstream:{item['reason']}"
        else:
            key = _group_key(item)
        item["group_key"] = key
        group = groups.get(key)
        if group is None:
            group = groups[key] = {
                "key": key,
                "category": item["category"],
                "reason": item["reason"],
                "root_transaction_id": None,
                "root_occurred_at": None,
                "root_wallet": None,
                "root_external_id": None,
                "root_amount_msat": None,
                "root_direction": None,
                "root_asset": None,
                "count": 0,
                "downstream_count": 0,
                "blocks_reports": False,
                "wallets": [],
                "earliest_occurred_at": None,
                "evidence": {},
                "actions": [],
                "root_transaction_ids": [],
                "root_count": 0,
            }
        group["count"] += 1
        if item["is_downstream"]:
            group["downstream_count"] += 1
        else:
            group["root_count"] += 1
            if len(group["root_transaction_ids"]) < MAX_GROUP_ROOTS:
                group["root_transaction_ids"].append(item["transaction_id"])
            if group["root_transaction_id"] is None:
                group.update(
                    {
                        "category": item["category"],
                        "reason": item["reason"],
                        "root_transaction_id": item["transaction_id"],
                        "root_occurred_at": item["occurred_at"],
                        "root_wallet": item["wallet"],
                        "root_external_id": item["external_id"],
                        "root_amount_msat": item["amount_msat"],
                        "root_direction": item["direction"],
                        "root_asset": item["asset"],
                        "evidence": item["evidence"],
                        "actions": item["actions"],
                    }
                )
        group["blocks_reports"] = group["blocks_reports"] or item["blocks_reports"]
        if item["wallet"] and item["wallet"] not in group["wallets"] and len(group["wallets"]) < 5:
            group["wallets"].append(item["wallet"])
        occurred = item["occurred_at"]
        if occurred and (
            group["earliest_occurred_at"] is None or occurred < group["earliest_occurred_at"]
        ):
            group["earliest_occurred_at"] = occurred
    ordered_groups = sorted(
        groups.values(),
        key=lambda group: (
            0 if group["blocks_reports"] else 1,
            1 if group["category"] == catalog.CATEGORY_DOWNSTREAM else 0,
            str(group["earliest_occurred_at"] or ""),
            group["key"],
        ),
    )

    by_category: dict[str, int] = {}
    for item in items:
        by_category[item["category"]] = by_category.get(item["category"], 0) + 1

    def waiting(item: Mapping[str, Any]) -> bool:
        return bool(item["is_downstream"] and item["root"] is not None)

    waiting_count = sum(1 for item in items if waiting(item))
    listed = (
        items
        if scope == SCOPE_ALL
        else [item for item in items if waiting(item) == (scope == SCOPE_WAITING)]
    )

    return {
        "summary": {
            "freshness": _journal_freshness(conn, profile),
            "blocking_count": sum(1 for item in items if item["blocks_reports"]),
            "reports_blocked": bool(blocking),
            "by_category": [
                {"category": category, "count": by_category[category]}
                for category in catalog.CATEGORIES
                if category in by_category
            ],
            "groups": ordered_groups[:MAX_GROUPS],
            "group_count": len(ordered_groups),
            "assumptions": _assumptions(conn, profile_id, set(by_id)),
            "attention_count": len(items) - waiting_count,
            "waiting_count": waiting_count,
            "scope": scope,
            "scope_count": len(listed),
        },
        "items": listed[offset : offset + limit],
    }


__all__ = ["review_quarantine"]
