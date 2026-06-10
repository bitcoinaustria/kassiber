"""Deterministic cross-layer evidence derivation for source-of-funds assembly.

This module turns data Kassiber already holds locally into provable funding
edges between transaction rows, so the source-of-funds graph can be
assembled automatically instead of hand-linked:

- ``utxo_spend``: real Bitcoin/Liquid transaction structure. Chain-synced
  rows store their inputs (``raw_json`` vin outpoints from esplora/electrum),
  and ``wallet_utxos`` records which outputs the user's wallets own (plus
  ``spent_by`` for Wasabi imports). Joining a transaction's inputs against
  owned outputs proves, per wallet: which earlier owned transaction funded
  this spend (same-wallet parent chaining through change/consolidation),
  and which wallet received which share of a multi-wallet transaction.
- ``payment_hash``: Lightning evidence. Two owned rows sharing one payment
  hash (an LN payment between own wallets, or an on-chain HTLC leg whose
  hash was extracted at import) are two legs of the same transfer.

Everything here is pure derivation over already-imported rows: no network
access, ever. More synced wallets and connection types mean more joins and
a more complete assembled graph — that scaling property is the point.

The module stays free of back-edges into ``source_funds``: it consumes
plain row mappings and returns candidate-pair dicts; the caller owns link
insertion, dedupe, privacy/samourai skip policy, and review semantics.
"""

from __future__ import annotations

import json
import sqlite3
from collections import defaultdict
from typing import Any, Callable, Mapping, Sequence

_COINBASE_TXID = "0" * 64


def _safe_json_loads(value: Any) -> Any:
    if not value:
        return None
    try:
        return json.loads(str(value))
    except (TypeError, ValueError):
        return None


def parse_vin_outpoints(raw_json: Any) -> list[tuple[str, int]]:
    """Extract input outpoints ``(prev_txid_lower, vout)`` from a stored tx.

    Handles both stored shapes: esplora's upstream tx JSON and electrum's
    locally-decoded transaction (both carry ``vin`` entries with ``txid`` +
    ``vout``). Rows without chain structure (CSV imports, bitcoinrpc sync)
    yield an empty list.
    """
    payload = _safe_json_loads(raw_json)
    if not isinstance(payload, dict):
        return []
    vin = payload.get("vin")
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
        index[(txid, int(row["vout"]))] = {
            "wallet_id": row["wallet_id"],
            "amount_msat": int(row["amount"] or 0),
            "branch_label": str(row["branch_label"] or ""),
            "spent_by": str(row["spent_by"] or "").strip().lower(),
            "asset": str(row["asset"] or ""),
        }
    return index


def _short_outpoints(outpoints: Sequence[tuple[str, int]], limit: int = 3) -> str:
    rendered = [f"{txid[:12]}…:{vout}" for txid, vout in outpoints[:limit]]
    if len(outpoints) > limit:
        rendered.append(f"+{len(outpoints) - limit} more")
    return ", ".join(rendered)


