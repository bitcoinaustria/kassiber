"""Austrian (§ 27b EStG) tax-semantic classification helpers.

Kassiber owns the normalization layer: raw transaction rows become
`NormalizedTaxEvent` values with typed Austrian fields (`at_regime`,
`at_pool`, `at_swap_link`). The rp2 adapter
serializes those fields into rp2's `notes` wire format; the rp2 AT
plugin (`rp2.plugin.country.at`) interprets them.

This module holds the classification logic that runs inside
`normalize_tax_asset_inputs` and the engine layer when the profile's
`tax_country == "at"`. It is deliberately small and has no rp2 import —
rp2 defines the same cutoff in `rp2.plugin.country.at.AT_NEU_CUTOFF`,
but duplicating the constant here keeps the normalization layer free
of rp2 imports (Kassiber-core does not depend on rp2 types).
"""

from __future__ import annotations

from collections import defaultdict, deque
from datetime import datetime
from typing import Any, Literal, Mapping, Optional, Sequence
from zoneinfo import ZoneInfo

from .pair_allocation import (
    allocate_fee_msat,
    clamped_receipt_msat,
    connected_pair_components,
    ordered_pair_component,
)

# Altvermögen / Neuvermögen cutoff per § 27b EStG. Acquisitions on or before
# 2021-02-28 Europe/Vienna are Altvermögen; after that, Neuvermögen.
# Matches `rp2.plugin.country.at.AT_NEU_CUTOFF` — if rp2 ever revises the
# cutoff we must update this constant in lockstep.
AT_NEU_CUTOFF = datetime(2021, 3, 1, 0, 0, 0, tzinfo=ZoneInfo("Europe/Vienna"))

REGIME_ALT: Literal["alt"] = "alt"
REGIME_NEU: Literal["neu"] = "neu"

# Single global moving-average pool id per asset (see resolve_pool_id). Matches
# rp2.plugin.country.at.AT_DEFAULT_POOL and rp2's absent-marker fallback.
AT_DEFAULT_POOL = "default"

# Quarantine reason emitted when a reviewed Neu cross-asset swap cannot be
# marked and fed into rp2's native carry path safely. The engine adds a
# `reason_code` in the detail so callers can distinguish missing pricing,
# inventory, and other readiness failures.
AT_SWAP_QUARANTINE_REASON = "at_swap_basis_carry_unresolved"

# Kassiber owns the presentation-layer mapping from RP2's semantic Austrian
# disposal categories onto FinanzOnline / BMF form codes.
AT_CATEGORY_TO_KENNZAHL: dict[str, int | None] = {
    "income_general": 172,
    "income_capital_yield": 172,
    "neu_gain": 174,
    "neu_loss": 176,
    "neu_swap": None,
    "alt_spekulation": 801,
    "alt_taxfree": None,
}


def _row_value(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    if hasattr(row, "keys") and key in row.keys():
        return row[key]
    return default


def _parse_occurred_at(value: str) -> datetime:
    """Parse an ISO-8601 occurred_at string into an aware datetime.

    Kassiber stores RFC3339 timestamps (always with Z or ±hh:mm). If the
    stored value is naive for any reason, treat it as UTC rather than
    raising — the cutoff comparison is coarse enough that UTC vs local
    offset never moves an event across the 2021-03-01 boundary except at
    the exact boundary hour, which the AT plugin itself documents as an
    operator-decision edge.
    """
    text = value.replace("Z", "+00:00")
    parsed = datetime.fromisoformat(text)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=ZoneInfo("UTC"))
    return parsed


def infer_regime_from_timestamp(occurred_at: str) -> Literal["alt", "neu"]:
    when = _parse_occurred_at(occurred_at)
    return REGIME_ALT if when < AT_NEU_CUTOFF else REGIME_NEU


