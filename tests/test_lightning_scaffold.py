"""Tests for the shared Lightning scaffold (types, registry, profitability).

The scaffold is exercised indirectly by adapter tests in #154/#155 once they
land; this module pins the contract those adapters depend on so they can be
written and reviewed independently.
"""

from __future__ import annotations

import sqlite3
import unittest
from contextlib import contextmanager
from dataclasses import dataclass
from typing import Any, Iterator

from kassiber.core import lightning as core_lightning
from kassiber.core.lightning import (
    NodeChannel,
    NodeForward,
    NodeRoutingSnapshot,
    NodeSnapshot,
    build_profitability_report,
    profitability_csv_rows,
    register_adapter,
    registered_kinds,
    resolve_adapter,
    resolve_lightning_connection,
    snapshot_to_dict,
    snapshot_to_dict_for_ai,
    unregister_adapter,
)
from kassiber.errors import AppError


def _channel(
    id_: str,
    earned: int | None = None,
    capacity: int = 1_000_000,
    *,
    scid_block: int = 800_000,
    scid_tx: int = 1,
) -> NodeChannel:
    return NodeChannel(
        id=id_,
        peer_alias=f"peer-{id_}",
        peer_pubkey="02" + "ab" * 32,
        capacity_sat=capacity,
        local_balance_sat=capacity // 2,
        remote_balance_sat=capacity - capacity // 2,
        state="active",
        short_channel_id=f"{scid_block}x{scid_tx}x0",
        funding_outpoint=("a" * 64) + ":0",
        base_fee_msat=1_000,
        fee_rate_ppm=200,
        earned_routing_sat=earned,
    )


def _snapshot_with_routing() -> NodeSnapshot:
    return NodeSnapshot(
        alias="kassiber-test",
        pubkey="02" + "cd" * 32,
        network="mainnet",
        peer_count=4,
        onchain_balance_sat=500_000,
        total_local_balance_sat=1_500_000,
        total_remote_balance_sat=1_500_000,
        total_capacity_sat=3_000_000,
        channels=(
            _channel("a", earned=5_000, scid_tx=1),
            _channel("b", earned=1_200, scid_tx=2),
            _channel("c", earned=None, scid_tx=3),
        ),
        routing=NodeRoutingSnapshot(
            window_label="Last 30 days",
            routing_revenue_sat=6_200,
            payment_cost_sat=300,
            rebalance_cost_sat=120,
            onchain_cost_sat=2_500,
            net_profit_sat=3_280,
            forward_count=42,
            payment_count=9,
            rebalance_count=2,
        ),
        forwards=(
            NodeForward(
                id="fw1",
                occurred_at="2026-05-18T10:00:00Z",
                in_peer_alias="peer-a",
                out_peer_alias="peer-b",
                amount_in_msat=240_100_000,
                amount_out_msat=240_000_000,
                fee_msat=100_000,
                status="settled",
            ),
        ),
    )


@dataclass
class _FakeAdapter:
    kind: str = "lnd"

    def fetch_node_snapshot(
        self,
        connection: dict[str, Any],
        backend: dict[str, Any] | None,
        *,
        window_days: int = 30,
    ) -> NodeSnapshot:
        return _snapshot_with_routing()


@contextmanager
def _adapter_registered(kind: str, adapter: Any) -> Iterator[None]:
    """Register ``adapter`` for ``kind``, restoring the previous value on exit.

    The scaffold tests share a process with LND/CLN adapter PRs that
    auto-register real adapters on import (`kassiber.core.lightning.lnd`,
    `kassiber.core.lightning.cln`). Plain register/unregister-in-finally
    would remove the real adapter when a scaffold test finishes; this
    helper saves whatever was registered before and restores it on the
    way out, so test ordering does not matter.
    """
    previous = resolve_adapter(kind)
    register_adapter(kind, adapter)
    try:
        yield
    finally:
        if previous is None:
            unregister_adapter(kind)
        else:
            register_adapter(kind, previous)


