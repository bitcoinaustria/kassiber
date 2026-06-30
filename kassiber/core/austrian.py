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

from collections import defaultdict
from datetime import datetime
from typing import Any, Literal, Mapping, Optional, Sequence
from zoneinfo import ZoneInfo

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
) -> Literal["alt", "neu"]:
    """Move regime availability for a self-transfer and return the regime the
    disposed network fee is drawn from.

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
    if sent_msat <= 0 or received_msat <= 0:
        return preferred_regime
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
        remaining_received -= carried
        if remaining_received <= 0:
            break

    # The fee is the first taxable slice of `sent`; only the remaining moved
    # quantity is carried to the destination. Fall back to the first moved regime
    # (or the timestamp regime) for zero-fee / out-of-scope inventory cases.
    for regime in regime_order:
        if fee_by_regime[regime] > 0:
            return regime
    for regime in regime_order:
        if moved[regime] > 0:
            return regime
    return preferred_regime


def infer_outbound_regimes(
    rows: Sequence[Mapping[str, Any]],
    intra_pairs: Optional[Sequence[Mapping[str, Mapping[str, Any]]]] = None,
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
    transfer_by_row_id: dict[str, Mapping[str, Mapping[str, Any]]] = {}
    for pair in intra_pairs or ():
        out_row = pair.get("out")
        in_row = pair.get("in")
        if out_row is None or in_row is None:
            continue
        transfer_by_row_id[str(out_row["id"])] = pair
        transfer_by_row_id[str(in_row["id"])] = pair
    handled_transfer_keys: set[tuple[str, str]] = set()

    for row in rows:
        transfer_pair = transfer_by_row_id.get(str(row["id"]))
        if transfer_pair is not None:
            out_row = transfer_pair["out"]
            in_row = transfer_pair["in"]
            # Process the move at its OUT leg's position, not whichever leg is
            # seen first. The IN leg is an inbound and sorts ahead of
            # same-timestamp acquisitions (and the move's own out leg); triggering
            # there would deplete the pool before those acquisitions land,
            # flipping the move's fee regime (and any later disposal) on a
            # transaction-id tiebreak. Skip the IN leg; the OUT leg, sorted after
            # same-timestamp acquisitions, drives the move.
            if str(row["id"]) != str(out_row["id"]):
                continue
            pair_key = (str(out_row["id"]), str(in_row["id"]))
            if pair_key not in handled_transfer_keys:
                handled_transfer_keys.add(pair_key)
                fee_regime = _move_transfer_availability(
                    alt_available_msat_by_key,
                    neu_available_msat_by_key,
                    out_row,
                    in_row,
                )
                # Tag the self-transfer's out row so its taxable miner-fee
                # disposal carries a regime; without it rp2's AT moving-average
                # aborts the whole asset on "Ambiguous Austrian disposal".
                regimes_by_row_id[str(out_row["id"])] = fee_regime
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
