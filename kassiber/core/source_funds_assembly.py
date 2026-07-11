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
  parents), and which wallet received each owned output. When several owned
  source wallets fund several owned destination wallets, Bitcoin does not
  define the source-to-destination matrix; Kassiber proposes pro-rata review
  allocations without calling them exact lineage.
- ``payment_hash``: node-native Lightning evidence. Two owned node rows sharing
  one canonical hash and the same principal are two legs of the same payment.

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

import sqlite3
from collections import defaultdict
from typing import Any, Callable, Mapping, Sequence

from ..transfers import (
    bitcoin_network_domain,
    canonical_txid,
    canonical_payment_hash,
    is_lightning_payment_hash_row,
    onchain_transfer_scope,
)
from ..wallet_descriptors import normalize_asset_code, normalize_chain, normalize_network
from .onchain import parse_vin_outpoints, stored_tx_mapping

# Full consensus component identity. Bitcoin has one ``BTC`` component; Liquid
# transactions can carry several independent assets under the same txid, so the
# fourth field is the Liquid asset id (never merely its display label).
PhysicalTxKey = tuple[str, str, str, str]
OwnedOutpointKey = tuple[str, str, str, int]


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    """Return SQLite column names for a fixed, trusted table name.

    Some local-only analysis callers deliberately use a minimal historical
    ``wallet_utxos`` shape.  The production table has always carried enough
    chain information to apply the old default-network convention, so a
    missing ``network`` column can be represented as ``NULL`` and normalized
    below without dropping chain/network from the physical identity key.
    """
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
    """Owned outputs keyed by ``(chain, network, txid, vout)``.

    ``amount_msat`` comes straight from the inventory (stored in msat);
    ``spent_by`` is the spending txid when the importer knew it (Wasabi).

    Minimal/legacy inventory tables may predate the explicit ``network``
    column.  Those rows retain the historical chain-specific default
    (Bitcoin ``main`` / Liquid ``liquidv1``).  When the column exists its
    normalized value remains part of the key, so identical outpoints on
    different networks never alias.
    """
    index: dict[OwnedOutpointKey, dict[str, Any]] = {}
    columns = _table_columns(conn, "wallet_utxos")
    network_select = (
        "network"
        if "network" in columns
        else "NULL AS network"
    )
    raw_json_select = "raw_json" if "raw_json" in columns else "NULL AS raw_json"
    rows = conn.execute(
        f"""
        SELECT wallet_id, chain, {network_select}, txid, vout, amount, branch_label,
               spent_by, asset, {raw_json_select}
        FROM wallet_utxos
        WHERE profile_id = ?
        """,
        (profile_id,),
    ).fetchall()
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
            raw = stored_tx_mapping(row["raw_json"]) or {}
            raw_asset_value = raw.get("asset_id")
            raw_asset_id = canonical_txid(raw_asset_value)
            display_asset_id = canonical_txid(asset)
            if raw_asset_value not in (None, "") and raw_asset_id is None:
                # Explicit malformed identity is contradictory, not missing.
                continue
            if (
                raw_asset_id is not None
                and display_asset_id is not None
                and raw_asset_id != display_asset_id
            ):
                continue
            asset_identity = raw_asset_id or display_asset_id
            if asset_identity is None:
                # ``LBTC`` alone is a display label. A wallet can be configured
                # with a non-standard policy asset, so never infer consensus
                # identity from that label at this accounting boundary.
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


def _owned_asset_identity(
    outpoint: OwnedOutpointKey, info: Mapping[str, Any]
) -> str:
    """Consensus asset identity for one owned output, or ``""`` if unknown."""

    chain = outpoint[0]
    explicit = info.get("asset_identity")
    if explicit not in (None, ""):
        if chain == "liquid":
            return canonical_txid(explicit) or ""
        return normalize_asset_code(str(explicit))
    asset = normalize_asset_code(str(info.get("asset") or ""))
    if chain == "liquid":
        # A full 32-byte asset id is consensus identity; a ticker/LBTC label is
        # not. Production indexes populate ``asset_identity`` from retained
        # inventory evidence, but keep this pure helper safe for direct callers.
        return canonical_txid(asset) or ""
    return asset or "BTC"