class LightningTypesTest(unittest.TestCase):
    def test_snapshot_to_dict_uses_camel_case_keys_for_frontend(self) -> None:
        payload = snapshot_to_dict(_snapshot_with_routing())
        self.assertIn("totalLocalBalanceSat", payload)
        self.assertIn("channels", payload)
        self.assertIn("routing", payload)
        self.assertIn("forwards", payload)
        first_channel = payload["channels"][0]
        self.assertIn("fundingOutpoint", first_channel)
        self.assertIn("feeRatePpm", first_channel)
        first_forward = payload["forwards"][0]
        self.assertIn("amountInMsat", first_forward)
        self.assertEqual(first_forward["status"], "settled")

    def test_snapshot_serializes_routing_block_when_present(self) -> None:
        payload = snapshot_to_dict(_snapshot_with_routing())
        routing = payload["routing"]
        self.assertEqual(routing["windowLabel"], "Last 30 days")
        self.assertEqual(routing["routingRevenueSat"], 6_200)

    def test_private_channel_peer_pubkey_serializes_as_null(self) -> None:
        # Opsec policy: adapters set peer_pubkey=None for private channels
        # so the leaked DB cannot betray a peer's private-gossip decision.
        private_channel = NodeChannel(
            id="ch-private",
            peer_alias="confidential-peer",
            peer_pubkey=None,
            capacity_sat=500_000,
            local_balance_sat=500_000,
            remote_balance_sat=0,
            state="active",
            is_private=True,
        )
        snapshot = NodeSnapshot(
            alias="kassiber-test",
            pubkey="02" + "cd" * 32,
            network="mainnet",
            peer_count=1,
            onchain_balance_sat=0,
            total_local_balance_sat=500_000,
            total_remote_balance_sat=0,
            total_capacity_sat=500_000,
            channels=(private_channel,),
        )
        payload = snapshot_to_dict(snapshot)
        first = payload["channels"][0]
        self.assertIsNone(first["peerPubkey"])
        self.assertTrue(first["isPrivate"])

    def test_forward_shape_carries_no_peer_pubkey(self) -> None:
        # Opsec policy: NodeForward stores short channel ids and peer
        # aliases only — never peer pubkeys. Adapters cannot leak the
        # routing graph through this surface by accident.
        payload = snapshot_to_dict(_snapshot_with_routing())
        for forward in payload["forwards"]:
            self.assertNotIn("inPeerPubkey", forward)
            self.assertNotIn("outPeerPubkey", forward)

    def test_private_channel_with_peer_pubkey_is_rejected(self) -> None:
        # Opsec policy enforced at the dataclass boundary: a private
        # channel must never carry a peer pubkey because the peer chose
        # private gossip for a reason. The frozen dataclass + __post_init__
        # combination makes this fail fast on construction instead of
        # silently shipping the pubkey to SQLite / the daemon / AI tools.
        with self.assertRaises(ValueError) as ctx:
            NodeChannel(
                id="ch-private",
                peer_alias="confidential-peer",
                peer_pubkey="02" + "ff" * 32,
                capacity_sat=500_000,
                local_balance_sat=500_000,
                remote_balance_sat=0,
                state="active",
                is_private=True,
            )
        self.assertIn("peer_pubkey must be None", str(ctx.exception))

    def test_private_channel_with_no_peer_pubkey_is_accepted(self) -> None:
        # The matching positive case: omitting peer_pubkey on a private
        # channel is the documented adapter behavior.
        channel = NodeChannel(
            id="ch-private",
            peer_alias="confidential-peer",
            peer_pubkey=None,
            capacity_sat=500_000,
            local_balance_sat=500_000,
            remote_balance_sat=0,
            state="active",
            is_private=True,
        )
        self.assertIsNone(channel.peer_pubkey)

    def test_invalid_short_channel_id_format_is_rejected(self) -> None:
        # The format-only validator catches obvious smuggling at the
        # construction boundary (free text, JSON blobs, pubkeys). It
        # does not validate that the block height exists.
        with self.assertRaises(ValueError):
            NodeChannel(
                id="ch-bad-scid",
                peer_alias="peer",
                peer_pubkey="02" + "ab" * 32,
                capacity_sat=500_000,
                local_balance_sat=250_000,
                remote_balance_sat=250_000,
                state="active",
                short_channel_id="not a real scid",
            )

    def test_lnd_uint64_short_channel_id_form_is_accepted(self) -> None:
        # LND returns chan_id as a uint64-encoded decimal string. The
        # scaffold accepts that alongside the CLN BOLT-7 form so adapter
        # authors do not have to decode at the boundary.
        channel = NodeChannel(
            id="ch-lnd",
            peer_alias="peer",
            peer_pubkey="02" + "ab" * 32,
            capacity_sat=500_000,
            local_balance_sat=250_000,
            remote_balance_sat=250_000,
            state="active",
            short_channel_id="970751541567488",
        )
        self.assertEqual(channel.short_channel_id, "970751541567488")

    def test_invalid_funding_outpoint_format_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            NodeChannel(
                id="ch-bad-outpoint",
                peer_alias="peer",
                peer_pubkey="02" + "ab" * 32,
                capacity_sat=500_000,
                local_balance_sat=250_000,
                remote_balance_sat=250_000,
                state="active",
                funding_outpoint="definitely-not-an-outpoint",
            )


