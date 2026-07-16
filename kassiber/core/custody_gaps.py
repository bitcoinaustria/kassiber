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

from bisect import bisect_left, bisect_right
import base64
import binascii
from dataclasses import dataclass, replace
from datetime import datetime
import hashlib
import json
import sqlite3
from typing import Any, Iterable, Mapping, Sequence

from . import custody_quantity_store as core_custody_quantity_store
from .custody_evidence import normalize_boundary_amounts, resolve_protocol_scope

from ..errors import AppError
from ..time_utils import now_iso, parse_iso_datetime_or_none


DEFAULT_MIN_COVERAGE_PPM = 800_000
DEFAULT_MAX_EXCESS_PPM = 250_000
# Above this many eligible rows, discovery deliberately stops producing weak
# amount/time-only hints and processes every typed privacy/Samourai boundary
# plus a bounded tail of ordinary sources.  It is a worklist threshold, not a
# global abort: structured boundaries must remain visible in large books.
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
MAX_RETAINED_GAP_PROJECTIONS = 8

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


class CustodyGapSearchLimitError(ValueError):
    """Advisory search stopped at a configured capacity ceiling.

    Capacity alone says nothing about custody or tax classification. Consumers
    may show an incomplete-search warning, but must never turn the exception
    into a global report blocker. ``blocking_source_ids`` is narrower: typed
    privacy-boundary evidence plus incomplete source discovery requires suspense
    for those exact sources only.
    """

    def __init__(
        self,
        message: str,
        *,
        candidate_count: int | None = None,
        promotion_eligible_count: int | None = None,
        limit_kind: str = "capacity",
        partial_candidates: Sequence[Any] = (),
        accounting_candidates: Sequence[Any] = (),
        normalized_legs: Sequence[Any] = (),
        blocking_source_ids: Iterable[str] = (),
    ) -> None:
        super().__init__(message)
        self.candidate_count = candidate_count
        self.promotion_eligible_count = promotion_eligible_count
        self.limit_kind = limit_kind
        self.partial_candidates = tuple(partial_candidates)
        # The UI prefix stays bounded by ``max_candidates``.  Canonical
        # accounting must nevertheless retain every already-scored structured
        # candidate: dropping one merely because the display queue is full
        # would let its boundary rows fall back into RP2.
        self.accounting_candidates = tuple(accounting_candidates)
        self.normalized_legs = tuple(normalized_legs)
        self.blocking_source_ids = tuple(
            sorted({str(item) for item in blocking_source_ids if item})
        )
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
class CustodyGapSearchResult:
    """Ordinary result for a complete or capacity-bounded advisory search."""

    candidates: tuple[CustodyGapCandidate, ...]
    accounting_candidates: tuple[CustodyGapCandidate, ...]
    search_complete: bool
    limit_kind: str | None = None
    candidate_count: int = 0
    promotion_eligible_count: int = 0
    blocking_source_ids: tuple[str, ...] = ()
    message: str | None = None
    projection_id: str | None = None


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


@dataclass(frozen=True)
class _ReturnEra:
    legs: tuple[_Leg, ...]
    total_msat: int


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
    worklist_limited = len(legs) > max_input_rows
    capacity_limited_candidate_ids: set[str] = set()
    capacity_limited_search = worklist_limited
    blocking_source_ids: set[str] = set()

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
        source_group_count = sum(
            max(0, len(sources) - size + 1)
            for size in range(1, max_source_legs + 1)
        )
        source_worklist_limited = source_group_count > max_source_groups
        if worklist_limited or source_worklist_limited:
            # Weak amount/time hints are useful on small books, but they must
            # not make a million-row book quadratic. Typed boundaries always
            # remain in the worklist; skipped ordinary hints keep the search
            # explicitly incomplete rather than creating a global empty cliff.
            structured = [leg for leg in sources if leg.signal_codes]
            blocking_source_ids.update(leg.id for leg in structured)
            structured_worklist = structured[:max_source_groups]
            ordinary_budget = max_source_groups - len(structured_worklist)
            ordinary = [leg for leg in sources if not leg.signal_codes]
            source_ids = {
                leg.id
                for leg in (
                    *structured_worklist,
                    *(ordinary[-ordinary_budget:] if ordinary_budget else ()),
                )
            }
            sources = [leg for leg in sources if leg.id in source_ids]
            capacity_limited_search = True
            if not sources:
                continue
        return_keys = [_leg_sort_key(leg) for leg in returns]
        returns_by_amount = (
            sorted(returns, key=lambda leg: (leg.principal_msat, *_leg_sort_key(leg)))
            if worklist_limited
            else []
        )
        return_eras = (
            _return_eras(returns, era_gap_seconds=return_era_gap_seconds)
            if worklist_limited
            else []
        )
        source_groups = _source_groups(
            sources,
            # Filtering a large worklist destroys original adjacency. Never
            # manufacture N:M source groups across the omitted history; exact
            # singletons remain reviewable and the search stays incomplete.
            max_legs=(
                1 if worklist_limited or source_worklist_limited else max_source_legs
            ),
            max_groups=max_source_groups,
        )
        for source_group in source_groups:
            boundary = max(leg.occurred_dt for leg in source_group)
            target = sum(leg.principal_msat for leg in source_group)
            first_return = bisect_right(return_keys, (boundary, "\uffff"))
            if first_return >= len(returns):
                continue
            if worklist_limited:
                return_pool = _indexed_return_pool(
                    returns_by_amount,
                    boundary=boundary,
                    target=target,
                    maximum=max_return_pool,
                )
                return_pool_limited = True
                eligible_returns: Sequence[_Leg] = ()
            else:
                eligible_returns = returns[first_return:]
                return_pool_limited = len(eligible_returns) > max_return_pool
                if return_pool_limited:
                    capacity_limited_search = True
                    blocking_source_ids.update(
                        leg.id for leg in source_group if leg.signal_codes
                    )
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
            aggregate_limited = False
            try:
                if worklist_limited:
                    aggregate_groups = _matching_return_eras(
                        return_eras,
                        boundary=boundary,
                        target=target,
                        min_coverage_ppm=min_coverage_ppm,
                        max_excess_ppm=max_excess_ppm,
                        max_legs=max_aggregate_return_legs,
                        result_limit=max_return_groups_per_source,
                    )
                else:
                    aggregate_groups = _wallet_era_return_groups(
                        eligible_returns,
                        target=target,
                        min_coverage_ppm=min_coverage_ppm,
                        max_excess_ppm=max_excess_ppm,
                        max_legs=max_aggregate_return_legs,
                        era_gap_seconds=return_era_gap_seconds,
                        result_limit=max_return_groups_per_source,
                    )
            except CustodyGapSearchLimitError:
                # One over-large wallet era must not erase candidates already
                # proved for unrelated source boundaries. The resulting source
                # stays explicitly capacity-limited and therefore review-only.
                aggregate_limited = True
                capacity_limited_search = True
                blocking_source_ids.update(
                    leg.id for leg in source_group if leg.signal_codes
                )
                aggregate_groups = []
            return_groups = _dedupe_groups((*return_groups, *aggregate_groups))
            for return_group in return_groups:
                candidate = _build_candidate(source_group, return_group)
                if candidate.score < 650:
                    continue
                generated[candidate.gap_id] = candidate
                if (
                    worklist_limited
                    or source_worklist_limited
                    or return_pool_limited
                    or aggregate_limited
                ):
                    capacity_limited_candidate_ids.add(candidate.gap_id)
            if source_worklist_limited and len(generated) > max_candidates:
                break

    # Stamp conflicts and promotion eligibility before applying the display
    # ceiling. The exception carries a fully scored deterministic prefix plus
    # ``blocking=False`` / ``search_complete=False``. Candidate discovery is
    # advisory and capacity alone is never accounting evidence.
    stamped = _stamp_conflicts(list(generated.values()))
    stamped = [
        replace(
            candidate,
            reason_codes=tuple(
                (*candidate.reason_codes, "search_capacity_incomplete")
            ),
        )
        if candidate.gap_id in capacity_limited_candidate_ids
        else candidate
        for candidate in stamped
    ]
    stamped = _stamp_promotion_eligibility(
        stamped, required_margin=promotion_score_margin
    )
    stamped = [
        replace(
            candidate,
            promotion_eligible=False,
            reason_codes=tuple(
                code
                for code in candidate.reason_codes
                if code != "promotion_eligible_structured_signal"
            )
            + ("capacity_source_suspense_required",),
        )
        if candidate.gap_id in capacity_limited_candidate_ids
        and candidate.promotion_eligible
        else candidate
        for candidate in stamped
    ]
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
            accounting_candidates=tuple(
                candidate for candidate in stamped if candidate.promotion_eligible
            ),
            blocking_source_ids=blocking_source_ids,
        )
    if capacity_limited_search or capacity_limited_candidate_ids:
        promotion_count = sum(candidate.promotion_eligible for candidate in stamped)
        raise CustodyGapSearchLimitError(
            "custody-gap search used a bounded source/return worklist; "
            "structured boundaries remain reviewable but discovery is incomplete",
            candidate_count=len(stamped),
            promotion_eligible_count=promotion_count,
            limit_kind="boundary_worklist",
            partial_candidates=stamped,
            accounting_candidates=tuple(
                candidate for candidate in stamped if candidate.promotion_eligible
            ),
            blocking_source_ids=blocking_source_ids,
        )
    return stamped


