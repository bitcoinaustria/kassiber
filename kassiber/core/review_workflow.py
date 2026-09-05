"""Portable, reviewed accounting changes over the canonical custody services.

Inspection and planning never write the book. Planning runs the same typed
operations and ledger builder on an isolated memory snapshot. Application
revalidates that result under SQLite's writer lock and stores one durable
receipt alongside existing domain history. A digest binds content, not authority.
"""
from __future__ import annotations

import base64
from contextlib import contextmanager
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
import hashlib
import json
import sqlite3
import uuid
from typing import Any, Mapping

from ..errors import AppError
from ..secrets import sqlcipher
from ..time_utils import now_iso
from . import custody_component_planner, custody_components, custody_journal, metadata, quarantine_resolution, tax_events

MAX_OPERATIONS = 50
MAX_ARTIFACT_BYTES = 1_000_000


@dataclass(frozen=True)
class ReviewHooks:
    metadata: metadata.MetadataHooks


def _error(message: str, code: str = "validation", **details: Any) -> AppError:
    return AppError(message, code=code, details=details or None, retryable=False)


def _json(value: Any) -> str:
    try:
        return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)
    except (TypeError, ValueError) as exc:
        raise _error("Review data must contain finite JSON values") from exc


def _digest(value: Any) -> str:
    return hashlib.sha256(_json(value).encode()).hexdigest()


def _profile(conn, profile):
    row = conn.execute(
        "SELECT * FROM profiles WHERE id = ? AND workspace_id = ?",
        (profile["id"], profile["workspace_id"]),
    ).fetchone()
    if row is None:
        raise _error("Review profile was not found", "not_found")
    return row


def _version(conn, profile) -> int:
    return int(_profile(conn, profile)["journal_input_version"] or 0)


def inspect_cases(conn, profile, *, limit=20, cursor=None) -> dict[str, Any]:
    """Read current cases inside one consistent, read-only SQLite snapshot."""
    owns_transaction = not conn.in_transaction
    if owns_transaction:
        conn.execute("BEGIN")
    try:
        return _inspect_cases(conn, profile, limit=limit, cursor=cursor)
    finally:
        if owns_transaction:
            conn.rollback()


def _inspect_cases(conn, profile, *, limit, cursor):
    if type(limit) is not int or not 1 <= limit <= 100:
        raise _error("Review limit must be between 1 and 100")
    profile = _profile(conn, profile)
    version = int(profile["journal_input_version"] or 0)
    scope = [profile["id"], version, profile["last_processed_at"]]
    after = ""
    if cursor is not None:
        try:
            payload = json.loads(base64.urlsafe_b64decode(cursor.encode()).decode())
            if payload["scope"] != scope:
                raise _error("Review cursor expired; inspect current evidence again", "review_cursor_stale")
            after = payload["after"]
            if not isinstance(after, str):
                raise ValueError()
        except AppError:
            raise
        except (ValueError, TypeError, KeyError, AttributeError) as exc:
            raise _error("Review cursor is malformed") from exc
    # Inspect current canonical blockers even if the stored journal is stale.
    state = _build(conn, profile)
    reasons = {str(q["transaction_id"]): str(q["reason"]) for q in state["quarantines"]}
    ids = sorted(tx for tx in reasons if tx > after)
    cases = []
    for transaction_id in ids[:limit]:
        row = conn.execute(
            "SELECT id AS transaction_id, direction, asset, occurred_at FROM transactions "
            "WHERE id = ? AND profile_id = ?", (transaction_id, profile["id"]),
        ).fetchone()
        if row is None:
            continue
        cases.append({
            **dict(row), "reason": reasons[transaction_id],
            "case_id": "quarantine:" + transaction_id,
            "missing_evidence": [{"code": reasons[transaction_id], "status": "unresolved"}],
            "supported_operations": (["price_override"] if "price" in reasons[transaction_id]
                                     else ["custody_component"] if any(word in reasons[transaction_id]
                                          for word in ("custody", "privacy", "transfer", "swap")) else []),
        })
    next_cursor = None
    if len(ids) > limit:
        next_cursor = base64.urlsafe_b64encode(_json({
            "scope": scope, "after": ids[limit - 1],
        }).encode()).decode()
    return {
        "schema_version": 1, "workspace_id": profile["workspace_id"],
        "profile_id": profile["id"], "input_version": version,
        "freshness": custody_journal.projection_freshness(conn, profile),
        "cases": cases, "next_cursor": next_cursor,
        "recent_receipts": _recent_receipts(conn, profile) if cursor is None else [],
    }


