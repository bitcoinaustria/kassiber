from __future__ import annotations

import hashlib
import json
import sqlite3
from typing import Any, Mapping, Sequence

from ..envelope import json_ready
from ..errors import AppError
from ..msat import msat_to_btc
from ..time_utils import now_iso
from ..util import str_or_none
from ..wallet_descriptors import normalize_asset_code

DEFAULT_WALLET_OUTPUT_INVENTORY_LIMIT = 500


def _stable_utxo_id(profile_id: str, wallet_id: str, txid: str, vout: int) -> str:
    payload = f"{profile_id}:{wallet_id}:{txid}:{vout}".encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def _normalize_txid(value: Any) -> str:
    txid = str_or_none(value)
    if txid is None:
        raise AppError("UTXO record is missing txid", code="validation")
    normalized = txid.strip().lower()
    if len(normalized) != 64:
        raise AppError("UTXO record has an invalid txid", code="validation")
    try:
        bytes.fromhex(normalized)
    except ValueError as exc:
        raise AppError("UTXO record has an invalid txid", code="validation") from exc
    return normalized


def _normalize_int(value: Any, field: str, *, minimum: int | None = None) -> int | None:
    if value is None or value == "":
        return None
    try:
        normalized = int(value)
    except (TypeError, ValueError) as exc:
        raise AppError(f"UTXO record has an invalid {field}", code="validation") from exc
    if minimum is not None and normalized < minimum:
        raise AppError(f"UTXO record has an invalid {field}", code="validation")
    return normalized


def _normalize_observed_output(
    output: Mapping[str, Any],
    *,
    profile_id: str,
    wallet_id: str,
    backend_name: str,
    backend_kind: str,
    chain: str,
    network: str,
    seen_at: str,
) -> dict[str, Any]:
    txid = _normalize_txid(output.get("txid"))
    vout = _normalize_int(output.get("vout"), "vout", minimum=0)
    if vout is None:
        raise AppError("UTXO record is missing vout", code="validation")
    amount_sats = _normalize_int(output.get("amount_sats"), "amount_sats", minimum=0)
    if amount_sats is None:
        raise AppError("UTXO record is missing amount_sats", code="validation")
    block_height = _normalize_int(output.get("block_height"), "block_height")
    confirmations = _normalize_int(output.get("confirmations"), "confirmations", minimum=0)
    branch_index = _normalize_int(output.get("branch_index"), "branch_index")
    address_index = _normalize_int(output.get("address_index"), "address_index")
    status = str_or_none(output.get("confirmation_status")) or (
        "confirmed" if block_height and block_height > 0 else "mempool"
    )
    outpoint = f"{txid}:{vout}"
    return {
        "id": _stable_utxo_id(profile_id, wallet_id, txid, vout),
        "workspace_id": output.get("workspace_id"),
        "profile_id": profile_id,
        "wallet_id": wallet_id,
        "backend_name": backend_name,
        "backend_kind": backend_kind,
        "chain": str(output.get("chain") or chain),
        "network": str(output.get("network") or network),
        "asset": normalize_asset_code(output.get("asset") or "BTC"),
        "amount": int(amount_sats) * 1000,
        "txid": txid,
        "vout": vout,
        "outpoint": outpoint,
        "confirmation_status": status,
        "confirmations": confirmations,
        "block_height": block_height,
        "block_time": str_or_none(output.get("block_time")),
        "address": str_or_none(output.get("address")),
        "address_label": str_or_none(output.get("address_label")),
        "branch_label": str_or_none(output.get("branch_label")),
        "branch_index": branch_index,
        "address_index": address_index,
        "seen_at": seen_at,
        "raw_json": json.dumps(json_ready(output.get("raw") or {}), sort_keys=True),
    }


