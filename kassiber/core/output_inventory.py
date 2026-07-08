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
SAMOURAI_SAFE_METADATA_FIELDS = {
    "role",
    "group_id",
    "group_label",
    "parent_wallet_id",
    "source",
    "section",
    "script_type",
    "root_path",
    "gap_limit",
    "privacy_boundary",
    "whirlpool",
    "toxic_change",
    "minimum_mix_count",
    "mix_count",
    "mix_count_confidence",
    "target_mix_count",
    "pool_denomination_sat",
    "coordinator_fee_sat",
    "miner_fee_sat",
    "round_txid",
    "round_txids",
    "tx0_role",
    "whirlpool_event",
    "privacy_event",
    "exit_kind",
    "ricochet_hops",
    "watch_only",
    "bip47",
    "paynym",
    "scanned_without_explicit_descriptor",
    "sections",
}
SAMOURAI_ENUM_VALUES = {
    "mix_count_confidence": {"minimum", "exact", "estimated", "unknown"},
    "tx0_role": {"deposit", "premix", "badbank", "fee"},
    "whirlpool_event": {
        "tx0",
        "premix_pending",
        "first_mix",
        "remix",
        "mix_to_wallet",
        "external_spend",
    },
    "privacy_event": {
        "coinjoin",
        "payjoin",
        "tx0",
        "first_mix",
        "remix",
        "ricochet",
        "exit",
    },
    "exit_kind": {"cold_storage", "external_spend", "ricochet", "toxic_change_spend"},
}
SAMOURAI_NON_NEGATIVE_INT_FIELDS = {
    "gap_limit",
    "minimum_mix_count",
    "mix_count",
    "target_mix_count",
    "pool_denomination_sat",
    "coordinator_fee_sat",
    "miner_fee_sat",
    "ricochet_hops",
}


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


def _normalize_bool(value: Any) -> bool | None:
    if value is None or value == "":
        return None
    if isinstance(value, bool):
        return value
    text = str(value).strip().lower()
    if text in {"1", "true", "yes", "on"}:
        return True
    if text in {"0", "false", "no", "off"}:
        return False
    return None


def _normalize_script_pubkey(value: Any) -> str | None:
    text = str_or_none(value)
    if text is None:
        return None
    normalized = text.lower()
    if len(normalized) % 2 != 0:
        return None
    try:
        bytes.fromhex(normalized)
    except ValueError:
        return None
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
    anonymity_score = _normalize_int(
        output.get("anonymity_score", output.get("anonymityScore")),
        "anonymity_score",
        minimum=0,
    )
    excluded_from_coinjoin = _normalize_bool(
        output.get("excluded_from_coinjoin", output.get("excludedFromCoinjoin"))
    )
    spent_by = str_or_none(output.get("spent_by", output.get("spentBy")))
    spent_flag = _normalize_bool(output.get("spent"))
    spent_at = str_or_none(output.get("spent_at"))
    if spent_at is None and (spent_by or spent_flag):
        spent_at = seen_at
    anon_history = output.get("anon_history", output.get("anonHistory", []))
    if not isinstance(anon_history, list):
        anon_history = []
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
        "script_pubkey": _normalize_script_pubkey(output.get("script_pubkey")),
        "address_label": str_or_none(output.get("address_label")),
        "branch_label": str_or_none(output.get("branch_label")),
        "branch_index": branch_index,
        "address_index": address_index,
        "anonymity_score": anonymity_score,
        "spent_by": spent_by,
        "excluded_from_coinjoin": (
            1 if excluded_from_coinjoin is True else 0 if excluded_from_coinjoin is False else None
        ),
        "key_state": str_or_none(output.get("key_state", output.get("keyState"))),
        "anon_history_json": json.dumps(json_ready(anon_history), sort_keys=True),
        "seen_at": seen_at,
        "spent_at": spent_at,
        "raw_json": json.dumps(json_ready(output.get("raw") or {}), sort_keys=True),
    }


def _safe_samourai_metadata_from_config(config_json: Any) -> dict[str, Any] | None:
    if not config_json:
        return None
    try:
        config = json.loads(config_json) if isinstance(config_json, str) else dict(config_json)
    except (TypeError, ValueError, json.JSONDecodeError):
        return None
    metadata = config.get("samourai") if isinstance(config, dict) else None
    if not isinstance(metadata, dict):
        return None
    safe = _safe_samourai_metadata(metadata)
    return safe or None


def _safe_samourai_metadata(metadata: Any) -> dict[str, Any]:
    if not isinstance(metadata, dict):
        return {}
    safe: dict[str, Any] = {}
    for key, value in metadata.items():
        if key not in SAMOURAI_SAFE_METADATA_FIELDS:
            continue
        normalized = _safe_samourai_metadata_value(key, value)
        if normalized is not None:
            safe[key] = normalized
    return safe


