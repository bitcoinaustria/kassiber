"""Typed Lightning adapter capability contract.

Capabilities are safe metadata: they describe which read-only Lightning
features an adapter implements, without exposing node identity, peer graph,
backend endpoints, tokens, descriptors, or wallet config.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Literal, Mapping

from ...errors import AppError

LightningCapability = Literal[
    "node_snapshot",
    "routing_profitability",
    "channel_balances",
    "channel_lifecycle",
    "forward_events",
    "invoice_activity",
    "payment_activity",
    "onchain_balance",
]

LIGHTNING_CAPABILITY_NAMES: tuple[LightningCapability, ...] = (
    "node_snapshot",
    "routing_profitability",
    "channel_balances",
    "channel_lifecycle",
    "forward_events",
    "invoice_activity",
    "payment_activity",
    "onchain_balance",
)

_CAPABILITY_WIRE_KEYS: dict[LightningCapability, str] = {
    "node_snapshot": "nodeSnapshot",
    "routing_profitability": "routingProfitability",
    "channel_balances": "channelBalances",
    "channel_lifecycle": "channelLifecycle",
    "forward_events": "forwardEvents",
    "invoice_activity": "invoiceActivity",
    "payment_activity": "paymentActivity",
    "onchain_balance": "onchainBalance",
}

@dataclass(frozen=True)
class LightningCapabilities:
    """Read-only features a Lightning adapter can satisfy."""

    node_snapshot: bool = False
    routing_profitability: bool = False
    channel_balances: bool = False
    channel_lifecycle: bool = False
    forward_events: bool = False
    invoice_activity: bool = False
    payment_activity: bool = False
    onchain_balance: bool = False

    def supports(self, capability: LightningCapability) -> bool:
        return bool(getattr(self, capability))

    def supported_capabilities(self) -> tuple[LightningCapability, ...]:
        return tuple(
            name for name in LIGHTNING_CAPABILITY_NAMES if self.supports(name)
        )

    def to_wire_dict(self) -> dict[str, bool]:
        return {
            _CAPABILITY_WIRE_KEYS[name]: self.supports(name)
            for name in LIGHTNING_CAPABILITY_NAMES
        }


EMPTY_LIGHTNING_CAPABILITIES = LightningCapabilities()


def normalize_lightning_capabilities(value: Any) -> LightningCapabilities:
    """Coerce adapter-declared capabilities into the stable dataclass."""

    if isinstance(value, LightningCapabilities):
        return value
    if isinstance(value, Mapping):
        kwargs: dict[str, bool] = {}
        for name in LIGHTNING_CAPABILITY_NAMES:
            wire_key = _CAPABILITY_WIRE_KEYS[name]
            kwargs[name] = bool(value.get(name) or value.get(wire_key))
        return LightningCapabilities(**kwargs)
    return EMPTY_LIGHTNING_CAPABILITIES


def lightning_capabilities_from_adapter(
    adapter: object | None,
) -> LightningCapabilities:
    """Return the explicit capability declaration for ``adapter``.

    Missing declarations are treated as no capabilities. That makes older or
    incomplete adapters fail through the deterministic
    ``lightning_capability_unsupported`` path instead of crashing later in a
    backend-specific transport call.
    """

    if adapter is None:
        return EMPTY_LIGHTNING_CAPABILITIES
    raw = getattr(adapter, "capabilities", None)
    if callable(raw):
        raw = raw()
    return normalize_lightning_capabilities(raw)


def lightning_capabilities_to_wire(value: Any) -> dict[str, bool]:
    return normalize_lightning_capabilities(value).to_wire_dict()


def require_lightning_capability(
    *,
    kind: str,
    adapter: object,
    capability: LightningCapability,
) -> LightningCapabilities:
    capabilities = lightning_capabilities_from_adapter(adapter)
    if capabilities.supports(capability):
        return capabilities
    supported = list(capabilities.supported_capabilities())
    raise AppError(
        (
            f"Lightning adapter for kind '{kind}' does not support"
            f" {capability.replace('_', ' ')}."
        ),
        code="lightning_capability_unsupported",
        hint="Use a Lightning connection whose adapter supports this feature.",
        details={
            "kind": kind,
            "capability": capability,
            "supported_capabilities": supported,
        },
        retryable=False,
    )
