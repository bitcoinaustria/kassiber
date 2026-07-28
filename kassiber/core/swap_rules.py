"""Auto-pair rules for the swap-candidate matcher.

A *rule* declares: "candidates that look like X should be paired as
``kind=Y`` with ``policy=Z`` automatically." A heavy user who repeatedly pairs
the same Phoenix→Liquid swap shape by hand writes one rule and future identical
candidates get auto-paired without per-row clicking.

Boundaries:

* Pure functions, no SQLite, no logging. Callers feed in
  :class:`~kassiber.core.transfer_matching.SwapCandidate` instances and
  parsed rule records, and receive a partition into ``(auto_paired,
  remaining)``.
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
import math
from dataclasses import dataclass
from typing import Any, Iterable, Mapping, Optional, Sequence

from ..util import parse_bool
from .custody_review_terms import TRANSFER_PAIR_KINDS, TRANSFER_PAIR_POLICIES
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
_PREDICATE_FIELD_SET = frozenset(_PREDICATE_FIELDS)
_STRING_PREDICATE_FIELDS = (
    "out_wallet_id",
    "in_wallet_id",
    "out_wallet_kind",
    "in_wallet_kind",
    "out_asset",
    "in_asset",
)
_MIN_CONFIDENCE_VALUES = frozenset({CONFIDENCE_STRONG, CONFIDENCE_EXACT})


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
    invalid_reason: Optional[str] = None


@dataclass(frozen=True)
class RuleMatch:
    """One candidate auto-paired by a rule."""

    rule_id: str
    rule_name: Optional[str]
    candidate: SwapCandidate


def load_rule(record: Mapping) -> SwapMatchingRule:
    """Decode one ``swap_matching_rules`` row into a typed dataclass.

    Invalid stored rows fail closed: the rule is force-disabled and the
    ``invalid_reason`` explains which field was malformed.
    """
    raw_predicate = _record_get(record, "predicate_json")
    invalid_messages: list[str] = []
    try:
        predicate = decode_rule_predicate(raw_predicate)
    except ValueError as exc:
        invalid_messages.append(str(exc))
        predicate = {}
    kind = str(_record_get(record, "kind") or "manual")
    if kind not in TRANSFER_PAIR_KINDS:
        invalid_messages.append(f"unsupported kind '{kind}'")
        kind = "manual"
    policy = str(_record_get(record, "policy") or "carrying-value")
    if policy not in TRANSFER_PAIR_POLICIES:
        invalid_messages.append(f"unsupported policy '{policy}'")
        policy = "carrying-value"
    raw_enabled = _record_get(record, "enabled")
    enabled = parse_bool(raw_enabled, default=True)
    if invalid_messages:
        enabled = False
    return SwapMatchingRule(
        id=str(_record_get(record, "id") or ""),
        profile_id=str(_record_get(record, "profile_id") or ""),
        name=_record_get(record, "name"),
        predicate=predicate,
        kind=kind,
        policy=policy,
        enabled=enabled,
        invalid_reason="; ".join(invalid_messages) or None,
    )


def decode_rule_predicate(raw_predicate: Any) -> dict[str, Any]:
    if raw_predicate in (None, ""):
        return {}
    if isinstance(raw_predicate, str):
        try:
            predicate = json.loads(raw_predicate)
        except json.JSONDecodeError as exc:
            raise ValueError("predicate JSON is invalid") from exc
    else:
        predicate = raw_predicate
    return validate_rule_predicate(predicate)


def validate_rule_predicate(predicate: Any) -> dict[str, Any]:
    if not isinstance(predicate, Mapping):
        raise ValueError("predicate must be an object")
    unknown_fields = sorted(
        str(key) for key in predicate.keys() if key not in _PREDICATE_FIELD_SET
    )
    if unknown_fields:
        fields = ", ".join(unknown_fields)
        raise ValueError(f"predicate has unsupported field(s): {fields}")
    normalized: dict[str, Any] = {}
    for field in _STRING_PREDICATE_FIELDS:
        value = predicate.get(field)
        if value in (None, ""):
            continue
        if not isinstance(value, str):
            raise ValueError(f"predicate field '{field}' must be a string")
        normalized[field] = value
    max_fee_pct = predicate.get("max_fee_pct")
    if max_fee_pct not in (None, ""):
        if isinstance(max_fee_pct, bool):
            raise ValueError("predicate field 'max_fee_pct' must be a number")
        try:
            parsed_max_fee_pct = float(max_fee_pct)
        except (TypeError, ValueError) as exc:
            raise ValueError("predicate field 'max_fee_pct' must be a number") from exc
        if not math.isfinite(parsed_max_fee_pct) or parsed_max_fee_pct < 0:
            raise ValueError(
                "predicate field 'max_fee_pct' must be a finite non-negative number"
            )
        normalized["max_fee_pct"] = parsed_max_fee_pct
    min_confidence = predicate.get("min_confidence")
    if min_confidence not in (None, ""):
        if not isinstance(min_confidence, str):
            raise ValueError("predicate field 'min_confidence' must be a string")
        if min_confidence not in _MIN_CONFIDENCE_VALUES:
            allowed = ", ".join(sorted(_MIN_CONFIDENCE_VALUES))
            raise ValueError(
                f"predicate field 'min_confidence' must be one of: {allowed}"
            )
        normalized["min_confidence"] = min_confidence
    return normalized


def predicate_matches(candidate: SwapCandidate, predicate: Mapping) -> bool:
    """``True`` when every non-default predicate field matches the candidate.

    Empty fields are treated as wildcards. ``max_fee_pct`` caps the
    fractional principal delta (``swap_fee_msat / out_amount_msat``);
    ``min_confidence`` requires the candidate's confidence to meet or
    beat the threshold (``exact`` always passes).
    """
    try:
        validated = validate_rule_predicate(predicate)
    except ValueError:
        return False
    for field in _PREDICATE_FIELDS:
        expected = validated.get(field)
        if expected in (None, ""):
            continue
        if field == "max_fee_pct":
            try:
                parsed_expected = float(expected)
            except (TypeError, ValueError):
                return False
            if not math.isfinite(parsed_expected) or parsed_expected < 0:
                return False
            if candidate.out_amount_msat <= 0:
                return False
            actual_pct = abs(candidate.swap_fee_msat) / candidate.out_amount_msat
            if actual_pct > parsed_expected:
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
