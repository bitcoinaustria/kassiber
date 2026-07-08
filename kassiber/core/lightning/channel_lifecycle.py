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
from ..loans import (
    CHANNEL_CLOSE,
    CHANNEL_CLOSE_MISMATCH,
    CHANNEL_OPEN,
    CHANNEL_OPEN_MISMATCH,
)
from ..transfer_matching import DEFAULT_FEE_PCT_MAX, DEFAULT_FEE_SATS_MIN, fee_threshold_msat

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


def _close_balance_mismatch(received_msat: int, balance_msat: int) -> bool:
    """True when the settled-balance gap is not a plausible close fee.

    The synthesized close pair clones the receipt row, so the generic
    transfer-fee implausibility guard (out.amount - in.amount) is
    definitionally zero for it and can never fire — this check is the ONLY
    ceiling between a mis-captured close balance (unsynced sweep, HTLC value
    lost to the peer) and an unbounded silent "fee" disposal. Symmetric:
    receiving clearly MORE than the settled balance is just as much a data
    problem as receiving less.
    """
    if balance_msat <= 0:
        return False
    tolerance = fee_threshold_msat(
        balance_msat, DEFAULT_FEE_PCT_MAX, DEFAULT_FEE_SATS_MIN
    )
    return abs(balance_msat - received_msat) > tolerance


def _close_leg_groups(
    closing_keys,
    close_balance_by_txid: Mapping[str, int],
    tx_rows,
) -> dict[str, dict[str, Any]]:
    """Group inbound close candidates per closing txid, classified TOGETHER.

    A close can pay the wallet in several legs (coop payout + timelocked
    to_local sweep + per-HTLC sweeps). The settled balance minus the group's
    TOTAL receipt is the single close fee — evaluating legs one at a time
    would book every other leg's amount as a "fee" once per leg.

    vin-matched legs (sweeps) are accepted in chronological order only while
    the group total is below the settled balance: once the close is fully
    accounted, a later inbound spending the commitment tx is the PEER's
    swept output coming back as an ordinary payment and must not be
    reclassified as our close leg (that would untax income). With no balance
    on record the txid/vin match stands alone (nothing to bound against).
    """
    candidates: dict[str, list[tuple[tuple, bool, Any]]] = {}
    for tx in tx_rows:
        if _field(tx, "direction") != "inbound":
            continue
        external_id = _field(tx, "external_id")
        key = normalize_group_txid(str(external_id)) if external_id else None
        close_key = key if key is not None and key in closing_keys else None
        matched_by_txid = close_key is not None
        if close_key is None and closing_keys:
            for vin_txid in _vin_txids(tx):
                if vin_txid in closing_keys:
                    close_key = vin_txid
                    break
        if close_key is None:
            continue
        sort_key = (str(_field(tx, "occurred_at") or ""), str(_field(tx, "id")))
        candidates.setdefault(close_key, []).append((sort_key, matched_by_txid, tx))

    groups: dict[str, dict[str, Any]] = {}
    for close_key, entries in candidates.items():
        entries.sort(key=lambda item: item[0])
        balance = int(close_balance_by_txid.get(close_key, 0))
        legs: list[Any] = []
        total = 0
        for _sort_key, matched_by_txid, tx in entries:
            if not matched_by_txid and balance > 0 and total >= balance:
                continue
            legs.append(tx)
            total += int(_field(tx, "amount") or 0)
        groups[close_key] = {
            "legs": legs,
            "total_msat": total,
            "balance_msat": balance,
            "mismatch": _close_balance_mismatch(total, balance),
        }
    return groups


