"""Lightning node integration scaffold.

Provides the shared shape used by node adapters (LND, Core Lightning, NWC,
…). Each adapter implements the :class:`LightningAdapter` protocol and
registers itself with the registry. Generic node-status snapshots and
routing-profitability reports are built on top of the adapter interface so
the CLI, daemon, and frontend stay node-agnostic.

Adapters belong under :mod:`kassiber.core.lightning` (e.g. ``lnd.py``,
``cln.py``); the scaffold itself never talks to a node directly.
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