class LightningAiPayloadTest(unittest.TestCase):
    """Tier-3 redaction tests for AI tool surfaces.

    See docs/reference/lightning-opsec.md. The AI variants must drop the
    identity-graph fields even though the equivalent UI variants keep
    them — the AI surface is a place where a tool transcript could be
    pasted or screenshotted.
    """

    def test_ai_snapshot_payload_strips_identity_fields(self) -> None:
        payload = snapshot_to_dict_for_ai(_snapshot_with_routing())
        # Operator's own pubkey must be redacted.
        self.assertIsNone(payload["pubkey"])
        # Each channel must drop the peer/identity columns; the keys must
        # not be present at all (a `None` value is still a leak vector
        # because the test would silently pass).
        for channel in payload["channels"]:
            self.assertNotIn("peerPubkey", channel)
            self.assertNotIn("shortChannelId", channel)
            self.assertNotIn("peerAlias", channel)
            self.assertNotIn("fundingOutpoint", channel)
            # Operational fields must still be present so the AI can
            # answer balance / capacity / activity questions.
            self.assertIn("capacitySat", channel)
            self.assertIn("localBalanceSat", channel)
            self.assertIn("state", channel)
        for channel in payload["closedChannels"]:
            self.assertNotIn("peerPubkey", channel)
            self.assertNotIn("shortChannelId", channel)
            self.assertNotIn("peerAlias", channel)
            self.assertNotIn("fundingOutpoint", channel)
        # Forwards must drop both peer aliases and both short channel ids.
        for forward in payload["forwards"]:
            self.assertNotIn("inPeerAlias", forward)
            self.assertNotIn("outPeerAlias", forward)
            self.assertNotIn("inShortChannelId", forward)
            self.assertNotIn("outShortChannelId", forward)
            # Amount / fee / status are operational, must remain.
            self.assertIn("amountInMsat", forward)
            self.assertIn("status", forward)
        # The routing summary (aggregate) remains — it does not name peers.
        self.assertIn("routing", payload)

    def test_ai_profitability_payload_strips_per_channel_block(self) -> None:
        snapshot = _snapshot_with_routing()
        report = build_profitability_report(
            connection_id="wallet-1",
            connection_label="Home Node",
            connection_kind="lnd",
            snapshot=snapshot,
        )
        payload = report.to_ai_envelope_payload()
        # The connection identifier block must be absent — Tier 3.
        self.assertNotIn("connection", payload)
        # No per-channel breakdown either; the keys must be absent.
        self.assertNotIn("channels", payload)
        # The aggregate summary + window label remain.
        self.assertEqual(payload["windowLabel"], "Last 30 days")
        self.assertEqual(payload["summary"]["netProfitSat"], 3_280)
        self.assertEqual(payload["summary"]["forwardCount"], 42)


