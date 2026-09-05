"""Local, once-only accounting review; never trust provider argument previews."""
from __future__ import annotations

import json
import re
from typing import Any, TextIO

ONCE_ONLY_TOOLS = frozenset({"ui.accounting.task_apply", "ui.accounting.task_cancel"})
_STEPS = {"prepare", "post", "close", "tax_finalize", "export_close", "export_tax"}


def transcript_record(record: dict[str, Any]) -> dict[str, Any]:
    """Exclude the local-only consent payload, without changing the live record."""
    result = {k: v for k, v in record.items() if k != "accounting_task_preview"}
    if isinstance(result.get("data"), dict):
        result["data"] = {k: v for k, v in result["data"].items() if k != "accounting_task_preview"}
    return result


def _money(value: Any) -> bool:
    return isinstance(value, str) and re.fullmatch(r"-?\d+", value, flags=re.ASCII) is not None


def _entry(value: Any) -> bool:
    return (
        isinstance(value, dict)
        and isinstance(value.get("description"), str)
        and isinstance(value.get("entry_date"), str)
        and isinstance(value.get("lines"), list)
        and bool(value["lines"])
        and all(
            isinstance(line, dict) and isinstance(line.get("account_code"), str)
            and _money(line.get("debit_minor")) and _money(line.get("credit_minor"))
            for line in value["lines"]
        )
    )


def _valid_preview(value: Any, task_id: str) -> bool:
    if not isinstance(value, dict) or value.get("status") != "ready":
        return False
    preview, book = value.get("preview"), value.get("book")
    if not isinstance(preview, dict) or not isinstance(book, dict):
        return False
    step = preview.get("step")
    if (not isinstance(step, str) or step not in _STEPS or value.get("step") != step or preview.get("id") != task_id
        or preview.get("ready") is not True or preview.get("blockers") != []
        or not isinstance(preview.get("period_id"), str)
        or not isinstance(preview.get("expected_digest"), str)
        or not re.fullmatch(r"[a-f0-9]{64}", preview["expected_digest"])
        or type(preview.get("expected_revision")) is not int or preview["expected_revision"] < 0
        or not isinstance(book.get("currency"), str) or not re.fullmatch(r"[A-Z]{3}", book["currency"])
        or type(book.get("minor_unit_exponent")) is not int or not 0 <= book["minor_unit_exponent"] <= 8):
        return False
    if step == "prepare":
        rows = preview.get("proposals")
        return isinstance(rows, list) and bool(rows) and all(isinstance(row, dict) and _entry(row.get("payload")) for row in rows)
    detail = preview.get("detail")
    if not isinstance(detail, dict):
        return False
    if step == "post":
        rows = detail.get("entries")
        return isinstance(rows, list) and bool(rows) and all(_entry(row) for row in rows)
    if step == "close":
        trial, statements = detail.get("trial_balance"), detail.get("statements")
        return (detail.get("period_id") == preview["period_id"] and detail.get("ready") is True
            and detail.get("blockers") == [] and isinstance(trial, dict) and isinstance(statements, dict)
            and isinstance(trial.get("rows"), list) and trial.get("balanced") is True
            and all(_money(trial.get(key)) for key in ("debit_minor", "credit_minor"))
            and isinstance(statements.get("profit_and_loss"), list) and isinstance(statements.get("balance_sheet"), list)
            and statements.get("balanced") is True)
    if step == "tax_finalize":
        forms = detail.get("forms")
        return isinstance(forms, list) and bool(forms) and all(isinstance(form, dict) and isinstance(form.get("form_id"), str) and isinstance(form.get("fields"), dict) for form in forms)
    keys = ("id", "snapshot_digest") if step == "export_close" else ("final_id", "report_digest")
    return all(isinstance(detail.get(key), str) and bool(detail[key]) for key in keys)


def decide(name: str, data: dict[str, Any], *, interactive: bool, stdin: TextIO, out: TextIO) -> str:
    """Show the exact daemon review and require a new terminal answer each time."""
    if name not in ONCE_ONLY_TOOLS:
        return "deny"
    if not interactive:
        out.write("Accounting action denied: interactive, once-only local review is required; blanket approvals do not apply.\n")
        out.flush()
        return "deny"
    arguments = data.get("arguments_preview")
    task_id = arguments.get("task_id") if isinstance(arguments, dict) else None
    if not isinstance(task_id, str) or not re.fullmatch(r"[a-f0-9]{32}", task_id):
        out.write("Accounting action denied: valid local task identity is unavailable.\n")
        out.flush()
        return "deny"
    if name == "ui.accounting.task_apply":
        # This field is provided by the daemon on tool_consent_required only.
        # A nested field in model-authored arguments is never an authority.
        preview = data.get("accounting_task_preview")
        if not _valid_preview(preview, task_id):
            out.write("Accounting action denied: a complete, current local preview is unavailable.\n")
            out.flush()
            return "deny"
        out.write("\nLOCAL ACCOUNTING REVIEW — not sent to the model or saved in --transcript\n")
        out.write("Amounts below are exact minor units; the book specifies currency and exponent.\n")
        # No truncation or model retelling. JSON escaping neutralizes terminal
        # control sequences, bidi controls and untrusted document descriptions.
        out.write(json.dumps(preview, indent=2, sort_keys=True, ensure_ascii=True) + "\n")
        out.write("This approves only the displayed step. Export preparation does not save a file.\n")
    else:
        out.write(f"\nCancel accounting task {task_id}? Already committed entries remain unchanged.\n")
    while True:
        out.write("Approve this action? [y] once, [n] deny, [c] cancel chat: ")
        out.flush()
        choice = stdin.readline().strip().lower()
        if choice in {"y", "yes"}:
            return "allow_once"
        if choice in {"", "n", "no", "deny"}:
            return "deny"
        if choice in {"c", "cancel"}:
            return "cancel"
        out.write("Session-wide approval is not available for accounting actions.\n")
