"""Cross-asset / cross-wallet swap-candidate matcher.

Sits between the raw ``transactions`` table and the review surfaces (CLI,
daemon, UI). Given a profile's unpaired transactions plus the existing
pair / dismissal records, it returns the candidate pairings the matcher
believes form one swap.

Confidence ladder
-----------------

* **exact** — both legs share deterministic evidence: a source-qualified
  Lightning ``payment_hash``; a unique provider/client ``swap_id`` whose
  canonical route txids and whole-row amounts agree; or a uniquely verified
  on-chain HTLC refund outpoint.
* **strong** — different wallets, opposite directions, time delta within
  the configured window, and the implicit ``out_amount - in_amount``
  delta sits below the fee tolerance (``max(fee_pct_max * out, fee_sats_min)``).
  Surfaced via ``method = "heuristic"``.

Anything weaker stays unmatched; the user explicitly pairs from the row
"…" menu.

Conflict clustering
-------------------

Two candidates conflict when they share an out- or in-leg. The matcher
builds a union-find over all surviving candidates and stamps a stable
``conflict_set_id`` on each: any cluster larger than one element is a
disambiguation the user has to resolve before pairing. Exact-confidence
candidates dominate any heuristic candidate sharing a leg — the
heuristic siblings drop out of the cluster.

Pure functions
--------------

No SQLite, no env, no logging. Callers feed in already-fetched rows
(typically enriched with ``wallet_label``, ``wallet_kind``) plus the
existing pair / dismissal records and receive a list of frozen
``SwapCandidate`` dataclasses. Two side-tables flow alongside:

* ``conflict_set_id`` / ``conflict_size`` so review surfaces can render
  clusters. The size is stamped at match time over the FULL candidate
  set — downstream filters (confidence, asset/route pair, swap-vs-
  transfer tabs) must never recompute it from a filtered list, or a
  cluster split across filters looks falsely solo and bulk-pair would
  silently choose for the user.
* ``swap_fee_msat`` / ``swap_fee_kind`` computed once at match time so
  the review surface can show the "what actually left your custody"
  number without re-deriving it.
"""

from __future__ import annotations

import bisect
import json
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Iterable, Mapping, Optional, Sequence

from ..transfers import (
    CHAIN_INFERENCE_WALLET_KINDS,
    LIGHTNING_INFERENCE_WALLET_KINDS,
    bitcoin_network_domain,
    canonical_txid,
    is_bitcoin_rail_pair,
    is_lightning_payment_hash_row,
    normalize_group_txid,
    normalize_wallet_kind_alias,
    onchain_transfer_scope,
)
from .htlc_parser import (
    extract_from_claim_witness,
    refund_funding_outpoint_from_tx_mapping,
)
from .onchain import exact_onchain_fee_msat_from_observations


# Compatibility exports for callers/UI routing. The canonical sets live next to
# the journal's wallet-kind normalization in ``kassiber.transfers``.
LIGHTNING_WALLET_KINDS = LIGHTNING_INFERENCE_WALLET_KINDS
CHAIN_WALLET_KINDS = CHAIN_INFERENCE_WALLET_KINDS

DEFAULT_TIME_WINDOW_SECONDS = 24 * 60 * 60  # 24h
DEFAULT_FEE_PCT_MAX = 0.01  # 1%
DEFAULT_FEE_SATS_MIN = 2500  # absolute floor for small swaps
SATS_TO_MSAT = 1000

METHOD_PAYMENT_HASH = "payment_hash"
METHOD_HEURISTIC = "heuristic"
# Deterministic link: an inbound refund's input spent the outbound's HTLC
# funding output (recorded as transactions.swap_refund_funding_txid by sync).
METHOD_HTLC_REFUND = "htlc_refund"
# Deterministic imported/client evidence for cooperative Taproot v2 and other
# provider-backed swaps where the chain spend itself is not identifying.
METHOD_PROVIDER_SWAP_ID = "provider_swap_id"
# Exact owned-output evidence surfaced from a journal ownership block. These
# candidates require explicit user review and are never rule/bulk auto-paired.
METHOD_OWNERSHIP_GRAPH = "ownership_graph"

CONFIDENCE_EXACT = "exact"
CONFIDENCE_STRONG = "strong"

KIND_SUBMARINE_SWAP = "submarine-swap"
KIND_REVERSE_SUBMARINE_SWAP = "reverse-submarine-swap"
KIND_CHAIN_SWAP = "chain-swap"
KIND_PEG_IN = "peg-in"
KIND_PEG_OUT = "peg-out"
KIND_SWAP_REFUND = "swap-refund"
KIND_MANUAL = "manual"

POLICY_CARRYING_VALUE = "carrying-value"
POLICY_TAXABLE = "taxable"


@dataclass(frozen=True)
class SwapCandidate:
    """One candidate pairing of an outbound leg with an inbound leg.

    Two legs in opposite directions across different wallets, judged a
    swap by deterministic evidence such as ``payment_hash``,
    ``provider_swap_id``, an HTLC refund funding link, or a journal-surfaced
    ownership proof, or by the time + amount heuristic. Fee, default kind,
    default policy, and conflict cluster are all computed once at match time so
    the review surface can render without re-deriving them.
    """

    out_id: str
    in_id: str
    out_asset: str
    in_asset: str
    out_amount_msat: int
    in_amount_msat: int
    out_wallet_id: str
    in_wallet_id: str
    out_wallet_label: str
    in_wallet_label: str
    out_wallet_kind: str
    in_wallet_kind: str
    out_occurred_at: str
    in_occurred_at: str
    confidence: str
    method: str
    swap_fee_msat: int
    swap_fee_kind: str
    default_kind: str
    default_policy: str
    conflict_set_id: str = ""
    # Cluster cardinality over the full (unfiltered) candidate set.
    # ``> 1`` means this candidate needs manual disambiguation even when
    # its cluster siblings are hidden by a downstream filter.
    conflict_size: int = 1
    evidence_provider: str = ""
    evidence_id: str = ""
    evidence_kind: str = ""
    evidence_status: str = ""
    evidence_version: str = ""
    evidence_taproot: str = ""
    evidence_cooperative: str = ""
    evidence_spend_path: str = ""


@dataclass(frozen=True)
class SwapFeeComponents:
    """Loss-of-custody components available from two normalized swap legs.

    The legacy pair table has one ``swap_fee_msat`` column, so callers still
    receive a combined number through :func:`compute_swap_fee`.  This shape is
    the future-proof seam for component accounting: an explicitly recorded
    source fee is kept separate from the unexplained principal delta instead
    of incorrectly calling the whole difference a provider fee.

    ``bridge_delta_msat`` can include a provider fee, a destination claim fee,
    rounding, or missing evidence.  It deliberately remains unallocated until
    a rail adapter supplies those facts.
    """

    source_fee_msat: int
    source_fee_kind: str
    bridge_delta_msat: int
    bridge_delta_kind: str
    total_msat: int