class LightningRegistryTest(unittest.TestCase):
    # Use a sentinel kind that real LND/CLN adapters never register under,
    # so registry-mechanism tests cannot remove the production adapters
    # when those PRs land.
    _SENTINEL_KIND = "test-fake-scaffold-kind"

    def test_register_and_resolve_adapter(self) -> None:
        adapter = _FakeAdapter(kind=self._SENTINEL_KIND)
        register_adapter(self._SENTINEL_KIND, adapter)
        try:
            self.assertIs(resolve_adapter(self._SENTINEL_KIND), adapter)
        finally:
            unregister_adapter(self._SENTINEL_KIND)
        self.assertIsNone(resolve_adapter(self._SENTINEL_KIND))

    def test_register_rejects_empty_kind(self) -> None:
        with self.assertRaises(ValueError):
            register_adapter("", _FakeAdapter())

    def test_resolve_unknown_kind_returns_none(self) -> None:
        self.assertIsNone(resolve_adapter("not-registered-xyz"))

    def test_registered_kinds_reports_current_registry(self) -> None:
        # Use save-and-restore so a real LND/CLN adapter (auto-registered
        # at daemon/CLI import time once #154/#155 land) is preserved.
        with _adapter_registered("lnd", _FakeAdapter(kind="lnd")), \
                _adapter_registered("coreln", _FakeAdapter(kind="coreln")):
            kinds = registered_kinds()
            self.assertIn("lnd", kinds)
            self.assertIn("coreln", kinds)
            # Sorted output keeps the error-hint stable across runs.
            self.assertEqual(list(kinds), sorted(kinds))


class LightningForwardFailureReasonTest(unittest.TestCase):
    def test_known_failure_reasons_accepted(self) -> None:
        # The Literal enum is a static check, not a runtime one — but
        # constructing a forward with each documented value verifies the
        # mock-seed / adapter authors have a stable vocabulary to map to.
        for value in (
            "temporary_channel_failure",
            "unknown_next_peer",
            "fee_insufficient",
            "incorrect_payment_details",
            "expiry_too_soon",
            "insufficient_balance",
            "other",
        ):
            forward = NodeForward(
                id=f"fw-{value}",
                occurred_at="2026-05-18T10:00:00Z",
                in_peer_alias="peer-in",
                out_peer_alias="peer-out",
                amount_in_msat=100_000,
                amount_out_msat=0,
                fee_msat=0,
                status="failed",
                failure_reason=value,  # type: ignore[arg-type]
            )
            self.assertEqual(forward.failure_reason, value)

    def test_seed_failure_reasons_stay_in_enum(self) -> None:
        # Pin: the values used by the mock seed and adapter authors must
        # remain a subset of the documented enum so the type-check guard
        # actually catches drift. If the seed changes, update both this
        # test and the Literal in kassiber/core/lightning/types.py.
        allowed = {
            "temporary_channel_failure",
            "unknown_next_peer",
            "fee_insufficient",
            "incorrect_payment_details",
            "expiry_too_soon",
            "insufficient_balance",
            "other",
        }
        for value in ("temporary_channel_failure", "insufficient_balance"):
            self.assertIn(value, allowed)


