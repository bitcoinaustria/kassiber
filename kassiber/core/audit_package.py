from __future__ import annotations

import hashlib
import json
import shutil
import sqlite3
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence
from urllib.parse import parse_qsl, urlencode, urlparse, urlunparse

from ..db import ensure_data_root, resolve_attachments_root
from ..envelope import json_ready
from ..errors import AppError
from ..msat import msat_to_btc
from . import source_funds as core_source_funds
from .attachments import attachment_display_label
from . import transaction_history

ScopeResolver = Callable[[sqlite3.Connection, str | None, str | None], tuple[Mapping[str, Any], Mapping[str, Any]]]
TransactionResolver = Callable[..., Mapping[str, Any]]
NowIso = Callable[[], str]

AUDIT_PACKAGE_SCHEMA_VERSION = 1
SENSITIVE_MATERIAL_EXCLUSIONS = [
    "wallet descriptors",
    "xpubs",
    "backend credentials",
    "backend URLs",
    "raw wallet files",
    "raw wallet config",
    "environment files",
    "logs",
    "AI settings",
    "unrelated books",
    "technical wallet evidence",
]
_DECISION_KEYWORDS = (
    "board",
    "decision",
    "management",
    "approval",
    "resolution",
    "director",
)
_SECRET_URL_QUERY_HINTS = (
    "access_token",
    "api_key",
    "auth",
    "bearer",
    "key",
    "password",
    "secret",
    "signature",
    "sig",
    "token",
)


@dataclass(frozen=True)
class AuditPackageHooks:
    resolve_scope: ScopeResolver
    resolve_transaction: TransactionResolver
    now_iso: NowIso


def _row_dict(row: Mapping[str, Any]) -> dict[str, Any]:
    return {key: row[key] for key in row.keys()}


def _row_int(row: Mapping[str, Any], key: str, default: int = 0) -> int:
    try:
        if hasattr(row, "keys") and key not in row.keys():
            return default
        value = row[key]
    except (IndexError, KeyError):
        return default
    return int(value or default)


