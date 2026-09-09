"""Version-bound, read-only explanations of stored capital-gains results."""
from __future__ import annotations

import json
from decimal import Decimal, localcontext
from collections import deque

from ..db import database_instance_id
from ..errors import AppError
from ..time_utils import parse_iso_datetime_or_none
from . import custody_journal, custody_quantity_store
from .book_network import require_book_accounting
from ..tax_policy import require_tax_processing_supported


def result_reference(conn, profile, entry_id):
    return {
        "database_id": database_instance_id(conn),
        "workspace_id": str(profile["workspace_id"]),
        "profile_id": str(profile["id"]),
        "input_version": int(profile["journal_input_version"] or 0),
        "processed_at": profile["last_processed_at"],
        "entry_id": str(entry_id),
    }


def explain_capital_gain(conn, profile, reference):
    """Read one snapshot; no rate lookup, journal rebuild or authored mutation."""
    owns_transaction = not conn.in_transaction
    if owns_transaction:
        conn.execute("BEGIN")
    try:
        return _explain(conn, profile, reference)
    finally:
        if owns_transaction:
            conn.rollback()


def _explain(conn, profile, reference):
    if not isinstance(reference, dict) or set(reference) != {
        "database_id", "workspace_id", "profile_id", "input_version", "processed_at", "entry_id"
    } or not isinstance(reference.get("entry_id"), str) or type(reference.get("input_version")) is not int:
        raise AppError("A complete capital-gains result reference is required", code="validation")
    profile = conn.execute("SELECT * FROM profiles WHERE id = ? AND workspace_id = ?",
                           (profile["id"], profile["workspace_id"])).fetchone()
    if profile is None or result_reference(conn, profile, reference["entry_id"]) != reference:
        raise AppError("This result belongs to another book or journal version. Reload the report.", code="report_explanation_stale")
    require_book_accounting(conn, profile["id"])
    require_tax_processing_supported(profile)
    if not custody_journal.projection_freshness(conn, profile)["is_current"]:
        raise AppError("This journal is missing or stale. Rebuild it before explaining the result.", code="report_explanation_stale")
    if custody_journal.component_integrity_blockers(conn, profile["id"]) or custody_quantity_store.blocking_quantity_issues(conn, profile["id"]):
        raise AppError("Custody review blocks this report result", code="report_explanation_blocked")
    row = conn.execute("""
        SELECT je.* FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.id = ? AND je.profile_id = ? AND je.entry_type IN ('disposal', 'income')
          AND COALESCE(t.taxability_override, 1) != 0 AND COALESCE(je.at_category, '') != 'neu_swap'
    """, (reference["entry_id"], profile["id"])).fetchone()
    if row is None:
        raise AppError("The capital-gains result is no longer available. Reload the report.", code="report_explanation_stale")
    fields = ("cost_basis_exact", "proceeds_exact", "gain_loss_exact")
    totals = {field: row[field] for field in fields}
    totals["quantity_msat"] = abs(int(row["quantity"]))
    quarantine_count = conn.execute("SELECT COUNT(*) FROM journal_quarantines WHERE profile_id = ?", (profile["id"],)).fetchone()[0]
    payload = {"reference": reference, "book_label": profile["label"], "currency": profile["fiat_currency"], "totals": totals,
               "quarantines": quarantine_count, "status": "engine_detail_unavailable",
               "calculation": None, "custody_decisions": [], "custody_truncated": False}
    if not row["calculation_json"] or any(totals[field] is None for field in fields):
        return payload
    calculation = json.loads(row["calculation_json"])
    fragments = calculation["fragments"]
    # Match the existing RP2 calculation precision; this validates stored
    # contributions, never reconstructs selection, pool evolution or tax rules.
    with localcontext() as context:
        context.prec = 32
        reconciled = all(sum((Decimal(item[field]) for item in fragments), Decimal(0)) == Decimal(totals[field]) for field in fields)
    reconciled = reconciled and sum(item["quantity_msat"] for item in fragments) == totals["quantity_msat"]
    if not reconciled:
        payload["status"] = "engine_detail_mismatch"
        return payload
    ids = {source["transaction_id"] for fragment in fragments for source in (fragment["event"], fragment["lot"])
           if source and source["transaction_id"]}
    custody, truncated = _custody_context(conn, str(profile["id"]), row, fragments, ids)
    payload["custody_decisions"] = custody
    payload["custody_truncated"] = truncated
    payload.update(status="available", calculation=calculation)
    return payload


def _custody_context(conn, profile_id, result, fragments, transaction_ids):
    """Bounded chronological wallet context, never a tax-lot ownership claim.

    Read only persisted canonical decisions. Follow incoming eligible carries
    backward from the result wallet, within the retained engine source period.
    Unrelated wallet branches and subsequent movements are outside this context.
    """
    cutoff = parse_iso_datetime_or_none(result["occurred_at"])
    starts = [parse_iso_datetime_or_none(source["occurred_at"])
              for fragment in fragments for source in (fragment["event"], fragment["lot"])
              if source]
    starts = [value for value in starts if value is not None]
    if cutoff is None or not starts:
        return [], True
    earliest = min(starts)
    pending = deque([(result["wallet_id"], result["asset"], cutoff)])
    visited = {}
    retained = {}
    budget = 500
    truncated = False
    # Source-touching edges remain useful for fees booked on a transfer itself.
    direct = True
    while pending and budget:
        wallet, asset, upper = pending.popleft()
        key = (wallet, asset)
        if key in visited and visited[key] >= upper:
            continue
        visited[key] = upper
        where = "d.target_wallet_id = ? AND d.target_asset = ?"
        params = [profile_id, wallet, asset]
        if direct and transaction_ids:
            placeholders = ",".join("?" for _ in transaction_ids)
            where = f"({where}) OR d.source_transaction_id IN ({placeholders}) OR d.target_transaction_id IN ({placeholders})"
            params.extend(sorted(transaction_ids))
            params.extend(sorted(transaction_ids))
        direct = False
        rows = conn.execute(f"""
            SELECT d.decision_id, d.source_transaction_id, d.target_transaction_id,
                   d.source_wallet_id, d.target_wallet_id, d.source_asset, d.target_asset,
                   d.state AS custody_state, d.basis_state, d.reason, d.component_id,
                   d.occurred_at, d.target_occurred_at
            FROM journal_custody_decisions d
            WHERE d.profile_id = ? AND ({where})
              AND julianday(COALESCE(d.target_occurred_at, d.occurred_at)) >= julianday(?)
              AND julianday(COALESCE(d.target_occurred_at, d.occurred_at)) <= julianday(?)
            ORDER BY julianday(COALESCE(d.target_occurred_at, d.occurred_at)) DESC, d.decision_id
            LIMIT ?
        """, (*params, earliest.isoformat(), upper.isoformat(), budget + 1)).fetchall()
        truncated |= len(rows) > budget
        for decision in rows[:budget]:
            budget -= 1
            source_at = parse_iso_datetime_or_none(decision["occurred_at"])
            target_at = parse_iso_datetime_or_none(decision["target_occurred_at"] or decision["occurred_at"])
            if source_at is None or target_at is None or not earliest <= target_at <= upper:
                truncated |= source_at is None or target_at is None
                continue
            retained[decision["decision_id"]] = dict(decision)
            if decision["basis_state"] == "eligible" and earliest <= source_at <= upper:
                pending.append((decision["source_wallet_id"], decision["source_asset"], source_at))
    return list(retained.values()), truncated or bool(pending)
