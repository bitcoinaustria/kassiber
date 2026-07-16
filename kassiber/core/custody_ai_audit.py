"""Append-only audit records for AI-assisted custody decisions.

Chat persistence is optional and is not accounting evidence.  This module
therefore records the narrow facts around a consented custody write inside the
book's SQLCipher boundary.  Raw proposals stay local and never replicate;
``redacted_audit_summary`` is the only representation intended for an audit
package.
"""

from __future__ import annotations

import hashlib
import json
import sqlite3
import uuid
from collections.abc import Mapping
from collections.abc import Sequence
from typing import Any

from ..time_utils import now_iso


_MAX_REASON_CHARS = 500
_PROPOSAL_KEYS = frozenset(
    {
        "gap_id",
        "expected_fingerprint",
        "reason",
        "classification",
        "economic_kind",
        "notes",
        "residual_action",
    }
)
_CONSENT_DECISIONS = frozenset(
    {"allow_once", "allow_session", "deny", "consent_timeout", "cancelled"}
)
_EXECUTION_STATUSES = frozenset({"executed", "failed", "denied", "cancelled"})


def _canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def _sha256_json(value: Any) -> str:
    return hashlib.sha256(_canonical_json(value).encode("utf-8")).hexdigest()


def _bounded_proposal(raw: Mapping[str, Any]) -> dict[str, Any]:
    """Keep only the closed custody proposal vocabulary and bounded scalars."""

    output: dict[str, Any] = {}
    for key in sorted(_PROPOSAL_KEYS):
        value = raw.get(key)
        if value is None:
            continue
        if isinstance(value, bool):
            output[key] = value
        elif isinstance(value, int):
            output[key] = value
        elif isinstance(value, str):
            output[key] = value[:_MAX_REASON_CHARS]
    return output


def append_assistance_record(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    tool_name: str,
    daemon_kind: str,
    call_id: str,
    provider_kind: str,
    model: str,
    model_proposal: Mapping[str, Any],
    final_proposal: Mapping[str, Any] | None,
    consent_decision: str,
    consent_requested_at: str,
    consent_decided_at: str,
    execution_status: str,
    execution_code: str | None = None,
    result: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Append one immutable custody-assistance record without committing.

    Callers own the surrounding transaction so a successful consent record can
    be committed atomically with the reviewed custody write.
    """

    if consent_decision not in _CONSENT_DECISIONS:
        raise ValueError("unsupported custody AI consent decision")
    if execution_status not in _EXECUTION_STATUSES:
        raise ValueError("unsupported custody AI execution status")

    model_packet = _bounded_proposal(model_proposal)
    final_packet = _bounded_proposal(final_proposal or model_proposal)
    user_edited = final_packet != model_packet
    candidate_fingerprint = final_packet.get("expected_fingerprint")
    gap_id = final_packet.get("gap_id")
    facts_packet = {
        "daemon_kind": daemon_kind,
        "gap_id": gap_id,
        "candidate_fingerprint": candidate_fingerprint,
        "proposal": final_packet,
    }
    result_packet = _bounded_result(result)
    record_id = str(uuid.uuid4())
    created_at = now_iso()
    conn.execute(
        """
        INSERT INTO custody_ai_assistance_audits(
            id, workspace_id, profile_id, tool_name, daemon_kind, call_id,
            provider_kind, model, gap_id, candidate_fingerprint,
            facts_sha256, model_proposal_json, final_proposal_json, user_edited,
            consent_decision, consent_requested_at, consent_decided_at,
            execution_status, execution_code, result_sha256, review_id,
            component_id, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            record_id,
            workspace_id,
            profile_id,
            str(tool_name)[:128],
            str(daemon_kind)[:128],
            str(call_id)[:256],
            str(provider_kind or "unknown")[:64],
            str(model)[:256],
            str(gap_id)[:256] if gap_id is not None else None,
            str(candidate_fingerprint)[:128]
            if candidate_fingerprint is not None
            else None,
            _sha256_json(facts_packet),
            _canonical_json(model_packet),
            _canonical_json(final_packet),
            int(user_edited),
            consent_decision,
            consent_requested_at,
            consent_decided_at,
            execution_status,
            str(execution_code)[:128] if execution_code else None,
            _sha256_json(result_packet) if result is not None else None,
            result_packet.get("review_id"),
            result_packet.get("component_id"),
            created_at,
        ),
    )
    return {
        "id": record_id,
        "facts_sha256": _sha256_json(facts_packet),
        "consent_decision": consent_decision,
        "execution_status": execution_status,
        "review_id": result_packet.get("review_id"),
        "component_id": result_packet.get("component_id"),
        "created_at": created_at,
    }