def _row_get(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    try:
        if hasattr(row, "keys") and key not in row.keys():
            return default
        return row[key]
    except (IndexError, KeyError):
        return default


def _btc_value(msat: int | None) -> float | None:
    if msat is None:
        return None
    return float(msat_to_btc(msat))


def _safe_filename(value: str | None) -> str:
    raw = Path(value or "").name.strip() or "evidence.bin"
    chars = []
    for char in raw:
        if char.isalnum() or char in {".", "_", "-"}:
            chars.append(char)
        else:
            chars.append("_")
    cleaned = "".join(chars).strip("._")
    return cleaned or "evidence.bin"


def _attachments_root(data_root: str) -> Path:
    return ensure_data_root(resolve_attachments_root(data_root))


def _resolve_stored_path(attachments_root: Path, stored_relpath: str | None) -> tuple[Path | None, bool]:
    raw = (stored_relpath or "").strip()
    if not raw:
        return None, True
    relpath = Path(raw)
    if relpath.is_absolute():
        return None, False
    root = attachments_root.resolve()
    candidate = (attachments_root / relpath).resolve(strict=False)
    try:
        candidate.relative_to(root)
    except ValueError:
        return None, False
    return candidate, True


def _hash_file(path: Path) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
            size += len(chunk)
    return size, digest.hexdigest()


def _url_safety(url: str | None) -> dict[str, Any]:
    raw = (url or "").strip()
    if not raw:
        return {"url": "", "redacted": False, "redacted_url": "", "reason": ""}
    parsed = urlparse(raw)
    redacted = False
    reason = ""
    query_pairs = parse_qsl(parsed.query, keep_blank_values=True)
    safe_query_pairs = []
    if parsed.username or parsed.password:
        redacted = True
        reason = "embedded credentials"
        parsed = parsed._replace(netloc=parsed.hostname or "")
    for key, value in query_pairs:
        key_lower = key.lower()
        if any(hint in key_lower for hint in _SECRET_URL_QUERY_HINTS):
            redacted = True
            reason = reason or "secret-like query parameter"
            safe_query_pairs.append((key, "REDACTED"))
        else:
            safe_query_pairs.append((key, value))
    fragment = parsed.fragment
    redacted_fragment = fragment
    fragment_pairs = parse_qsl(fragment.lstrip("?"), keep_blank_values=True)
    if fragment_pairs:
        safe_fragment_pairs = []
        for key, value in fragment_pairs:
            key_lower = key.lower()
            if any(hint in key_lower for hint in _SECRET_URL_QUERY_HINTS):
                redacted = True
                reason = reason or "secret-like URL fragment"
                safe_fragment_pairs.append((key, "REDACTED"))
            else:
                safe_fragment_pairs.append((key, value))
        redacted_fragment = urlencode(safe_fragment_pairs)
    elif fragment and any(hint in fragment.lower() for hint in _SECRET_URL_QUERY_HINTS):
        redacted = True
        reason = reason or "secret-like URL fragment"
        redacted_fragment = "REDACTED"
    redacted_query = urlencode(safe_query_pairs)
    redacted_url = urlunparse(parsed._replace(query=redacted_query, fragment=redacted_fragment))
    return {
        "url": raw if not redacted else "",
        "redacted": redacted,
        "redacted_url": redacted_url,
        "reason": reason,
        "host": parsed.netloc or parsed.hostname or "",
        "scheme": parsed.scheme,
    }


def _attachment_summary(
    row: Mapping[str, Any],
    attachments_root: Path,
    *,
    include_url: bool,
) -> dict[str, Any]:
    stored_path, path_valid = _resolve_stored_path(attachments_root, row["stored_relpath"])
    exists = stored_path.exists() if stored_path else (False if row["stored_relpath"] and not path_valid else None)
    url = _url_safety(row["source_url"])
    item = {
        "id": row["id"],
        "attachment_type": row["attachment_type"],
        "label": attachment_display_label(row),
        "original_filename": row["original_filename"] or "",
        "media_type": row["media_type"] or "",
        "size_bytes": int(row["size_bytes"]) if row["size_bytes"] is not None else None,
        "sha256": row["sha256"] or "",
        "exists": exists,
        "url_host": url.get("host", ""),
        "url_scheme": url.get("scheme", ""),
        "url_redacted": bool(url.get("redacted")),
        "url_redaction_reason": url.get("reason", ""),
        "copied_from_attachment_id": row["copied_from_attachment_id"] or "",
        "copied_from_transaction_id": row["copied_from_transaction_id"] or "",
        "created_at": row["created_at"],
    }
    if include_url and row["attachment_type"] == "url":
        item["source_url"] = url["url"]
        item["source_url_redacted"] = url["redacted_url"] if url["redacted"] else ""
    return item


def _attachment_rows_by_ids(
    conn: sqlite3.Connection,
    profile_id: str,
    ids: Sequence[str],
) -> list[sqlite3.Row]:
    unique_ids = sorted(set(ids))
    if not unique_ids:
        return []
    placeholders = ",".join("?" for _ in unique_ids)
    return conn.execute(
        f"""
        SELECT *
        FROM attachments
        WHERE profile_id = ? AND id IN ({placeholders})
        ORDER BY created_at ASC, id ASC
        """,
        (profile_id, *unique_ids),
    ).fetchall()


def _direct_attachment_rows(
    conn: sqlite3.Connection,
    profile_id: str,
    tx_id: str,
) -> list[sqlite3.Row]:
    return conn.execute(
        """
        SELECT *
        FROM attachments
        WHERE profile_id = ? AND transaction_id = ?
        ORDER BY created_at ASC, id ASC
        """,
        (profile_id, tx_id),
    ).fetchall()


def _journal_freshness(conn: sqlite3.Connection, profile: Mapping[str, Any]) -> dict[str, Any]:
    active_count = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        """,
        (profile["id"],),
    ).fetchone()["count"]
    active_count = int(active_count or 0)
    last_processed_at = profile["last_processed_at"] if "last_processed_at" in profile.keys() else None
    last_processed_tx_count = _row_int(profile, "last_processed_tx_count")
    journal_input_version = _row_int(profile, "journal_input_version")
    last_processed_input_version = _row_int(profile, "last_processed_input_version")
    if active_count == 0:
        status = "no_transactions"
        reason = "no active transactions"
    elif not last_processed_at:
        status = "not_processed"
        reason = "journals have not been processed"
    elif last_processed_tx_count != active_count:
        status = "stale"
        reason = "active transaction count changed since last processing"
    elif journal_input_version != last_processed_input_version:
        status = "stale"
        reason = "journal inputs changed since last processing"
    else:
        status = "current"
        reason = "journals match the active transaction count and input version"
    return {
        "status": status,
        "needs_processing": status in {"not_processed", "stale"},
        "reason": reason,
        "last_processed_at": last_processed_at,
        "last_processed_tx_count": last_processed_tx_count,
        "journal_input_version": journal_input_version,
        "last_processed_input_version": last_processed_input_version,
        "active_transaction_count": active_count,
    }


def _readiness_journal_freshness(
    freshness: Mapping[str, Any],
    *,
    include_journal_state: bool,
) -> dict[str, Any] | Mapping[str, Any]:
    if include_journal_state:
        return freshness
    neutral = dict(freshness)
    neutral["needs_processing"] = False
    neutral["reason"] = "journal state excluded by export options"
    return neutral


def _tx_tags(conn: sqlite3.Connection, tx_id: str) -> list[str]:
    return [
        row["label"]
        for row in conn.execute(
            """
            SELECT tg.label
            FROM transaction_tags tt
            JOIN tags tg ON tg.id = tt.tag_id
            WHERE tt.transaction_id = ?
            ORDER BY tg.label ASC, tg.id ASC
            """,
            (tx_id,),
        ).fetchall()
    ]


def _transaction_summary(row: Mapping[str, Any], tags: Sequence[str] | None = None) -> dict[str, Any]:
    return {
        "id": row["id"],
        "external_id": row["external_id"] or "",
        "occurred_at": row["occurred_at"] or "",
        "direction": row["direction"],
        "asset": row["asset"],
        "amount": _btc_value(row["amount"]),
        "amount_msat": row["amount"],
        "fee": _btc_value(row["fee"]),
        "fee_msat": row["fee"],
        "fiat_currency": row["fiat_currency"] or "",
        "fiat_value": row["fiat_value"],
        "fiat_price_source": row["fiat_price_source"] or "",
        "pricing_source_kind": row["pricing_source_kind"] or "",
        "pricing_provider": row["pricing_provider"] or "",
        "pricing_quality": row["pricing_quality"] or "",
        "kind": row["kind"] or "",
        "description": row["description"] or "",
        "counterparty": row["counterparty"] or "",
        "note": row["note"] or "",
        "excluded": bool(row["excluded"]),
        "tags": list(tags or ()),
    }


def _attachment_ids_for_join(
    conn: sqlite3.Connection,
    table: str,
    id_column: str,
    value: str,
) -> list[str]:
    rows = conn.execute(
        f"""
        SELECT attachment_id
        FROM {table}
        WHERE {id_column} = ?
        ORDER BY created_at ASC, attachment_id ASC
        """,
        (value,),
    ).fetchall()
    return [row["attachment_id"] for row in rows]


def _source_summary(
    conn: sqlite3.Connection,
    profile_id: str,
    source_id: str,
    attachments_root: Path,
) -> dict[str, Any] | None:
    source = conn.execute(
        """
        SELECT *
        FROM source_funds_sources
        WHERE profile_id = ? AND id = ?
        """,
        (profile_id, source_id),
    ).fetchone()
    if not source:
        return None
    attachment_rows = _attachment_rows_by_ids(
        conn,
        profile_id,
        _attachment_ids_for_join(conn, "source_funds_source_attachments", "source_id", source_id),
    )
    return {
        "id": source["id"],
        "source_type": source["source_type"],
        "label": source["label"],
        "asset": source["asset"],
        "amount": _btc_value(source["amount"]),
        "amount_msat": source["amount"],
        "fiat_currency": source["fiat_currency"] or "",
        "fiat_value": source["fiat_value"],
        "acquired_at": source["acquired_at"] or "",
        "review_state": source["review_state"],
        "description": source["description"] or "",
        "attachments": [
            _attachment_summary(row, attachments_root, include_url=False)
            for row in attachment_rows
        ],
    }


def _source_funds_links_for_tx(
    conn: sqlite3.Connection,
    profile_id: str,
    tx_id: str,
    attachments_root: Path,
) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT *
        FROM source_funds_links
        WHERE profile_id = ? AND to_transaction_id = ? AND state != 'rejected'
        ORDER BY state ASC, created_at ASC, id ASC
        """,
        (profile_id, tx_id),
    ).fetchall()
    links = []
    for row in rows:
        is_reviewed = row["state"] == "reviewed"
        attachment_rows = (
            _attachment_rows_by_ids(
                conn,
                profile_id,
                _attachment_ids_for_join(conn, "source_funds_link_attachments", "link_id", row["id"]),
            )
            if is_reviewed
            else []
        )
        parent_tx = None
        if is_reviewed and row["from_transaction_id"]:
            tx_row = conn.execute(
                "SELECT * FROM transactions WHERE profile_id = ? AND id = ?",
                (profile_id, row["from_transaction_id"]),
            ).fetchone()
            if tx_row:
                parent_tx = _transaction_summary(tx_row)
        from_source = (
            _source_summary(conn, profile_id, row["from_source_id"], attachments_root)
            if is_reviewed and row["from_source_id"]
            else None
        )
        links.append(
            {
                "id": row["id"],
                "link_type": row["link_type"],
                "state": row["state"],
                "confidence": row["confidence"],
                "method": row["method"],
                "asset": row["asset"],
                "allocation_amount": _btc_value(row["allocation_amount"]) if is_reviewed else None,
                "allocation_amount_msat": row["allocation_amount"] if is_reviewed else None,
                "from_asset": (row["from_asset"] or row["asset"]) if is_reviewed else None,
                "from_allocation_amount": (
                    _btc_value(row["from_allocation_amount"]) if is_reviewed else None
                ),
                "from_allocation_amount_msat": row["from_allocation_amount"] if is_reviewed else None,
                "allocation_policy": row["allocation_policy"] if is_reviewed else "",
                "explanation": (row["explanation"] or "") if is_reviewed else "",
                "uses_chain_observation": bool(row["uses_chain_observation"]),
                "chain_data_confirmed": bool(row["chain_data_confirmed"]) if is_reviewed else False,
                "review_details_redacted": not is_reviewed,
                "attachments": [
                    _attachment_summary(attachment, attachments_root, include_url=False)
                    for attachment in attachment_rows
                ],
                "from_source": from_source,
                "from_transaction": parent_tx,
                "created_at": row["created_at"],
                "updated_at": row["updated_at"],
            }
        )
    return links


