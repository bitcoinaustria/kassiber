"""Portable close snapshots and independent arithmetic verification.

Checksums prove integrity against a supplied commitment, not authorship or
external anchoring. This verifier deliberately does not call the ledger report
builder or RP2, and does not claim tax recalculation or source completeness.
"""

from __future__ import annotations

import hashlib
import json
from datetime import date
from typing import Any

from ...errors import AppError


FORMAT = "kassiber.accounting-close.v1"
MAX_SNAPSHOT_BYTES = 64 * 1024 * 1024


def export_close(conn, profile_id: str, *, close_id: str) -> dict[str, Any]:
    from .ledger import require_book

    require_book(conn, profile_id)
    cursor = conn.execute(
        "SELECT id,period_id,revision,snapshot_json,snapshot_digest FROM gl_period_events "
        "WHERE id=? AND profile_id=? AND action='close'", (close_id, profile_id),
    )
    row = cursor.fetchone()
    if row is None:
        raise AppError("Close snapshot was not found in this book", code="accounting_close_not_found")
    package = dict(zip((column[0] for column in cursor.description), row))
    package.update(format=FORMAT, profile_id=profile_id, disclosure="plaintext_financial_records")
    package["verification"] = verify_package(package)
    return package


def _integer(value):
    if type(value) is not int or not 0 <= value <= 2**63 - 1:
        raise ValueError("Invalid monetary integer")
    return value


def _exact_report_amounts(value):
    if isinstance(value, dict):
        for key, item in value.items():
            if key.endswith("_minor") and type(item) is not int:
                raise ValueError("Report amounts must be exact integers")
            _exact_report_amounts(item)
    elif isinstance(value, list):
        for item in value:
            _exact_report_amounts(item)


