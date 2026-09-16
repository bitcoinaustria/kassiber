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



# A change or consolidation output is never its own row: the observer stores one
# NET row per wallet per transaction, so a self-consolidation becomes an
# outbound fee row and change is netted out of a withdrawal's amount. Spending
# such an output therefore has to pass THROUGH that row to the inbound rows
# behind it.
# Bounded fresh work per TARGET spend. A split/recombine graph can reach the
# same ancestors from many directions; memoisation collapses that, and this is
# the backstop for anything it cannot. There is deliberately no limit on chain
# LENGTH: a long change chain is still fully observed history, and refusing it
# would report a misleading "no root source" for lineage the book can prove.
_MAX_ANCESTOR_RESOLUTIONS_PER_TARGET = 5000


def _owned_inputs(
    row: Mapping[str, Any],
    chain: str,
    network: str,
    owned_index: Mapping[OwnedOutpointKey, Mapping[str, Any]],
) -> list[tuple[str, int, int]] | None:
    """Return (txid, vout, msat) for every input, or None unless ALL are owned.

    One helper for every traversed transaction, direct or intermediate. An
    earlier version checked inventory only on the spend's own inputs, so an
    input contradicted by inventory could still be adopted one hop deeper --
    ownership has to be vetoed at every hop or it is not vetoed at all.
    """
    parsed = parse_ownership_tx(row["raw_json"])
    if parsed is None:
        return None
    entries = list(parsed.get("inputs") or ())
    if not entries or _declared_inputs(row["raw_json"]) != len(entries):
        # A truncated graph cannot prove complete ownership of the inputs.
        return None
    outer = stored_tx_mapping(row["raw_json"]) or {}
    owned_scripts = {
        str(script) for script in (outer.get("observer_owned_scripts") or ()) if script
    }
    wallet_id = str(row["wallet_id"])
    asset_identity = _owned_asset_identity((chain, "", "", 0), {"asset": row["asset"]})
    resolved: list[tuple[str, int, int]] = []
    for entry in entries:
        # parse_ownership_tx normalizes a vin to "txid:vout" plus resolved
        # value/script; the raw vin shape is not what comes back here.
        outpoint = str(entry.get("outpoint") or "")
        prev_txid, _, vout_text = outpoint.partition(":")
        prev_txid = canonical_txid(prev_txid)
        try:
            vout = int(vout_text)
            value_sats = int(entry["value_sats"])
        except (KeyError, TypeError, ValueError):
            return None
        script = str(entry.get("script") or "")
        if prev_txid is None or vout < 0 or not script:
            return None
        if script not in owned_scripts and str(entry.get("role") or "") != "owned":
            # A foreign input: this spend is collaborative or externally
            # funded, and nothing about it is ours to claim.
            return None
        info = owned_index.get((chain, network, prev_txid, vout))
        if info is not None:
            # Inventory corroborates when it has the outpoint; a disagreement
            # about ownership is a reason to stop, never to overrule the
            # observation.
            if info.get("ambiguous") or str(info.get("wallet_id")) != wallet_id:
                return None
            if _owned_asset_identity(
                (chain, network, prev_txid, vout), info
            ) != asset_identity:
                return None
        resolved.append((prev_txid, vout, value_sats * 1000))
    return resolved


def _scale_ancestors(
    ancestors: Sequence[tuple[Mapping[str, Any], int]], value_msat: int
) -> list[tuple[Mapping[str, Any], int]]:
    """Carry `value_msat` down an ancestor distribution, conserving the total."""
    shares = _allocate_by_weight([msat for _row, msat in ancestors], value_msat)
    return [
        (row, share) for (row, _msat), share in zip(ancestors, shares) if share > 0
    ]