def _funding_amount_mismatch(tx: Any, funded_msat: int) -> bool:
    """True when the outbound's amount exceeds the funded amount implausibly.

    ``amount`` on node-backed rows excludes change and the miner fee, so any
    excess over the funded channel balance is value to a non-channel output.
    The tolerance mirrors the transfer-fee ceiling so ordinary rounding and
    fee-convention noise never trips it.
    """
    if funded_msat <= 0:
        return False
    recorded = int(_field(tx, "amount") or 0)
    return recorded - funded_msat > fee_threshold_msat(
        funded_msat, DEFAULT_FEE_PCT_MAX, DEFAULT_FEE_SATS_MIN
    )


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
    funding_amount_by_txid: dict[str, int] = {}
    close_balance_by_txid: dict[str, int] = {}
    for row in channel_rows:
        fund = _txid_from_outpoint(
            _field(row, "funding_txid") or _field(row, "funding_outpoint")
        )
        if fund:
            fund_key = normalize_group_txid(fund)
            funding.add(fund_key)
            funded = int(_field(row, "funding_amount_msat") or 0)
            if funded > 0:
                # A batched open (multifundchannel) shares one funding tx
                # across N channel records: SUM the funded amounts, or a
                # clean batched open false-positives the mismatch guard.
                funding_amount_by_txid[fund_key] = (
                    funding_amount_by_txid.get(fund_key, 0) + funded
                )
        close = _field(row, "closing_txid")
        if close:
            close_key = normalize_group_txid(str(close))
            closing.add(close_key)
            balance = int(_field(row, "close_balance_msat") or 0)
            if balance > 0:
                # Several channels can share one close/sweep txid (batched
                # opens closing together): sum our settled balances.
                close_balance_by_txid[close_key] = (
                    close_balance_by_txid.get(close_key, 0) + balance
                )

    tx_rows = list(tx_rows)
    roles: dict[str, str] = {}
    # Direct payout from the close tx (coop close / to_remote) matches by
    # txid; a force-close's timelocked sweep matches by spending the
    # commitment tx. All legs of one close are classified TOGETHER.
    if closing:
        for group in _close_leg_groups(
            closing, close_balance_by_txid, tx_rows
        ).values():
            role = CHANNEL_CLOSE_MISMATCH if group["mismatch"] else CHANNEL_CLOSE
            for tx in group["legs"]:
                roles[str(_field(tx, "id"))] = role
    for tx in tx_rows:
        external_id = _field(tx, "external_id")
        if not external_id:
            continue
        key = normalize_group_txid(str(external_id))
        direction = _field(tx, "direction")
        tx_id = str(_field(tx, "id"))
        # The funding tx leaves the on-chain wallet (outbound). Guarding on
        # direction avoids mislabeling a change/receive leg that happens to
        # share the txid.
        if direction == "outbound" and key in funding:
            if _funding_amount_mismatch(tx, funding_amount_by_txid.get(key, 0)):
                # The recorded outflow clearly exceeds the funded amount: the
                # tx ALSO paid an external recipient. Suppressing the whole
                # row would silently untax that payment — flag for review.
                roles[tx_id] = CHANNEL_OPEN_MISMATCH
                continue
            roles[tx_id] = CHANNEL_OPEN
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
    ambiguous_funding_txids: set[str] = set()
    ambiguous_closing_txids: set[str] = set()
    close_balance_by_txid: dict[str, int] = {}
    funding_amount_by_txid: dict[str, int] = {}
    close_funding_by_txid: dict[str, set[str]] = {}

    def _remember_owner(
        owners: dict[str, str], ambiguous: set[str], key: str, wallet_id: str
    ) -> None:
        existing = owners.get(key)
        if existing is None:
            if key not in ambiguous:
                owners[key] = wallet_id
            return
        if existing != wallet_id:
            ambiguous.add(key)
            owners.pop(key, None)

    for row in channel_rows:
        wallet_id = _field(row, "wallet_id")
        if not wallet_id or str(wallet_id) not in wallet_refs_by_id:
            continue
        fund = _txid_from_outpoint(
            _field(row, "funding_txid") or _field(row, "funding_outpoint")
        )
        fund_key = normalize_group_txid(fund) if fund else None
        if fund:
            _remember_owner(
                funding_wallet_by_txid,
                ambiguous_funding_txids,
                fund_key,
                str(wallet_id),
            )
            funded = int(_field(row, "funding_amount_msat") or 0)
            if funded > 0:
                # A batched open (multifundchannel) shares one funding tx
                # across N channel records: SUM the funded amounts, or a
                # clean batched open false-positives the mismatch guard.
                funding_amount_by_txid[fund_key] = (
                    funding_amount_by_txid.get(fund_key, 0) + funded
                )
        close = _field(row, "closing_txid")
        if close:
            close_key = normalize_group_txid(str(close))
            _remember_owner(
                closing_wallet_by_txid,
                ambiguous_closing_txids,
                close_key,
                str(wallet_id),
            )
            if fund_key:
                close_funding_by_txid.setdefault(close_key, set()).add(fund_key)
            balance = int(_field(row, "close_balance_msat") or 0)
            if balance > 0:
                # Several channels can share one close/sweep txid (batched
                # opens closing together): sum our settled balances.
                close_balance_by_txid[close_key] = (
                    close_balance_by_txid.get(close_key, 0) + balance
                )

    tx_rows = list(tx_rows)
    pairs: list[dict[str, Any]] = []
    opened_funding_txids: set[str] = set()
    for tx in tx_rows:
        external_id = _field(tx, "external_id")
        if not external_id:
            continue
        tx_id = str(_field(tx, "id"))
        key = normalize_group_txid(str(external_id))
        direction = _field(tx, "direction")
        amount = int(_field(tx, "amount") or 0)
        if amount <= 0:
            continue
        if direction == "outbound" and key in funding_wallet_by_txid:
            if _funding_amount_mismatch(tx, funding_amount_by_txid.get(key, 0)):
                # role map flags this row CHANNEL_OPEN_MISMATCH; a synthesized
                # MOVE would absorb the external payment as channel capacity.
                continue
            opened_funding_txids.add(key)
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

    if not closing_wallet_by_txid:
        return pairs
    eligible_closing_wallet_by_txid = {
        close_key: wallet_id
        for close_key, wallet_id in closing_wallet_by_txid.items()
        if close_key not in ambiguous_closing_txids
        and bool(close_funding_by_txid.get(close_key, set()) & opened_funding_txids)
    }
    if not eligible_closing_wallet_by_txid:
        return pairs
    for close_key, group in _close_leg_groups(
        eligible_closing_wallet_by_txid, close_balance_by_txid, tx_rows
    ).items():
        if group["mismatch"]:
            # role map flags these legs CHANNEL_CLOSE_MISMATCH for quarantine;
            # a synthesized MOVE would book the whole gap as an unbounded
            # "fee" (the generic implausibility guard cannot fire on a
            # cloned-amount pair).
            continue
        node_wallet_id = eligible_closing_wallet_by_txid[close_key]
        # When our settled channel balance at close is known, the gap to the
        # GROUP's total receipt is the single close fee (commitment + sweep
        # miner fees). It rides on the largest leg so the node wallet is
        # debited fully instead of stranding the difference — booking it per
        # leg would count every other leg's amount as a "fee" once each.
        close_fee = 0
        balance = group["balance_msat"]
        if balance > group["total_msat"]:
            close_fee = balance - group["total_msat"]
        fee_leg_id = None
        if close_fee > 0 and group["legs"]:
            fee_leg_id = str(
                _field(
                    max(
                        group["legs"],
                        key=lambda leg: (
                            int(_field(leg, "amount") or 0),
                            str(_field(leg, "id")),
                        ),
                    ),
                    "id",
                )
            )
        for tx in group["legs"]:
            if int(_field(tx, "amount") or 0) <= 0:
                continue
            tx_id = str(_field(tx, "id"))
            out_row = _clone_channel_leg(
                tx,
                wallet_refs_by_id[node_wallet_id],
                row_id=f"channel-close:{tx_id}:out:{node_wallet_id}",
                direction="outbound",
                fee=close_fee if tx_id == fee_leg_id else 0,
            )
            pairs.append(
                {
                    "out": out_row,
                    "in": tx,
                    "source": "channel_lifecycle",
                    "kind": CHANNEL_CLOSE,
                }
            )
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