def suggest_swap_candidates(
    rows: Sequence[Mapping],
    *,
    pair_records: Iterable[Mapping] = (),
    dismissals: Iterable[Mapping] = (),
    time_window_seconds: int = DEFAULT_TIME_WINDOW_SECONDS,
    fee_pct_max: float = DEFAULT_FEE_PCT_MAX,
    fee_sats_min: int = DEFAULT_FEE_SATS_MIN,
    now_iso: Optional[str] = None,
) -> list[SwapCandidate]:
    """Return the swap candidates the matcher believes form valid pairings.

    Args:
        rows: Iterable of transaction rows. Each row must expose the
            fields ``id``, ``profile_id``, ``wallet_id``, ``wallet_label``,
            ``wallet_kind``, ``payment_hash``, ``external_id``, ``occurred_at``,
            ``direction``, ``asset``, ``amount``, ``excluded``. Rows with
            ``excluded`` truthy are ignored. Rows touched by an active
            pair record are ignored.
        pair_records: Iterable of pair rows with ``out_transaction_id``,
            ``in_transaction_id``, ``deleted_at``. Only active pairs
            (``deleted_at IS NULL``) suppress candidates.
        dismissals: Iterable of dismissal rows with
            ``out_transaction_id``, ``in_transaction_id``, ``expires_at``.
            A dismissal that has not yet expired (relative to ``now_iso``)
            suppresses that exact pair.
        time_window_seconds: Maximum seconds between out and in
            ``occurred_at`` for the heuristic to consider them.
        fee_pct_max: Maximum fractional fee tolerance for the heuristic.
        fee_sats_min: Absolute minimum fee tolerance in sats, applied
            even when ``fee_pct_max * out_amount`` falls below it.
        now_iso: Override the "current time" used to evaluate dismissal
            expiry. Defaults to ``datetime.now(UTC)`` when omitted.

    Returns:
        Sorted list of :class:`SwapCandidate` (exact first, then
        heuristic; within each method by smaller fee delta then by
        ``out_occurred_at``).
    """
    now_seconds = _seconds_or_now(now_iso)
    paired_ids = _active_paired_ids(pair_records)
    dismissed_pairs = _active_dismissals(dismissals, now_seconds)

    # Evidence cardinality is a property of the full imported population, not
    # the currently-unpaired remainder.  Otherwise pairing one member of a
    # duplicate 2x2 hash/provider set would falsely promote the remaining 1x1
    # edge to exact on the next run.
    population_rows = _select_eligible_rows(rows, set())
    eligible_rows = _select_eligible_rows(rows, paired_ids)
    # Deterministic suppression mirrors the journal's fixed safety ceiling.
    # Caller flags widen only heuristic generation; they must never hide a row
    # that the journal still quarantines with the default ceiling.
    deterministic_transfer_ids = _deterministic_self_transfer_ids(population_rows)
    out_rows = [
        row
        for row in eligible_rows
        if row["direction"] == "outbound"
        and _record_get(row, "id") not in deterministic_transfer_ids
    ]
    in_rows = [
        row
        for row in eligible_rows
        if row["direction"] == "inbound"
        and _record_get(row, "id") not in deterministic_transfer_ids
    ]

    population_out_rows = [
        row
        for row in population_rows
        if row["direction"] == "outbound"
        and _record_get(row, "id") not in deterministic_transfer_ids
    ]
    population_in_rows = [
        row
        for row in population_rows
        if row["direction"] == "inbound"
        and _record_get(row, "id") not in deterministic_transfer_ids
    ]
    eligible_ids = {_record_get(row, "id") for row in eligible_rows}

    hash_pairs = [
        pair
        for pair in _match_by_payment_hash(population_out_rows, population_in_rows)
        if pair[0]["id"] in eligible_ids and pair[1]["id"] in eligible_ids
        if (pair[0]["id"], pair[1]["id"]) not in dismissed_pairs
    ]

    # Build every non-heuristic evidence edge before reserving anything. A
    # competing provider/hash/refund edge is independent contradictory evidence
    # and must remain in the global conflict cluster even when another method is
    # exact. Only pure time/amount heuristics may be pruned by exact evidence.
    evidence_pairs = [
        pair
        for pair in _match_by_provider_swap_id(
            population_out_rows, population_in_rows
        )
        if pair[0]["id"] in eligible_ids and pair[1]["id"] in eligible_ids
        if (pair[0]["id"], pair[1]["id"]) not in dismissed_pairs
    ]
    refund_pairs = [
        pair
        for pair in _match_by_refund_link(
            population_out_rows, population_in_rows
        )
        if pair[0]["id"] in eligible_ids and pair[1]["id"] in eligible_ids
        if (pair[0]["id"], pair[1]["id"]) not in dismissed_pairs
    ]
    exact_out_ids = {pair[0]["id"] for pair in hash_pairs if pair[2]}
    exact_in_ids = {pair[1]["id"] for pair in hash_pairs if pair[2]}
    exact_out_ids |= {pair[0]["id"] for pair in evidence_pairs if pair[3]}
    exact_in_ids |= {pair[1]["id"] for pair in evidence_pairs if pair[3]}
    exact_out_ids |= {
        pair[0]["id"]
        for pair in refund_pairs
        if pair[2].version == "outpoint_exact"
    }
    exact_in_ids |= {
        pair[1]["id"]
        for pair in refund_pairs
        if pair[2].version == "outpoint_exact"
    }
    heuristic_pairs = _match_heuristic(
        [row for row in out_rows if row["id"] not in exact_out_ids],
        [row for row in in_rows if row["id"] not in exact_in_ids],
        time_window_seconds=time_window_seconds,
        fee_pct_max=fee_pct_max,
        fee_sats_min=fee_sats_min,
    )

    raw_candidates: list[SwapCandidate] = []
    for out_row, in_row, whole_row_exact in hash_pairs:
        if (out_row["id"], in_row["id"]) in dismissed_pairs:
            continue
        raw_candidates.append(_build_candidate(
            out_row,
            in_row,
            confidence=CONFIDENCE_EXACT if whole_row_exact else CONFIDENCE_STRONG,
            method=METHOD_PAYMENT_HASH,
        ))
    for out_row, in_row, evidence, whole_row_exact in evidence_pairs:
        if (out_row["id"], in_row["id"]) in dismissed_pairs:
            continue
        raw_candidates.append(_build_candidate(
            out_row,
            in_row,
            confidence=CONFIDENCE_EXACT if whole_row_exact else CONFIDENCE_STRONG,
            method=METHOD_PROVIDER_SWAP_ID,
            default_kind=evidence.kind or None,
            evidence=evidence,
        ))
    for out_row, in_row, evidence in refund_pairs:
        if (out_row["id"], in_row["id"]) in dismissed_pairs:
            continue
        refund_exact = evidence.version == "outpoint_exact"
        raw_candidates.append(_build_candidate(
            out_row,
            in_row,
            confidence=(
                CONFIDENCE_EXACT
                if refund_exact
                else CONFIDENCE_STRONG
            ),
            method=METHOD_HTLC_REFUND,
            default_kind=KIND_SWAP_REFUND,
            evidence=evidence,
        ))
    for out_row, in_row in heuristic_pairs:
        if (out_row["id"], in_row["id"]) in dismissed_pairs:
            continue
        raw_candidates.append(_build_candidate(
            out_row,
            in_row,
            confidence=CONFIDENCE_STRONG,
            method=METHOD_HEURISTIC,
        ))

    candidates = finalize_candidate_conflicts(raw_candidates)
    candidates.sort(
        key=lambda c: (
            0 if c.confidence == CONFIDENCE_EXACT else 1,
            abs(c.swap_fee_msat),
            c.out_occurred_at,
            c.out_id,
            c.in_id,
        )
    )
    return candidates


def compute_swap_fee(
    out_amount_msat: int,
    in_amount_msat: int,
    out_fee_msat: int = 0,
) -> tuple[int, str]:
    """Return ``(swap_fee_msat, swap_fee_kind)``.

    Signed delta — positive when the principal plus outbound network fee
    shrank across the swap (the common case), negative when the inbound
    exceeds that total (anomaly, useful for "do not auto-pair" guards).
    The kind defaults to ``"combined"``; future commits can split network
    vs service fee when the data supports it.
    """
    components = compute_swap_fee_components(
        out_amount_msat,
        in_amount_msat,
        out_fee_msat,
    )
    return components.total_msat, "combined"


