"""Tests for the shared Lightning scaffold (types, registry, profitability).

The scaffold is exercised indirectly by adapter tests in #154/#155 once they
land; this module pins the contract those adapters depend on so they can be
written and reviewed independently.
"""

from __future__ import annotations

import sqlite3
import unittest
from dataclasses import dataclass
from typing import Any

from kassiber.core import lightning as core_lightning
from kassiber.core.lightning import (
    NodeChannel,
    NodeForward,
    NodeRoutingSnapshot,
    NodeSnapshot,
    build_profitability_report,
    profitability_csv_rows,
    register_adapter,
    resolve_adapter,
    resolve_lightning_connection,
    snapshot_to_dict,
    unregister_adapter,
)
from kassiber.errors import AppError


def _channel(
    id_: str,
    earned: int | None = None,
    capacity: int = 1_000_000,
) -> NodeChannel:
    return NodeChannel(
        id=id_,
        peer_alias=f"peer-{id_}",
        peer_pubkey="02" + "ab" * 32,
        capacity_sat=capacity,
        local_balance_sat=capacity // 2,
        remote_balance_sat=capacity - capacity // 2,
        state="active",
        short_channel_id=f"800000x{id_}x0",
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
            _channel("a", earned=5_000),
            _channel("b", earned=1_200),
            _channel("c", earned=None),
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


class LightningRegistryTest(unittest.TestCase):
    def test_register_and_resolve_adapter(self) -> None:
        adapter = _FakeAdapter(kind="lnd")
        register_adapter("lnd", adapter)
        try:
            self.assertIs(resolve_adapter("lnd"), adapter)
        finally:
            unregister_adapter("lnd")
        self.assertIsNone(resolve_adapter("lnd"))

    def test_register_rejects_empty_kind(self) -> None:
        with self.assertRaises(ValueError):
            register_adapter("", _FakeAdapter())

    def test_resolve_unknown_kind_returns_none(self) -> None:
        self.assertIsNone(resolve_adapter("not-registered-xyz"))


class LightningProfitabilityTest(unittest.TestCase):
    def test_build_report_carries_summary_and_channel_break_even(self) -> None:
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

    def test_snapshot_payload_returns_adapter_unavailable_without_registration(
        self,
    ) -> None:
        from kassiber.daemon import _lightning_node_snapshot_payload

        conn = self._conn_with_lightning_wallet()
        unregister_adapter("coreln")
        with self.assertRaises(AppError) as ctx:
            _lightning_node_snapshot_payload(conn, {}, {"connection": "w-1"})
        self.assertEqual(ctx.exception.code, "lightning_adapter_unavailable")
        self.assertFalse(ctx.exception.retryable)

    def test_profitability_payload_returns_adapter_unavailable_without_registration(
        self,
    ) -> None:
        from kassiber.daemon import _lightning_profitability_payload

        conn = self._conn_with_lightning_wallet()
        unregister_adapter("coreln")
        with self.assertRaises(AppError) as ctx:
            _lightning_profitability_payload(conn, {}, {"connection": "w-1"})
        self.assertEqual(ctx.exception.code, "lightning_adapter_unavailable")

    def test_snapshot_payload_merges_connection_block_when_adapter_registered(
        self,
    ) -> None:
        from kassiber.daemon import _lightning_node_snapshot_payload

        conn = self._conn_with_lightning_wallet()
        register_adapter("coreln", _FakeAdapter(kind="coreln"))
        try:
            payload = _lightning_node_snapshot_payload(
                conn, {}, {"connection": "w-1"}
            )
        finally:
            unregister_adapter("coreln")
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
        register_adapter("coreln", _FakeAdapter(kind="coreln"))
        try:
            payload = _lightning_profitability_payload(
                conn, {}, {"connection": "w-1"}
            )
        finally:
            unregister_adapter("coreln")
        self.assertEqual(payload["connection"]["id"], "w-1")
        self.assertEqual(payload["summary"]["netProfitSat"], 3_280)
        self.assertGreaterEqual(len(payload["channels"]), 1)


class LightningModuleExportsTest(unittest.TestCase):
    def test_top_level_exports_match_public_api(self) -> None:
        expected = {
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
        }
        self.assertTrue(expected.issubset(set(core_lightning.__all__)))


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
