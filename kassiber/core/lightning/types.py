"""Typed shapes for Lightning node snapshots.

These mirror the TypeScript `NodeSnapshot` / `NodeChannel` / `NodeForward`
shapes that the desktop reads from `ui.connections.node.snapshot`. Keep the
field names and units consistent — the daemon serializes these straight
through to JSON. Amounts are stored as integer sat / msat per project
convention.

Opsec note: see [docs/reference/lightning-opsec.md](../../../docs/reference/lightning-opsec.md)
for the discard policy adapters must apply *before* filling these shapes —
preimages, payment_secrets, encoded bolt11 strings, route hop pubkey
lists, route hints from received invoices, and `failure_source_pubkey`
never belong on the wire. Note in particular that ``NodeChannel.peer_pubkey``
is intentionally optional: adapters should pass ``None`` for private
channels (``is_private=True``) unless the operator explicitly opts in,
because the peer chose private gossip for a reason and Kassiber should
not undo that decision in a local DB that might leak via diagnostics —
``NodeChannel.__post_init__`` enforces the ``None``-for-private rule at
construction time so adapters that forget this fail fast at the dataclass
boundary, not at wire serialization time.
``NodeForward`` deliberately carries only short channel ids and peer
*aliases*, never peer pubkeys, for the same reason; the ``failure_reason``
is a categorical enum so adapters cannot smuggle raw node error blobs
through the field.
"""

from __future__ import annotations

import re
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

#: Runtime-enforced allowlist matching :data:`NodeForwardStatus`. The
#: ``Literal`` alias above is a type-checker hint only; the frozen set
#: lets :meth:`NodeForward.__post_init__` reject e.g. ``status="bogus"``
#: at construction time so adapters cannot smuggle raw node state codes
#: into the wire payload by accident.
_NODE_FORWARD_STATUSES: frozenset[str] = frozenset({"settled", "failed", "offered"})

#: Categorical failure reasons for :class:`NodeForward`. Adapters MUST map
#: raw node error strings into one of these buckets and use ``"other"``
#: when the underlying code doesn't fit. Keeping this categorical prevents
#: adapters from accidentally smuggling ``failure_source_pubkey``, payment
#: hashes, route-hint JSON, or other Tier-1 material through what looks
#: like a free-text field. See ``docs/reference/lightning-opsec.md``.
NodeForwardFailureReason = Literal[
    "temporary_channel_failure",
    "unknown_next_peer",
    "fee_insufficient",
    "incorrect_payment_details",
    "expiry_too_soon",
    "insufficient_balance",
    "other",
]

#: Runtime-enforced allowlist matching :data:`NodeForwardFailureReason`.
#: The ``Literal`` alias above is type-checker-only. Without this set,
#: an adapter that writes ``failure_reason="failure_source_pubkey=02..."``
#: would silently pass through to the wire payload — exactly the Tier-1
#: leak the categorical enum exists to prevent. Adapters that need a
#: bucket for an unknown failure type use ``"other"``.
_NODE_FORWARD_FAILURE_REASONS: frozenset[str] = frozenset({
    "temporary_channel_failure",
    "unknown_next_peer",
    "fee_insufficient",
    "incorrect_payment_details",
    "expiry_too_soon",
    "insufficient_balance",
    "other",
})

# Short-channel-id format check. Two structured representations are
# accepted, matching the two adapter ecosystems:
#   - BOLT-7 ``<block>x<tx>x<output>`` (Core Lightning native form).
#   - Decimal uint64 (LND native form — chan_id encodes block/tx/output
#     in a single 64-bit integer; LND's REST returns it as a numeric
#     string).
# Both are 1..20 digit / 'x'-separated structured identifiers; the
# regex still rejects whitespace, colons, slashes, hex pubkeys, JSON
# braces, and other free-text smuggling. The check is format-only — it
# does not validate that the block height or chan_id exists.
_SHORT_CHANNEL_ID_RE = re.compile(r"^(?:[0-9]+x[0-9]+x[0-9]+|[0-9]{1,20})$")
# Outpoint format check: 64-hex txid + ``:`` + non-negative integer vout.
_OUTPOINT_RE = re.compile(r"^[0-9a-fA-F]{64}:[0-9]+$")