def verify_package(package: dict[str, Any]) -> dict[str, Any]:
    """Validate one explicit in-memory package without files or network access."""
    def reject(message):
        raise AppError(message, code="accounting_package_invalid")

    if not isinstance(package, dict) or package.get("format") != FORMAT:
        reject("Unsupported accounting package format")
    source = package.get("snapshot_json")
    try:
        encoded_source = source.encode("utf-8") if isinstance(source, str) else None
    except UnicodeError:
        reject("Accounting snapshot is not valid UTF-8 text")
    if encoded_source is None or len(encoded_source) > MAX_SNAPSHOT_BYTES:
        reject("Invalid accounting snapshot size")
    digest = hashlib.sha256(encoded_source).hexdigest()
    if digest != package.get("snapshot_digest"):
        reject("Accounting snapshot checksum does not match")
    try:
        state = json.loads(source)
        _exact_report_amounts(state["trial_balance"])
        _exact_report_amounts(state["statements"])
        book, period = state["book"], state["period"]
        if book["profile_id"] != package["profile_id"] or period["profile_id"] != package["profile_id"]:
            raise ValueError("Book scope mismatch")
        if period["id"] != package["period_id"]:
            raise ValueError("Period mismatch")
        if not isinstance(book["currency"], str) or len(book["currency"]) != 3 or not book["currency"].isascii() or not book["currency"].isupper() or not book["currency"].isalpha():
            raise ValueError("Invalid book currency")
        if type(book["minor_unit_exponent"]) is not int or not 0 <= book["minor_unit_exponent"] <= 8:
            raise ValueError("Invalid currency exponent")
        context = {"profile_id": book["profile_id"], "period_id": period["id"], "currency": book["currency"], "minor_unit_exponent": book["minor_unit_exponent"], "revision": book["revision"]}
        for report in (state["trial_balance"], state["statements"]):
            if any(type(report[key]) is not type(value) or report[key] != value for key, value in context.items()):
                raise ValueError("Report context differs from its book and period")
            if report["balanced"] is not True:
                raise ValueError("Report does not assert balanced arithmetic")
        if state["trial_balance"]["as_of"] is not None:
            raise ValueError("Period trial balance cannot have an extra date filter")
        accounts = {row["code"]: row for row in state["accounts"]}
        if len(accounts) != len(state["accounts"]):
            raise ValueError("Duplicate account code")
        if any(row["profile_id"] != book["profile_id"] or row["kind"] not in {"asset", "liability", "equity", "income", "expense"} for row in accounts.values()):
            raise ValueError("Invalid account scope or kind")
        for field in ("start_date", "end_date"):
            if date.fromisoformat(period[field]).isoformat() != period[field]:
                raise ValueError("Invalid fiscal date")
        if period["start_date"] > period["end_date"]:
            raise ValueError("Reversed fiscal interval")
        movements: dict[str, list[int]] = {code: [0, 0] for code in accounts}
        cumulative = {code: [0, 0] for code in accounts}
        pnl = {code: 0 for code in accounts}
        pnl_turnover = {code: [0, 0] for code in accounts}
        entry_ids: set[str] = set()
        line_ids: set[str] = set()
        period_entries = {}
        closing_ids = {entry["id"] for entry in state["journal"] if entry["entry_kind"] == "closing"}
        by_id = {entry["id"]: entry for entry in state["journal"]}
        for entry in state["journal"]:
            if entry["entry_kind"] not in {"normal", "opening", "closing", "reversal"}:
                raise ValueError("Unknown posting kind")
            target = entry.get("reversal_of")
            if entry["entry_kind"] == "reversal":
                if target not in by_id or by_id[target]["entry_kind"] == "reversal":
                    raise ValueError("Invalid reversal target")
            elif target is not None:
                raise ValueError("Only reversal entries may name a reversal target")
            if entry["id"] in entry_ids or entry["profile_id"] != book["profile_id"] or entry["status"] != "posted":
                raise ValueError("Duplicate or out-of-scope posting")
            entry_ids.add(entry["id"])
            if date.fromisoformat(entry["entry_date"]).isoformat() != entry["entry_date"]:
                raise ValueError("Invalid entry date")
            # Recompute the posting commitment independently from its specified
            # wire fields, not via the production ledger implementation.
            committed = {key: entry[key] for key in ("idempotency_key", "period_id", "entry_date", "description", "source_ref", "entry_kind", "reversal_of")}
            committed["lines"] = [{key: line[key] for key in ("account_code", "account_name", "account_kind", "debit_minor", "credit_minor")} for line in entry["lines"]]
            encoded = json.dumps(committed, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)
            if hashlib.sha256(encoded.encode()).hexdigest() != entry["payload_digest"]:
                raise ValueError("Posting commitment does not match its fields")
            if entry["entry_date"] > period["end_date"] or len(entry["lines"]) < 2:
                raise ValueError("Entry outside snapshot scope")
            in_period = entry["period_id"] == period["id"]
            if in_period != (period["start_date"] <= entry["entry_date"] <= period["end_date"]):
                raise ValueError("Entry outside its fiscal interval")
            debit = credit = 0
            for line in entry["lines"]:
                account = accounts[line["account_code"]]
                if (line["id"] in line_ids or line["entry_id"] != entry["id"]
                    or line["profile_id"] != book["profile_id"]
                    or line["account_kind"] != account["kind"] or line["account_name"] != account["name"]):
                    raise ValueError("Invalid line identity or chart snapshot")
                line_ids.add(line["id"])
                d, c = _integer(line["debit_minor"]), _integer(line["credit_minor"])
                if (d > 0) == (c > 0):
                    raise ValueError("Line must have exactly one positive side")
                debit += d
                credit += c
                totals = cumulative[line["account_code"]]
                totals[0] += d
                totals[1] += c
                if in_period:
                    totals = movements[line["account_code"]]
                    totals[0] += d
                    totals[1] += c
                    if entry["entry_kind"] != "closing" and entry.get("reversal_of") not in closing_ids:
                        pnl[line["account_code"]] += d - c
                        pnl_turnover[line["account_code"]][0] += d
                        pnl_turnover[line["account_code"]][1] += c
            if debit != credit or debit > 2**63 - 1:
                raise ValueError("Unbalanced or overflowing entry")
            if in_period:
                period_entries[entry["id"]] = entry["payload_digest"]
        if len(state["entries"]) != len(period_entries) or period_entries != {row["id"]: row["payload_digest"] for row in state["entries"]}:
            raise ValueError("Close entry inventory does not match journal")
        actual_rows = {row["account_code"]: row for row in state["trial_balance"]["rows"]}
        if len(actual_rows) != len(state["trial_balance"]["rows"]):
            raise ValueError("Duplicate trial balance account")
        for code, (d, c) in movements.items():
            row = actual_rows.get(code)
            if row is None:
                if d or c:
                    raise ValueError("Missing trial balance account")
            elif (row["debit_minor"], row["credit_minor"], row["balance_minor"]) != (d, c, d - c):
                raise ValueError("Trial balance does not reconcile")
        if set(actual_rows) - set(accounts):
            raise ValueError("Unknown trial balance account")
        total_debit = sum(d for d, _ in movements.values())
        total_credit = sum(c for _, c in movements.values())
        if (state["trial_balance"]["debit_minor"], state["trial_balance"]["credit_minor"]) != (total_debit, total_credit):
            raise ValueError("Trial balance totals do not reconcile")
        expected_profit = -sum(value for code, value in pnl.items() if accounts[code]["kind"] in {"income", "expense"})
        if state["statements"]["profit_minor"] != expected_profit:
            raise ValueError("Period result does not reconcile")
        pnl_rows = {row["account_code"]: row for row in state["statements"]["profit_and_loss"]}
        if len(pnl_rows) != len(state["statements"]["profit_and_loss"]) or set(pnl_rows) - {code for code in accounts if accounts[code]["kind"] in {"income", "expense"}}:
            raise ValueError("Invalid profit and loss inventory")
        for code, (d, c) in pnl_turnover.items():
            if accounts[code]["kind"] not in {"income", "expense"}:
                continue
            row = pnl_rows.get(code)
            if row is None:
                if d or c:
                    raise ValueError("Missing profit and loss account")
            elif (row["debit_minor"], row["credit_minor"], row["balance_minor"]) != (d, c, d-c):
                raise ValueError("Profit and loss account does not reconcile")
        expected_result = -sum(d-c for code, (d, c) in cumulative.items() if accounts[code]["kind"] in {"income", "expense"})
        if state["statements"]["unappropriated_result_minor"] != expected_result:
            raise ValueError("Carried result does not reconcile")
        balance_rows = {row["account_code"]: row for row in state["statements"]["balance_sheet"]}
        if len(balance_rows) != len(state["statements"]["balance_sheet"]) or set(balance_rows) - {code for code in accounts if accounts[code]["kind"] not in {"income", "expense"}}:
            raise ValueError("Invalid balance sheet account inventory")
        for code, (d, c) in cumulative.items():
            if accounts[code]["kind"] in {"income", "expense"}:
                continue
            row = balance_rows.get(code)
            if row is None:
                if d or c:
                    raise ValueError("Missing balance sheet account")
            elif (row["debit_minor"], row["credit_minor"], row["balance_minor"]) != (d, c, d-c):
                raise ValueError("Balance sheet does not reconcile")
    except (KeyError, TypeError, ValueError, RecursionError, OverflowError) as exc:
        reject("Accounting package failed structural or arithmetic verification")
    return {
        "content_digest": "verified", "ledger_arithmetic": "verified",
        "source_reconciliation": "not_independently_replayed",
        "tax_calculation_replay": "not_performed", "external_authenticity": "not_proven",
        "entries_checked": len(entry_ids), "lines_checked": len(line_ids),
    }
