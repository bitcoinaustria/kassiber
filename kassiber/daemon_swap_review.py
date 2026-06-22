"""Deterministic swap-review context for daemon and AI tool calls."""

from __future__ import annotations

import json
import sqlite3
from typing import Any, Mapping

from .cli.handlers import (
    list_saved_views_cli,
    list_transaction_pairs,
    list_transfer_rules,
    resolve_scope,
    suggest_transfer_candidates,
)
from .core import transfer_matching as core_transfer_matching
from .errors import AppError
from .msat import msat_to_btc
from .redaction import redact_secret_text


SWAP_REVIEW_DEFAULT_LIMIT = 8
SWAP_REVIEW_MAX_LIMIT = 50

_SWAP_REVIEW_KEYWORDS = (
    "aqua",
    "boltz",
    "federation",
    "lbtc",
    "lightning",
    "liquid",
    "ln",
    "peg",
    "phoenix",
    "submarine",
    "swap",
    "tausch",
    "übertrag",
    "uebertrag",
)


def _swap_review_limit(args: dict[str, Any]) -> int:
    raw = args.get("limit", SWAP_REVIEW_DEFAULT_LIMIT)
    try:
        limit = int(raw)
    except (TypeError, ValueError) as exc:
        raise AppError(
            "ui.transfers.review_context limit must be an integer",
            code="validation",
        ) from exc
    if limit < 1:
        raise AppError(
            "ui.transfers.review_context limit must be positive",
            code="validation",
        )
    return min(limit, SWAP_REVIEW_MAX_LIMIT)


def _sql_placeholders(values: list[str]) -> str:
    return ",".join("?" for _ in values)


def _safe_review_text(value: Any, *, limit: int = 240) -> str:
    text = redact_secret_text(str(value or "").strip())
    if len(text) <= limit:
        return text
    return text[:limit].rstrip() + "...<truncated>"


def _review_keyword_mentions(fields: Mapping[str, Any]) -> list[dict[str, str]]:
    mentions: list[dict[str, str]] = []
    for field, raw_value in fields.items():
        if isinstance(raw_value, list):
            values = [str(item) for item in raw_value if item]
        else:
            values = [str(raw_value)] if raw_value else []
        for value in values:
            lowered = value.lower()
            for keyword in _SWAP_REVIEW_KEYWORDS:
                if keyword in lowered:
                    mentions.append(
                        {
                            "field": str(field),
                            "keyword": keyword,
                            "excerpt": _safe_review_text(value, limit=160),
                        }
                    )
                    break
    return mentions


