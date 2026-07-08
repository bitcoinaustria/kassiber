"""Classify on-chain transactions that are Lightning channel opens/closes.

Opening a channel moves the operator's own BTC from their on-chain wallet into a
2-of-2 output they co-control — the coins stay owned, so it is NOT a disposal.
Closing returns them — NOT an acquisition; the basis carries. But an on-chain
backend that happens to watch the node's on-chain addresses (a "dual-sync"
setup: an LN adapter for the node PLUS a separate wallet for its L1 UTXOs) sees
the funding tx as a plain send (disposal) and the close as a plain receive
(acquisition), which mis-taxes both.

This module does NOT import channel transactions (that would double-count
against the L1 wallet). Instead it derives, from the owned channels' funding and
closing txids, a ``transaction_id -> role`` map that the tax engine consumes via
the same non-event suppression it uses for loan collateral lock/release
(``kassiber.core.loans`` CHANNEL_OPEN / CHANNEL_CLOSE). If no L1 wallet recorded
the channel tx, the map is empty and nothing changes (correct).

The funding txid is taken from each channel's ``funding_outpoint`` (always
captured). The closing txid is best-effort — see the adapter capture — so the
close side only fires when a closing txid is known.
"""

from __future__ import annotations

import json
from typing import Any, Iterable, Mapping

from ...transfers import normalize_group_txid
from ..loans import CHANNEL_CLOSE, CHANNEL_OPEN

CHANNEL_RECORD_TYPE = "channel"


def _field(row: Any, key: str) -> Any:
    """Read ``key`` from a sqlite3.Row-like, dict, or object row."""
    if isinstance(row, Mapping):
        return row.get(key)
    try:
        keys = row.keys()
    except AttributeError:
        return getattr(row, key, None)
    return row[key] if key in keys else None


def _txid_from_outpoint(value: Any) -> str | None:
    """``<txid>:<vout>`` -> ``<txid>``; a bare txid passes through."""
    if not value:
        return None
    return str(value).split(":", 1)[0] or None


def _vin_txids(row: Any) -> set[str]:
    """Normalized txids the row's transaction spends from (raw_json vin).

    A force-close pays the wallet via a separate timelocked SWEEP tx whose own
    txid never equals the recorded closing txid — but its inputs spend the
    commitment tx, so the vin reference is the deterministic close signal.
    """
    raw = _field(row, "raw_json")
    if not raw:
        return set()
    if isinstance(raw, str):
        try:
            payload = json.loads(raw)
        except ValueError:
            return set()
    else:
        payload = raw
    if not isinstance(payload, Mapping):
        return set()
    vin = payload.get("vin")
    txids: set[str] = set()
    if isinstance(vin, list):
        for entry in vin:
            if isinstance(entry, Mapping) and entry.get("txid"):
                txids.add(normalize_group_txid(str(entry["txid"])))
    return txids


def channel_role_map(
    channel_rows: Iterable[Any],
    tx_rows: Iterable[Any],
) -> dict[str, str]:
    """Return ``{transaction_id: CHANNEL_OPEN|CHANNEL_CLOSE}`` for on-chain txs.

    ``channel_rows`` are persisted channel records exposing ``funding_txid`` /
    ``funding_outpoint`` and (optionally) ``closing_txid``. ``tx_rows`` are
    transaction rows exposing ``id``, ``external_id`` and ``direction``. Txids
    are compared with the same 64-hex case-folding the transfer detector uses.
    """
    funding: set[str] = set()
    closing: set[str] = set()
    for row in channel_rows:
        fund = _txid_from_outpoint(
            _field(row, "funding_txid") or _field(row, "funding_outpoint")
        )
        if fund:
            funding.add(normalize_group_txid(fund))
        close = _field(row, "closing_txid")
        if close:
            closing.add(normalize_group_txid(str(close)))

    roles: dict[str, str] = {}
    for tx in tx_rows:
        external_id = _field(tx, "external_id")
        if not external_id:
            continue
        key = normalize_group_txid(str(external_id))
        direction = _field(tx, "direction")
        tx_id = str(_field(tx, "id"))
        # The funding tx leaves the on-chain wallet (outbound); the close tx
        # returns funds (inbound). Guarding on direction avoids mislabeling a
        # change/receive leg that happens to share the txid.
        if direction == "outbound" and key in funding:
            roles[tx_id] = CHANNEL_OPEN
        elif direction == "inbound" and (
            key in closing or (closing and _vin_txids(tx) & closing)
        ):
            # Direct payout from the close tx (coop close / to_remote) matches
            # by txid; a force-close's timelocked sweep matches by spending the
            # commitment tx.
            roles[tx_id] = CHANNEL_CLOSE
    return roles