def _safe_samourai_metadata_value(key: str, value: Any) -> Any:
    if key in SAMOURAI_NON_NEGATIVE_INT_FIELDS:
        try:
            normalized = int(value)
        except (TypeError, ValueError):
            return None
        return normalized if normalized >= 0 else None
    if key in {"privacy_boundary", "whirlpool", "toxic_change", "watch_only", "paynym"}:
        return bool(value)
    if key == "round_txids":
        if not isinstance(value, list):
            return None
        txids = [_normalize_txid_or_none(item) for item in value]
        return [txid for txid in txids if txid is not None] or None
    if key == "round_txid":
        return _normalize_txid_or_none(value)
    if key == "sections":
        if not isinstance(value, list):
            return None
        sections = [str(item).strip().lower() for item in value if str(item).strip()]
        return sections or None
    if key in SAMOURAI_ENUM_VALUES:
        normalized = str(value or "").strip().lower()
        return normalized if normalized in SAMOURAI_ENUM_VALUES[key] else None
    if isinstance(value, str):
        normalized = value.strip()
        return normalized[:128] if normalized else None
    return value if value is None or isinstance(value, (int, bool)) else None


def _normalize_txid_or_none(value: Any) -> str | None:
    normalized = str(value or "").strip().lower()
    if len(normalized) != 64:
        return None
    try:
        bytes.fromhex(normalized)
    except ValueError:
        return None
    return normalized


