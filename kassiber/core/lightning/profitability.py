"""Generic Lightning profitability report.

Computes the routing-revenue, payment-cost, rebalance-cost, on-chain-cost,
net-profit pentad plus per-channel break-even from a node snapshot. The
adapter is responsible for filling :class:`NodeSnapshot.routing` and
:class:`NodeSnapshot.channels` — this module only reshapes and exports.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from .types import NodeChannel, NodeSnapshot


@dataclass(frozen=True)
class ChannelBreakEven:
    """Per-channel routing earnings used as a coarse break-even indicator.

    Kassiber doesn't model the exact channel-open on-chain fee per channel
    today, so ``break_even`` is reported as ``True`` when
    ``earned_routing_sat`` exceeds ``open_cost_sat`` (a coarse 2_500 sat
    default). Adapters that know their precise open cost can compute
    break-even themselves and pass it through here.
    """

    channel_id: str
    peer_alias: str
    capacity_sat: int
    earned_routing_sat: int
    open_cost_sat: int
    break_even: bool


@dataclass(frozen=True)
class LightningProfitabilityReport:
    connection_id: str
    connection_label: str
    connection_kind: str
    window_label: str
    routing_revenue_sat: int
    payment_cost_sat: int
    rebalance_cost_sat: int
    onchain_cost_sat: int
    net_profit_sat: int
    forward_count: int
    payment_count: int
    rebalance_count: int
    channels: tuple[ChannelBreakEven, ...]

    def to_envelope_payload(self) -> dict[str, Any]:
        return {
            "connection": {
                "id": self.connection_id,
                "label": self.connection_label,
                "kind": self.connection_kind,
            },
            "windowLabel": self.window_label,
            "summary": {
                "routingRevenueSat": self.routing_revenue_sat,
                "paymentCostSat": self.payment_cost_sat,
                "rebalanceCostSat": self.rebalance_cost_sat,
                "onchainCostSat": self.onchain_cost_sat,
                "netProfitSat": self.net_profit_sat,
                "forwardCount": self.forward_count,
                "paymentCount": self.payment_count,
                "rebalanceCount": self.rebalance_count,
            },
            "channels": [
                {
                    "channelId": channel.channel_id,
                    "peerAlias": channel.peer_alias,
                    "capacitySat": channel.capacity_sat,
                    "earnedRoutingSat": channel.earned_routing_sat,
                    "openCostSat": channel.open_cost_sat,
                    "breakEven": channel.break_even,
                }
                for channel in self.channels
            ],
        }


def build_profitability_report(
    *,
    connection_id: str,
    connection_label: str,
    connection_kind: str,
    snapshot: NodeSnapshot,
    default_open_cost_sat: int = 2_500,
) -> LightningProfitabilityReport:
    """Reshape a :class:`NodeSnapshot` into a profitability report.

    The snapshot's ``routing`` block is taken as authoritative for the
    summary numbers; per-channel break-even is derived from
    ``earned_routing_sat`` vs. ``default_open_cost_sat``.
    """
    routing = snapshot.routing
    if routing is None:
        return LightningProfitabilityReport(
            connection_id=connection_id,
            connection_label=connection_label,
            connection_kind=connection_kind,
            window_label="No routing window reported",
            routing_revenue_sat=0,
            payment_cost_sat=0,
            rebalance_cost_sat=0,
            onchain_cost_sat=0,
            net_profit_sat=0,
            forward_count=0,
            payment_count=0,
            rebalance_count=0,
            channels=tuple(_channel_break_evens(snapshot.channels, default_open_cost_sat)),
        )
    return LightningProfitabilityReport(
        connection_id=connection_id,
        connection_label=connection_label,
        connection_kind=connection_kind,
        window_label=routing.window_label,
        routing_revenue_sat=routing.routing_revenue_sat,
        payment_cost_sat=routing.payment_cost_sat,
        rebalance_cost_sat=routing.rebalance_cost_sat,
        onchain_cost_sat=routing.onchain_cost_sat,
        net_profit_sat=routing.net_profit_sat,
        forward_count=routing.forward_count,
        payment_count=routing.payment_count,
        rebalance_count=routing.rebalance_count,
        channels=tuple(_channel_break_evens(snapshot.channels, default_open_cost_sat)),
    )


def _channel_break_evens(
    channels: tuple[NodeChannel, ...],
    default_open_cost_sat: int,
) -> list[ChannelBreakEven]:
    rows: list[ChannelBreakEven] = []
    for channel in channels:
        earned = channel.earned_routing_sat or 0
        rows.append(
            ChannelBreakEven(
                channel_id=channel.id,
                peer_alias=channel.peer_alias,
                capacity_sat=channel.capacity_sat,
                earned_routing_sat=earned,
                open_cost_sat=default_open_cost_sat,
                break_even=earned >= default_open_cost_sat,
            )
        )
    return rows


def profitability_csv_rows(
    report: LightningProfitabilityReport,
) -> list[list[str]]:
    """Return CSV rows (header first) for ``reports export-lightning-profitability-csv``."""
    header = [
        "section",
        "key",
        "value_sat",
        "value_btc",
        "detail",
    ]
    rows: list[list[str]] = [header]
    for label, value in (
        ("routing_revenue", report.routing_revenue_sat),
        ("payment_cost", report.payment_cost_sat),
        ("rebalance_cost", report.rebalance_cost_sat),
        ("onchain_cost", report.onchain_cost_sat),
        ("net_profit", report.net_profit_sat),
    ):
        rows.append(
            [
                "summary",
                label,
                str(value),
                _sat_to_btc(value),
                report.window_label,
            ]
        )
    for label, value in (
        ("forward_count", report.forward_count),
        ("payment_count", report.payment_count),
        ("rebalance_count", report.rebalance_count),
    ):
        rows.append(["summary", label, str(value), "", report.window_label])
    for channel in report.channels:
        rows.append(
            [
                "channel",
                channel.peer_alias,
                str(channel.earned_routing_sat),
                _sat_to_btc(channel.earned_routing_sat),
                f"capacity={channel.capacity_sat} sat;"
                f" open_cost={channel.open_cost_sat} sat;"
                f" break_even={'yes' if channel.break_even else 'no'};"
                f" id={channel.channel_id}",
            ]
        )
    return rows


def _sat_to_btc(value: int) -> str:
    return f"{value / 100_000_000:.8f}"