def search_custody_gap_candidates(
    rows: Sequence[Mapping[str, Any]],
    **kwargs: Any,
) -> CustodyGapSearchResult:
    """Return advisory candidates and explicit search-completeness metadata.

    This is the production boundary. The legacy list/exception API remains
    temporarily available to callers outside the canonical journal seam.
    """

    try:
        candidates = tuple(suggest_custody_gap_candidates(rows, **kwargs))
    except CustodyGapSearchLimitError as exc:
        visible = tuple(
            item
            for item in exc.partial_candidates
            if isinstance(item, CustodyGapCandidate)
        )
        accounting = tuple(
            item
            for item in exc.accounting_candidates
            if isinstance(item, CustodyGapCandidate)
        )
        return CustodyGapSearchResult(
            candidates=visible,
            accounting_candidates=accounting,
            search_complete=False,
            limit_kind=exc.limit_kind,
            candidate_count=(
                int(exc.candidate_count)
                if exc.candidate_count is not None
                else len(visible)
            ),
            promotion_eligible_count=(
                int(exc.promotion_eligible_count)
                if exc.promotion_eligible_count is not None
                else sum(item.promotion_eligible for item in visible)
            ),
            blocking_source_ids=tuple(exc.blocking_source_ids),
            message=str(exc),
        )
    return CustodyGapSearchResult(
        candidates=candidates,
        accounting_candidates=tuple(
            item for item in candidates if item.promotion_eligible
        ),
        search_complete=True,
        candidate_count=len(candidates),
        promotion_eligible_count=sum(
            item.promotion_eligible for item in candidates
        ),
    )


def _read_projection_page(
    conn: sqlite3.Connection,
    profile_id: str,
    projection_id: str,
    *,
    limit: int,
    after: tuple[int, int, str] | None = None,
    gap_id: str | None = None,
) -> dict[str, Any]:
    header = conn.execute(
        "SELECT version_json, summary_json, display_ready, display_context "
        "FROM custody_gap_candidate_projections "
        "WHERE id = ? AND profile_id = ?",
        (projection_id, profile_id),
    ).fetchone()
    if header is None or not bool(header["display_ready"]):
        raise ValueError("cursor is unknown or expired; reload the first page")
    if tuple(json.loads(header["version_json"])) != _gap_snapshot_version(
        conn, profile_id
    ):
        raise ValueError("cursor expired because custody evidence changed")
    journal_status = _journal_status(conn, profile_id)
    if str(header["display_context"]) != _display_context(
        conn, profile_id, journal_status
    ):
        raise ValueError("cursor expired because journal readiness changed")
    params: list[Any] = [projection_id]
    where = "projection_id = ?"
    if gap_id is not None:
        where += " AND gap_id = ?"
        params.append(gap_id)
    elif after is not None:
        sort_group, ordinal, last_gap_id = after
        where += (
            " AND (sort_group > ? OR "
            "(sort_group = ? AND ordinal > ?) OR "
            "(sort_group = ? AND ordinal = ? AND gap_id > ?))"
        )
        params.extend(
            (sort_group, sort_group, ordinal, sort_group, ordinal, last_gap_id)
        )
    rows = conn.execute(
        "SELECT sort_group, ordinal, gap_id, payload_json "
        "FROM custody_gap_projection_rows WHERE "
        + where
        + " ORDER BY sort_group, ordinal, gap_id LIMIT ?",
        (*params, limit + 1),
    ).fetchall()
    visible = rows[:limit]
    next_cursor = None
    if len(rows) > limit and visible:
        last = visible[-1]
        next_cursor = _encode_projection_cursor(
            projection_id,
            int(last["sort_group"]),
            int(last["ordinal"]),
            str(last["gap_id"]),
        )
    return {
        "summary": dict(json.loads(header["summary_json"])),
        "gaps": [dict(json.loads(row["payload_json"])) for row in visible],
        "next_cursor": next_cursor,
    }