def _bounded_result(result: Mapping[str, Any] | None) -> dict[str, Any]:
    if not isinstance(result, Mapping):
        return {}
    packet: dict[str, Any] = {}
    for key in (
        "gap_id",
        "status",
        "id",
        "review_id",
        "review_revision",
        "component_id",
        "classification",
        "economic_kind",
    ):
        value = result.get(key)
        if isinstance(value, (str, int)) and not isinstance(value, bool):
            packet[key] = value
    if "review_id" not in packet and isinstance(result.get("id"), str):
        packet["review_id"] = result["id"]
    return packet


def redacted_audit_summary(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    limit: int = 500,
    transaction_ids: Sequence[str] | None = None,
) -> dict[str, Any]:
    """Return hash-only custody AI history suitable for an audit package."""

    bounded_limit = max(1, min(int(limit), 500))
    where = "profile_id = ?"
    params: list[Any] = [profile_id]
    if transaction_ids is not None:
        selected = tuple(sorted({str(item) for item in transaction_ids if str(item)}))
        if not selected:
            return {
                "records": [],
                "count": 0,
                "truncated": False,
                "raw_proposals_included": False,
                "replicated": False,
                "chat_history_required": False,
            }
        placeholders = ",".join("?" for _ in selected)
        where += (
            " AND component_id IN ("
            "SELECT DISTINCT component_id FROM custody_component_legs "
            f"WHERE anchor_transaction_id IN ({placeholders})"
            ")"
        )
        params.extend(selected)
    params.append(bounded_limit + 1)
    rows = conn.execute(
        f"""
        SELECT id, tool_name, daemon_kind, provider_kind, gap_id,
               candidate_fingerprint, facts_sha256, user_edited,
               consent_decision, consent_requested_at, consent_decided_at,
               execution_status, execution_code, result_sha256, review_id,
               component_id, created_at
        FROM custody_ai_assistance_audits
        WHERE {where}
        ORDER BY created_at DESC, id DESC
        LIMIT ?
        """,
        params,
    ).fetchall()
    truncated = len(rows) > bounded_limit
    records = []
    for row in rows[:bounded_limit]:
        records.append(
            {
                "id": row["id"],
                "tool_name": row["tool_name"],
                "daemon_kind": row["daemon_kind"],
                "provider_kind": row["provider_kind"],
                "gap_id": row["gap_id"],
                "candidate_fingerprint": row["candidate_fingerprint"],
                "facts_sha256": row["facts_sha256"],
                "user_edited": bool(row["user_edited"]),
                "consent_decision": row["consent_decision"],
                "consent_requested_at": row["consent_requested_at"],
                "consent_decided_at": row["consent_decided_at"],
                "execution_status": row["execution_status"],
                "execution_code": row["execution_code"],
                "result_sha256": row["result_sha256"],
                "review_id": row["review_id"],
                "component_id": row["component_id"],
                "created_at": row["created_at"],
            }
        )
    return {
        "records": records,
        "count": len(records),
        "truncated": truncated,
        "raw_proposals_included": False,
        "replicated": False,
        "chat_history_required": False,
    }
