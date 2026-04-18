from __future__ import annotations

from typing import Any, Iterable, Mapping

from ...errors import AppError
from ..tax_events import NormalizedTaxAssetInputs
from .base import TaxEngineAssetResult, TaxEngineLedgerInputs, TaxEngineLedgerResult


class ExperimentalAustrianTaxEngine:
    """Placeholder engine for the experimental Austrian policy path.

    The policy is registered so profiles can opt into Austrian metadata,
    but journal processing remains explicitly gated until the dedicated
    engine has enough provenance support and Steuerberater review.
    """

    def __init__(self, profile: Mapping[str, Any]):
        self.profile = profile

    def _raise(self):
        raise AppError(
            "The Austrian tax engine is registered but still experimental and not yet available for journal processing.",
            code="experimental_tax_policy",
            hint=(
                "Use --tax-country generic for the current RP2-backed flow, or wait until the Austrian engine "
                "lands and is reviewed by a Steuerberater."
            ),
            details={
                "tax_country": "at",
                "status": "experimental",
                "review_required": "steuerberater",
            },
        )

    def make_configuration(self, wallet_labels: Iterable[str], assets: Iterable[str]) -> tuple[Any, str | None]:
        self._raise()

    def build_ledger_state(self, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        self._raise()

    def process_asset(
        self,
        normalized_inputs: NormalizedTaxAssetInputs,
        wallet_refs_by_label: Mapping[str, Mapping[str, Any]],
        configuration: Any,
    ) -> TaxEngineAssetResult:
        self._raise()


__all__ = ["ExperimentalAustrianTaxEngine"]