def update_wallet_output_inventory(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    backend: Mapping[str, Any],
    sync_state: Mapping[str, Any] | Any,
    observed_outputs: Sequence[Mapping[str, Any]],
    *,
    seen_at: str | None = None,
    commit: bool = True,
) -> dict[str, Any]:
    """Persist one wallet's latest source-backed unspent-output view."""
    timestamp = seen_at or now_iso()
    backend_name = str(backend.get("name") or "")
    backend_kind = str(backend.get("kind") or "")
    chain = str(getattr(sync_state, "chain", "") or "")
    network = str(getattr(sync_state, "network", "") or "")
    normalized = [
        _normalize_observed_output(
            output,
            profile_id=str(profile["id"]),
            wallet_id=str(wallet["id"]),
            backend_name=backend_name,
            backend_kind=backend_kind,
            chain=chain,
            network=network,
            seen_at=timestamp,
        )
        for output in observed_outputs
    ]
    seen_outpoints = {row["outpoint"] for row in normalized}
    for row in normalized:
        conn.execute(
            """
            INSERT INTO wallet_utxos(
                id, workspace_id, profile_id, wallet_id, backend_name, backend_kind,
                chain, network, asset, amount, txid, vout, outpoint,
                confirmation_status, confirmations, block_height, block_time,
                address, address_label, branch_label, branch_index, address_index,
                first_seen_at, last_seen_at, spent_at, raw_json
            ) VALUES(
                ?, ?, ?, ?, ?, ?,
                ?, ?, ?, ?, ?, ?, ?,
                ?, ?, ?, ?,
                ?, ?, ?, ?, ?,
                ?, ?, NULL, ?
            )
            ON CONFLICT(wallet_id, txid, vout) DO UPDATE SET
                backend_name = excluded.backend_name,
                backend_kind = excluded.backend_kind,
                chain = excluded.chain,
                network = excluded.network,
                asset = excluded.asset,
                amount = excluded.amount,
                outpoint = excluded.outpoint,
                confirmation_status = excluded.confirmation_status,
                confirmations = excluded.confirmations,
                block_height = excluded.block_height,
                block_time = excluded.block_time,
                address = excluded.address,
                address_label = excluded.address_label,
                branch_label = excluded.branch_label,
                branch_index = excluded.branch_index,
                address_index = excluded.address_index,
                last_seen_at = excluded.last_seen_at,
                spent_at = NULL,
                raw_json = excluded.raw_json
            """,
            (
                row["id"],
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                row["backend_name"],
                row["backend_kind"],
                row["chain"],
                row["network"],
                row["asset"],
                row["amount"],
                row["txid"],
                row["vout"],
                row["outpoint"],
                row["confirmation_status"],
                row["confirmations"],
                row["block_height"],
                row["block_time"],
                row["address"],
                row["address_label"],
                row["branch_label"],
                row["branch_index"],
                row["address_index"],
                timestamp,
                timestamp,
                row["raw_json"],
            ),
        )
    if seen_outpoints:
        placeholders = ", ".join("?" for _ in seen_outpoints)
        spent_cursor = conn.execute(
            f"""
            UPDATE wallet_utxos
            SET spent_at = ?
            WHERE wallet_id = ?
              AND spent_at IS NULL
              AND outpoint NOT IN ({placeholders})
            """,
            (timestamp, wallet["id"], *sorted(seen_outpoints)),
        )
    else:
        spent_cursor = conn.execute(
            """
            UPDATE wallet_utxos
            SET spent_at = ?
            WHERE wallet_id = ?
              AND spent_at IS NULL
            """,
            (timestamp, wallet["id"]),
        )
    conn.execute(
        """
        INSERT INTO wallet_utxo_refreshes(
            wallet_id, workspace_id, profile_id, backend_name, backend_kind,
            chain, network, observed_count, active_count, last_seen_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(wallet_id) DO UPDATE SET
            workspace_id = excluded.workspace_id,
            profile_id = excluded.profile_id,
            backend_name = excluded.backend_name,
            backend_kind = excluded.backend_kind,
            chain = excluded.chain,
            network = excluded.network,
            observed_count = excluded.observed_count,
            active_count = excluded.active_count,
            last_seen_at = excluded.last_seen_at
        """,
        (
            wallet["id"],
            profile["workspace_id"],
            profile["id"],
            backend_name,
            backend_kind,
            chain,
            network,
            len(normalized),
            len(seen_outpoints),
            timestamp,
        ),
    )
    if commit:
        conn.commit()
    return {
        "observed": len(normalized),
        "active": len(seen_outpoints),
        "spent": int(spent_cursor.rowcount or 0),
        "last_seen_at": timestamp,
    }


