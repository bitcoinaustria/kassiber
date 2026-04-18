"""Core tax-engine helpers."""

from __future__ import annotations

from typing import Any, Mapping

from ...tax_policy import AUSTRIAN_TAX_COUNTRY, normalize_tax_country, profile_value
from .austria import ExperimentalAustrianTaxEngine
from .base import TaxEngine, TaxEngineLedgerInputs, TaxEngineLedgerResult
from .rp2 import GenericRP2TaxEngine


def build_tax_engine(profile: Mapping[str, Any]) -> TaxEngine:
    """Return the current tax engine implementation for a profile."""

    country = normalize_tax_country(profile_value(profile, "tax_country"))
    if country == AUSTRIAN_TAX_COUNTRY:
        return ExperimentalAustrianTaxEngine(profile)
    return GenericRP2TaxEngine(profile)


__all__ = [
    "TaxEngine",
    "TaxEngineLedgerInputs",
    "TaxEngineLedgerResult",
    "build_tax_engine",
]