def _availability_key(wallet_id: Any) -> str:
    if wallet_id is None:
        return AT_DEFAULT_POOL
    text = str(wallet_id)
    return text if text else AT_DEFAULT_POOL


def _deplete_available(
    available_msat_by_key: dict[str, int],
    key: str,
    amount_msat: int,
) -> int:
    if amount_msat <= 0:
        return 0
    available = available_msat_by_key.get(key, 0)
    taken = min(available, amount_msat)
    available_msat_by_key[key] = max(0, available - taken)
    return taken


def _move_transfer_availability(
    alt_available_msat_by_key: dict[str, int],
    neu_available_msat_by_key: dict[str, int],
    out_row: Mapping[str, Any],
    in_row: Mapping[str, Any],
) -> tuple[Literal["alt", "neu"], dict[str, dict[str, int]]]:
    """Move regime availability for a self-transfer.

    Returns ``(fee_regime, flows)``: the regime the disposed network fee is
    drawn from, plus the per-regime quantity flows —
    ``flows["out"]`` = what left the source per regime (carried + fee slice),
    ``flows["in"]`` = what the destination received per regime. Consumers
    that classify whole MOVE quantities (tax-free balance hints) need the
    split; the single fee regime is NOT representative of the carried coins.

    The MOVE itself is non-taxable, but its miner fee IS a disposal, so under the
    Austrian moving-average method it must be tagged with a regime — otherwise
    rp2 raises ``Ambiguous Austrian disposal`` when the wallet holds both Alt and
    Neu lots and aborts the whole asset. The fee is the first slice of ``sent``,
    so it comes from the move's preferred regime (Neu when available, Alt when
    only Alt remains).
    """
    from_key = _availability_key(_row_value(out_row, "wallet_id"))
    to_key = _availability_key(_row_value(in_row, "wallet_id"))
    sent_msat = int(_row_value(out_row, "amount") or 0) + int(
        _row_value(out_row, "fee") or 0
    )
    received_msat = int(_row_value(in_row, "amount") or 0)

    preferred_regime = infer_regime_from_timestamp(str(out_row["occurred_at"]))
    empty_flows = {
        "out": {REGIME_ALT: 0, REGIME_NEU: 0},
        "in": {REGIME_ALT: 0, REGIME_NEU: 0},
    }
    if sent_msat <= 0 or received_msat <= 0:
        return preferred_regime, empty_flows
    if (
        preferred_regime == REGIME_NEU
        and alt_available_msat_by_key.get(from_key, 0) > 0
        and neu_available_msat_by_key.get(from_key, 0) <= 0
    ):
        preferred_regime = REGIME_ALT

    moved: dict[str, int] = {REGIME_ALT: 0, REGIME_NEU: 0}
    remaining_sent = sent_msat
    regime_order = (
        (REGIME_NEU, REGIME_ALT)
        if preferred_regime == REGIME_NEU
        else (REGIME_ALT, REGIME_NEU)
    )
    for regime in regime_order:
        bucket = (
            neu_available_msat_by_key
            if regime == REGIME_NEU
            else alt_available_msat_by_key
        )
        taken = _deplete_available(bucket, from_key, remaining_sent)
        moved[regime] += taken
        remaining_sent -= taken
        if remaining_sent <= 0:
            break

    fee_msat = max(0, sent_msat - received_msat)
    fee_by_regime: dict[str, int] = {REGIME_ALT: 0, REGIME_NEU: 0}
    remaining_fee = fee_msat
    for regime in regime_order:
        fee_slice = min(moved[regime], remaining_fee)
        if fee_slice <= 0:
            continue
        moved[regime] -= fee_slice
        fee_by_regime[regime] += fee_slice
        remaining_fee -= fee_slice
        if remaining_fee <= 0:
            break

    carried_by_regime: dict[str, int] = {REGIME_ALT: 0, REGIME_NEU: 0}
    remaining_received = received_msat
    for regime in regime_order:
        carried = min(moved[regime], remaining_received)
        if carried <= 0:
            continue
        bucket = (
            neu_available_msat_by_key
            if regime == REGIME_NEU
            else alt_available_msat_by_key
        )
        bucket[to_key] += carried
        carried_by_regime[regime] += carried
        remaining_received -= carried
        if remaining_received <= 0:
            break

    flows = {
        "out": {
            regime: fee_by_regime[regime] + carried_by_regime[regime]
            for regime in (REGIME_ALT, REGIME_NEU)
        },
        "in": dict(carried_by_regime),
    }
    # The fee is the first taxable slice of `sent`; only the remaining moved
    # quantity is carried to the destination. Fall back to the first moved regime
    # (or the timestamp regime) for zero-fee / out-of-scope inventory cases.
    for regime in regime_order:
        if fee_by_regime[regime] > 0:
            return regime, flows
    for regime in regime_order:
        if moved[regime] > 0:
            return regime, flows
    return preferred_regime, flows


