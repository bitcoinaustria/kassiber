"""Bark (Ark) wallet history served by the BTCPay Bark plugin.

Since plugin 0.10 the plugin serves the store's whole Bark wallet at the
standard Greenfield path
``/api/v1/stores/{storeId}/payment-methods/BTC-ARK/wallet/transactions``,
behind the read-only ``btcpay.store.canviewwallet``. Each row is one wallet
movement (one received VTXO for an Ark receive) or one transaction of the
wallet's own on-chain part; the rows of a complete ledger add up to the
wallet's balance.

``amount`` is the signed change of that balance with the fee included, and
``fee`` is the part of it the wallet paid (null when unknown). Kassiber books
each final row so that its booked effect equals ``amount`` exactly, in msat:

- a receive books its gross principal (``amount + fee``) inbound, and the fee
  as a fee-only outbound record;
- a send books its principal (``-amount - fee``) outbound with ``fee``;
- a row whose whole amount is its fee (refresh, board, exit fees, an
  offboard into the wallet's own on-chain part) books one fee-only record.

Unknown kinds and amounts the fee cannot explain refuse rather than guess.
Plugins before 0.10 used the payment method id ``BARK`` and served no wallet
history; Kassiber's own earlier canonical form of that id was ``BARK-CHAIN``.
"""

from __future__ import annotations

import json
from decimal import Decimal, InvalidOperation
from typing import Any, Mapping

from ..errors import AppError
from ..msat import msat_to_btc
from .payment_methods import BARK_PAYMENT_METHOD_ID
BARK_RAW_SOURCE = "btcpay_bark_wallet"
BARK_PAYMENT_HASH_SOURCE = "btcpay_bark"
FALLBACK_IDENTITY_PREFIX = "btcpay-bark"

INBOUND_KINDS = frozenset({"ark-receive", "lightning-receive", "onchain-receive"})
SEND_KINDS = frozenset({"ark-send", "lightning-send", "offboard", "send-onchain", "onchain-send", "exit"})
FEE_KINDS = frozenset({"refresh", "board", "exit-fee", "exit-claim", "onchain-internal"})
KNOWN_KINDS = INBOUND_KINDS | SEND_KINDS | FEE_KINDS
LIGHTNING_KINDS = frozenset({"lightning-receive", "lightning-send"})

PART_PRINCIPAL = "principal"
PART_FEE = "fee"

# No row can move more than every bitcoin there will be.
MAX_MSAT = 21_000_000 * 100_000_000 * 1000
_HEX = frozenset("0123456789abcdef")


def _is_hex64(value: str) -> bool:
    return len(value) == 64 and all(char in _HEX for char in value)


def _unsupported(row_id: str, kind: str, reason: str, message: str) -> AppError:
    return AppError(
        message,
        code="bark_row_unsupported",
        hint=(
            "Kassiber books a Bark wallet only when every row's amount and fee agree. Update Kassiber, "
            "or keep this store's Bark payments as a payment ledger until it understands this row."
        ),
        details={"row_id": row_id, "kind": kind, "reason": reason},
        retryable=False,
    )


def _malformed(message: str, row_id: str | None = None) -> AppError:
    details = {"row_id": row_id} if row_id else None
    return AppError(message, code="protocol_error", details=details, retryable=False)


def btc_msat(value: Any, *, field: str, row_id: str) -> int:
    """Exact msat of a BTC amount; refuses anything that is not a whole msat."""

    if isinstance(value, bool) or not isinstance(value, (int, float, str)):
        raise _malformed(f"Bark wallet row '{row_id}' has an invalid {field}", row_id)
    try:
        amount = Decimal(str(value).strip())
    except InvalidOperation as exc:
        raise _malformed(f"Bark wallet row '{row_id}' has an invalid {field}", row_id) from exc
    if not amount.is_finite():
        raise _malformed(f"Bark wallet row '{row_id}' has an invalid {field}", row_id)
    if amount.is_zero():
        return 0
    # Exact and independent of the decimal context: digits times a power of ten.
    if amount.adjusted() > 8:
        raise _malformed(f"Bark wallet row '{row_id}' has an invalid {field}", row_id)
    if amount.adjusted() < -11:
        raise _malformed(f"The {field} of Bark wallet row '{row_id}' is not a whole msat", row_id)
    sign, digits, exponent = amount.as_tuple()
    coefficient = int("".join(map(str, digits)))
    shift = exponent + 11
    if shift >= 0:
        msat = coefficient * 10**shift
    else:
        msat, rest = divmod(coefficient, 10**-shift)
        if rest:
            raise _malformed(f"The {field} of Bark wallet row '{row_id}' is not a whole msat", row_id)
    if msat > MAX_MSAT:
        raise _malformed(f"Bark wallet row '{row_id}' has an invalid {field}", row_id)
    return -msat if sign else msat


