"""Core tax-engine helpers."""

from __future__ import annotations

from typing import Any, Mapping

from ...tax_policy import require_tax_processing_supported
from .base import TaxEngine, TaxEngineLedgerInputs, TaxEngineLedgerResult
from .rp2 import GenericRP2TaxEngine


def build_tax_engine(profile: Mapping[str, Any]) -> TaxEngine:
    """Return the current tax engine implementation for a profile."""

    require_tax_processing_supported(profile)
    return GenericRP2TaxEngine(profile)


__all__ = [
    "TaxEngine",
    "TaxEngineLedgerInputs",
    "TaxEngineLedgerResult",
    "build_tax_engine",
]