def derive_utxo_spend_pairs(
    rows: Sequence[Mapping[str, Any]],
    owned_index: Mapping[OwnedOutpointKey, Mapping[str, Any]],
    *,
    skip_row: Callable[[Mapping[str, Any]], bool],
) -> list[dict[str, Any]]:
    """Derive funding edges from transaction input/output structure.

    Two edge kinds come out of one pass:

    - ``parent_spend``: within one wallet, spend transaction T consumes
      outputs created by earlier owned transactions (consolidations and
      change chains). Net-zero in-wallet consolidation legs are resolved
      transparently: their own parents become the edge sources, so chains
      like P -> consolidation -> S link P directly to S.
    - ``leg_funding``: across wallets, the outbound leg of T in the paying
      wallet funds the inbound leg of T in a receiving wallet that owns
      outputs of T. This resolves multi-wallet transactions that the 1:1
      raw-id heuristic refuses.
    A transaction with multiple owned source wallets and multiple owned
    destination wallets does not define which input funded which output.
    Kassiber still returns a deterministic pro-rata accounting allocation for
    review, but marks those cross-wallet edges ``strong`` / ``requires_review``
    instead of claiming exact physical lineage.  One-source fan-out,
    many-source consolidation into one destination, and parent-spend edges are
    structurally unambiguous at the wallet-leg level and remain exact. More
    than one stored row for the same component/wallet/direction is ambiguous
    evidence, so that physical component emits no derived edges.
    """
    rows_by_external: dict[PhysicalTxKey, list[Mapping[str, Any]]] = defaultdict(list)
    for row in rows:
        scope = onchain_transfer_scope(row)
        if scope is not None:
            rows_by_external[scope].append(row)

    legs_by_identity: dict[
        tuple[PhysicalTxKey, str, str], list[Mapping[str, Any]]
    ] = defaultdict(list)
    for external, scoped_rows in rows_by_external.items():
        for row in scoped_rows:
            legs_by_identity[
                (
                    external,
                    str(row["wallet_id"]),
                    str(row["direction"]),
                )
            ].append(row)
    ambiguous_leg_components = {
        external
        for (external, _wallet_id, _direction), matching_rows in legs_by_identity.items()
        if len(matching_rows) != 1
    }

    # Privacy/samourai policy applies per transaction: any flagged leg
    # poisons every leg of the same physical transaction scope.
    blocked_physical_transactions = {
        scope[:3]
        for row in rows
        if (scope := onchain_transfer_scope(row)) is not None and skip_row(row)
    }

    owned_outputs_by_txid: dict[
        PhysicalTxKey, list[tuple[OwnedOutpointKey, Mapping[str, Any]]]
    ] = defaultdict(list)
    spenders_of_outpoint: dict[
        PhysicalTxKey, list[tuple[OwnedOutpointKey, Mapping[str, Any]]]
    ] = defaultdict(list)
    for outpoint, info in owned_index.items():
        if info.get("ambiguous"):
            continue
        asset_identity = _owned_asset_identity(outpoint, info)
        if not asset_identity:
            continue
        component = (outpoint[0], outpoint[1], outpoint[2], asset_identity)
        owned_outputs_by_txid[component].append((outpoint, info))
        if info.get("spent_by"):
            spenders_of_outpoint[
                (
                    outpoint[0],
                    outpoint[1],
                    str(info["spent_by"]),
                    asset_identity,
                )
            ].append((outpoint, info))

    def leg(
        external: PhysicalTxKey,
        wallet_id: str,
        direction: str,
    ) -> Mapping[str, Any] | None:
        # A physical component may have at most one accounting leg for a given
        # wallet and direction. Choosing the first duplicate would make exact
        # lineage depend on query/import order and can double-cover one set of
        # inputs. Fail the whole component closed instead.
        if external in ambiguous_leg_components:
            return None
        matching_rows = legs_by_identity.get((external, wallet_id, direction), [])
        return matching_rows[0] if len(matching_rows) == 1 else None

    def tx_inputs(external: PhysicalTxKey) -> list[OwnedOutpointKey]:
        vin_outpoints: list[OwnedOutpointKey] = []
        seen: set[OwnedOutpointKey] = set()
        for row in rows_by_external.get(external, []):
            for txid, vout in parse_vin_outpoints(row["raw_json"]):
                outpoint = (external[0], external[1], txid, vout)
                info = owned_index.get(outpoint)
                if (
                    info is not None
                    and _owned_asset_identity(outpoint, info) != external[3]
                ):
                    continue
                if outpoint not in seen:
                    seen.add(outpoint)
                    vin_outpoints.append(outpoint)
        for outpoint, _info in spenders_of_outpoint.get(external, []):
            if outpoint not in seen:
                seen.add(outpoint)
                vin_outpoints.append(outpoint)
        return vin_outpoints

    def resolve_parents(
        external: PhysicalTxKey,
        wallet_id: str,
        *,
        visited: frozenset[PhysicalTxKey] | None = None,
    ) -> dict[PhysicalTxKey, dict[str, Any]]:
        """Owned inputs of ``external`` for one wallet, grouped by the
        parent transaction whose leg can carry the link.

        Net-zero parent legs (in-wallet consolidations) cannot satisfy any
        allocation demand, so they are resolved transparently into THEIR
        owned parents, recording the passthrough txids for the explanation.
        """
        # ``visited`` is the active recursion path, not a global graph-wide
        # set.  Sibling branches can legitimately reconverge on outputs of an
        # earlier consolidation; sharing their mutable visited state silently
        # drops the later branch.  A path-local immutable set still rejects
        # malformed cycles without imposing an arbitrary history-depth limit.
        active_path = visited or frozenset()
        if external in active_path:
            return {}
        active_path = active_path | {external}
        groups: dict[PhysicalTxKey, dict[str, Any]] = {}
        for outpoint in tx_inputs(external):
            info = owned_index.get(outpoint)
            if info and info.get("ambiguous"):
                continue
            if not info or str(info["wallet_id"]) != wallet_id:
                continue
            asset_identity = _owned_asset_identity(outpoint, info)
            if not asset_identity or asset_identity != external[3]:
                continue
            parent_txid = outpoint[2]
            parent_component = (
                outpoint[0],
                outpoint[1],
                parent_txid,
                asset_identity,
            )
            if parent_component[:3] in blocked_physical_transactions:
                continue
            amount = int(info["amount_msat"] or 0)
            if amount <= 0:
                continue
            parent_leg = leg(parent_component, wallet_id, "inbound") or leg(
                parent_component, wallet_id, "outbound"
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
                    parent_component,
                    wallet_id,
                    visited=active_path,
                ).items():
                    group = groups.setdefault(
                        key,
                        {
                            "parent_leg": nested["parent_leg"],
                            "contributed_msat": 0,
                            "outpoints": [],
                            "contributions": {},
                            "via": [],
                        },
                    )
                    # Reconvergent branches can reach the same terminal owned
                    # outpoint more than once.  Preserve every route in
                    # ``via`` while counting that physical contribution once.
                    for source_outpoint, contribution in nested[
                        "contributions"
                    ].items():
                        if source_outpoint in group["contributions"]:
                            continue
                        group["contributions"][source_outpoint] = contribution
                        group["contributed_msat"] += contribution
                        group["outpoints"].append(source_outpoint)
                    via = [*nested["via"], parent_txid]
                    group["via"] = sorted(set(group["via"]) | set(via))
                continue
            if parent_leg is None:
                continue
            group = groups.setdefault(
                parent_component,
                {
                    "parent_leg": parent_leg,
                    "contributed_msat": 0,
                    "outpoints": [],
                    "contributions": {},
                    "via": [],
                },
            )
            source_outpoint = (outpoint[2], outpoint[3])
            if source_outpoint not in group["contributions"]:
                group["contributions"][source_outpoint] = amount
                group["contributed_msat"] += amount
                group["outpoints"].append(source_outpoint)
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
        confidence: str = "exact",
        requires_review: bool = False,
    ) -> None:
        if from_row["id"] == to_row["id"]:
            return
        if allocation_msat <= 0:
            return
        from_scope = onchain_transfer_scope(from_row)
        to_scope = onchain_transfer_scope(to_row)
        if (
            from_scope is None
            or to_scope is None
            or (from_scope[0], from_scope[1], from_scope[3])
            != (to_scope[0], to_scope[1], to_scope[3])
        ):
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
            if requires_review:
                explanation += (
                    " Bitcoin's pooled inputs do not define this source-to-output "
                    "allocation; this is a deterministic pro-rata accounting "
                    "allocation proposal that requires manual review."
                )
        pairs.append(
            {
                "from_row": from_row,
                "to_row": to_row,
                "kind": kind,
                "allocation_msat": int(allocation_msat),
                "from_allocation_msat": int(from_allocation_msat),
                "outpoints": list(outpoints),
                "confidence": confidence,
                "requires_review": requires_review,
                "explanation": explanation,
            }
        )

    for external, legs in rows_by_external.items():
        if (
            external[:3] in blocked_physical_transactions
            or external in ambiguous_leg_components
        ):
            continue
        vin_outpoints = tx_inputs(external)

        spending_wallets: set[str] = set()
        input_amounts_by_wallet_component: dict[tuple[str, str], int] = defaultdict(int)
        for outpoint in vin_outpoints:
            info = owned_index.get(outpoint)
            if info and info.get("ambiguous"):
                continue
            if info:
                asset_identity = _owned_asset_identity(outpoint, info)
                if not asset_identity or asset_identity != external[3]:
                    continue
                wallet_id = str(info["wallet_id"])
                amount = int(info["amount_msat"] or 0)
                spending_wallets.add(wallet_id)
                if amount > 0:
                    input_amounts_by_wallet_component[
                        (wallet_id, asset_identity)
                    ] += amount

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
                groups = resolve_parents(external, wallet_id)
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
                pooled_residual = len(ordered) > 1 and target_sum != total
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
                        confidence="strong" if pooled_residual else "exact",
                        requires_review=pooled_residual,
                    )

        # leg_funding edges: outbound leg of the spending wallet -> inbound
        # leg of each receiving wallet that owns outputs of this tx.
        if not spending_wallets:
            continue
        received_by_wallet: dict[
            tuple[str, str], list[tuple[OwnedOutpointKey, int]]
        ] = defaultdict(list)
        for outpoint, info in owned_outputs_by_txid.get(external, []):
            asset_identity = _owned_asset_identity(outpoint, info)
            if asset_identity != external[3]:
                continue
            received_by_wallet[
                (str(info["wallet_id"]), asset_identity)
            ].append((outpoint, int(info["amount_msat"] or 0)))
        edge_candidates: list[dict[str, Any]] = []
        for (receiver_wallet, asset_identity), received in sorted(
            received_by_wallet.items()
        ):
            contributors = [
                (wallet_id, amount)
                for (wallet_id, input_asset_identity), amount in sorted(
                    input_amounts_by_wallet_component.items()
                )
                if input_asset_identity == asset_identity
                and wallet_id != receiver_wallet
                and amount > 0
            ]
            if not contributors:
                continue
            received_total = sum(amount for _outpoint, amount in received)
            in_leg = leg(external, receiver_wallet, "inbound")
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
                out_leg = leg(external, spender_wallet, "outbound")
                if out_leg is None:
                    continue
                edge_candidates.append(
                    {
                        "spender_wallet": spender_wallet,
                        "asset_identity": asset_identity,
                        "out_leg": out_leg,
                        "in_leg": in_leg,
                        "allocation": allocation,
                        "contributed": contributed,
                        "outpoints": [
                            (outpoint[2], outpoint[3])
                            for outpoint, _amount in received
                        ],
                    }
                )
        source_wallets_by_component: dict[str, set[str]] = defaultdict(set)
        destination_wallets_by_component: dict[str, set[str]] = defaultdict(set)
        for candidate in edge_candidates:
            if int(candidate["allocation"]) <= 0:
                continue
            asset_identity = str(candidate["asset_identity"])
            source_wallets_by_component[asset_identity].add(
                str(candidate["spender_wallet"])
            )
            destination_wallets_by_component[asset_identity].add(
                str(candidate["in_leg"]["wallet_id"])
            )
        review_components = {
            asset_identity
            for asset_identity, sources in source_wallets_by_component.items()
            if len(sources) > 1
            and len(
                destination_wallets_by_component.get(asset_identity, set())
            )
            > 1
        }
        # Even an apparent N:1 consolidation is not an exact source allocation
        # when pooled inputs exceed the owned outputs we can attribute.  The
        # shortfall can be miner fee, external payment, or a missing change
        # wallet/output; Bitcoin does not say which source wallet funded it.
        # Preserve the useful pro-rata proposal, but keep it out of bulk review.
        for asset_identity, sources in source_wallets_by_component.items():
            if len(sources) <= 1:
                continue
            contributed_total = sum(
                amount
                for (
                    wallet_id,
                    input_asset_identity,
                ), amount in input_amounts_by_wallet_component.items()
                if input_asset_identity == asset_identity and wallet_id in sources
            )
            attributed_total = sum(
                int(candidate["allocation"])
                for candidate in edge_candidates
                if candidate["asset_identity"] == asset_identity
            )
            if attributed_total != contributed_total:
                review_components.add(asset_identity)
        candidates_by_spender: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
        for candidate in edge_candidates:
            if int(candidate["allocation"]) > 0:
                candidates_by_spender[
                    (
                        str(candidate["spender_wallet"]),
                        str(candidate["asset_identity"]),
                    )
                ].append(candidate)
        for (spender_wallet, asset_identity), candidates in sorted(
            candidates_by_spender.items()
        ):
            candidates = sorted(
                candidates,
                key=lambda candidate: str(candidate["in_leg"]["id"]),
            )
            child_total = sum(int(candidate["allocation"]) for candidate in candidates)
            if child_total <= 0:
                continue
            contributed = int(
                input_amounts_by_wallet_component[
                    (spender_wallet, asset_identity)
                ]
            )
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
                requires_review = asset_identity in review_components
                emit(
                    candidate["out_leg"],
                    candidate["in_leg"],
                    "leg_funding",
                    int(candidate["allocation"]),
                    from_allocation,
                    candidate["outpoints"],
                    confidence="strong" if requires_review else "exact",
                    requires_review=requires_review,
                )

    return pairs


