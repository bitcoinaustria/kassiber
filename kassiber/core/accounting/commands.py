"""Explicit accounting commands shared by CLI and scoped internal adapters.

This is not a generic Python/SQL dispatcher and is not an AI tool. Amounts
cross JSON as integer strings so JavaScript cannot silently round int64 values.
"""

from __future__ import annotations

import re
from typing import Any, Mapping

from ...errors import AppError
from .supporting_commands import READ_ACTIONS as SUPPORTING_READ_ACTIONS, WRITE_ACTIONS as SUPPORTING_WRITE_ACTIONS
from .tax_workpapers import READ_ACTIONS as TAX_READ_ACTIONS, WRITE_ACTIONS as TAX_WRITE_ACTIONS
from .projection_commands import READ_ACTIONS as PROJECTION_READ_ACTIONS, WRITE_ACTIONS as PROJECTION_WRITE_ACTIONS
from .cash_commands import READ_ACTIONS as CASH_READ_ACTIONS, WRITE_ACTIONS as CASH_WRITE_ACTIONS
from .tasks import READ_ACTIONS as TASK_READ_ACTIONS, WRITE_ACTIONS as TASK_WRITE_ACTIONS


READ_ACTIONS = frozenset({"capabilities", "snapshot", "journal", "reports", "account-ledger", "close-readiness", "workbench",
    "batch-preview",
    "document-capabilities", "document-get", "document-search"}) | SUPPORTING_READ_ACTIONS | (TAX_READ_ACTIONS - {"tax-export"}) | PROJECTION_READ_ACTIONS | CASH_READ_ACTIONS | TASK_READ_ACTIONS
WRITE_ACTIONS = frozenset({
    "configure", "account-create", "period-create", "draft", "post",
    "reverse", "close", "reopen", "discard", "export-close",
    "document-extract", "document-transcribe", "document-review",
    "tax-export",
    "batch-post",
}) | SUPPORTING_WRITE_ACTIONS | TAX_WRITE_ACTIONS | PROJECTION_WRITE_ACTIONS | CASH_WRITE_ACTIONS | TASK_WRITE_ACTIONS
ACTIONS = READ_ACTIONS | WRITE_ACTIONS


def _fields(payload: Mapping[str, Any], allowed: set[str], required: set[str] = frozenset()) -> dict[str, Any]:
    unknown = set(payload) - allowed
    missing = required - set(payload)
    if unknown or missing:
        raise AppError(
            "Invalid accounting command fields", code="accounting_invalid_fields",
            details={"unknown_count": len(unknown), "missing": sorted(missing)},
        )
    return dict(payload)


def _exact_integer_key(key: str) -> bool:
    return key.endswith(("_minor", "_atomic", "_msat"))


def wire_values(value: Any) -> Any:
    """Encode monetary and atomic-quantity integers as exact decimal strings."""
    if isinstance(value, dict):
        return {
            key: str(item) if _exact_integer_key(key) and type(item) is int else wire_values(item)
            for key, item in value.items()
        }
    if isinstance(value, (list, tuple)):
        return [wire_values(item) for item in value]
    return value


def _minor_values(value: Any) -> Any:
    if isinstance(value, dict):
        result = {}
        for key, item in value.items():
            if _exact_integer_key(key) and type(item) is int and abs(item) > 2**53 - 1:
                raise AppError("Large amounts must be canonical integer strings, not JSON numbers", code="accounting_invalid_amount")
            if _exact_integer_key(key) and isinstance(item, str):
                if not re.fullmatch(r"-?(0|[1-9][0-9]{0,18})", item) or item == "-0":
                    raise AppError("Amount must be a canonical minor-unit integer", code="accounting_invalid_amount")
                item = int(item)
            result[key] = _minor_values(item)
        return result
    if isinstance(value, list):
        return [_minor_values(item) for item in value]
    return value