def _store_projection_display(
    conn: sqlite3.Connection,
    profile_id: str,
    projection_id: str,
    *,
    display_context: str,
    summary: Mapping[str, Any],
    gaps: Sequence[Mapping[str, Any]],
) -> None:
    owns_transaction = not conn.in_transaction
    savepoint = "custody_gap_display_write"
    conn.execute(f"SAVEPOINT {savepoint}")
    try:
        conn.execute(
            "DELETE FROM custody_gap_projection_rows WHERE projection_id = ?",
            (projection_id,),
        )
        group_ordinals = [0, 0]
        for gap in gaps:
            sort_group = (
                0 if gap.get("status") in {"needs_review", "conflicting"} else 1
            )
            ordinal = group_ordinals[sort_group]
            group_ordinals[sort_group] += 1
            conn.execute(
                """
                INSERT INTO custody_gap_projection_rows(
                    projection_id, profile_id, sort_group, ordinal,
                    gap_id, payload_json
                ) VALUES (?, ?, ?, ?, ?, ?)
                """,
                (
                    projection_id,
                    profile_id,
                    sort_group,
                    ordinal,
                    str(gap.get("gap_id") or ""),
                    json.dumps(gap, sort_keys=True, separators=(",", ":")),
                ),
            )
        conn.execute(
            "UPDATE custody_gap_candidate_projections "
            "SET summary_json = ?, display_ready = 1, display_context = ? "
            "WHERE id = ?",
            (
                json.dumps(summary, sort_keys=True, separators=(",", ":")),
                display_context,
                projection_id,
            ),
        )
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
        if owns_transaction:
            conn.commit()
    except Exception:
        conn.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
        if owns_transaction:
            conn.rollback()
        raise


def _display_context(
    conn: sqlite3.Connection,
    profile_id: str,
    journal_status: str,
) -> str:
    """Version presentation rows without invalidating candidate evidence."""

    try:
        review_version = conn.execute(
            """
            SELECT COUNT(*), COALESCE(MAX(revision), 0),
                   COALESCE(MAX(created_at), '')
            FROM custody_gap_reviews WHERE profile_id = ?
            """,
            (profile_id,),
        ).fetchone()
        review_parts = (
            int(review_version[0]),
            int(review_version[1]),
            str(review_version[2]),
        )
    except sqlite3.OperationalError:
        review_parts = (0, 0, "")
    return json.dumps([journal_status, *review_parts], separators=(",", ":"))


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
    raw transaction graphs, or wallet configuration. It reads the shared
    normalized candidate projection and materializes privacy-safe rows so
    subsequent keyset pages neither rerun nor reorder discovery; this cache is
    replaceable and never authored evidence.
    ``summary.search_complete`` says whether the bounded population is complete;
    ``gaps`` is the requested page and ``next_cursor`` continues that
    deterministic order.
    """

    if not isinstance(profile_id, str) or not profile_id.strip():
        raise ValueError("profile_id is required")
    if type(limit) is not int or limit < 1 or limit > 500:
        raise ValueError("limit must be an integer between 1 and 500")
    if cursor is not None and not isinstance(cursor, str):
        raise ValueError("cursor must be a string")
    # The CLI historically used ``--cursor 0`` as its explicit first-page
    # sentinel. Later pages are opaque/version-bound; retain only this neutral
    # sentinel so scripted review flows do not need a migration alias.
    if cursor == "0":
        cursor = None
    if gap_id is not None and cursor is not None:
        raise ValueError("cursor is not supported when gap_id is provided")

    if cursor is not None:
        projection_id, sort_group, ordinal, last_gap_id = (
            _decode_projection_cursor(cursor)
        )
        return _read_projection_page(
            conn,
            profile_id,
            projection_id,
            limit=limit,
            after=(sort_group, ordinal, last_gap_id),
        )

    journal_status = _journal_status(conn, profile_id)
    # Page size must not change matcher semantics. Every page is cut from the
    # same bounded advisory population; an incomplete population is labelled
    # explicitly rather than represented as a global accounting failure.
    search_result, normalized = load_gap_search_result(
        conn,
        profile_id,
        limit=DEFAULT_MAX_CANDIDATES,
        include_journal_claims=journal_status == "current",
    )
    projection_id = search_result.projection_id
    if projection_id is None:
        raise RuntimeError("custody gap search did not return a projection id")
    display_context = _display_context(conn, profile_id, journal_status)
    display = conn.execute(
        "SELECT display_ready, display_context "
        "FROM custody_gap_candidate_projections WHERE id = ?",
        (projection_id,),
    ).fetchone()
    if (
        display is not None
        and bool(display["display_ready"])
        and str(display["display_context"]) == display_context
    ):
        return _read_projection_page(
            conn,
            profile_id,
            projection_id,
            limit=limit,
            gap_id=gap_id,
        )
    candidates = list(search_result.candidates)
    from . import custody_gap_reviews

    reviews = custody_gap_reviews.latest_reviews(conn, profile_id)
    current_gaps: list[dict[str, Any]] = []
    for candidate in candidates:
        gap = _snapshot_gap(candidate, normalized)
        gap["candidate_fingerprint"] = custody_gap_reviews.candidate_fingerprint(
            candidate
        )
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
        conn,
        profile_id,
        exclude_gap_ids=[candidate.gap_id for candidate in candidates],
    )
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
    summary = {
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
        "search_complete": search_result.search_complete,
        "search_status": (
            "complete" if search_result.search_complete else "capacity_limited"
        ),
        "search_limit_kind": search_result.limit_kind,
        "search_candidate_count": (
            len(candidates)
            if search_result.search_complete
            else search_result.candidate_count
        ),
    }
    _store_projection_display(
        conn,
        profile_id,
        projection_id,
        display_context=display_context,
        summary=summary,
        gaps=all_gaps,
    )
    return _read_projection_page(
        conn,
        profile_id,
        projection_id,
        limit=limit,
        gap_id=gap_id,
    )


def _gap_snapshot_version(conn, profile_id: str) -> tuple[Any, ...]:
    profile = conn.execute(
        """
        SELECT journal_input_version
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
    if profile is None:
        return (active_count, 0)
    return (
        active_count,
        int(profile["journal_input_version"] or 0),
    )


