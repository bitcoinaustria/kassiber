"""Private owned-outpoint index and intra-wallet chain-structure lineage.

Source-of-funds asks a strictly larger question than the custody projection:
the projection books movements across custody boundaries, while provenance
must also explain a spend funded by the wallet's own earlier outputs. That
shape is not a transfer and the projection correctly has no opinion on it,
so it is derived here -- from observed transaction structure only, never by
re-interpreting anything the projection already decided.
"""

from __future__ import annotations

import sqlite3
from collections import defaultdict
from collections.abc import Callable, Mapping, Sequence
from typing import Any

from ..transfers import canonical_txid
from ..wallet_descriptors import normalize_asset_code, normalize_chain, normalize_network
from .custody_evidence import assess_authoritative_chain_observation
from .onchain import parse_ownership_tx, stored_tx_mapping
from .transfer_matching import onchain_transfer_scope


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


def _allocate_by_weight(weights: Sequence[int], target_sum: int) -> list[int]:
    """Integer weighted allocation whose shares sum exactly to target_sum."""
    if target_sum <= 0:
        return [0 for _weight in weights]
    total = sum(max(0, weight) for weight in weights)
    if total <= 0:
        return [0 for _weight in weights]
    allocations: list[int] = []
    floor_sum = 0
    remainders: list[int] = []
    for weight in weights:
        exact = max(0, weight) * target_sum
        floor_value = exact // total
        allocations.append(floor_value)
        floor_sum += floor_value
        remainders.append(exact % total)
    for index in sorted(
        range(len(weights)),
        key=lambda position: (-remainders[position], position),
    )[: target_sum - floor_sum]:
        allocations[index] += 1
    return allocations


def _owned_asset_identity(outpoint: OwnedOutpointKey, info: Mapping[str, Any]) -> str:
    """Consensus asset identity for one owned output, or ``""`` if unknown."""
    chain = outpoint[0]
    explicit = info.get("asset_identity")
    if explicit not in (None, ""):
        if chain == "liquid":
            return canonical_txid(explicit) or ""
        return normalize_asset_code(str(explicit))
    asset = normalize_asset_code(str(info.get("asset") or ""))
    if chain == "liquid":
        # A full 32-byte asset id is consensus identity; a ticker label is not.
        return canonical_txid(asset) or ""
    return asset or "BTC"


def _declared_inputs(raw_json: Any) -> int:
    """How many inputs the stored payload claims, before parsing."""
    outer = stored_tx_mapping(raw_json)
    if outer is None:
        return 0
    for candidate in (outer, outer.get("tx"), outer.get("ownership_graph")):
        if isinstance(candidate, Mapping) and isinstance(candidate.get("vin"), list):
            return len(candidate["vin"])
    return 0


def _row_is_authoritative(row: Mapping[str, Any]) -> bool:
    """Only a closed observation commitment may found lineage.

    Imported rows can carry a hand-written ``vin`` array; treating that as
    structure would let a CSV author its own provenance. The commitment hashes
    ``raw_json``, so anything not written by an authoritative observer apply
    fails closed here.
    """
    try:
        return bool(assess_authoritative_chain_observation(row).authoritative)
    except (AttributeError, KeyError, TypeError, ValueError):
        return False