def compute_swap_fee_components(
    out_amount_msat: int,
    in_amount_msat: int,
    out_fee_msat: int = 0,
    *,
    source_fee_kind: str = "source_network_or_routing",
) -> SwapFeeComponents:
    """Return the fee facts that can be proven without guessing allocation.

    The source transaction's explicit fee and the cross-rail principal delta
    are independent facts.  Keeping them separate prevents later component
    storage from double-counting the source miner/routing fee as a provider
    service fee.  Negative deltas are preserved as anomalies for review.
    """

    source_fee = max(0, int(out_fee_msat or 0))
    bridge_delta = int(out_amount_msat or 0) - int(in_amount_msat or 0)
    return SwapFeeComponents(
        source_fee_msat=source_fee,
        source_fee_kind=str(source_fee_kind or "source_network_or_routing"),
        bridge_delta_msat=bridge_delta,
        bridge_delta_kind="unallocated_bridge_delta",
        total_msat=source_fee + bridge_delta,
    )


def default_kind_for(
    out_asset: str,
    in_asset: str,
    out_wallet_kind: str,
    in_wallet_kind: str,
) -> str:
    """Infer the swap kind from leg shapes.

    Heavy-user defaults:

    * Chain → Lightning → ``submarine-swap``.
    * Lightning → chain → ``reverse-submarine-swap``.
    * Both legs are chain wallets:
      * BTC → LBTC → ``peg-in``.
      * LBTC → BTC → ``peg-out``.
    * Everything else → ``manual`` (the user picks).
    """
    out_kind = normalize_wallet_kind_alias(out_wallet_kind)
    in_kind = normalize_wallet_kind_alias(in_wallet_kind)
    out_is_lightning = out_kind in LIGHTNING_WALLET_KINDS
    in_is_lightning = in_kind in LIGHTNING_WALLET_KINDS
    if out_is_lightning and not in_is_lightning:
        return KIND_REVERSE_SUBMARINE_SWAP
    if in_is_lightning and not out_is_lightning:
        return KIND_SUBMARINE_SWAP
    if out_kind in CHAIN_WALLET_KINDS and in_kind in CHAIN_WALLET_KINDS:
        if out_asset == "BTC" and in_asset == "LBTC":
            return KIND_PEG_IN
        if out_asset == "LBTC" and in_asset == "BTC":
            return KIND_PEG_OUT
    return KIND_MANUAL


def default_ownership_policy_for(
    out_asset: Optional[str] = None,
    in_asset: Optional[str] = None,
) -> str:
    """Return a country-neutral ownership/rail policy recommendation.

    Matching evidence is Bitcoin technology, not tax law. Same-asset moves and
    enabled BTC/LBTC rail changes represent the same technical exposure;
    everything else defaults to taxable until a downstream tax-policy adapter
    recommends otherwise. No country is accepted at this boundary.
    """
    if is_bitcoin_rail_pair(out_asset, in_asset):
        return POLICY_CARRYING_VALUE
    return POLICY_TAXABLE


def fee_threshold_msat(out_amount_msat: int, fee_pct_max: float, fee_sats_min: int) -> int:
    """``max(fee_pct_max * out, fee_sats_min)`` expressed in msat.

    The percentage governs large swaps; the absolute floor catches small
    swaps where fixed service / network fees dwarf any percentage band.
    """
    pct_floor = int(abs(out_amount_msat) * fee_pct_max)
    abs_floor = int(fee_sats_min) * SATS_TO_MSAT
    return max(pct_floor, abs_floor)


# -- internals --------------------------------------------------------------


def _seconds_or_now(now_iso: Optional[str]) -> float:
    if now_iso is None:
        return datetime.now(timezone.utc).timestamp()
    return _iso_to_seconds(now_iso)


def _iso_to_seconds(value: Optional[str]) -> Optional[float]:
    if not value:
        return None
    raw = str(value).strip()
    if not raw:
        return None
    if raw.endswith("Z"):
        raw = raw[:-1] + "+00:00"
    try:
        return datetime.fromisoformat(raw).timestamp()
    except ValueError:
        return None


def _active_paired_ids(pair_records: Iterable[Mapping]) -> set[str]:
    paired: set[str] = set()
    for record in pair_records:
        if _record_get(record, "deleted_at"):
            continue
        out_id = _record_get(record, "out_transaction_id")
        in_id = _record_get(record, "in_transaction_id")
        if out_id:
            paired.add(out_id)
        if in_id:
            paired.add(in_id)
    return paired


def _active_dismissals(
    dismissals: Iterable[Mapping], now_seconds: float
) -> set[tuple[str, str]]:
    active: set[tuple[str, str]] = set()
    for record in dismissals:
        expires_at = _record_get(record, "expires_at")
        if expires_at:
            expires_seconds = _iso_to_seconds(expires_at)
            if expires_seconds is not None and expires_seconds <= now_seconds:
                continue
        out_id = _record_get(record, "out_transaction_id")
        in_id = _record_get(record, "in_transaction_id")
        if out_id and in_id:
            active.add((out_id, in_id))
    return active


def _select_eligible_rows(rows: Sequence[Mapping], paired_ids: set[str]) -> list[Mapping]:
    eligible: list[Mapping] = []
    for row in rows:
        if _record_get(row, "excluded"):
            continue
        if _record_get(row, "id") in paired_ids:
            continue
        if _record_get(row, "direction") not in ("outbound", "inbound"):
            continue
        eligible.append(row)
    return eligible


def _deterministic_self_transfer_ids(rows: Sequence[Mapping]) -> set[object]:
    """Return row ids that are already proven same-chain self-transfers.

    The swap review queue is for ambiguous layer hops. One outbound and one or
    more inbound rows in one canonical chain/network/txid/asset scope, across
    owned wallets, are the conservative on-chain self-transfer shapes
    used by the journal pipeline. The conserving 1->N shape is suppressed as one
    group so its largest leg cannot leak back as a strong heuristic candidate.

    A non-fee-inclusive row reports its network/routing fee separately. Therefore
    any positive ``out_amount - in_amount`` is unallocated principal, not a fee;
    it stays visible for component review. Net-delta rows may suppress a gap only
    when a complete valued graph proves that exact network fee.
    """
    grouped: dict[tuple[str, str, str, str], list[Mapping]] = {}
    for row in rows:
        scope = onchain_transfer_scope(row)
        if scope is None:
            continue
        grouped.setdefault(scope, []).append(row)

    deterministic_ids: set[object] = set()
    for group in grouped.values():
        outs = [
            row
            for row in group
            if _record_get(row, "direction") == "outbound"
            and int(_record_get(row, "amount") or 0) > 0
        ]
        ins = [
            row
            for row in group
            if _record_get(row, "direction") == "inbound"
            and int(_record_get(row, "amount") or 0) > 0
        ]
        if len(outs) != 1 or not ins:
            continue
        out_row = outs[0]
        out_wallet_id = _record_get(out_row, "wallet_id")
        if any(out_wallet_id == _record_get(in_row, "wallet_id") for in_row in ins):
            continue
        out_amount = int(_record_get(out_row, "amount") or 0)
        in_amount = sum(int(_record_get(in_row, "amount") or 0) for in_row in ins)
        if in_amount > out_amount:
            continue
        gap_msat = out_amount - in_amount
        if _record_get(out_row, "amount_includes_fee") and gap_msat > 0:
            exact_fee_msat = exact_onchain_fee_msat_from_observations(
                [_record_get(row, "raw_json") for row in group],
                asset=str(_record_get(out_row, "asset") or ""),
            )
            if exact_fee_msat != gap_msat:
                # A net wallet delta can include a recipient/payment as well as
                # the miner fee.  Without a complete valued graph, this is not a
                # deterministic MOVE and must remain visible for review.
                continue
        elif gap_msat > 0:
            # The explicit fee column already accounts for network/routing cost.
            # This remaining principal delta is an external payment or a missing
            # wallet leg until an authored custody component proves otherwise.
            continue
        deterministic_ids.add(_record_get(out_row, "id"))
        deterministic_ids.update(_record_get(in_row, "id") for in_row in ins)

    # Mirror of the journal's Lightning payment-hash pass
    # (transfers.detect_intra_transfers): an own-node payment whose hash
    # matches another owned node's invoice is netted as a MOVE by the journal,
    # so it must not surface as an exact payment_hash swap candidate. ONLY
    # node-sourced hashes qualify — a chain_script HTLC hash (reverse swap
    # claim) is swap evidence and stays reviewable.
    by_hash: dict[tuple[str, str, str], list[Mapping]] = {}
    for row in rows:
        if _record_get(row, "id") in deterministic_ids:
            continue
        payment_hash = _normalized_payment_hash(_record_get(row, "payment_hash"))
        if not payment_hash:
            continue
        if not is_lightning_payment_hash_row(row):
            continue
        network_domain = bitcoin_network_domain(row)
        if network_domain is None:
            continue
        by_hash.setdefault(
            (payment_hash, _record_get(row, "asset"), network_domain), []
        ).append(row)
    for group in by_hash.values():
        outs = [
            row
            for row in group
            if _record_get(row, "direction") == "outbound"
            and int(_record_get(row, "amount") or 0) > 0
        ]
        ins = [
            row
            for row in group
            if _record_get(row, "direction") == "inbound"
            and int(_record_get(row, "amount") or 0) > 0
        ]
        if len(outs) != 1 or len(ins) != 1:
            continue
        out_row, in_row = outs[0], ins[0]
        if int(_record_get(out_row, "amount") or 0) != int(
            _record_get(in_row, "amount") or 0
        ):
            continue
        deterministic_ids.add(_record_get(out_row, "id"))
        deterministic_ids.add(_record_get(in_row, "id"))
    return deterministic_ids