def _projection_identity(
    profile_id: str,
    version: tuple[Any, ...],
    ignored_ids: Sequence[str],
    accounting_ignored_ids: Sequence[str],
    rows: Sequence[Mapping[str, Any]],
) -> tuple[str, str, str]:
    version_json = json.dumps(version, separators=(",", ":"))
    evidence_rows = []
    for row in rows:
        raw_json = _get(row, "raw_json", "")
        if isinstance(raw_json, Mapping):
            raw_json = json.dumps(raw_json, sort_keys=True, separators=(",", ":"))
        evidence_rows.append(
            (
                str(_get(row, "id") or ""),
                str(_get(row, "wallet_id") or ""),
                str(_get(row, "occurred_at") or ""),
                str(_get(row, "direction") or ""),
                str(_get(row, "asset") or ""),
                _get(row, "amount_msat", _get(row, "amount")),
                _get(row, "fee", 0),
                bool(_get(row, "amount_includes_fee", False)),
                bool(_get(row, "excluded", False)),
                str(_get(row, "kind") or ""),
                str(_get(row, "privacy_boundary") or ""),
                str(_get(row, "chain") or ""),
                str(_get(row, "network") or ""),
                str(
                    _get(
                        row,
                        "wallet_config_json",
                        _get(row, "config_json", "{}"),
                    )
                    or "{}"
                ),
                str(raw_json or ""),
            )
        )
    input_json = json.dumps(
        [tuple(ignored_ids), tuple(accounting_ignored_ids), evidence_rows],
        ensure_ascii=False,
        separators=(",", ":"),
    )
    input_hash = hashlib.sha256(input_json.encode()).hexdigest()
    material = json.dumps(
        ["custody-gap-projection-v1", profile_id, version_json, input_hash],
        separators=(",", ":"),
    )
    return hashlib.sha256(material.encode()).hexdigest(), version_json, input_hash


def _load_matching_journal_projection(
    conn: sqlite3.Connection,
    profile_id: str,
    version: tuple[Any, ...],
    rows: Sequence[Mapping[str, Any]],
) -> CustodyGapSearchResult | None:
    """Reuse the builder's authoritative ignored-boundary population."""

    version_json = json.dumps(version, separators=(",", ":"))
    try:
        headers = conn.execute(
            """
            SELECT id, ignored_ids_json, accounting_ignored_ids_json
            FROM custody_gap_candidate_projections
            WHERE profile_id = ? AND version_json = ?
              AND producer_kind = 'journal'
            ORDER BY rowid DESC
            LIMIT 1
            """,
            (profile_id, version_json),
        ).fetchall()
    except sqlite3.OperationalError:
        return None
    for header in headers:
        ignored = tuple(sorted(str(item) for item in json.loads(header[1])))
        accounting_ignored = tuple(
            sorted(str(item) for item in json.loads(header[2]))
        )
        projection_id, _version_json, _input_hash = _projection_identity(
            profile_id,
            version,
            ignored,
            accounting_ignored,
            rows,
        )
        if projection_id == str(header[0]):
            return _load_candidate_projection(conn, projection_id)
    return None


def _candidate_from_row(
    row: Mapping[str, Any],
    boundaries: Mapping[tuple[str, str], tuple[str, ...]],
) -> CustodyGapCandidate:
    gap_id = str(_get(row, "gap_id") or "")
    return CustodyGapCandidate(
        gap_id=gap_id,
        profile_id=str(_get(row, "profile_id") or ""),
        asset=str(_get(row, "asset") or ""),
        protocol_chain=str(_get(row, "protocol_chain") or ""),
        network=str(_get(row, "network") or ""),
        source_ids=boundaries.get((gap_id, "source"), ()),
        return_ids=boundaries.get((gap_id, "return"), ()),
        source_wallet_ids=tuple(json.loads(_get(row, "source_wallet_ids_json"))),
        destination_wallet_ids=tuple(
            json.loads(_get(row, "destination_wallet_ids_json"))
        ),
        source_wallet_labels=tuple(
            json.loads(_get(row, "source_wallet_labels_json"))
        ),
        destination_wallet_labels=tuple(
            json.loads(_get(row, "destination_wallet_labels_json"))
        ),
        source_total_msat=int(_get(row, "source_total_msat")),
        source_fee_msat=int(_get(row, "source_fee_msat")),
        source_debit_msat=int(_get(row, "source_debit_msat")),
        return_total_msat=int(_get(row, "return_total_msat")),
        retained_msat=int(_get(row, "retained_msat")),
        residual_msat=int(_get(row, "residual_msat")),
        excess_msat=int(_get(row, "excess_msat")),
        coverage_ppm=int(_get(row, "coverage_ppm")),
        started_at=str(_get(row, "started_at")),
        ended_at=str(_get(row, "ended_at")),
        elapsed_seconds=int(_get(row, "elapsed_seconds")),
        score=int(_get(row, "score")),
        confidence=str(_get(row, "confidence")),
        reason_codes=tuple(json.loads(_get(row, "reason_codes_json"))),
        promotion_eligible=bool(_get(row, "promotion_eligible")),
        competitor_score_margin=_get(row, "competitor_score_margin"),
        conflict_set_id=str(_get(row, "conflict_set_id") or ""),
        conflict_size=int(_get(row, "conflict_size")),
    )