def _recent_receipts(conn, profile):
    rows = conn.execute(
        "SELECT receipt_json FROM review_workflow_receipts WHERE profile_id = ? "
        "ORDER BY created_at DESC, id DESC LIMIT 10", (profile["id"],),
    ).fetchall()
    keys = ("id", "created_at", "result_input_version", "artifact_digest", "status")
    return [{key: value.get(key) for key in keys} for value in
            (json.loads(row["receipt_json"]) for row in rows)]


def request_input(
    conn, profile, *, action, case_ids, expected_input_version, explanation=None,
) -> dict[str, Any]:
    """Describe missing user input without creating evidence or changing accounting.

    The packet identifies current canonical cases, not a successful resolution.
    Its digest is a stable UI correlation ID, never an authorization token.
    """
    if action not in ("connect_wallet", "import_history", "attach_evidence"):
        raise _error("Unsupported review input action")
    if (not isinstance(case_ids, list) or not 1 <= len(case_ids) <= 20
            or any(not isinstance(value, str) or not value.startswith("quarantine:")
                   or len(value) > 300 for value in case_ids)
            or len(set(case_ids)) != len(case_ids)):
        raise _error("Review input requires 1 to 20 distinct canonical case IDs")
    if action == "attach_evidence" and len(case_ids) != 1:
        raise _error("Attach evidence requires exactly one review case")
    if explanation is not None and (
        not isinstance(explanation, str) or not 1 <= len(explanation.strip()) <= 1000
        or any(ord(char) < 32 and char not in "\n\t" for char in explanation)
    ):
        raise _error("Review input explanation must be bounded plain text")
    owns_transaction = not conn.in_transaction
    if owns_transaction:
        conn.execute("BEGIN")
    try:
        current = _profile(conn, profile)
        custody_components.require_review_input_version(
            conn, workspace_id=current["workspace_id"], profile_id=current["id"],
            expected_input_version=expected_input_version,
        )
        state = _build(conn, current)
        reasons = {str(row["transaction_id"]): str(row["reason"])
                   for row in state["quarantines"]}
        cases = []
        for case_id in sorted(case_ids):
            transaction_id = case_id.removeprefix("quarantine:")
            row = conn.execute(
                "SELECT id AS transaction_id, wallet_id, direction, asset, occurred_at "
                "FROM transactions WHERE id = ? AND profile_id = ?",
                (transaction_id, current["id"]),
            ).fetchone()
            if row is None or transaction_id not in reasons:
                raise _error("Requested review case is no longer current", "review_case_stale")
            cases.append({**dict(row), "case_id": case_id, "reason": reasons[transaction_id]})
        packet = {
            "schema_version": 1, "workspace_id": current["workspace_id"],
            "profile_id": current["id"], "input_version": expected_input_version,
            "action": action, "cases": cases,
            "explanation": explanation.strip() if explanation is not None else None,
        }
        return {**packet, "request_id": _digest(packet)}
    finally:
        if owns_transaction:
            conn.rollback()


def _operation_summary(operation):
    if operation["type"] != "custody_component":
        return dict(operation)
    request = operation["request"]
    result = {"type": "custody_component", "action": request.get("action")}
    for field in ("component_id", "reason", "activate"):
        if field in request:
            result[field] = request[field]
    if "components" in request:
        result["component_count"] = len(request["components"])
    return result