def assert_lightning_field_format(value: str | None) -> None:
    """Lightweight format-only validator for ``short_channel_id``-shaped fields.

    Adapters may legitimately omit the field (``None``); when present it
    must look like a structured short channel id — either BOLT-7
    ``<block>x<tx>x<output>`` (Core Lightning native form) or a decimal
    uint64 (LND native form). The check is intentionally format-only —
    it does not verify the block height exists. The point is to reject
    obvious smuggling (free text, JSON blobs, hex pubkeys) at the
    construction boundary.
    """
    if value is None:
        return
    if not isinstance(value, str) or not _SHORT_CHANNEL_ID_RE.match(value):
        raise ValueError(
            "short_channel_id must look like '<block>x<tx>x<output>' (BOLT-7)"
            " or a decimal uint64 (LND);"
            f" got {value!r}. Adapters: keep raw node strings out of typed fields."
        )


def assert_outpoint_format(value: str | None) -> None:
    """Lightweight format-only validator for ``funding_outpoint``-shaped fields."""
    if value is None:
        return
    if not isinstance(value, str) or not _OUTPOINT_RE.match(value):
        raise ValueError(
            "funding_outpoint must be '<txid_hex>:<vout>' (64 hex chars + colon"
            f" + non-negative integer); got {value!r}."
        )


@dataclass(frozen=True)
class NodeChannel:
    id: str
    peer_alias: str
    #: Hex node id of the channel peer. Adapters MUST set this to ``None``
    #: for private channels (``is_private=True``) — this is enforced at
    #: construction time by :meth:`__post_init__`. Public-channel peer ids
    #: are already in gossip so the marginal privacy cost is near zero;
    #: private-channel peers chose private gossip for a reason and Kassiber
    #: must not undo that decision in a local DB that might leak via
    #: diagnostics or backups.
    peer_pubkey: str | None
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

    def __post_init__(self) -> None:
        # Opsec policy: a private channel's peer pubkey must never reach
        # the typed surface. See docs/reference/lightning-opsec.md. We
        # check at construction time so an adapter that violates the
        # policy fails immediately at the dataclass boundary instead of
        # silently shipping the pubkey to SQLite / the daemon / AI tools.
        # Frozen dataclasses still execute __post_init__, so this works
        # without re-implementing __init__.
        if self.is_private and self.peer_pubkey is not None:
            raise ValueError(
                "NodeChannel: peer_pubkey must be None for private channels"
                " (is_private=True). See docs/reference/lightning-opsec.md."
                " Adapters that need to surface a private-channel peer id"
                " must do so on explicit operator opt-in via a separate"
                " field, not by populating peer_pubkey."
            )
        assert_lightning_field_format(self.short_channel_id)
        assert_outpoint_format(self.funding_outpoint)


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
    failure_reason: NodeForwardFailureReason | None = None

    def __post_init__(self) -> None:
        assert_lightning_field_format(self.in_short_channel_id)
        assert_lightning_field_format(self.out_short_channel_id)
        # ``status`` and ``failure_reason`` are ``Literal`` aliases — that
        # is a type-checker hint only. Enforce the allowlist at construction
        # time so an adapter cannot ship a raw node code (e.g.
        # ``failure_reason='failure_source_pubkey=02...'``) through what
        # looks like a categorical field. See docs/reference/lightning-opsec.md.
        if self.status not in _NODE_FORWARD_STATUSES:
            raise ValueError(
                "NodeForward.status must be one of"
                f" {sorted(_NODE_FORWARD_STATUSES)}; got {self.status!r}."
                " Adapters: map raw node states to a category before"
                " populating."
            )
        if (
            self.failure_reason is not None
            and self.failure_reason not in _NODE_FORWARD_FAILURE_REASONS
        ):
            raise ValueError(
                "NodeForward.failure_reason must be one of"
                f" {sorted(_NODE_FORWARD_FAILURE_REASONS)};"
                f" got {self.failure_reason!r}. Adapters: map raw node"
                " errors to a category before populating (use 'other'"
                " when the underlying code does not fit)."
            )


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


