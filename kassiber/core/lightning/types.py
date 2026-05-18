"""Typed shapes for Lightning node snapshots.

These mirror the TypeScript `NodeSnapshot` / `NodeChannel` / `NodeForward`
shapes that the desktop reads from `ui.connections.node.snapshot`. Keep the
field names and units consistent — the daemon serializes these straight
through to JSON. Amounts are stored as integer sat / msat per project
convention.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Literal

NodeChannelState = Literal[
    "active",
    "inactive",
    "pending_open",
    "pending_close",
    "closed",
    "force_closed",
]

NodeForwardStatus = Literal["settled", "failed", "offered"]


@dataclass(frozen=True)
class NodeChannel:
    id: str
    peer_alias: str
    peer_pubkey: str
    capacity_sat: int
    local_balance_sat: int
    remote_balance_sat: int
    state: NodeChannelState
    is_private: bool = False
    is_initiator: bool = True
    short_channel_id: str | None = None
    funding_outpoint: str | None = None
    base_fee_msat: int | None = None
    fee_rate_ppm: int | None = None
    opened_at: str | None = None
    closed_at: str | None = None
    close_kind: Literal["cooperative", "force", "breach"] | None = None
    forward_count: int | None = None
    earned_routing_sat: int | None = None
    htlc_count: int | None = None
    last_activity_at: str | None = None


@dataclass(frozen=True)
class NodeRoutingSnapshot:
    window_label: str
    routing_revenue_sat: int
    payment_cost_sat: int
    rebalance_cost_sat: int
    onchain_cost_sat: int
    net_profit_sat: int
    forward_count: int
    payment_count: int
    rebalance_count: int


@dataclass(frozen=True)
class NodeForward:
    id: str
    occurred_at: str
    in_peer_alias: str
    out_peer_alias: str
    amount_in_msat: int
    amount_out_msat: int
    fee_msat: int
    status: NodeForwardStatus
    in_short_channel_id: str | None = None
    out_short_channel_id: str | None = None
    failure_reason: str | None = None


@dataclass(frozen=True)
class NodeSnapshot:
    alias: str
    pubkey: str
    network: str
    peer_count: int
    onchain_balance_sat: int
    total_local_balance_sat: int
    total_remote_balance_sat: int
    total_capacity_sat: int
    channels: tuple[NodeChannel, ...] = field(default_factory=tuple)
    closed_channels: tuple[NodeChannel, ...] = field(default_factory=tuple)
    implementation_version: str | None = None
    block_height: int | None = None
    routing: NodeRoutingSnapshot | None = None
    forwards: tuple[NodeForward, ...] = field(default_factory=tuple)


def channel_to_dict(channel: NodeChannel) -> dict[str, Any]:
    return {
        "id": channel.id,
        "shortChannelId": channel.short_channel_id,
        "fundingOutpoint": channel.funding_outpoint,
        "peerAlias": channel.peer_alias,
        "peerPubkey": channel.peer_pubkey,
        "capacitySat": channel.capacity_sat,
        "localBalanceSat": channel.local_balance_sat,
        "remoteBalanceSat": channel.remote_balance_sat,
        "state": channel.state,
        "isPrivate": channel.is_private,
        "isInitiator": channel.is_initiator,
        "baseFeeMsat": channel.base_fee_msat,
        "feeRatePpm": channel.fee_rate_ppm,
        "openedAt": channel.opened_at,
        "closedAt": channel.closed_at,
        "closeKind": channel.close_kind,
        "forwardCount": channel.forward_count,
        "earnedRoutingSat": channel.earned_routing_sat,
        "htlcCount": channel.htlc_count,
        "lastActivityAt": channel.last_activity_at,
    }


def routing_to_dict(routing: NodeRoutingSnapshot) -> dict[str, Any]:
    return {
        "windowLabel": routing.window_label,
        "routingRevenueSat": routing.routing_revenue_sat,
        "paymentCostSat": routing.payment_cost_sat,
        "rebalanceCostSat": routing.rebalance_cost_sat,
        "onchainCostSat": routing.onchain_cost_sat,
        "netProfitSat": routing.net_profit_sat,
        "forwardCount": routing.forward_count,
        "paymentCount": routing.payment_count,
        "rebalanceCount": routing.rebalance_count,
    }


def forward_to_dict(forward: NodeForward) -> dict[str, Any]:
    return {
        "id": forward.id,
        "occurredAt": forward.occurred_at,
        "inPeerAlias": forward.in_peer_alias,
        "inShortChannelId": forward.in_short_channel_id,
        "outPeerAlias": forward.out_peer_alias,
        "outShortChannelId": forward.out_short_channel_id,
        "amountInMsat": forward.amount_in_msat,
        "amountOutMsat": forward.amount_out_msat,
        "feeMsat": forward.fee_msat,
        "status": forward.status,
        "failureReason": forward.failure_reason,
    }


def snapshot_to_dict(snapshot: NodeSnapshot) -> dict[str, Any]:
    return {
        "alias": snapshot.alias,
        "pubkey": snapshot.pubkey,
        "network": snapshot.network,
        "implementationVersion": snapshot.implementation_version,
        "peerCount": snapshot.peer_count,
        "blockHeight": snapshot.block_height,
        "onchainBalanceSat": snapshot.onchain_balance_sat,
        "totalLocalBalanceSat": snapshot.total_local_balance_sat,
        "totalRemoteBalanceSat": snapshot.total_remote_balance_sat,
        "totalCapacitySat": snapshot.total_capacity_sat,
        "channels": [channel_to_dict(channel) for channel in snapshot.channels],
        "closedChannels": [
            channel_to_dict(channel) for channel in snapshot.closed_channels
        ],
        "routing": routing_to_dict(snapshot.routing) if snapshot.routing else None,
        "forwards": [forward_to_dict(forward) for forward in snapshot.forwards],
    }
