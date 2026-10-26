"""Austrian (§ 27b EStG) tax-semantic helpers.

The tax engine (`tax-engine/core`) derives the Austrian markers (regime,
pool, swap link, election) and classifies disposals. This module keeps what
Kassiber's Python layers read around it: the Altvermögen/Neuvermögen cutoff
and the timestamp parse the request builder sends with every row
(`engines/ledger_request.py`), the Europe/Vienna tax-year helpers of the
reports, the Austrian names the engine's manifest is checked against, and
the report layer's Kennzahl mapping. It has no engine import: the engine
defines the same cutoff as `NEU_CUTOFF_US`.
"""

from __future__ import annotations

from datetime import datetime
from typing import Literal, Optional
from zoneinfo import ZoneInfo

# Altvermögen / Neuvermögen cutoff per § 27b EStG. Acquisitions on or before
# 2021-02-28 Europe/Vienna are Altvermögen; after that, Neuvermögen.
# Matches `NEU_CUTOFF_US` in `tax-engine/core/src/austria.rs`; a revised
# cutoff must update both in lockstep (docs/austrian-handoff.md).
AT_TAX_TZ = ZoneInfo("Europe/Vienna")
AT_NEU_CUTOFF = datetime(2021, 3, 1, 0, 0, 0, tzinfo=AT_TAX_TZ)

REGIME_ALT: Literal["alt"] = "alt"
REGIME_NEU: Literal["neu"] = "neu"

# Austrian plugin wire id for the global moving-average pool. Generic books
# use the opaque `global` identity; only the engine's Austrian descriptions
# translate that identity to this legacy marker value.
AT_DEFAULT_POOL = "default"

# Quarantine reason emitted when a reviewed Neu cross-asset swap cannot be
# marked and fed into the engine's swap carry safely. The engine adds a
# `reason_code` in the detail so callers can distinguish missing pricing,
# inventory, and other readiness failures.
AT_SWAP_QUARANTINE_REASON = "at_swap_basis_carry_unresolved"

# Kassiber owns the presentation-layer mapping from the tax engine's semantic
# Austrian disposal categories onto FinanzOnline / BMF form codes.
AT_CATEGORY_TO_KENNZAHL: dict[str, int | None] = {
    "income_general": 172,
    "income_capital_yield": 172,
    "neu_gain": 174,
    "neu_loss": 176,
    "neu_swap": None,
    "alt_spekulation": 801,
    "alt_taxfree": None,
}


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


def tax_year_in_vienna(occurred_at: str) -> int:
    """Return the Austrian calendar tax year for an RFC3339 timestamp.

    Austrian reports follow the Europe/Vienna local calendar. Stored timestamps
    are UTC, so a disposal at ``2024-12-31T23:30:00Z`` is tax year **2025**.
    """
    return _parse_occurred_at(occurred_at).astimezone(AT_TAX_TZ).year


def vienna_local_date(occurred_at: str) -> str:
    """Return ``YYYY-MM-DD`` in Europe/Vienna for an RFC3339 timestamp."""
    return _parse_occurred_at(occurred_at).astimezone(AT_TAX_TZ).date().isoformat()


def vienna_tax_year_utc_window(tax_year: int) -> tuple[str, str]:
    """Loose UTC bounds that cover every Europe/Vienna local date in ``tax_year``.

    Callers should still apply :func:`tax_year_in_vienna` for the exact year.
    """
    year = int(tax_year)
    return (f"{year - 1}-12-31T00:00:00Z", f"{year + 1}-01-02T00:00:00Z")


def kennzahl_for_disposal_category(category: Optional[str]) -> int | None:
    if category is None:
        return None
    return AT_CATEGORY_TO_KENNZAHL.get(str(category))


__all__ = [
    "AT_CATEGORY_TO_KENNZAHL",
    "AT_DEFAULT_POOL",
    "AT_NEU_CUTOFF",
    "AT_SWAP_QUARANTINE_REASON",
    "AT_TAX_TZ",
    "REGIME_ALT",
    "REGIME_NEU",
    "infer_regime_from_timestamp",
    "kennzahl_for_disposal_category",
    "tax_year_in_vienna",
    "vienna_local_date",
    "vienna_tax_year_utc_window",
]