def _operations(operations):
    if not isinstance(operations, list) or not 1 <= len(operations) <= MAX_OPERATIONS:
        raise _error(f"Review requires 1 to {MAX_OPERATIONS} operations")
    encoded = _json(operations)
    if len(encoded.encode()) > MAX_ARTIFACT_BYTES:
        raise _error("Review operations exceed the size limit")
    normalized = json.loads(encoded)
    for operation in normalized:
        if not isinstance(operation, dict):
            raise _error("Each review operation must be an object")
        kind = operation.get("type")
        if kind == "custody_component":
            if set(operation) != {"type", "request"} or not isinstance(operation["request"], dict):
                raise _error("Custody review requires a typed request")
            action = operation["request"].get("action")
            if not isinstance(action, str) or action not in custody_component_planner.COMPONENT_REVIEW_ACTIONS:
                raise _error("Custody review requires a supported action")
            allowed = {"action", "components", "component_id", "spec", "activate", "reason"}
            if set(operation["request"]) - allowed:
                raise _error("Custody review request contains unsupported fields")
            continue
        allowed = {"type", "transaction_id", "reason"}
        if kind == "price_override":
            allowed |= {"fiat_rate", "fiat_value"}
            supplied = [key for key in ("fiat_rate", "fiat_value") if key in operation]
            if len(supplied) != 1:
                raise _error("Price override requires exactly one fiat_rate or fiat_value")
            key = supplied[0]
            if not isinstance(operation[key], str) or len(operation[key]) > 512:
                raise _error("Review prices must be exact decimal strings of at most 512 characters")
            try:
                value = Decimal(operation[key])
                if (not value.is_finite() or abs(value.as_tuple().exponent) > 512
                        or value < 0 or (key == "fiat_rate" and value == 0)):
                    raise ValueError()
                # Bound exponent expansion before allocating the fixed notation.
                operation[key] = format(value, "f")
                if len(operation[key]) > 512:
                    raise ValueError()
            except (InvalidOperation, ValueError) as exc:
                raise _error("Review price is invalid") from exc
        elif kind != "exclude":
            raise _error("Review operation type is unsupported")
        if set(operation) - allowed:
            raise _error("Review operation contains unsupported fields")
        for key in ("transaction_id", "reason"):
            if not isinstance(operation.get(key), str) or not operation[key].strip():
                raise _error(f"Review operation requires {key}")
            operation[key] = operation[key].strip()
        if len(operation["reason"]) > 2000:
            raise _error("Review reason exceeds 2000 characters")
    return normalized


@contextmanager
def _snapshot(conn):
    # SQLCipher's driver cannot back up to a stdlib SQLite connection. Keep
    # the exact binding and all decrypted pages in RAM; never use temp files.
    binding = sqlite3 if isinstance(conn, sqlite3.Connection) else sqlcipher.require_sqlcipher()
    clone = binding.connect(":memory:")
    try:
        clone.row_factory = binding.Row
        if binding is not sqlite3:
            status = conn.execute("PRAGMA cipher_status").fetchone()
            if status and str(status[0]) == "1":
                # SQLCipher backup requires an encrypted destination, even in
                # memory. A fresh ephemeral key suffices; never read the book key.
                clone.execute("PRAGMA key = '" + uuid.uuid4().hex + "'")
        clone.execute("PRAGMA temp_store = MEMORY")
        def progress(status, remaining, total):
            if status in (sqlite3.SQLITE_BUSY, sqlite3.SQLITE_LOCKED):
                raise _error("Finish the current write transaction before planning a review", "review_transaction_active")
        conn.backup(clone, pages=128, progress=progress)
        clone.execute("PRAGMA foreign_keys = ON")
        yield clone
    finally:
        clone.close()


