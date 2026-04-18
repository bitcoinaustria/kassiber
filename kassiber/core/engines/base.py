from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping, Protocol

from ..tax_events import NormalizedTaxAssetInputs


@dataclass(frozen=True)
class TaxEngineAssetResult:
    entries: list[dict[str, Any]]
    quarantines: list[dict[str, Any]]
    intra_audit: list[dict[str, Any]]
    account_holdings: dict[tuple[Any, ...], dict[str, Any]]
    wallet_holdings: dict[tuple[Any, ...], dict[str, Any]]


class TaxEngine(Protocol):
    def make_configuration(self, wallet_labels: Iterable[str], assets: Iterable[str]) -> tuple[Any, str | None]:
        """Return the engine-specific configuration plus an optional cleanup token."""

    def process_asset(
        self,
        normalized_inputs: NormalizedTaxAssetInputs,
        wallet_refs_by_label: Mapping[str, Mapping[str, Any]],
        configuration: Any,
    ) -> TaxEngineAssetResult:
        """Return journal entries, quarantines, and holding deltas for one asset."""


__all__ = ["TaxEngine", "TaxEngineAssetResult"]