def _load_candidate_projection(
    conn: sqlite3.Connection,
    projection_id: str,
) -> CustodyGapSearchResult | None:
    try:
        header = conn.execute(
            "SELECT * FROM custody_gap_candidate_projections WHERE id = ?",
            (projection_id,),
        ).fetchone()
    except sqlite3.OperationalError:
        return None
    if header is None:
        return None
    rows = conn.execute(
        "SELECT * FROM custody_gap_candidates WHERE projection_id = ? "
        "ORDER BY ordinal, gap_id",
        (projection_id,),
    ).fetchall()
    boundary_rows = conn.execute(
        "SELECT gap_id, side, transaction_id FROM custody_gap_candidate_boundaries "
        "WHERE projection_id = ? ORDER BY gap_id, side, ordinal",
        (projection_id,),
    ).fetchall()
    boundaries: dict[tuple[str, str], list[str]] = {}
    for boundary in boundary_rows:
        boundaries.setdefault((str(boundary[0]), str(boundary[1])), []).append(
            str(boundary[2])
        )
    frozen = {key: tuple(values) for key, values in boundaries.items()}
    if any(
        len(frozen.get((str(row["gap_id"]), "source"), ()))
        != int(row["source_count"])
        or len(frozen.get((str(row["gap_id"]), "return"), ()))
        != int(row["return_count"])
        for row in rows
    ):
        # Transaction retraction cascades normalized boundary relations. A
        # header without its committed cardinality is an invalid derived
        # projection, never permission to reconstruct a partial candidate.
        return None
    candidates = tuple(_candidate_from_row(row, frozen) for row in rows)
    return CustodyGapSearchResult(
        candidates=tuple(item for row, item in zip(rows, candidates) if bool(row["visible"])),
        accounting_candidates=tuple(
            item for row, item in zip(rows, candidates) if bool(row["accounting"])
        ),
        search_complete=bool(header["search_complete"]),
        limit_kind=header["limit_kind"],
        candidate_count=int(header["candidate_count"]),
        promotion_eligible_count=int(header["promotion_eligible_count"]),
        blocking_source_ids=tuple(json.loads(header["blocking_source_ids_json"])),
        message=header["message"],
        projection_id=projection_id,
    )


def _persist_candidate_projection(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    projection_id: str,
    version_json: str,
    input_hash: str,
    ignored_ids: Sequence[str],
    accounting_ignored_ids: Sequence[str],
    producer_kind: str,
    result: CustodyGapSearchResult,
) -> CustodyGapSearchResult:
    visible_ids = {item.gap_id for item in result.candidates}
    accounting_ids = {item.gap_id for item in result.accounting_candidates}
    candidates = {
        item.gap_id: item
        for item in (*result.candidates, *result.accounting_candidates)
    }
    owns_transaction = not conn.in_transaction
    savepoint = "custody_gap_projection_write"
    conn.execute(f"SAVEPOINT {savepoint}")
    try:
        conn.execute(
            """
            INSERT OR IGNORE INTO custody_gap_candidate_projections(
                id, profile_id, producer_kind, version_json, input_hash,
                ignored_ids_json, accounting_ignored_ids_json, search_complete,
                limit_kind, candidate_count, promotion_eligible_count,
                blocking_source_ids_json, message, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                projection_id,
                profile_id,
                producer_kind,
                version_json,
                input_hash,
                json.dumps(ignored_ids, separators=(",", ":")),
                json.dumps(accounting_ignored_ids, separators=(",", ":")),
                int(result.search_complete),
                result.limit_kind,
                result.candidate_count,
                result.promotion_eligible_count,
                json.dumps(result.blocking_source_ids, separators=(",", ":")),
                result.message,
                now_iso(),
            ),
        )
        if producer_kind == "journal":
            # A review search may have reached the same population first. The
            # builder upgrades that derived header so current UI/AI reads can
            # consume its exact ignored-boundary authority.
            conn.execute(
                "UPDATE custody_gap_candidate_projections "
                "SET producer_kind = 'journal', ignored_ids_json = ?, "
                "accounting_ignored_ids_json = ? "
                "WHERE id = ?",
                (
                    json.dumps(ignored_ids, separators=(",", ":")),
                    json.dumps(accounting_ignored_ids, separators=(",", ":")),
                    projection_id,
                ),
            )
        for ordinal, candidate in enumerate(candidates.values()):
            conn.execute(
                """
                INSERT OR IGNORE INTO custody_gap_candidates(
                    projection_id, profile_id, gap_id, ordinal,
                    source_count, return_count, visible, accounting,
                    asset, protocol_chain, network, source_wallet_ids_json,
                    destination_wallet_ids_json, source_wallet_labels_json,
                    destination_wallet_labels_json, source_total_msat, source_fee_msat,
                    source_debit_msat, return_total_msat, retained_msat, residual_msat,
                    excess_msat, coverage_ppm, started_at, ended_at, elapsed_seconds,
                    score, confidence, reason_codes_json, promotion_eligible,
                    competitor_score_margin, conflict_set_id, conflict_size
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                          ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    projection_id, profile_id, candidate.gap_id, ordinal,
                    len(candidate.source_ids), len(candidate.return_ids),
                    int(candidate.gap_id in visible_ids),
                    int(candidate.gap_id in accounting_ids), candidate.asset,
                    candidate.protocol_chain, candidate.network,
                    json.dumps(candidate.source_wallet_ids, separators=(",", ":")),
                    json.dumps(candidate.destination_wallet_ids, separators=(",", ":")),
                    json.dumps(candidate.source_wallet_labels, separators=(",", ":")),
                    json.dumps(candidate.destination_wallet_labels, separators=(",", ":")),
                    candidate.source_total_msat, candidate.source_fee_msat,
                    candidate.source_debit_msat, candidate.return_total_msat,
                    candidate.retained_msat, candidate.residual_msat,
                    candidate.excess_msat, candidate.coverage_ppm,
                    candidate.started_at, candidate.ended_at, candidate.elapsed_seconds,
                    candidate.score, candidate.confidence,
                    json.dumps(candidate.reason_codes, separators=(",", ":")),
                    int(candidate.promotion_eligible), candidate.competitor_score_margin,
                    candidate.conflict_set_id, candidate.conflict_size,
                ),
            )
            for side, transaction_ids in (
                ("source", candidate.source_ids),
                ("return", candidate.return_ids),
            ):
                conn.executemany(
                    "INSERT OR IGNORE INTO custody_gap_candidate_boundaries("
                    "projection_id, profile_id, gap_id, side, ordinal, transaction_id) "
                    "VALUES (?, ?, ?, ?, ?, ?)",
                    [
                        (
                            projection_id,
                            profile_id,
                            candidate.gap_id,
                            side,
                            index,
                            transaction_id,
                        )
                        for index, transaction_id in enumerate(transaction_ids)
                    ],
                )
        conn.execute(
            """
            DELETE FROM custody_gap_candidate_projections
            WHERE profile_id = ?
              AND id NOT IN (
                  SELECT id FROM custody_gap_candidate_projections
                  WHERE profile_id = ?
                  ORDER BY rowid DESC
                  LIMIT ?
              )
              AND id NOT IN (
                  SELECT id FROM custody_gap_candidate_projections
                  WHERE profile_id = ? AND producer_kind = 'journal'
                  ORDER BY rowid DESC
                  LIMIT 1
              )
            """,
            (
                profile_id,
                profile_id,
                MAX_RETAINED_GAP_PROJECTIONS,
                profile_id,
            ),
        )
        # Serialized page populations are derived and have no rollback/audit
        # value. Clear the legacy cache after the normalized replacement is
        # safely present; the table remains only so older databases can open.
        conn.execute(
            "DELETE FROM custody_gap_candidate_snapshots WHERE profile_id = ?",
            (profile_id,),
        )
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
        if owns_transaction:
            conn.commit()
    except Exception:
        conn.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
        if owns_transaction:
            conn.rollback()
        raise
    loaded = _load_candidate_projection(conn, projection_id)
    if loaded is None:
        raise RuntimeError("custody gap projection did not persist")
    return loaded