def _row_msat(row: Mapping[str, Any], key: str) -> int:
    return int(_row_value(row, key) or 0)


def _copy_row_with_amount(
    row: Mapping[str, Any],
    *,
    amount_msat: int | None = None,
    fee_msat: int | None = None,
) -> dict[str, Any]:
    copied = {key: row[key] for key in row.keys()} if hasattr(row, "keys") else dict(row)
    if amount_msat is not None:
        copied["amount"] = amount_msat
    if fee_msat is not None:
        copied["fee"] = fee_msat
    return copied





def _intra_pair_components(
    intra_pairs: Sequence[Mapping[str, Mapping[str, Any]]],
) -> list[list[Mapping[str, Mapping[str, Any]]]]:
    def _leg_ids(pair):
        out_row = pair.get("out") if hasattr(pair, "get") else None
        in_row = pair.get("in") if hasattr(pair, "get") else None
        if out_row is None or in_row is None:
            return None
        return (out_row["id"], in_row["id"])

    # Unlike booking's multi-pair builder, ALL pairs (auto, derived,
    # samourai regime pairs) form components here: a shared inbound must be
    # allocated ONCE across the pairs that feed it, whatever their origin.
    # (Booking handles derived groups through their own per-pair path.)
    return connected_pair_components(intra_pairs, _leg_ids)

