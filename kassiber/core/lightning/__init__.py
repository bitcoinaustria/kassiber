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
route hop pubkey lists, route hints from received invoices, and
``failure_source_pubkey`` — none of which contribute to routine tax
computation and all of which endanger other users if they end up in a
leaked DB. Adapters MUST discard these at the adapter boundary, never
persist them, and never let them reach a :class:`NodeSnapshot` field. The
shapes in :mod:`kassiber.core.lightning.types` deliberately have nowhere
to put that data so adapters cannot leak it by accident.
"""

from .adapter import LightningAdapter
from .connections import LIGHTNING_ADAPTER_KINDS, resolve_lightning_connection
from .profitability import (
    DEFAULT_OPEN_COST_SAT,
    ChannelOpenCostCheck,
    LightningProfitabilityReport,
    build_profitability_report,
    profitability_csv_rows,
)
from .registry import (
    register_adapter,
    registered_kinds,
    resolve_adapter,
    unregister_adapter,
)
from .types import (
    NodeChannel,
    NodeChannelState,
    NodeForward,
    NodeForwardFailureReason,
    NodeForwardStatus,
    NodeRoutingSnapshot,
    NodeSnapshot,
    snapshot_to_dict,
    snapshot_to_dict_for_ai,
)

# Import side-effect: each adapter module registers itself with the registry
# at import time. Listing them here ensures the daemon and CLI can resolve
# the adapter without an explicit import.
from . import cln as _cln  # noqa: F401  -- register_adapter("coreln", ...)

__all__ = [
    "ChannelOpenCostCheck",
    "DEFAULT_OPEN_COST_SAT",
    "LIGHTNING_ADAPTER_KINDS",
    "LightningAdapter",
    "LightningProfitabilityReport",
    "NodeChannel",
    "NodeChannelState",
    "NodeForward",
    "NodeForwardFailureReason",
    "NodeForwardStatus",
    "NodeRoutingSnapshot",
    "NodeSnapshot",
    "build_profitability_report",
    "profitability_csv_rows",
    "register_adapter",
    "registered_kinds",
    "resolve_adapter",
    "resolve_lightning_connection",
    "snapshot_to_dict",
    "snapshot_to_dict_for_ai",
    "unregister_adapter",
]
