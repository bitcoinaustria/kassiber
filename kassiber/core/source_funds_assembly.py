"""Deterministic cross-layer evidence derivation for source-of-funds assembly.

This module turns data Kassiber already holds locally into provable funding
edges between transaction rows, so the source-of-funds graph can be
assembled automatically instead of hand-linked:

- ``utxo_spend``: real transaction structure. Chain-synced rows store their
  inputs (``raw_json`` vin outpoints), and ``wallet_utxos`` records which
  outputs the user's wallets own (plus ``spent_by`` for Wasabi imports).
  Joining a transaction's inputs against owned outputs proves, per wallet:
  which earlier owned transaction funded this spend (same-wallet parent
  chaining through change/consolidations, including net-zero in-wallet
  consolidation hops, which are resolved transparently to their own
  parents), and which wallet received which share of a multi-wallet
  transaction.
- ``payment_hash``: Lightning evidence. Two owned rows sharing one payment
  hash (an LN payment between own wallets, or an on-chain HTLC leg whose
  hash was extracted at import) are two legs of the same transfer.

Allocation semantics follow the engine's exact-cover rule: the parent edges
feeding one spend leg are sized as a group so they sum to exactly
``min(total contributed, spend leg amount)`` — per-edge capping would
over-cover multi-input consolidations by the fee and trip the
``ambiguous_allocation`` export gate.

Everything here is pure derivation over already-imported rows: no network
access, ever. More synced wallets and connection types mean more joins and
a more complete assembled graph — that scaling property is the point.

The module stays free of back-edges into ``source_funds``: it consumes
plain row mappings and returns candidate-pair dicts; the caller owns link
insertion, dedupe, and review semantics. The caller's ``skip_row``
predicate carries privacy/samourai policy and is applied per TRANSACTION:
one flagged leg poisons every leg of that txid, so lineage is never
asserted through a privacy boundary via an unflagged sibling row.
"""

from __future__ import annotations

import json
import sqlite3
from collections import defaultdict
from typing import Any, Callable, Mapping, Sequence

from ..wallet_descriptors import normalize_asset_code

_COINBASE_TXID = "0" * 64
# In-wallet consolidations net to ~0 and cannot carry allocation demand;
# resolving through more than this many of them in a row means the chain
# shape is unexpected and manual review is the right answer.
_MAX_PASSTHROUGH_DEPTH = 8


def _safe_json_loads(value: Any) -> Any:
    if not value:
        return None
    try:
        return json.loads(str(value))
    except (TypeError, ValueError):
        return None


def parse_vin_outpoints(raw_json: Any) -> list[tuple[str, int]]:
    """Extract input outpoints ``(prev_txid_lower, vout)`` from a stored tx.

    Handles the stored shapes that carry structured inputs: esplora's
    upstream tx JSON, electrum's locally-decoded transaction, and Liquid
    component records (all carry ``vin`` entries with ``txid`` + ``vout``).
    Rows without chain structure (CSV imports, bitcoinrpc sync) yield an
    empty list.
    """
    payload = _safe_json_loads(raw_json)
    if not isinstance(payload, dict):
        return []
    vin = payload.get("vin")
    if not isinstance(vin, list):
        nested = payload.get("tx")
        vin = nested.get("vin") if isinstance(nested, dict) else None
    if not isinstance(vin, list):
        return []
    outpoints: list[tuple[str, int]] = []
    for entry in vin:
        if not isinstance(entry, Mapping):
            continue
        txid = str(entry.get("txid") or "").strip().lower()
        if not txid or txid == _COINBASE_TXID:
            continue
        try:
            vout = int(entry.get("vout"))
        except (TypeError, ValueError):
            continue
        if vout < 0:
            continue
        outpoints.append((txid, vout))
    return outpoints