def _encode_projection_cursor(
    projection_id: str, sort_group: int, ordinal: int, gap_id: str
) -> str:
    payload = json.dumps(
        [projection_id, sort_group, ordinal, gap_id], separators=(",", ":")
    ).encode()
    return "cgp1." + base64.urlsafe_b64encode(payload).decode().rstrip("=")


def _decode_projection_cursor(cursor: str) -> tuple[str, int, int, str]:
    if not cursor.startswith("cgp1."):
        raise ValueError("cursor is malformed")
    encoded = cursor[5:]
    try:
        payload = json.loads(
            base64.urlsafe_b64decode(encoded + "=" * (-len(encoded) % 4))
        )
    except (ValueError, TypeError, json.JSONDecodeError, binascii.Error) as exc:
        raise ValueError("cursor is malformed") from exc
    if (
        not isinstance(payload, list)
        or len(payload) != 4
        or not isinstance(payload[0], str)
        or len(payload[0]) != 64
        or payload[1] not in (0, 1)
        or type(payload[2]) is not int
        or payload[2] < 0
        or not isinstance(payload[3], str)
    ):
        raise ValueError("cursor is malformed")
    return payload[0], payload[1], payload[2], payload[3]


def load_gap_search_result(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    limit: int = DEFAULT_MAX_CANDIDATES,
    include_journal_claims: bool | None = None,
    ignored_transaction_ids: Iterable[str] | None = None,
    accounting_ignored_transaction_ids: Iterable[str] | None = None,
    producer_kind: str = "review",
    persist_projection: bool = True,
) -> tuple[CustodyGapSearchResult, list[_Leg]]:
    """Load candidates with ordinary search-completeness metadata."""

    if producer_kind not in {"journal", "review"}:
        raise ValueError("producer_kind must be journal or review")

    row_count = conn.execute(
        "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile_id,),
    ).fetchone()[0]
    row_select = """
        SELECT t.id, t.profile_id, t.wallet_id, w.label AS wallet_label,
               w.kind AS wallet_kind, t.occurred_at, t.direction, t.asset,
               t.amount, t.fee, t.amount_includes_fee, t.excluded,
               t.kind, t.privacy_boundary, t.external_id, t.raw_json
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
    """
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

    def mapped_rows(raw_rows: Sequence[sqlite3.Row]) -> list[dict[str, Any]]:
        output: list[dict[str, Any]] = []
        for raw_row in raw_rows:
            mapped = {key: raw_row[key] for key in raw_row.keys()}
            mapped["wallet_config_json"] = wallet_configs.get(
                str(mapped["wallet_id"]), "{}"
            )
            output.append(mapped)
        return output

    large_book = row_count > DEFAULT_MAX_INPUT_ROWS
    if not large_book:
        raw_rows = conn.execute(
            row_select
            + """
            WHERE t.profile_id = ? AND t.excluded = 0
            ORDER BY t.occurred_at, t.created_at, t.id
            """,
            (profile_id,),
        ).fetchall()
        rows = mapped_rows(raw_rows)
    else:
        privacy = tuple(sorted(_PRIVACY_BOUNDARIES))
        wallet_kinds = tuple(sorted(_SAMOURAI_WALLET_KINDS))
        transaction_kinds = tuple(sorted(_SAMOURAI_TRANSACTION_KINDS))
        structured_sql = f"""
               LOWER(COALESCE(t.privacy_boundary, '')) IN ({','.join('?' for _ in privacy)})
            OR LOWER(COALESCE(w.kind, '')) IN ({','.join('?' for _ in wallet_kinds)})
            OR LOWER(COALESCE(t.kind, '')) IN ({','.join('?' for _ in transaction_kinds)})
            OR LOWER(COALESCE(w.config_json, '')) LIKE '%\"samourai\"%'
        """
        structured_params = (*privacy, *wallet_kinds, *transaction_kinds)
        source_rows = mapped_rows(
            conn.execute(
                row_select
                + f"""
                WHERE t.profile_id = ? AND t.excluded = 0
                  AND t.direction = 'outbound'
                  AND ({structured_sql})
                ORDER BY t.occurred_at, t.created_at, t.id
                LIMIT ?
                """,
                (
                    profile_id,
                    *structured_params,
                    DEFAULT_MAX_SOURCE_GROUPS,
                ),
            ).fetchall()
        )
        ordinary_budget = DEFAULT_MAX_SOURCE_GROUPS - len(source_rows)
        if ordinary_budget:
            # A missing intermediary often leaves no typed marker on the old
            # wallet's payment. Preserve a bounded high-value lane for that
            # 10 BTC out / 9.9 BTC back shape. These remain review-only because
            # the large-book search is explicitly incomplete.
            source_rows.extend(
                mapped_rows(
                    conn.execute(
                        row_select
                        + f"""
                        WHERE t.profile_id = ? AND t.excluded = 0
                          AND t.direction = 'outbound'
                          AND NOT ({structured_sql})
                        ORDER BY t.amount DESC, t.occurred_at, t.created_at, t.id
                        LIMIT ?
                        """,
                        (profile_id, *structured_params, ordinary_budget),
                    ).fetchall()
                )
            )
        source_legs = [
            leg
            for row in source_rows
            if (leg := _normalize_leg(row, set())) is not None
        ]
        amount_ranges: dict[str, tuple[int, int]] = {}
        for leg in source_legs:
            minimum_total = (
                leg.principal_msat * DEFAULT_MIN_COVERAGE_PPM // 1_000_000
            )
            # This is an advisory per-slot floor, not a completeness proof: a
            # valid uneven group may contain smaller legs. Large-book results
            # therefore stay review-only and explicitly capacity-limited.
            minimum_leg = max(
                1,
                (minimum_total + DEFAULT_MAX_AGGREGATE_RETURN_LEGS - 1)
                // DEFAULT_MAX_AGGREGATE_RETURN_LEGS,
            )
            maximum_total = leg.principal_msat + (
                leg.principal_msat * DEFAULT_MAX_EXCESS_PPM // 1_000_000
            )
            previous = amount_ranges.get(leg.asset)
            amount_ranges[leg.asset] = (
                minimum_leg if previous is None else min(previous[0], minimum_leg),
                maximum_total if previous is None else max(previous[1], maximum_total),
            )
        return_rows: list[dict[str, Any]] = []
        if amount_ranges:
            range_sql = " OR ".join(
                "(t.asset = ? AND t.amount BETWEEN ? AND ?)"
                for _asset in sorted(amount_ranges)
            )
            range_params = tuple(
                value
                for asset in sorted(amount_ranges)
                for value in (asset, *amount_ranges[asset])
            )
            return_rows = mapped_rows(
                conn.execute(
                    row_select
                    + f"""
                    WHERE t.profile_id = ? AND t.excluded = 0
                      AND t.direction = 'inbound'
                      AND ({range_sql})
                    ORDER BY t.occurred_at, t.created_at, t.id
                    LIMIT ?
                    """,
                    (profile_id, *range_params, DEFAULT_MAX_INPUT_ROWS),
                ).fetchall()
            )
        rows = [*source_rows, *return_rows]
        rows.sort(
            key=lambda row: (
                str(row.get("occurred_at") or ""),
                str(row.get("id") or ""),
            )
        )
    normalized = [
        leg for row in rows if (leg := _normalize_leg(row, set())) is not None
    ]
    if include_journal_claims is None:
        include_journal_claims = _journal_status(conn, profile_id) == "current"
    version = _gap_snapshot_version(conn, profile_id)
    if ignored_transaction_ids is None and include_journal_claims:
        journal_projection = _load_matching_journal_projection(
            conn,
            profile_id,
            version,
            rows,
        )
        if journal_projection is not None:
            return journal_projection, normalized
    claimed_ids = (
        {str(item) for item in ignored_transaction_ids if item}
        if ignored_transaction_ids is not None
        else _claimed_transaction_ids(
            conn,
            profile_id,
            include_journal_claims=include_journal_claims,
        )
    )
    ignored = tuple(sorted(claimed_ids))
    accounting_ignored = (
        ignored
        if accounting_ignored_transaction_ids is None
        else tuple(
            sorted(
                {
                    str(item)
                    for item in accounting_ignored_transaction_ids
                    if item
                }
            )
        )
    )
    projection_id, version_json, input_hash = _projection_identity(
        profile_id,
        version,
        ignored,
        accounting_ignored,
        rows,
    )
    cached = _load_candidate_projection(conn, projection_id)
    if cached is not None:
        if producer_kind == "journal" and persist_projection:
            cached = _persist_candidate_projection(
                conn,
                profile_id=profile_id,
                projection_id=projection_id,
                version_json=version_json,
                input_hash=input_hash,
                ignored_ids=ignored,
                accounting_ignored_ids=accounting_ignored,
                producer_kind=producer_kind,
                result=cached,
            )
        return cached, normalized
    worklist_threshold = DEFAULT_MAX_INPUT_ROWS if not large_book else 1
    result = search_custody_gap_candidates(
        rows,
        ignored_ids=ignored,
        max_input_rows=worklist_threshold,
        max_candidates=limit,
    )
    if large_book and result.search_complete:
        result = replace(
            result,
            accounting_candidates=(),
            search_complete=False,
            limit_kind="boundary_worklist",
            promotion_eligible_count=0,
            message="custody-gap search used a bounded large-book worklist",
        )
    if accounting_ignored != ignored:
        accounting_result = search_custody_gap_candidates(
            rows,
            ignored_ids=accounting_ignored,
            max_input_rows=worklist_threshold,
            max_candidates=limit,
        )
        if large_book and accounting_result.search_complete:
            accounting_result = replace(
                accounting_result,
                accounting_candidates=(),
                search_complete=False,
                limit_kind="boundary_worklist",
                promotion_eligible_count=0,
                message="custody-gap search used a bounded large-book worklist",
            )
        result = replace(
            result,
            accounting_candidates=accounting_result.accounting_candidates,
            promotion_eligible_count=(
                accounting_result.promotion_eligible_count
            ),
            blocking_source_ids=accounting_result.blocking_source_ids,
        )
    if persist_projection:
        result = _persist_candidate_projection(
            conn,
            profile_id=profile_id,
            projection_id=projection_id,
            version_json=version_json,
            input_hash=input_hash,
            ignored_ids=ignored,
            accounting_ignored_ids=accounting_ignored,
            producer_kind=producer_kind,
            result=result,
        )
    return result, normalized