def _with_samourai_raw_json(raw_json: str, metadata: dict[str, Any] | None) -> str:
    if not metadata:
        return raw_json
    try:
        raw = json.loads(raw_json or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        raw = {}
    if not isinstance(raw, dict):
        raw = {}
    merged = dict(metadata)
    merged.update(_safe_samourai_metadata(raw.get("samourai")))
    raw["samourai"] = merged
    return json.dumps(json_ready(raw), sort_keys=True)


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
    wallet_config_json = (
        wallet.get("config_json")
        if hasattr(wallet, "get")
        else wallet["config_json"]
        if hasattr(wallet, "keys") and "config_json" in wallet.keys()
        else None
    )
    samourai_metadata = _safe_samourai_metadata_from_config(wallet_config_json)
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
    active_outpoints = {row["outpoint"] for row in normalized if row["spent_at"] is None}
    if samourai_metadata:
        for row in normalized:
            row["raw_json"] = _with_samourai_raw_json(row["raw_json"], samourai_metadata)
    conn.executemany(
        """
        INSERT INTO wallet_utxos(
            id, workspace_id, profile_id, wallet_id, backend_name, backend_kind,
            chain, network, asset, amount, txid, vout, outpoint,
            confirmation_status, confirmations, block_height, block_time,
            address, script_pubkey, address_label, branch_label, branch_index, address_index,
            anonymity_score, spent_by, excluded_from_coinjoin, key_state,
            anon_history_json, first_seen_at, last_seen_at, spent_at, raw_json
        ) VALUES(
            ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?,
            ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?, ?,
            ?, ?, ?, ?
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
            script_pubkey = excluded.script_pubkey,
            address_label = excluded.address_label,
            branch_label = excluded.branch_label,
            branch_index = excluded.branch_index,
            address_index = excluded.address_index,
            anonymity_score = excluded.anonymity_score,
            spent_by = excluded.spent_by,
            excluded_from_coinjoin = excluded.excluded_from_coinjoin,
            key_state = excluded.key_state,
            anon_history_json = excluded.anon_history_json,
            last_seen_at = excluded.last_seen_at,
            spent_at = excluded.spent_at,
            raw_json = excluded.raw_json
        """,
        [
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
                row["script_pubkey"],
                row["address_label"],
                row["branch_label"],
                row["branch_index"],
                row["address_index"],
                row["anonymity_score"],
                row["spent_by"],
                row["excluded_from_coinjoin"],
                row["key_state"],
                row["anon_history_json"],
                timestamp,
                timestamp,
                row["spent_at"],
                row["raw_json"],
            )
            for row in normalized
        ],
    )
    if active_outpoints:
        placeholders = ", ".join("?" for _ in active_outpoints)
        spent_cursor = conn.execute(
            f"""
            UPDATE wallet_utxos
            SET spent_at = ?
            WHERE wallet_id = ?
              AND COALESCE(backend_name, '') = ?
              AND COALESCE(backend_kind, '') = ?
              AND chain = ?
              AND network = ?
              AND spent_at IS NULL
              AND outpoint NOT IN ({placeholders})
            """,
            (
                timestamp,
                wallet["id"],
                backend_name,
                backend_kind,
                chain,
                network,
                *sorted(active_outpoints),
            ),
        )
    else:
        spent_cursor = conn.execute(
            """
            UPDATE wallet_utxos
            SET spent_at = ?
            WHERE wallet_id = ?
              AND COALESCE(backend_name, '') = ?
              AND COALESCE(backend_kind, '') = ?
              AND chain = ?
              AND network = ?
              AND spent_at IS NULL
            """,
            (timestamp, wallet["id"], backend_name, backend_kind, chain, network),
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
            len(active_outpoints),
            timestamp,
        ),
    )
    if commit:
        conn.commit()
    return {
        "observed": len(normalized),
        "active": len(active_outpoints),
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


def clear_backend_output_inventory(
    conn: sqlite3.Connection,
    backend_name: str,
    *,
    commit: bool = True,
) -> dict[str, Any]:
    """Drop cached output inventory stamped with a backend that was changed."""
    normalized_backend = str(backend_name or "").strip().lower()
    if not normalized_backend:
        return {"utxos_deleted": 0, "refreshes_deleted": 0}
    utxo_cursor = conn.execute(
        "DELETE FROM wallet_utxos WHERE backend_name = ?",
        (normalized_backend,),
    )
    refresh_cursor = conn.execute(
        "DELETE FROM wallet_utxo_refreshes WHERE backend_name = ?",
        (normalized_backend,),
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
        backend_name=backend_name,
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


def wallet_unspent_outpoint_amounts(
    conn: sqlite3.Connection,
    wallet_id: str,
    *,
    backend_name: str | Sequence[str] | None = None,
    backend_kind: str | Sequence[str] | None = None,
    chain: str | Sequence[str] | None = None,
    network: str | Sequence[str] | None = None,
    assets: Sequence[str] | None = None,
) -> list[dict[str, Any]]:
    """Unspent amount per outpoint for a wallet, restricted to a source filter.

    Mirrors the source filtering used by ``wallet_output_inventory_totals`` so
    callers that dedupe outpoints across wallets only ever see the rows that
    actually feed a wallet's displayed chain balance. Rows left behind by an old
    backend/chain/network (for example after a ``backends update`` that does not
    clear ``wallet_utxos``) are excluded, exactly as they are for the balance.
    """
    source_where, source_params = _source_filter_sql(
        backend_name=backend_name,
        backend_kind=backend_kind,
        chain=chain,
        network=network,
    )
    asset_where = ""
    asset_params: list[Any] = []
    asset_values = [str(asset).upper() for asset in (assets or []) if str_or_none(asset)]
    if asset_values:
        placeholders = ", ".join("?" for _ in asset_values)
        asset_where = f"AND UPPER(asset) IN ({placeholders})"
        asset_params = sorted(set(asset_values))
    rows = conn.execute(
        f"""
        SELECT
            UPPER(asset) AS asset,
            COALESCE(NULLIF(outpoint, ''), lower(txid) || ':' || vout) AS outpoint_key,
            SUM(amount) AS amount_msat
        FROM wallet_utxos
        WHERE wallet_id = ?
          AND spent_at IS NULL
          {asset_where}
          {source_where}
        GROUP BY UPPER(asset), outpoint_key
        """,
        (wallet_id, *asset_params, *source_params),
    ).fetchall()
    return [
        {
            "asset": row["asset"],
            "outpoint_key": row["outpoint_key"],
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
        row_keys = set(row.keys()) if hasattr(row, "keys") else set()
        amount_msat = int(row["amount"])
        try:
            anon_history_json = row["anon_history_json"] if "anon_history_json" in row_keys else "[]"
            anon_history = json.loads(anon_history_json or "[]")
        except (TypeError, ValueError, json.JSONDecodeError):
            anon_history = []
        if not isinstance(anon_history, list):
            anon_history = []
        excluded_from_coinjoin = (
            row["excluded_from_coinjoin"] if "excluded_from_coinjoin" in row_keys else None
        )
        try:
            raw = json.loads(row["raw_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            raw = {}
        samourai = raw.get("samourai") if isinstance(raw, dict) else None
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
                "anonymity_score": (
                    row["anonymity_score"] if "anonymity_score" in row_keys else None
                ),
                "spent_by": row["spent_by"] if "spent_by" in row_keys and row["spent_by"] else "",
                "excluded_from_coinjoin": (
                    None
                    if excluded_from_coinjoin is None
                    else bool(excluded_from_coinjoin)
                ),
                "key_state": row["key_state"] if "key_state" in row_keys and row["key_state"] else "",
                "anon_history": anon_history,
                "source": {
                    "backend": row["backend_name"] or "",
                    "backend_kind": row["backend_kind"] or "",
                    "chain": row["chain"],
                    "network": row["network"],
                    "first_seen_at": row["first_seen_at"],
                    "last_seen_at": row["last_seen_at"],
                    "spent_at": row["spent_at"],
                },
                "samourai": samourai if isinstance(samourai, dict) else None,
            }
        )
    return output