def derive_payment_hash_pairs(
    rows: Sequence[Mapping[str, Any]],
    *,
    skip_row: Callable[[Mapping[str, Any]], bool],
) -> list[dict[str, Any]]:
    """Derive exact Lightning edges from shared payment hashes.

    A canonical payment hash names exactly one payment. When the profile holds
    exactly one outbound and one inbound node row for a hash, those are two
    legs of the same transfer (including a same-node circular payment).
    Node-native principal must agree exactly; LND/CLN record routing fees
    separately. Script-derived HTLC hashes remain review evidence elsewhere.
    """
    by_hash: dict[tuple[str, str], list[Mapping[str, Any]]] = defaultdict(list)
    for row in rows:
        payment_hash = canonical_payment_hash(row["payment_hash"])
        network_domain = bitcoin_network_domain(row)
        if (
            payment_hash is not None
            and network_domain is not None
            and is_lightning_payment_hash_row(row)
        ):
            by_hash[(payment_hash, network_domain)].append(row)

    pairs: list[dict[str, Any]] = []
    for (payment_hash, network_domain), group in by_hash.items():
        outs = [row for row in group if row["direction"] == "outbound"]
        ins = [row for row in group if row["direction"] == "inbound"]
        if len(outs) != 1 or len(ins) != 1:
            continue
        out_tx, in_tx = outs[0], ins[0]
        if normalize_asset_code(str(out_tx["asset"])) != normalize_asset_code(str(in_tx["asset"])):
            continue
        if skip_row(out_tx) or skip_row(in_tx):
            continue
        if _event_time(out_tx) > _event_time(in_tx):
            continue
        out_amount = int(out_tx["amount"])
        in_amount = int(in_tx["amount"])
        if out_amount <= 0 or in_amount <= 0 or in_amount != out_amount:
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
                    f"a Lightning payment hash inside Bitcoin {network_domain} "
                    "names exactly one payment."
                ),
            }
        )
    return pairs
