"""Indicative Austrian tax for one tax year of one book.

The assistant and the reports read this figure instead of applying a rate
themselves: a model that recalls tax law from its weights gets the rate or the
netting wrong, and a figure the core computes is the same everywhere.

Scope, deliberately narrow:

- Base: E 1kv KZ 172 (laufende Einkünfte) + KZ 174 (realisierte
  Wertsteigerungen) − KZ 176 (realisierte Wertverluste), exactly as the
  Kennzahl summary already reports them. Losses net within the year; a negative
  base estimates zero.
- Rate: the 27.5 % Sondersteuersatz (§ 27a EStG), shared with the exit-tax
  preview.
- KZ 801 (Altbestand Spekulationsgeschäfte, § 31 EStG) is taxed at the
  progressive tariff, which depends on income outside the book. It is reported
  as an amount and never estimated.
- Any report blocker withholds the figure. A stale journal, a quarantine or a
  custody gap would make a confident-looking number wrong.

This module owns no tax math beyond multiplying the reported base; it does not
rebuild Alt/Neu classification or moving-average basis (RP2 owns those).
Not tax advice.
"""

from __future__ import annotations

from decimal import ROUND_HALF_UP, Decimal
from typing import Any, Iterable, Mapping

from .exit_tax import AT_SONDERSTEUERSATZ

SPECIAL_RATE_GAIN_KENNZAHLEN = (172, 174)
SPECIAL_RATE_LOSS_KENNZAHLEN = (176,)
PROGRESSIVE_RATE_KENNZAHLEN = (801,)

CAVEATS = (
    "Indicative only, not tax advice; a Steuerberater confirms the figure.",
    "Covers this book only. Capital income and losses outside it (securities, "
    "other books) net against the same 27.5% pool on the real return.",
    "Kassiber files every amount under the foreign-income Kennzahlen; tax an "
    "Austrian provider already withheld (KESt) is not deducted.",
)
PROGRESSIVE_NOTE = (
    "KZ 801 (Altbestand Spekulationsgeschäfte) is taxed at the progressive "
    "income-tax rate, which depends on income outside this book; it is not "
    "estimated."
)


def _cents(rows: Mapping[int, int], codes: Iterable[int]) -> int:
    return sum(int(rows.get(code, 0)) for code in codes)


def austrian_indicative_tax(
    *,
    tax_year: int,
    kennzahl_rows: Iterable[Mapping[str, Any]],
    blocker_ids: Iterable[str] = (),
) -> dict[str, Any]:
    """Return the indicative 27.5 % figure for one year, or why it is withheld.

    ``kennzahl_rows`` are ``build_austrian_kennzahl_summary`` rows, whose
    ``amount_eur_cents`` already carries KZ 176 as a positive loss amount.
    """

    by_code: dict[int, int] = {}
    for row in kennzahl_rows:
        try:
            code = int(row["kennzahl"])
        except (KeyError, TypeError, ValueError):
            continue
        by_code[code] = by_code.get(code, 0) + int(row.get("amount_eur_cents") or 0)

    gains = _cents(by_code, SPECIAL_RATE_GAIN_KENNZAHLEN)
    losses = _cents(by_code, SPECIAL_RATE_LOSS_KENNZAHLEN)
    base = gains - losses
    blockers = sorted({str(item) for item in blocker_ids if item})
    estimate: int | None = None
    if not blockers:
        estimate = int(
            (Decimal(max(base, 0)) * AT_SONDERSTEUERSATZ).quantize(
                Decimal("1"), rounding=ROUND_HALF_UP
            )
        )
    progressive = {
        str(code): by_code[code]
        for code in PROGRESSIVE_RATE_KENNZAHLEN
        if by_code.get(code)
    }
    return {
        "jurisdiction": "AT",
        "tax_year": int(tax_year),
        "status": "withheld" if blockers else "indicative",
        "withheld_by": blockers,
        "rate": str(AT_SONDERSTEUERSATZ),
        "basis": "E 1kv KZ 172 + KZ 174 - KZ 176, this book only",
        "special_rate_gains_eur_cents": gains,
        "special_rate_losses_eur_cents": losses,
        "special_rate_base_eur_cents": base,
        "estimated_tax_eur_cents": estimate,
        "progressive_rate_amounts_eur_cents": progressive,
        "notes": [*CAVEATS, *([PROGRESSIVE_NOTE] if progressive else [])],
    }


__all__ = [
    "CAVEATS",
    "PROGRESSIVE_RATE_KENNZAHLEN",
    "SPECIAL_RATE_GAIN_KENNZAHLEN",
    "SPECIAL_RATE_LOSS_KENNZAHLEN",
    "austrian_indicative_tax",
]