def execute(conn, profile_id: str, action: str, payload: Mapping[str, Any]) -> dict[str, Any]:
    """Execute an allowlisted operation; caller owns commit/rollback."""
    from . import ledger

    if action not in ACTIONS:
        raise AppError("Unknown accounting operation", code="accounting_unknown_operation")
    if not isinstance(payload, dict):
        raise AppError("Accounting payload must be an object", code="accounting_invalid_fields")
    p = _minor_values(payload)
    if action in TASK_READ_ACTIONS | TASK_WRITE_ACTIONS:
        from . import tasks
        result = tasks.execute(conn, profile_id, action, p)
    elif action in CASH_READ_ACTIONS | CASH_WRITE_ACTIONS:
        from . import cash_commands
        result = cash_commands.execute(conn, profile_id, action, p)
    elif action in PROJECTION_READ_ACTIONS | PROJECTION_WRITE_ACTIONS:
        from . import projection_commands
        result = projection_commands.execute(conn, profile_id, action, p)
    elif action in TAX_READ_ACTIONS | TAX_WRITE_ACTIONS:
        from . import tax_workpapers
        result = tax_workpapers.execute(conn, profile_id, action, p)
    elif action in SUPPORTING_READ_ACTIONS | SUPPORTING_WRITE_ACTIONS:
        from .supporting_commands import execute_supporting

        result = execute_supporting(conn, profile_id, action, p)
    elif action in {"batch-preview", "batch-post"}:
        from . import posting_batch
        if action == "batch-preview":
            result = posting_batch.preview(conn, profile_id, **_fields(p, {"draft_ids"}, {"draft_ids"}))
        else:
            keys = {"draft_ids", "expected_revision", "expected_digest", "idempotency_key", "reason"}
            result = posting_batch.post(conn, profile_id, **_fields(p, keys, keys))
    elif action == "capabilities":
        from . import capabilities
        _fields(p, set())
        result = capabilities.snapshot(conn, profile_id)
    elif action == "snapshot":
        _fields(p, set())
        result = ledger.snapshot(conn, profile_id)
    elif action == "account-ledger":
        from .account_ledger import account_ledger
        args = _fields(p, {"account_code", "period_id", "limit", "cursor"}, {"account_code", "period_id"})
        result = account_ledger(conn, profile_id, **args)
    elif action.startswith("document-"):
        from . import document_text
        if action == "document-capabilities":
            _fields(p, set())
            ledger.require_book(conn, profile_id)
            result = document_text.capabilities()
        elif action == "document-get":
            result = document_text.get(conn, profile_id, **_fields(p, {"extraction_id"}, {"extraction_id"}))
        elif action == "document-search":
            result = document_text.search(conn, profile_id, **_fields(p, {"query", "limit"}, {"query"}))
        elif action == "document-extract":
            result = document_text.extract(conn, profile_id, **_fields(p, {"evidence_id", "method", "ocr_pages", "ocr_language"}, {"evidence_id"}))
        elif action == "document-transcribe":
            result = document_text.transcribe(conn, profile_id, **_fields(p, {"evidence_id", "pages", "reason"}, {"evidence_id", "pages", "reason"}))
        else:
            keys = {"extraction_id", "expected_digest", "previous_id", "fields", "spans", "reason"}
            result = document_text.review_fields(conn, profile_id, **_fields(p, keys, keys))
    elif action == "configure":
        args = _fields(p, {"currency", "minor_unit_exponent", "timezone", "entity_kind", "accounting_regime"}, {"currency", "timezone"})
        result = ledger.configure_book(conn, profile_id, **args)
    elif action == "account-create":
        args = _fields(p, {"code", "name", "kind"}, {"code", "name", "kind"})
        result = ledger.create_account(conn, profile_id, **args)
    elif action == "period-create":
        args = _fields(p, {"period_id", "start_date", "end_date"}, {"period_id", "start_date", "end_date"})
        result = ledger.create_period(conn, profile_id, **args)
    elif action == "draft":
        _fields(p, {"idempotency_key", "period_id", "entry_date", "description", "lines", "source_ref", "entry_kind"}, {"idempotency_key", "period_id", "entry_date", "description", "lines"})
        result = ledger.create_draft(conn, profile_id, p)
    elif action == "post":
        args = _fields(p, {"draft_id", "expected_digest"}, {"draft_id", "expected_digest"})
        result = ledger.post_draft(conn, profile_id, **args)
    elif action == "discard":
        args = _fields(p, {"draft_id", "expected_digest"}, {"draft_id", "expected_digest"})
        result = ledger.discard_draft(conn, profile_id, **args)
    elif action == "export-close":
        from .package import export_close

        args = _fields(p, {"close_id", "confirm_plaintext"}, {"close_id", "confirm_plaintext"})
        if args.pop("confirm_plaintext") is not True:
            raise AppError("Explicit plaintext financial export approval is required", code="accounting_export_consent_required")
        result = export_close(conn, profile_id, **args)
    elif action == "reverse":
        args = _fields(p, {"entry_id", "entry_date", "period_id", "idempotency_key", "reason"}, {"entry_id", "entry_date", "period_id", "idempotency_key", "reason"})
        result = ledger.reverse_entry(conn, profile_id, **args)
    elif action == "journal":
        args = _fields(p, {"period_id", "status", "limit", "cursor"})
        result = ledger.journal_page(conn, profile_id, **args)
    elif action == "reports":
        args = _fields(p, {"period_id"}, {"period_id"})
        result = {
            "trial_balance": ledger.trial_balance(conn, profile_id, **args),
            "statements": ledger.financial_statements(conn, profile_id, **args),
        }
    elif action == "workbench":
        from .workbench import snapshot
        result = snapshot(conn, profile_id, **_fields(p, {"period_id"}, {"period_id"}))
    elif action == "close-readiness":
        result = ledger.close_readiness(conn, profile_id, **_fields(p, {"period_id"}, {"period_id"}))
    elif action == "close":
        args = _fields(p, {"period_id", "expected_revision"}, {"period_id", "expected_revision"})
        result = ledger.close_period(conn, profile_id, **args)
    else:
        args = _fields(p, {"period_id", "reason", "expected_revision"}, {"period_id", "reason", "expected_revision"})
        result = ledger.reopen_period(conn, profile_id, **args)
    return wire_values(result)
