"""Auto-pair rules for the swap-candidate matcher.

A *rule* declares: "candidates that look like X should be paired as
``kind=Y`` with ``policy=Z`` automatically." Once a heavy user has
manually paired the same Phoenix→Liquid swap shape a handful of times,
this module promotes that pattern to a rule so future identical
candidates get auto-paired without per-row clicking.

Boundaries:

* Pure functions, no SQLite, no logging. Callers feed in
  :class:`~kassiber.core.transfer_matching.SwapCandidate` instances and
  parsed rule records, receive a partition into ``(auto_paired,
  remaining)`` plus a separate "detected pattern" surface for the UI's
  *Create rule from pattern* prompt.
* Specificity is explicit: the rule with the most non-default predicate
  fields wins. Ties are broken by ``id`` so the result is deterministic.
* ``min_confidence`` gates whether a rule applies to heuristic
  candidates. Default ``"strong"`` covers both exact and heuristic;
  ``"exact"`` restricts auto-pairing to deterministic links, such as
  payment hashes, provider swap metadata, or refund funding links.

The auto-pair output is *suggestions*, not writes. The caller decides
whether to commit them — typically through the consent flow on the
``ui.transfers.bulk_pair`` daemon kind, which gives the user one
confirm step rather than per-row consent.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Iterable, Mapping, Optional, Sequence

from ..util import parse_bool
from .transfer_matching import (
    CONFIDENCE_EXACT,
    CONFIDENCE_STRONG,
    SwapCandidate,
)


_PREDICATE_FIELDS = (
    "out_wallet_id",
    "in_wallet_id",
    "out_wallet_kind",
    "in_wallet_kind",
    "out_asset",
    "in_asset",
    "max_fee_pct",
    "min_confidence",
)

DEFAULT_MIN_CONFIDENCE = CONFIDENCE_STRONG
PATTERN_MIN_OCCURRENCES = 3


@dataclass(frozen=True)
class SwapMatchingRule:
    """Materialised swap-matching rule.

    ``predicate`` is the decoded JSON ``predicate_json`` from the
    ``swap_matching_rules`` table. The shape is a flat mapping with any
    of: ``out_wallet_id``, ``in_wallet_id``, ``out_wallet_kind``,
    ``in_wallet_kind``, ``out_asset``, ``in_asset``, ``max_fee_pct``,
    ``min_confidence``. All fields are optional; an empty predicate
    matches every candidate (useful for a single global rule).
    """

    id: str
    profile_id: str
    name: Optional[str]
    predicate: Mapping
    kind: str
    policy: str
    enabled: bool


@dataclass(frozen=True)
class RuleMatch:
    """One candidate auto-paired by a rule."""

    rule_id: str
    rule_name: Optional[str]
    candidate: SwapCandidate


@dataclass(frozen=True)
class PatternSuggestion:
    """A repeating shape in the manual-pair history worth promoting to a rule."""

    out_wallet_id: Optional[str]
    in_wallet_id: Optional[str]
    out_asset: Optional[str]
    in_asset: Optional[str]
    kind: str
    policy: str
    occurrences: int

    def to_predicate(self) -> dict:
        predicate: dict = {}
        if self.out_wallet_id:
            predicate["out_wallet_id"] = self.out_wallet_id
        if self.in_wallet_id:
            predicate["in_wallet_id"] = self.in_wallet_id
        if self.out_asset:
            predicate["out_asset"] = self.out_asset
        if self.in_asset:
            predicate["in_asset"] = self.in_asset
        return predicate


def load_rule(record: Mapping) -> SwapMatchingRule:
    """Decode one ``swap_matching_rules`` row into a typed dataclass.

    Tolerant of missing / malformed ``predicate_json`` — empty / invalid
    blobs decode to ``{}`` so the rule still parses (and matches
    everything, which is the user's stated intent when they save it).
    """
    raw_predicate = _record_get(record, "predicate_json") or "{}"
    try:
        predicate = json.loads(raw_predicate) if isinstance(raw_predicate, str) else dict(raw_predicate)
    except (TypeError, ValueError, json.JSONDecodeError):
        predicate = {}
    if not isinstance(predicate, dict):
        predicate = {}
    raw_enabled = _record_get(record, "enabled")
    return SwapMatchingRule(
        id=str(_record_get(record, "id") or ""),
        profile_id=str(_record_get(record, "profile_id") or ""),
        name=_record_get(record, "name"),
        predicate=predicate,
        kind=str(_record_get(record, "kind") or "manual"),
        policy=str(_record_get(record, "policy") or "carrying-value"),
        enabled=parse_bool(raw_enabled, default=True),
    )


def predicate_matches(candidate: SwapCandidate, predicate: Mapping) -> bool:
    """``True`` when every non-default predicate field matches the candidate.

    Empty fields are treated as wildcards. ``max_fee_pct`` caps the
    fractional principal delta (``swap_fee_msat / out_amount_msat``);
    ``min_confidence`` requires the candidate's confidence to meet or
    beat the threshold (``exact`` always passes).
    """
    for field in _PREDICATE_FIELDS:
        expected = predicate.get(field)
        if expected in (None, ""):
            continue
        if field == "max_fee_pct":
            if candidate.out_amount_msat <= 0:
                return False
            actual_pct = abs(candidate.swap_fee_msat) / candidate.out_amount_msat
            if actual_pct > float(expected):
                return False
            continue
        if field == "min_confidence":
            if not _confidence_meets(candidate.confidence, str(expected)):
                return False
            continue
        actual = getattr(candidate, field, None)
        if actual != expected:
            return False
    return True


def rule_specificity(rule: SwapMatchingRule) -> int:
    """Higher = more specific. More non-default predicate keys = higher score."""
    return sum(
        1
        for field in _PREDICATE_FIELDS
        if rule.predicate.get(field) not in (None, "")
    )


def apply_rules(
    candidates: Sequence[SwapCandidate],
    rules: Sequence[SwapMatchingRule],
) -> tuple[list[RuleMatch], list[SwapCandidate]]:
    """Partition candidates into ``(auto_paired, remaining)``.

    A candidate is auto-paired by the first applicable rule when
    sorted by descending specificity (ties broken by rule id). Disabled
    rules are skipped; candidates in a conflict cluster larger than one
    are intentionally NOT auto-paired — the user must disambiguate.
    Conflict membership comes from the matcher-stamped ``conflict_size``
    so a filtered candidate list cannot make a cluster member look solo.
    """
    enabled = [rule for rule in rules if rule.enabled]
    enabled.sort(key=lambda rule: (-rule_specificity(rule), rule.id))

    auto_paired: list[RuleMatch] = []
    remaining: list[SwapCandidate] = []
    for candidate in candidates:
        if candidate.conflict_size > 1:
            remaining.append(candidate)
            continue
        match = None
        for rule in enabled:
            min_confidence = rule.predicate.get("min_confidence") or DEFAULT_MIN_CONFIDENCE
            if not _confidence_meets(candidate.confidence, str(min_confidence)):
                continue
            if predicate_matches(candidate, rule.predicate):
                match = rule
                break
        if match is None:
            remaining.append(candidate)
            continue
        auto_paired.append(
            RuleMatch(
                rule_id=match.id,
                rule_name=match.name,
                candidate=candidate,
            )
        )
    return auto_paired, remaining


def detect_repeating_patterns(
    pair_history: Iterable[Mapping],
    *,
    min_occurrences: int = PATTERN_MIN_OCCURRENCES,
) -> list[PatternSuggestion]:
    """Find wallet-pair / asset-pair shapes that have been manually paired
    at least ``min_occurrences`` times.

    Each pair_history record must expose ``out_wallet_id``,
    ``in_wallet_id``, ``out_asset``, ``in_asset``, ``kind``, ``policy``,
    ``pair_source``. Only records with ``pair_source`` of ``"manual"``
    feed pattern detection — bulk-paired and rule-paired rows do not
    count, otherwise rules would propagate themselves indefinitely.
    """
    buckets: dict[tuple, dict] = {}
    for record in pair_history:
        if (_record_get(record, "pair_source") or "manual") != "manual":
            continue
        out_wallet = _record_get(record, "out_wallet_id")
        in_wallet = _record_get(record, "in_wallet_id")
        out_asset = _record_get(record, "out_asset")
        in_asset = _record_get(record, "in_asset")
        kind = _record_get(record, "kind") or "manual"
        policy = _record_get(record, "policy") or "carrying-value"
        key = (out_wallet, in_wallet, out_asset, in_asset, kind, policy)
        bucket = buckets.setdefault(
            key,
            {
                "out_wallet_id": out_wallet,
                "in_wallet_id": in_wallet,
                "out_asset": out_asset,
                "in_asset": in_asset,
                "kind": kind,
                "policy": policy,
                "occurrences": 0,
            },
        )
        bucket["occurrences"] += 1
    suggestions = [
        PatternSuggestion(
            out_wallet_id=bucket["out_wallet_id"],
            in_wallet_id=bucket["in_wallet_id"],
            out_asset=bucket["out_asset"],
            in_asset=bucket["in_asset"],
            kind=bucket["kind"],
            policy=bucket["policy"],
            occurrences=bucket["occurrences"],
        )
        for bucket in buckets.values()
        if bucket["occurrences"] >= min_occurrences
    ]
    suggestions.sort(key=lambda s: (-s.occurrences, s.kind, str(s.out_wallet_id), str(s.in_wallet_id)))
    return suggestions


def _confidence_meets(actual: str, required: str) -> bool:
    """``exact`` always satisfies ``strong``; everything matches itself."""
    if actual == required:
        return True
    if required == CONFIDENCE_STRONG and actual == CONFIDENCE_EXACT:
        return True
    return False


def _record_get(record: Mapping, key: str):
    if hasattr(record, "keys") and not isinstance(record, dict):
        keys = record.keys()
        if hasattr(keys, "__contains__") and key in keys:
            return record[key]
        return None
    if isinstance(record, dict):
        return record.get(key)
    return getattr(record, key, None)