def _match_by_payment_hash(
    out_rows: Sequence[Mapping], in_rows: Sequence[Mapping]
) -> list[tuple[Mapping, Mapping, bool]]:
    """Return only uniquely attributable, conserving cross-rail hash pairs.

    A payment hash commits to a preimage, but an arbitrary imported string is
    not by itself proof of a custody transfer.  Exact matching therefore needs
    compatible rail roles (Lightning node versus chain/provider HTLC evidence),
    one outbound and one inbound leg for the hash, and a plausible 1:1 Bitcoin
    conservation delta.  Duplicate MPP/attempt/import rows remain unresolved
    instead of becoming a Cartesian product of exact candidates.
    """
    out_by_hash: dict[tuple[str, str | None], list[Mapping]] = {}
    for row in out_rows:
        payment_hash = _normalized_payment_hash(_record_get(row, "payment_hash"))
        if not payment_hash:
            continue
        out_by_hash.setdefault(
            (payment_hash, bitcoin_network_domain(row)), []
        ).append(row)
    in_by_hash: dict[tuple[str, str | None], list[Mapping]] = {}
    for row in in_rows:
        payment_hash = _normalized_payment_hash(_record_get(row, "payment_hash"))
        if not payment_hash:
            continue
        in_by_hash.setdefault(
            (payment_hash, bitcoin_network_domain(row)), []
        ).append(row)
    pairs: list[tuple[Mapping, Mapping, bool]] = []
    for (_payment_hash, network_domain), outs in out_by_hash.items():
        ins = in_by_hash.get((_payment_hash, network_domain), [])
        if len(outs) != 1 or len(ins) != 1:
            continue
        out_row, in_row = outs[0], ins[0]
        if _record_get(out_row, "wallet_id") == _record_get(in_row, "wallet_id"):
            continue
        confidence = _payment_hash_confidence(out_row, in_row)
        if confidence is None:
            continue
        out_domain = bitcoin_network_domain(out_row)
        in_domain = bitcoin_network_domain(in_row)
        if out_domain is not None and in_domain is not None and out_domain != in_domain:
            continue
        if confidence == CONFIDENCE_EXACT and (
            out_domain is None or in_domain is None
        ):
            continue
        if not _payment_hash_amounts_conserve(out_row, in_row):
            continue
        pairs.append((out_row, in_row, confidence == CONFIDENCE_EXACT))
    return pairs


_PAYMENT_HASH_HEX = frozenset("0123456789abcdefABCDEF")


def _normalized_payment_hash(value: object) -> str:
    text = str(value or "").strip()
    if len(text) != 64 or any(char not in _PAYMENT_HASH_HEX for char in text):
        return ""
    return text.lower()


def _payment_hash_role(row: Mapping) -> str:
    """Classify the provenance role of a hash-bearing transaction leg."""
    if is_lightning_payment_hash_row(row):
        return "lightning_node"
    source = str(_record_get(row, "payment_hash_source") or "").strip().lower()
    if source == "chain_script_unique_outpoint":
        return (
            "chain_htlc"
            if _row_has_verified_unique_claim_outpoint(row)
            else "chain_htlc_unscoped"
        )
    if source == "chain_script":
        # Legacy enrichment selected the first matching witness in a batched
        # claim and did not retain a unique funding outpoint. Keep it visible to
        # the heuristic/manual queue, but never upgrade the aggregate row to an
        # exact payment-hash link.
        return "chain_htlc_unscoped"
    # Stable importer labels say where the field came from, not that its value
    # is authenticated. They can support a strong review suggestion, never
    # exact/bulk closure by themselves. Unknown user-authored labels do not gain
    # semantics merely by being nonempty.
    if source in {
        "boltz",
        "boltz-regtest",
        "bullbitcoin",
        "generic_ledger",
        "importer",
        "importer_backfill",
    }:
        return "provider_or_import"
    return "unknown"


def _row_has_verified_unique_claim_outpoint(row: Mapping) -> bool:
    """Replay the witness/outpoint proof behind an exact claim hash label."""

    payload = _raw_json_payload(row)
    if not isinstance(payload, Mapping):
        return False
    effective = payload.get("tx")
    effective = effective if isinstance(effective, Mapping) else payload
    vins = effective.get("vin")
    if not isinstance(vins, list) or len(vins) != 1:
        return False
    vin = vins[0]
    if not isinstance(vin, Mapping):
        return False
    if canonical_txid(vin.get("txid")) is None:
        return False
    raw_vout = vin.get("vout")
    if type(raw_vout) is not int or raw_vout < 0:
        return False
    raw_witness = vin.get("witness", vin.get("txinwitness"))
    if not isinstance(raw_witness, list) or not raw_witness:
        return False
    witness_items = []
    for item in raw_witness:
        if isinstance(item, str):
            try:
                witness_items.append(bytes.fromhex(item))
            except ValueError:
                return False
        elif isinstance(item, (bytes, bytearray)):
            witness_items.append(bytes(item))
        else:
            return False
    extraction = extract_from_claim_witness(witness_items)
    if extraction is None or not extraction.payment_hash:
        return False
    if extraction.payment_hash != _normalized_payment_hash(
        _record_get(row, "payment_hash")
    ):
        return False
    return onchain_transfer_scope(row) is not None


def _payment_hash_confidence(
    out_row: Mapping, in_row: Mapping
) -> str | None:
    roles = {_payment_hash_role(out_row), _payment_hash_role(in_row)}
    if roles & {"unknown", "chain_htlc_unscoped"}:
        return None
    if roles == {"lightning_node", "chain_htlc"}:
        return CONFIDENCE_EXACT
    if "lightning_node" in roles and "provider_or_import" in roles:
        return CONFIDENCE_STRONG
    if roles == {"provider_or_import"}:
        return CONFIDENCE_STRONG
    # A script-derived claim/refund can be linked to an explicitly imported
    # funding leg even when neither wallet is itself a Lightning node.
    if roles == {"chain_htlc", "provider_or_import"}:
        return CONFIDENCE_STRONG
    return None


def _payment_hash_amounts_conserve(out_row: Mapping, in_row: Mapping) -> bool:
    out_amount = int(_record_get(out_row, "amount") or 0)
    in_amount = int(_record_get(in_row, "amount") or 0)
    if out_amount <= 0 or in_amount <= 0 or in_amount > out_amount:
        return False
    return out_amount - in_amount <= fee_threshold_msat(
        out_amount,
        DEFAULT_FEE_PCT_MAX,
        DEFAULT_FEE_SATS_MIN,
    )


