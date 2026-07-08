"""Cross-asset / cross-wallet swap-candidate matcher.

Sits between the raw ``transactions`` table and the review surfaces (CLI,
daemon, UI). Given a profile's unpaired transactions plus the existing
pair / dismissal records, it returns the candidate pairings the matcher
believes form one swap.

Confidence ladder
-----------------

* **exact** — both legs share deterministic evidence: a Lightning
  ``payment_hash``, a redacted provider/client ``swap_id``, or an
  on-chain HTLC refund spend.
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
    is_bitcoin_rail_pair,
    is_lightning_payment_hash_row,
    normalize_group_txid,
)


LIGHTNING_WALLET_KINDS = frozenset({"phoenix", "coreln", "lnd", "nwc"})
# On-chain self-custody BTC wallet kinds — eligible ends of a base-layer <-> Liquid
# peg. wasabi/samourai are on-chain BTC wallets too, so a peg from them must still
# be recognized (else the cross-asset route guard hides legitimate candidates).
CHAIN_WALLET_KINDS = frozenset(
    {"descriptor", "xpub", "address", "wasabi", "samourai"}
)

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
    ``provider_swap_id``, or an HTLC refund funding link, or by the time
    + amount heuristic. Fee, default kind, default policy, and conflict
    cluster are all computed once at match time so the review surface can
    render without re-deriving them.
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


def suggest_swap_candidates(
    rows: Sequence[Mapping],
    *,
    pair_records: Iterable[Mapping] = (),
    dismissals: Iterable[Mapping] = (),
    time_window_seconds: int = DEFAULT_TIME_WINDOW_SECONDS,
    fee_pct_max: float = DEFAULT_FEE_PCT_MAX,
    fee_sats_min: int = DEFAULT_FEE_SATS_MIN,
    tax_country: Optional[str] = None,
    bitcoin_rail_carrying_value: bool = True,
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
        tax_country: Profile tax country code; informs the default policy.
        bitcoin_rail_carrying_value: When true, BTC/LBTC rail movements default
            to ``carrying-value`` outside country-specific rules.
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

    eligible_rows = _select_eligible_rows(rows, paired_ids)
    deterministic_transfer_ids = _deterministic_self_transfer_ids(
        eligible_rows, fee_pct_max, fee_sats_min
    )
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

    exact_pairs = _match_by_payment_hash(out_rows, in_rows)

    consumed_out = {pair[0]["id"] for pair in exact_pairs}
    consumed_in = {pair[1]["id"] for pair in exact_pairs}
    evidence_pairs = _match_by_provider_swap_id(
        [row for row in out_rows if row["id"] not in consumed_out],
        [row for row in in_rows if row["id"] not in consumed_in],
    )
    consumed_out |= {pair[0]["id"] for pair in evidence_pairs}
    consumed_in |= {pair[1]["id"] for pair in evidence_pairs}
    refund_pairs = _match_by_refund_link(
        [row for row in out_rows if row["id"] not in consumed_out],
        [row for row in in_rows if row["id"] not in consumed_in],
    )
    consumed_out |= {pair[0]["id"] for pair in refund_pairs}
    consumed_in |= {pair[1]["id"] for pair in refund_pairs}
    heuristic_pairs = _match_heuristic(
        [row for row in out_rows if row["id"] not in consumed_out],
        [row for row in in_rows if row["id"] not in consumed_in],
        time_window_seconds=time_window_seconds,
        fee_pct_max=fee_pct_max,
        fee_sats_min=fee_sats_min,
    )

    raw_candidates: list[SwapCandidate] = []
    for out_row, in_row in exact_pairs:
        if (out_row["id"], in_row["id"]) in dismissed_pairs:
            continue
        raw_candidates.append(_build_candidate(
            out_row,
            in_row,
            confidence=CONFIDENCE_EXACT,
            method=METHOD_PAYMENT_HASH,
            tax_country=tax_country,
            bitcoin_rail_carrying_value=bitcoin_rail_carrying_value,
        ))
    for out_row, in_row, evidence in evidence_pairs:
        if (out_row["id"], in_row["id"]) in dismissed_pairs:
            continue
        raw_candidates.append(_build_candidate(
            out_row,
            in_row,
            confidence=CONFIDENCE_EXACT,
            method=METHOD_PROVIDER_SWAP_ID,
            tax_country=tax_country,
            bitcoin_rail_carrying_value=bitcoin_rail_carrying_value,
            default_kind=evidence.kind or None,
            evidence=evidence,
        ))
    for out_row, in_row in refund_pairs:
        if (out_row["id"], in_row["id"]) in dismissed_pairs:
            continue
        raw_candidates.append(_build_candidate(
            out_row,
            in_row,
            confidence=CONFIDENCE_EXACT,
            method=METHOD_HTLC_REFUND,
            tax_country=tax_country,
            bitcoin_rail_carrying_value=bitcoin_rail_carrying_value,
            default_kind=KIND_SWAP_REFUND,
        ))
    for out_row, in_row in heuristic_pairs:
        if (out_row["id"], in_row["id"]) in dismissed_pairs:
            continue
        raw_candidates.append(_build_candidate(
            out_row,
            in_row,
            confidence=CONFIDENCE_STRONG,
            method=METHOD_HEURISTIC,
            tax_country=tax_country,
            bitcoin_rail_carrying_value=bitcoin_rail_carrying_value,
        ))

    candidates = _stamp_conflict_set_ids(raw_candidates)
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
    return out_amount_msat + out_fee_msat - in_amount_msat, "combined"


def default_kind_for(
    out_asset: str,
    in_asset: str,
    out_wallet_kind: str,
    in_wallet_kind: str,
) -> str:
    """Infer the swap kind from leg shapes.

    Heavy-user defaults:

    * Either leg is a Lightning wallet → ``submarine-swap``
      (Boltz / Aqua / similar).
    * Both legs are chain wallets:
      * BTC → LBTC → ``peg-in``.
      * LBTC → BTC → ``peg-out``.
    * Everything else → ``manual`` (the user picks).
    """
    out_kind = (out_wallet_kind or "").lower()
    in_kind = (in_wallet_kind or "").lower()
    if out_kind in LIGHTNING_WALLET_KINDS or in_kind in LIGHTNING_WALLET_KINDS:
        return KIND_SUBMARINE_SWAP
    if out_kind in CHAIN_WALLET_KINDS and in_kind in CHAIN_WALLET_KINDS:
        if out_asset == "BTC" and in_asset == "LBTC":
            return KIND_PEG_IN
        if out_asset == "LBTC" and in_asset == "BTC":
            return KIND_PEG_OUT
    return KIND_MANUAL


def default_policy_for(
    tax_country: Optional[str],
    out_asset: Optional[str] = None,
    in_asset: Optional[str] = None,
    *,
    bitcoin_rail_carrying_value: bool = True,
) -> str:
    """Return the profile default transfer-pair policy.

    BTC/LBTC rail changes are carrying-value candidates for every profile
    when the profile setting is enabled, because they represent the same
    Bitcoin exposure on different rails. Other non-Austrian cross-asset
    candidates keep the taxable default.
    """
    if (tax_country or "").strip().lower() == "at":
        return POLICY_CARRYING_VALUE
    if bitcoin_rail_carrying_value and is_bitcoin_rail_pair(out_asset, in_asset):
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


def _deterministic_self_transfer_ids(
    rows: Sequence[Mapping],
    fee_pct_max: float = DEFAULT_FEE_PCT_MAX,
    fee_sats_min: int = DEFAULT_FEE_SATS_MIN,
) -> set[object]:
    """Return row ids that are already proven same-chain self-transfers.

    The swap review queue is for ambiguous layer hops. A single outbound and
    single inbound row with the same external transaction id and same asset,
    across two owned wallets, is the conservative on-chain self-transfer signal
    used by the journal pipeline. This mirrors
    ``kassiber.transfers.detect_intra_transfers``; keep the predicates in
    lockstep so ordinary cold-to-hot moves do not look like swaps to review.

    Exception: when the implied fee (``out_amount - in_amount``) blows past the
    swap-fee tolerance, the outbound almost certainly fanned out to an
    unrecognized recipient (a cross-asset peg/swap or a payment), so this is NOT
    a clean self-transfer. The journal pipeline quarantines it
    (``transfer_fee_implausible`` in ``normalize_tax_asset_inputs``) rather than
    booking the residual as a fee; here we correspondingly leave it eligible for
    swap review instead of silently claiming it as a proven self-transfer.
    """
    grouped: dict[tuple[str, str], list[Mapping]] = {}
    for row in rows:
        external_id = _record_get(row, "external_id")
        if not external_id:
            continue
        # Mirror transfers.detect_intra_transfers so a mixed-case txid does not
        # desync the swap queue from the journal (same self-transfer grouping).
        key = (normalize_group_txid(external_id), _record_get(row, "asset"))
        grouped.setdefault(key, []).append(row)

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
        if len(outs) != 1 or len(ins) != 1:
            continue
        out_row = outs[0]
        in_row = ins[0]
        if _record_get(out_row, "wallet_id") == _record_get(in_row, "wallet_id"):
            continue
        out_amount = int(_record_get(out_row, "amount") or 0)
        in_amount = int(_record_get(in_row, "amount") or 0)
        # When the out leg's `amount` is a net wallet delta with the fee folded
        # in (BTCPay; `amount_includes_fee`), the out/in gap IS the miner fee, so
        # it is not an implausible residual — keep it a proven self-transfer
        # (suppressed from swap review), matching the journal's transfer-fee guard
        # in tax_events.normalize_tax_asset_inputs.
        if not _record_get(out_row, "amount_includes_fee") and (
            out_amount - in_amount
            > fee_threshold_msat(out_amount, fee_pct_max, fee_sats_min)
        ):
            # Implausible implied fee — likely an unrecognized peg/payment leg.
            # Don't claim it as a proven self-transfer; let it reach swap review.
            continue
        deterministic_ids.add(_record_get(out_row, "id"))
        deterministic_ids.add(_record_get(in_row, "id"))

    # Mirror of the journal's Lightning payment-hash pass
    # (transfers.detect_intra_transfers): an own-node payment whose hash
    # matches another owned node's invoice is netted as a MOVE by the journal,
    # so it must not surface as an exact payment_hash swap candidate. ONLY
    # node-sourced hashes qualify — a chain_script HTLC hash (reverse swap
    # claim) is swap evidence and stays reviewable.
    by_hash: dict[tuple[str, str], list[Mapping]] = {}
    for row in rows:
        if _record_get(row, "id") in deterministic_ids:
            continue
        payment_hash = _record_get(row, "payment_hash")
        if not payment_hash:
            continue
        if not is_lightning_payment_hash_row(row):
            continue
        by_hash.setdefault(
            (str(payment_hash), _record_get(row, "asset")), []
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
        if _record_get(out_row, "wallet_id") == _record_get(in_row, "wallet_id"):
            continue
        deterministic_ids.add(_record_get(out_row, "id"))
        deterministic_ids.add(_record_get(in_row, "id"))
    return deterministic_ids


def _match_by_payment_hash(
    out_rows: Sequence[Mapping], in_rows: Sequence[Mapping]
) -> list[tuple[Mapping, Mapping]]:
    out_by_hash: dict[str, list[Mapping]] = {}
    for row in out_rows:
        payment_hash = _record_get(row, "payment_hash")
        if not payment_hash:
            continue
        out_by_hash.setdefault(payment_hash, []).append(row)
    in_by_hash: dict[str, list[Mapping]] = {}
    for row in in_rows:
        payment_hash = _record_get(row, "payment_hash")
        if not payment_hash:
            continue
        in_by_hash.setdefault(payment_hash, []).append(row)
    pairs: list[tuple[Mapping, Mapping]] = []
    for payment_hash, outs in out_by_hash.items():
        ins = in_by_hash.get(payment_hash, [])
        for out_row in outs:
            for in_row in ins:
                if _record_get(out_row, "wallet_id") == _record_get(in_row, "wallet_id"):
                    continue
                pairs.append((out_row, in_row))
    return pairs


def _match_by_refund_link(
    out_rows: Sequence[Mapping], in_rows: Sequence[Mapping]
) -> list[tuple[Mapping, Mapping]]:
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
    pairs: list[tuple[Mapping, Mapping]] = []
    for in_row in in_rows:
        funding_txid = _record_get(in_row, "swap_refund_funding_txid")
        if not funding_txid:
            continue
        in_asset = str(_record_get(in_row, "asset") or "").upper()
        for out_row in out_by_external_id.get(str(funding_txid).lower(), []):
            if str(_record_get(out_row, "asset") or "").upper() != in_asset:
                continue
            pairs.append((out_row, in_row))
    return pairs


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


def _match_by_provider_swap_id(
    out_rows: Sequence[Mapping], in_rows: Sequence[Mapping]
) -> list[tuple[Mapping, Mapping, _ProviderSwapEvidence]]:
    """Pair rows carrying the same provider-scoped swap id.

    This is the exact path for cooperative Taproot swaps: a key-path spend is
    deliberately indistinguishable on-chain, so deterministic evidence has to
    come from a redacted provider/client export, SDK regtest bridge, or other
    local metadata import. The matcher requires a structured provider/source
    marker plus a swap id; arbitrary raw ``id`` fields and free-text
    counterparty labels are intentionally ignored.
    """
    out_by_key: dict[tuple[str, str], list[tuple[Mapping, _ProviderSwapEvidence]]] = {}
    for row in out_rows:
        evidence = _provider_swap_evidence(row)
        if evidence is None:
            continue
        out_by_key.setdefault((evidence.provider, evidence.swap_id), []).append(
            (row, evidence)
        )

    pairs: list[tuple[Mapping, Mapping, _ProviderSwapEvidence]] = []
    for in_row in in_rows:
        in_evidence = _provider_swap_evidence(in_row)
        if in_evidence is None:
            continue
        for out_row, out_evidence in out_by_key.get(
            (in_evidence.provider, in_evidence.swap_id), []
        ):
            # Failed-swap refunds can legitimately return to the same wallet that
            # funded the lockup, so provider evidence intentionally does not apply
            # the same-wallet skip used by payment-hash swap claims.
            evidence = _merge_provider_evidence(out_evidence, in_evidence)
            if not _provider_route_matches_row(evidence, out_row, side="out"):
                continue
            if not _provider_route_matches_row(evidence, in_row, side="in"):
                continue
            pairs.append((out_row, in_row, evidence))
    return pairs


def _provider_swap_evidence(row: Mapping) -> Optional[_ProviderSwapEvidence]:
    payload = _raw_json_payload(row)
    if not payload:
        return None
    provider = _normalize_evidence_provider(
        _first_text(
            payload,
            "provider",
            "source",
            "source_format",
        )
    )
    if not provider:
        return None
    swap_id = _first_text(
        payload,
        "swap_id",
        "swapId",
        "swapID",
        "provider_swap_id",
        "boltz_id",
        "boltzId",
    )
    if not swap_id and provider in {"boltz", "bullbitcoin"}:
        swap_id = _first_text(payload, "id")
    if not swap_id:
        return None
    flow = _first_text(
        payload,
        "flow",
        "type",
        "swap_type",
        "swapType",
        "kind",
    )
    status = _first_text(payload, "status", "state", "finality") or ""
    kind = _kind_from_provider_status(status) or _kind_from_provider_flow(flow) or ""
    send_txid = _first_text(
        payload, "send_txid", "sendTxid", "lockup_txid", "lockupTxid"
    )
    receive_txid = _first_text(
        payload,
        "receive_txid",
        "receiveTxid",
        "claim_txid",
        "claimTxid",
        "refund_txid",
        "refundTxid",
    )
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
    tax_country: Optional[str],
    bitcoin_rail_carrying_value: bool = True,
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
        else default_policy_for(
            tax_country,
            out_asset,
            in_asset,
            bitcoin_rail_carrying_value=bitcoin_rail_carrying_value,
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