def clear_wallet_output_inventory(
    conn: sqlite3.Connection,
    wallet_id: str,
    *,
    commit: bool = True,
) -> dict[str, Any]:
    """Drop cached output inventory for a wallet after watch-target changes."""
    utxo_cursor = conn.execute("DELETE FROM wallet_utxos WHERE wallet_id = ?", (wallet_id,))
    refresh_cursor = conn.execute(
        "DELETE FROM wallet_utxo_refreshes WHERE wallet_id = ?",
        (wallet_id,),
    )
    if commit:
        conn.commit()
    return {
        "utxos_deleted": int(utxo_cursor.rowcount or 0),
        "refreshes_deleted": int(refresh_cursor.rowcount or 0),
    }


def _filter_values(value: str | Sequence[str] | None) -> list[str]:
    values: Sequence[str | None]
    if value is None:
        return []
    if isinstance(value, str):
        values = [value]
    else:
        values = value
    normalized = []
    for item in values:
        text = str_or_none(item)
        if text and text not in normalized:
            normalized.append(text)
    return normalized


def _append_source_filter(
    clauses: list[str],
    params: list[Any],
    column: str,
    value: str | Sequence[str] | None,
) -> None:
    values = _filter_values(value)
    if not values:
        return
    if len(values) == 1:
        clauses.append(f"{column} = ?")
        params.append(values[0])
        return
    placeholders = ", ".join("?" for _ in values)
    clauses.append(f"{column} IN ({placeholders})")
    params.extend(values)


def _source_filter_sql(
    *,
    prefix: str = "",
    backend_name: str | Sequence[str] | None = None,
    backend_kind: str | Sequence[str] | None = None,
    chain: str | Sequence[str] | None = None,
    network: str | Sequence[str] | None = None,
) -> tuple[str, list[Any]]:
    clauses: list[str] = []
    params: list[Any] = []
    _append_source_filter(clauses, params, f"{prefix}backend_name", backend_name)
    _append_source_filter(clauses, params, f"{prefix}backend_kind", backend_kind)
    _append_source_filter(clauses, params, f"{prefix}chain", chain)
    _append_source_filter(clauses, params, f"{prefix}network", network)
    if not clauses:
        return "", params
    return " AND " + " AND ".join(clauses), params


def _refresh_filter_sql(
    *,
    prefix: str = "",
    backend_kind: str | Sequence[str] | None = None,
    chain: str | Sequence[str] | None = None,
    network: str | Sequence[str] | None = None,
) -> tuple[str, list[Any]]:
    clauses: list[str] = []
    params: list[Any] = []
    _append_source_filter(clauses, params, f"{prefix}backend_kind", backend_kind)
    _append_source_filter(clauses, params, f"{prefix}chain", chain)
    _append_source_filter(clauses, params, f"{prefix}network", network)
    if not clauses:
        return "", params
    return " AND " + " AND ".join(clauses), params


def wallet_output_inventory_summary(
    conn: sqlite3.Connection,
    wallet_id: str,
    *,
    backend_name: str | Sequence[str] | None = None,
    backend_kind: str | Sequence[str] | None = None,
    chain: str | Sequence[str] | None = None,
    network: str | Sequence[str] | None = None,
) -> dict[str, Any]:
    source_where, source_params = _source_filter_sql(
        backend_name=backend_name,
        backend_kind=backend_kind,
        chain=chain,
        network=network,
    )
    refresh_where, refresh_params = _refresh_filter_sql(
        backend_kind=backend_kind,
        chain=chain,
        network=network,
    )
    active_row = conn.execute(
        f"""
        SELECT
            COUNT(*) AS active_count,
            MAX(last_seen_at) AS active_last_seen_at
        FROM wallet_utxos
        WHERE wallet_id = ?
          AND spent_at IS NULL
          {source_where}
        """,
        (wallet_id, *source_params),
    ).fetchone()
    refresh_row = conn.execute(
        f"""
        SELECT observed_count, active_count, last_seen_at
        FROM wallet_utxo_refreshes
        WHERE wallet_id = ?
          {refresh_where}
        """,
        (wallet_id, *refresh_params),
    ).fetchone()
    spent_row = conn.execute(
        f"""
        SELECT MAX(spent_at) AS last_spent_at
        FROM wallet_utxos
        WHERE wallet_id = ?
          AND spent_at IS NOT NULL
          {source_where}
        """,
        (wallet_id, *source_params),
    ).fetchone()
    return {
        "active_count": int(active_row["active_count"] or 0) if active_row else 0,
        "observed_count": (
            int(refresh_row["observed_count"] or 0) if refresh_row else 0
        ),
        "last_seen_at": (
            refresh_row["last_seen_at"]
            if refresh_row
            else active_row["active_last_seen_at"] if active_row else None
        ),
        "last_spent_at": spent_row["last_spent_at"] if spent_row else None,
    }