def _match_by_refund_link(
    out_rows: Sequence[Mapping], in_rows: Sequence[Mapping]
) -> list[tuple[Mapping, Mapping, "_ProviderSwapEvidence"]]:
    """Pair an inbound HTLC refund with the outbound funding leg it spent.

    Chain sync stamps ``swap_refund_funding_txid`` on an inbound row when its
    input spends a swap HTLC via the CLTV timeout branch; that txid is the
    funding (lockup) transaction's id, so it equals the outbound funding leg's
    ``external_id``. The link is deterministic — the refund provably spends the
    lockup output — so this path intentionally does NOT skip same-wallet pairs
    (a failed swap normally refunds to the funding wallet) and ignores the time
    window (a CLTV refund routinely lands well past the 24h heuristic window).
    Same-asset only: a refund returns the asset that was locked up.
    """
    # txids are hex, so match case-insensitively: sync lowercases
    # ``swap_refund_funding_txid`` but the lockup's ``external_id`` is stored
    # verbatim, so normalizing both sides here keeps the join self-contained.
    out_by_external_id: dict[str, list[Mapping]] = {}
    for row in out_rows:
        external_id = _record_get(row, "external_id")
        if external_id:
            out_by_external_id.setdefault(str(external_id).lower(), []).append(row)
    pairs: list[tuple[Mapping, Mapping, _ProviderSwapEvidence]] = []
    for in_row in in_rows:
        funding_txid, funding_vout, outpoint_proven = _refund_funding_reference(
            in_row
        )
        if not funding_txid:
            continue
        in_asset = str(_record_get(in_row, "asset") or "").upper()
        funding_rows = out_by_external_id.get(str(funding_txid).lower(), [])
        # One transaction row cannot distinguish two separate HTLC lockups in
        # the same funding transaction.  Do not manufacture exact candidates
        # when the imported source has duplicate funding rows.
        if len(funding_rows) != 1:
            continue
        out_row = funding_rows[0]
        if str(_record_get(out_row, "asset") or "").upper() != in_asset:
            continue
        if not _payment_hash_amounts_conserve(out_row, in_row):
            continue
        evidence_version = "txid_legacy"
        if funding_vout is not None:
            funding_output_msat = _funding_output_amount_msat(out_row, funding_vout)
            if funding_output_msat is None:
                evidence_version = "outpoint_unverified"
            elif funding_output_msat != int(_record_get(out_row, "amount") or 0):
                # The row aggregates more than this HTLC output. A row pair would
                # move/dispose the whole transaction; only a custody component can
                # split the exact outpoint safely, so do not offer a pair action.
                continue
            elif (
                outpoint_proven
                and canonical_txid(funding_txid) is not None
                and (
                    (out_scope := onchain_transfer_scope(out_row)) is not None
                    and out_scope[2] == canonical_txid(funding_txid)
                )
                and (
                    (in_scope := onchain_transfer_scope(in_row)) is not None
                    and in_scope[:2] == out_scope[:2]
                )
            ):
                evidence_version = "outpoint_exact"
            else:
                evidence_version = "outpoint_unverified"
        evidence_id = (
            f"{funding_txid}:{funding_vout}"
            if funding_vout is not None
            else str(funding_txid)
        )
        pairs.append(
            (
                out_row,
                in_row,
                _ProviderSwapEvidence(
                    provider="chain_htlc",
                    swap_id=evidence_id,
                    kind=KIND_SWAP_REFUND,
                    flow="refund",
                    status="refunded",
                    version=evidence_version,
                    spend_path="timeout",
                ),
            )
        )
    return pairs


def _funding_output_amount_msat(row: Mapping, vout: int) -> int | None:
    raw = _record_get(row, "raw_json")
    try:
        payload = json.loads(raw) if isinstance(raw, str) else raw
    except (TypeError, ValueError, json.JSONDecodeError):
        return None
    if not isinstance(payload, Mapping):
        return None
    outputs = payload.get("vout")
    if not isinstance(outputs, list):
        return None
    for position, output in enumerate(outputs):
        if not isinstance(output, Mapping):
            continue
        raw_index = output.get("n", position)
        try:
            index = int(raw_index)
        except (TypeError, ValueError):
            continue
        if index != vout:
            continue
        raw_sats = output.get("value_sats", output.get("value"))
        try:
            sats = int(raw_sats)
        except (TypeError, ValueError):
            return None
        return sats * 1000 if sats >= 0 else None
    return None


def _refund_funding_reference(row: Mapping) -> tuple[str, int | None, bool]:
    """Return the exact refund funding reference when the row exposes it.

    Current databases persist the txid.  Newer import/sync paths can add the
    backward-compatible ``swap_refund_funding_vout`` (or a combined outpoint)
    without changing this matcher again.  The vout is returned for component
    evidence even though the legacy transaction-pair schema cannot persist it.
    """
    def recovered_from_raw() -> tuple[tuple[str, int] | None, bool]:
        raw = _record_get(row, "raw_json")
        try:
            payload = json.loads(raw) if isinstance(raw, str) else raw
        except (TypeError, ValueError, json.JSONDecodeError):
            return None, False
        if not isinstance(payload, Mapping):
            return None, False
        effective = payload.get("tx")
        effective = effective if isinstance(effective, Mapping) else payload
        vin = effective.get("vin")
        return (
            refund_funding_outpoint_from_tx_mapping(payload),
            isinstance(vin, list) and len(vin) == 1,
        )

    recovered, recovered_covers_whole_row = recovered_from_raw()
    outpoint = _record_get(row, "swap_refund_funding_outpoint")
    if outpoint:
        txid, separator, raw_vout = str(outpoint).partition(":")
        if txid:
            try:
                vout = int(raw_vout) if separator else None
                proof = (
                    vout is not None
                    and recovered is not None
                    and recovered_covers_whole_row
                    and canonical_txid(txid) == recovered[0]
                    and vout == recovered[1]
                )
                return txid, vout, proof
            except (TypeError, ValueError):
                return txid, None, False

    txid = str(_record_get(row, "swap_refund_funding_txid") or "").strip()
    if not txid:
        if recovered is not None:
            return recovered[0], recovered[1], recovered_covers_whole_row
        return "", None, False
    raw_vout = _record_get(row, "swap_refund_funding_vout")
    try:
        if raw_vout not in (None, ""):
            vout = int(raw_vout)
            proof = (
                recovered is not None
                and recovered_covers_whole_row
                and canonical_txid(txid) == recovered[0]
                and vout == recovered[1]
            )
            return txid, vout, proof
    except (TypeError, ValueError):
        pass
    if recovered is not None and normalize_group_txid(recovered[0]) == normalize_group_txid(txid):
        return (
            recovered[0],
            recovered[1],
            recovered_covers_whole_row and canonical_txid(txid) == recovered[0],
        )
    return txid, None, False


@dataclass(frozen=True)
class _ProviderSwapEvidence:
    provider: str
    swap_id: str
    kind: str
    flow: str
    status: str = ""
    send_txid: str = ""
    receive_txid: str = ""
    version: str = ""
    taproot: str = ""
    cooperative: str = ""
    spend_path: str = ""
    send_amount_msat: int | None = None
    receive_amount_msat: int | None = None
    amount_evidence_conflict: bool = False
    route_evidence_conflict: bool = False
    identity_evidence_conflict: bool = False
    semantic_evidence_conflict: bool = False


