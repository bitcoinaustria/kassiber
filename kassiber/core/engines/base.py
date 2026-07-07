from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Protocol, Sequence


@dataclass(frozen=True)
class TaxEngineLedgerInputs:
    """Raw per-profile inputs loaded from SQLite before engine processing."""

    rows: Sequence[Mapping[str, Any]]
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]]
    manual_pair_records: Sequence[Mapping[str, Any]]
    direct_payout_records: Sequence[Mapping[str, Any]] = ()
    # Prebuilt profile-wide address-ownership index (kassiber.core.ownership.
    # OwnedIndex) used to derive self-transfers from the transaction graph;
    # ``None`` when no on-chain transaction JSON is available to read.
    owned_index: Any = None
    # Active loan legs (rows from ``loan_legs`` with a non-null transaction_id).
    # Each carries a ``role`` that classifies the matching journal transaction:
    # collateral lock/release and borrowed-principal receive/repay roles are
    # non-events, while unmarked liquidation falls through to normal disposal.
    loan_legs: Sequence[Mapping[str, Any]] = ()
    # Derived Lightning channel-lifecycle roles: ``{transaction_id: role}`` where
    # role is ``channel_open`` / ``channel_close`` (kassiber.core.loans). Built
    # from owned channels' funding/closing txids matched against on-chain rows;
    # merged into the same non-event suppression as loan legs so a channel
    # funding tx is not booked as a disposal nor a close as an acquisition.
    channel_roles: Mapping[str, str] = None  # type: ignore[assignment]


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


class TaxEngine(Protocol):
    """Profile-level tax engine interface.

    Engines receive raw per-profile journal inputs and return the aggregated
    ledger state that handlers persist into Kassiber's journal tables.
    """

    def build_ledger_state(self, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        """Return aggregated ledger state for one profile."""


__all__ = [
    "TaxEngine",
    "TaxEngineLedgerInputs",
    "TaxEngineLedgerResult",
]