def wallet_output_inventory_totals(
    conn: sqlite3.Connection,
    wallet_id: str,
    *,
    include_spent: bool = False,
    backend_name: str | Sequence[str] | None = None,
    backend_kind: str | Sequence[str] | None = None,
    chain: str | Sequence[str] | None = None,
    network: str | Sequence[str] | None = None,
) -> list[dict[str, Any]]:
    where_spent = "" if include_spent else "AND spent_at IS NULL"
    source_where, source_params = _source_filter_sql(
        backend_name=backend_name,
        backend_kind=backend_kind,
        chain=chain,
        network=network,
    )
    rows = conn.execute(
        f"""
        SELECT asset, SUM(amount) AS amount_msat
        FROM wallet_utxos
        WHERE wallet_id = ?
          {where_spent}
          {source_where}
        GROUP BY asset
        ORDER BY asset ASC
        """,
        (wallet_id, *source_params),
    ).fetchall()
    return [
        {
            "asset": row["asset"],
            "amount": msat_to_btc(int(row["amount_msat"] or 0)),
            "amount_sat": int(row["amount_msat"] or 0) // 1000,
            "amount_msat": int(row["amount_msat"] or 0),
        }
        for row in rows
    ]


def list_wallet_output_inventory(
    conn: sqlite3.Connection,
    wallet_id: str,
    *,
    include_spent: bool = False,
    backend_name: str | Sequence[str] | None = None,
    backend_kind: str | Sequence[str] | None = None,
    chain: str | Sequence[str] | None = None,
    network: str | Sequence[str] | None = None,
    limit: int | None = None,
) -> list[dict[str, Any]]:
    where_spent = "" if include_spent else "AND spent_at IS NULL"
    source_where, source_params = _source_filter_sql(
        backend_name=backend_name,
        backend_kind=backend_kind,
        chain=chain,
        network=network,
    )
    limit_clause = ""
    limit_params: list[Any] = []
    if limit is not None:
        normalized_limit = int(limit)
        if normalized_limit <= 0:
            raise AppError("UTXO inventory limit must be positive", code="validation")
        limit_clause = "LIMIT ?"
        limit_params.append(normalized_limit)
    rows = conn.execute(
        f"""
        SELECT
            u.*,
            (
                SELECT t.id
                FROM transactions t
                WHERE t.profile_id = u.profile_id
                  AND t.wallet_id = u.wallet_id
                  AND lower(t.external_id) = lower(u.txid)
                ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
                LIMIT 1
            ) AS transaction_id
        FROM wallet_utxos u
        WHERE u.wallet_id = ?
          {where_spent}
          {source_where}
        ORDER BY
          asset ASC,
          CASE WHEN block_height IS NULL OR block_height <= 0 THEN 1 ELSE 0 END ASC,
          block_height ASC,
          txid ASC,
          vout ASC
        {limit_clause}
        """,
        (wallet_id, *source_params, *limit_params),
    ).fetchall()
    output = []
    for row in rows:
        amount_msat = int(row["amount"])
        output.append(
            {
                "id": row["id"],
                "transaction_id": row["transaction_id"] or "",
                "outpoint": row["outpoint"],
                "txid": row["txid"],
                "vout": int(row["vout"]),
                "asset": row["asset"],
                "amount": msat_to_btc(amount_msat),
                "amount_sat": amount_msat // 1000,
                "amount_msat": amount_msat,
                "confirmation_status": row["confirmation_status"],
                "confirmations": row["confirmations"],
                "block_height": row["block_height"],
                "block_time": row["block_time"],
                "address": row["address"] or "",
                "address_label": row["address_label"] or "",
                "branch_label": row["branch_label"] or "",
                "branch_index": row["branch_index"],
                "address_index": row["address_index"],
                "source": {
                    "backend": row["backend_name"] or "",
                    "backend_kind": row["backend_kind"] or "",
                    "chain": row["chain"],
                    "network": row["network"],
                    "first_seen_at": row["first_seen_at"],
                    "last_seen_at": row["last_seen_at"],
                    "spent_at": row["spent_at"],
                },
            }
        )
    return output
