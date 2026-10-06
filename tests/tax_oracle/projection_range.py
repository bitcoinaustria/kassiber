"""Shapes a finalized tax projection cannot have (design §3.5, LD-5).

``compile_finalized_tax_projection`` never emits these inputs, so the ledger
request builder may refuse them before the walk, where the oracle quarantined
them through dead branches or failed later. The comparison harness accepts a
production ``EngineContractError`` only when ``violations`` names the same
invariant, and counts every such event.

This module is written from the design, not from product code, so a builder
check cannot vouch for itself: the synthetic id prefixes and the privacy
boundary vocabulary are copied here on purpose.
"""

from __future__ import annotations

from decimal import Decimal, InvalidOperation
from typing import Any, Mapping

# One name per invariant, in the order ``violations`` reports them.
NON_GLOBAL_POOL_SCOPE = "non_global_pool_scope"
WALLET_REF_ID_MISMATCH = "wallet_ref_id_mismatch"
NON_SYNTHETIC_ROW_ID = "non_synthetic_row_id"
MISSING_ANCHOR = "missing_anchor"
UNKNOWN_WALLET = "unknown_wallet"
UNKNOWN_DIRECTION = "unknown_direction"
NON_INTEGER_MSAT = "non_integer_msat"
NEGATIVE_FEE = "negative_fee"
NON_POSITIVE_AMOUNT = "non_positive_amount"
AMOUNT_INCLUDES_FEE = "amount_includes_fee"
CUSTODY_COMPONENT_FIELDS = "custody_component_fields"
CORRUPT_DECIMAL_TEXT = "corrupt_decimal_text"
GROUP_BLOCK_ROWS = "group_block_rows"
UNREVIEWED_PRIVACY_PAIR = "unreviewed_privacy_pair"
SHARED_PAIR_LEG = "shared_pair_leg"
UNEQUAL_MOVE_AMOUNTS = "unequal_move_amounts"

INVARIANTS = (
    NON_GLOBAL_POOL_SCOPE,
    WALLET_REF_ID_MISMATCH,
    NON_SYNTHETIC_ROW_ID,
    MISSING_ANCHOR,
    UNKNOWN_WALLET,
    UNKNOWN_DIRECTION,
    NON_INTEGER_MSAT,
    NEGATIVE_FEE,
    NON_POSITIVE_AMOUNT,
    AMOUNT_INCLUDES_FEE,
    CUSTODY_COMPONENT_FIELDS,
    CORRUPT_DECIMAL_TEXT,
    GROUP_BLOCK_ROWS,
    UNREVIEWED_PRIVACY_PAIR,
    SHARED_PAIR_LEG,
    UNEQUAL_MOVE_AMOUNTS,
)

# The prefixes of the ids the projection gives its rows
# (``transfers._SYNTHETIC_TRANSFER_ID_PREFIXES``).
SYNTHETIC_ROW_ID_PREFIXES = (
    "custody-tax:",
    "cross-split:",
    "direct-payout:",
    "multi-consol:",
    "owned-derive:",
    "recorded-fanout:",
)
# The privacy boundaries a row can carry (``privacy_hops.PRIVACY_HOP_TYPES``).
PRIVACY_BOUNDARIES = frozenset({"coinjoin", "collaborative", "payjoin", "payment_in_coinjoin", "sweep"})
REVIEWED_PRIVACY_PAIR_KIND = "custody-reviewed-coinjoin"
DIRECTIONS = ("inbound", "outbound")


def _get(row: Any, key: str, default: Any = None) -> Any:
    if isinstance(row, Mapping):
        return row.get(key, default)
    if hasattr(row, "keys") and key in row.keys():
        return row[key]
    return default


def _keys(row: Any) -> tuple[str, ...]:
    return tuple(str(key) for key in row.keys()) if hasattr(row, "keys") else ()


def _is_msat(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _has_privacy_boundary(row: Any) -> bool:
    value = _get(row, "privacy_boundary")
    if value is None:
        return False
    text = str(value).strip().lower().replace("-", "_")
    return text in PRIVACY_BOUNDARIES


def _corrupt_decimal(*values: Any) -> bool:
    # The first non-empty value is the one the adapter parses.
    for value in values:
        if value in (None, ""):
            continue
        try:
            Decimal(str(value))
        except InvalidOperation:
            return True
        return False
    return False


def _pool_scope(profile: Any) -> str:
    return str(_get(profile, "cost_basis_pool_scope") or "global").strip().lower()


def _row_violations(row: Any, wallet_ids: set[str]) -> set[str]:
    found: set[str] = set()
    if not str(_get(row, "id") or "").startswith(SYNTHETIC_ROW_ID_PREFIXES):
        found.add(NON_SYNTHETIC_ROW_ID)
    anchor = _get(row, "journal_transaction_id")
    if not isinstance(anchor, str) or not anchor:
        found.add(MISSING_ANCHOR)
    if str(_get(row, "wallet_id")) not in wallet_ids:
        found.add(UNKNOWN_WALLET)
    direction = _get(row, "direction")
    if direction not in DIRECTIONS:
        found.add(UNKNOWN_DIRECTION)
    amount, fee = _get(row, "amount"), _get(row, "fee")
    if not _is_msat(amount) or not _is_msat(fee):
        found.add(NON_INTEGER_MSAT)
    else:
        if fee < 0:
            found.add(NEGATIVE_FEE)
        if (direction == "inbound" and amount <= 0) or (direction == "outbound" and amount + fee <= 0):
            found.add(NON_POSITIVE_AMOUNT)
    if _get(row, "amount_includes_fee"):
        found.add(AMOUNT_INCLUDES_FEE)
    if any(key.startswith("custody_component_") for key in _keys(row)):
        found.add(CUSTODY_COMPONENT_FIELDS)
    if _corrupt_decimal(_get(row, "fiat_rate_exact"), _get(row, "fiat_rate")) or _corrupt_decimal(
        _get(row, "fiat_value_exact"), _get(row, "fiat_value")
    ):
        found.add(CORRUPT_DECIMAL_TEXT)
    return found


def violations(profile: Any, inputs: Any) -> tuple[str, ...]:
    """Every invariant ``inputs`` violate, in ``INVARIANTS`` order."""
    found: set[str] = set()
    if _pool_scope(profile) != "global":
        found.add(NON_GLOBAL_POOL_SCOPE)
    wallet_refs = inputs.wallet_refs_by_id
    if any(str(key) != str(_get(ref, "id")) for key, ref in wallet_refs.items()):
        found.add(WALLET_REF_ID_MISMATCH)
    wallet_ids = {str(key) for key in wallet_refs}
    projection = inputs.finalized_tax_projection
    for row in projection.rows:
        found |= _row_violations(row, wallet_ids)
    legs: set[str] = set()
    for pair in projection.intra_pairs:
        out_row, in_row = pair["out"], pair["in"]
        if pair.get("group_block_rows"):
            found.add(GROUP_BLOCK_ROWS)
        if (_has_privacy_boundary(out_row) or _has_privacy_boundary(in_row)) and pair.get(
            "kind"
        ) != REVIEWED_PRIVACY_PAIR_KIND:
            found.add(UNREVIEWED_PRIVACY_PAIR)
        for leg in (out_row, in_row):
            leg_id = str(_get(leg, "id"))
            if leg_id in legs:
                found.add(SHARED_PAIR_LEG)
            legs.add(leg_id)
        if _get(out_row, "amount") != _get(in_row, "amount"):
            found.add(UNEQUAL_MOVE_AMOUNTS)
    return tuple(name for name in INVARIANTS if name in found)


__all__ = ["INVARIANTS", "violations"]
