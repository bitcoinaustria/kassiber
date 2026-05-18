"""Lightning node integration scaffold.

Provides the shared shape used by node adapters (LND, Core Lightning, NWC,
…). Each adapter implements the :class:`LightningAdapter` protocol and
registers itself with the registry. Generic node-status snapshots and
routing-profitability reports are built on top of the adapter interface so
the CLI, daemon, and frontend stay node-agnostic.

Adapters belong under :mod:`kassiber.core.lightning` (e.g. ``lnd.py``,
``cln.py``); the scaffold itself never talks to a node directly.

Opsec policy — REQUIRED reading for adapter authors:
[docs/reference/lightning-opsec.md](../../../docs/reference/lightning-opsec.md).
Lightning APIs expose preimages, payment_secrets, encoded bolt11 blobs,
onion route hops, route hints from received invoices, and
``failure_source_pubkey`` — none of which have tax value and all of which
endanger other users if they end up in a leaked DB. Adapters MUST discard
these at the adapter boundary, never persist them, and never let them
reach a :class:`NodeSnapshot` field. The shapes in
:mod:`kassiber.core.lightning.types` deliberately have nowhere to put
that data so adapters cannot leak it by accident.
"""

from .adapter import LightningAdapter
from .connections import LIGHTNING_WALLET_KINDS, resolve_lightning_connection
from .profitability import (
    DEFAULT_OPEN_COST_SAT,
    LightningProfitabilityReport,
    build_profitability_report,
    profitability_csv_rows,
)
from .registry import register_adapter, resolve_adapter, unregister_adapter
from .types import (
    NodeChannel,
    NodeChannelState,
    NodeForward,
    NodeForwardStatus,
    NodeRoutingSnapshot,
    NodeSnapshot,
    snapshot_to_dict,
)

__all__ = [
    "DEFAULT_OPEN_COST_SAT",
    "LIGHTNING_WALLET_KINDS",
    "LightningAdapter",
    "LightningProfitabilityReport",
    "NodeChannel",
    "NodeChannelState",
    "NodeForward",
    "NodeForwardStatus",
    "NodeRoutingSnapshot",
    "NodeSnapshot",
    "build_profitability_report",
    "profitability_csv_rows",
    "register_adapter",
    "resolve_adapter",
    "resolve_lightning_connection",
    "snapshot_to_dict",
    "unregister_adapter",
]