def _transfer_actions_for_intra_pairs(
    intra_pairs: Optional[Sequence[Mapping[str, Mapping[str, Any]]]],
) -> tuple[
    dict[str, list[tuple[Mapping[str, Any], Mapping[str, Any], str]]],
    set[str],
]:
    actions_by_trigger_id: dict[
        str, list[tuple[Mapping[str, Any], Mapping[str, Any], str]]
    ] = defaultdict(list)
    transfer_row_ids: set[str] = set()

    for component in _intra_pair_components(intra_pairs or ()):
        ordered_pairs = ordered_pair_component(component)
        out_rows_by_id = {
            str(pair["out"]["id"]): pair["out"] for pair in ordered_pairs
        }
        in_rows_by_id = {
            str(pair["in"]["id"]): pair["in"] for pair in ordered_pairs
        }
        group_row_ids = set(out_rows_by_id) | set(in_rows_by_id)
        transfer_row_ids.update(group_row_ids)

        if len(out_rows_by_id) > 1 and len(in_rows_by_id) > 1:
            # The tax normalizer quarantines these as ambiguous; inference must
            # not also treat their legs as ordinary acquisitions/disposals.
            continue

        total_sent_msat = sum(
            _row_msat(row, "amount") + _row_msat(row, "fee")
            for row in out_rows_by_id.values()
        )
        total_received_msat = sum(_row_msat(row, "amount") for row in in_rows_by_id.values())
        if len(ordered_pairs) == 1:
            # Booking clamps a sub-sat receipt excess on single pairs and
            # BOOKS the move (sat-truncated LND import vs msat-exact partner
            # leg). Inference must accept the identical pair or the legs
            # vanish from availability entirely — the MOVE then books with no
            # regime and a later disposal from the destination is mis-tagged.
            total_received_msat = clamped_receipt_msat(
                total_sent_msat, total_received_msat
            )
        if total_sent_msat < total_received_msat:
            # The normalizer will quarantine transfer_mismatch. Do not poison
            # wallet availability by crediting any side as a normal row.
            continue

        fee_msat = total_sent_msat - total_received_msat
        if len(out_rows_by_id) == 1:
            fee_allocations = allocate_fee_msat(
                fee_msat, [_row_msat(pair["in"], "amount") for pair in ordered_pairs]
            )
            for pair, fee_allocation in zip(ordered_pairs, fee_allocations):
                out_row = pair["out"]
                in_row = pair["in"]
                # min() carries the single-pair sub-sat clamp into the leg
                # (a no-op for genuine multi-leg components).
                received_msat = min(
                    _row_msat(in_row, "amount"), total_received_msat
                )
                action_out = _copy_row_with_amount(
                    out_row,
                    amount_msat=received_msat,
                    fee_msat=fee_allocation,
                )
                actions_by_trigger_id[str(out_row["id"])].append(
                    (action_out, in_row, str(out_row["id"]))
                )
            continue

        sent_amounts = [
            _row_msat(pair["out"], "amount") + _row_msat(pair["out"], "fee")
            for pair in ordered_pairs
        ]
        fee_allocations = allocate_fee_msat(fee_msat, sent_amounts)
        for pair, sent_msat, fee_allocation in zip(
            ordered_pairs, sent_amounts, fee_allocations
        ):
            received_msat = sent_msat - fee_allocation
            if received_msat <= 0:
                continue
            out_row = pair["out"]
            in_row = pair["in"]
            action_in = _copy_row_with_amount(in_row, amount_msat=received_msat)
            actions_by_trigger_id[str(out_row["id"])].append(
                (out_row, action_in, str(out_row["id"]))
            )

    return actions_by_trigger_id, transfer_row_ids


