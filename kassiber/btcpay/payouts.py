"""Normalise BTCPay payouts: refunds, pull-payment claims, and store payouts.

Payouts are the merchant's outgoing BTCPay flows. Invoice refunds create a
pull payment named ``Refund {invoiceId}``; plugins such as Prism (Lightning
splits) and payroll/vendor-pay plugins create store payouts directly. Each
completed payout carries a payment proof with the on-chain txid or the
Lightning payment hash, which Kassiber uses to suggest a link to the outgoing
wallet transaction.

Privacy: Lightning proofs may include the preimage and a payout destination
may be an encoded BOLT11 invoice or LNURL. Neither is kept; only on-chain
addresses and Lightning Addresses survive as destinations, matching the
Lightning discard policy.
"""

from __future__ import annotations

import datetime as _dt
import re
from typing import Any, Mapping

from .payment_methods import asset_for_payment_method, canonical_payment_method_id

PAYOUT_STATES = ("AwaitingApproval", "AwaitingPayment", "InProgress", "Completed", "Cancelled")
# BTCPay names invoice refunds "Refund {invoiceId}"; invoice ids are ~22
# base58 characters, which keeps custom names like "Refund customers" out.
_REFUND_NAME = re.compile(r"^\s*refund\s+(?P<invoice_id>[1-9A-HJ-NP-Za-km-z]{16,})\s*$", re.IGNORECASE)
_HEX64 = re.compile(r"^[0-9a-fA-F]{64}$")
_BOLT11 = re.compile(r"^(lightning:)?ln(bc|tb|bcrt|tbs|sb)[0-9a-z]+$", re.IGNORECASE)
_LNURL = re.compile(r"^(lightning:)?lnurl[0-9a-z]+$", re.IGNORECASE)
_SENSITIVE_PROOF_KEYS = {"preimage", "paymentpreimage", "secret"}


def _text(value: Any) -> str | None:
    if value in (None, ""):
        return None
    text = str(value).strip()
    return text or None


def _timestamp(value: Any) -> str | None:
    """BTCPay payout dates are Unix seconds; keep ISO strings as-is."""

    if value in (None, ""):
        return None
    if isinstance(value, str) and not value.strip().lstrip("-").isdigit():
        return value
    try:
        seconds = int(float(value))
    except (TypeError, ValueError):
        return None
    return _dt.datetime.fromtimestamp(seconds, tz=_dt.timezone.utc).isoformat().replace("+00:00", "Z")


def refund_invoice_id(pull_payment: Mapping[str, Any] | None) -> str | None:
    if not isinstance(pull_payment, Mapping):
        return None
    for key in ("name", "description"):
        match = _REFUND_NAME.match(str(pull_payment.get(key) or ""))
        if match:
            return match.group("invoice_id")
    return None


def safe_destination(value: Any) -> str | None:
    text = _text(value)
    if text is None:
        return None
    if _BOLT11.match(text) or _LNURL.match(text):
        return None
    return text


def sanitize_proof(proof: Any) -> dict[str, Any]:
    if not isinstance(proof, Mapping):
        return {}
    return {
        str(key): value
        for key, value in proof.items()
        if str(key).lower() not in _SENSITIVE_PROOF_KEYS
    }


def proof_references(proof: Any, payout_method_id: str | None) -> tuple[str | None, str | None]:
    """Return ``(txid, payment_hash)`` from a payout payment proof.

    BTCPay 2.x serialises proofs with PascalCase keys (``ProofType``,
    ``TransactionId``) while the Greenfield docs show camelCase, so keys are
    matched case-insensitively.
    """

    if not isinstance(proof, Mapping):
        return None, None
    lowered = {str(key).lower(): value for key, value in proof.items()}
    proof_type = str(lowered.get("prooftype") or "").lower()
    rail_is_lightning = "lightning" in proof_type or str(payout_method_id or "").upper().endswith(("-LN", "-LNURL"))
    candidates = [
        lowered.get("transactionid"),
        lowered.get("txid"),
        lowered.get("paymenthash"),
        lowered.get("id"),
    ]
    txid = None
    payment_hash = None
    for value in candidates:
        text = _text(value)
        if not text or not _HEX64.match(text):
            continue
        if rail_is_lightning:
            payment_hash = payment_hash or text.lower()
        else:
            txid = txid or text.lower()
    return txid, payment_hash


def normalize_payout(
    store_id: str,
    payout: Mapping[str, Any],
    *,
    pull_payments: Mapping[str, Mapping[str, Any]] | None = None,
) -> dict[str, Any] | None:
    payout_id = _text(payout.get("id"))
    if payout_id is None:
        return None
    payout_method_id = canonical_payment_method_id(
        payout.get("payoutMethodId") or payout.get("paymentMethod") or payout.get("paymentMethodId")
    ) or None
    pull_payment_id = _text(payout.get("pullPaymentId"))
    pull_payment = (pull_payments or {}).get(pull_payment_id or "") if pull_payment_id else None
    invoice_id = refund_invoice_id(pull_payment)
    proof = sanitize_proof(payout.get("paymentProof"))
    txid, payment_hash = proof_references(proof, payout_method_id)
    asset = asset_for_payment_method(payout_method_id) if payout_method_id else None
    payout_currency = _text(payout.get("payoutCurrency") or payout.get("cryptoCode"))
    amount = _text(payout.get("payoutAmount") or payout.get("paymentMethodAmount"))
    if payout_currency and asset and payout_currency.upper() != asset:
        # Amount is denominated in something other than the payout asset.
        amount = None
    original_currency = _text(payout.get("originalCurrency") or payout.get("currency"))
    original_amount = _text(payout.get("originalAmount") or payout.get("amount"))
    if invoice_id:
        origin_kind = "refund"
        origin_label = _text((pull_payment or {}).get("name")) or f"Refund {invoice_id}"
    elif pull_payment_id:
        origin_kind = "pull_payment"
        origin_label = _text((pull_payment or {}).get("name")) or pull_payment_id
    else:
        origin_kind = "store_payout"
        metadata = payout.get("metadata") if isinstance(payout.get("metadata"), Mapping) else {}
        origin_label = _text(
            metadata.get("source")
            or metadata.get("label")
            or metadata.get("description")
            or metadata.get("name")
        )
    raw = dict(payout)
    raw["paymentProof"] = proof or None
    raw["destination"] = safe_destination(payout.get("destination"))
    fiat_currency = original_currency if original_currency and original_currency.upper() not in {"BTC", "SATS", "LBTC"} else None
    return {
        "store_id": store_id,
        "payout_id": payout_id,
        "pull_payment_id": pull_payment_id,
        "invoice_id": invoice_id,
        "payout_method_id": payout_method_id,
        "state": _text(payout.get("state")),
        "occurred_at": _timestamp(payout.get("date")),
        "asset": asset,
        "amount": amount,
        "txid": txid,
        "payment_hash": payment_hash,
        "destination": safe_destination(payout.get("destination")),
        "origin_kind": origin_kind,
        "origin_label": origin_label,
        "origin_url": _text((pull_payment or {}).get("viewLink")),
        "fiat_currency": fiat_currency,
        "fiat_value": original_amount if fiat_currency else None,
        "payout": raw,
    }


__all__ = [
    "PAYOUT_STATES",
    "normalize_payout",
    "proof_references",
    "refund_invoice_id",
    "safe_destination",
    "sanitize_proof",
]
