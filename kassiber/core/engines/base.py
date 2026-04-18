from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Protocol, Sequence


@dataclass(frozen=True)
class TaxEngineAssetResult:
    """Per-asset engine result kept for internal engine compatibility."""

    entries: list[dict[str, Any]]
    quarantines: list[dict[str, Any]]
    intra_audit: list[dict[str, Any]]
    account_holdings: dict[tuple[Any, ...], dict[str, Any]]
    wallet_holdings: dict[tuple[Any, ...], dict[str, Any]]


@dataclass(frozen=True)
class TaxEngineLedgerInputs:
    """Raw per-profile inputs loaded from SQLite before engine processing."""

    rows: Sequence[Mapping[str, Any]]
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]]
    manual_pair_records: Sequence[Mapping[str, Any]]


@dataclass(frozen=True)
class TaxEngineLedgerResult:
    """Aggregated journal state emitted for one processed profile.

    ``entries`` and ``quarantines`` match the rows persisted into the journal
    tables. ``intra_audit`` contains the same-asset transfer audit trail used by
    the current generic RP2 path. ``cross_asset_pairs`` carries manual
    cross-asset pair metadata for envelope/report consumers without feeding
    those pairs into RP2.

    ``account_holdings`` keys are ``(account_id, account_code, account_label,
    asset)`` tuples. ``wallet_holdings`` keys are ``(wallet_id, wallet_label,
    account_code, asset)`` tuples. Each mapped value stores Decimal
    ``quantity`` and ``cost_basis`` totals.
    """

    entries: list[dict[str, Any]]
    quarantines: list[dict[str, Any]]
    intra_audit: list[dict[str, Any]]
    cross_asset_pairs: list[dict[str, Any]]
    account_holdings: dict[tuple[Any, ...], dict[str, Any]]
    wallet_holdings: dict[tuple[Any, ...], dict[str, Any]]


class TaxEngine(Protocol):
    """Profile-level tax engine interface."""

    def build_ledger_state(self, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        """Return aggregated ledger state for one profile."""


__all__ = [
    "TaxEngine",
    "TaxEngineLedgerInputs",
    "TaxEngineLedgerResult",
]