def channel_transfer_pairs(
    channel_rows: Iterable[Any],
    tx_rows: Iterable[Any],
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]],
) -> list[dict[str, Any]]:
    """Return explicit same-asset pairs for channel capacity moves.

    ``channel_role_map`` only suppresses the L1 row as a non-event. That preserves
    profile-wide holdings but strands the capacity in the funding wallet. When the
    channel metadata carries the node wallet id, synthesize the missing other leg
    so funding becomes an on-chain-wallet -> node MOVE and cooperative close
    becomes node -> on-chain-wallet.
    """
    funding_wallet_by_txid: dict[str, str] = {}
    closing_wallet_by_txid: dict[str, str] = {}
    close_balance_by_txid: dict[str, int] = {}
    for row in channel_rows:
        wallet_id = _field(row, "wallet_id")
        if not wallet_id or str(wallet_id) not in wallet_refs_by_id:
            continue
        fund = _txid_from_outpoint(
            _field(row, "funding_txid") or _field(row, "funding_outpoint")
        )
        if fund:
            funding_wallet_by_txid.setdefault(normalize_group_txid(fund), str(wallet_id))
        close = _field(row, "closing_txid")
        if close:
            close_key = normalize_group_txid(str(close))
            closing_wallet_by_txid.setdefault(close_key, str(wallet_id))
            balance = int(_field(row, "close_balance_msat") or 0)
            if balance > 0:
                close_balance_by_txid.setdefault(close_key, balance)

    pairs: list[dict[str, Any]] = []
    paired_real_ids: set[str] = set()
    for tx in tx_rows:
        external_id = _field(tx, "external_id")
        if not external_id:
            continue
        tx_id = str(_field(tx, "id"))
        if tx_id in paired_real_ids:
            continue
        key = normalize_group_txid(str(external_id))
        direction = _field(tx, "direction")
        amount = int(_field(tx, "amount") or 0)
        if amount <= 0:
            continue
        if direction == "outbound" and key in funding_wallet_by_txid:
            node_wallet_id = funding_wallet_by_txid[key]
            in_row = _clone_channel_leg(
                tx,
                wallet_refs_by_id[node_wallet_id],
                row_id=f"channel-open:{tx_id}:in:{node_wallet_id}",
                direction="inbound",
                fee=0,
            )
            pairs.append(
                {
                    "out": tx,
                    "in": in_row,
                    "source": "channel_lifecycle",
                    "kind": CHANNEL_OPEN,
                }
            )
            paired_real_ids.add(tx_id)
        elif direction == "inbound":
            close_key = key if key in closing_wallet_by_txid else None
            if close_key is None and closing_wallet_by_txid:
                # Force-close sweep: the receiving tx's inputs spend the
                # commitment tx recorded as the closing txid.
                for vin_txid in _vin_txids(tx):
                    if vin_txid in closing_wallet_by_txid:
                        close_key = vin_txid
                        break
            if close_key is None:
                continue
            node_wallet_id = closing_wallet_by_txid[close_key]
            # When our settled channel balance at close is known, the gap to
            # the on-chain receipt IS the close fee (commitment + sweep
            # miner fees): put it on the synthesized node-side out leg so the
            # MOVE books it as a taxable fee disposal and the node wallet is
            # debited fully instead of stranding the difference forever. An
            # implausibly large gap (bad capture / partial sweep) trips the
            # normalizer's transfer_fee_implausible ceiling and quarantines.
            close_fee = 0
            balance = close_balance_by_txid.get(close_key, 0)
            if balance > amount:
                close_fee = balance - amount
            out_row = _clone_channel_leg(
                tx,
                wallet_refs_by_id[node_wallet_id],
                row_id=f"channel-close:{tx_id}:out:{node_wallet_id}",
                direction="outbound",
                fee=close_fee,
            )
            pairs.append(
                {
                    "out": out_row,
                    "in": tx,
                    "source": "channel_lifecycle",
                    "kind": CHANNEL_CLOSE,
                }
            )
            paired_real_ids.add(tx_id)
    return pairs


def _clone_channel_leg(
    row: Any,
    wallet_ref: Mapping[str, Any],
    *,
    row_id: str,
    direction: str,
    fee: int,
) -> dict[str, Any]:
    cloned = _row_dict(row)
    cloned.update(
        {
            "id": row_id,
            "journal_transaction_id": _field(row, "id"),
            "direction": direction,
            "fee": fee,
            "wallet_id": wallet_ref["id"],
            "wallet_label": wallet_ref["label"],
            "wallet_account_id": wallet_ref.get("wallet_account_id"),
            "account_code": wallet_ref.get("account_code"),
            "account_label": wallet_ref.get("account_label"),
            "kind": "self_transfer_in" if direction == "inbound" else "self_transfer_out",
            "description": row_id,
        }
    )
    return cloned


def _row_dict(row: Any) -> dict[str, Any]:
    if isinstance(row, Mapping):
        return dict(row)
    try:
        return {key: row[key] for key in row.keys()}
    except AttributeError:
        return dict(getattr(row, "__dict__", {}))
