"""Generic Lightning profitability report.

Computes the routing-revenue, payment-cost, rebalance-cost, on-chain-cost,
net-profit pentad plus per-channel covers-open-cost from a node snapshot.
The adapter is responsible for filling :class:`NodeSnapshot.routing` and
:class:`NodeSnapshot.channels` — this module only reshapes and exports.

Unit convention: the routing summary works in **sat** because adapters
aggregate at the sat granularity that bookkeeper/listforwards APIs
naturally produce; individual :class:`NodeForward` records keep msat
precision. Adapters that compute internally in msat should round
(``msat // 1000``) before filling :class:`NodeRoutingSnapshot`.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from .types import NodeChannel, NodeSnapshot

# Coarse default open-cost heuristic: ~2_500 sat for a 200 vB open+close at
# 12 sat/vB. Adapters that know each channel's exact funding-fee should
# pass it through ``build_profitability_report(default_open_cost_sat=...)``.
# Mock fixtures and the desktop reference this constant so the value stays
# in one place.
DEFAULT_OPEN_COST_SAT: int = 2_500


@dataclass(frozen=True)
class ChannelOpenCostCheck:
    """Per-channel routing earnings vs. a coarse open-cost reference.

    ``covers_open_cost`` is intentionally narrower than "broke even":
    Kassiber doesn't yet model rebalance attribution, on-chain
    amortization, or the channel's actual funding fee. It is ``True`` when
    ``earned_routing_sat >= open_cost_sat`` (default
    :data:`DEFAULT_OPEN_COST_SAT`). Adapters that know their precise open
    cost can pass it through ``build_profitability_report(default_open_cost_sat=...)``.
    """

    channel_id: str
    peer_alias: str
    capacity_sat: int
    earned_routing_sat: int
    open_cost_sat: int
    covers_open_cost: bool


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
    channels: tuple[ChannelOpenCostCheck, ...]

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
                    "coversOpenCost": channel.covers_open_cost,
                }
                for channel in self.channels
            ],
        }

    def to_ai_envelope_payload(self) -> dict[str, Any]:
        """Redacted AI variant: aggregate-only profitability.

        Tier-3 opsec policy (``docs/reference/lightning-opsec.md``) keeps
        the operator's connection identifiers and per-channel peer aliases
        / channel ids out of AI tool output. The aggregate routing summary
        + ``windowLabel`` answer "did this node earn anything this
        window?" without exposing the per-channel identity graph that
        per-channel rows would leak (``channelId`` is a short channel id;
        ``peerAlias`` is gossip-sourced user content).
        """
        return {
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
        }


def build_profitability_report(
    *,
    connection_id: str,
    connection_label: str,
    connection_kind: str,
    snapshot: NodeSnapshot,
    default_open_cost_sat: int = DEFAULT_OPEN_COST_SAT,
) -> LightningProfitabilityReport:
    """Reshape a :class:`NodeSnapshot` into a profitability report.

    The snapshot's ``routing`` block is taken as authoritative for the
    summary numbers; per-channel ``covers_open_cost`` is derived from
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
            channels=tuple(_channel_open_cost_checks(snapshot.channels, default_open_cost_sat)),
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
        channels=tuple(_channel_open_cost_checks(snapshot.channels, default_open_cost_sat)),
    )


def _channel_open_cost_checks(
    channels: tuple[NodeChannel, ...],
    default_open_cost_sat: int,
) -> list[ChannelOpenCostCheck]:
    rows: list[ChannelOpenCostCheck] = []
    for channel in channels:
        earned = channel.earned_routing_sat or 0
        rows.append(
            ChannelOpenCostCheck(
                channel_id=channel.id,
                peer_alias=channel.peer_alias,
                capacity_sat=channel.capacity_sat,
                earned_routing_sat=earned,
                open_cost_sat=default_open_cost_sat,
                covers_open_cost=earned >= default_open_cost_sat,
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
                f" covers_open_cost={'yes' if channel.covers_open_cost else 'no'};"
                f" id={channel.channel_id}",
            ]
        )
    return rows


def _sat_to_btc(value: int) -> str:
    return f"{value / 100_000_000:.8f}"
