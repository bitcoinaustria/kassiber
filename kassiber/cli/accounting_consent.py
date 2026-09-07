"""Local, once-only accounting review; never trust provider argument previews."""
from __future__ import annotations

import json
import re
from typing import Any, TextIO

ONCE_ONLY_TOOLS = frozenset({"ui.accounting.task_apply", "ui.accounting.task_cancel"})
_STEPS = {"prepare", "post", "close", "tax_finalize", "export_close", "export_tax"}


def transcript_record(record: dict[str, Any], *, include_preview=False) -> dict[str, Any]:
    """Exclude the local-only consent payload, without changing the live record."""
    private = {"accounting_local_export"} if include_preview else {"accounting_task_preview", "accounting_local_export"}
    result = {k: v for k, v in record.items() if k not in private}
    if isinstance(result.get("data"), dict):
        result["data"] = {k: v for k, v in result["data"].items() if k not in private}
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


def _digest(value: Any) -> bool:
    return isinstance(value, str) and re.fullmatch(r'[a-f0-9]{64}', value) is not None


def _decimal(value: Any) -> bool:
    return isinstance(value, str) and re.fullmatch(r'-?\d+(?:\.\d+)?', value, flags=re.ASCII) is not None


def _quantity(value: Any) -> bool:
    return (isinstance(value, dict) and value.get('asset') in ('BTC', 'LBTC')
        and isinstance(value.get('account_code'), str) and bool(value['account_code'])
        and value.get('location') in ('inventory', 'transit')
        and _money(value.get('quantity_msat')) and _money(value.get('book_value_minor'))
        and _decimal(value.get('basis_exact')))


def _bitcoin_proposal(row: Any, period_id: str, *, posting=False) -> bool:
    """Validate the existing typed local projection, not its accounting arithmetic."""
    if not isinstance(row, dict) or row.get('source_kind') != 'bitcoin':
        return False
    value = row.get('projection')
    if not isinstance(value, dict) or set(value) != {'request', 'quantitative_posting', 'lines', 'policy_digest', 'valuation_release_digest'}:
        return False
    request, quantity, lines = value['request'], value['quantitative_posting'], value['lines']
    keys = {'policy_id', 'artifact_id', 'binding_id', 'event_id', 'category', 'period_id', 'idempotency_key'}
    if (not isinstance(request, dict) or set(request) != keys or not all(isinstance(v, str) and v for v in request.values())
        or request['event_id'] != row.get('source_id') or request['period_id'] != period_id
        or request['category'] not in ('purchase', 'income', 'capital', 'disposal', 'custody_move', 'transfer_dispatch', 'transfer_receipt')
        or not _digest(value['policy_digest']) or (value['valuation_release_digest'] is not None and not _digest(value['valuation_release_digest']))
        or not _quantity(quantity) or not isinstance(lines, list)
        or any(not isinstance(line, dict) or not isinstance(line.get('account_code'), str)
               or not _money(line.get('debit_minor')) or not _money(line.get('credit_minor')) for line in lines)):
        return False
    related, rounding = quantity.get('related_postings', []), quantity.get('currency_rounding')
    if (not isinstance(related, list) or not all(_quantity(item) for item in related)
        or not isinstance(rounding, list) or not rounding or any(
            not isinstance(item, dict) or not isinstance(item.get('account_code'), str)
            or not _decimal(item.get('before_basis_exact')) or not _digest(item.get('dependencies_digest'))
            or not all(_money(item.get(key)) for key in ('before_minor', 'unrounded_event_minor', 'remainder_minor')) for item in rounding)):
        return False
    if not lines and any(item['book_value_minor'] != '0' for item in [quantity, *related]):
        return False
    if request['category'] == 'custody_move':
        move = quantity.get('custody_move')
        if not isinstance(move, dict) or not all(_money(move.get(key)) for key in ('crypto_sent_msat', 'crypto_received_msat', 'crypto_fee_msat')):
            return False
    if request['category'] in ('transfer_dispatch', 'transfer_receipt') and not related:
        return False
    if posting:
        return (row.get('status') == 'draft' and isinstance(row.get('proposal_id'), str) and bool(row['proposal_id'])
            and _digest(row.get('proposal_digest')) and _digest(row.get('artifact_digest'))
            and row.get('policy_digest') == value['policy_digest'])
    return row.get('request') == request


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
        return isinstance(rows, list) and bool(rows) and all(isinstance(row, dict) and (
            _bitcoin_proposal(row, preview['period_id']) if row.get('source_kind') == 'bitcoin' else _entry(row.get('payload'))) for row in rows)
    detail = preview.get("detail")
    if not isinstance(detail, dict):
        return False
    if step == "post":
        rows = detail.get("entries")
        projections = detail.get('projections', [])
        return (isinstance(rows, list) and isinstance(projections, list) and bool(rows or projections)
            and all(_entry(row) for row in rows)
            and all(_bitcoin_proposal(row, preview['period_id'], posting=True) for row in projections))
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


def decide(name: str, data: dict[str, Any], *, interactive: bool, stdin: TextIO, out: TextIO, destination=None) -> str:
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
        if preview['step'] in ('export_close', 'export_tax'):
            out.write("This releases the exact plaintext artifact to this local client, never to the model.\n")
            out.write('CLI destination: ' + json.dumps(str(destination), ensure_ascii=True) + '\n' if destination is not None
                      else "No CLI destination selected: the artifact will not be saved.\n")
        out.write("This approves only the displayed step. Only a LOCAL EXPORT RECEIPT confirms a saved, verified file.\n")
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
