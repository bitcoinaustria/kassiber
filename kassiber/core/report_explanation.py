"""Version-bound, read-only explanations of stored capital-gains results."""
from __future__ import annotations

import json
from decimal import Decimal, localcontext

from ..db import database_instance_id
from ..errors import AppError
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
    custody = custody_quantity_store.custody_decision_rows(conn, str(profile["id"]), transaction_ids=sorted(ids), limit=500)
    # Keep only semantic links; raw observation hashes/slices are not navigation.
    payload["custody_decisions"] = [{key: record.get(key) for key in (
        "decision_id", "source_transaction_id", "target_transaction_id", "component_id",
        "custody_state", "basis_state", "reason", "occurred_at",
    )} for record in custody["records"]]
    payload["custody_truncated"] = bool(custody.get("next_cursor"))
    payload.update(status="available", calculation=calculation)
    return payload
