"""Lightning adapter Protocol.

Implementations live in sibling modules (e.g. ``lnd.py``, ``cln.py``) and
register themselves with :mod:`kassiber.core.lightning.registry`. Every
adapter must be strictly read-only — Kassiber never closes channels,
opens channels, or pays invoices through a Lightning node.

Adapters are also the **discard boundary** for sensitive node data: see
[docs/reference/lightning-opsec.md](../../../docs/reference/lightning-opsec.md)
for the full policy. Summary: drop preimages, payment_secrets, encoded
bolt11 strings, route hop pubkey lists, route hints from received
invoices, and ``failure_source_pubkey`` before populating
:class:`NodeSnapshot`; pass ``None`` for :attr:`NodeChannel.peer_pubkey`
on private channels by default (enforced by ``NodeChannel.__post_init__``);
aggregate per-forward data at the day-per-channel grain when persisting.
"""

from __future__ import annotations

from typing import Any, Protocol

from .types import NodeSnapshot


class LightningAdapter(Protocol):
    """Read-only Lightning node adapter.

    Connections are resolved by their ``kind`` (`lnd`, `core-ln`, `nwc`,
    …). The daemon hands the adapter a ``connection`` row (dict-like) and
    a ``backend`` row (dict-like or ``None``) — the adapter is responsible
    for talking to the node and shaping a :class:`NodeSnapshot`.

    The ``window_days`` argument bounds the routing/forwards window the
    snapshot should cover; adapters may ignore it and return their own
    natural window, but the desktop assumes 30 days by default.
    """

    kind: str

    def fetch_node_snapshot(
        self,
        connection: dict[str, Any],
        backend: dict[str, Any] | None,
        *,
        window_days: int = 30,
    ) -> NodeSnapshot: ...
