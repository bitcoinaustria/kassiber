from __future__ import annotations

import json
import unittest
from typing import Any, Iterable


TIER3_LINKAGE_IDENTIFIER_KEYS = frozenset(
    {
        "external_id",
        "fingerprint",
        "outpoint",
        "txid",
    }
)


def _json_keys(value: Any) -> set[str]:
    if isinstance(value, dict):
        keys = {str(key) for key in value}
        for item in value.values():
            keys.update(_json_keys(item))
        return keys
    if isinstance(value, list):
        keys: set[str] = set()
        for item in value:
            keys.update(_json_keys(item))
        return keys
    return set()


def assert_tier3_linkage_identifiers_absent(
    case: unittest.TestCase,
    payload: Any,
    *,
    forbidden_values: Iterable[str],
) -> None:
    """Assert an AI/export-safe privacy payload has no raw linkage handles."""

    serialized = json.dumps(payload, sort_keys=True)
    for value in forbidden_values:
        if value:
            case.assertNotIn(value, serialized)
    case.assertFalse(TIER3_LINKAGE_IDENTIFIER_KEYS & _json_keys(payload))