def _match_by_provider_swap_id(
    out_rows: Sequence[Mapping], in_rows: Sequence[Mapping]
) -> list[tuple[Mapping, Mapping, _ProviderSwapEvidence, bool]]:
    """Pair rows carrying the same provider-scoped swap id.

    Provider metadata remains useful review evidence even when incomplete.  It
    reaches ``exact`` only for one outbound and one inbound under the provider
    key, with both canonical route txids tied to the rows and a plausible
    whole-row amount delta.  Duplicate/batched records, missing route ids, and
    implausible coverage are returned as ``strong`` manual-review candidates;
    they can never enter exact bulk/rule application.
    """
    out_by_key: dict[tuple[str, str], list[tuple[Mapping, _ProviderSwapEvidence]]] = {}
    for row in out_rows:
        evidence = _provider_swap_evidence(row)
        if evidence is None:
            continue
        out_by_key.setdefault((evidence.provider, evidence.swap_id), []).append(
            (row, evidence)
        )

    in_by_key: dict[tuple[str, str], list[tuple[Mapping, _ProviderSwapEvidence]]] = {}
    for in_row in in_rows:
        in_evidence = _provider_swap_evidence(in_row)
        if in_evidence is None:
            continue
        in_by_key.setdefault((in_evidence.provider, in_evidence.swap_id), []).append(
            (in_row, in_evidence)
        )

    pairs: list[tuple[Mapping, Mapping, _ProviderSwapEvidence, bool]] = []
    for key, outs in out_by_key.items():
        ins = in_by_key.get(key, [])
        unique_key = len(outs) == 1 and len(ins) == 1
        for out_row, out_evidence in outs:
            for in_row, in_evidence in ins:
            # Failed-swap refunds can legitimately return to the same wallet that
            # funded the lockup, so provider evidence intentionally does not apply
            # the same-wallet skip used by payment-hash swap claims.
                evidence = _merge_provider_evidence(out_evidence, in_evidence)
                if not _provider_route_matches_row(evidence, out_row, side="out"):
                    continue
                if not _provider_route_matches_row(evidence, in_row, side="in"):
                    continue
                whole_row_exact = unique_key and _provider_whole_row_coverage(
                    evidence, out_row, in_row
                )
                pairs.append((out_row, in_row, evidence, whole_row_exact))
    return pairs


def _provider_whole_row_coverage(
    evidence: _ProviderSwapEvidence,
    out_row: Mapping,
    in_row: Mapping,
) -> bool:
    """Whether provider evidence identifies both complete transaction rows."""

    send_txid = canonical_txid(evidence.send_txid)
    receive_txid = canonical_txid(evidence.receive_txid)
    if send_txid is None or receive_txid is None:
        return False
    out_scope = onchain_transfer_scope(out_row)
    in_scope = onchain_transfer_scope(in_row)
    if out_scope is None or in_scope is None:
        return False
    if out_scope[2] != send_txid or in_scope[2] != receive_txid:
        return False
    out_domain = bitcoin_network_domain(out_row)
    in_domain = bitcoin_network_domain(in_row)
    if out_domain is None or in_domain is None or out_domain != in_domain:
        return False
    if evidence.amount_evidence_conflict:
        return False
    if evidence.route_evidence_conflict:
        return False
    if evidence.identity_evidence_conflict:
        return False
    if evidence.semantic_evidence_conflict:
        return False
    if evidence.send_amount_msat != int(_record_get(out_row, "amount") or 0):
        return False
    if evidence.receive_amount_msat != int(_record_get(in_row, "amount") or 0):
        return False
    return _payment_hash_amounts_conserve(out_row, in_row)


def _provider_swap_evidence(row: Mapping) -> Optional[_ProviderSwapEvidence]:
    payload = _raw_json_payload(row)
    if not payload:
        return None
    provider_values = [
        _normalize_evidence_provider(value)
        for value in _all_text(payload, "provider", "source", "source_format")
    ]
    provider_values = [value for value in provider_values if value]
    provider = provider_values[0] if provider_values else ""
    if not provider:
        return None
    swap_id_values = _all_text(
        payload,
        "swap_id",
        "swapId",
        "swapID",
        "provider_swap_id",
        "boltz_id",
        "boltzId",
    )
    swap_id = swap_id_values[0] if swap_id_values else ""
    if not swap_id and provider in {"boltz", "bullbitcoin"}:
        swap_id = _first_text(payload, "id")
    if not swap_id:
        return None
    flow_values = _all_text(
        payload,
        "flow",
        "type",
        "swap_type",
        "swapType",
        "kind",
    )
    flow = flow_values[0] if flow_values else ""
    status_values = _all_text(payload, "status", "state", "finality")
    status = status_values[0] if status_values else ""
    kind = _kind_from_provider_status(status) or _kind_from_provider_flow(flow) or ""
    send_txid_keys = ("send_txid", "sendTxid", "lockup_txid", "lockupTxid")
    receive_txid_keys = (
        "receive_txid",
        "receiveTxid",
        "claim_txid",
        "claimTxid",
        "refund_txid",
        "refundTxid",
    )
    send_txid_values = _all_text(payload, *send_txid_keys)
    receive_txid_values = _all_text(payload, *receive_txid_keys)
    send_txid = send_txid_values[0] if send_txid_values else ""
    receive_txid = receive_txid_values[0] if receive_txid_values else ""
    if not kind and not (send_txid or receive_txid):
        return None
    return _ProviderSwapEvidence(
        provider=provider,
        swap_id=str(swap_id),
        kind=kind,
        flow=_normalize_flow_text(flow),
        status=status,
        send_txid=send_txid or "",
        receive_txid=receive_txid or "",
        version=_first_text(payload, "version", "swap_version", "swapVersion") or "",
        taproot=_first_text(payload, "taproot", "is_taproot", "isTaproot") or "",
        cooperative=_first_text(payload, "cooperative", "cooperative_claim", "cooperativeClaim")
        or "",
        spend_path=_first_text(payload, "spend_path", "spendPath", "claim_path", "claimPath")
        or "",
        send_amount_msat=_first_nonnegative_int(payload, "send_amount_msat"),
        receive_amount_msat=_first_nonnegative_int(payload, "receive_amount_msat"),
        route_evidence_conflict=(
            len({value.lower() for value in send_txid_values}) > 1
            or len({value.lower() for value in receive_txid_values}) > 1
        ),
        identity_evidence_conflict=(
            len(set(provider_values)) > 1
            or len({value.lower() for value in swap_id_values}) > 1
        ),
        semantic_evidence_conflict=(
            len({_normalize_flow_text(value) for value in flow_values}) > 1
            or len({_normalize_flow_text(value) for value in status_values}) > 1
        ),
    )


def _raw_json_payload(row: Mapping) -> Optional[Mapping]:
    raw_json = _record_get(row, "raw_json")
    if not raw_json:
        return None
    if isinstance(raw_json, Mapping):
        return raw_json
    try:
        payload = json.loads(str(raw_json))
    except (TypeError, ValueError):
        return None
    return payload if isinstance(payload, Mapping) else None


def _first_text(payload: Mapping, *keys: str) -> str:
    for key in keys:
        value = payload.get(key)
        if value is None:
            continue
        text = str(value).strip()
        if text:
            return text
    return ""


def _all_text(payload: Mapping, *keys: str) -> list[str]:
    values = []
    for key in keys:
        value = payload.get(key)
        if value is None:
            continue
        text = str(value).strip()
        if text:
            values.append(text)
    return values


def _first_nonnegative_int(payload: Mapping, *keys: str) -> int | None:
    """Read only explicitly msat-denominated provider amount evidence."""

    for key in keys:
        value = payload.get(key)
        if isinstance(value, bool) or value in (None, ""):
            continue
        if isinstance(value, int):
            parsed = value
        elif isinstance(value, str) and value.strip().isdigit():
            parsed = int(value.strip())
        else:
            # ``int(100.9)`` truncates. A fractional JSON number must never be
            # promoted into exact/bulk-eligible whole-row amount evidence.
            continue
        return parsed if parsed >= 0 else None
    return None


def _normalize_evidence_provider(value: str) -> str:
    text = (value or "").strip().lower().replace("-", "_")
    if not text:
        return ""
    if "bullbitcoin" in text or text == "bull":
        return "bullbitcoin"
    if "boltz" in text:
        return "boltz"
    return text


