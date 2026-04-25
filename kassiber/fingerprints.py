"""Stable transaction fingerprint helpers."""

import hashlib
import json


def make_transaction_fingerprint(wallet_id, external_id, occurred_at, direction, asset, amount, fee) -> str:
    payload = json.dumps(
        {
            "wallet_id": wallet_id,
            "external_id": external_id,
            "occurred_at": occurred_at,
            "direction": direction,
            "asset": asset,
            "amount": str(amount),
            "fee": str(fee),
        },
        sort_keys=True,
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()