def _economic_values(value):
    if isinstance(value, Mapping):
        return {str(key): _economic_values(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_economic_values(item) for item in value]
    if isinstance(value, Decimal):
        return str(value)
    return value


def _effects(state) -> dict[str, Any]:
    # Hash full economic output while bounding the human/model-facing preview.
    # IDs created for derived journal rows are deliberately not economic facts.
    fields = ("transaction_id", "wallet_id", "account_id", "occurred_at", "entry_type",
              "asset", "quantity", "fiat_value", "unit_cost", "cost_basis", "proceeds",
              "gain_loss", "at_category", "at_kennzahl", "capital_gains_type")
    entries = []
    for entry in state["entries"]:
        entries.append({key: str(entry[key]) if entry.get(key) is not None else None for key in fields})
    entries.sort(key=_json)
    quarantines = sorted({(str(q["transaction_id"]), str(q["reason"])) for q in state["quarantines"]})
    holdings = sorted([
        {"wallet_id": key[0], "asset": key[3], "quantity": str(value["quantity"]),
         "cost_basis": str(value["cost_basis"])}
        for key, value in state["wallet_holdings"].items()
    ], key=_json)
    quantity = state.get("custody_quantity")
    blocked = bool(state.get("custody_component_blockers") or (quantity and quantity.report_blocked))
    return {
        "entries_count": len(entries), "quarantine_count": len(quarantines),
        "quarantines": [{"transaction_id": tx, "reason": reason} for tx, reason in quarantines[:100]],
        "wallet_holdings": holdings[:100], "report_ready": not quarantines and not blocked,
        "journal_digest": _digest({"entries": entries, "quarantines": quarantines, "holdings": holdings,
                                   "tax_summary": _economic_values(state["tax_summary"]), "blocked": blocked}),
    }


def _apply_operations(conn, profile, operations, hooks, authored_source, case_ids):
    results = []
    for operation in operations:
        kind = operation["type"]
        if kind == "custody_component":
            result = custody_component_planner.apply_component_review(
                conn, workspace_id=profile["workspace_id"], profile_id=profile["id"],
                expected_input_version=_version(conn, profile), authored_source=authored_source,
                commit=False, **operation["request"],
            )
        else:
            tx = conn.execute("SELECT * FROM transactions WHERE id = ? AND profile_id = ?",
                              (operation["transaction_id"], profile["id"])).fetchone()
            if tx is None:
                raise _error("Review transaction was not found", "not_found")
            if tx["id"] not in case_ids:
                raise _error("Review transaction is not quarantined", "review_case_changed")
            updated = quarantine_resolution.update_quarantine_metadata(
                conn, profile, transaction_id=tx["id"], action=kind, hooks=hooks.metadata,
                authored_source=authored_source, reason=operation["reason"],
                fiat_rate=operation.get("fiat_rate"), fiat_value=operation.get("fiat_value"),
                commit=False,
            )
            result = {"transaction_id": tx["id"], "history_event_id": updated["history_event_id"],
                      "updated": updated["updated"]}
        results.append({"type": kind, "result": result})
    return results


def _build(conn, profile):
    if conn.execute("SELECT 1 FROM sync_conflicts WHERE profile_id = ? AND status = 'open' LIMIT 1",
                    (profile["id"],)).fetchone():
        raise _error("Review is blocked by unresolved synchronization conflicts", "sync_conflicts_open")
    state = custody_journal.build_ledger_state(conn, _profile(conn, profile))
    # Match the canonical store projection: one reason per live raw anchor;
    # synthetic engine-only rows do not become separately actionable cases.
    live_ids = {str(row[0]) for row in conn.execute(
        "SELECT id FROM transactions WHERE profile_id = ?", (profile["id"],),
    )}
    state["quarantines"] = [q for q in tax_events.dedupe_quarantines(state["quarantines"])
                            if str(q["transaction_id"]) in live_ids]
    return state


def plan_review(conn, profile, *, operations, expected_input_version, hooks):
    """Return an immutable portable preview from current recorded facts/rates."""
    operations = _operations(operations)
    with _snapshot(conn) as clone:
        current = _profile(clone, profile)
        custody_components.require_review_input_version(
            clone, workspace_id=current["workspace_id"], profile_id=current["id"],
            expected_input_version=expected_input_version,
        )
        before_state = _build(clone, current)
        before = _effects(before_state)
        _apply_operations(clone, current, operations, hooks, "user",
                          {q["transaction_id"] for q in before_state["quarantines"]})
        after = _effects(_build(clone, current))
        artifact = {
            "schema_version": 1, "workspace_id": current["workspace_id"], "profile_id": current["id"],
            "base_input_version": expected_input_version, "operations": operations,
            "before": before, "after": after,
        }
        artifact["digest"] = _digest(artifact)
        if len(_json(artifact).encode()) > MAX_ARTIFACT_BYTES:
            raise _error("Review artifact exceeds the size limit")
        return artifact


def validate_review(conn, profile, *, artifact, hooks):
    """Recompute a portable proposal before presenting its consent preview."""
    if not isinstance(artifact, Mapping) or artifact.get("workspace_id") != profile["workspace_id"] or artifact.get("profile_id") != profile["id"]:
        raise _error("Review artifact belongs to another profile", "review_scope_mismatch")
    fresh = plan_review(conn, profile, operations=artifact.get("operations"),
                        expected_input_version=artifact.get("base_input_version"), hooks=hooks)
    if fresh != artifact:
        raise _error("Review artifact changed; create a fresh preview", "review_artifact_invalid")
    return fresh


def get_receipt(conn, profile, *, receipt_id=None, idempotency_key=None):
    profile = _profile(conn, profile)
    if (receipt_id is None) == (idempotency_key is None):
        raise _error("Specify exactly one receipt_id or idempotency_key")
    field = "id" if receipt_id is not None else "idempotency_key"
    value = receipt_id if receipt_id is not None else idempotency_key
    row = conn.execute(
        f"SELECT receipt_json FROM review_workflow_receipts WHERE profile_id = ? AND {field} = ?",
        (profile["id"], value),
    ).fetchone()
    if row is None:
        raise _error("Review receipt was not found", "not_found")
    return json.loads(row["receipt_json"])


def audit_receipt_summary(conn, profile_id, *, transaction_ids=None, limit=500):
    """Export bounded hash links, never proposal text or profile-wide effects."""
    where = "profile_id = ?"
    params = [profile_id]
    if transaction_ids is not None:
        selected = sorted(set(transaction_ids))
        if not selected:
            return {"records": [], "count": 0, "truncated": False, "raw_proposals_included": False}
        where += " AND EXISTS (SELECT 1 FROM json_each(receipt_json, '$.transaction_ids') WHERE value IN (" + ",".join("?" for _ in selected) + "))"
        params.extend(selected)
    bounded_limit = min(max(int(limit), 1), 500)
    rows = conn.execute(
        "SELECT id,artifact_digest,receipt_json,created_at FROM review_workflow_receipts WHERE "
        + where + " ORDER BY created_at DESC,id DESC LIMIT ?", (*params, bounded_limit + 1),
    ).fetchall()
    records = []
    for row in rows[:bounded_limit]:
        receipt = json.loads(row["receipt_json"])
        records.append({"id": row["id"], "artifact_digest": row["artifact_digest"],
                        "receipt_digest": _digest(receipt), "created_at": row["created_at"],
                        "status": receipt["status"], "base_input_version": receipt["base_input_version"],
                        "result_input_version": receipt["result_input_version"]})
    return {"records": records, "count": len(records), "truncated": len(rows) > bounded_limit,
            "raw_proposals_included": False}


def _receipt_transaction_ids(results):
    ids = set()
    for item in results:
        result = item["result"]
        if result.get("transaction_id"):
            ids.add(result["transaction_id"])
        components = result.get("components", [])
        if result.get("component"):
            components = [*components, result["component"]]
        for component in components:
            for leg in component.get("legs", []):
                anchor = leg.get("anchor_transaction_id") or leg.get("transaction_id")
                if anchor:
                    ids.add(anchor)
    return sorted(ids)


def apply_review(conn, profile, *, artifact, idempotency_key, hooks, authored_source="user", commit=True):
    """Revalidate, apply and verify once, atomically with the durable receipt."""
    if not isinstance(artifact, Mapping) or set(artifact) != {
        "schema_version", "workspace_id", "profile_id", "base_input_version", "operations", "before", "after", "digest",
    }:
        raise _error("Review artifact is malformed", "review_artifact_invalid")
    if len(_json(artifact).encode()) > MAX_ARTIFACT_BYTES:
        raise _error("Review artifact exceeds the size limit")
    payload = {key: value for key, value in artifact.items() if key != "digest"}
    if artifact["schema_version"] != 1 or _digest(payload) != artifact["digest"]:
        raise _error("Review artifact digest is invalid", "review_artifact_invalid")
    if artifact["profile_id"] != profile["id"] or artifact["workspace_id"] != profile["workspace_id"]:
        raise _error("Review artifact belongs to another profile", "review_scope_mismatch")
    operations = _operations(artifact["operations"])
    if operations != artifact["operations"]:
        raise _error("Review operations are not canonical", "review_artifact_invalid")
    if not isinstance(idempotency_key, str) or not 1 <= len(idempotency_key.strip()) <= 200:
        raise _error("Review requires an idempotency key of 1 to 200 characters")
    own_transaction = not conn.in_transaction
    if own_transaction:
        conn.execute("BEGIN IMMEDIATE")
    conn.execute("SAVEPOINT review_workflow_apply")
    released = False
    try:
        # This write reserves the writer even when a caller owns the transaction.
        conn.execute("UPDATE profiles SET journal_input_version = journal_input_version WHERE id = ?", (profile["id"],))
        existing = conn.execute(
            "SELECT artifact_digest, receipt_json FROM review_workflow_receipts WHERE profile_id = ? AND idempotency_key = ?",
            (profile["id"], idempotency_key),
        ).fetchone()
        if existing:
            if existing["artifact_digest"] != artifact["digest"]:
                raise _error("Idempotency key already belongs to another proposal", "review_idempotency_conflict")
            receipt = json.loads(existing["receipt_json"])
        else:
            custody_components.require_review_input_version(
                conn, workspace_id=profile["workspace_id"], profile_id=profile["id"],
                expected_input_version=artifact["base_input_version"],
            )
            before_state = _build(conn, profile)
            if _effects(before_state) != artifact["before"]:
                raise _error("Review evidence changed; create a fresh preview", "review_plan_stale")
            results = _apply_operations(conn, profile, operations, hooks, authored_source,
                                        {q["transaction_id"] for q in before_state["quarantines"]})
            state = _build(conn, profile)
            after = _effects(state)
            if after != artifact["after"]:
                raise _error("Review effects changed; create a fresh preview", "review_plan_stale")
            current = _profile(conn, profile)
            stored = custody_journal.store_ledger_state(conn, current, state)
            receipt = {
                "id": str(uuid.uuid4()), "schema_version": 1,
                "workspace_id": current["workspace_id"], "profile_id": current["id"],
                "idempotency_key": idempotency_key, "artifact_digest": artifact["digest"],
                "base_input_version": artifact["base_input_version"],
                "result_input_version": int(current["journal_input_version"] or 0),
                "status": "verified", "operations": results,
                "transaction_ids": _receipt_transaction_ids(results),
                "before": artifact["before"], "proposed_operations": [_operation_summary(op) for op in operations],
                "verification": {**after, "processed_at": stored["processed_at"]},
                "authored_source": authored_source, "created_at": now_iso(),
            }
            conn.execute(
                "INSERT INTO review_workflow_receipts(id,workspace_id,profile_id,idempotency_key,artifact_digest,receipt_json,created_at) VALUES(?,?,?,?,?,?,?)",
                (receipt["id"], current["workspace_id"], current["id"], idempotency_key,
                 artifact["digest"], _json(receipt), receipt["created_at"]),
            )
        conn.execute("RELEASE SAVEPOINT review_workflow_apply")
        released = True
        if commit:
            conn.commit()
        return receipt
    except Exception:
        if not released:
            conn.execute("ROLLBACK TO SAVEPOINT review_workflow_apply")
            conn.execute("RELEASE SAVEPOINT review_workflow_apply")
        if own_transaction or (released and commit):
            conn.rollback()
        raise
