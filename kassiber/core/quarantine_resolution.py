"""Audited quarantine metadata effects shared by CLI and portable reviews.

Callers establish the current quarantine case and own projection rebuilding.
This service never deletes a derived quarantine or implies that a case cleared.
"""
from decimal import Decimal

from ..errors import AppError
from ..msat import dec, msat_to_btc
from . import metadata, pricing


def update_quarantine_metadata(
    conn, profile, *, transaction_id, action, hooks, fiat_rate=None,
    fiat_value=None, authored_source="cli", reason=None, commit=False,
):
    tx = conn.execute(
        "SELECT * FROM transactions WHERE id = ? AND profile_id = ?",
        (transaction_id, profile["id"]),
    ).fetchone()
    if tx is None:
        raise AppError("Quarantine transaction was not found", code="not_found")
    kwargs = {}
    if action == "exclude":
        kwargs["excluded"] = True
    elif action == "price_override":
        if fiat_rate is None and fiat_value is None:
            raise AppError("Provide a fiat rate or value", code="validation")
        rate = dec(fiat_rate) if fiat_rate is not None else None
        value = dec(fiat_value) if fiat_value is not None else None
        amount = abs(msat_to_btc(tx["amount"]))
        if rate is None and value is not None and amount > 0:
            rate = value / amount
        if value is None and rate is not None and amount > 0:
            value = rate * amount
        if rate is not None and (not Decimal(rate).is_finite() or rate <= 0):
            raise AppError("Fiat rate must be positive and finite", code="validation")
        if value is not None and (not Decimal(value).is_finite() or value < 0):
            raise AppError("Fiat value must be nonnegative and finite", code="validation")
        kwargs["pricing_update"] = {
            "fiat_rate": str(rate) if rate is not None else None,
            "fiat_value": str(value) if value is not None else None,
            "source_kind": pricing.SOURCE_MANUAL_OVERRIDE,
            "quality": pricing.QUALITY_EXACT,
            "method": "quarantine_price_override",
        }
    else:
        raise AppError("Unsupported quarantine metadata action", code="validation")
    return metadata.update_transaction_metadata(
        conn, profile["workspace_id"], profile["id"], transaction_id, hooks,
        source="gui" if authored_source == "user" else authored_source,
        reason=reason, commit=commit, **kwargs,
    )