def _normalize_flow_text(value: str) -> str:
    return (value or "").strip().lower().replace("_", "-").replace(" ", "-")


def _kind_from_provider_flow(value: str) -> Optional[str]:
    flow = _normalize_flow_text(value)
    if not flow:
        return None
    if flow in {"chain", "chain-swap", "chainswap"}:
        return KIND_CHAIN_SWAP
    if flow in {"reverse", "reverse-swap", "reverse-submarine", "reverse-submarine-swap"}:
        return KIND_REVERSE_SUBMARINE_SWAP
    if flow in {"submarine", "submarine-swap"}:
        return KIND_SUBMARINE_SWAP
    if flow in {"refund", "swap-refund", "failed-swap-refund"}:
        return KIND_SWAP_REFUND
    return None


def _kind_from_provider_status(value: str) -> Optional[str]:
    status = _normalize_flow_text(value)
    if "refund" in status:
        return KIND_SWAP_REFUND
    return None


def _prefer_provider_kind(*kinds: str) -> str:
    if KIND_SWAP_REFUND in kinds:
        return KIND_SWAP_REFUND
    return next((kind for kind in kinds if kind), "")


def _merge_provider_evidence(
    out_evidence: _ProviderSwapEvidence,
    in_evidence: _ProviderSwapEvidence,
) -> _ProviderSwapEvidence:
    send_amounts = {
        value
        for value in (out_evidence.send_amount_msat, in_evidence.send_amount_msat)
        if value is not None
    }
    receive_amounts = {
        value
        for value in (
            out_evidence.receive_amount_msat,
            in_evidence.receive_amount_msat,
        )
        if value is not None
    }
    send_txids = {
        str(value).strip().lower()
        for value in (out_evidence.send_txid, in_evidence.send_txid)
        if str(value or "").strip()
    }
    receive_txids = {
        str(value).strip().lower()
        for value in (out_evidence.receive_txid, in_evidence.receive_txid)
        if str(value or "").strip()
    }
    return _ProviderSwapEvidence(
        provider=out_evidence.provider,
        swap_id=out_evidence.swap_id,
        kind=_prefer_provider_kind(out_evidence.kind, in_evidence.kind),
        flow=out_evidence.flow or in_evidence.flow,
        status=out_evidence.status or in_evidence.status,
        send_txid=out_evidence.send_txid or in_evidence.send_txid,
        receive_txid=out_evidence.receive_txid or in_evidence.receive_txid,
        version=out_evidence.version or in_evidence.version,
        taproot=out_evidence.taproot or in_evidence.taproot,
        cooperative=out_evidence.cooperative or in_evidence.cooperative,
        spend_path=out_evidence.spend_path or in_evidence.spend_path,
        send_amount_msat=next(iter(send_amounts), None),
        receive_amount_msat=next(iter(receive_amounts), None),
        amount_evidence_conflict=(
            out_evidence.amount_evidence_conflict
            or in_evidence.amount_evidence_conflict
            or len(send_amounts) > 1
            or len(receive_amounts) > 1
        ),
        route_evidence_conflict=(
            out_evidence.route_evidence_conflict
            or in_evidence.route_evidence_conflict
            or len(send_txids) > 1
            or len(receive_txids) > 1
        ),
        identity_evidence_conflict=(
            out_evidence.identity_evidence_conflict
            or in_evidence.identity_evidence_conflict
            or out_evidence.provider != in_evidence.provider
            or out_evidence.swap_id.lower() != in_evidence.swap_id.lower()
        ),
        semantic_evidence_conflict=(
            out_evidence.semantic_evidence_conflict
            or in_evidence.semantic_evidence_conflict
            or (
                bool(out_evidence.flow)
                and bool(in_evidence.flow)
                and _normalize_flow_text(out_evidence.flow)
                != _normalize_flow_text(in_evidence.flow)
            )
            or (
                bool(out_evidence.status)
                and bool(in_evidence.status)
                and _normalize_flow_text(out_evidence.status)
                != _normalize_flow_text(in_evidence.status)
            )
        ),
    )


def _provider_route_matches_row(
    evidence: _ProviderSwapEvidence,
    row: Mapping,
    *,
    side: str,
) -> bool:
    external_id = str(_record_get(row, "external_id") or "").strip()
    if not external_id:
        return True
    expected = evidence.send_txid if side == "out" else evidence.receive_txid
    if not expected:
        return True
    return external_id.lower() == str(expected).strip().lower()


def _match_heuristic(
    out_rows: Sequence[Mapping],
    in_rows: Sequence[Mapping],
    *,
    time_window_seconds: int,
    fee_pct_max: float,
    fee_sats_min: int,
) -> list[tuple[Mapping, Mapping]]:
    """Pair each outbound with every inbound inside the time window and
    fee tolerance.

    Inbound rows are resolved and time-sorted once so each outbound only
    scans the ``[out - window, out + window]`` bisect slice instead of
    every inbound — O((n_out + n_in) log n_in + matches) instead of
    O(n_out × n_in). Pair emission order differs from input order, which
    is fine: ``suggest_swap_candidates`` applies a total-order sort and
    conflict clustering is order-independent.
    """
    in_entries: list[tuple[float, Mapping, int]] = []
    for in_row in in_rows:
        in_seconds = _iso_to_seconds(_record_get(in_row, "occurred_at"))
        if in_seconds is None:
            continue
        in_amount = int(_record_get(in_row, "amount") or 0)
        if in_amount <= 0:
            # Zero/negative inbound rows (failed imports, placeholder
            # rows) would otherwise match any small outbound within the
            # absolute fee floor.
            continue
        in_entries.append((in_seconds, in_row, in_amount))
    in_entries.sort(key=lambda entry: entry[0])
    in_times = [entry[0] for entry in in_entries]

    pairs: list[tuple[Mapping, Mapping]] = []
    for out_row in out_rows:
        out_seconds = _iso_to_seconds(_record_get(out_row, "occurred_at"))
        if out_seconds is None:
            continue
        out_amount = int(_record_get(out_row, "amount") or 0)
        if out_amount <= 0:
            continue
        threshold = fee_threshold_msat(out_amount, fee_pct_max, fee_sats_min)
        out_wallet_id = _record_get(out_row, "wallet_id")
        # Both bounds inclusive, matching the previous abs(delta) > window
        # rejection exactly.
        lo = bisect.bisect_left(in_times, out_seconds - time_window_seconds)
        hi = bisect.bisect_right(in_times, out_seconds + time_window_seconds)
        out_asset = str(_record_get(out_row, "asset") or "")
        out_wallet_kind = str(_record_get(out_row, "wallet_kind") or "")
        for _, in_row, in_amount in in_entries[lo:hi]:
            if out_wallet_id == _record_get(in_row, "wallet_id"):
                continue
            delta = out_amount - in_amount
            if delta < 0 or delta > threshold:
                continue
            in_asset = str(_record_get(in_row, "asset") or "")
            if out_asset.upper() != in_asset.upper():
                # Cross-asset (layer-hop) candidates must look like a recognized
                # peg / submarine route, not just two similar-sized legs that
                # happen to fall inside the window+fee band. Without this, an
                # unrelated L-BTC disposal and BTC acquisition get matched and
                # stamped `strong`, weldable into a carrying-value pair that
                # corrupts basis on both sides. Genuine pegs are ~1:1 minus a
                # tiny federation fee, so the standard fee band still admits them.
                kind = default_kind_for(
                    out_asset, in_asset,
                    out_wallet_kind,
                    str(_record_get(in_row, "wallet_kind") or ""),
                )
                if kind == KIND_MANUAL:
                    continue
            pairs.append((out_row, in_row))
    return pairs


