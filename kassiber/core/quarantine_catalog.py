"""One vocabulary for why a transaction is held out of the tax journals.

The custody interpreter, the finalized tax projection and the RP2 gate produce
quarantine reasons independently. This module only classifies those reasons:
which one explains a transaction when several apply, what kind of evidence is
missing, and which user action can supply it. It never decides whether a
transaction is quarantined, and an action here is a pointer to an existing
review flow, not an accounting decision.

A *downstream* reason is a consequence of another transaction's problem (for
example every later disposal in a pool behind an unresolved custody gap). It
clears by itself once that root is resolved, so it never outranks a root cause
for the same transaction.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Sequence

CATEGORY_MISSING_WALLET_HISTORY = "missing_wallet_history"
CATEGORY_MISSING_CHAIN_EVIDENCE = "missing_chain_evidence"
CATEGORY_MISSING_PRICE = "missing_price"
CATEGORY_MISSING_ACQUISITION_HISTORY = "missing_acquisition_history"
CATEGORY_NEEDS_DECISION = "needs_decision"
CATEGORY_UNSUPPORTED = "unsupported"
CATEGORY_DOWNSTREAM = "downstream"

CATEGORIES = (
    CATEGORY_MISSING_WALLET_HISTORY,
    CATEGORY_MISSING_CHAIN_EVIDENCE,
    CATEGORY_MISSING_PRICE,
    CATEGORY_MISSING_ACQUISITION_HISTORY,
    CATEGORY_NEEDS_DECISION,
    CATEGORY_UNSUPPORTED,
    CATEGORY_DOWNSTREAM,
)

ACTION_SYNC_WALLET = "sync_wallet"
ACTION_CONNECT_WALLET = "connect_wallet"
ACTION_IMPORT_HISTORY = "import_history"
ACTION_SET_PRICE = "set_price"
ACTION_CLASSIFY = "classify"
ACTION_PAIR_TRANSFER = "pair_transfer"
ACTION_REVIEW_PAIR = "review_pair"
ACTION_REVIEW_CUSTODY_GAP = "review_custody_gap"
ACTION_ATTACH_EVIDENCE = "attach_evidence"
ACTION_WAIT_FOR_CONFIRMATION = "wait_for_confirmation"
ACTION_RESOLVE_ROOT = "resolve_root"
ACTION_PROCESS_JOURNALS = "process_journals"


@dataclass(frozen=True)
class ReasonInfo:
    category: str
    actions: tuple[str, ...] = ()

    @property
    def downstream(self) -> bool:
        return self.category == CATEGORY_DOWNSTREAM


def _info(category: str, *actions: str) -> ReasonInfo:
    return ReasonInfo(category, tuple(actions))


_REASONS: dict[str, ReasonInfo] = {
    # Own-wallet movements whose other side is not (fully) observed.
    "ownership_transfer_source_missing": _info(
        CATEGORY_MISSING_WALLET_HISTORY, ACTION_SYNC_WALLET, ACTION_IMPORT_HISTORY
    ),
    "ownership_transfer_destination_missing_ref": _info(
        CATEGORY_MISSING_WALLET_HISTORY, ACTION_CONNECT_WALLET
    ),
    "channel_open_unresolved": _info(
        CATEGORY_MISSING_WALLET_HISTORY, ACTION_IMPORT_HISTORY, ACTION_CONNECT_WALLET
    ),
    "channel_close_unresolved": _info(
        CATEGORY_MISSING_WALLET_HISTORY, ACTION_IMPORT_HISTORY, ACTION_CONNECT_WALLET
    ),
    # The chain graph Kassiber holds is not complete enough to prove quantities.
    "ownership_transfer_amount_mismatch": _info(
        CATEGORY_MISSING_CHAIN_EVIDENCE, ACTION_SYNC_WALLET
    ),
    "ownership_transfer_conservation_mismatch": _info(
        CATEGORY_MISSING_CHAIN_EVIDENCE, ACTION_SYNC_WALLET
    ),
    "ownership_transfer_fee_evidence_incomplete": _info(
        CATEGORY_MISSING_CHAIN_EVIDENCE, ACTION_SYNC_WALLET
    ),
    "ownership_transfer_asset_evidence_incomplete": _info(
        CATEGORY_MISSING_CHAIN_EVIDENCE, ACTION_SYNC_WALLET
    ),
    "liquid_transfer_graph_incomplete": _info(
        CATEGORY_MISSING_CHAIN_EVIDENCE, ACTION_SYNC_WALLET, ACTION_CONNECT_WALLET
    ),
    "recorded_transfer_authority_conflict": _info(
        CATEGORY_MISSING_CHAIN_EVIDENCE, ACTION_SYNC_WALLET
    ),
    "samourai_native_event_unverified": _info(
        CATEGORY_MISSING_CHAIN_EVIDENCE, ACTION_SYNC_WALLET
    ),
    "pending_onchain_confirmation": _info(
        CATEGORY_MISSING_CHAIN_EVIDENCE, ACTION_WAIT_FOR_CONFIRMATION
    ),
    "conflicting_spend": _info(
        CATEGORY_MISSING_CHAIN_EVIDENCE, ACTION_WAIT_FOR_CONFIRMATION, ACTION_SYNC_WALLET
    ),
    # Prices.
    "missing_spot_price": _info(CATEGORY_MISSING_PRICE, ACTION_SET_PRICE),
    "pricing_review_required": _info(CATEGORY_MISSING_PRICE, ACTION_SET_PRICE),
    "at_swap_price_required": _info(CATEGORY_MISSING_PRICE, ACTION_SET_PRICE),
    # A disposal needs earlier acquisitions Kassiber has not seen or priced.
    "insufficient_lots": _info(
        CATEGORY_MISSING_ACQUISITION_HISTORY, ACTION_IMPORT_HISTORY, ACTION_CONNECT_WALLET
    ),
    "missing_cost_basis": _info(
        CATEGORY_MISSING_ACQUISITION_HISTORY, ACTION_IMPORT_HISTORY, ACTION_SET_PRICE
    ),
    # The evidence is present but has more than one valid reading.
    "ownership_transfer_source_ambiguous": _info(
        CATEGORY_NEEDS_DECISION, ACTION_PAIR_TRANSFER
    ),
    "ownership_transfer_destination_ambiguous": _info(
        CATEGORY_NEEDS_DECISION, ACTION_PAIR_TRANSFER
    ),
    "ownership_transfer_ambiguous_output": _info(CATEGORY_NEEDS_DECISION),
    "ownership_transfer_duplicate_outbound": _info(CATEGORY_NEEDS_DECISION),
    "owned_fanout_unresolved": _info(
        CATEGORY_NEEDS_DECISION, ACTION_PAIR_TRANSFER, ACTION_CONNECT_WALLET
    ),
    "unscoped_transfer_review": _info(CATEGORY_NEEDS_DECISION, ACTION_PAIR_TRANSFER),
    "manual_multi_pair_ambiguous": _info(CATEGORY_NEEDS_DECISION, ACTION_PAIR_TRANSFER),
    "transfer_mismatch": _info(CATEGORY_NEEDS_DECISION, ACTION_PAIR_TRANSFER),
    "transfer_pair_chronology_mismatch": _info(
        CATEGORY_NEEDS_DECISION, ACTION_PAIR_TRANSFER
    ),
    "transfer_network_mismatch": _info(CATEGORY_NEEDS_DECISION, ACTION_PAIR_TRANSFER),
    "transfer_fee_implausible": _info(
        CATEGORY_NEEDS_DECISION, ACTION_PAIR_TRANSFER, ACTION_CONNECT_WALLET
    ),
    "privacy_hop_unresolved": _info(
        CATEGORY_NEEDS_DECISION, ACTION_CONNECT_WALLET, ACTION_ATTACH_EVIDENCE
    ),
    "native_transition_ambiguous": _info(CATEGORY_NEEDS_DECISION, ACTION_PAIR_TRANSFER),
    "native_transition_amount_mismatch": _info(
        CATEGORY_NEEDS_DECISION, ACTION_ATTACH_EVIDENCE
    ),
    "custody_authored_migration_incomplete": _info(CATEGORY_NEEDS_DECISION),
    "custody_component_blocked": _info(CATEGORY_NEEDS_DECISION),
    "custody_interpreter_blocked": _info(CATEGORY_NEEDS_DECISION),
    "unclassified_income_kind": _info(CATEGORY_NEEDS_DECISION, ACTION_CLASSIFY),
    "non_sale_disposal_kind": _info(CATEGORY_NEEDS_DECISION, ACTION_CLASSIFY),
    # Kassiber cannot book this shape yet; no user input makes it automatic.
    "native_transition_fee_timing_unresolved": _info(CATEGORY_UNSUPPORTED),
    "acquisition_valuation_unsupported": _info(CATEGORY_UNSUPPORTED, ACTION_CLASSIFY),
    # Consequences of another transaction's unresolved state.
    "custody_basis_barrier": _info(CATEGORY_DOWNSTREAM, ACTION_RESOLVE_ROOT),
    "transfer_pair_dependency_blocked": _info(CATEGORY_DOWNSTREAM, ACTION_RESOLVE_ROOT),
    "basis_provenance_incomplete": _info(CATEGORY_DOWNSTREAM, ACTION_RESOLVE_ROOT),
    "derived_transfer_group_blocked": _info(CATEGORY_DOWNSTREAM, ACTION_RESOLVE_ROOT),
    "at_swap_basis_carry_unresolved": _info(CATEGORY_DOWNSTREAM, ACTION_RESOLVE_ROOT),
    "bitcoin_rail_carry_basis_unresolved": _info(
        CATEGORY_DOWNSTREAM, ACTION_RESOLVE_ROOT
    ),
}

# ``custody_quantity_unresolved`` is an umbrella; its blocker code names the
# actual custody problem.
_QUANTITY_BLOCKERS: dict[str, ReasonInfo] = {
    "custody_gap_review_required": _info(
        CATEGORY_MISSING_WALLET_HISTORY, ACTION_CONNECT_WALLET, ACTION_REVIEW_CUSTODY_GAP
    ),
    "implicit_wallet_delta_unallocated": _info(
        CATEGORY_MISSING_WALLET_HISTORY, ACTION_SYNC_WALLET, ACTION_IMPORT_HISTORY
    ),
    "search_capacity_incomplete": _info(
        CATEGORY_NEEDS_DECISION, ACTION_REVIEW_CUSTODY_GAP
    ),
    "capacity_source_suspense_required": _info(
        CATEGORY_NEEDS_DECISION, ACTION_REVIEW_CUSTODY_GAP
    ),
    "source_overlap_quantity_unresolved": _info(CATEGORY_NEEDS_DECISION),
    # A reviewed route moved less than it took: the difference is held in
    # suspense. Most often a pair joined two transactions that are not one
    # movement, so the pair itself is what to look at first.
    "reviewed_residual_suspense": _info(
        CATEGORY_NEEDS_DECISION, ACTION_REVIEW_PAIR, ACTION_REVIEW_CUSTODY_GAP
    ),
    "unclaimed_source_residual": _info(
        CATEGORY_NEEDS_DECISION, ACTION_CONNECT_WALLET, ACTION_ATTACH_EVIDENCE
    ),
}
_QUANTITY_DEFAULT = _info(CATEGORY_NEEDS_DECISION)
_UNKNOWN = _info(CATEGORY_NEEDS_DECISION)

# The order in which a quantity blocker explains a transaction when several
# issues name it: a missing wallet first, then evidence Kassiber could re-read,
# then reviewed-component problems.
_QUANTITY_BLOCKER_PRIORITY = (
    "custody_gap_review_required",
    "implicit_wallet_delta_unallocated",
    "unclaimed_source_residual",
    "source_overlap_quantity_unresolved",
    "search_capacity_incomplete",
    "capacity_source_suspense_required",
)


def reason_info(reason: str, detail: Mapping[str, Any] | None = None) -> ReasonInfo:
    """Classify one stored reason, resolving umbrella quantity blockers."""

    if reason == "custody_quantity_unresolved":
        blocker = str((detail or {}).get("blocker_code") or "")
        if blocker in _QUANTITY_BLOCKERS:
            return _QUANTITY_BLOCKERS[blocker]
        return _QUANTITY_DEFAULT
    return _REASONS.get(reason, _UNKNOWN)


def is_downstream(reason: str) -> bool:
    return reason_info(reason).downstream


#: Every stored reason that only follows another transaction's problem.
DOWNSTREAM_REASONS: tuple[str, ...] = tuple(
    sorted(reason for reason, info in _REASONS.items() if info.downstream)
)


def quantity_blocker_rank(blocker_code: str) -> tuple[int, str]:
    """Deterministic precedence for several quantity blockers on one row."""

    try:
        return (_QUANTITY_BLOCKER_PRIORITY.index(blocker_code), blocker_code)
    except ValueError:
        if blocker_code.startswith("custody_component_"):
            return (len(_QUANTITY_BLOCKER_PRIORITY), blocker_code)
        return (len(_QUANTITY_BLOCKER_PRIORITY) + 1, blocker_code)


def primary_reasons(quarantines: Sequence[Mapping[str, Any]]) -> dict[str, str]:
    """Return the reason that explains each transaction, as stored.

    Uses the same precedence as :func:`kassiber.core.tax_events.dedupe_quarantines`:
    the first root cause in emission order, or the first downstream reason when
    a transaction has no root cause of its own.
    """

    primary: dict[str, str] = {}
    for quarantine in quarantines:
        transaction_id = str(quarantine["transaction_id"])
        reason = str(quarantine["reason"])
        current = primary.get(transaction_id)
        if current is None or (is_downstream(current) and not is_downstream(reason)):
            primary[transaction_id] = reason
    return primary


__all__ = [
    "ACTION_ATTACH_EVIDENCE",
    "ACTION_CLASSIFY",
    "ACTION_CONNECT_WALLET",
    "ACTION_IMPORT_HISTORY",
    "ACTION_PAIR_TRANSFER",
    "ACTION_PROCESS_JOURNALS",
    "ACTION_RESOLVE_ROOT",
    "ACTION_REVIEW_CUSTODY_GAP",
    "ACTION_REVIEW_PAIR",
    "ACTION_SET_PRICE",
    "ACTION_SYNC_WALLET",
    "ACTION_WAIT_FOR_CONFIRMATION",
    "CATEGORIES",
    "CATEGORY_DOWNSTREAM",
    "CATEGORY_MISSING_ACQUISITION_HISTORY",
    "CATEGORY_MISSING_CHAIN_EVIDENCE",
    "CATEGORY_MISSING_PRICE",
    "CATEGORY_MISSING_WALLET_HISTORY",
    "CATEGORY_NEEDS_DECISION",
    "CATEGORY_UNSUPPORTED",
    "DOWNSTREAM_REASONS",
    "ReasonInfo",
    "is_downstream",
    "primary_reasons",
    "quantity_blocker_rank",
    "reason_info",
]
