"""Undo hints for reversible AI tool results.

A reversible mutating tool (see ``REVERSIBLE_TOOLS`` in ``tools``) returns
``undo: {"tool": <wire name>, "arguments": {...}}`` naming the call that takes
the change back. Hints are derived only from the tool's own arguments and
result; when the result lacks what the inverse needs (a no-op edit, a batch of
several components), there is no hint. Plan-bound inverses name their plan
tool: the model plans, shows the effect and then applies, as for any change.
"""

from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal
import math
from typing import Any, Callable, Mapping

from ..msat import msat_to_btc


Hint = dict[str, Any] | None


def _text(value: Any) -> str | None:
    return value.strip() if isinstance(value, str) and value.strip() else None


def _hint(tool: str, **arguments: Any) -> dict[str, Any]:
    return {
        "tool": tool,
        "arguments": {key: value for key, value in arguments.items() if value is not None},
    }


def _btc_text(msat: Any) -> str | None:
    if type(msat) is not int or msat <= 0:
        return None
    value = msat_to_btc(msat)
    text = format(value if isinstance(value, Decimal) else Decimal(str(value)), "f")
    return text.rstrip("0").rstrip(".") if "." in text else text


def _pair(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    pair_id = _text(data.get("id"))
    return _hint("ui_transfers_unpair", pair_id=pair_id) if pair_id else None


def _unpair(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    pair = data.get("pair")
    if not isinstance(pair, Mapping):
        return None
    out_id, in_id = _text(pair.get("out_transaction_id")), _text(pair.get("in_transaction_id"))
    if not out_id or not in_id:
        return None
    return _hint(
        "ui_transfers_pair",
        tx_out=out_id,
        tx_in=in_id,
        kind=_text(pair.get("kind")),
        policy=_text(pair.get("policy")),
        notes=_text(pair.get("notes")),
        out_amount=_btc_text(pair.get("out_amount")),
    )


def _dismiss(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    dismissal_id = _text(data.get("id"))
    return _hint("ui_transfers_dismissals_delete", dismissal_id=dismissal_id) if dismissal_id else None


def _remaining_days(expires_at: Any) -> int | None:
    """Whole days left on a dismissal; 0 means it never expired."""

    if expires_at is None:
        return 0
    if not isinstance(expires_at, str):
        return None
    try:
        moment = datetime.fromisoformat(expires_at.replace("Z", "+00:00"))
    except ValueError:
        return None
    seconds = (moment - datetime.now(timezone.utc)).total_seconds()
    # An already expired dismissal no longer suppressed anything.
    return math.ceil(seconds / 86_400) if seconds > 0 else None


def _undismiss(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    dismissal = data.get("dismissal")
    if not isinstance(dismissal, Mapping):
        return None
    out_id = _text(dismissal.get("out_transaction_id"))
    in_id = _text(dismissal.get("in_transaction_id"))
    days = _remaining_days(dismissal.get("expires_at"))
    if not out_id or not in_id or days is None:
        return None
    return _hint(
        "ui_transfers_dismiss",
        tx_out=out_id,
        tx_in=in_id,
        reason=_text(dismissal.get("reason")),
        expires_in_days=days,
    )


def _rule_create(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    rule_id = _text(data.get("id"))
    return _hint("ui_transfers_rules_delete", rule_id=rule_id) if rule_id else None


def _rule_delete(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    rule = data.get("rule")
    if not isinstance(rule, Mapping) or rule.get("invalid_reason"):
        return None
    kind, policy = _text(rule.get("kind")), _text(rule.get("policy"))
    if not kind or not policy:
        return None
    predicate = rule.get("predicate")
    return _hint(
        "ui_transfers_rules_create",
        name=_text(rule.get("name")),
        predicate=dict(predicate) if isinstance(predicate, Mapping) else None,
        kind=kind,
        policy=policy,
        enabled=bool(rule.get("enabled")),
    )


def _rule_toggle(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    rule_id = _text(data.get("id"))
    enabled = data.get("enabled")
    if not rule_id or type(enabled) is not bool:
        return None
    return _hint("ui_transfers_rules_set_enabled", rule_id=rule_id, enabled=not enabled)


def _payout_create(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    payout_id = _text(data.get("id"))
    return _hint("ui_transfers_payouts_delete", payout_id=payout_id) if payout_id else None


def _components(arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    """Retire what was created; restore what a revision or supersede replaced."""

    action = arguments.get("action", "create")
    target = _text(arguments.get("component_id"))
    if action in {"revise", "supersede"} and target:
        return _hint("ui_transfers_components_plan", action="undo", component_id=target)
    if action == "activate" and target:
        return _hint("ui_transfers_components_plan", action="supersede", component_id=target)
    created: list[Any] = []
    if isinstance(data.get("components"), list):
        created = data["components"]
    elif isinstance(data.get("component"), Mapping):
        created = [data["component"]]
    ids = [
        _text(item.get("id"))
        for item in created
        if isinstance(item, Mapping) and item.get("state") == "active"
    ]
    if len(ids) == 1 and ids[0]:
        return _hint("ui_transfers_components_plan", action="supersede", component_id=ids[0])
    return None


def _custody_review(arguments: Mapping[str, Any], _data: Mapping[str, Any]) -> Hint:
    gap_id = _text(arguments.get("gap_id"))
    if not gap_id or arguments.get("action") == "reopen":
        return None
    return _hint("ui_custody_review_plan", action="reopen", gap_id=gap_id)


def _saved_view(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    view_id = _text(data.get("id"))
    return _hint("ui_saved_views_delete", view_id=view_id) if view_id else None


def _loan_mark(arguments: Mapping[str, Any], _data: Mapping[str, Any]) -> Hint:
    txid = _text(arguments.get("txid"))
    return _hint("ui_loans_unmark", txid=txid) if txid else None


def _revert(transaction: Any, event: Any) -> Hint:
    transaction, event = _text(transaction), _text(event)
    if not transaction or not event:
        return None
    return _hint("ui_transactions_history_revert", transaction=transaction, event=event)


def _metadata_update(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    if data.get("updated") is False:
        return None
    return _revert(data.get("transaction_id"), data.get("history_event_id"))


def _history_revert(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    if data.get("updated") is False:
        return None
    transaction = data.get("transaction")
    transaction_id = (
        transaction.get("transaction_id") if isinstance(transaction, Mapping) else None
    )
    return _revert(transaction_id, data.get("history_event_id"))


def _quarantine_resolve(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    resolution = data.get("resolution")
    event = resolution.get("history_event_id") if isinstance(resolution, Mapping) else None
    return _revert(data.get("transaction_id"), event)


def _attachment_import(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    batch = _text(data.get("import_batch_id"))
    return _hint("ui_imports_rollback_plan", batch=batch) if batch else None


def _case_save(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    case_id = _text(data.get("id"))
    return _hint("ui_chain_analysis_cases_delete", id=case_id) if case_id else None


def _watch_create(_arguments: Mapping[str, Any], data: Mapping[str, Any]) -> Hint:
    watch_id, revision = _text(data.get("id")), data.get("revision")
    if not watch_id or type(revision) is not int:
        return None
    return _hint("ui_chain_analysis_watches_delete", id=watch_id, expected_revision=revision)


UNDO_HINTS: dict[str, Callable[[Mapping[str, Any], Mapping[str, Any]], Hint]] = {
    "ui.transfers.pair": _pair,
    "ui.transfers.unpair": _unpair,
    "ui.transfers.dismiss": _dismiss,
    "ui.transfers.dismissals.delete": _undismiss,
    "ui.transfers.rules.create": _rule_create,
    "ui.transfers.rules.delete": _rule_delete,
    "ui.transfers.rules.set_enabled": _rule_toggle,
    "ui.transfers.payouts.create": _payout_create,
    "ui.transfers.components.apply": _components,
    "ui.custody.review.apply": _custody_review,
    "ui.saved_views.create": _saved_view,
    "ui.loans.mark": _loan_mark,
    "ui.transactions.metadata.update": _metadata_update,
    "ui.transactions.history.revert": _history_revert,
    "ui.journals.quarantine.resolve": _quarantine_resolve,
    "ui.wallets.import_attachment.apply": _attachment_import,
    "ui.chain_analysis.cases.save": _case_save,
    "ui.chain_analysis.watches.create": _watch_create,
}


def undo_hint(tool_name: str, arguments: Mapping[str, Any], data: Any) -> Hint:
    """The call that takes this tool's change back, or None."""

    builder = UNDO_HINTS.get(tool_name)
    if builder is None or not isinstance(data, Mapping):
        return None
    try:
        return builder(arguments if isinstance(arguments, Mapping) else {}, data)
    except (TypeError, ValueError, ArithmeticError):
        return None