def _channel_to_dict_for_ai(channel: NodeChannel) -> dict[str, Any]:
    """Channel payload for AI tools: operational shape minus identity graph.

    Drops every field tagged Tier 3 in ``docs/reference/lightning-opsec.md``:
    ``shortChannelId`` (encodes block height → on-chain timing leak),
    ``fundingOutpoint``, ``peerAlias`` (user-content from gossip),
    ``peerPubkey`` (even on public channels — Tier 3 says "never in AI
    tool output", and one rule is easier to reason about than a "but
    only for public" carveout). Keeps balances, capacities, fee policies,
    states, opened/closed timestamps, routing counts so the AI can still
    answer "is this channel earning?" or "is this channel imbalanced?".
    """
    return {
        "id": channel.id,
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


def _forward_to_dict_for_ai(forward: NodeForward) -> dict[str, Any]:
    """Forward payload for AI tools: drops every peer/channel identifier.

    The aggregate routing summary on :class:`NodeRoutingSnapshot` already
    answers "did this node earn anything this window?". Per-forward rows
    with peer aliases and short channel ids let an AI assistant — or
    anyone reading the tool transcript — reconstruct the operator's
    routing graph. Drop ``inPeerAlias`` / ``outPeerAlias`` /
    ``inShortChannelId`` / ``outShortChannelId`` and keep only the
    operational shape (amounts, fee, status, occurred-at, failure
    bucket).
    """
    return {
        "id": forward.id,
        "occurredAt": forward.occurred_at,
        "amountInMsat": forward.amount_in_msat,
        "amountOutMsat": forward.amount_out_msat,
        "feeMsat": forward.fee_msat,
        "status": forward.status,
        "failureReason": forward.failure_reason,
    }


def snapshot_to_dict_for_ai(snapshot: NodeSnapshot) -> dict[str, Any]:
    """Redacted ``snapshot_to_dict`` for AI tool surfaces.

    Tier-3 identifiers from ``docs/reference/lightning-opsec.md`` are
    dropped here so the AI tool never receives them, even by accident:

    * operator's own ``pubkey`` → ``None``
    * per-channel ``shortChannelId`` / ``fundingOutpoint`` / ``peerPubkey``
      / ``peerAlias`` → omitted
    * per-forward ``inPeerAlias`` / ``outPeerAlias`` / ``inShortChannelId``
      / ``outShortChannelId`` → omitted

    Kept: alias, network, peer count, block height, balances, capacities,
    counts, fee policies, channel states, the routing summary, and the
    operator's own connection label (their own wallet label is theirs, not
    someone else's identity). The shape stays JSON-stable: keys are
    present but redacted, so a downstream consumer that does
    ``payload["pubkey"]`` does not crash.
    """
    return {
        "alias": snapshot.alias,
        "pubkey": None,
        "network": snapshot.network,
        "implementationVersion": snapshot.implementation_version,
        "peerCount": snapshot.peer_count,
        "blockHeight": snapshot.block_height,
        "onchainBalanceSat": snapshot.onchain_balance_sat,
        "totalLocalBalanceSat": snapshot.total_local_balance_sat,
        "totalRemoteBalanceSat": snapshot.total_remote_balance_sat,
        "totalCapacitySat": snapshot.total_capacity_sat,
        "channels": [_channel_to_dict_for_ai(channel) for channel in snapshot.channels],
        "closedChannels": [
            _channel_to_dict_for_ai(channel) for channel in snapshot.closed_channels
        ],
        "routing": routing_to_dict(snapshot.routing) if snapshot.routing else None,
        "forwards": [_forward_to_dict_for_ai(forward) for forward in snapshot.forwards],
    }