def load_gap_candidates(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    limit: int = DEFAULT_MAX_CANDIDATES,
    include_journal_claims: bool | None = None,
) -> tuple[list[CustodyGapCandidate], list[_Leg]]:
    """Compatibility wrapper for the former list/exception API."""

    result, normalized = load_gap_search_result(
        conn,
        profile_id,
        limit=limit,
        include_journal_claims=include_journal_claims,
    )
    if not result.search_complete:
        raise CustodyGapSearchLimitError(
            result.message or "custody-gap search is incomplete",
            candidate_count=result.candidate_count,
            promotion_eligible_count=result.promotion_eligible_count,
            limit_kind=result.limit_kind or "capacity",
            partial_candidates=result.candidates,
            accounting_candidates=result.accounting_candidates,
            normalized_legs=normalized,
            blocking_source_ids=result.blocking_source_ids,
        )
    return list(result.candidates), normalized


def find_gap_candidate(
    conn: sqlite3.Connection,
    profile_id: str,
    gap_id: str,
    *,
    persist_projection: bool = True,
) -> CustodyGapCandidate:
    result, _normalized = load_gap_search_result(
        conn, profile_id, persist_projection=persist_projection
    )
    # A fully scored prefix remains safe to review even though the advisory
    # queue is incomplete. The action still re-derives and fingerprints the
    # exact candidate; this does not imply anything about omitted hints.
    for candidate in result.candidates:
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
    boundary = normalize_boundary_amounts(
        direction=direction,
        amount_msat=amount,
        fee_msat=fee if direction == "outbound" else 0,
        amount_includes_fee=_truthy(_get(row, "amount_includes_fee")),
    )
    principal = boundary.principal_msat
    fee = boundary.fee_msat
    debit = boundary.wallet_movement_msat
    if principal <= 0:
        return None
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
) -> Iterable[tuple[_Leg, ...]]:
    ordered = sorted(rows, key=_leg_sort_key)
    # Yield chronological partitions instead of materializing one global
    # population. Every singleton and every allowed adjacent group is visited
    # exactly once, so conflict discovery stays complete without the former
    # 87-source all-or-nothing cliff. ``max_groups`` now controls the ordinary
    # source tail selected by the caller when the input worklist is large.
    del max_groups
    for end in range(len(ordered)):
        for size in range(1, min(max_legs, end + 1) + 1):
            yield tuple(ordered[end - size + 1 : end + 1])


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
    if len(ordered) <= maximum:
        return ordered
    # The beam needs plausible individual legs, not the complete suffix. Keep
    # the deterministic amount-nearest rows, then restore chronology for the
    # beam. Wallet-era aggregation still inspects the full suffix separately.
    selected = sorted(
        ordered,
        key=lambda leg: (
            abs(target - leg.principal_msat),
            leg.occurred_dt,
            leg.id,
        ),
    )[:maximum]
    selected.sort(key=_leg_sort_key)
    return selected


