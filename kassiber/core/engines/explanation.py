"""Retain RP2's actual gain fragments without selecting lots or valuing pools."""
from __future__ import annotations

from decimal import Decimal
from ...msat import btc_to_msat


def _exact(value):
    return None if value is None else format(Decimal(value), "f")


def _source(transaction, rows):
    if transaction is None:
        return None
    row = rows.get(transaction.unique_id)
    source = dict(row) if row is not None else {}
    # Explicit allowlist: no notes, wallet configuration or raw importer payload.
    return {
        "transaction_id": source.get("journal_transaction_id") or source.get("id"),
        "engine_event_id": transaction.unique_id,
        "occurred_at": transaction.timestamp.isoformat(),
        "asset": transaction.asset,
        "spot_price_exact": _exact(transaction.spot_price),
        "fiat_fee_exact": _exact(getattr(transaction, "fiat_fee", None)),
        "crypto_fee_msat": btc_to_msat(getattr(transaction, "crypto_fee", 0)),
        "crypto_fee_msat_exact": str(btc_to_msat(getattr(transaction, "crypto_fee", 0))),
        "pricing": {key: source.get(key) for key in (
            "fiat_currency", "fiat_rate_exact", "fiat_value_exact", "pricing_source_kind",
            "pricing_quality", "fiat_price_source", "pricing_timestamp",
            "pricing_provider", "pricing_pair", "pricing_fetched_at", "pricing_granularity", "pricing_method",
        )},
    }


def gain_fragment(gain, rows, computed):
    lot = gain.acquired_lot
    override = gain.unit_cost_basis_override
    return {
        "quantity_msat": btc_to_msat(gain.crypto_amount),
        "quantity_msat_exact": str(btc_to_msat(gain.crypto_amount)),
        "cost_basis_exact": _exact(gain.fiat_cost_basis),
        "proceeds_exact": _exact(gain.taxable_event_fiat_amount_with_fee_fraction),
        "gain_loss_exact": _exact(gain.fiat_gain),
        "unit_basis_override_exact": _exact(override),
        "basis_authority": "engine_unit_override" if override is not None else "acquired_lot" if lot else "income",
        "acquisition_basis_exact": _exact(computed.get_in_transaction_fiat_in_with_fee(lot)) if lot else None,
        "acquisition_quantity_msat": btc_to_msat(lot.crypto_in) if lot else None,
        "acquisition_quantity_msat_exact": str(btc_to_msat(lot.crypto_in)) if lot else None,
        "event": _source(gain.taxable_event, rows),
        "lot": _source(lot, rows),
    }