def infer_outbound_regimes(
    rows: Sequence[Mapping[str, Any]],
    intra_pairs: Optional[Sequence[Mapping[str, Mapping[str, Any]]]] = None,
    transfer_flows: Optional[dict[tuple[str, str], dict[str, dict[str, int]]]] = None,
) -> dict[str, Literal["alt", "neu"]]:
    """Infer Austrian disposal regimes from the rows seen so far.

    v1 keeps the branch's existing bias toward Neu for post-cutoff disposals
    when a Neu pool is still populated, but it must not force `at_regime=neu`
    once only Alt inventory remains in the disposing wallet. The emitted rp2
    moving-average pool is global, but regime availability stays wallet-aware
    and explicit same-asset internal transfers move availability between wallets.
    The result is a best-effort per-row map that callers can reuse both for
    normal outbound events and for cross-asset swap classification.
    """

    alt_available_msat_by_key: dict[str, int] = defaultdict(int)
    neu_available_msat_by_key: dict[str, int] = defaultdict(int)
    regimes_by_row_id: dict[str, Literal["alt", "neu"]] = {}
    transfer_actions_by_row_id, transfer_row_ids = _transfer_actions_for_intra_pairs(intra_pairs)

    for row in rows:
        row_id = str(row["id"])
        transfer_actions = transfer_actions_by_row_id.get(row_id)
        if transfer_actions:
            # Process moves at their OUT leg's position, not whichever leg is
            # seen first. IN legs sort ahead of same-timestamp acquisitions, and
            # triggering there can deplete the pool before those acquisitions
            # land. Multi-pair groups are sliced here the same way the tax
            # normalizer later emits their transfer rows.
            for out_row, in_row, regime_row_id in transfer_actions:
                fee_regime, flows = _move_transfer_availability(
                    alt_available_msat_by_key,
                    neu_available_msat_by_key,
                    out_row,
                    in_row,
                )
                # Tag the self-transfer's out row so its taxable miner-fee
                # disposal carries a regime; without it rp2's AT moving-average
                # aborts the whole asset on "Ambiguous Austrian disposal".
                regimes_by_row_id.setdefault(regime_row_id, fee_regime)
                if transfer_flows is not None:
                    # Per-leg regime flows so downstream consumers (tax-free
                    # balance hints) classify the moved QUANTITIES, not the
                    # whole MOVE by the fee's single regime.
                    transfer_flows[
                        (str(out_row["id"]), str(in_row["id"]))
                    ] = flows
            continue
        if row_id in transfer_row_ids:
            continue

        direction = str(_row_value(row, "direction") or "").strip().lower()
        amount_msat = int(_row_value(row, "amount") or 0)
        fee_msat = int(_row_value(row, "fee") or 0)
        availability_key = _availability_key(_row_value(row, "wallet_id"))
        if direction == "inbound":
            regime = infer_regime_from_timestamp(str(row["occurred_at"]))
            if regime == REGIME_ALT:
                alt_available_msat_by_key[availability_key] += amount_msat
            else:
                neu_available_msat_by_key[availability_key] += amount_msat
            continue
        if direction != "outbound":
            continue

        regime = infer_regime_from_timestamp(str(row["occurred_at"]))
        if (
            regime == REGIME_NEU
            and alt_available_msat_by_key.get(availability_key, 0) > 0
            and neu_available_msat_by_key.get(availability_key, 0) <= 0
        ):
            regime = REGIME_ALT
        regimes_by_row_id[str(row["id"])] = regime

        needed_msat = amount_msat + fee_msat
        if needed_msat <= 0:
            continue
        if regime == REGIME_ALT:
            _deplete_available(alt_available_msat_by_key, availability_key, needed_msat)
        else:
            _deplete_available(neu_available_msat_by_key, availability_key, needed_msat)

    return regimes_by_row_id


def resolve_pool_id(wallet_id: Optional[str]) -> str:  # wallet_id reserved as a future per-wallet seam; intentionally unused
    """Single global Neuvermögen moving-average pool per asset.

    The gleitender Durchschnittspreis (§ 2 KryptowährungsVO) is computed over the
    taxpayer's *entire holding of each cryptocurrency*, not per wallet. rp2 runs one
    accounting pass per asset, so a single constant pool id == the whole per-asset
    holding. This also keeps the Austrian cost-basis pool consistent with the rest of
    the engine, where cost basis is global (universal-application FIFO) and only the
    *availability* gate is per-`(exchange, holder)` — see kassiber/core/engines/rp2.py.

    Returning one pool id regardless of wallet fixes the multi-wallet hazard where coins
    acquired in one wallet and sold from another were tagged with different `at_pool`s, so
    rp2's `moving_average_at` found no lots in the disposal's pool (bitcoinaustria/kassiber#213,
    bitcoinaustria/rp2#7). `wallet_id` is retained as the seam for a hypothetical future
    per-wallet scheme; it is intentionally unused today. The wallet-keyed *output shape*
    (a single pool-id string) is preserved, so upstream consumers do not change.
    """
    return AT_DEFAULT_POOL


def kennzahl_for_disposal_category(category: Optional[str]) -> int | None:
    if category is None:
        return None
    return AT_CATEGORY_TO_KENNZAHL.get(str(category))


__all__ = [
    "AT_CATEGORY_TO_KENNZAHL",
    "AT_DEFAULT_POOL",
    "AT_NEU_CUTOFF",
    "AT_SWAP_QUARANTINE_REASON",
    "REGIME_ALT",
    "REGIME_NEU",
    "infer_outbound_regimes",
    "infer_regime_from_timestamp",
    "kennzahl_for_disposal_category",
    "resolve_pool_id",
]