def build_owned_outpoint_index(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[tuple[str, int], dict[str, Any]]:
    """Owned outputs of this profile keyed by ``(creating_txid_lower, vout)``.

    ``amount_msat`` comes straight from the inventory (stored in msat);
    ``spent_by`` is the spending txid when the importer knew it (Wasabi).
    """
    index: dict[tuple[str, int], dict[str, Any]] = {}
    rows = conn.execute(
        """
        SELECT wallet_id, txid, vout, amount, branch_label, spent_by, asset
        FROM wallet_utxos
        WHERE profile_id = ?
        """,
        (profile_id,),
    ).fetchall()
    for row in rows:
        txid = str(row["txid"] or "").strip().lower()
        if not txid:
            continue
        key = (txid, int(row["vout"]))
        if key in index:
            index[key]["ambiguous"] = True
            continue
        index[key] = {
            "wallet_id": row["wallet_id"],
            "amount_msat": int(row["amount"] or 0),
            "branch_label": str(row["branch_label"] or ""),
            "spent_by": str(row["spent_by"] or "").strip().lower(),
            "asset": normalize_asset_code(str(row["asset"] or "")),
            "ambiguous": False,
        }
    return index


def _short_outpoints(outpoints: Sequence[tuple[str, int]], limit: int = 3) -> str:
    rendered = [f"{txid[:12]}…:{vout}" for txid, vout in outpoints[:limit]]
    if len(outpoints) > limit:
        rendered.append(f"+{len(outpoints) - limit} more")
    return ", ".join(rendered)


def _event_time(row: Mapping[str, Any]) -> str:
    return str(row["occurred_at"] or "")


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


def _allocate_pro_rata(amounts: Sequence[int], target_sum: int) -> list[int]:
    """Integer pro-rata allocation whose shares sum exactly to target_sum."""
    total = sum(amount for amount in amounts if amount > 0)
    target_sum = min(max(0, target_sum), total)
    if total <= target_sum:
        return [max(0, amount) for amount in amounts]
    return _allocate_by_weight(amounts, target_sum)


def derive_utxo_spend_pairs(
    rows: Sequence[Mapping[str, Any]],
    owned_index: Mapping[tuple[str, int], Mapping[str, Any]],
    *,
    skip_row: Callable[[Mapping[str, Any]], bool],
) -> list[dict[str, Any]]:
    """Derive exact funding edges from transaction input/output structure.

    Two edge kinds come out of one pass:

    - ``parent_spend``: within one wallet, spend transaction T consumes
      outputs created by earlier owned transactions (consolidations and
      change chains). Net-zero in-wallet consolidation legs are resolved
      transparently: their own parents become the edge sources, so chains
      like P -> consolidation -> S link P directly to S.
    - ``leg_funding``: across wallets, the outbound leg of T in the paying
      wallet funds the inbound leg of T in a receiving wallet that owns
      outputs of T. This resolves multi-wallet transactions that the 1:1
      same-external-id heuristic refuses.
    """
    rows_by_external: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for row in rows:
        external = str(row["external_id"] or "").strip().lower()
        if external:
            rows_by_external[external].append(row)

    # Privacy/samourai policy applies per transaction: any flagged leg
    # poisons every leg of the same txid.
    blocked_externals = {
        str(row["external_id"] or "").strip().lower()
        for row in rows
        if row["external_id"] and skip_row(row)
    }

    owned_outputs_by_txid: dict[str, list[tuple[tuple[str, int], Mapping[str, Any]]]] = defaultdict(list)
    spenders_of_outpoint: dict[str, list[tuple[tuple[str, int], Mapping[str, Any]]]] = defaultdict(list)
    for outpoint, info in owned_index.items():
        if info.get("ambiguous"):
            continue
        owned_outputs_by_txid[outpoint[0]].append((outpoint, info))
        if info.get("spent_by"):
            spenders_of_outpoint[str(info["spent_by"])].append((outpoint, info))

    def leg(
        external: str,
        wallet_id: str,
        direction: str,
        asset: str | None = None,
    ) -> Mapping[str, Any] | None:
        for row in rows_by_external.get(external, []):
            if row["wallet_id"] != wallet_id or row["direction"] != direction:
                continue
            if asset is not None and normalize_asset_code(str(row["asset"])) != asset:
                continue
            return row
        return None

    def tx_inputs(external: str) -> list[tuple[str, int]]:
        vin_outpoints: list[tuple[str, int]] = []
        seen: set[tuple[str, int]] = set()
        for row in rows_by_external.get(external, []):
            for outpoint in parse_vin_outpoints(row["raw_json"]):
                if outpoint not in seen:
                    seen.add(outpoint)
                    vin_outpoints.append(outpoint)
        for outpoint, _info in spenders_of_outpoint.get(external, []):
            if outpoint not in seen:
                seen.add(outpoint)
                vin_outpoints.append(outpoint)
        return vin_outpoints

    def resolve_parents(
        external: str,
        wallet_id: str,
        asset: str,
        *,
        depth: int = 0,
        visited: set[str] | None = None,
    ) -> dict[str, dict[str, Any]]:
        """Owned inputs of ``external`` for one wallet, grouped by the
        parent transaction whose leg can carry the link.

        Net-zero parent legs (in-wallet consolidations) cannot satisfy any
        allocation demand, so they are resolved transparently into THEIR
        owned parents, recording the passthrough txids for the explanation.
        """
        visited = visited or set()
        if external in visited or depth > _MAX_PASSTHROUGH_DEPTH:
            return {}
        visited.add(external)
        groups: dict[str, dict[str, Any]] = {}
        for outpoint in tx_inputs(external):
            info = owned_index.get(outpoint)
            if info and info.get("ambiguous"):
                continue
            if not info or str(info["wallet_id"]) != wallet_id:
                continue
            if info["asset"] != asset:
                continue
            parent_txid = outpoint[0]
            if parent_txid in blocked_externals:
                continue
            amount = int(info["amount_msat"] or 0)
            if amount <= 0:
                continue
            parent_leg = leg(parent_txid, wallet_id, "inbound", asset) or leg(
                parent_txid, wallet_id, "outbound", asset
            )
            if parent_leg is not None:
                parent_amount = int(parent_leg["amount"])
                is_change_passthrough = (
                    str(parent_leg["direction"] or "") == "outbound"
                    and amount > parent_amount
                )
            else:
                parent_amount = 0
                is_change_passthrough = False
            if parent_leg is not None and (parent_amount <= 0 or is_change_passthrough):
                # Net-zero or change-output passthrough: attribute this input
                # to the spend's own parents instead of sizing the child from
                # a small external-payment row.
                for key, nested in resolve_parents(
                    parent_txid, wallet_id, asset, depth=depth + 1, visited=visited
                ).items():
                    group = groups.setdefault(
                        key,
                        {
                            "parent_leg": nested["parent_leg"],
                            "contributed_msat": 0,
                            "outpoints": [],
                            "via": [],
                        },
                    )
                    group["contributed_msat"] += nested["contributed_msat"]
                    group["outpoints"].extend(nested["outpoints"])
                    via = [*nested["via"], parent_txid]
                    group["via"] = sorted(set(group["via"]) | set(via))
                continue
            if parent_leg is None:
                continue
            group = groups.setdefault(
                parent_txid,
                {
                    "parent_leg": parent_leg,
                    "contributed_msat": 0,
                    "outpoints": [],
                    "via": [],
                },
            )
            group["contributed_msat"] += amount
            group["outpoints"].append(outpoint)
        return groups

    pairs: list[dict[str, Any]] = []
    seen_pairs: set[tuple[str, str]] = set()

    def emit(
        from_row: Mapping[str, Any],
        to_row: Mapping[str, Any],
        kind: str,
        allocation_msat: int,
        from_allocation_msat: int,
        outpoints: list[tuple[str, int]],
        *,
        via: Sequence[str] = (),
    ) -> None:
        if from_row["id"] == to_row["id"]:
            return
        if allocation_msat <= 0:
            return
        if normalize_asset_code(str(from_row["asset"])) != normalize_asset_code(str(to_row["asset"])):
            return
        if _event_time(from_row) > _event_time(to_row):
            # A link the chronology gate would reject forever is noise, not
            # a suggestion.
            return
        key = (from_row["id"], to_row["id"])
        if key in seen_pairs:
            return
        seen_pairs.add(key)
        if kind == "parent_spend":
            explanation = (
                f"Spend consumes {len(outpoints)} owned output"
                f"{'' if len(outpoints) == 1 else 's'} of this parent transaction "
                f"({_short_outpoints(outpoints)})"
                + (
                    f" via in-wallet consolidation {_short_outpoints([(txid, 0) for txid in via])[:64]}"
                    if via
                    else ""
                )
                + "; derived locally from synced wallet data."
            )
        else:
            explanation = (
                f"Receiving wallet owns {len(outpoints)} output"
                f"{'' if len(outpoints) == 1 else 's'} of this transaction "
                f"({_short_outpoints(outpoints)}); derived locally from synced wallet data."
            )
        pairs.append(
            {
                "from_row": from_row,
                "to_row": to_row,
                "kind": kind,
                "allocation_msat": int(allocation_msat),
                "from_allocation_msat": int(from_allocation_msat),
                "outpoints": list(outpoints),
                "explanation": explanation,
            }
        )

    for external, legs in rows_by_external.items():
        if external in blocked_externals:
            continue
        vin_outpoints = tx_inputs(external)

        spending_wallets: set[str] = set()
        input_amounts_by_wallet_asset: dict[tuple[str, str], int] = defaultdict(int)
        for outpoint in vin_outpoints:
            info = owned_index.get(outpoint)
            if info and info.get("ambiguous"):
                continue
            if info:
                wallet_id = str(info["wallet_id"])
                asset = normalize_asset_code(str(info["asset"]))
                amount = int(info["amount_msat"] or 0)
                spending_wallets.add(wallet_id)
                if amount > 0:
                    input_amounts_by_wallet_asset[(wallet_id, asset)] += amount

        # parent_spend edges, sized as a group so they sum to exactly
        # min(total contributed, spend leg amount).
        for wallet_id in spending_wallets:
            out_legs = [
                row
                for row in legs
                if row["wallet_id"] == wallet_id and row["direction"] == "outbound"
            ]
            for out_leg in out_legs:
                if int(out_leg["amount"]) <= 0:
                    # Net-zero consolidation legs carry no demand; their
                    # children resolve through them transparently.
                    continue
                asset = normalize_asset_code(str(out_leg["asset"]))
                groups = resolve_parents(external, wallet_id, asset)
                if not groups:
                    continue
                ordered = sorted(groups.items())
                total = sum(group["contributed_msat"] for _key, group in ordered)
                if total <= 0:
                    continue
                target_sum = min(total, int(out_leg["amount"]))
                allocations = _allocate_pro_rata(
                    [group["contributed_msat"] for _key, group in ordered],
                    target_sum,
                )
                for (_key, group), allocation in zip(ordered, allocations):
                    emit(
                        group["parent_leg"],
                        out_leg,
                        "parent_spend",
                        allocation,
                        min(
                            group["contributed_msat"],
                            int(group["parent_leg"]["amount"]),
                        ),
                        group["outpoints"],
                        via=group["via"],
                    )

        # leg_funding edges: outbound leg of the spending wallet -> inbound
        # leg of each receiving wallet that owns outputs of this tx.
        if not spending_wallets:
            continue
        received_by_wallet: dict[
            tuple[str, str], list[tuple[tuple[str, int], int]]
        ] = defaultdict(list)
        for outpoint, info in owned_outputs_by_txid.get(external, []):
            received_by_wallet[(str(info["wallet_id"]), str(info["asset"]))].append(
                (outpoint, int(info["amount_msat"] or 0))
            )
        edge_candidates: list[dict[str, Any]] = []
        for (receiver_wallet, asset), received in sorted(received_by_wallet.items()):
            contributors = [
                (wallet_id, amount)
                for (wallet_id, input_asset), amount in sorted(
                    input_amounts_by_wallet_asset.items()
                )
                if input_asset == asset and wallet_id != receiver_wallet and amount > 0
            ]
            if not contributors:
                continue
            received_total = sum(amount for _outpoint, amount in received)
            in_leg = leg(external, receiver_wallet, "inbound", asset)
            if in_leg is None:
                continue
            target_sum = min(received_total, int(in_leg["amount"]))
            allocations = _allocate_pro_rata(
                [amount for _wallet_id, amount in contributors],
                target_sum,
            )
            for (spender_wallet, contributed), allocation in zip(
                contributors, allocations
            ):
                if receiver_wallet == spender_wallet:
                    continue
                out_leg = leg(external, spender_wallet, "outbound", asset)
                if out_leg is None:
                    continue
                edge_candidates.append(
                    {
                        "spender_wallet": spender_wallet,
                        "asset": asset,
                        "out_leg": out_leg,
                        "in_leg": in_leg,
                        "allocation": allocation,
                        "contributed": contributed,
                        "outpoints": [outpoint for outpoint, _amount in received],
                    }
                )
        candidates_by_spender: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
        for candidate in edge_candidates:
            if int(candidate["allocation"]) > 0:
                candidates_by_spender[
                    (str(candidate["spender_wallet"]), str(candidate["asset"]))
                ].append(candidate)
        for (spender_wallet, asset), candidates in sorted(candidates_by_spender.items()):
            candidates = sorted(
                candidates,
                key=lambda candidate: str(candidate["in_leg"]["id"]),
            )
            child_total = sum(int(candidate["allocation"]) for candidate in candidates)
            if child_total <= 0:
                continue
            contributed = int(input_amounts_by_wallet_asset[(spender_wallet, asset)])
            out_leg_amount = int(candidates[0]["out_leg"]["amount"])
            requirement_total = min(contributed, out_leg_amount)
            if requirement_total <= 0:
                continue
            child_allocations = [int(candidate["allocation"]) for candidate in candidates]
            if requirement_total <= child_total:
                from_allocations = _allocate_pro_rata(child_allocations, requirement_total)
            else:
                extras = _allocate_by_weight(child_allocations, requirement_total - child_total)
                from_allocations = [
                    allocation + extra
                    for allocation, extra in zip(child_allocations, extras)
                ]
            for candidate, from_allocation in zip(candidates, from_allocations):
                emit(
                    candidate["out_leg"],
                    candidate["in_leg"],
                    "leg_funding",
                    int(candidate["allocation"]),
                    from_allocation,
                    candidate["outpoints"],
                )

    return pairs


def derive_payment_hash_pairs(
    rows: Sequence[Mapping[str, Any]],
    *,
    skip_row: Callable[[Mapping[str, Any]], bool],
) -> list[dict[str, Any]]:
    """Derive exact Lightning edges from shared payment hashes.

    A payment hash names exactly one payment. When the profile holds
    exactly one outbound and one inbound row for a hash, in different
    wallets, those are two legs of the same transfer — an LN payment
    between own wallets, or an on-chain HTLC leg (hash extracted at
    import) matching an LN leg.
    """
    by_hash: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for row in rows:
        payment_hash = str(row["payment_hash"] or "").strip().lower()
        if payment_hash:
            by_hash[payment_hash].append(row)

    pairs: list[dict[str, Any]] = []
    for payment_hash, group in by_hash.items():
        outs = [row for row in group if row["direction"] == "outbound"]
        ins = [row for row in group if row["direction"] == "inbound"]
        if len(outs) != 1 or len(ins) != 1:
            continue
        out_tx, in_tx = outs[0], ins[0]
        if out_tx["wallet_id"] == in_tx["wallet_id"]:
            continue
        if normalize_asset_code(str(out_tx["asset"])) != normalize_asset_code(str(in_tx["asset"])):
            continue
        if skip_row(out_tx) or skip_row(in_tx):
            continue
        if _event_time(out_tx) > _event_time(in_tx):
            continue
        out_amount = int(out_tx["amount"])
        in_amount = int(in_tx["amount"])
        if out_amount <= 0 or in_amount <= 0 or in_amount > out_amount:
            continue
        pairs.append(
            {
                "from_row": out_tx,
                "to_row": in_tx,
                "payment_hash": payment_hash,
                "allocation_msat": in_amount,
                "from_allocation_msat": out_amount,
                "explanation": (
                    f"Both legs share payment hash {payment_hash[:16]}…; "
                    "a Lightning payment hash names exactly one payment."
                ),
            }
        )
    return pairs
