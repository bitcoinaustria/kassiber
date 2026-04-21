"""Austrian (§ 27b EStG) tax-semantic classification helpers.

Kassiber owns the normalization layer: raw transaction rows become
`NormalizedTaxEvent` values with typed Austrian fields (`at_regime`,
`at_pool`, `at_swap_link`, `carried_basis_fiat`). The rp2 adapter
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

# Quarantine reason emitted when a Neu cross-asset swap cannot have its
# carried basis computed in a single normalization pass. The engine adds
# a `reason_code` in the detail to distinguish v1 skip-reasons from other
# swap validation problems that might share the same reason in the future.
AT_SWAP_QUARANTINE_REASON = "at_swap_basis_carry_unresolved"
AT_SWAP_TWO_PASS_REASON_CODE = "needs_two_pass_compute"

# Kassiber owns the presentation-layer mapping from RP2's semantic Austrian
# disposal categories onto FinanzOnline / BMF form codes.
AT_CATEGORY_TO_KENNZAHL: dict[str, int | None] = {
    "income_general": 172,
    "income_capital_yield": 175,
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


def infer_outbound_regimes(rows: Sequence[Mapping[str, Any]]) -> dict[str, Literal["alt", "neu"]]:
    """Infer Austrian disposal regimes from the rows seen so far.

    v1 keeps the branch's existing bias toward Neu for post-cutoff disposals
    when a Neu pool is still populated, but it must not force `at_regime=neu`
    once only Alt inventory remains. The result is a best-effort per-row map
    that callers can reuse both for normal outbound events and for cross-asset
    swap classification.
    """

    alt_available_msat = 0
    neu_available_msat_by_pool: dict[str, int] = defaultdict(int)
    regimes_by_row_id: dict[str, Literal["alt", "neu"]] = {}

    for row in rows:
        direction = str(_row_value(row, "direction") or "").strip().lower()
        amount_msat = int(_row_value(row, "amount") or 0)
        fee_msat = int(_row_value(row, "fee") or 0)
        if direction == "inbound":
            regime = infer_regime_from_timestamp(str(row["occurred_at"]))
            if regime == REGIME_ALT:
                alt_available_msat += amount_msat
            else:
                pool_id = resolve_pool_id(_row_value(row, "wallet_id"))
                neu_available_msat_by_pool[pool_id] += amount_msat
            continue
        if direction != "outbound":
            continue

        pool_id = resolve_pool_id(_row_value(row, "wallet_id"))
        regime = infer_regime_from_timestamp(str(row["occurred_at"]))
        if regime == REGIME_NEU and alt_available_msat > 0 and neu_available_msat_by_pool.get(pool_id, 0) <= 0:
            regime = REGIME_ALT
        regimes_by_row_id[str(row["id"])] = regime

        needed_msat = amount_msat + fee_msat
        if needed_msat <= 0:
            continue
        if regime == REGIME_ALT:
            alt_available_msat = max(0, alt_available_msat - needed_msat)
        else:
            neu_available_msat_by_pool[pool_id] = max(0, neu_available_msat_by_pool.get(pool_id, 0) - needed_msat)

    return regimes_by_row_id


def resolve_pool_id(wallet_id: Optional[str]) -> str:
    """One pool per wallet. Falls back to `"default"` when wallet_id is missing.

    v1 choice — documented in docs/austrian-handoff.md. Future multi-wallet
    pool schemes (e.g. one pool per exchange account, one global pool) must
    feed the same wallet_id-keyed output shape so upstream consumers don't
    change.
    """
    if not wallet_id:
        return "default"
    return str(wallet_id)


def kennzahl_for_disposal_category(category: Optional[str]) -> int | None:
    if category is None:
        return None
    return AT_CATEGORY_TO_KENNZAHL.get(str(category))


__all__ = [
    "AT_CATEGORY_TO_KENNZAHL",
    "AT_NEU_CUTOFF",
    "AT_SWAP_QUARANTINE_REASON",
    "AT_SWAP_TWO_PASS_REASON_CODE",
    "REGIME_ALT",
    "REGIME_NEU",
    "infer_outbound_regimes",
    "infer_regime_from_timestamp",
    "kennzahl_for_disposal_category",
    "resolve_pool_id",
]
