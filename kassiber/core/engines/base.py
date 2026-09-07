from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Sequence

from ..custody_tax_projection import FinalizedTaxProjection


@dataclass(frozen=True)
class TaxEngineLedgerInputs:
    """Strict tax-engine input: already-finalized custody tax projection.

    Production engines must never accept ``transactions`` rows.  Raw evidence
    belongs to the custody interpreter/arbitrator phase and is represented here
    only through :class:`FinalizedTaxProjection`.
    """

    finalized_tax_projection: FinalizedTaxProjection
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]]
    # Compatibility/report metadata only; it cannot introduce tax rows.
    direct_payout_records: Sequence[Mapping[str, Any]] = ()


@dataclass(frozen=True)
class TaxEngineLedgerResult:
    """Aggregated journal state emitted for one processed profile.

    ``entries`` and ``quarantines`` match the rows persisted into the journal
    tables. ``intra_audit`` contains the same-asset transfer audit trail used by
    the current generic RP2 path. ``cross_asset_pairs`` carries manual
    cross-asset pair metadata for envelope/report consumers without feeding
    those pairs into RP2. ``tax_summary`` carries RP2 yearly gain/loss summary
    rows before report-layer totals are added.

    ``account_holdings`` keys are ``(account_id, account_code, account_label,
    asset)`` tuples. ``wallet_holdings`` keys are ``(wallet_id, wallet_label,
    account_code, asset)`` tuples. Each mapped value stores Decimal
    ``quantity`` and ``cost_basis`` totals.
    """

    entries: list[dict[str, Any]]
    quarantines: list[dict[str, Any]]
    intra_audit: list[dict[str, Any]]
    cross_asset_pairs: list[dict[str, Any]]
    direct_swap_payouts: list[dict[str, Any]]
    tax_summary: list[dict[str, Any]]
    account_holdings: dict[tuple[Any, ...], dict[str, Any]]
    wallet_holdings: dict[tuple[Any, ...], dict[str, Any]]


@dataclass(frozen=True)
class TaxEngineCalculationResult:
    """Exact replay material from an explicitly bounded calculation run.

    This is not a general ledger or a legal approval. Original acquisition
    values, cutoff-specific effective basis and raw gain/loss fragments remain
    distinct. No runtime RP2 object, wallet configuration or raw node JSON may
    cross this Interface.
    """

    cutoff_exclusive_utc: str
    calculation_timezone: str
    inputs: dict[str, Any]
    assets: list[dict[str, Any]]
    blockers: list[dict[str, Any]]


__all__ = [
    "TaxEngineLedgerInputs",
    "TaxEngineLedgerResult",
    "TaxEngineCalculationResult",
]