def _transaction_summaries_for_review(
    conn: sqlite3.Connection,
    transaction_ids: list[str],
) -> dict[str, dict[str, Any]]:
    unique_ids = list(dict.fromkeys(transaction_ids))
    if not unique_ids:
        return {}
    rows = conn.execute(
        f"""
        SELECT
            t.id,
            t.external_id,
            t.occurred_at,
            t.confirmed_at,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_currency,
            t.fiat_rate,
            t.fiat_value,
            t.kind,
            t.description,
            t.counterparty,
            t.note,
            t.excluded,
            t.payment_hash,
            t.payment_hash_source,
            w.label AS wallet_label,
            w.kind AS wallet_kind
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.id IN ({_sql_placeholders(unique_ids)})
        """,
        tuple(unique_ids),
    ).fetchall()
    summaries: dict[str, dict[str, Any]] = {}
    for row in rows:
        tx_id = row["id"]
        summaries[tx_id] = {
            "transaction_id": tx_id,
            "external_id": row["external_id"] or "",
            "occurred_at": row["occurred_at"],
            "confirmed_at": row["confirmed_at"],
            "direction": row["direction"],
            "asset": row["asset"],
            "amount_msat": int(row["amount"] or 0),
            "amount": float(msat_to_btc(row["amount"] or 0)),
            "fee_msat": int(row["fee"] or 0),
            "fee": float(msat_to_btc(row["fee"] or 0)),
            "fiat_currency": row["fiat_currency"],
            "fiat_rate": row["fiat_rate"],
            "fiat_value": row["fiat_value"],
            "kind": row["kind"],
            "description": _safe_review_text(row["description"]),
            "counterparty": _safe_review_text(row["counterparty"]),
            "note": _safe_review_text(row["note"]),
            "excluded": bool(row["excluded"]),
            "payment_hash_present": bool(row["payment_hash"]),
            "payment_hash_source": row["payment_hash_source"],
            "wallet": {
                "label": row["wallet_label"],
                "kind": row["wallet_kind"],
            },
            "tags": [],
            "metadata_mentions": [],
        }

    tag_rows = conn.execute(
        f"""
        SELECT tt.transaction_id, tags.code, tags.label
        FROM transaction_tags tt
        JOIN tags ON tags.id = tt.tag_id
        WHERE tt.transaction_id IN ({_sql_placeholders(unique_ids)})
        ORDER BY tt.transaction_id ASC, tags.code ASC
        """,
        tuple(unique_ids),
    ).fetchall()
    for tag in tag_rows:
        summary = summaries.get(tag["transaction_id"])
        if summary is None:
            continue
        summary["tags"].append({"code": tag["code"], "label": tag["label"]})

    for summary in summaries.values():
        summary["metadata_mentions"] = _review_keyword_mentions(
            {
                "description": summary.get("description"),
                "counterparty": summary.get("counterparty"),
                "note": summary.get("note"),
                "tags": [
                    tag.get("label") or tag.get("code")
                    for tag in summary.get("tags", [])
                    if isinstance(tag, dict)
                ],
            }
        )
    return summaries


def _json_object_or_empty(raw: Any) -> dict[str, Any]:
    try:
        parsed = json.loads(raw or "{}")
    except (TypeError, ValueError):
        return {}
    return parsed if isinstance(parsed, dict) else {}


