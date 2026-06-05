from __future__ import annotations

import json
from typing import Any, Mapping

from ..errors import AppError
from ..util import str_or_none

PRIVACY_HOP_TYPES = frozenset(
    {
        "coinjoin",
        "payjoin",
        "payment_in_coinjoin",
        "sweep",
    }
)

PRIVACY_BOUNDARY_FIELD = "privacy_boundary"
PRIVACY_BOUNDARY_RAW_KEYS = (
    PRIVACY_BOUNDARY_FIELD,
    "privacy_boundary_kind",
    "privacyBoundary",
    "privacyBoundaryKind",
    "privacy_hop",
    "privacy_hop_kind",
    "privacyHop",
    "privacyHopKind",
)
LIKELY_COINJOIN_KEYS = (
    "islikelycoinjoin",
    "is_likely_coinjoin",
    "isLikelyCoinJoin",
    "likely_coinjoin",
    "likelyCoinJoin",
)


def normalize_privacy_boundary(value: Any) -> str | None:
    text = str_or_none(value)
    if text is None:
        return None
    normalized = text.strip().lower().replace("-", "_")
    if normalized in PRIVACY_HOP_TYPES:
        return normalized
    return None


def privacy_boundary_from_import_record(record: Mapping[str, Any]) -> str | None:
    for key in PRIVACY_BOUNDARY_RAW_KEYS:
        boundary = _boundary_from_explicit_field(record, key)
        if boundary:
            return boundary
    for key in LIKELY_COINJOIN_KEYS:
        if _boolish(record.get(key)):
            return "coinjoin"
    raw_json = record.get("raw_json")
    payload = _raw_payload(raw_json)
    if payload is None:
        return None
    for key in PRIVACY_BOUNDARY_RAW_KEYS:
        boundary = _boundary_from_explicit_field(payload, key)
        if boundary:
            return boundary
    for key in LIKELY_COINJOIN_KEYS:
        if _boolish(payload.get(key)):
            return "coinjoin"
    return None


def privacy_hop_evidence_from_row(row: Mapping[str, Any]) -> dict[str, Any] | None:
    boundary = normalize_privacy_boundary(_mapping_get(row, PRIVACY_BOUNDARY_FIELD))
    if boundary is None:
        return None
    payload = _raw_payload(_mapping_get(row, "raw_json")) or {}
    return {
        "privacy_boundary": boundary,
        "privacy_hop": boundary,
        "source": payload.get("source") or "",
        "islikelycoinjoin": any(_boolish(payload.get(key)) for key in LIKELY_COINJOIN_KEYS),
        "required_for": "explicit_user_owned_provenance",
    }


def privacy_hop_type_from_row(row: Mapping[str, Any]) -> str | None:
    evidence = privacy_hop_evidence_from_row(row)
    if evidence is None:
        return None
    return str(evidence["privacy_hop"])


def _boundary_from_explicit_field(payload: Mapping[str, Any], key: str) -> str | None:
    if key not in payload:
        return None
    value = payload.get(key)
    text = str_or_none(value)
    if text is None:
        return None
    boundary = normalize_privacy_boundary(text)
    if boundary is not None:
        return boundary
    supported = ", ".join(sorted(PRIVACY_HOP_TYPES))
    raise AppError(
        f"Unsupported privacy boundary '{text}'",
        code="validation",
        hint=f"Supported privacy boundaries: {supported}.",
    )


def _mapping_get(row: Mapping[str, Any], key: str) -> Any:
    if hasattr(row, "keys") and key not in row.keys():
        return None
    if hasattr(row, "get"):
        return row.get(key)
    return row[key]


def _raw_payload(raw_json: Any) -> Mapping[str, Any] | None:
    if isinstance(raw_json, Mapping):
        return raw_json
    if not raw_json:
        return None
    try:
        payload = json.loads(raw_json)
    except (TypeError, ValueError, json.JSONDecodeError):
        return None
    if not isinstance(payload, Mapping):
        return None
    return payload


def _boolish(value: Any) -> bool:
    if value is True:
        return True
    if value is False or value in (None, ""):
        return False
    return str(value).strip().lower() in {"1", "true", "yes", "on"}
