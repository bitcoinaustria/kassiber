"""Core tax-engine helpers."""

from __future__ import annotations

from typing import Any, Mapping

from .base import TaxEngine, TaxEngineAssetResult
from .rp2 import GenericRP2TaxEngine


def build_tax_engine(profile: Mapping[str, Any]) -> TaxEngine:
    """Return the current tax engine implementation for a profile."""

    return GenericRP2TaxEngine(profile)


__all__ = [
    "TaxEngine",
    "TaxEngineAssetResult",
    "build_tax_engine",
]