def _journal_entries_for_review(
    conn: sqlite3.Connection,
    transaction_ids: list[str],
) -> tuple[dict[str, list[dict[str, Any]]], dict[str, dict[str, Any]]]:
    unique_ids = list(dict.fromkeys(transaction_ids))
    if not unique_ids:
        return {}, {}
    journal_rows = conn.execute(
        f"""
        SELECT
            id,
            transaction_id,
            occurred_at,
            entry_type,
            asset,
            quantity,
            fiat_value,
            cost_basis,
            proceeds,
            gain_loss,
            at_category,
            at_kennzahl,
            description
        FROM journal_entries
        WHERE transaction_id IN ({_sql_placeholders(unique_ids)})
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        tuple(unique_ids),
    ).fetchall()
    journal_by_tx: dict[str, list[dict[str, Any]]] = {
        tx_id: [] for tx_id in unique_ids
    }
    for row in journal_rows:
        journal_by_tx.setdefault(row["transaction_id"], []).append(
            {
                "id": row["id"],
                "occurred_at": row["occurred_at"],
                "entry_type": row["entry_type"],
                "asset": row["asset"],
                "quantity_msat": int(row["quantity"] or 0),
                "quantity": float(msat_to_btc(row["quantity"] or 0)),
                "fiat_value": row["fiat_value"],
                "cost_basis": row["cost_basis"],
                "proceeds": row["proceeds"],
                "gain_loss": row["gain_loss"],
                "at_category": row["at_category"],
                "at_kennzahl": row["at_kennzahl"],
                "description": _safe_review_text(row["description"]),
            }
        )

    quarantine_rows = conn.execute(
        f"""
        SELECT transaction_id, reason, detail_json, created_at
        FROM journal_quarantines
        WHERE transaction_id IN ({_sql_placeholders(unique_ids)})
        """,
        tuple(unique_ids),
    ).fetchall()
    quarantines = {
        row["transaction_id"]: {
            "reason": row["reason"],
            "detail": _json_object_or_empty(row["detail_json"]),
            "created_at": row["created_at"],
        }
        for row in quarantine_rows
    }
    return journal_by_tx, quarantines


def _swap_review_confidence_reason(candidate: Mapping[str, Any]) -> dict[str, Any]:
    method = candidate.get("method")
    confidence = candidate.get("confidence")
    if method == "payment_hash":
        return {
            "confidence": confidence,
            "method": method,
            "reason": "both legs share a Lightning payment_hash",
            "needs_human_confirmation": False,
        }
    if method == "htlc_refund":
        return {
            "confidence": confidence,
            "method": method,
            "reason": (
                "the inbound refund spends the outbound's on-chain HTLC "
                "funding output (deterministic link, same-wallet safe)"
            ),
            "needs_human_confirmation": False,
        }
    return {
        "confidence": confidence,
        "method": method,
        "reason": "legs match by direction, amount delta, wallets, and time window",
        "needs_human_confirmation": True,
    }


def _swap_review_fee(
    candidate: Mapping[str, Any],
    *,
    fee_pct_max: float,
    fee_sats_min: int,
) -> dict[str, Any]:
    fee_msat = int(candidate.get("swap_fee_msat") or 0)
    out_msat = abs(int(candidate.get("out_amount_msat") or 0))
    threshold_msat = max(int(out_msat * fee_pct_max), fee_sats_min * 1000)
    pct_of_out = (fee_msat / out_msat) if out_msat else None
    if fee_msat < 0:
        assessment = "anomaly_inbound_exceeds_outbound"
    elif fee_msat == 0:
        assessment = "no_fee_detected"
    elif threshold_msat and fee_msat > threshold_msat:
        assessment = "above_default_heuristic_threshold"
    else:
        assessment = "normal"
    return {
        "swap_fee_msat": fee_msat,
        "swap_fee": float(msat_to_btc(fee_msat)),
        "swap_fee_kind": candidate.get("swap_fee_kind"),
        "pct_of_out": pct_of_out,
        "default_threshold_msat": threshold_msat,
        "assessment": assessment,
    }


def _swap_review_suggested_action(
    candidate: Mapping[str, Any],
    *,
    conflict_size: int,
    fee_assessment: str,
) -> dict[str, Any]:
    base_args = {
        "tx_out": candidate.get("out_id"),
        "tx_in": candidate.get("in_id"),
        "kind": candidate.get("default_kind"),
        "policy": candidate.get("default_policy"),
        "confidence_at_pair": candidate.get("confidence"),
    }
    if conflict_size > 1:
        return {
            "action": "resolve_conflict",
            "requires_consent": False,
            "reason": "candidate shares a leg with other candidates",
        }
    if fee_assessment == "anomaly_inbound_exceeds_outbound":
        return {
            "action": "inspect_before_pairing",
            "requires_consent": False,
            "reason": (
                "computed fee is negative, so the legs may be reversed "
                "or mismatched"
            ),
        }
    if candidate.get("default_policy") == "taxable":
        return {
            "action": "pair_taxable_after_confirmation",
            "daemon_kind": "ui.transfers.pair",
            "arguments": base_args,
            "requires_consent": True,
            "reason": (
                "non-Austrian profiles keep cross-asset swaps as SELL + BUY "
                "while recording the audit link"
            ),
        }
    if candidate.get("confidence") == "exact":
        reason = (
            "the inbound refund deterministically spends the outbound's HTLC "
            "funding output, and is non-conflicted"
            if candidate.get("method") == "htlc_refund"
            else "payment_hash identity is exact and non-conflicted"
        )
        return {
            "action": "bulk_pair_exact_or_pair",
            "daemon_kind": "ui.transfers.bulk_pair",
            "arguments": {"confidence": "exact"},
            "requires_consent": True,
            "reason": reason,
        }
    return {
        "action": "ask_user_to_confirm_then_pair",
        "daemon_kind": "ui.transfers.pair",
        "arguments": base_args,
        "requires_consent": True,
        "reason": "heuristic candidates need human confirmation before pairing",
    }


def _swap_review_report_impact(
    *,
    out_entries: list[dict[str, Any]],
    in_entries: list[dict[str, Any]],
    quarantines: list[dict[str, Any]],
    default_policy: str,
) -> dict[str, Any]:
    entry_types = sorted(
        {
            str(entry.get("entry_type"))
            for entry in [*out_entries, *in_entries]
            if entry.get("entry_type")
        }
    )
    if quarantines:
        status = "blocked_by_quarantine"
        summary = (
            "one or both legs are quarantined, so report impact is not "
            "trustworthy yet"
        )
    elif not out_entries and not in_entries:
        status = "no_current_journal_entries"
        summary = "journals do not currently expose entries for these legs"
    elif default_policy == "carrying-value":
        status = "would_remain_separate_until_paired"
        summary = (
            "leaving this unpaired keeps the legs as separate journal events; "
            "pairing preserves principal and surfaces the swap fee"
        )
    else:
        status = "audit_link_only_by_default"
        summary = (
            "default policy records a reviewed link but leaves SELL + BUY tax "
            "treatment in place"
        )
    return {
        "status": status,
        "summary": summary,
        "entry_types": entry_types,
        "out_entry_count": len(out_entries),
        "in_entry_count": len(in_entries),
        "quarantine_count": len(quarantines),
    }


def build_swap_review_context_payload(
    conn: sqlite3.Connection,
    args: dict[str, Any],
) -> dict[str, Any]:
    allowed = {
        "workspace",
        "profile",
        "limit",
        "confidence",
        "method",
        "asset_pair",
        "route_pair",
        "candidate_type",
        "time_window_seconds",
        "fee_pct_max",
        "fee_sats_min",
    }
    unknown = sorted(set(args) - allowed)
    if unknown:
        raise AppError(
            "ui.transfers.review_context received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    workspace = args.get("workspace")
    profile_ref = args.get("profile")
    workspace_row, profile = resolve_scope(conn, workspace, profile_ref)
    limit = _swap_review_limit(args)
    fee_pct_max = float(
        args.get("fee_pct_max") or core_transfer_matching.DEFAULT_FEE_PCT_MAX
    )
    fee_sats_min = int(
        args.get("fee_sats_min") or core_transfer_matching.DEFAULT_FEE_SATS_MIN
    )
    candidate_payload = suggest_transfer_candidates(
        conn,
        workspace,
        profile_ref,
        time_window_seconds=int(
            args.get("time_window_seconds")
            or core_transfer_matching.DEFAULT_TIME_WINDOW_SECONDS
        ),
        fee_pct_max=fee_pct_max,
        fee_sats_min=fee_sats_min,
        confidence=args.get("confidence"),
        asset_pair=args.get("asset_pair"),
        route_pair=args.get("route_pair"),
        method=args.get("method"),
        candidate_type=args.get("candidate_type") or "swap",
    )
    candidates = list(candidate_payload.get("candidates", []))
    limited_candidates = candidates[:limit]

    tx_ids: list[str] = []
    for candidate in limited_candidates:
        for key in ("out_id", "in_id"):
            value = candidate.get(key)
            if isinstance(value, str) and value:
                tx_ids.append(value)
    tx_summaries = _transaction_summaries_for_review(conn, tx_ids)
    journal_by_tx, quarantines_by_tx = _journal_entries_for_review(conn, tx_ids)

    review_items: list[dict[str, Any]] = []
    for candidate in limited_candidates:
        out_id = str(candidate.get("out_id") or "")
        in_id = str(candidate.get("in_id") or "")
        conflict_id = str(candidate.get("conflict_set_id") or "")
        # Matcher-stamped over the full candidate set; the filters applied
        # above must not shrink it, or hidden siblings would be ignored.
        conflict_size = int(candidate.get("conflict_size") or 0)
        fee = _swap_review_fee(
            candidate,
            fee_pct_max=fee_pct_max,
            fee_sats_min=fee_sats_min,
        )
        out_entries = journal_by_tx.get(out_id, [])
        in_entries = journal_by_tx.get(in_id, [])
        quarantines = [
            item
            for item in (quarantines_by_tx.get(out_id), quarantines_by_tx.get(in_id))
            if item
        ]
        out_summary = tx_summaries.get(out_id, {"transaction_id": out_id})
        in_summary = tx_summaries.get(in_id, {"transaction_id": in_id})
        metadata_mentions = [
            *out_summary.get("metadata_mentions", []),
            *in_summary.get("metadata_mentions", []),
        ]
        review_items.append(
            {
                "candidate": {
                    key: candidate.get(key)
                    for key in (
                        "out_id",
                        "in_id",
                        "out_asset",
                        "in_asset",
                        "out_amount_msat",
                        "out_amount",
                        "in_amount_msat",
                        "in_amount",
                        "out_occurred_at",
                        "in_occurred_at",
                        "default_kind",
                        "default_policy",
                    )
                },
                "out": out_summary,
                "in": in_summary,
                "confidence": _swap_review_confidence_reason(candidate),
                "fee": fee,
                "conflict": {
                    "set_id": conflict_id,
                    "candidate_count": conflict_size,
                    "requires_manual_resolution": conflict_size > 1,
                },
                "rule_match": candidate.get("rule_match"),
                "metadata_mentions": metadata_mentions,
                "current_journal": {
                    "out_entries": out_entries,
                    "in_entries": in_entries,
                    "quarantines": quarantines,
                },
                "report_impact_if_left_unpaired": _swap_review_report_impact(
                    out_entries=out_entries,
                    in_entries=in_entries,
                    quarantines=quarantines,
                    default_policy=str(candidate.get("default_policy") or ""),
                ),
                "suggested_action": _swap_review_suggested_action(
                    candidate,
                    conflict_size=conflict_size,
                    fee_assessment=str(fee.get("assessment") or ""),
                ),
            }
        )

    active_pairs_all = list_transaction_pairs(conn, workspace, profile_ref)
    rules_all = list_transfer_rules(conn, workspace, profile_ref)
    saved_views_all = list_saved_views_cli(
        conn,
        workspace,
        profile_ref,
        surface="swap_candidates",
    )
    active_pairs = active_pairs_all[:limit]
    rules = rules_all[:limit]
    saved_views = saved_views_all[:limit]
    return {
        "summary": {
            "workspace": workspace_row["label"],
            "profile": profile["label"],
            "tax_country": profile["tax_country"],
            "candidate_count": int(
                candidate_payload.get("counts", {}).get("total") or 0
            ),
            "exact_candidates": int(
                candidate_payload.get("counts", {}).get("exact") or 0
            ),
            "strong_candidates": int(
                candidate_payload.get("counts", {}).get("strong") or 0
            ),
            "conflict_clusters": int(
                candidate_payload.get("counts", {}).get("conflicts") or 0
            ),
            "rule_matches": int(
                candidate_payload.get("counts", {}).get("rule_matches") or 0
            ),
            "review_items": len(review_items),
            "active_pairs": len(active_pairs_all),
            "rules": len(rules_all),
            "saved_views": len(saved_views_all),
            "limit": limit,
        },
        "filters": {
            key: args[key]
            for key in (
                "confidence",
                "method",
                "asset_pair",
                "route_pair",
                "time_window_seconds",
                "fee_pct_max",
                "fee_sats_min",
            )
            if key in args and args[key] is not None
        },
        "counts": candidate_payload.get("counts", {}),
        "review_items": review_items,
        "active_pairs": active_pairs,
        "rules": rules,
        "saved_views": saved_views,
    }


__all__ = ("SWAP_REVIEW_DEFAULT_LIMIT", "build_swap_review_context_payload")