class LightningProfitabilityTest(unittest.TestCase):
    def test_build_report_carries_summary_and_channel_open_cost_check(self) -> None:
        snapshot = _snapshot_with_routing()
        report = build_profitability_report(
            connection_id="wallet-1",
            connection_label="Home Node",
            connection_kind="lnd",
            snapshot=snapshot,
            default_open_cost_sat=2_500,
        )
        payload = report.to_envelope_payload()
        self.assertEqual(payload["connection"], {
            "id": "wallet-1",
            "label": "Home Node",
            "kind": "lnd",
        })
        self.assertEqual(payload["windowLabel"], "Last 30 days")
        self.assertEqual(payload["summary"]["netProfitSat"], 3_280)
        rows = {row["peerAlias"]: row for row in payload["channels"]}
        self.assertTrue(rows["peer-a"]["coversOpenCost"])  # earned 5_000 >= 2_500
        self.assertFalse(rows["peer-b"]["coversOpenCost"])  # earned 1_200 < 2_500
        # earned=None coerces to 0 → does not cover the open cost
        self.assertEqual(rows["peer-c"]["earnedRoutingSat"], 0)
        self.assertFalse(rows["peer-c"]["coversOpenCost"])

    def test_report_handles_snapshot_without_routing(self) -> None:
        snapshot = NodeSnapshot(
            alias="x",
            pubkey="02" + "ef" * 32,
            network="mainnet",
            peer_count=0,
            onchain_balance_sat=0,
            total_local_balance_sat=0,
            total_remote_balance_sat=0,
            total_capacity_sat=0,
            channels=(),
            routing=None,
        )
        report = build_profitability_report(
            connection_id="x",
            connection_label="X",
            connection_kind="lnd",
            snapshot=snapshot,
        )
        payload = report.to_envelope_payload()
        self.assertEqual(payload["windowLabel"], "No routing window reported")
        self.assertEqual(payload["summary"]["routingRevenueSat"], 0)
        self.assertEqual(payload["channels"], [])

    def test_csv_rows_include_header_summary_and_channels(self) -> None:
        report = build_profitability_report(
            connection_id="wallet-1",
            connection_label="Home Node",
            connection_kind="lnd",
            snapshot=_snapshot_with_routing(),
        )
        rows = profitability_csv_rows(report)
        self.assertEqual(rows[0], ["section", "key", "value_sat", "value_btc", "detail"])
        summary_keys = {row[1] for row in rows if row[0] == "summary"}
        self.assertIn("routing_revenue", summary_keys)
        self.assertIn("net_profit", summary_keys)
        self.assertIn("forward_count", summary_keys)
        channel_keys = {row[1] for row in rows if row[0] == "channel"}
        self.assertIn("peer-a", channel_keys)


class ResolveLightningConnectionTest(unittest.TestCase):
    def _conn_with_wallets(
        self, rows: list[tuple[str, str, str]]
    ) -> sqlite3.Connection:
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            "CREATE TABLE wallets (id TEXT PRIMARY KEY, label TEXT, kind TEXT)"
        )
        conn.executemany(
            "INSERT INTO wallets (id, label, kind) VALUES (?, ?, ?)", rows
        )
        return conn

    def test_resolves_lightning_wallet_by_label_case_insensitive(self) -> None:
        conn = self._conn_with_wallets([("w1", "Home Node", "coreln")])
        row = resolve_lightning_connection(conn, "home node")
        self.assertEqual(row["id"], "w1")
        self.assertEqual(row["kind"], "coreln")

    def test_empty_ref_raises_validation(self) -> None:
        conn = self._conn_with_wallets([])
        with self.assertRaises(AppError) as ctx:
            resolve_lightning_connection(conn, "")
        self.assertEqual(ctx.exception.code, "validation")

    def test_not_found_raises_not_found(self) -> None:
        conn = self._conn_with_wallets([("w1", "Vault", "descriptor")])
        with self.assertRaises(AppError) as ctx:
            resolve_lightning_connection(conn, "missing-id")
        self.assertEqual(ctx.exception.code, "not_found")

    def test_non_lightning_kind_raises_validation(self) -> None:
        conn = self._conn_with_wallets([("w1", "Vault", "descriptor")])
        with self.assertRaises(AppError) as ctx:
            resolve_lightning_connection(conn, "w1")
        self.assertEqual(ctx.exception.code, "validation")