def _build_candidate(
    out_row: Mapping,
    in_row: Mapping,
    *,
    confidence: str,
    method: str,
    default_kind: Optional[str] = None,
    evidence: Optional[_ProviderSwapEvidence] = None,
) -> SwapCandidate:
    out_amount = int(_record_get(out_row, "amount") or 0)
    in_amount = int(_record_get(in_row, "amount") or 0)
    swap_fee_msat, swap_fee_kind = compute_swap_fee(
        out_amount,
        in_amount,
        _outbound_fee_component_msat(out_row),
    )
    out_asset = str(_record_get(out_row, "asset") or "")
    in_asset = str(_record_get(in_row, "asset") or "")
    out_wallet_kind = str(_record_get(out_row, "wallet_kind") or "")
    in_wallet_kind = str(_record_get(in_row, "wallet_kind") or "")
    default_policy = (
        POLICY_CARRYING_VALUE
        if out_asset.upper() == in_asset.upper()
        else default_ownership_policy_for(
            out_asset,
            in_asset,
        )
    )
    return SwapCandidate(
        out_id=str(_record_get(out_row, "id")),
        in_id=str(_record_get(in_row, "id")),
        out_asset=out_asset,
        in_asset=in_asset,
        out_amount_msat=out_amount,
        in_amount_msat=in_amount,
        out_wallet_id=str(_record_get(out_row, "wallet_id") or ""),
        in_wallet_id=str(_record_get(in_row, "wallet_id") or ""),
        out_wallet_label=str(_record_get(out_row, "wallet_label") or ""),
        in_wallet_label=str(_record_get(in_row, "wallet_label") or ""),
        out_wallet_kind=out_wallet_kind,
        in_wallet_kind=in_wallet_kind,
        out_occurred_at=str(_record_get(out_row, "occurred_at") or ""),
        in_occurred_at=str(_record_get(in_row, "occurred_at") or ""),
        confidence=confidence,
        method=method,
        swap_fee_msat=swap_fee_msat,
        swap_fee_kind=swap_fee_kind,
        default_kind=default_kind
        or default_kind_for(out_asset, in_asset, out_wallet_kind, in_wallet_kind),
        default_policy=default_policy,
        evidence_provider=evidence.provider if evidence else "",
        evidence_id=evidence.swap_id if evidence else "",
        evidence_kind=evidence.kind if evidence else "",
        evidence_status=evidence.status if evidence else "",
        evidence_version=evidence.version if evidence else "",
        evidence_taproot=evidence.taproot if evidence else "",
        evidence_cooperative=evidence.cooperative if evidence else "",
        evidence_spend_path=evidence.spend_path if evidence else "",
        # conflict_set_id / conflict_size are filled in by _stamp_conflict_set_ids
    )


def _outbound_fee_component_msat(row: Mapping) -> int:
    if _record_get(row, "amount_includes_fee"):
        return 0
    try:
        return max(0, int(_record_get(row, "fee") or 0))
    except (TypeError, ValueError):
        return 0


def _dedupe_candidate_edges(
    candidates: Sequence[SwapCandidate],
) -> list[SwapCandidate]:
    """Keep the strongest evidence method for each identical row edge.

    Conflict cardinality represents alternative counterparties, not the number
    of evidence labels that happened to support the same out/in pair.
    """

    method_rank = {
        METHOD_HTLC_REFUND: 0,
        METHOD_PAYMENT_HASH: 1,
        METHOD_PROVIDER_SWAP_ID: 2,
        METHOD_OWNERSHIP_GRAPH: 3,
        METHOD_HEURISTIC: 4,
    }
    by_edge: dict[tuple[str, str], SwapCandidate] = {}
    for candidate in candidates:
        key = (candidate.out_id, candidate.in_id)
        current = by_edge.get(key)
        candidate_rank = (
            0 if candidate.confidence == CONFIDENCE_EXACT else 1,
            method_rank.get(candidate.method, 99),
        )
        current_rank = (
            0 if current and current.confidence == CONFIDENCE_EXACT else 1,
            method_rank.get(current.method, 99) if current else 99,
        )
        if current is None or candidate_rank < current_rank:
            by_edge[key] = candidate
    return list(by_edge.values())


def finalize_candidate_conflicts(
    candidates: Sequence[SwapCandidate],
) -> list[SwapCandidate]:
    """Deduplicate edges and stamp conflicts over one complete review set.

    Callers that add evidence sources outside this pure matcher (currently the
    ownership-graph quarantine bridge) must pass the combined population here
    before filtering, bulk application, or rule evaluation.
    """

    return _stamp_conflict_set_ids(_dedupe_candidate_edges(candidates))


def _stamp_conflict_set_ids(candidates: Sequence[SwapCandidate]) -> list[SwapCandidate]:
    """Stamp each candidate's ``conflict_set_id`` and ``conflict_size``.

    Two candidates conflict when they share an out or in leg. The
    cluster id is the lexicographic minimum of the candidate keys in
    that cluster, so the same data always yields the same cluster id;
    the size is the cluster's cardinality over the full candidate set,
    so downstream filtering cannot make a conflicted candidate look
    solo. Exact-confidence candidates dominate heuristic candidates
    sharing a leg — the heuristic siblings drop out so they can't be
    bulk-paired by mistake.
    """
    surviving: list[SwapCandidate] = []
    consumed_legs_by_exact: set[str] = set()
    for candidate in candidates:
        if candidate.confidence == CONFIDENCE_EXACT:
            surviving.append(candidate)
            consumed_legs_by_exact.add(candidate.out_id)
            consumed_legs_by_exact.add(candidate.in_id)
    for candidate in candidates:
        if candidate.confidence == CONFIDENCE_EXACT:
            continue
        # Pure amount/time heuristics add no independent identity evidence and
        # may be pruned when an exact edge owns the same leg. Ownership/provider
        # review evidence is different: a contradictory edge is itself a reason
        # the "exact" edge must not auto-apply, so retain it in the global
        # conflict cluster.
        if candidate.method == METHOD_HEURISTIC:
            if candidate.out_id in consumed_legs_by_exact:
                continue
            if candidate.in_id in consumed_legs_by_exact:
                continue
        surviving.append(candidate)

    parent: dict[str, str] = {}

    def find(node: str) -> str:
        while parent.get(node, node) != node:
            parent[node] = parent.get(parent[node], parent[node])
            node = parent[node]
        return node

    def union(a: str, b: str) -> None:
        root_a = find(a)
        root_b = find(b)
        if root_a == root_b:
            return
        if root_a < root_b:
            parent[root_b] = root_a
        else:
            parent[root_a] = root_b

    leg_to_candidates: dict[str, list[str]] = {}
    for candidate in surviving:
        key = _candidate_key(candidate)
        parent.setdefault(key, key)
        for leg in (candidate.out_id, candidate.in_id):
            leg_to_candidates.setdefault(leg, []).append(key)
    for siblings in leg_to_candidates.values():
        if len(siblings) <= 1:
            continue
        first = siblings[0]
        for other in siblings[1:]:
            union(first, other)

    cluster_sizes: dict[str, int] = {}
    for candidate in surviving:
        root = find(_candidate_key(candidate))
        cluster_sizes[root] = cluster_sizes.get(root, 0) + 1

    stamped: list[SwapCandidate] = []
    for candidate in surviving:
        root = find(_candidate_key(candidate))
        stamped.append(
            SwapCandidate(
                **{
                    **candidate.__dict__,
                    "conflict_set_id": root,
                    "conflict_size": cluster_sizes[root],
                }
            )
        )
    return stamped


def _candidate_key(candidate: SwapCandidate) -> str:
    return f"{candidate.out_id}->{candidate.in_id}"


def _record_get(record: Mapping, key: str):
    if hasattr(record, "keys") and not isinstance(record, dict):
        keys = record.keys()
        if hasattr(keys, "__contains__") and key in keys:
            return record[key]
        return None
    if isinstance(record, dict):
        return record.get(key)
    return getattr(record, key, None)
