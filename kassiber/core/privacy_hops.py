from __future__ import annotations

import json
from typing import Any, Mapping

PRIVACY_HOP_TYPES = frozenset(
    {
        "coinjoin",
        "payjoin",
        "payment_in_coinjoin",
        "sweep",
    }
)


def privacy_hop_evidence_from_raw(raw_json: Any) -> dict[str, Any] | None:
    payload = _raw_payload(raw_json)
    if payload is None:
        return None
    hop = str(payload.get("privacy_hop") or "").strip().lower()
    if hop not in PRIVACY_HOP_TYPES:
        if payload.get("islikelycoinjoin") is True:
            hop = "coinjoin"
        else:
            return None
    return {
        "privacy_hop": hop,
        "source": payload.get("source") or "",
        "islikelycoinjoin": bool(payload.get("islikelycoinjoin")),
        "required_for": "explicit_user_owned_provenance",
    }


def privacy_hop_type_from_raw(raw_json: Any) -> str | None:
    evidence = privacy_hop_evidence_from_raw(raw_json)
    if evidence is None:
        return None
    return str(evidence["privacy_hop"])


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
