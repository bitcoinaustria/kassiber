"""Closed persisted provenance for authoritative chain observations."""

from __future__ import annotations

import hashlib
import json
import sqlite3
from typing import Any, Mapping, Sequence

from ...errors import AppError
from ...time_utils import now_iso


AUTHORITY_VERSION = 1


def _field(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    if hasattr(row, "keys") and key not in row.keys():
        return default
    if hasattr(row, "get"):
        return row.get(key, default)
    return row[key]


def canonical_graph_hash(raw_json: Any) -> str:
    """Hash canonical JSON; invalid/opaque text remains distinguishable."""

    if isinstance(raw_json, str):
        try:
            value = json.loads(raw_json or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            value = {"opaque_text": raw_json}
    else:
        value = raw_json
    encoded = json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        default=str,
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def canonical_observed_quantity_hash(row: Mapping[str, Any]) -> str:
    """Commit to the exact normalized quantity an observer authorized."""

    payload = {
        "schema_version": 1,
        "wallet_id": str(_field(row, "wallet_id") or ""),
        "external_id": str(_field(row, "external_id") or "").strip().lower(),
        "direction": str(_field(row, "direction") or "").strip().lower(),
        "asset": str(_field(row, "asset") or "").strip().upper(),
        "amount_msat": int(_field(row, "amount") or 0),
        "fee_msat": int(_field(row, "fee") or 0),
        "amount_includes_fee": bool(_field(row, "amount_includes_fee")),
    }
    encoded = json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def fee_attribution_from_raw(raw_json: Any) -> str:
    try:
        payload = json.loads(raw_json or "{}") if isinstance(raw_json, str) else raw_json
    except (TypeError, ValueError, json.JSONDecodeError):
        return "unknown"
    if not isinstance(payload, Mapping):
        return "unknown"
    component = payload.get("component")
    value = component.get("fee_attribution") if isinstance(component, Mapping) else None
    if value in {"exact", "implicit_wallet_delta"}:
        return str(value)
    # Bitcoin/Core observations have an explicit row fee when ownership of the
    # funding inputs is known. Lack of a Liquid component does not make those
    # exact fees ambiguous.
    if str(payload.get("observer") or "").lower() in {"bdk", "bitcoinrpc"}:
        return "exact"
    return "unknown"


def _record_key(record: Mapping[str, Any]) -> tuple[str, str, str]:
    return (
        str(record.get("txid") or record.get("external_id") or "").strip().lower(),
        str(record.get("asset") or "").strip().upper(),
        str(record.get("direction") or "").strip().lower(),
    )


def provenance_entries_for_facts(
    records_by_observer: Sequence[tuple[Any, Sequence[Mapping[str, Any]]]],
    final_records: Sequence[Mapping[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    """Build private record-key to structural-observer mappings."""

    sources: dict[tuple[str, str, str], dict[str, set[str]]] = {}
    for identity, records in records_by_observer:
        for record in records:
            key = _record_key(record)
            if not key[0]:
                continue
            item = sources.setdefault(key, {"ids": set(), "kinds": set()})
            item["ids"].add(str(identity.id))
            item["kinds"].add(str(identity.observer_kind))
    if final_records is not None:
        by_txid: dict[str, dict[str, set[str]]] = {}
        for key, value in sources.items():
            aggregate = by_txid.setdefault(key[0], {"ids": set(), "kinds": set()})
            aggregate["ids"].update(value["ids"])
            aggregate["kinds"].update(value["kinds"])
        projected: dict[tuple[str, str, str], dict[str, set[str]]] = {}
        for record in final_records:
            key = _record_key(record)
            source = sources.get(key) or by_txid.get(key[0])
            if key[0] and source is not None:
                projected[key] = source
        sources = projected
    return [
        {
            "external_id": key[0],
            "asset": key[1],
            "direction": key[2],
            "observer_ids": sorted(value["ids"]),
            "observer_kinds": sorted(value["kinds"]),
        }
        for key, value in sorted(sources.items())
    ]


def persist_chain_observation_provenance(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    *,
    application_revision: str,
    chain: str,
    network: str,
    entries: Sequence[Mapping[str, Any]],
) -> int:
    """Persist authority after normalized insertion, without committing."""

    persisted = 0
    timestamp = now_iso()
    for entry in entries:
        rows = conn.execute(
            """
            SELECT * FROM transactions
            WHERE profile_id = ? AND wallet_id = ? AND external_id = ?
              AND asset = ? AND direction = ?
            ORDER BY created_at DESC
            LIMIT 2
            """,
            (
                str(_field(profile, "id") or ""),
                str(_field(wallet, "id") or ""),
                str(entry.get("external_id") or ""),
                str(entry.get("asset") or ""),
                str(entry.get("direction") or ""),
            ),
        ).fetchall()
        if len(rows) != 1:
            raise AppError(
                "Authoritative observation did not resolve to one transaction row",
                code="observer_projection_conflict",
                details={
                    "external_id": str(entry.get("external_id") or ""),
                    "asset": str(entry.get("asset") or ""),
                    "direction": str(entry.get("direction") or ""),
                    "match_count": len(rows),
                },
                retryable=False,
            )
        row = rows[0]
        conn.execute(
            """
            INSERT INTO chain_observation_provenance(
                transaction_id, workspace_id, profile_id, wallet_id,
                authority_version, observer_ids_json, observer_kinds_json,
                chain, network, application_revision, graph_hash, quantity_hash,
                fee_attribution, observed_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(transaction_id) DO UPDATE SET
                authority_version = excluded.authority_version,
                observer_ids_json = excluded.observer_ids_json,
                observer_kinds_json = excluded.observer_kinds_json,
                chain = excluded.chain,
                network = excluded.network,
                application_revision = excluded.application_revision,
                graph_hash = excluded.graph_hash,
                quantity_hash = excluded.quantity_hash,
                fee_attribution = excluded.fee_attribution,
                observed_at = excluded.observed_at,
                updated_at = excluded.updated_at
            """,
            (
                str(row["id"]),
                str(row["workspace_id"]),
                str(row["profile_id"]),
                str(row["wallet_id"]),
                AUTHORITY_VERSION,
                json.dumps(sorted({str(value) for value in entry.get("observer_ids", ())})),
                json.dumps(sorted({str(value) for value in entry.get("observer_kinds", ())})),
                str(chain),
                str(network),
                str(application_revision),
                canonical_graph_hash(row["raw_json"]),
                canonical_observed_quantity_hash(row),
                fee_attribution_from_raw(row["raw_json"]),
                timestamp,
                timestamp,
            ),
        )
        persisted += 1
    return persisted


def row_has_current_authoritative_observation(row: Mapping[str, Any]) -> bool:
    """Fail closed unless persisted authority matches the current row exactly."""

    try:
        if int(_field(row, "observation_authority_version") or 0) != AUTHORITY_VERSION:
            return False
    except (TypeError, ValueError):
        return False
    return (
        str(_field(row, "observation_graph_hash") or "")
        == canonical_graph_hash(_field(row, "raw_json", "{}"))
        and str(_field(row, "observation_quantity_hash") or "")
        == canonical_observed_quantity_hash(row)
    )


__all__ = [
    "AUTHORITY_VERSION",
    "canonical_graph_hash",
    "canonical_observed_quantity_hash",
    "fee_attribution_from_raw",
    "persist_chain_observation_provenance",
    "provenance_entries_for_facts",
    "row_has_current_authoritative_observation",
]