def _ancestor_distribution(
    txid: str,
    *,
    wallet_id: str,
    chain: str,
    network: str,
    rows_by_scope: Mapping[tuple[str, str, str], Sequence[Mapping[str, Any]]],
    owned_index: Mapping[OwnedOutpointKey, Mapping[str, Any]],
    blocked: frozenset[tuple[str, str, str]],
    memo: dict[tuple[str, str, str, str], list[tuple[Mapping[str, Any], int]] | None],
    budget: list[int],
) -> list[tuple[Mapping[str, Any], int]] | None:
    """Every inbound row behind one transaction, with the value each carries.

    Resolved once per (wallet, scope, transaction) rather than once per
    referencing input: a wallet that splits and recombines would otherwise
    revisit the same ancestors combinatorially. The key includes the wallet on
    purpose -- one batched transaction pays several wallets, and a txid-only
    memo handed wallet B the ancestry resolved for wallet A, which is the one
    thing this deriver must never do.

    Iterative, with an explicit stack: a long change chain is legitimate
    history, and its length must not be able to overflow the interpreter --
    which is what happened when rows arrived in reverse order and every hop was
    a fresh recursion. Returns None whenever the chain cannot be resolved
    completely, so a partial answer is never mistaken for a whole one.
    """

    def key_for(t: str) -> tuple[str, str, str, str]:
        return (wallet_id, chain, network, t)

    if key_for(txid) in memo:
        return memo[key_for(txid)]

    # frame: [txid, path, inputs, next_index, carried]
    #   inputs is None until the frame's first visit classifies the row.
    stack: list[list[Any]] = [[txid, frozenset(), None, 0, {}]]

    def finish(frame: list[Any], result: list[tuple[Mapping[str, Any], int]] | None) -> None:
        memo[key_for(frame[0])] = result
        stack.pop()

    while stack:
        frame = stack[-1]
        f_txid, path, inputs, index, carried = frame
        if inputs is None:
            budget[0] -= 1
            if budget[0] < 0:
                # A pathological graph must cost a bounded amount and fail closed.
                finish(frame, None)
                continue
            if (chain, network, f_txid) in blocked:
                # A CoinJoin/PayJoin hop keeps its deferred semantics. Passing
                # lineage through one would assert exactly the participant
                # linkage the privacy boundary exists to refuse.
                finish(frame, None)
                continue
            candidates = rows_by_scope.get((chain, network, f_txid), ())
            inbound = [r for r in candidates if str(r["wallet_id"]) == wallet_id
                       and str(r["direction"]) == "inbound"]
            if len(inbound) > 1:
                # More than one inbound leg for this wallet is ambiguous evidence.
                finish(frame, None)
                continue
            if inbound:
                parent = inbound[0]
                finish(frame, (
                    [(parent, int(parent["amount"] or 0))]
                    if _row_is_authoritative(parent) and int(parent["amount"] or 0) > 0
                    else None
                ))
                continue
            outbound = [r for r in candidates if str(r["wallet_id"]) == wallet_id
                        and str(r["direction"]) == "outbound"]
            if len(outbound) != 1 or not _row_is_authoritative(outbound[0]):
                finish(frame, None)
                continue
            resolved_inputs = _owned_inputs(outbound[0], chain, network, owned_index)
            if not resolved_inputs:
                finish(frame, None)
                continue
            frame[2] = resolved_inputs
            continue

        if index < len(inputs):
            prev_txid, _vout, msat = inputs[index]
            child_key = key_for(prev_txid)
            if child_key not in memo:
                if prev_txid == f_txid or prev_txid in path:
                    # Path-local, so a diamond still resolves while a cycle cannot.
                    memo[child_key] = None
                else:
                    stack.append([prev_txid, path | {f_txid}, None, 0, {}])
                    continue
            upstream = memo[child_key]
            if upstream is None:
                finish(frame, None)
                continue
            for ancestor, share in _scale_ancestors(upstream, msat):
                entry = carried.setdefault(str(ancestor["id"]), [ancestor, 0])
                entry[1] += share
            frame[3] = index + 1
            continue

        finish(frame, [
            (ancestor, msat) for ancestor, msat in sorted(
                carried.values(), key=lambda item: str(item[0]["id"]),
            )
        ] if carried else None)

    return memo[key_for(txid)]


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

    # Shared across every spend in this pass: a wallet that splits and
    # recombines reaches the same ancestors from many directions. Keyed by
    # wallet as well as transaction -- see _ancestor_distribution.
    memo: dict[tuple[str, str, str, str], list[tuple[Mapping[str, Any], int]] | None] = {}

    pairs: list[dict[str, Any]] = []
    for row in rows:
        if str(row["direction"]) != "outbound":
            continue
        # Budget is per target, not per pass. A pass-wide budget let thousands
        # of unrelated spends exhaust it before a fully resolvable target was
        # reached, misreporting available history as missing -- and which
        # target lost depended on row order. Memo hits cost nothing, so a
        # shared ancestor resolved for one target is free for the next.
        budget = [_MAX_ANCESTOR_RESOLUTIONS_PER_TARGET]
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
        chain, network = scope[0], scope[1]
        # The spend attests which scripts its wallet watched. That is the
        # ownership proof, not the UTXO inventory: inventory is built from the
        # backend's CURRENT unspent set, so a wallet first synced after these
        # outputs were already spent has no row for any of them, and the
        # lineage would be invisible exactly when the history is longest.
        inputs = _owned_inputs(row, chain, network, owned_index)
        if not inputs:
            continue

        contributions: dict[str, int] = defaultdict(int)
        for prev_txid, _vout, msat in inputs:
            contributions[prev_txid] += msat

        by_parent: dict[str, list[Any]] = {}
        resolved_all = True
        for prev_txid, gross_msat in sorted(contributions.items()):
            upstream = _ancestor_distribution(
                prev_txid, wallet_id=wallet_id, chain=chain, network=network,
                rows_by_scope=rows_by_scope, owned_index=owned_index,
                blocked=frozenset(blocked), memo=memo, budget=budget,
            )
            if upstream is None:
                resolved_all = False
                break
            for ancestor, share in _scale_ancestors(upstream, gross_msat):
                entry = by_parent.setdefault(str(ancestor["id"]), [ancestor, 0])
                entry[1] += share
        if not resolved_all or not by_parent:
            continue
        parents = [(parent, msat) for parent, msat in sorted(
            by_parent.values(), key=lambda item: str(item[0]["id"]),
        )]
        if any(msat > int(parent["amount"] or 0) for parent, msat in parents):
            continue
        passthrough = any(
            str(parent["external_id"] or "").lower() not in contributions
            for parent, _msat in parents
        )

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
                    (
                        "This spend's observed inputs trace back to this transaction "
                        "through intermediate transactions in the same wallet. Value is "
                        "attributed across those hops in proportion to the amounts they "
                        "carried, which conserves the total but does not identify which "
                        "individual coins moved."
                    )
                    if passthrough
                    else (
                        "This spend's observed inputs include outputs created by this "
                        "transaction in the same wallet."
                    )
                ),
            })
    return pairs


__all__ = [
    "OwnedOutpointKey",
    "build_owned_outpoint_index",
    "derive_parent_spend_pairs",
]