def _timestamp(value: Any, row_id: str) -> str:
    if value is None:
        raise _malformed(f"Bark wallet row '{row_id}' is missing 'timestamp'", row_id)
    # Late import: sync_btcpay imports this module.
    from ..sync_btcpay import _unix_to_iso

    return _unix_to_iso(value)


def _text(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def invoice_label_ref(labels: Any) -> str | None:
    """The BTCPay invoice id of a row's ``invoice`` label, if any."""

    if not isinstance(labels, Mapping):
        return None
    label = labels.get("invoice")
    if not isinstance(label, Mapping):
        return None
    return _text(label.get("ref") or label.get("id"))


def row_id(row: Any) -> str:
    if not isinstance(row, Mapping):
        raise _malformed("Bark wallet transaction record was not a JSON object")
    value = _text(row.get("id"))
    if value is None:
        raise _malformed("Bark wallet transaction record is missing 'id'")
    if any(0xD800 <= ord(char) <= 0xDFFF for char in value):
        # An id becomes a stored identity; lone surrogates cannot be stored.
        raise _malformed("Bark wallet transaction record has an invalid 'id'")
    return value


def row_keys(row: Mapping[str, Any]) -> dict[str, Any]:
    """What ties a row to a BTCPay invoice payment: outpoint, payment hash, movement."""

    outpoint = _text(row.get("outpoint"))
    payment_hash = _text(row.get("paymentHash"))
    movement = row.get("movementId")
    return {
        "outpoint": outpoint.lower() if outpoint else None,
        "payment_hash": payment_hash.lower() if payment_hash and _is_hex64(payment_hash.lower()) else None,
        "movement_id": movement if isinstance(movement, int) and not isinstance(movement, bool) else None,
        "invoice_id": invoice_label_ref(row.get("labels")),
    }


def payment_keys(payment_id: Any) -> dict[str, Any]:
    """What a BTCPay Bark invoice payment id names.

    ``txid-vout`` is an Ark payment (the received VTXO ``txid:vout``); a bare
    64-hex id is a Lightning payment hash; ``{invoiceId}#ark-movement-N``
    (plugin 0.7 and invoices from before 0.10) names Bark's movement ``N``.
    """

    raw = str(payment_id or "").strip()
    lowered = raw.lower()
    if _is_hex64(lowered):
        return {"payment_hash": lowered}
    txid, separator, vout = lowered.partition("-")
    if separator and _is_hex64(txid) and vout.isdigit() and vout.isascii():
        return {"outpoint": f"{txid}:{int(vout)}"}
    invoice_id, separator, movement = raw.partition("#ark-movement-")
    if separator and invoice_id and movement.isdigit() and movement.isascii():
        return {"invoice_id": invoice_id, "movement_id": int(movement)}
    return {}


def fallback_identity(store_id: str, bark_row_id: str, part: str = PART_PRINCIPAL) -> str:
    base = f"{FALLBACK_IDENTITY_PREFIX}:{store_id}:{bark_row_id}"
    return base if part == PART_PRINCIPAL else f"{base}:{part}"


def is_fallback_identity(store_id: str, external_id: Any) -> bool:
    return str(external_id or "").startswith(f"{FALLBACK_IDENTITY_PREFIX}:{store_id}:")


def bark_records(row: Mapping[str, Any], *, store_id: str) -> list[dict[str, Any]]:
    """Import records for one final Bark wallet row, before identity assignment.

    Each record carries ``_bark_row_id`` and ``_bark_part``; the caller sets
    ``id`` (``bark_identity``) once it knows which invoice payment a row is.
    """

    bark_row_id = row_id(row)
    kind = _text(row.get("kind")) or "unknown"
    if kind not in KNOWN_KINDS:
        raise _unsupported(bark_row_id, kind, "unknown_kind", f"Bark wallet row '{bark_row_id}' has an unknown kind '{kind}'")
    if "amount" not in row or row.get("amount") is None:
        raise _malformed(f"Bark wallet row '{bark_row_id}' is missing 'amount'", bark_row_id)
    amount = btc_msat(row.get("amount"), field="amount", row_id=bark_row_id)
    fee = 0 if row.get("fee") is None else btc_msat(row.get("fee"), field="fee", row_id=bark_row_id)
    if fee < 0:
        raise _unsupported(bark_row_id, kind, "negative_fee", f"Bark wallet row '{bark_row_id}' has a negative fee")
    occurred_at = _timestamp(row.get("timestamp"), bark_row_id)
    keys = row_keys(row)
    payment_hash = keys["payment_hash"] if kind in LIGHTNING_KINDS else None
    comment = _text(row.get("comment"))
    labels = row.get("labels")

    def record(part: str, direction: str, principal_msat: int, fee_msat: int, record_kind: str) -> dict[str, Any]:
        raw = {
            "source": BARK_RAW_SOURCE,
            "store_id": store_id,
            "bark_row_id": bark_row_id,
            "part": part,
            "kind": kind,
            "amount_msat": amount,
            "fee_msat": fee,
            "transaction_hash": _text(row.get("transactionHash")),
            "outpoint": keys["outpoint"],
            "payment_hash": keys["payment_hash"],
            "movement_id": keys["movement_id"],
            "invoice_id": keys["invoice_id"],
        }
        principal = part == PART_PRINCIPAL
        return {
            "_bark_row_id": bark_row_id,
            "_bark_part": part,
            "_bark_keys": keys if principal else {},
            "occurred_at": occurred_at,
            "confirmed_at": occurred_at,
            "direction": direction,
            "asset": "BTC",
            "amount": msat_to_btc(principal_msat),
            "fee": msat_to_btc(fee_msat),
            "amount_includes_fee": False,
            "kind": record_kind,
            "description": comment or f"Bark {kind}" + ("" if principal else " fee"),
            "counterparty": None,
            "payment_hash": payment_hash if principal else None,
            "payment_hash_source": BARK_PAYMENT_HASH_SOURCE if principal and payment_hash else None,
            "_btcpay_comment": comment if principal else None,
            "_btcpay_labels": _labels(labels) if principal else [],
            "raw_json": json.dumps(raw, sort_keys=True),
        }

    if amount > 0:
        if kind not in INBOUND_KINDS:
            raise _unsupported(bark_row_id, kind, "unexpected_inflow", f"Bark wallet row '{bark_row_id}' ({kind}) adds funds")
        records = [record(PART_PRINCIPAL, "inbound", amount + fee, 0, "deposit")]
        if fee:
            records.append(record(PART_FEE, "outbound", 0, fee, "fee"))
        return records
    if amount < 0:
        if kind in INBOUND_KINDS:
            raise _unsupported(bark_row_id, kind, "unexpected_outflow", f"Bark wallet row '{bark_row_id}' ({kind}) removes funds")
        if -amount == fee:
            return [record(PART_PRINCIPAL, "outbound", 0, fee, "fee")]
        if -amount < fee:
            raise _unsupported(bark_row_id, kind, "fee_exceeds_amount", f"Bark wallet row '{bark_row_id}' has a fee larger than its amount")
        if kind not in SEND_KINDS:
            raise _unsupported(bark_row_id, kind, "unexpected_outflow", f"Bark wallet row '{bark_row_id}' ({kind}) sends funds")
        return [record(PART_PRINCIPAL, "outbound", -amount - fee, fee, "withdrawal")]
    if fee:
        raise _unsupported(bark_row_id, kind, "fee_without_amount", f"Bark wallet row '{bark_row_id}' has a fee but no amount")
    return []


def _labels(value: Any) -> list[str]:
    # Late import: importers pulls in most of the package.
    from ..importers import parse_btcpay_labels

    return parse_btcpay_labels(value)


__all__ = [
    "BARK_PAYMENT_HASH_SOURCE",
    "BARK_PAYMENT_METHOD_ID",
    "BARK_RAW_SOURCE",
    "KNOWN_KINDS",
    "PART_FEE",
    "PART_PRINCIPAL",
    "bark_records",
    "btc_msat",
    "fallback_identity",
    "invoice_label_ref",
    "is_fallback_identity",
    "payment_keys",
    "row_id",
    "row_keys",
]
