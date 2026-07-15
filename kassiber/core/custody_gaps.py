"""Deterministic suggestions for gaps in a profile's custody history.

This module is deliberately an advisory layer.  It does not author custody
components, pair transactions, mutate journals, or decide that an unmatched
outflow was taxable.  Given already-imported transaction observations, it
finds bounded 1:1 and N:M groups whose aggregate Bitcoin quantity may have
left one known wallet and returned to another after an unobserved interval.

Time is a score, never a cutoff: a return one year later remains eligible.
The search is nevertheless operationally bounded by explicit row, grouping,
beam, and result limits. When a capacity ceiling is exceeded the matcher marks
the advisory search incomplete; callers must neither claim a clear wallet
history nor turn that operational limit into a tax/report blocker.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from datetime import datetime
import hashlib
import json
import sqlite3
from typing import Any, Iterable, Mapping, Sequence

from . import custody_quantity_store as core_custody_quantity_store
from .custody_evidence import resolve_protocol_scope

from ..errors import AppError
from ..time_utils import parse_iso_datetime_or_none


DEFAULT_MIN_COVERAGE_PPM = 800_000
DEFAULT_MAX_EXCESS_PPM = 250_000
# A long-lived CoinJoin wallet can easily exceed a few thousand observations.
# Grouping and beam limits below bound the expensive work; this ceiling exists
# to prevent a pathological caller from materializing an unlimited history.
DEFAULT_MAX_INPUT_ROWS = 50_000
DEFAULT_MAX_SOURCE_LEGS = 3
DEFAULT_MAX_RETURN_LEGS = 16
DEFAULT_MAX_AGGREGATE_RETURN_LEGS = 256
DEFAULT_MAX_SOURCE_GROUPS = 256
DEFAULT_MAX_RETURN_POOL = 512
DEFAULT_BEAM_WIDTH = 48
DEFAULT_MAX_RETURN_GROUPS_PER_SOURCE = 24
# Hard ceiling for the displayed candidate population. It is not a page size:
# exceeding it carries a fully scored deterministic prefix on the explicit
# nonblocking/incomplete-search exception.
DEFAULT_MAX_CANDIDATES = 250
DEFAULT_RETURN_ERA_GAP_SECONDS = 180 * 86_400
DEFAULT_PROMOTION_SCORE_MARGIN = 75

_PRIVACY_BOUNDARIES = frozenset({"coinjoin", "payjoin", "whirlpool"})
_SAMOURAI_WALLET_KINDS = frozenset({"samourai", "samourai-whirlpool", "whirlpool"})
_SAMOURAI_SECTIONS = frozenset({"deposit", "badbank", "premix", "postmix", "ricochet"})
_SAMOURAI_TRANSACTION_KINDS = frozenset(
    {
        "samourai_deposit",
        "samourai_tx0",
        "samourai_premix",
        "samourai_postmix",
        "samourai_badbank",
        "whirlpool_tx0",
        "whirlpool_mix",
        "premix",
        "postmix",
        "badbank",
        "coinjoin",
    }
)
_EXTERNAL_ORIGIN_KINDS = frozenset(
    {"income", "revenue", "sale", "exchange_buy", "customer_payment"}
)
_CHRONOLOGY_SIGNALS = frozenset(
    {"source_retired_before_destination_active", "wallet_roll_overlap"}
)
_TOPOLOGY_SIGNALS = frozenset(
    {"direct_ancestry", "native_path", "shared_policy_epoch"}
)


class CustodyGapSearchLimitError(ValueError):
    """Advisory search stopped at a configured capacity ceiling.

    Capacity says nothing about custody or tax classification. Consumers may
    show an incomplete-search warning, but must never turn this exception alone
    into a global report blocker.
    """

    def __init__(
        self,
        message: str,
        *,
        candidate_count: int | None = None,
        promotion_eligible_count: int | None = None,
        limit_kind: str = "capacity",
        partial_candidates: Sequence[Any] = (),
        normalized_legs: Sequence[Any] = (),
    ) -> None:
        super().__init__(message)
        self.candidate_count = candidate_count
        self.promotion_eligible_count = promotion_eligible_count
        self.limit_kind = limit_kind
        self.partial_candidates = tuple(partial_candidates)
        self.normalized_legs = tuple(normalized_legs)
        self.blocking = False
        self.search_complete = False


@dataclass(frozen=True)
class CustodyGapCandidate:
    """One non-authoritative missing-custody-history suggestion.

    ``retained_msat`` is capped at the source quantity.  A larger return is
    exposed separately as ``excess_msat``; it can never manufacture additional
    carried basis.  Likewise, ``residual_msat`` remains explicitly unresolved
    and is never labelled a fee by this matcher. ``source_total_msat`` is the
    principal used for matching; known network fees and the observed wallet
    debit remain separate in ``source_fee_msat`` / ``source_debit_msat``.
    """

    gap_id: str
    profile_id: str
    asset: str
    protocol_chain: str
    network: str
    source_ids: tuple[str, ...]
    return_ids: tuple[str, ...]
    source_wallet_ids: tuple[str, ...]
    destination_wallet_ids: tuple[str, ...]
    source_wallet_labels: tuple[str, ...]
    destination_wallet_labels: tuple[str, ...]
    source_total_msat: int
    source_fee_msat: int
    source_debit_msat: int
    return_total_msat: int
    retained_msat: int
    residual_msat: int
    excess_msat: int
    coverage_ppm: int
    started_at: str
    ended_at: str
    elapsed_seconds: int
    score: int
    confidence: str
    reason_codes: tuple[str, ...]
    promotion_eligible: bool = False
    competitor_score_margin: int | None = None
    conflict_set_id: str = ""
    # Cardinality is computed over the complete bounded population. Exceeding
    # that population's hard ceiling fails the search instead of returning a
    # partial conflict cluster.
    conflict_size: int = 1


@dataclass(frozen=True)
class _Leg:
    id: str
    profile_id: str
    wallet_id: str
    wallet_label: str
    occurred_at: str
    occurred_dt: datetime
    direction: str
    asset: str
    chain: str
    network: str
    principal_msat: int
    fee_msat: int
    debit_msat: int
    signal_codes: tuple[str, ...]
    disqualifier_codes: tuple[str, ...]


def suggest_custody_gap_candidates(
    rows: Sequence[Mapping[str, Any]],
    *,
    ignored_ids: Iterable[str] = (),
    min_coverage_ppm: int = DEFAULT_MIN_COVERAGE_PPM,
    max_excess_ppm: int = DEFAULT_MAX_EXCESS_PPM,
    max_input_rows: int = DEFAULT_MAX_INPUT_ROWS,
    max_source_legs: int = DEFAULT_MAX_SOURCE_LEGS,
    max_return_legs: int = DEFAULT_MAX_RETURN_LEGS,
    max_aggregate_return_legs: int = DEFAULT_MAX_AGGREGATE_RETURN_LEGS,
    max_source_groups: int = DEFAULT_MAX_SOURCE_GROUPS,
    max_return_pool: int = DEFAULT_MAX_RETURN_POOL,
    beam_width: int = DEFAULT_BEAM_WIDTH,
    max_return_groups_per_source: int = DEFAULT_MAX_RETURN_GROUPS_PER_SOURCE,
    max_candidates: int = DEFAULT_MAX_CANDIDATES,
    return_era_gap_seconds: int = DEFAULT_RETURN_ERA_GAP_SECONDS,
    promotion_score_margin: int = DEFAULT_PROMOTION_SCORE_MARGIN,
) -> list[CustodyGapCandidate]:
    """Suggest long-horizon custody bridges from transaction observations.

    The matcher assumes one profile represents one legal owner, but it does
    *not* assume that every wallet owned by that profile has been imported.
    Its output is a review candidate only and cannot activate a bridge.

    Amounts use Kassiber's exact integer msat representation. Matching compares
    return quantity with source principal only. A separately recorded network
    fee contributes to the wallet debit but never inflates the residual;
    ``amount_includes_fee`` prevents double counting net-delta imports.

    ``max_candidates`` bounds the complete generated population. Exceeding it
    raises :class:`CustodyGapSearchLimitError`; it never truncates the result.
    """

    _validate_limits(
        min_coverage_ppm=min_coverage_ppm,
        max_excess_ppm=max_excess_ppm,
        max_input_rows=max_input_rows,
        max_source_legs=max_source_legs,
        max_return_legs=max_return_legs,
        max_aggregate_return_legs=max_aggregate_return_legs,
        max_source_groups=max_source_groups,
        max_return_pool=max_return_pool,
        beam_width=beam_width,
        max_return_groups_per_source=max_return_groups_per_source,
        max_candidates=max_candidates,
        return_era_gap_seconds=return_era_gap_seconds,
        promotion_score_margin=promotion_score_margin,
    )
    ignored = {str(value) for value in ignored_ids}
    legs = [leg for row in rows if (leg := _normalize_leg(row, ignored)) is not None]
    if len(legs) > max_input_rows:
        raise CustodyGapSearchLimitError(
            f"custody-gap search received {len(legs)} eligible rows; "
            f"configured maximum is {max_input_rows}",
            candidate_count=None,
            promotion_eligible_count=0,
            limit_kind="input_rows",
        )

    by_scope: dict[tuple[str, str, str, str], list[_Leg]] = {}
    for leg in legs:
        by_scope.setdefault(
            (leg.profile_id, leg.asset, leg.chain, leg.network), []
        ).append(leg)

    generated: dict[str, CustodyGapCandidate] = {}
    for scope in sorted(by_scope):
        scoped = sorted(by_scope[scope], key=_leg_sort_key)
        sources = [leg for leg in scoped if leg.direction == "outbound"]
        returns = [leg for leg in scoped if leg.direction == "inbound"]
        if not sources or not returns:
            continue
        source_groups = _source_groups(
            sources,
            max_legs=max_source_legs,
            max_groups=max_source_groups,
        )
        for source_group in source_groups:
            boundary = max(leg.occurred_dt for leg in source_group)
            eligible_returns = [leg for leg in returns if leg.occurred_dt > boundary]
            if not eligible_returns:
                continue
            target = sum(leg.principal_msat for leg in source_group)
            return_pool = _bounded_return_pool(
                eligible_returns,
                target=target,
                maximum=max_return_pool,
            )
            return_groups = _return_groups(
                return_pool,
                target=target,
                min_coverage_ppm=min_coverage_ppm,
                max_excess_ppm=max_excess_ppm,
                max_legs=max_return_legs,
                beam_width=beam_width,
                result_limit=max_return_groups_per_source,
            )
            aggregate_groups = _wallet_era_return_groups(
                eligible_returns,
                target=target,
                min_coverage_ppm=min_coverage_ppm,
                max_excess_ppm=max_excess_ppm,
                max_legs=max_aggregate_return_legs,
                era_gap_seconds=return_era_gap_seconds,
                result_limit=max_return_groups_per_source,
            )
            return_groups = _dedupe_groups((*return_groups, *aggregate_groups))
            for return_group in return_groups:
                candidate = _build_candidate(source_group, return_group)
                if candidate.score < 650:
                    continue
                generated[candidate.gap_id] = candidate

    # Stamp conflicts and promotion eligibility before applying the display
    # ceiling. The exception carries a fully scored deterministic prefix plus
    # ``blocking=False`` / ``search_complete=False``. Candidate discovery is
    # advisory and capacity alone is never accounting evidence.
    stamped = _stamp_conflicts(list(generated.values()))
    stamped = _stamp_promotion_eligibility(
        stamped, required_margin=promotion_score_margin
    )
    stamped.sort(key=_candidate_sort_key)
    if len(stamped) > max_candidates:
        promotion_count = sum(candidate.promotion_eligible for candidate in stamped)
        raise CustodyGapSearchLimitError(
            "custody-gap search generated "
            f"{len(stamped)} candidates, including {promotion_count} "
            "promotion-eligible candidates; configured maximum is "
            f"{max_candidates}",
            candidate_count=len(stamped),
            promotion_eligible_count=promotion_count,
            limit_kind="candidate_population",
            partial_candidates=stamped[:max_candidates],
        )
    return stamped


def build_gap_snapshot(
    conn,
    profile_id: str,
    *,
    gap_id: str | None = None,
    limit: int = 100,
    cursor: str | None = None,
) -> dict[str, Any]:
    """Build the privacy-safe read payload used by desktop and AI surfaces.

    This adapter reads only imported transaction/wallet labels and existing
    authored claims.  It never exposes addresses, scripts, descriptors, xpubs,
    raw transaction graphs, or wallet configuration, and it performs no write.
    ``summary.search_complete`` says whether the bounded population is complete;
    ``gaps`` is the requested page and ``next_cursor`` continues that
    deterministic order.
    """

    if not isinstance(profile_id, str) or not profile_id.strip():
        raise ValueError("profile_id is required")
    if type(limit) is not int or limit < 1 or limit > 500:
        raise ValueError("limit must be an integer between 1 and 500")
    if cursor is not None and (
        not isinstance(cursor, str) or not cursor.isdigit()
    ):
        raise ValueError("cursor must be a non-negative integer string")
    offset = int(cursor or 0)
    if offset > 2**31 - 1:
        raise ValueError("cursor is out of range")
    if gap_id is not None and offset:
        raise ValueError("cursor is not supported when gap_id is provided")

    journal_status = _journal_status(conn, profile_id)
    # Page size must not change matcher semantics. Every page is cut from the
    # same bounded advisory population; an incomplete population is labelled
    # explicitly rather than represented as a global accounting failure.
    search_limit: CustodyGapSearchLimitError | None = None
    try:
        candidates, normalized = load_gap_candidates(
            conn,
            profile_id,
            limit=DEFAULT_MAX_CANDIDATES,
            include_journal_claims=journal_status == "current",
        )
    except CustodyGapSearchLimitError as exc:
        # Suggestions are advisory. Preserve any deterministic prefix that was
        # fully scored, disclose that discovery is incomplete, and continue to
        # report canonical readiness independently. A capacity limit is never
        # evidence that custody or tax basis is wrong.
        search_limit = exc
        candidates = [
            item
            for item in exc.partial_candidates
            if isinstance(item, CustodyGapCandidate)
        ]
        normalized = [
            item for item in exc.normalized_legs if isinstance(item, _Leg)
        ]
    if gap_id is not None:
        candidates = [candidate for candidate in candidates if candidate.gap_id == gap_id]
    from . import custody_gap_reviews

    reviews = custody_gap_reviews.latest_reviews(conn, profile_id)
    current_gaps: list[dict[str, Any]] = []
    for candidate in candidates:
        gap = _snapshot_gap(candidate, normalized)
        gap["candidate_fingerprint"] = custody_gap_reviews.candidate_fingerprint(candidate)
        review = reviews.get(candidate.gap_id)
        state = custody_gap_reviews.review_state(conn, candidate, review)
        gap["status"] = state["status"]
        if state["reason"]:
            gap["status_reason"] = state["reason"]
        if state.get("native_support_status"):
            gap["native_support_status"] = state["native_support_status"]
        if review and review.get("action") == "resolved":
            gap["correction"] = {
                "component_id": str(review.get("component_id") or ""),
                "strategy": "create_revision_then_activate",
            }
        current_gaps.append(gap)
    historical = custody_gap_reviews.historical_review_gaps(
        conn, profile_id, exclude_gap_ids=[candidate.gap_id for candidate in candidates]
    )
    if gap_id is not None:
        historical = [gap for gap in historical if gap.get("gap_id") == gap_id]
    # Review state is known only after matching.  Page unresolved work first
    # so high-scoring dismissed/resolved rows cannot starve lower-scoring
    # needs-review or conflicting rows from a bounded desktop/AI response.
    # Python's stable sort preserves deterministic matcher/history order
    # within each group.
    all_gaps = sorted(
        [*current_gaps, *historical],
        key=lambda gap: (
            0
            if gap.get("status") in {"needs_review", "conflicting"}
            else 1
        ),
    )
    page_end = offset + limit
    gaps = all_gaps[offset:page_end]
    next_cursor = str(page_end) if page_end < len(all_gaps) else None

    residual_by_cluster: dict[tuple[str, str], int] = {}
    for candidate, gap in zip(candidates, current_gaps):
        status = gap["status"]
        if status not in {"needs_review", "conflicting"}:
            continue
        cluster = (candidate.asset, candidate.conflict_set_id or candidate.gap_id)
        residual_by_cluster[cluster] = max(
            residual_by_cluster.get(cluster, 0), candidate.residual_msat
        )
    canonical = core_custody_quantity_store.custody_quantity_readiness_summary(
        conn,
        profile_id,
        journal_status=journal_status,
    )
    canonical_issue_count = int(canonical["issue_count"])
    canonical_unresolved_by_asset = canonical["unresolved_by_asset"]
    canonical_unresolved_msat = next(
        (
            int(item["amount_msat"])
            for item in canonical_unresolved_by_asset
            if item["asset"] == "BTC"
        ),
        0,
    )
    candidate_residual_by_asset_map: dict[str, int] = {}
    for (asset, _cluster), amount_msat in residual_by_cluster.items():
        candidate_residual_by_asset_map[asset] = (
            candidate_residual_by_asset_map.get(asset, 0) + amount_msat
        )
    candidate_residual_by_asset = [
        {"asset": asset, "amount_msat": amount_msat}
        for asset, amount_msat in sorted(candidate_residual_by_asset_map.items())
    ]
    candidate_residual_msat = candidate_residual_by_asset_map.get("BTC", 0)
    counts = {
        status: sum(gap.get("status") == status for gap in all_gaps)
        for status in ("needs_review", "conflicting", "resolved", "dismissed")
    }
    return {
        "summary": {
            "total": len(candidates) + len(historical),
            **counts,
            "unresolved_msat": (
                canonical_unresolved_msat
                if canonical_issue_count
                else candidate_residual_msat
            ),
            "candidate_residual_msat": candidate_residual_msat,
            "candidate_residual_by_asset": candidate_residual_by_asset,
            "canonical_unresolved_msat": canonical_unresolved_msat,
            "canonical_issue_count": canonical_issue_count,
            "canonical_unresolved_by_asset": canonical_unresolved_by_asset,
            "canonical_unquantified_issue_count": canonical[
                "unquantified_issue_count"
            ],
            "canonical_status": canonical["status"],
            "canonical_status_text": canonical["status_text"],
            "derived_state_current": canonical["derived_state_current"],
            "qualification": canonical["qualification"],
            "search_complete": search_limit is None,
            "search_status": (
                "complete" if search_limit is None else "capacity_limited"
            ),
            "search_limit_kind": (
                None if search_limit is None else search_limit.limit_kind
            ),
            "search_candidate_count": (
                len(candidates)
                if search_limit is None or search_limit.candidate_count is None
                else search_limit.candidate_count
            ),
        },
        "gaps": gaps,
        "next_cursor": next_cursor,
    }


def load_gap_candidates(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    limit: int = DEFAULT_MAX_CANDIDATES,
    include_journal_claims: bool | None = None,
) -> tuple[list[CustodyGapCandidate], list[_Leg]]:
    """Load the bounded current candidate set and normalized safe legs."""

    row_count = conn.execute(
        "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile_id,),
    ).fetchone()[0]
    if row_count > DEFAULT_MAX_INPUT_ROWS:
        raise CustodyGapSearchLimitError(
            "Custody-gap scan is larger than the current bounded search ceiling",
            candidate_count=None,
            promotion_eligible_count=0,
            limit_kind="input_rows",
        )

    raw_rows = conn.execute(
        """
        SELECT t.id, t.profile_id, t.wallet_id, w.label AS wallet_label,
               w.kind AS wallet_kind, t.occurred_at, t.direction, t.asset,
               t.amount, t.fee, t.amount_includes_fee, t.excluded,
               t.kind, t.privacy_boundary, t.external_id, t.raw_json
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND t.excluded = 0
        ORDER BY t.occurred_at, t.created_at, t.id
        """,
        (profile_id,),
    ).fetchall()
    wallet_configs: dict[str, Any] = {}
    try:
        wallet_configs = {
            str(_get(row, "id") or row[0]): _get(row, "config_json", row[1])
            for row in conn.execute(
                "SELECT id, config_json FROM wallets WHERE profile_id = ?",
                (profile_id,),
            ).fetchall()
        }
    except sqlite3.OperationalError:
        # Narrow migration/surface fixtures may predate wallet config_json.
        pass
    rows: list[dict[str, Any]] = []
    for row in raw_rows:
        mapped = {key: row[key] for key in row.keys()}
        mapped["wallet_config_json"] = wallet_configs.get(str(mapped["wallet_id"]), "{}")
        rows.append(mapped)
    if include_journal_claims is None:
        include_journal_claims = _journal_status(conn, profile_id) == "current"
    claimed_ids = _claimed_transaction_ids(
        conn,
        profile_id,
        include_journal_claims=include_journal_claims,
    )
    normalized = [leg for row in rows if (leg := _normalize_leg(row, set())) is not None]
    try:
        candidates = suggest_custody_gap_candidates(
            rows,
            ignored_ids=claimed_ids,
            max_candidates=limit,
        )
    except CustodyGapSearchLimitError as exc:
        exc.normalized_legs = tuple(normalized)
        raise
    return candidates, normalized


def find_gap_candidate(
    conn: sqlite3.Connection, profile_id: str, gap_id: str
) -> CustodyGapCandidate:
    try:
        candidates, _normalized = load_gap_candidates(conn, profile_id)
    except CustodyGapSearchLimitError as exc:
        # A fully scored prefix remains safe to review even though the advisory
        # queue is incomplete. The action still re-derives and fingerprints the
        # exact candidate; this does not imply anything about omitted hints.
        candidates = [
            item
            for item in exc.partial_candidates
            if isinstance(item, CustodyGapCandidate)
        ]
    for candidate in candidates:
        if candidate.gap_id == gap_id:
            return candidate
    raise AppError(
        "Custody gap not found in current evidence",
        code="custody_gap_not_found",
        hint="Reload the review queue; the evidence may have changed.",
    )


def _normalize_leg(row: Mapping[str, Any], ignored: set[str]) -> _Leg | None:
    row_id = str(_get(row, "id") or "").strip()
    if not row_id or row_id in ignored or _truthy(_get(row, "excluded")):
        return None
    direction = str(_get(row, "direction") or "").strip().lower()
    if direction not in {"outbound", "inbound"}:
        return None
    profile_id = str(_get(row, "profile_id") or "").strip()
    wallet_id = str(_get(row, "wallet_id") or "").strip()
    asset = str(_get(row, "asset") or "").strip().upper()
    occurred_dt = parse_iso_datetime_or_none(_get(row, "occurred_at"))
    amount = _exact_positive_int(_get(row, "amount_msat", _get(row, "amount")))
    if not profile_id or not wallet_id or not asset or occurred_dt is None or amount is None:
        return None
    fee = _exact_nonnegative_int(_get(row, "fee", 0))
    if fee is None:
        return None
    if direction == "outbound":
        if _truthy(_get(row, "amount_includes_fee")):
            # Net-delta imports fold any known fee into amount.  Most such
            # imports have fee=0 because the fee is unavailable; in that case
            # the whole observed debit is the safest available principal.
            if fee > amount:
                return None
            principal = amount - fee
            debit = amount
        else:
            principal = amount
            debit = amount + fee
        if principal <= 0:
            return None
    else:
        principal = amount
        fee = 0
        debit = amount
    occurred_at = occurred_dt.isoformat().replace("+00:00", "Z")
    try:
        scope = resolve_protocol_scope(row)
    except (TypeError, ValueError):
        # Canonical quantity records the typed invalid-scope blocker. The
        # advisory matcher must neither crash nor guess Bitcoin mainnet.
        return None
    signal_codes, disqualifier_codes = _structured_evidence_codes(row)
    return _Leg(
        id=row_id,
        profile_id=profile_id,
        wallet_id=wallet_id,
        wallet_label=str(_get(row, "wallet_label") or wallet_id),
        occurred_at=occurred_at,
        occurred_dt=occurred_dt,
        direction=direction,
        asset=asset,
        chain=scope.protocol_chain,
        network=scope.network,
        principal_msat=principal,
        fee_msat=fee,
        debit_msat=debit,
        signal_codes=signal_codes,
        disqualifier_codes=disqualifier_codes,
    )


def _structured_evidence_codes(
    row: Mapping[str, Any],
) -> tuple[tuple[str, ...], tuple[str, ...]]:
    """Read only typed evidence fields; labels and free text never promote."""

    signals: set[str] = set()
    disqualifiers: set[str] = set()
    privacy_boundary = str(_get(row, "privacy_boundary") or "").strip().lower()
    if privacy_boundary in _PRIVACY_BOUNDARIES:
        signals.add("structured_privacy_boundary")

    wallet_kind = str(_get(row, "wallet_kind") or "").strip().lower()
    if wallet_kind in _SAMOURAI_WALLET_KINDS:
        signals.add("structured_samourai_wallet")

    transaction_kind = str(_get(row, "kind") or "").strip().lower()
    if transaction_kind in _SAMOURAI_TRANSACTION_KINDS:
        signals.add("structured_samourai_transaction")
    if transaction_kind in _EXTERNAL_ORIGIN_KINDS:
        disqualifiers.add("structured_external_origin")

    config = _json_mapping(_get(row, "wallet_config_json"))
    samourai = config.get("samourai") if isinstance(config, dict) else None
    if isinstance(samourai, Mapping):
        role = str(samourai.get("role") or "").strip().lower()
        section = str(samourai.get("section") or "").strip().lower()
        if role in {"parent", "child"} and section in _SAMOURAI_SECTIONS:
            signals.add("structured_samourai_policy")

    samourai_role = str(_get(row, "samourai_role") or "").strip().lower()
    samourai_section = str(_get(row, "samourai_section") or "").strip().lower()
    if samourai_role in {"parent", "child"} and samourai_section in _SAMOURAI_SECTIONS:
        signals.add("structured_samourai_policy")

    chronology = str(_get(row, "custody_chronology_signal") or "").strip().lower()
    if chronology in _CHRONOLOGY_SIGNALS:
        signals.add("structured_custody_chronology")
    topology = str(_get(row, "custody_topology_signal") or "").strip().lower()
    if topology in _TOPOLOGY_SIGNALS:
        signals.add("structured_custody_topology")
    return tuple(sorted(signals)), tuple(sorted(disqualifiers))


def _json_mapping(value: Any) -> dict[str, Any]:
    if isinstance(value, Mapping):
        return dict(value)
    if not isinstance(value, str) or not value.strip():
        return {}
    try:
        parsed = json.loads(value)
    except (TypeError, ValueError):
        return {}
    return dict(parsed) if isinstance(parsed, Mapping) else {}


def _source_groups(
    rows: Sequence[_Leg], *, max_legs: int, max_groups: int
) -> list[tuple[_Leg, ...]]:
    ordered = sorted(rows, key=_leg_sort_key)
    groups: list[tuple[_Leg, ...]] = [(row,) for row in ordered]
    # Adjacent chronological boundary rows cover staged wallet rolls without
    # opening an unbounded powerset search.  Singletons are always retained.
    for size in range(2, max_legs + 1):
        for start in range(0, len(ordered) - size + 1):
            groups.append(tuple(ordered[start : start + size]))
    groups.sort(key=lambda group: (_group_end(group), len(group), _group_ids(group)))
    if len(groups) <= max_groups:
        return groups
    # Sampling would make ``search_complete=true`` a lie and could let the UI
    # show a qualified empty state after silently skipping a real wallet gap.
    # Capacity is advisory, but it must be explicit.
    raise CustodyGapSearchLimitError(
        "custody-gap source grouping needs to inspect "
        f"{len(groups)} groups; configured maximum is {max_groups}",
        candidate_count=None,
        promotion_eligible_count=0,
        limit_kind="source_groups",
    )


def _bounded_return_pool(
    rows: Sequence[_Leg], *, target: int, maximum: int
) -> list[_Leg]:
    # The caller passes the profile's chronology-sorted return stream.  Keep a
    # defensive linear check rather than sorting the full history for every
    # source group.
    ordered = list(rows)
    if any(
        _leg_sort_key(left) > _leg_sort_key(right)
        for left, right in zip(ordered, ordered[1:])
    ):
        ordered.sort(key=_leg_sort_key)
    if len(ordered) > maximum:
        raise CustodyGapSearchLimitError(
            "custody-gap beam needs to inspect "
            f"{len(ordered)} return rows for source amount {target}; "
            f"configured maximum is {maximum}"
        )
    return ordered


def _return_groups(
    rows: Sequence[_Leg],
    *,
    target: int,
    min_coverage_ppm: int,
    max_excess_ppm: int,
    max_legs: int,
    beam_width: int,
    result_limit: int,
) -> list[tuple[_Leg, ...]]:
    max_total = target + (target * max_excess_ppm // 1_000_000)
    minimum = target * min_coverage_ppm // 1_000_000
    states: list[tuple[tuple[_Leg, ...], int]] = [((), 0)]
    qualifying: dict[tuple[str, ...], tuple[_Leg, ...]] = {}
    for row in sorted(rows, key=_leg_sort_key):
        additions: list[tuple[tuple[_Leg, ...], int]] = []
        for group, total in states:
            if len(group) >= max_legs:
                continue
            next_total = total + row.principal_msat
            if next_total > max_total:
                continue
            next_group = group + (row,)
            additions.append((next_group, next_total))
            if next_total >= minimum:
                qualifying[_group_ids(next_group)] = next_group
        states.extend(additions)
        # Keep a deterministic mix of near-target and under-target partials.
        states = _dedupe_states(states)
        states.sort(key=lambda item: _beam_key(item, target))
        states = states[:beam_width]
    groups = list(qualifying.values())
    groups.sort(
        key=lambda group: (
            abs(target - sum(leg.principal_msat for leg in group)),
            len(group),
            _group_ids(group),
        )
    )
    return groups[:result_limit]


def _wallet_era_return_groups(
    rows: Sequence[_Leg],
    *,
    target: int,
    min_coverage_ppm: int,
    max_excess_ppm: int,
    max_legs: int,
    era_gap_seconds: int,
    result_limit: int,
) -> list[tuple[_Leg, ...]]:
    """Aggregate realistic many-receipt returns by wallet and activity era.

    A Postmix exit may fan into dozens of receipts.  Enumerating that powerset
    is neither useful nor bounded, so the deterministic unit is one destination
    wallet's contiguous activity era.  The time gap separates unrelated eras;
    it never rejects a candidate based on source-to-return distance.
    """

    by_wallet: dict[str, list[_Leg]] = {}
    for row in rows:
        by_wallet.setdefault(row.wallet_id, []).append(row)
    minimum = target * min_coverage_ppm // 1_000_000
    max_total = target + (target * max_excess_ppm // 1_000_000)
    groups: list[tuple[_Leg, ...]] = []
    for wallet_id in sorted(by_wallet):
        ordered = sorted(by_wallet[wallet_id], key=_leg_sort_key)
        eras: list[list[_Leg]] = [[]]
        for row in ordered:
            if eras[-1]:
                delta = int((row.occurred_dt - eras[-1][-1].occurred_dt).total_seconds())
                if delta > era_gap_seconds:
                    eras.append([])
            eras[-1].append(row)
        for era in eras:
            total = sum(row.principal_msat for row in era)
            if not minimum <= total <= max_total:
                continue
            if len(era) > max_legs:
                raise CustodyGapSearchLimitError(
                    "custody-gap wallet/era aggregation needs "
                    f"{len(era)} return legs; configured maximum is {max_legs}"
                )
            groups.append(tuple(era))
    groups.sort(
        key=lambda group: (
            abs(target - sum(row.principal_msat for row in group)),
            len(group),
            _group_ids(group),
        )
    )
    return groups[:result_limit]


def _dedupe_groups(groups: Sequence[tuple[_Leg, ...]]) -> list[tuple[_Leg, ...]]:
    deduped = {_group_ids(group): group for group in groups}
    return [deduped[key] for key in sorted(deduped)]


def _build_candidate(
    sources: tuple[_Leg, ...], returns: tuple[_Leg, ...]
) -> CustodyGapCandidate:
    source_total = sum(leg.principal_msat for leg in sources)
    source_fee = sum(leg.fee_msat for leg in sources)
    source_debit = sum(leg.debit_msat for leg in sources)
    return_total = sum(leg.principal_msat for leg in returns)
    retained = min(source_total, return_total)
    residual = source_total - retained
    excess = return_total - retained
    coverage_ppm = retained * 1_000_000 // source_total
    started = min(leg.occurred_dt for leg in sources)
    ended = max(leg.occurred_dt for leg in returns)
    elapsed = max(0, int((ended - started).total_seconds()))
    source_wallet_ids = tuple(sorted({leg.wallet_id for leg in sources}))
    destination_wallet_ids = tuple(sorted({leg.wallet_id for leg in returns}))
    source_labels = tuple(sorted({leg.wallet_label for leg in sources}))
    destination_labels = tuple(sorted({leg.wallet_label for leg in returns}))
    reasons = _reason_codes(
        sources,
        returns,
        coverage_ppm=coverage_ppm,
        residual=residual,
        excess=excess,
        elapsed=elapsed,
        source_wallet_ids=source_wallet_ids,
        destination_wallet_ids=destination_wallet_ids,
    )
    score = _score(
        coverage_ppm=coverage_ppm,
        excess_msat=excess,
        source_total_msat=source_total,
        elapsed_seconds=elapsed,
        different_wallets=set(source_wallet_ids) != set(destination_wallet_ids),
        structured_signal_count=sum(
            len(leg.signal_codes) for leg in (*sources, *returns)
        ),
    )
    gap_id = custody_gap_id(
        sources[0].profile_id,
        sources[0].asset,
        _group_ids(sources),
        _group_ids(returns),
    )
    return CustodyGapCandidate(
        gap_id=gap_id,
        profile_id=sources[0].profile_id,
        asset=sources[0].asset,
        protocol_chain=sources[0].chain,
        network=sources[0].network,
        source_ids=_group_ids(sources),
        return_ids=_group_ids(returns),
        source_wallet_ids=source_wallet_ids,
        destination_wallet_ids=destination_wallet_ids,
        source_wallet_labels=source_labels,
        destination_wallet_labels=destination_labels,
        source_total_msat=source_total,
        source_fee_msat=source_fee,
        source_debit_msat=source_debit,
        return_total_msat=return_total,
        retained_msat=retained,
        residual_msat=residual,
        excess_msat=excess,
        coverage_ppm=coverage_ppm,
        started_at=started.isoformat().replace("+00:00", "Z"),
        ended_at=ended.isoformat().replace("+00:00", "Z"),
        elapsed_seconds=elapsed,
        score=score,
        confidence="strong" if score >= 850 else "moderate" if score >= 750 else "weak",
        reason_codes=reasons,
    )


def _reason_codes(
    sources: Sequence[_Leg],
    returns: Sequence[_Leg],
    *,
    coverage_ppm: int,
    residual: int,
    excess: int,
    elapsed: int,
    source_wallet_ids: tuple[str, ...],
    destination_wallet_ids: tuple[str, ...],
) -> tuple[str, ...]:
    reasons = [
        "amount_coverage_high" if coverage_ppm >= 950_000 else "amount_coverage_partial"
    ]
    if elapsed >= 90 * 86_400:
        reasons.append("long_horizon")
    if len(sources) > 1:
        reasons.append("split_source")
    if len(returns) > 1:
        reasons.append("split_return")
    if set(source_wallet_ids) != set(destination_wallet_ids):
        reasons.append("wallet_transition")
    if residual:
        reasons.append("unresolved_residual")
    if excess:
        reasons.append("return_exceeds_source")
    signal_codes = sorted(
        {code for leg in (*sources, *returns) for code in leg.signal_codes}
    )
    reasons.extend(signal_codes)
    disqualifier_codes = sorted(
        {code for leg in (*sources, *returns) for code in leg.disqualifier_codes}
    )
    reasons.extend(disqualifier_codes)
    return tuple(reasons)


def _score(
    *,
    coverage_ppm: int,
    excess_msat: int,
    source_total_msat: int,
    elapsed_seconds: int,
    different_wallets: bool,
    structured_signal_count: int,
) -> int:
    amount_score = coverage_ppm * 700 // 1_000_000
    days = elapsed_seconds // 86_400
    # A logarithmic penalty ranks near-time candidates higher without ever
    # imposing a historical cutoff.
    time_score = max(25, 200 - 10 * (days + 1).bit_length())
    wallet_score = 50 if different_wallets else 0
    evidence_score = min(80, structured_signal_count * 40)
    excess_ppm = excess_msat * 1_000_000 // source_total_msat
    excess_penalty = min(150, excess_ppm * 150 // 250_000)
    total = amount_score + time_score + wallet_score + evidence_score - excess_penalty
    return max(0, min(1_000, total))


def _stamp_conflicts(
    candidates: Sequence[CustodyGapCandidate],
) -> list[CustodyGapCandidate]:
    parent = {candidate.gap_id: candidate.gap_id for candidate in candidates}

    def find(node: str) -> str:
        while parent[node] != node:
            parent[node] = parent[parent[node]]
            node = parent[node]
        return node

    def union(left: str, right: str) -> None:
        left_root = find(left)
        right_root = find(right)
        if left_root == right_root:
            return
        first, second = sorted((left_root, right_root))
        parent[second] = first

    by_leg: dict[str, list[str]] = {}
    for candidate in candidates:
        for leg_id in (*candidate.source_ids, *candidate.return_ids):
            by_leg.setdefault(leg_id, []).append(candidate.gap_id)
    for siblings in by_leg.values():
        for sibling in siblings[1:]:
            union(siblings[0], sibling)

    sizes: dict[str, int] = {}
    for candidate in candidates:
        root = find(candidate.gap_id)
        sizes[root] = sizes.get(root, 0) + 1
    return [
        replace(
            candidate,
            conflict_set_id=find(candidate.gap_id),
            conflict_size=sizes[find(candidate.gap_id)],
        )
        for candidate in candidates
    ]


def _stamp_promotion_eligibility(
    candidates: Sequence[CustodyGapCandidate], *, required_margin: int
) -> list[CustodyGapCandidate]:
    by_cluster: dict[str, list[CustodyGapCandidate]] = {}
    for candidate in candidates:
        by_cluster.setdefault(candidate.conflict_set_id, []).append(candidate)

    ranked_scores = {
        cluster: sorted(
            ((candidate.score, candidate.gap_id) for candidate in siblings),
            reverse=True,
        )
        for cluster, siblings in by_cluster.items()
    }
    stamped: list[CustodyGapCandidate] = []
    for candidate in candidates:
        ranking = ranked_scores[candidate.conflict_set_id]
        competitor_score = next(
            (score for score, gap_id in ranking if gap_id != candidate.gap_id),
            None,
        )
        margin = (
            candidate.score - competitor_score
            if competitor_score is not None
            else None
        )
        structured_signal = any(
            reason.startswith("structured_")
            and reason != "structured_external_origin"
            for reason in candidate.reason_codes
        )
        disqualified = "structured_external_origin" in candidate.reason_codes
        clear_margin = margin is None or margin >= required_margin
        eligible = structured_signal and not disqualified and clear_margin
        reasons = list(candidate.reason_codes)
        if eligible:
            reasons.append("promotion_eligible_structured_signal")
        elif disqualified:
            reasons.append("promotion_ineligible_external_origin")
        elif not structured_signal:
            reasons.append("search_hint_only")
        else:
            reasons.append("competitor_margin_insufficient")
        stamped.append(
            replace(
                candidate,
                promotion_eligible=eligible,
                competitor_score_margin=margin,
                reason_codes=tuple(reasons),
            )
        )
    return stamped


def _snapshot_gap(candidate: CustodyGapCandidate, rows: Sequence[_Leg]) -> dict[str, Any]:
    destination_wallets = set(candidate.destination_wallet_ids)
    ended = parse_iso_datetime_or_none(candidate.ended_at)
    affected = [
        leg
        for leg in rows
        if leg.direction == "outbound"
        and leg.wallet_id in destination_wallets
        and ended is not None
        and leg.occurred_dt > ended
    ]
    return {
        "gap_id": candidate.gap_id,
        # A conflict remains a review item, but must not look like a solo
        # suggestion merely because the competing candidate is off screen.
        "status": "conflicting" if candidate.conflict_size > 1 else "needs_review",
        "asset": candidate.asset,
        "source_wallet_label": " + ".join(candidate.source_wallet_labels),
        "destination_wallet_labels": list(candidate.destination_wallet_labels),
        "source_total_msat": candidate.source_total_msat,
        "source_fee_msat": candidate.source_fee_msat,
        "source_debit_msat": candidate.source_debit_msat,
        "return_total_msat": candidate.return_total_msat,
        "retained_msat": candidate.retained_msat,
        "residual_msat": candidate.residual_msat,
        "excess_msat": candidate.excess_msat,
        "started_at": candidate.started_at,
        "ended_at": candidate.ended_at,
        "confidence": candidate.confidence,
        "reason_codes": list(candidate.reason_codes),
        "promotion_eligible": candidate.promotion_eligible,
        "competitor_score_margin": candidate.competitor_score_margin,
        "downstream": {
            "affected_disposals": len(affected),
            "affected_years": sorted({leg.occurred_dt.year for leg in affected}),
        },
    }


def _claimed_transaction_ids(
    conn,
    profile_id: str,
    *,
    include_journal_claims: bool,
) -> set[str]:
    claimed: set[str] = set()
    # A normal Kassiber DB has both tables.  Keeping the reads separate makes
    # this helper usable against narrow schema fixtures during migrations.
    try:
        rows = conn.execute(
            """
            SELECT out_transaction_id AS transaction_id
            FROM transaction_pairs WHERE profile_id = ? AND deleted_at IS NULL
            UNION
            SELECT in_transaction_id AS transaction_id
            FROM transaction_pairs WHERE profile_id = ? AND deleted_at IS NULL
            """,
            (profile_id, profile_id),
        ).fetchall()
        claimed.update(str(_get(row, "transaction_id") or row[0]) for row in rows)
    except sqlite3.OperationalError:
        pass
    if include_journal_claims:
        try:
            # Automatic descriptor/graph ownership paths are derived rather
            # than authored in transaction_pairs. Only a current projection may
            # suppress their boundaries from the live gap matcher.
            rows = conn.execute(
                """
                SELECT DISTINCT transaction_id
                FROM journal_entries
                WHERE profile_id = ?
                  AND entry_type IN ('transfer_out', 'transfer_in')
                """,
                (profile_id,),
            ).fetchall()
            claimed.update(str(_get(row, "transaction_id") or row[0]) for row in rows)
        except sqlite3.OperationalError:
            pass
    try:
        rows = conn.execute(
            """
            SELECT out_transaction_id AS transaction_id
            FROM direct_swap_payouts
            WHERE profile_id = ? AND deleted_at IS NULL
            """,
            (profile_id,),
        ).fetchall()
        claimed.update(str(_get(row, "transaction_id") or row[0]) for row in rows)
    except sqlite3.OperationalError:
        pass
    try:
        rows = conn.execute(
            """
            SELECT COALESCE(l.transaction_id, l.anchor_transaction_id) AS transaction_id
            FROM custody_component_legs l
            JOIN custody_components c ON c.id = l.component_id
            WHERE c.profile_id = ? AND c.state = 'active'
            """,
            (profile_id,),
        ).fetchall()
        claimed.update(str(_get(row, "transaction_id") or row[0]) for row in rows if row[0])
    except sqlite3.OperationalError:
        pass
    return claimed


def _journal_status(conn, profile_id: str) -> str:
    """Return the same fail-closed freshness state used by report readiness."""

    try:
        profile = conn.execute(
            """
            SELECT last_processed_at, last_processed_tx_count,
                   journal_input_version, last_processed_input_version
            FROM profiles WHERE id = ?
            """,
            (profile_id,),
        ).fetchone()
        active_count = int(
            conn.execute(
                "SELECT COUNT(*) FROM transactions "
                "WHERE profile_id = ? AND excluded = 0",
                (profile_id,),
            ).fetchone()[0]
        )
    except sqlite3.OperationalError:
        return "not_processed"
    if profile is None:
        return "not_processed"
    if active_count == 0:
        return "no_transactions"
    if not profile["last_processed_at"]:
        return "not_processed"
    if int(profile["last_processed_tx_count"] or 0) != active_count:
        return "stale"
    if int(profile["journal_input_version"] or 0) != int(
        profile["last_processed_input_version"] or 0
    ):
        return "stale"
    return "current"


def _validate_limits(**limits: int) -> None:
    for name, value in limits.items():
        if type(value) is not int or value < 0:
            raise ValueError(f"{name} must be a non-negative integer")
    if not 1 <= limits["min_coverage_ppm"] <= 1_000_000:
        raise ValueError("min_coverage_ppm must be between 1 and 1000000")
    for name in (
        "max_input_rows",
        "max_source_legs",
        "max_return_legs",
        "max_aggregate_return_legs",
        "max_source_groups",
        "max_return_pool",
        "beam_width",
        "max_return_groups_per_source",
        "max_candidates",
        "return_era_gap_seconds",
    ):
        if limits[name] < 1:
            raise ValueError(f"{name} must be at least 1")


def _dedupe_states(
    states: Sequence[tuple[tuple[_Leg, ...], int]],
) -> list[tuple[tuple[_Leg, ...], int]]:
    deduped: dict[tuple[str, ...], tuple[tuple[_Leg, ...], int]] = {}
    for state in states:
        deduped[_group_ids(state[0])] = state
    return list(deduped.values())


def _beam_key(item: tuple[tuple[_Leg, ...], int], target: int) -> tuple[Any, ...]:
    group, total = item
    # Empty state is retained so every later row may start a group.
    if not group:
        return (0, target, 0, ())
    return (1, abs(target - total), len(group), _group_ids(group))


def custody_gap_id(
    profile_id: str,
    asset: str,
    source_ids: tuple[str, ...],
    return_ids: tuple[str, ...],
) -> str:
    """Return the stable content identity for one candidate boundary."""

    if not profile_id or not asset or not source_ids or not return_ids:
        raise ValueError("custody gap ids require profile, asset, source, and return ids")
    payload = "\x1f".join(
        (profile_id, asset, *sorted(source_ids), "->", *sorted(return_ids))
    )
    return "cg_" + hashlib.sha256(payload.encode("utf-8")).hexdigest()[:24]


def _candidate_sort_key(candidate: CustodyGapCandidate) -> tuple[Any, ...]:
    return (
        -candidate.score,
        -candidate.coverage_ppm,
        -candidate.source_total_msat,
        candidate.elapsed_seconds,
        candidate.gap_id,
    )


def _leg_sort_key(leg: _Leg) -> tuple[Any, ...]:
    return (leg.occurred_dt, leg.id)


def _group_ids(group: Sequence[_Leg]) -> tuple[str, ...]:
    return tuple(sorted(leg.id for leg in group))


def _group_end(group: Sequence[_Leg]) -> datetime:
    return max(leg.occurred_dt for leg in group)


def _exact_positive_int(value: Any) -> int | None:
    parsed = _exact_nonnegative_int(value)
    return parsed if parsed is not None and parsed > 0 else None


def _exact_nonnegative_int(value: Any) -> int | None:
    if type(value) is int and value >= 0:
        return value
    if isinstance(value, str) and value.isdigit():
        return int(value)
    return None


def _truthy(value: Any) -> bool:
    return value is True or value == 1 or str(value or "").strip().lower() in {"true", "yes", "on"}


def _get(record: Mapping[str, Any], key: str, default: Any = None) -> Any:
    if isinstance(record, dict):
        return record.get(key, default)
    keys = record.keys() if hasattr(record, "keys") else ()
    return record[key] if key in keys else default
