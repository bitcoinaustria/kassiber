"""Scoped internal accounting transport. No implicit AI disclosure or egress."""

from __future__ import annotations

from typing import Any, Mapping

from .core.accounting.commands import ACTIONS, READ_ACTIONS, execute
from .core.repo import current_context_ids, resolve_scope
from .errors import AppError


ACCOUNTING_UI_KINDS = tuple("ui.accounting." + action.replace("-", "_") for action in sorted(ACTIONS))
PAGED_KINDS = frozenset("ui.accounting." + action for action in (
    "journal", "account_ledger", "evidence_list", "bank_list", "item_list", "schedule_list", "tax_list",
    "projection_events", "projection_list", "projection_policy_list",
))


def dispatch_accounting_ui(conn, *, kind: str, args: Mapping[str, Any]) -> dict[str, Any]:
    if kind not in ACCOUNTING_UI_KINDS:
        raise AppError("Unknown accounting kind", code="unsupported_kind")
    allowed = {"profile_id", "payload", "cursor"} if kind in PAGED_KINDS else {"profile_id", "payload"}
    if set(args) - allowed:
        raise AppError("Invalid accounting request fields", code="accounting_invalid_fields")
    action = kind.removeprefix("ui.accounting.").replace("_", "-")
    expected_profile = args.get("profile_id")
    workspace_ref, profile_ref = current_context_ids(conn)
    if not str(workspace_ref or "").strip() or not str(profile_ref or "").strip():
        if expected_profile is not None or action not in READ_ACTIONS:
            raise AppError("The selected book changed; review the operation again", code="accounting_scope_changed")
        if action == "capabilities":
            from .core.accounting.capabilities import snapshot
            if not isinstance(args.get("payload", {}), dict) or args.get("payload", {}):
                raise AppError("Invalid accounting request fields", code="accounting_invalid_fields")
            return {"workspace_id": None, "profile_id": None, **snapshot(conn, None)}
        raise AppError("Create or select a book first", code="accounting_context_required")
    workspace, profile = resolve_scope(conn)
    if (action not in READ_ACTIONS and expected_profile is None) or (
        expected_profile is not None and expected_profile != profile["id"]
    ):
        raise AppError("The selected book changed; review the operation again", code="accounting_scope_changed")
    payload = args.get("payload", {})
    if "cursor" in args:
        if not isinstance(payload, dict) or "cursor" in payload:
            raise AppError("Cursor must be supplied at exactly one request level", code="accounting_invalid_fields")
        payload = dict(payload, cursor=args["cursor"])
    try:
        result = execute(conn, profile["id"], action, payload)
        if action not in READ_ACTIONS:
            conn.commit()
    except Exception:
        conn.rollback()
        raise
    return {"workspace_id": workspace["id"], "profile_id": profile["id"], **result}
