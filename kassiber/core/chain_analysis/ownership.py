"""Private inventory branch evidence, normalized once at observation ingestion."""
from __future__ import annotations

from typing import Any

from ...wallet_descriptors import SCRIPT_TYPE_BRANCH_BASE

_BRANCH_ROLES = {value + offset: role for value in SCRIPT_TYPE_BRANCH_BASE.values()
                 for offset, role in ((0, "receive"), (1, "change"))}
_SCRIPT_TOKENS = {token for kind in SCRIPT_TYPE_BRANCH_BASE
                  for token in (kind.replace("-", ""), *kind.split("-"))}


def branch_evidence(label: Any, index: Any) -> dict[str, str]:
    normalized = "".join(ch if ch.isalnum() else " " for ch in str(label or "").strip().lower())
    tokens = {*normalized.split(), normalized.replace(" ", "")}
    role = "change" if tokens & {"change", "internal"} else "receive" if tokens & {"receive", "external", "deposit"} else None
    if role:
        imported = bool(tokens & _SCRIPT_TOKENS)
        return {"branch_role": role, "branch_evidence_level": "exact",
                "change_evidence": "imported" if imported else "ground_truth",
                "branch_source": "imported_branch_role" if imported else "wallet_branch_role"}
    try:
        number = int(index) if not isinstance(index, bool) and str(index).strip() == str(int(index)) else None
    except (ValueError, TypeError, OverflowError):
        number = None
    role = _BRANCH_ROLES.get(number)
    return {"branch_role": role or "unknown", "branch_evidence_level": "derived" if role else "unknown",
            "change_evidence": "heuristic" if role else "unavailable",
            "branch_source": "numeric_branch_convention" if role else "branch_metadata_unavailable"}
