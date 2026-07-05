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
        elif direction == "inbound" and key in closing:
            roles[tx_id] = CHANNEL_CLOSE
    return roles