def _indexed_return_pool(
    rows_by_amount: Sequence[_Leg],
    *,
    boundary: datetime,
    target: int,
    maximum: int,
) -> list[_Leg]:
    """Select a bounded amount-nearest pool without scanning all returns."""

    pivot = bisect_left(
        rows_by_amount,
        target,
        key=lambda leg: leg.principal_msat,
    )
    left = pivot - 1
    right = pivot
    selected: list[_Leg] = []
    # Rows before the source boundary may occupy the nearest amount buckets.
    # Bound that defensive scan as well; the enclosing large-book search is
    # explicitly incomplete and never presents the sample as universe closure.
    inspected = 0
    inspection_limit = maximum * 8
    while len(selected) < maximum and inspected < inspection_limit:
        if left < 0 and right >= len(rows_by_amount):
            break
        left_distance = (
            abs(target - rows_by_amount[left].principal_msat)
            if left >= 0
            else None
        )
        right_distance = (
            abs(target - rows_by_amount[right].principal_msat)
            if right < len(rows_by_amount)
            else None
        )
        if right_distance is not None and (
            left_distance is None or right_distance < left_distance
        ):
            leg = rows_by_amount[right]
            right += 1
        else:
            leg = rows_by_amount[left]
            left -= 1
        inspected += 1
        if leg.occurred_dt > boundary:
            selected.append(leg)
    selected.sort(key=_leg_sort_key)
    return selected


def _return_eras(
    rows: Sequence[_Leg], *, era_gap_seconds: int
) -> list[_ReturnEra]:
    """Precompute wallet activity eras once for a large-book scope."""

    by_wallet: dict[str, list[_Leg]] = {}
    for row in rows:
        by_wallet.setdefault(row.wallet_id, []).append(row)
    groups: list[tuple[_Leg, ...]] = []
    for wallet_id in sorted(by_wallet):
        ordered = sorted(by_wallet[wallet_id], key=_leg_sort_key)
        current: list[_Leg] = []
        for row in ordered:
            if current and int(
                (row.occurred_dt - current[-1].occurred_dt).total_seconds()
            ) > era_gap_seconds:
                groups.append(tuple(current))
                current = []
            current.append(row)
        if current:
            groups.append(tuple(current))
    eras = [
        _ReturnEra(legs=group, total_msat=sum(row.principal_msat for row in group))
        for group in groups
    ]
    eras.sort(
        key=lambda era: (
            era.total_msat,
            _group_end(era.legs),
            _group_ids(era.legs),
        )
    )
    return eras


def _matching_return_eras(
    eras_by_amount: Sequence[_ReturnEra],
    *,
    boundary: datetime,
    target: int,
    min_coverage_ppm: int,
    max_excess_ppm: int,
    max_legs: int,
    result_limit: int,
) -> list[tuple[_Leg, ...]]:
    minimum = target * min_coverage_ppm // 1_000_000
    maximum = target + (target * max_excess_ppm // 1_000_000)

    start = bisect_left(eras_by_amount, minimum, key=lambda era: era.total_msat)
    end = bisect_right(eras_by_amount, maximum, key=lambda era: era.total_msat)
    matches: list[_ReturnEra] = []
    inspected = 0
    for index in range(start, end):
        inspected += 1
        if inspected > DEFAULT_MAX_RETURN_POOL:
            break
        era = eras_by_amount[index]
        group = era.legs
        if group[0].occurred_dt <= boundary:
            continue
        if len(group) > max_legs:
            raise CustodyGapSearchLimitError(
                "custody-gap wallet/era aggregation needs "
                f"{len(group)} return legs; configured maximum is {max_legs}"
            )
        matches.append(era)
    matches.sort(
        key=lambda era: (
            abs(target - era.total_msat),
            len(era.legs),
            _group_ids(era.legs),
        )
    )
    return [era.legs for era in matches[:result_limit]]


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
        # Narrow migration fixtures may omit authored transaction pairs.
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
            # A reduced schema has no derived journal claims to suppress.
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
        # Older/reduced schemas may not contain direct payout reviews.
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
        # Component tables may be absent in migration and surface fixtures.
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