def derive_parent_spend_pairs(
    rows: Sequence[Mapping[str, Any]],
    owned_index: Mapping[OwnedOutpointKey, Mapping[str, Any]],
    *,
    skip_row: Callable[[Mapping[str, Any]], bool],
) -> list[dict[str, Any]]:
    """Funding edges from a spend's own observed inputs to their parent rows.

    Scope is deliberately narrow. Only the intra-wallet ``parent_spend`` shape
    is derived: a spend consuming outputs that earlier transactions of the SAME
    wallet created (consolidations and change chains). Anything crossing a
    custody boundary belongs to the custody projection and is never derived
    here, so the two can never disagree.

    Every edge is all-or-nothing. If one input is foreign, unresolved, of a
    different wallet, or its parent leg is not itself an authoritative
    observation, the spend emits nothing at all and the honest
    ``missing_history`` finding stands. A partial cover would instead raise
    ``ambiguous_allocation``, which is strictly worse for the user: it has no
    one-click attestation path, while ``missing_history`` does.
    """
    rows_by_scope: dict[tuple[str, str, str], list[Mapping[str, Any]]] = defaultdict(list)
    for row in rows:
        scope = onchain_transfer_scope(row)
        if scope is not None:
            rows_by_scope[scope[:3]].append(row)

    # One flagged leg poisons every leg of the same physical transaction.
    blocked = {
        scope[:3]
        for row in rows
        if (scope := onchain_transfer_scope(row)) is not None and skip_row(row)
    }

    pairs: list[dict[str, Any]] = []
    for row in rows:
        if str(row["direction"]) != "outbound":
            continue
        scope = onchain_transfer_scope(row)
        if scope is None or scope[:3] in blocked:
            continue
        if not _row_is_authoritative(row):
            continue
        wallet_id = str(row["wallet_id"])
        # More than one stored leg for this wallet/direction is ambiguous
        # evidence about which leg the inputs funded.
        if len([
            other for other in rows_by_scope[scope[:3]]
            if str(other["wallet_id"]) == wallet_id
            and str(other["direction"]) == "outbound"
        ]) != 1:
            continue
        parsed = parse_ownership_tx(row["raw_json"])
        if parsed is None:
            continue
        inputs = list(parsed.get("inputs") or ())
        declared = _declared_inputs(row["raw_json"])
        # A truncated graph cannot prove complete ownership of the inputs.
        if not inputs or declared != len(inputs):
            continue

        chain, network = scope[0], scope[1]
        contributions: dict[str, int] = defaultdict(int)
        complete = True
        for entry in inputs:
            # parse_ownership_tx normalizes a vin to "txid:vout" plus resolved
            # value/script; the raw vin shape is not what comes back here.
            outpoint = str(entry.get("outpoint") or "")
            prev_txid, _, vout_text = outpoint.partition(":")
            prev_txid = canonical_txid(prev_txid)
            try:
                vout = int(vout_text)
                value_sats = int(entry["value_sats"])
            except (KeyError, TypeError, ValueError):
                complete = False
                break
            if prev_txid is None or vout < 0 or not str(entry.get("script") or ""):
                complete = False
                break
            info = owned_index.get((chain, network, prev_txid, vout))
            if info is None or info.get("ambiguous"):
                complete = False
                break
            if str(info.get("wallet_id")) != wallet_id:
                # Cross-wallet funding is a custody question, not ours.
                complete = False
                break
            if _owned_asset_identity((chain, network, prev_txid, vout), info) != _owned_asset_identity(
                (chain, network, prev_txid, vout), {"asset": row["asset"]}
            ):
                complete = False
                break
            contributions[prev_txid] += int(value_sats) * 1000
        if not complete or not contributions:
            continue

        parents = []
        for prev_txid, gross_msat in sorted(contributions.items()):
            parent = None
            for candidate in rows_by_scope.get((chain, network, prev_txid), ()):
                if (
                    str(candidate["wallet_id"]) == wallet_id
                    and str(candidate["direction"]) == "inbound"
                ):
                    parent = candidate if parent is None else False
            if not parent or not _row_is_authoritative(parent):
                parents = []
                break
            if gross_msat > int(parent["amount"] or 0):
                parents = []
                break
            parents.append((parent, gross_msat))
        if not parents:
            continue

        target_msat = int(row["amount"] or 0)
        if target_msat <= 0:
            continue
        shares = _allocate_by_weight([gross for _parent, gross in parents], target_msat)
        for (parent, gross_msat), share_msat in zip(parents, shares):
            if share_msat <= 0:
                continue
            pairs.append({
                "from_row": parent,
                "to_row": row,
                "allocation_msat": share_msat,
                "from_allocation_msat": gross_msat,
                "explanation": (
                    "This spend's observed inputs include outputs created by this "
                    "transaction in the same wallet."
                ),
            })
    return pairs


__all__ = [
    "OwnedOutpointKey",
    "build_owned_outpoint_index",
    "derive_parent_spend_pairs",
]