def _journal_entries_for_tx(conn: sqlite3.Connection, profile_id: str, tx_id: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT *
        FROM journal_entries
        WHERE profile_id = ? AND transaction_id = ?
        ORDER BY occurred_at ASC, entry_type ASC, id ASC
        """,
        (profile_id, tx_id),
    ).fetchall()
    return [
        {
            "id": row["id"],
            "occurred_at": row["occurred_at"],
            "entry_type": row["entry_type"],
            "asset": row["asset"],
            "quantity": _btc_value(row["quantity"]),
            "quantity_msat": row["quantity"],
            "fiat_value": row["fiat_value"],
            "gain_loss": row["gain_loss"],
            "at_category": row["at_category"] if "at_category" in row.keys() else "",
            "at_kennzahl": row["at_kennzahl"] if "at_kennzahl" in row.keys() else None,
            "pricing_source_kind": row["pricing_source_kind"] if "pricing_source_kind" in row.keys() else "",
            "pricing_quality": row["pricing_quality"] if "pricing_quality" in row.keys() else "",
            "description": row["description"] if "description" in row.keys() else "",
        }
        for row in rows
    ]


def _journal_quarantine_for_tx(conn: sqlite3.Connection, profile_id: str, tx_id: str) -> dict[str, Any] | None:
    row = conn.execute(
        """
        SELECT *
        FROM journal_quarantines
        WHERE profile_id = ? AND transaction_id = ?
        """,
        (profile_id, tx_id),
    ).fetchone()
    if not row:
        return None
    return {
        "reason": row["reason"],
        "details": row["details"] or "",
        "created_at": row["created_at"],
    }


def _warning(code: str, severity: str, message: str, *, action: str | None = None) -> dict[str, Any]:
    item = {"code": code, "severity": severity, "message": message}
    if action:
        item["action"] = action
    return item


def _decision_evidence_relevant(tx: Mapping[str, Any], tags: Sequence[str]) -> bool:
    haystack = " ".join(
        str(_row_get(tx, field, "") or "")
        for field in ("kind", "description", "counterparty", "note")
    )
    haystack += " " + " ".join(tags)
    normalized = haystack.lower()
    return any(keyword in normalized for keyword in _DECISION_KEYWORDS)


def _has_pricing_evidence(tx: Mapping[str, Any], quarantine: Mapping[str, Any] | None) -> bool:
    if quarantine and "price" in str(quarantine.get("reason") or "").lower():
        return False
    if tx["fiat_rate"] is None and tx["fiat_value"] is None:
        return False
    return bool(
        tx["pricing_source_kind"]
        or tx["fiat_price_source"]
        or tx["pricing_provider"]
        or tx["pricing_external_ref"]
        or tx["fiat_rate_exact"]
        or tx["fiat_value_exact"]
    )


def _evidence_warnings(
    tx: Mapping[str, Any],
    tags: Sequence[str],
    direct_attachments: Sequence[Mapping[str, Any]],
    source_links: Sequence[Mapping[str, Any]],
    journal_freshness: Mapping[str, Any],
    quarantine: Mapping[str, Any] | None,
    *,
    include_review_state: bool,
) -> list[dict[str, Any]]:
    warnings: list[dict[str, Any]] = []
    if tx["excluded"]:
        warnings.append(
            _warning(
                "transaction_excluded",
                "warning",
                "This transaction is excluded from accounting reports.",
                action="Review whether the exclusion is intended before handoff.",
            )
        )
    if not direct_attachments:
        warnings.append(
            _warning(
                "receipt_missing",
                "blocker",
                "No direct receipt, invoice, note, or URL reference is attached to this transaction.",
                action="Attach a local receipt file or a URL reference from transaction detail.",
            )
        )
    if any(item["attachment_type"] == "file" and item.get("exists") is False for item in direct_attachments):
        warnings.append(
            _warning(
                "receipt_file_missing",
                "blocker",
                "A copied attachment row points to a missing managed file.",
                action="Reattach the local file or remove the stale attachment row.",
            )
        )
    if _decision_evidence_relevant(tx, tags):
        decision_evidence = [
            item
            for item in direct_attachments
            if any(keyword in item["label"].lower() for keyword in _DECISION_KEYWORDS)
        ]
        if not decision_evidence:
            warnings.append(
                _warning(
                    "decision_evidence_missing",
                    "blocker",
                    "This transaction looks decision-backed, but no board or management decision evidence is attached.",
                    action="Attach the decision document as a URL reference or local file.",
                )
            )
    if include_review_state:
        reviewed_links = [link for link in source_links if link["state"] == "reviewed"]
        suggested_links = [link for link in source_links if link["state"] == "suggested"]
        if not reviewed_links:
            warnings.append(
                _warning(
                    "source_link_missing",
                    "blocker",
                    "No reviewed source-of-funds link or root source is connected to this transaction.",
                    action="Review or create a source-of-funds link before exporting.",
                )
            )
        if suggested_links:
            warnings.append(
                _warning(
                    "source_link_unreviewed",
                    "blocker",
                    "At least one source-of-funds suggestion is still unreviewed.",
                    action="Accept, edit, or reject the suggested link.",
                )
            )
        for link in reviewed_links:
            link_evidence_count = len(link.get("attachments") or [])
            source = link.get("from_source") or {}
            source_evidence_count = len(source.get("attachments") or []) if isinstance(source, dict) else 0
            if link_evidence_count == 0 and source_evidence_count == 0:
                warnings.append(
                    _warning(
                        "source_evidence_missing",
                        "warning",
                        "A reviewed source-of-funds link has no attached supporting evidence.",
                        action="Attach evidence to the link or its root source.",
                    )
                )
                break
    if journal_freshness.get("needs_processing"):
        warnings.append(
            _warning(
                "journal_stale",
                "blocker",
                f"Journal state is {journal_freshness.get('status')}: {journal_freshness.get('reason')}.",
                action="Run journal processing before handing this transaction to an auditor.",
            )
        )
    if not _has_pricing_evidence(tx, quarantine):
        warnings.append(
            _warning(
                "pricing_evidence_missing",
                "blocker",
                "No persisted pricing source is available for this transaction.",
                action="Add a manual price or refresh rates so the pricing source is recorded.",
            )
        )
    if quarantine:
        warnings.append(
            _warning(
                "journal_quarantined",
                "blocker",
                f"Journal processing quarantined this transaction: {quarantine.get('reason')}.",
                action="Resolve the journal quarantine before export.",
            )
        )
    warnings.append(
        _warning(
            "sensitive_material_excluded",
            "info",
            "Descriptors, xpubs, backend URLs, credentials, wallet files, logs, AI settings, and technical wallet evidence are excluded from this audit surface.",
        )
    )
    return warnings


def _transaction_rows_for_scope(
    conn: sqlite3.Connection,
    profile_id: str,
    hooks: AuditPackageHooks,
    transaction_refs: Sequence[str] | None,
) -> list[sqlite3.Row]:
    if transaction_refs is not None:
        rows = []
        seen = set()
        for ref in transaction_refs:
            if not isinstance(ref, str) or not ref.strip():
                raise AppError("transaction references must be non-empty strings", code="validation")
            row = hooks.resolve_transaction(conn, profile_id, ref.strip())
            if row["id"] not in seen:
                rows.append(row)
                seen.add(row["id"])
        return sorted(rows, key=lambda row: (row["occurred_at"] or "", row["created_at"] or "", row["id"]))
    return conn.execute(
        """
        SELECT *
        FROM transactions
        WHERE profile_id = ?
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()


def _transaction_refs_from_case(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: AuditPackageHooks,
    case_ref: str,
) -> tuple[list[str], dict[str, Any]]:
    source_hooks = core_source_funds.SourceFundsHooks(
        resolve_scope=hooks.resolve_scope,
        resolve_transaction=hooks.resolve_transaction,
        format_table=lambda *args, **kwargs: [],
    )
    snapshot = core_source_funds.load_case_snapshot(conn, workspace_ref, profile_ref, source_hooks, case_ref)
    tx_refs = []
    for node in snapshot.get("graph", {}).get("nodes", []):
        if isinstance(node, dict) and node.get("node_type") == "transaction" and node.get("transaction_id"):
            tx_refs.append(str(node["transaction_id"]))
    target = snapshot.get("target", {})
    if isinstance(target, dict) and target.get("transaction_id"):
        tx_refs.append(str(target["transaction_id"]))
    return sorted(set(tx_refs)), {
        "id": case_ref,
        "snapshot_hash": snapshot.get("case", {}).get("snapshot_hash") or "",
        "reveal_mode": snapshot.get("reveal_mode") or "",
        "target_transaction_id": target.get("transaction_id") or "",
        "target_label": target.get("label") or "",
        "exportable": bool(snapshot.get("explain_gates", {}).get("exportable")),
    }


def build_evidence_summary(
    conn: sqlite3.Connection,
    data_root: str,
    workspace_ref: str | None,
    profile_ref: str | None,
    hooks: AuditPackageHooks,
    *,
    transaction_refs: Sequence[str] | None = None,
    source_funds_case_ref: str | None = None,
    include_journal_state: bool = True,
    include_review_state: bool = True,
    include_edit_history: bool = False,
) -> dict[str, Any]:
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    scope: dict[str, Any] = {"type": "active_profile"}
    if transaction_refs is not None and source_funds_case_ref:
        raise AppError("Choose transactions or source_funds_case, not both", code="validation")
    resolved_transaction_refs = transaction_refs
    if source_funds_case_ref:
        refs, case = _transaction_refs_from_case(
            conn,
            workspace_ref,
            profile_ref,
            hooks,
            source_funds_case_ref,
        )
        resolved_transaction_refs = refs
        scope = {"type": "source_funds_case", "case": case}
    elif transaction_refs is not None:
        scope = {"type": "transactions", "transaction_count": len(transaction_refs)}

    attachments_root = _attachments_root(data_root)
    freshness = _journal_freshness(conn, profile)
    readiness_freshness = _readiness_journal_freshness(
        freshness,
        include_journal_state=include_journal_state,
    )
    transactions = []
    warning_counts: dict[str, int] = {}
    tx_rows = _transaction_rows_for_scope(conn, profile["id"], hooks, resolved_transaction_refs)
    tx_ids = [tx["id"] for tx in tx_rows]
    edit_history = (
        transaction_history.history_for_transaction_ids(conn, profile, tx_ids)
        if include_edit_history
        else {}
    )
    edit_history_count = sum(len(events) for events in edit_history.values())
    if not include_edit_history and tx_ids:
        placeholders = ",".join("?" for _ in tx_ids)
        edit_history_count = int(
            conn.execute(
                f"""
                SELECT COUNT(*) AS count
                FROM transaction_edit_events
                WHERE profile_id = ? AND transaction_id IN ({placeholders})
                """,
                (profile["id"], *tx_ids),
            ).fetchone()["count"]
            or 0
        )
    for tx in tx_rows:
        tags = _tx_tags(conn, tx["id"])
        direct_attachments = [
            _attachment_summary(row, attachments_root, include_url=False)
            for row in _direct_attachment_rows(conn, profile["id"], tx["id"])
        ]
        links = (
            _source_funds_links_for_tx(conn, profile["id"], tx["id"], attachments_root)
            if include_review_state
            else []
        )
        quarantine = _journal_quarantine_for_tx(conn, profile["id"], tx["id"]) if include_journal_state else None
        warnings = _evidence_warnings(
            tx,
            tags,
            direct_attachments,
            links,
            readiness_freshness,
            quarantine,
            include_review_state=include_review_state,
        )
        if not include_review_state:
            warnings.append(
                _warning(
                    "review_state_excluded",
                    "info",
                    "Source-of-funds review state was excluded by export options.",
                )
            )
        if not include_journal_state:
            warnings.append(
                _warning(
                    "journal_state_excluded",
                    "info",
                    "Journal and quarantine state was excluded by export options.",
                )
            )
        for warning in warnings:
            warning_counts[warning["code"]] = warning_counts.get(warning["code"], 0) + 1
        item = {
            "transaction": _transaction_summary(tx, tags),
            "readiness": {
                "status": (
                    "blocked"
                    if any(warning["severity"] == "blocker" for warning in warnings)
                    else "warning"
                    if any(warning["severity"] == "warning" for warning in warnings)
                    else "ready"
                ),
                "warnings": warnings,
            },
            "direct_attachments": direct_attachments,
            "source_funds_links": links,
        }
        if include_journal_state:
            item["journal"] = {
                "entries": _journal_entries_for_tx(conn, profile["id"], tx["id"]),
                "quarantine": quarantine,
            }
        if include_edit_history:
            item["edit_history"] = edit_history.get(tx["id"], [])
        transactions.append(item)

    return {
        "schema_version": AUDIT_PACKAGE_SCHEMA_VERSION,
        "workspace": {"id": workspace["id"], "label": workspace["label"]},
        "profile": {"id": profile["id"], "label": profile["label"]},
        "scope": scope,
        "journal_freshness": freshness,
        "transactions": transactions,
        "summary": {
            "transaction_count": len(transactions),
            "ready_count": sum(1 for item in transactions if item["readiness"]["status"] == "ready"),
            "blocked_count": sum(1 for item in transactions if item["readiness"]["status"] == "blocked"),
            "warning_count": sum(1 for item in transactions if item["readiness"]["status"] == "warning"),
            "warning_codes": dict(sorted(warning_counts.items())),
            "edit_history_event_count": edit_history_count,
            "edit_history_included": bool(include_edit_history),
        },
        "excluded_sensitive_material": SENSITIVE_MATERIAL_EXCLUSIONS,
    }


def _collect_attachment_ids(summary: Mapping[str, Any], *, include_review_state: bool) -> set[str]:
    ids: set[str] = set()
    for item in summary.get("transactions", []):
        for attachment in item.get("direct_attachments", []):
            ids.add(attachment["id"])
        if not include_review_state:
            continue
        for link in item.get("source_funds_links", []):
            if link.get("state") != "reviewed":
                continue
            for attachment in link.get("attachments", []):
                ids.add(attachment["id"])
            source = link.get("from_source") or {}
            if isinstance(source, dict):
                for attachment in source.get("attachments", []):
                    ids.add(attachment["id"])
    return ids


def _copy_evidence_files(
    rows: Sequence[Mapping[str, Any]],
    attachments_root: Path,
    output_dir: Path,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    evidence_dir = output_dir / "evidence"
    included = []
    warnings = []
    for row in rows:
        if row["attachment_type"] != "file":
            continue
        stored_path, path_valid = _resolve_stored_path(attachments_root, row["stored_relpath"])
        if not path_valid or stored_path is None:
            warnings.append(
                _warning(
                    "attachment_storage_path_invalid",
                    "blocker",
                    f"Attachment {row['id']} has an invalid managed storage path.",
                )
            )
            continue
        if not stored_path.exists():
            warnings.append(
                _warning(
                    "attachment_file_missing",
                    "blocker",
                    f"Attachment {row['id']} points to a missing managed file.",
                )
            )
            continue
        size_bytes, sha256 = _hash_file(stored_path)
        if row["sha256"] and sha256 != row["sha256"]:
            warnings.append(
                _warning(
                    "attachment_hash_mismatch",
                    "blocker",
                    f"Attachment {row['id']} hash does not match the copied file.",
                )
            )
            continue
        display_label = attachment_display_label(row)
        safe_name = _safe_filename(row["original_filename"] or display_label)
        relpath = Path("evidence") / f"{row['id']}-{safe_name}"
        destination = output_dir / relpath
        evidence_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(stored_path, destination)
        included.append(
            {
                "attachment_id": row["id"],
                "label": display_label,
                "path": relpath.as_posix(),
                "sha256": sha256,
                "size_bytes": size_bytes,
                "media_type": row["media_type"] or "",
                "copied_from_attachment_id": row["copied_from_attachment_id"] or "",
                "copied_from_transaction_id": row["copied_from_transaction_id"] or "",
            }
        )
    return sorted(included, key=lambda item: item["attachment_id"]), warnings


def _url_references(rows: Sequence[Mapping[str, Any]], *, include_url_references: bool) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    references = []
    warnings = []
    if not include_url_references:
        if any(row["attachment_type"] == "url" for row in rows):
            warnings.append(
                _warning(
                    "url_references_excluded",
                    "info",
                    "URL references exist but were excluded by export options.",
                )
            )
        return [], warnings
    for row in rows:
        if row["attachment_type"] != "url":
            continue
        safety = _url_safety(row["source_url"])
        item = {
            "attachment_id": row["id"],
            "label": attachment_display_label(row),
            "host": safety.get("host", ""),
            "scheme": safety.get("scheme", ""),
            "redacted": bool(safety.get("redacted")),
            "copied_from_attachment_id": row["copied_from_attachment_id"] or "",
            "copied_from_transaction_id": row["copied_from_transaction_id"] or "",
        }
        if include_url_references:
            item["url"] = safety["url"]
            item["redacted_url"] = safety["redacted_url"] if safety["redacted"] else ""
        if safety["redacted"]:
            item["redaction_reason"] = safety["reason"]
            warnings.append(
                _warning(
                    "secret_bearing_url_redacted",
                    "warning",
                    f"URL reference {row['id']} was redacted because it contains {safety['reason']}.",
                )
            )
        references.append(item)
    return sorted(references, key=lambda item: item["attachment_id"]), warnings


def _without_url_attachment_summaries(summary: Mapping[str, Any]) -> dict[str, Any]:
    scrubbed = dict(summary)
    transactions = []
    for item in summary.get("transactions", []):
        tx_item = dict(item)
        tx_item["direct_attachments"] = [
            attachment
            for attachment in item.get("direct_attachments", [])
            if attachment.get("attachment_type") != "url"
        ]
        links = []
        for link in item.get("source_funds_links", []):
            link_item = dict(link)
            link_item["attachments"] = [
                attachment
                for attachment in link.get("attachments", [])
                if attachment.get("attachment_type") != "url"
            ]
            source = link.get("from_source")
            if isinstance(source, Mapping):
                source_item = dict(source)
                source_item["attachments"] = [
                    attachment
                    for attachment in source.get("attachments", [])
                    if attachment.get("attachment_type") != "url"
                ]
                link_item["from_source"] = source_item
            links.append(link_item)
        tx_item["source_funds_links"] = links
        transactions.append(tx_item)
    scrubbed["transactions"] = transactions
    return scrubbed


def export_audit_package(
    conn: sqlite3.Connection,
    data_root: str,
    workspace_ref: str | None,
    profile_ref: str | None,
    dir_path: str | Path,
    hooks: AuditPackageHooks,
    *,
    transaction_refs: Sequence[str] | None = None,
    source_funds_case_ref: str | None = None,
    include_copied_attachments: bool = True,
    include_url_references: bool = True,
    include_journal_state: bool = True,
    include_review_state: bool = True,
    include_edit_history: bool = False,
) -> dict[str, Any]:
    output_dir = Path(dir_path)
    if output_dir.exists() and any(output_dir.iterdir()):
        raise AppError("Audit package export directory already exists and is not empty", code="conflict")
    output_dir.mkdir(parents=True, exist_ok=True)
    summary = build_evidence_summary(
        conn,
        data_root,
        workspace_ref,
        profile_ref,
        hooks,
        transaction_refs=transaction_refs,
        source_funds_case_ref=source_funds_case_ref,
        include_journal_state=include_journal_state,
        include_review_state=include_review_state,
        include_edit_history=include_edit_history,
    )
    profile_id = summary["profile"]["id"]
    attachments_root = _attachments_root(data_root)
    attachment_rows = _attachment_rows_by_ids(
        conn,
        profile_id,
        sorted(_collect_attachment_ids(summary, include_review_state=include_review_state)),
    )
    evidence_files: list[dict[str, Any]] = []
    package_warnings: list[dict[str, Any]] = []
    if include_copied_attachments:
        evidence_files, copy_warnings = _copy_evidence_files(attachment_rows, attachments_root, output_dir)
        package_warnings.extend(copy_warnings)
    elif any(row["attachment_type"] == "file" for row in attachment_rows):
        package_warnings.append(
            _warning(
                "copied_attachments_excluded",
                "info",
                "Managed copied evidence files exist but were excluded by export options.",
            )
        )
    url_references, url_warnings = _url_references(
        attachment_rows,
        include_url_references=include_url_references,
    )
    package_warnings.extend(url_warnings)
    if not include_edit_history and summary["summary"].get("edit_history_event_count", 0):
        package_warnings.append(
            _warning(
                "edit_history_excluded",
                "info",
                "Transaction edit history exists but was excluded by export options.",
            )
        )
    package_warnings.append(
        _warning(
            "sensitive_material_excluded",
            "info",
            "Descriptors, xpubs, backend URLs, credentials, wallet files, logs, AI settings, unrelated books, and technical wallet evidence were excluded.",
        )
    )

    manifest_summary = (
        summary
        if include_url_references
        else _without_url_attachment_summaries(summary)
    )

    manifest = {
        **manifest_summary,
        "package": {
            "schema_version": AUDIT_PACKAGE_SCHEMA_VERSION,
            "generated_at": hooks.now_iso(),
            "options": {
                "include_copied_attachments": bool(include_copied_attachments),
                "include_url_references": bool(include_url_references),
                "include_journal_state": bool(include_journal_state),
                "include_review_state": bool(include_review_state),
                "include_edit_history": bool(include_edit_history),
            },
            "evidence_files": evidence_files,
            "url_references": url_references,
            "warnings": package_warnings,
        },
    }
    manifest_path = output_dir / "manifest.json"
    manifest_path.write_text(
        json.dumps(json_ready(manifest), indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return {
        "dir": str(output_dir.resolve()),
        "manifest": str(manifest_path.resolve()),
        "format": "directory",
        "scope": "audit_package",
        "filename": output_dir.name,
        "transaction_count": summary["summary"]["transaction_count"],
        "ready_count": summary["summary"]["ready_count"],
        "blocked_count": summary["summary"]["blocked_count"],
        "evidence_file_count": len(evidence_files),
        "url_reference_count": len(url_references),
        "warnings": package_warnings,
    }


__all__ = [
    "AUDIT_PACKAGE_SCHEMA_VERSION",
    "AuditPackageHooks",
    "SENSITIVE_MATERIAL_EXCLUSIONS",
    "build_evidence_summary",
    "export_audit_package",
]