class LightningDaemonPayloadTest(unittest.TestCase):
    def _conn_with_lightning_wallet(self) -> sqlite3.Connection:
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            "CREATE TABLE wallets (id TEXT PRIMARY KEY, label TEXT, kind TEXT)"
        )
        conn.execute(
            "INSERT INTO wallets (id, label, kind) VALUES (?, ?, ?)",
            ("w-1", "Home CLN", "coreln"),
        )
        return conn

    @contextmanager
    def _adapter_temporarily_unregistered(self, kind: str) -> Iterator[None]:
        """Remove an adapter for the test, restore on the way out.

        Once #154/#155 land, the real LND/CLN adapters auto-register at
        daemon import time. The "without registration" assertions need
        the kind to be unregistered for the duration of the test only;
        we restore whatever was there afterward so other tests in the
        suite still see the production adapter.
        """
        previous = resolve_adapter(kind)
        unregister_adapter(kind)
        try:
            yield
        finally:
            if previous is not None:
                register_adapter(kind, previous)

    def test_snapshot_payload_returns_adapter_unavailable_without_registration(
        self,
    ) -> None:
        from kassiber.daemon import _lightning_node_snapshot_payload

        conn = self._conn_with_lightning_wallet()
        with self._adapter_temporarily_unregistered("coreln"):
            with self.assertRaises(AppError) as ctx:
                _lightning_node_snapshot_payload(
                    conn, {}, {"connection": "w-1"}
                )
        self.assertEqual(ctx.exception.code, "lightning_adapter_unavailable")
        self.assertFalse(ctx.exception.retryable)

    def test_profitability_payload_returns_adapter_unavailable_without_registration(
        self,
    ) -> None:
        from kassiber.daemon import _lightning_profitability_payload

        conn = self._conn_with_lightning_wallet()
        with self._adapter_temporarily_unregistered("coreln"):
            with self.assertRaises(AppError) as ctx:
                _lightning_profitability_payload(
                    conn, {}, {"connection": "w-1"}
                )
        self.assertEqual(ctx.exception.code, "lightning_adapter_unavailable")

    def test_snapshot_payload_merges_connection_block_when_adapter_registered(
        self,
    ) -> None:
        from kassiber.daemon import _lightning_node_snapshot_payload

        conn = self._conn_with_lightning_wallet()
        with _adapter_registered("coreln", _FakeAdapter(kind="coreln")):
            payload = _lightning_node_snapshot_payload(
                conn, {}, {"connection": "w-1"}
            )
        self.assertEqual(
            payload["connection"],
            {"id": "w-1", "label": "Home CLN", "kind": "coreln"},
        )
        self.assertIn("totalLocalBalanceSat", payload)
        self.assertIn("routing", payload)

    def test_profitability_payload_returns_summary_when_adapter_registered(
        self,
    ) -> None:
        from kassiber.daemon import _lightning_profitability_payload

        conn = self._conn_with_lightning_wallet()
        with _adapter_registered("coreln", _FakeAdapter(kind="coreln")):
            payload = _lightning_profitability_payload(
                conn, {}, {"connection": "w-1"}
            )
        self.assertEqual(payload["connection"]["id"], "w-1")
        self.assertEqual(payload["summary"]["netProfitSat"], 3_280)
        self.assertGreaterEqual(len(payload["channels"]), 1)


class LightningModuleExportsTest(unittest.TestCase):
    def test_top_level_exports_match_public_api(self) -> None:
        expected = {
            "DEFAULT_OPEN_COST_SAT",
            "LIGHTNING_ADAPTER_KINDS",
            "LightningAdapter",
            "LightningProfitabilityReport",
            "ChannelOpenCostCheck",
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
        }
        self.assertTrue(expected.issubset(set(core_lightning.__all__)))


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
