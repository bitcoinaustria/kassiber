"""Private owned-outpoint index shared by local lineage/privacy readers."""

from __future__ import annotations

import sqlite3
from typing import Any

from ..transfers import canonical_txid
from ..wallet_descriptors import normalize_asset_code, normalize_chain, normalize_network
from .onchain import stored_tx_mapping


OwnedOutpointKey = tuple[str, str, str, int]


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    rows = conn.execute(f"PRAGMA table_info({table})").fetchall()
    columns: set[str] = set()
    for row in rows:
        try:
            columns.add(str(row["name"]))
        except (KeyError, TypeError, IndexError):
            columns.add(str(row[1]))
    return columns


def build_owned_outpoint_index(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[OwnedOutpointKey, dict[str, Any]]:
    """Return private owned outputs keyed by canonical physical outpoint."""

    columns = _table_columns(conn, "wallet_utxos")
    network_select = "network" if "network" in columns else "NULL AS network"
    raw_json_select = "raw_json" if "raw_json" in columns else "NULL AS raw_json"
    rows = conn.execute(
        f"""
        SELECT wallet_id, chain, {network_select}, txid, vout, amount,
               branch_label, spent_by, asset, {raw_json_select}
        FROM wallet_utxos
        WHERE profile_id = ?
        """,
        (profile_id,),
    ).fetchall()
    index: dict[OwnedOutpointKey, dict[str, Any]] = {}
    for row in rows:
        txid = canonical_txid(row["txid"])
        try:
            chain = normalize_chain(row["chain"])
            network = normalize_network(chain, row["network"])
            vout = int(row["vout"])
        except (TypeError, ValueError):
            continue
        if txid is None or vout < 0:
            continue
        asset = normalize_asset_code(str(row["asset"] or "BTC"))
        if chain == "liquid":
            raw_asset = (stored_tx_mapping(row["raw_json"]) or {}).get("asset_id")
            raw_asset_id = canonical_txid(raw_asset)
            display_asset_id = canonical_txid(asset)
            if raw_asset not in (None, "") and raw_asset_id is None:
                continue
            if raw_asset_id and display_asset_id and raw_asset_id != display_asset_id:
                continue
            asset_identity = raw_asset_id or display_asset_id
            if asset_identity is None:
                continue
        else:
            asset_identity = asset
        key = (chain, network, txid, vout)
        if key in index:
            index[key]["ambiguous"] = True
            continue
        index[key] = {
            "wallet_id": row["wallet_id"],
            "amount_msat": int(row["amount"] or 0),
            "branch_label": str(row["branch_label"] or ""),
            "spent_by": canonical_txid(row["spent_by"]) or "",
            "asset": asset,
            "asset_identity": asset_identity,
            "ambiguous": False,
        }
    return index


__all__ = ["OwnedOutpointKey", "build_owned_outpoint_index"]