def derive_utxo_spend_pairs(
    rows: Sequence[Mapping[str, Any]],
    owned_index: Mapping[tuple[str, int], Mapping[str, Any]],
    *,
    skip_row: Callable[[Mapping[str, Any]], bool],
) -> list[dict[str, Any]]:
    """Derive exact funding edges from transaction input/output structure.

    Two edge kinds come out of one pass:

    - ``parent_spend``: within one wallet, spend transaction T consumes
      outputs created by earlier owned transaction P (consolidations and
      change chains). Evidence: T's vin outpoints (or P-outputs'
      ``spent_by``) match owned outputs created by P in the same wallet.
    - ``leg_funding``: across wallets, the outbound leg of T in the paying
      wallet funds the inbound leg of T in a receiving wallet that owns
      outputs of T. This resolves multi-wallet transactions that the 1:1
      same-external-id heuristic refuses.

    ``skip_row`` carries the caller's privacy/samourai policy: edges never
    assert lineage through rows the caller wants left alone.
    """
    rows_by_external: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for row in rows:
        external = str(row["external_id"] or "").strip().lower()
        if external:
            rows_by_external[external].append(row)

    # Owned outputs grouped by the transaction that created them, so the
    # receiving side of a tx and the parents of a spend resolve in O(1).
    owned_outputs_by_txid: dict[str, list[tuple[tuple[str, int], Mapping[str, Any]]]] = defaultdict(list)
    spenders_of_outpoint: dict[str, list[tuple[tuple[str, int], Mapping[str, Any]]]] = defaultdict(list)
    for outpoint, info in owned_index.items():
        owned_outputs_by_txid[outpoint[0]].append((outpoint, info))
        if info.get("spent_by"):
            spenders_of_outpoint[str(info["spent_by"])].append((outpoint, info))

    pairs: list[dict[str, Any]] = []
    seen: set[tuple[str, str]] = set()

    def leg(external: str, wallet_id: str, direction: str) -> Mapping[str, Any] | None:
        candidates = [
            row
            for row in rows_by_external.get(external, [])
            if row["wallet_id"] == wallet_id and row["direction"] == direction
        ]
        return candidates[0] if candidates else None

    def emit(
        from_row: Mapping[str, Any],
        to_row: Mapping[str, Any],
        kind: str,
        contributed_msat: int,
        outpoints: list[tuple[str, int]],
    ) -> None:
        if from_row["id"] == to_row["id"]:
            return
        if skip_row(from_row) or skip_row(to_row):
            return
        key = (from_row["id"], to_row["id"])
        if key in seen:
            return
        seen.add(key)
        if kind == "parent_spend":
            explanation = (
                f"Spend consumes {len(outpoints)} owned output"
                f"{'' if len(outpoints) == 1 else 's'} of this parent transaction "
                f"({_short_outpoints(outpoints)}); derived locally from synced wallet data."
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
                "allocation_msat": min(int(contributed_msat), int(to_row["amount"])),
                "from_allocation_msat": min(int(contributed_msat), int(from_row["amount"])),
                "outpoints": list(outpoints),
                "explanation": explanation,
            }
        )

    for external, legs in rows_by_external.items():
        # Inputs of this transaction, from any leg that carries chain
        # structure (esplora/electrum store the same tx per wallet) plus
        # spent_by back-references (Wasabi).
        vin_outpoints: list[tuple[str, int]] = []
        seen_outpoints: set[tuple[str, int]] = set()
        for row in legs:
            for outpoint in parse_vin_outpoints(row["raw_json"]):
                if outpoint not in seen_outpoints:
                    seen_outpoints.add(outpoint)
                    vin_outpoints.append(outpoint)
        for outpoint, _info in spenders_of_outpoint.get(external, []):
            if outpoint not in seen_outpoints:
                seen_outpoints.add(outpoint)
                vin_outpoints.append(outpoint)

        # Owned inputs grouped by (spending wallet, parent txid).
        contributed_by_wallet: dict[str, int] = defaultdict(int)
        by_wallet_parent: dict[tuple[str, str], list[tuple[tuple[str, int], int]]] = defaultdict(list)
        for outpoint in vin_outpoints:
            info = owned_index.get(outpoint)
            if not info:
                continue
            wallet_id = str(info["wallet_id"])
            amount = int(info["amount_msat"] or 0)
            contributed_by_wallet[wallet_id] += amount
            by_wallet_parent[(wallet_id, outpoint[0])].append((outpoint, amount))

        # parent_spend edges: parent tx row -> outbound leg, same wallet.
        for (wallet_id, parent_txid), spent in by_wallet_parent.items():
            out_leg = leg(external, wallet_id, "outbound")
            if out_leg is None:
                continue
            parent_leg = leg(parent_txid, wallet_id, "inbound") or leg(
                parent_txid, wallet_id, "outbound"
            )
            if parent_leg is None:
                continue
            contributed = sum(amount for _outpoint, amount in spent)
            emit(
                parent_leg,
                out_leg,
                "parent_spend",
                contributed,
                [outpoint for outpoint, _amount in spent],
            )

        # leg_funding edges: outbound leg of the spending wallet -> inbound
        # leg of each receiving wallet that owns outputs of this tx.
        if not contributed_by_wallet:
            continue
        received_by_wallet: dict[str, list[tuple[tuple[str, int], int]]] = defaultdict(list)
        for outpoint, info in owned_outputs_by_txid.get(external, []):
            received_by_wallet[str(info["wallet_id"])].append(
                (outpoint, int(info["amount_msat"] or 0))
            )
        for spender_wallet in contributed_by_wallet:
            out_leg = leg(external, spender_wallet, "outbound")
            if out_leg is None:
                continue
            for receiver_wallet, received in received_by_wallet.items():
                if receiver_wallet == spender_wallet:
                    continue
                in_leg = leg(external, receiver_wallet, "inbound")
                if in_leg is None:
                    continue
                received_total = sum(amount for _outpoint, amount in received)
                emit(
                    out_leg,
                    in_leg,
                    "leg_funding",
                    received_total,
                    [outpoint for outpoint, _amount in received],
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
        if out_tx["asset"] != in_tx["asset"]:
            continue
        if skip_row(out_tx) or skip_row(in_tx):
            continue
        pairs.append(
            {
                "from_row": out_tx,
                "to_row": in_tx,
                "payment_hash": payment_hash,
                "allocation_msat": int(in_tx["amount"]),
                "from_allocation_msat": int(out_tx["amount"]),
                "explanation": (
                    f"Both legs share payment hash {payment_hash[:16]}…; "
                    "a Lightning payment hash names exactly one payment."
                ),
            }
        )
    return pairs
